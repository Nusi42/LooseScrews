// HTTP clients for the server list, parties and duels (M3 package B7):
// the game server's JSON API (`/api/...`, crates/ls-server/src/api.rs) and
// the directory's (`GET /servers`, `GET /invites`, crates/ls-directory).
// Both answer CORS, so a page served by one server can talk to another.
// `fetch` is injectable for tests.

/** The part of `fetch` the clients use. */
export type JsonFetch = (
  url: string,
  init?: { method?: string; headers?: Record<string, string>; body?: string; signal?: AbortSignal },
) => Promise<{ ok: boolean; status: number; json(): Promise<unknown> }>;

/** A request failed (network, HTTP status or unexpected JSON). */
export class ApiError extends Error {
  override name = "ApiError";
  readonly status: number;
  constructor(message: string, status = 0) {
    super(message);
    this.status = status;
  }
}

/** Default time limit of one request. */
export const REQUEST_TIMEOUT_MS = 5000;

const browserFetch: JsonFetch = (url, init) => fetch(url, init);

/** A player in a server's roster. */
export interface RosterEntry {
  name: string;
  id: string;
}

/** What a server publishes (directory entry, `GET /api/info`). */
export interface ServerInfo {
  name: string;
  address: string;
  cert_hash?: string;
  region: string;
  mode: string;
  map_code: string;
  rules_code?: string;
  players: number;
  max_players: number;
  protocol: number;
  data_hash: string;
  roster?: RosterEntry[];
}

/** `GET /api/info`. */
export interface InfoReply extends ServerInfo {
  directory: string | null;
  rooms: { label: string; phase: string; mode: string; share_code: string; ruleset: string; players: number; max_players: number }[];
  private_rooms_free: number;
}

/** A message in a player's inbox (`POST /api/presence`). */
export interface Note {
  id: string;
  /** `duel` (a challenge), `accepted` or `declined` (answers to yours). */
  kind: string;
  from_id: string;
  from_name: string;
  room?: string;
  world_code?: string;
  rules_code?: string;
  expires_in_ms: number;
  /** Server the note points at (relayed invites: the challenger's server). */
  address?: string;
}

/** A directory list entry. */
export interface ServerEntry {
  id: string;
  age_ms: number;
  server: ServerInfo;
}

/** A relayed invite (`GET /invites?player=`). */
export interface RelayedInvite {
  id: string;
  expires_in_ms: number;
  invite: {
    to: string;
    kind: string;
    from_name: string;
    from_id: string;
    address: string;
    room?: string;
    world_code?: string;
    rules_code?: string;
  };
}

async function request<T>(fetchFn: JsonFetch, url: string, method: string, body?: unknown, timeoutMs = REQUEST_TIMEOUT_MS): Promise<T> {
  const ctl = typeof AbortController === "undefined" ? null : new AbortController();
  const timer = ctl ? setTimeout(() => ctl.abort(), timeoutMs) : null;
  try {
    const init: { method: string; headers?: Record<string, string>; body?: string; signal?: AbortSignal } = { method };
    if (body !== undefined) {
      init.headers = { "Content-Type": "application/json" };
      init.body = JSON.stringify(body);
    }
    if (ctl) init.signal = ctl.signal;
    let r;
    try {
      r = await fetchFn(url, init);
    } catch (err) {
      throw new ApiError(`cannot reach ${url}: ${err instanceof Error ? err.message : String(err)}`);
    }
    let json: unknown = null;
    try {
      json = await r.json();
    } catch {
      json = null;
    }
    if (!r.ok) {
      const msg = json && typeof json === "object" && "error" in json ? String((json as { error: unknown }).error) : `HTTP ${r.status}`;
      throw new ApiError(msg, r.status);
    }
    return json as T;
  } finally {
    if (timer) clearTimeout(timer);
  }
}

/** The game server's `/api`. `base` is `http(s)://host:port`. */
export class ServerApi {
  readonly base: string;
  private readonly fetchFn: JsonFetch;

  constructor(base: string, fetchFn: JsonFetch = browserFetch) {
    this.base = base.replace(/\/+$/, "");
    this.fetchFn = fetchFn;
  }

