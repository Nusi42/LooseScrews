import assert from "node:assert/strict";
import { test } from "node:test";
import type { GameHandle, WasmApi } from "../wasm/api.ts";
import {
  alreadyInRoom,
  answeredDuels,
  rememberAnswered,
  duelTargets,
  IDENTITY_KEY,
  inviteLink,
  loadIdentity,
  noteText,
  parseJoin,
  serverAddress,
  sessionView,
  withJoinCodes,
  type KeyValueStore,
} from "./party.ts";
import { modeName, serverRows } from "./serverList.ts";
import type { Note, ServerEntry } from "../net/social.ts";

const PAGE = "https://play.example.com/?transport=ws&editor=x";

test("invite links carry server, party, room, world and rules", () => {
  const link = inviteLink(PAGE, {
    address: "eu.example.com:7777",
    party: "P-ABC123",
    world: "LS08-0000",
    rules: "LR8V-6097",
  });
  const u = new URL(link);
  assert.equal(u.origin + u.pathname, "https://play.example.com/");
  assert.equal(u.searchParams.get("join"), "eu.example.com:7777");
  assert.equal(u.searchParams.get("party"), "P-ABC123");
  assert.equal(u.searchParams.get("code"), "LS08-0000");
  assert.equal(u.searchParams.get("rules"), "LR8V-6097");
  assert.equal(u.searchParams.get("transport"), "ws", "dev parameters stay");
  assert.equal(u.searchParams.get("editor"), null, "others go");
  assert.deepEqual(parseJoin(link), {
    address: "eu.example.com:7777",
    party: "P-ABC123",
    room: null,
    world: "LS08-0000",
    rules: "LR8V-6097",
    directory: null,
    servers: false,
  });
});

test("invite parameters are read leniently and checked", () => {
  const j = parseJoin("http://h/?join=127.0.0.1:7777&party=p-abc123&room=r-0000ab&world=ls08-1&directory=http://dir:7780&servers");
  assert.equal(j.address, "127.0.0.1:7777");
  assert.equal(j.party, "P-ABC123");
  assert.equal(j.room, "R-0000AB");
  assert.equal(j.world, "LS08-1", "the editor's world= works as code=");
  assert.equal(j.directory, "http://dir:7780");
  assert.equal(j.servers, true);
  const bad = parseJoin("http://h/?join=evil host/x&party=<script>&directory=javascript:alert(1)");
  assert.equal(bad.address, null);
  assert.equal(bad.party, null);
  assert.equal(bad.directory, null);
  assert.deepEqual(parseJoin("http://h/"), {
    address: null,
    party: null,
    room: null,
    world: null,
    rules: null,
    directory: null,
    servers: false,
  });
});

test("the server address comes from join=, ?server= or the page", () => {
  const none = parseJoin("http://h/");
  assert.equal(serverAddress({ host: "page:5173", search: "" }, none), "page:5173");
  assert.equal(serverAddress({ host: "page:5173", search: "?server=127.0.0.1:7777" }, none), "127.0.0.1:7777");
  assert.equal(serverAddress({ host: "page:5173", search: "?server=ws://10.0.0.2:7000/ws" }, none), "10.0.0.2:7000");
  assert.equal(serverAddress({ host: "page", search: "?server=x:1" }, parseJoin("http://h/?join=far:9")), "far:9");
});

/** A Map-backed store; `failing` throws on writes. */
function store(failing = false): KeyValueStore & { data: Map<string, string> } {
  const data = new Map<string, string>();
  return {
    data,
    getItem: (k) => data.get(k) ?? null,
    setItem: (k, v) => {
      if (failing) throw new Error("quota");
      data.set(k, v);
    },
  };
}

test("the identity is created once and kept", () => {
  let n = 0;
  const random = (bytes: number): string => (n++ % 16).toString(16).repeat(bytes * 2);
  const s = store();
  const a = loadIdentity(s, random);
  assert.match(a.id, /^p-[0-9a-f]{16}$/);
  assert.match(a.token, /^[0-9a-f]{32}$/);
  assert.notEqual(a.id.slice(2), a.token.slice(0, 16));
  assert.deepEqual(loadIdentity(s, random), a, "stored");
  s.data.set(IDENTITY_KEY, '{"id":"bad","token":"x"}');
  assert.notDeepEqual(loadIdentity(s, random), a, "garbage is replaced");
  // Without storage it still works (this tab only).
  const b = loadIdentity(store(true), random);
  assert.match(b.id, /^p-/);
  assert.match(loadIdentity(null).token, /^[0-9a-f]{32}$/);
});

test("hello packets carry the room and party codes", () => {
  const calls: unknown[][] = [];
  const api = {
    helloPacket: () => new Uint8Array([1]),
    helloPacketAs: () => new Uint8Array([2]),
    helloPacketWith: (...args: unknown[]) => {
      calls.push(args);
      return new Uint8Array([3]);
    },
  } as unknown as WasmApi;
  let room: string | null = null;
  const w = withJoinCodes(api, () => room, () => null);
  assert.deepEqual(w.helloPacket("Ace", -1), new Uint8Array([1]), "plain without codes");
  assert.deepEqual(w.helloPacketAs?.("Ace", 1, 2), new Uint8Array([2]));
  // The room code is read when the Hello is built (a custom match opened late).
  room = "R-000001";
  assert.deepEqual(w.helloPacket("Ace", -1), new Uint8Array([3]));
  assert.deepEqual(w.helloPacketAs?.("Ace", 1, 2), new Uint8Array([3]));
  assert.deepEqual(calls, [
    ["Ace", -1, 255, "R-000001", ""],
    ["Ace", 1, 2, "R-000001", ""],
  ]);
  withJoinCodes(api, () => null, () => "P-ABC123").helloPacket("Bea", -1);
  assert.deepEqual(calls.at(-1), ["Bea", -1, 255, "", "P-ABC123"]);
  // An old core without helloPacketWith: plain Hellos.
  const old = { helloPacket: () => new Uint8Array([1]) } as unknown as WasmApi;
  assert.equal(withJoinCodes(old, () => "R-1", () => "P-2"), old);
});

