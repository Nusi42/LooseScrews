// Server list screen (M3 package B7; plan §3.1 "Directory service"): the
// servers registered with an `ls-directory` that this build can join (same
// protocol and data hash), with name, region, mode, players, ping (measured
// by the browser, net/ping.ts) and world; each server's roster has a Duel
// button. Join opens the server's invite link (`?join=<host:port>`).
//
// The directory comes from `?directory=<url>`, else from the page's own
// server (`GET /api/info`, its `--directory`). The protocol version and data
// hash the list is filtered by are the page's server's (the page is served
// by a server of the same build). Opened with `?servers` (the "Server list"
// link of the start screen); "Back" returns to the start screen.

import { formatPing, pingAll, probe, wsUrlFor } from "../net/ping.ts";
import { DirectoryApi, ServerApi, type InfoReply, type ServerEntry } from "../net/social.ts";
import { injectSocialStyle, inviteLink, type Identity } from "./party.ts";

/** Display names of the mode ids (`ls_sim::ModeKind::id`). */
export const MODE_NAMES: Readonly<Record<string, string>> = {
  paint_the_void: "Paint the Void",
  team_brawl: "Team Brawl",
  the_squeeze: "The Squeeze",
  heist: "Heist",
  hot_reactor: "Hot Reactor",
  last_color_standing: "Last Color Standing",
};

export function modeName(id: string): string {
  return MODE_NAMES[id] ?? (id || "-");
}

/** One row of the list. */
export interface ServerRow {
  name: string;
  address: string;
  region: string;
  mode: string;
  players: string;
  ping: string;
  world: string;
  full: boolean;
  roster: { id: string; name: string }[];
}

/**
 * Rows for the list: fullest servers first, then by ping (unknown last),
 * then by name; servers of another build are left out when `filter` is given.
 */
export function serverRows(
  entries: readonly ServerEntry[],
  pings: ReadonlyMap<string, number | null>,
  filter?: { protocol: number; dataHash: string },
): ServerRow[] {
  const list = entries.filter((e) => !filter || (e.server.protocol === filter.protocol && e.server.data_hash === filter.dataHash));
  const pingOf = (a: string): number => {
    const p = pings.get(a);
    return p === undefined || p === null ? Number.POSITIVE_INFINITY : p;
  };
  return [...list]
    .sort(
      (a, b) =>
        b.server.players - a.server.players ||
        pingOf(a.server.address) - pingOf(b.server.address) ||
        a.server.name.localeCompare(b.server.name) ||
        a.server.address.localeCompare(b.server.address),
    )
    .map((e) => ({
      name: e.server.name,
      address: e.server.address,
      region: e.server.region || "-",
      mode: modeName(e.server.mode),
      players: `${e.server.players}/${e.server.max_players}`,
      ping: formatPing(pings.get(e.server.address)),
      world: e.server.map_code,
      full: e.server.players >= e.server.max_players,
      roster: e.server.roster ?? [],
    }));
}

export interface ServerListOptions {
  pageHref: string;
  pageProtocol: string;
  /** The page's own server (directory URL, build filter, challenges). */
  originApi: ServerApi;
  /** `?directory=`, preferred over the origin server's. */
  directory: string | null;
  identity: Identity;
  /** Name challenges are sent with. */
  name: string;
  navigate: (url: string) => void;
}

