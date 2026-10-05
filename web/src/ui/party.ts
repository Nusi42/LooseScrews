// Invites, parties and duel challenges in the browser (M3 package B7; plan
// §3.1 "Party codes", "Invite links", "Duels", "Player identity").
//
// - Invite links: `?join=<host:port>&code=<LS..>&rules=<LR..>&party=<P-..>`
//   (every part optional; `room=<R-..>` joins a private room, `world=` is
//   accepted for `code=` as the rule set editor writes it). Opening one joins
//   that server; with a world or rule set code but no party or room code it
//   first opens a private custom match there (`POST /api/rooms`).
// - Identity: a random public player id and a private token, kept in
//   localStorage (per tab if storage is unavailable). The id is what others
//   challenge; the token proves it to the server.
// - In game (`SocialSetup.attach`): the browser polls the server's
//   `/api/presence` (roster entry + inbox) and, if the server names a
//   directory, the directory's relayed invites. A challenge shows a toast
//   with Accept/Decline (30 s); accepting opens the duel room on the
//   challenger's server and both players move there by invite link. The
//   party panel (P button) creates a party invite for the current room and
//   lists the players on the server with a Duel button (on the current
//   world: the challenger's seed).
//
// main.ts makes one call, `startSocial`, and uses its `serverUrl`, `api`
// and `attach`.

import { httpBaseFor, wsUrlFor } from "../net/ping.ts";
import { DirectoryApi, NoteInbox, noteFromRelay, ServerApi, type Note, type RosterEntry } from "../net/social.ts";
import { serverUrl } from "../net/transport.ts";
import type { GameHandle, WasmApi } from "../wasm/api.ts";
import { showServerList } from "./serverList.ts";

/** What an invite link (or the page URL) asks for. */
export interface JoinParams {
  /** Server `host:port` (`join=`), or null for the page's server. */
  address: string | null;
  party: string | null;
  room: string | null;
  /** World share code (`code=` or `world=`). */
  world: string | null;
  /** Rule set code (`rules=`). */
  rules: string | null;
  /** Directory URL (`directory=`). */
  directory: string | null;
  /** Show the server list first (`servers`). */
  servers: boolean;
}

const CODE_RE = /^[A-Za-z0-9-]{1,600}$/;
const ADDRESS_RE = /^[A-Za-z0-9.\-:[\]]{1,255}$/;

function codeParam(u: URL, ...keys: string[]): string | null {
  for (const k of keys) {
    const v = u.searchParams.get(k)?.trim();
    if (v && CODE_RE.test(v)) return v.toUpperCase();
  }
  return null;
}

/** Reads the invite parameters of a page URL (invalid parts are ignored). */
export function parseJoin(href: string | URL): JoinParams {
  const u = typeof href === "string" ? new URL(href) : href;
  const join = u.searchParams.get("join")?.trim() ?? "";
  const directory = u.searchParams.get("directory")?.trim() ?? "";
  return {
    address: ADDRESS_RE.test(join) ? join : null,
    party: codeParam(u, "party"),
    room: codeParam(u, "room"),
    world: codeParam(u, "code", "world"),
    rules: codeParam(u, "rules"),
    directory: /^https?:\/\/[^\s]+$/i.test(directory) ? directory : null,
    servers: u.searchParams.has("servers"),
  };
}

/** The parts of an invite link. */
export interface InviteParts {
  address?: string | null;
  party?: string | null;
  room?: string | null;
  world?: string | null;
  rules?: string | null;
  directory?: string | null;
}

/** An invite link on the page `pageHref` (its origin and path, new query). */
export function inviteLink(pageHref: string, parts: InviteParts): string {
  const u = new URL(pageHref);
  const keep = new URLSearchParams();
  // Development parameters of the page stay.
  for (const k of ["transport"]) {
    const v = u.searchParams.get(k);
    if (v) keep.set(k, v);
  }
  if (parts.address) keep.set("join", parts.address);
  if (parts.party) keep.set("party", parts.party);
  if (parts.room) keep.set("room", parts.room);
  if (parts.world) keep.set("code", parts.world);
  if (parts.rules) keep.set("rules", parts.rules);
  if (parts.directory) keep.set("directory", parts.directory);
  u.search = keep.toString();
  u.hash = "";
  return u.toString();
}

