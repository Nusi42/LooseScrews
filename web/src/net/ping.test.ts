import assert from "node:assert/strict";
import { test } from "node:test";
import { formatPing, httpBaseFor, pingAll, probe, wsUrlFor } from "./ping.ts";
import { ApiError, DirectoryApi, NoteInbox, noteFromRelay, ServerApi, type JsonFetch, type Note } from "./social.ts";
import type { SocketLike } from "./transport.ts";

/** A fake WebSocket that opens (or fails) when told to. */
class FakeSocket implements SocketLike {
  binaryType: BinaryType = "arraybuffer";
  readyState = 0;
  bufferedAmount = 0;
  closed: { code?: number; reason?: string } | null = null;
  onopen: ((ev: Event) => unknown) | null = null;
  onmessage: ((ev: MessageEvent) => unknown) | null = null;
  onclose: ((ev: CloseEvent) => unknown) | null = null;
  onerror: ((ev: Event) => unknown) | null = null;
  readonly url: string;
  constructor(url: string) {
    this.url = url;
  }
  send(): void {
    throw new Error("a ping never sends");
  }
  close(code?: number, reason?: string): void {
    this.closed = { code, reason } as { code?: number; reason?: string };
  }
}

test("urls for a server address follow the page's scheme", () => {
  assert.equal(wsUrlFor("game.example.com:7777", "https:"), "wss://game.example.com:7777/ws");
  assert.equal(wsUrlFor("127.0.0.1:7777", "http:"), "ws://127.0.0.1:7777/ws");
  assert.equal(httpBaseFor("127.0.0.1:7777", "http:"), "http://127.0.0.1:7777");
  assert.equal(httpBaseFor("h:1", "https:"), "https://h:1");
});

test("a probe is half the WebSocket open time and closes without sending", async () => {
  let clock = 1000;
  let sock: FakeSocket | null = null;
  const p = probe("ws://h:1/ws", {
    socketFactory: (url) => (sock = new FakeSocket(url)),
    now: () => clock,
  });
  assert.ok(sock);
  const s = sock as FakeSocket;
  assert.equal(s.url, "ws://h:1/ws");
  clock += 84;
  s.onopen?.(new Event("open"));
  assert.equal(await p, 42);
  assert.deepEqual(s.closed, { code: 1000, reason: "ping" });
  // Errors and timeouts give null.
  let failing: FakeSocket | null = null;
  const q = probe("ws://h:2/ws", { socketFactory: (url) => (failing = new FakeSocket(url)) });
  (failing as unknown as FakeSocket).onerror?.(new Event("error"));
  assert.equal(await q, null);
  assert.equal(await probe("ws://h:3/ws", { socketFactory: (url) => new FakeSocket(url), timeoutMs: 5 }), null);
  assert.equal(
    await probe("ws://h:4/ws", {
      socketFactory: () => {
        throw new Error("blocked");
      },
    }),
    null,
  );
});

test("pingAll caps the probes in flight and reports every address once", async () => {
  let inFlight = 0;
  let peak = 0;
  const seen: string[] = [];
  const ping = async (a: string): Promise<number | null> => {
    inFlight++;
    peak = Math.max(peak, inFlight);
    await new Promise((r) => setTimeout(r, 2));
    inFlight--;
    if (a === "bad:1") throw new Error("nope");
    return a.length;
  };
  const addrs = ["a:1", "bb:2", "ccc:3", "bad:1", "a:1", "dddd:4", "eeeee:5"];
  const out = await pingAll(addrs, ping, 2, (a) => seen.push(a));
  assert.ok(peak <= 2, `peak ${peak}`);
  assert.equal(out.size, 6, "duplicates are pinged once");
  assert.equal(out.get("bb:2"), 4);
  assert.equal(out.get("bad:1"), null);
  assert.equal(seen.length, 6);
  assert.deepEqual(await pingAll([], ping), new Map());
  assert.equal(formatPing(undefined), "…");
  assert.equal(formatPing(null), "-");
  assert.equal(formatPing(12), "12 ms");
});