  info(): Promise<InfoReply> {
    return request(this.fetchFn, `${this.base}/api/info`, "GET");
  }

  players(): Promise<{ players: RosterEntry[] }> {
    return request(this.fetchFn, `${this.base}/api/players`, "GET");
  }

  presence(id: string, token: string, name: string): Promise<{ invites: Note[]; address: string; directory: string | null }> {
    return request(this.fetchFn, `${this.base}/api/presence`, "POST", { id, token, name });
  }

  party(req: { room?: string | null; player?: number | null; world_code?: string | null; rules_code?: string | null }): Promise<{
    party: string;
    room: string | null;
    address: string;
  }> {
    return request(this.fetchFn, `${this.base}/api/party`, "POST", req);
  }

  customRoom(world: string | null, rules: string | null): Promise<{ room: string | null; address: string }> {
    return request(this.fetchFn, `${this.base}/api/rooms`, "POST", { world_code: world, rules_code: rules });
  }

  duel(req: {
    id: string;
    token: string;
    name: string;
    to: string;
    to_name?: string;
    world_code?: string | null;
    rules_code?: string | null;
  }): Promise<{ challenge: string; relayed: boolean }> {
    return request(this.fetchFn, `${this.base}/api/duel`, "POST", req);
  }

  answer(from: string, id: string, accept: boolean): Promise<{ room: string | null; address: string }> {
    return request(this.fetchFn, `${this.base}/api/duel/answer`, "POST", { from, id, accept });
  }
}

/** The directory's API. `base` is its URL (`http://host:port[/path]`). */
export class DirectoryApi {
  readonly base: string;
  private readonly fetchFn: JsonFetch;

  constructor(base: string, fetchFn: JsonFetch = browserFetch) {
    this.base = base.replace(/\/+$/, "");
    this.fetchFn = fetchFn;
  }

  /** Servers this build can join (same protocol and data hash; both optional filters). */
  async servers(protocol?: number, dataHash?: string): Promise<ServerEntry[]> {
    const q: string[] = [];
    if (protocol !== undefined) q.push(`protocol=${protocol}`);
    if (dataHash) q.push(`data_hash=${encodeURIComponent(dataHash)}`);
    const url = `${this.base}/servers${q.length ? `?${q.join("&")}` : ""}`;
    const r = await request<{ servers?: ServerEntry[] }>(this.fetchFn, url, "GET");
    return Array.isArray(r.servers) ? r.servers : [];
  }

  /** Invites relayed to a public player id. */
  async invites(player: string): Promise<RelayedInvite[]> {
    const r = await request<{ invites?: RelayedInvite[] }>(this.fetchFn, `${this.base}/invites?player=${encodeURIComponent(player)}`, "GET");
    return Array.isArray(r.invites) ? r.invites : [];
  }
}

/** A relayed invite as an inbox note pointing at the challenger's server. */
export function noteFromRelay(r: RelayedInvite): Note {
  const n: Note = {
    id: `dir-${r.id}`,
    kind: r.invite.kind,
    from_id: r.invite.from_id,
    from_name: r.invite.from_name,
    expires_in_ms: r.expires_in_ms,
    address: r.invite.address,
  };
  if (r.invite.room) n.room = r.invite.room;
  if (r.invite.world_code) n.world_code = r.invite.world_code;
  if (r.invite.rules_code) n.rules_code = r.invite.rules_code;
  return n;
}

/** Remembers which notes were shown, so polls that repeat them stay quiet. */
export class NoteInbox {
  private readonly seen = new Map<string, number>();

  /** The notes not seen before (`now` in ms; entries are forgotten after they expire). */
  fresh(notes: readonly Note[], now: number): Note[] {
    for (const [id, until] of this.seen) if (until < now) this.seen.delete(id);
    const out: Note[] = [];
    for (const n of notes) {
      if (this.seen.has(n.id)) continue;
      this.seen.set(n.id, now + Math.max(0, n.expires_in_ms) + 60_000);
      out.push(n);
    }
    return out;
  }
}