/** The `host:port` of the server the page plays on: `join=`, else `?server=` (address or ws URL), else the page's host. */
export function serverAddress(loc: { host: string; search: string }, params: JoinParams): string {
  if (params.address) return params.address;
  const override = new URLSearchParams(loc.search).get("server");
  if (override) {
    const m = /^wss?:\/\/([^/]+)/i.exec(override);
    return m?.[1] ?? override;
  }
  return loc.host;
}

/** The player's public id and private token. */
export interface Identity {
  id: string;
  token: string;
}

/** Storage the identity lives in (localStorage in the browser). */
export interface KeyValueStore {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

export const IDENTITY_KEY = "ls.identity";

/** `bytes` random bytes as lower-case hex. */
export function randomHex(bytes: number): string {
  const b = new Uint8Array(bytes);
  crypto.getRandomValues(b);
  return Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");
}

const ID_RE = /^p-[0-9a-f]{16}$/;
const TOKEN_RE = /^[0-9a-f]{32}$/;

/** The stored identity, or a new one (stored when possible). */
export function loadIdentity(store: KeyValueStore | null, random: (bytes: number) => string = randomHex): Identity {
  try {
    const raw = store?.getItem(IDENTITY_KEY);
    if (raw) {
      const v = JSON.parse(raw) as Partial<Identity>;
      if (typeof v.id === "string" && ID_RE.test(v.id) && typeof v.token === "string" && TOKEN_RE.test(v.token)) {
        return { id: v.id, token: v.token };
      }
    }
  } catch {
    /* unreadable: make a new one */
  }
  const fresh = { id: `p-${random(8)}`, token: random(16) };
  try {
    store?.setItem(IDENTITY_KEY, JSON.stringify(fresh));
  } catch {
    /* not stored: this tab only */
  }
  return fresh;
}

function browserStore(): KeyValueStore | null {
  try {
    return localStorage;
  } catch {
    try {
      return sessionStorage;
    } catch {
      return null;
    }
  }
}

type HelloWith = (name: string, preferredTeam: number, shipType: number, room: string, party: string) => Uint8Array;

/** A code now (read when the Hello is built). */
export type CodeSource = () => string | null;

/**
 * `api` whose Hello packets carry the room and party code current at the
 * time (plain Hellos while both are null, and with a core that has no
 * `helloPacketWith`).
 */
export function withJoinCodes(api: WasmApi, room: CodeSource, party: CodeSource): WasmApi {
  const helloWith = (api as WasmApi & { helloPacketWith?: HelloWith }).helloPacketWith;
  if (!helloWith) return api;
  const plainAs = api.helloPacketAs?.bind(api);
  return {
    ...api,
    helloPacket: (name, team) => {
      const [r, p] = [room(), party()];
      return r || p ? helloWith(name, team, 255, r ?? "", p ?? "") : api.helloPacket(name, team);
    },
    helloPacketAs: (name, team, ship) => {
      const [r, p] = [room(), party()];
      if (r || p) return helloWith(name, team, ship, r ?? "", p ?? "");
      return plainAs ? plainAs(name, team, ship) : api.helloPacket(name, team);
    },
  };
}

/** Opens connections (main.ts's `connectTransport` call). */
export type Opener<T> = (url: string) => Promise<T>;

/** Toast text for an inbox note. */
export function noteText(n: Note): string {
  switch (n.kind) {
    case "duel":
      return `${n.from_name} challenges you to a duel`;
    case "accepted":
      return `${n.from_name} accepted your duel: joining…`;
    case "declined":
      return `${n.from_name} declined your duel`;
    default:
      return `${n.from_name}: ${n.kind}`;
  }
}

/**
 * Whether an "accepted" note's room is where this page already plays (its
 * invite link's `room=`, or the running session's room): the server keeps
 * the note in the inbox until the challenge expires, so the page the note
 * led to sees it again and must not follow it once more (that reloaded the
 * challenger's page every poll for 30 s).
 */
export function alreadyInRoom(n: Note, pageHref: string, currentRoom: string | null): boolean {
  if (!n.room) return false;
  const room = n.room.toUpperCase();
  return parseJoin(pageHref).room === room || (currentRoom ?? "").toUpperCase() === room;
}

export const ANSWERED_KEY = "ls.answeredDuels";

/** Challenge note ids this tab answered, with their expiry (ms), still valid at `now`. */
export function answeredDuels(store: KeyValueStore | null, now: number): Map<string, number> {
  const out = new Map<string, number>();
  try {
    const raw = store?.getItem(ANSWERED_KEY);
    const v = raw ? (JSON.parse(raw) as Record<string, unknown>) : {};
    for (const [id, until] of Object.entries(v)) if (typeof until === "number" && until > now) out.set(id, until);
  } catch {
    /* unreadable: nothing answered */
  }
  return out;
}

/**
 * Remembers that the challenge note `id` was answered (until `until`, ms):
 * the server and the directory keep a challenge until it expires, so the
 * page an accepted duel leads to would offer the same challenge again.
 */
export function rememberAnswered(store: KeyValueStore | null, id: string, until: number, now: number): void {
  const m = answeredDuels(store, now);
  m.set(id, until);
  try {
    store?.setItem(ANSWERED_KEY, JSON.stringify(Object.fromEntries(m)));
  } catch {
    /* not stored: this page only */
  }
}

function tabStore(): KeyValueStore | null {
  try {
    return sessionStorage;
  } catch {
    return null;
  }
}

/** Players one can challenge: everyone in the roster but oneself, by name. */
export function duelTargets(roster: readonly RosterEntry[], self: string): RosterEntry[] {
  return roster.filter((p) => p.id !== self).sort((a, b) => a.name.localeCompare(b.name) || a.id.localeCompare(b.id));
}

/** What the in-game panel needs to know about the running game. */
export interface SessionView {
  /** Own wire id (`Welcome::player_id`). */
  player: number;
  /** Private room code ("" in the public room). */
  room: string;
  /** World of the match (the challenger's seed for duels). */
  shareCode: string;
}

/** A session view from the running game (the room code getter is optional in older builds). */
export function sessionView(game: GameHandle): SessionView {
  const room = (game as GameHandle & { readonly roomCode?: string }).roomCode ?? "";
  return { player: game.player, room, shareCode: game.shareCode };
}

/** What `startSocial` hands to main.ts. */
export interface SocialSetup {
  /** WebSocket URL of the server to play on. */
  serverUrl: string;
  /** The game core with the invite's room/party code in its Hello. */
  api: WasmApi;
  /**
   * Wraps the connection opener: before the first connection of an invite
   * with a world/rule set code but no party or room code, it opens that
   * custom match on the server (so the room exists when the Hello asks for
   * it, not when the page loaded).
   */
  opener<T>(open: Opener<T>): Opener<T>;
  /** `host:port` of that server. */
  address: string;
  /** Mounts the party panel, the presence poll and the challenge toasts (after the start screen). */
  attach(name: string, current: () => SessionView | null): void;
}

/** Name stored by the start screen (for challenges from the server list). */
function storedName(): string {
  try {
    const raw = localStorage.getItem("ls.start");
    const name = raw ? (JSON.parse(raw) as { name?: unknown }).name : null;
    return typeof name === "string" && name.trim() ? name.trim() : "Scrapling";
  } catch {
    return "Scrapling";
  }
}

/**
 * Reads the invite in the page URL, shows the server list when asked
 * (`?servers`), opens a custom match for a link with a world/rules code but
 * no party or room, and returns how to connect.
 */
export async function startSocial(ui: HTMLElement, api: WasmApi, loc: Location): Promise<SocialSetup> {
  const params = parseJoin(loc.href);
  const address = serverAddress(loc, params);
  const base = httpBaseFor(address, loc.protocol);
  const server = new ServerApi(base);
  const identity = loadIdentity(browserStore());
  if (params.servers) {
    await showServerList(ui, {
      pageHref: loc.href,
      pageProtocol: loc.protocol,
      originApi: new ServerApi(httpBaseFor(serverAddress(loc, { ...params, address: null }), loc.protocol)),
      directory: params.directory,
      identity,
      name: storedName(),
      navigate: (url) => loc.assign(url),
    });
  }
  let room = params.room;
  let custom = !params.party && !room && (params.world !== null || params.rules !== null);
  const ws = params.address ? wsUrlFor(params.address, loc.protocol) : serverUrl(loc);
  addServerListLink(ui, loc.href, params.directory);
  return {
    serverUrl: ws,
    api: withJoinCodes(api, () => room, () => params.party),
    opener:
      <T>(open: Opener<T>): Opener<T> =>
      async (url: string): Promise<T> => {
        if (custom) {
          custom = false;
          try {
            room = (await server.customRoom(params.world, params.rules)).room;
          } catch (err) {
            console.warn("custom match not opened; joining the server's rotation:", err);
          }
        }
        return open(url);
      },
    address,
    attach: (name, current) => {
      removeServerListLink();
      mountSocial({ name, current, server, address, identity, pageHref: loc.href, directory: params.directory, navigate: (u) => loc.assign(u) });
    },
  };
}

// ----- DOM -------------------------------------------------------------------

const STYLE_ID = "ls-social-style";

export function injectSocialStyle(): void {
  if (document.getElementById(STYLE_ID)) return;
  const s = document.createElement("style");
  s.id = STYLE_ID;
  s.textContent = `
.ls-social-btn { position: fixed; left: 12px; top: 12px; z-index: 30; font: 13px system-ui, sans-serif;
  background: rgba(16,20,28,.85); color: #e8ecf4; border: 1px solid #3a4558; border-radius: 6px; padding: 6px 10px; cursor: pointer; }
.ls-social-panel { position: fixed; left: 12px; top: 52px; z-index: 30; width: 300px; max-height: 60vh; overflow: auto;
  background: rgba(16,20,28,.94); color: #e8ecf4; border: 1px solid #3a4558; border-radius: 8px; padding: 10px 12px; font: 13px system-ui, sans-serif; }
.ls-social-panel h3 { margin: 0 0 6px; font-size: 14px; }
.ls-social-panel button, .ls-toast button, .ls-servers button { background: #2b6cb0; color: #fff; border: 0; border-radius: 4px; padding: 4px 9px; cursor: pointer; margin: 2px 4px 2px 0; }
.ls-social-panel button.secondary, .ls-toast button.secondary, .ls-servers button.secondary { background: #4a5568; }
.ls-social-panel input, .ls-servers input { width: 100%; box-sizing: border-box; background: #0d1117; color: #e8ecf4; border: 1px solid #3a4558; border-radius: 4px; padding: 4px 6px; font: 12px ui-monospace, monospace; }
.ls-social-panel .row { display: flex; justify-content: space-between; align-items: center; margin: 3px 0; }
.ls-social-panel .muted, .ls-servers .muted { color: #9aa5b8; font-size: 12px; }
.ls-toasts { position: fixed; top: 12px; left: 50%; transform: translateX(-50%); z-index: 40; display: flex; flex-direction: column; gap: 6px; }
.ls-toast { background: rgba(20,26,38,.96); color: #f4f6fb; border: 1px solid #5a6b88; border-radius: 8px; padding: 8px 12px; font: 14px system-ui, sans-serif; min-width: 260px; }
.ls-servers { position: fixed; inset: 0; z-index: 50; overflow: auto; background: #0b0e14; color: #e8ecf4; font: 14px system-ui, sans-serif; padding: 24px; }
.ls-servers table { border-collapse: collapse; width: 100%; max-width: 980px; }
.ls-servers th, .ls-servers td { text-align: left; padding: 6px 8px; border-bottom: 1px solid #222a38; vertical-align: top; }
.ls-servers .roster { font-size: 12px; color: #c3cad8; }
.ls-server-list-link { position: fixed; right: 12px; bottom: 12px; z-index: 20; color: #9cc3ff; font: 13px system-ui, sans-serif; }
`;
  document.head.appendChild(s);
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls = "", text = ""): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text) e.textContent = text;
  return e;
}