/** Shows the server list until "Back" (a Join navigates away). */
export function showServerList(parent: HTMLElement, o: ServerListOptions): Promise<void> {
  injectSocialStyle();
  return new Promise((resolve) => {
    const root = document.createElement("div");
    root.className = "ls-servers";
    parent.appendChild(root);
    const status = document.createElement("div");
    status.className = "muted";
    const seed = document.createElement("input");
    seed.placeholder = "Duel world (share code; empty: random)";
    const rulesInput = document.createElement("input");
    rulesInput.placeholder = "Duel rule set code (optional)";
    const table = document.createElement("table");
    let info: InfoReply | null = null;
    let entries: ServerEntry[] = [];
    let pings = new Map<string, number | null>();
    let waiting: ReturnType<typeof setInterval> | null = null;

    const close = (): void => {
      if (waiting) clearInterval(waiting);
      root.remove();
      resolve();
    };
    const back = document.createElement("button");
    back.type = "button";
    back.className = "secondary";
    back.textContent = "Back";
    back.addEventListener("click", close);
    const refresh = document.createElement("button");
    refresh.type = "button";
    refresh.textContent = "Refresh";
    refresh.addEventListener("click", () => void load());
    const title = document.createElement("h2");
    title.textContent = "Servers";
    root.append(title, back, refresh, status, seed, rulesInput, table);

    const challenge = (address: string, to: { id: string; name: string }): void => {
      const api = o.originApi;
      api
        .presence(o.identity.id, o.identity.token, o.name)
        .then(() =>
          api.duel({
            id: o.identity.id,
            token: o.identity.token,
            name: o.name,
            to: to.id,
            to_name: to.name,
            world_code: seed.value.trim() || null,
            rules_code: rulesInput.value.trim() || null,
          }),
        )
        .then(() => {
          status.textContent = `Challenge sent to ${to.name} on ${address}; waiting for the answer (30 s)…`;
          if (waiting) clearInterval(waiting);
          const until = Date.now() + 31_000;
          waiting = setInterval(() => {
            if (Date.now() > until) {
              if (waiting) clearInterval(waiting);
              status.textContent = `${to.name} did not answer.`;
              return;
            }
            api
              .presence(o.identity.id, o.identity.token, o.name)
              .then((r) => {
                const n = r.invites.find((x) => x.from_id === to.id && (x.kind === "accepted" || x.kind === "declined"));
                if (!n) return;
                if (waiting) clearInterval(waiting);
                if (n.kind === "accepted" && n.room) o.navigate(inviteLink(o.pageHref, { address: r.address, room: n.room, directory: o.directory }));
                else status.textContent = `${to.name} declined.`;
              })
              .catch(() => {});
          }, 1500);
        })
        .catch((err: unknown) => {
          status.textContent = `Challenge failed: ${err instanceof Error ? err.message : String(err)}`;
        });
    };

    const render = (): void => {
      const filter = info ? { protocol: info.protocol, dataHash: info.data_hash } : undefined;
      const rows = serverRows(entries, pings, filter);
      table.replaceChildren();
      const head = document.createElement("tr");
      for (const h of ["Server", "Region", "Mode", "Players", "Ping", "World", ""]) {
        const th = document.createElement("th");
        th.textContent = h;
        head.appendChild(th);
      }
      table.appendChild(head);
      for (const r of rows) {
        const tr = document.createElement("tr");
        const nameCell = document.createElement("td");
        nameCell.textContent = r.name;
        const addr = document.createElement("div");
        addr.className = "muted";
        addr.textContent = r.address;
        nameCell.appendChild(addr);
        const roster = document.createElement("div");
        roster.className = "roster";
        for (const p of r.roster) {
          if (p.id === o.identity.id) continue;
          const line = document.createElement("div");
          line.textContent = `${p.name} `;
          const b = document.createElement("button");
          b.type = "button";
          b.className = "secondary";
          b.textContent = "Duel";
          b.addEventListener("click", () => challenge(r.address, p));
          line.appendChild(b);
          roster.appendChild(line);
        }
        nameCell.appendChild(roster);
        tr.appendChild(nameCell);
        for (const text of [r.region, r.mode, r.players, r.ping, r.world]) {
          const td = document.createElement("td");
          td.textContent = text;
          tr.appendChild(td);
        }
        const td = document.createElement("td");
        const join = document.createElement("button");
        join.type = "button";
        join.textContent = r.full ? "Full" : "Join";
        join.disabled = r.full;
        join.addEventListener("click", () => o.navigate(inviteLink(o.pageHref, { address: r.address, directory: o.directory })));
        td.appendChild(join);
        tr.appendChild(td);
        table.appendChild(tr);
      }
      if (!rows.length && entries.length) status.textContent = "No server of this game version is listed.";
    };

    const load = async (): Promise<void> => {
      status.textContent = "Loading…";
      try {
        info = await o.originApi.info();
      } catch {
        info = null;
      }
      const url = o.directory ?? info?.directory ?? null;
      if (!url) {
        status.textContent = "No server directory: open the page with ?directory=http://host:port, or ask the host to run ls-server --directory.";
        return;
      }
      try {
        entries = await new DirectoryApi(url).servers(info?.protocol, info?.data_hash);
      } catch (err) {
        status.textContent = `Directory unreachable (${url}): ${err instanceof Error ? err.message : String(err)}`;
        return;
      }
      status.textContent = entries.length ? `${entries.length} server(s) from ${url}` : `No servers registered with ${url}.`;
      pings = new Map();
      render();
      await pingAll(
        entries.map((e) => e.server.address),
        (a) => probe(wsUrlFor(a, o.pageProtocol)),
        undefined,
        (a, ms) => {
          pings.set(a, ms);
          render();
        },
      );
    };
    void load();
  });
}