/** A fetch that records requests and answers from a table. */
function fakeFetch(answers: Record<string, { status?: number; body: unknown }>): { fetch: JsonFetch; calls: { url: string; method: string; body: unknown }[] } {
  const calls: { url: string; method: string; body: unknown }[] = [];
  const f: JsonFetch = async (url, init) => {
    calls.push({ url, method: init?.method ?? "GET", body: init?.body ? JSON.parse(init.body) : undefined });
    const path = url.replace(/^https?:\/\/[^/]+/, "").split("?")[0] ?? "";
    const a = answers[path];
    if (!a) throw new TypeError("failed to fetch");
    const status = a.status ?? 200;
    return { ok: status >= 200 && status < 300, status, json: async () => a.body };
  };
  return { fetch: f, calls };
}

test("the server API client posts JSON and surfaces errors", async () => {
  const { fetch, calls } = fakeFetch({
    "/api/presence": { body: { invites: [], address: "h:1", directory: null } },
    "/api/party": { body: { party: "P-ABC123", room: null, address: "h:1" } },
    "/api/duel": { status: 404, body: { error: "that player is not on this server" } },
    "/api/duel/answer": { body: { room: "R-000001", address: "h:1" } },
    "/api/rooms": { status: 503, body: null },
  });
  const api = new ServerApi("http://h:1/", fetch);
  assert.equal(api.base, "http://h:1");
  const r = await api.presence("p-0011223344556677", "t".repeat(32), "Ace");
  assert.equal(r.address, "h:1");
  assert.deepEqual(calls[0], {
    url: "http://h:1/api/presence",
    method: "POST",
    body: { id: "p-0011223344556677", token: "t".repeat(32), name: "Ace" },
  });
  assert.equal((await api.party({ room: null, player: 3 })).party, "P-ABC123");
  assert.deepEqual(calls[1]?.body, { room: null, player: 3 });
  await assert.rejects(api.duel({ id: "a", token: "b", name: "c", to: "d" }), (e: unknown) => {
    assert.ok(e instanceof ApiError);
    assert.equal(e.status, 404);
    assert.match(e.message, /not on this server/);
    return true;
  });
  assert.equal((await api.answer("p-a", "p-b", true)).room, "R-000001");
  assert.deepEqual(calls.at(-1)?.body, { from: "p-a", id: "p-b", accept: true });
  await assert.rejects(api.customRoom(null, null), /HTTP 503/);
  await assert.rejects(api.info(), /cannot reach/);
});

test("the directory client filters by build and relays invites as notes", async () => {
  const { fetch, calls } = fakeFetch({
    "/servers": { body: { servers: [{ id: "s1", age_ms: 5, server: { name: "A" } }] } },
    "/invites": {
      body: {
        invites: [
          {
            id: "i7",
            expires_in_ms: 25_000,
            invite: { to: "p-b", kind: "duel", from_name: "Ace", from_id: "p-a", address: "far:7777", world_code: "LS2" },
          },
        ],
      },
    },
  });
  const dir = new DirectoryApi("http://dir:7780", fetch);
  const servers = await dir.servers(5, "0123456789abcdef");
  assert.equal(servers.length, 1);
  assert.equal(calls[0]?.url, "http://dir:7780/servers?protocol=5&data_hash=0123456789abcdef");
  const invites = await dir.invites("p-b c");
  assert.equal(calls[1]?.url, "http://dir:7780/invites?player=p-b%20c");
  const note = noteFromRelay(invites[0]!);
  assert.deepEqual(note, {
    id: "dir-i7",
    kind: "duel",
    from_id: "p-a",
    from_name: "Ace",
    expires_in_ms: 25_000,
    address: "far:7777",
    world_code: "LS2",
  });
});

test("the inbox shows each note once until it expires", () => {
  const inbox = new NoteInbox();
  const n = (id: string, ms = 30_000): Note => ({ id, kind: "duel", from_id: "p-a", from_name: "A", expires_in_ms: ms });
  assert.deepEqual(
    inbox.fresh([n("c1"), n("c2")], 0).map((x) => x.id),
    ["c1", "c2"],
  );
  assert.deepEqual(inbox.fresh([n("c1"), n("c3")], 1000).map((x) => x.id), ["c3"]);
  // Long after expiry an id is forgotten (ids are unique per challenge anyway).
  assert.deepEqual(inbox.fresh([n("c1")], 10_000_000).map((x) => x.id), ["c1"]);
});