function button(text: string, onClick: () => void, secondary = false): HTMLButtonElement {
  const b = el("button", secondary ? "secondary" : "", text);
  b.type = "button";
  b.addEventListener("click", (ev) => {
    ev.stopPropagation();
    onClick();
  });
  return b;
}

let listLink: HTMLElement | null = null;

/** "Server list" link on the start screen (opens `?servers`). */
function addServerListLink(parent: HTMLElement, href: string, directory: string | null): void {
  injectSocialStyle();
  const u = new URL(href);
  u.search = `?servers${directory ? `&directory=${encodeURIComponent(directory)}` : ""}`;
  u.hash = "";
  const a = el("a", "ls-server-list-link", "Server list");
  a.setAttribute("href", u.toString());
  parent.appendChild(a);
  listLink = a;
  // Gone once the game takes the pointer (practice too).
  document.addEventListener("pointerlockchange", removeServerListLink, { once: true });
}

function removeServerListLink(): void {
  listLink?.remove();
  listLink = null;
}

/** Shows a toast; returns its element (removed after `ms` unless 0). */
function toast(text: string, ms: number, actions: HTMLElement[] = []): HTMLElement {
  injectSocialStyle();
  let box = document.querySelector<HTMLElement>(".ls-toasts");
  if (!box) {
    box = el("div", "ls-toasts");
    document.body.appendChild(box);
  }
  const t = el("div", "ls-toast");
  t.appendChild(el("div", "", text));
  if (actions.length) {
    const row = el("div");
    for (const a of actions) row.appendChild(a);
    t.appendChild(row);
  }
  box.appendChild(t);
  if (ms > 0) setTimeout(() => t.remove(), ms);
  return t;
}