test("notes, duel targets and the session view", () => {
  const base = { id: "c1", from_id: "p-a", from_name: "Ace", expires_in_ms: 1 };
  assert.equal(noteText({ ...base, kind: "duel" }), "Ace challenges you to a duel");
  assert.match(noteText({ ...base, kind: "accepted", room: "R-1" }), /accepted/);
  assert.match(noteText({ ...base, kind: "declined" }), /declined/);
  const roster = [
    { id: "p-me", name: "Me" },
    { id: "p-z", name: "Zed" },
    { id: "p-a", name: "Ace" },
  ];
  assert.deepEqual(
    duelTargets(roster, "p-me").map((p) => p.name),
    ["Ace", "Zed"],
  );
  const game = { player: 3, shareCode: "LS08-1", roomCode: "R-000009" } as unknown as GameHandle;
  assert.deepEqual(sessionView(game), { player: 3, room: "R-000009", shareCode: "LS08-1" });
  const old = { player: 1, shareCode: "LS" } as unknown as GameHandle;
  assert.equal(sessionView(old).room, "");
});

function entry(name: string, address: string, players: number, extra: Partial<ServerEntry["server"]> = {}): ServerEntry {
  return {
    id: name,
    age_ms: 1,
    server: {
      name,
      address,
      region: "eu",
      mode: "team_brawl",
      map_code: "LS08",
      players,
      max_players: 8,
      protocol: 5,
      data_hash: "0123456789abcdef",
      ...extra,
    },
  };
}

test("server list rows: build filter, order, ping and mode names", () => {
  const entries = [
    entry("Quiet", "q:1", 0),
    entry("Busy", "b:1", 5, { roster: [{ id: "p-x", name: "X" }] }),
    entry("Near", "n:1", 0, { mode: "the_squeeze", region: "" }),
    entry("Other build", "o:1", 7, { data_hash: "ffffffffffffffff" }),
    entry("Full", "f:1", 8),
  ];
  const pings = new Map<string, number | null>([
    ["q:1", 80],
    ["n:1", 20],
    ["b:1", null],
  ]);
  const rows = serverRows(entries, pings, { protocol: 5, dataHash: "0123456789abcdef" });
  assert.deepEqual(
    rows.map((r) => r.name),
    ["Full", "Busy", "Near", "Quiet"],
  );
  const busy = rows[1]!;
  assert.equal(busy.players, "5/8");
  assert.equal(busy.ping, "-");
  assert.equal(busy.mode, "Team Brawl");
  assert.deepEqual(busy.roster, [{ id: "p-x", name: "X" }]);
  assert.equal(rows[0]!.full, true);
  assert.equal(rows[2]!.mode, "The Squeeze");
  assert.equal(rows[2]!.region, "-");
  assert.equal(rows[2]!.ping, "20 ms");
  assert.equal(rows[3]!.ping, "80 ms");
  assert.equal(serverRows(entries, new Map()).length, 5, "no filter: everything");
  assert.equal(serverRows([entry("A", "a:1", 0)], new Map())[0]!.ping, "…");
  assert.equal(modeName("lobby"), "lobby");
  assert.equal(modeName(""), "-");
});

test("an accepted duel is not followed again from inside its room", () => {
  const note: Note = {
    id: "c1-accepted",
    kind: "accepted",
    from_id: "p-b",
    from_name: "Bea",
    room: "R-ABC123",
    address: "127.0.0.1:7777",
    expires_in_ms: 20_000,
  };
  const page = "http://127.0.0.1:7777/?join=127.0.0.1%3A7777&room=R-ABC123";
  assert.equal(alreadyInRoom(note, page, null), true, "the invite link's room");
  assert.equal(alreadyInRoom(note, "http://127.0.0.1:7777/", "r-abc123"), true, "the session's room");
  assert.equal(alreadyInRoom(note, "http://127.0.0.1:7777/?servers", ""), false, "elsewhere: follow it");
  const { room: _room, ...roomless } = note;
  assert.equal(alreadyInRoom(roomless, page, null), false);
});

test("answered challenges are remembered per tab until they expire", () => {
  const data = new Map<string, string>();
  const store: KeyValueStore = { getItem: (k) => data.get(k) ?? null, setItem: (k, v) => void data.set(k, v) };
  rememberAnswered(store, "c1", 2000, 0);
  rememberAnswered(store, "c2", 500, 0);
  assert.deepEqual([...answeredDuels(store, 1000).keys()], ["c1"], "c2 expired");
  assert.equal(answeredDuels(null, 0).size, 0);
  data.set("ls.answeredDuels", "not json");
  assert.equal(answeredDuels(store, 0).size, 0);
});