/** Copies text to the clipboard (silently does nothing without permission). */
function copy(text: string): void {
  navigator.clipboard?.writeText(text).catch(() => {});
}

interface MountOptions {
  name: string;
  current: () => SessionView | null;
  server: ServerApi;
  address: string;
  identity: Identity;
  pageHref: string;
  directory: string | null;
  navigate: (url: string) => void;
}

/** Presence poll interval. */
const PRESENCE_MS = 3000;
/** Directory invite poll interval. */
const DIRECTORY_MS = 4000;

function mountSocial(o: MountOptions): void {
  injectSocialStyle();
  const inbox = new NoteInbox();
  let directory: DirectoryApi | null = o.directory ? new DirectoryApi(o.directory) : null;
  let roster: RosterEntry[] = [];

  const goTo = (address: string, room: string | null): void => {
    o.navigate(inviteLink(o.pageHref, { address, room, directory: o.directory }));
  };
  const show = (notes: Note[]): void => {
    const answered = answeredDuels(tabStore(), Date.now());
    for (const n of inbox.fresh(notes, Date.now())) {
      if (n.kind === "duel") {
        if (answered.has(n.id)) continue;
        const target = n.address ?? o.address;
        let t: HTMLElement | null = null;
        const answer = (accept: boolean): void => {
          t?.remove();
          rememberAnswered(tabStore(), n.id, Date.now() + Math.max(0, n.expires_in_ms) + 60_000, Date.now());
          const api = target === o.address ? o.server : new ServerApi(httpBaseFor(target, location.protocol));
          api
            .answer(n.from_id, o.identity.id, accept)
            .then((r) => {
              if (accept && r.room) goTo(r.address || target, r.room);
            })
            .catch((err: unknown) => toast(`Duel failed: ${err instanceof Error ? err.message : String(err)}`, 6000));
        };
        t = toast(noteText(n), Math.max(1000, n.expires_in_ms), [button("Accept", () => answer(true)), button("Decline", () => answer(false), true)]);
      } else if (n.kind === "accepted" && n.room) {
        if (alreadyInRoom(n, o.pageHref, o.current()?.room ?? null)) continue;
        toast(noteText(n), 4000);
        goTo(n.address ?? o.address, n.room);
      } else {
        toast(noteText(n), 6000);
      }
    }
  };

  const pollPresence = (): void => {
    o.server
      .presence(o.identity.id, o.identity.token, o.name)
      .then((r) => {
        if (!directory && r.directory) directory = new DirectoryApi(r.directory);
        show(r.invites);
      })
      .catch(() => {});
  };
  const pollDirectory = (): void => {
    directory
      ?.invites(o.identity.id)
      .then((list) => show(list.map(noteFromRelay)))
      .catch(() => {});
  };
  pollPresence();
  setInterval(pollPresence, PRESENCE_MS);
  setInterval(pollDirectory, DIRECTORY_MS);

  // Party panel.
  const btn = el("button", "ls-social-btn", "Party & duels");
  btn.type = "button";
  const panel = el("div", "ls-social-panel");
  panel.style.display = "none";
  document.body.append(btn, panel);
  // The last party invite (or error), kept across re-renders: the roster
  // fetch re-renders the panel and used to wipe a link created meanwhile.
  let invite: { party: string; link: string } | { error: string } | null = null;
  const render = (): void => {
    panel.replaceChildren();
    panel.appendChild(el("h3", "", "Party"));
    const out = el("div", "muted", "Invite friends into your room (and team).");
    if (invite && "error" in invite) {
      out.replaceChildren(el("div", "", `No party: ${invite.error}`));
    } else if (invite) {
      const link = invite.link;
      out.replaceChildren(el("div", "", `Party ${invite.party}`));
      const input = el("input");
      input.value = link;
      input.readOnly = true;
      out.append(input, button("Copy link", () => copy(link)));
    }
    panel.appendChild(out);
    panel.appendChild(
      button("Create party invite", () => {
        const s = o.current();
        o.server
          .party({ room: s?.room || null, player: s ? s.player : null })
          .then((r) => {
            invite = { party: r.party, link: inviteLink(o.pageHref, { address: r.address || o.address, party: r.party, directory: o.directory }) };
            render();
          })
          .catch((err: unknown) => {
            invite = { error: err instanceof Error ? err.message : String(err) };
            render();
          });
      }),
    );
    panel.appendChild(el("h3", "", "Duel a player"));
    const targets = duelTargets(roster, o.identity.id);
    if (!targets.length) panel.appendChild(el("div", "muted", "Nobody else on this server has the game open."));
    for (const p of targets) {
      const row = el("div", "row");
      row.appendChild(el("span", "", p.name));
      row.appendChild(
        button("Duel", () => {
          const world = o.current()?.shareCode || null;
          o.server
            .duel({ id: o.identity.id, token: o.identity.token, name: o.name, to: p.id, to_name: p.name, world_code: world })
            .then(() => toast(`Challenge sent to ${p.name} (30 s)`, 4000))
            .catch((err: unknown) => toast(`Challenge failed: ${err instanceof Error ? err.message : String(err)}`, 6000));
        }),
      );
      panel.appendChild(row);
    }
  };
  btn.addEventListener("click", () => {
    const open = panel.style.display === "none";
    panel.style.display = open ? "block" : "none";
    if (open) {
      render();
      o.server
        .players()
        .then((r) => {
          roster = r.players;
          render();
        })
        .catch(() => {});
    }
  });
}
