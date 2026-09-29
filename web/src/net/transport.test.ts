import assert from "node:assert/strict";
import { test } from "node:test";
import { MAX_BUFFERED_BYTES, serverUrl, WebSocketTransport, type SocketLike } from "./transport.ts";

test("server URL follows the page (ws/wss) and honours ?server=", () => {
  assert.equal(serverUrl({ protocol: "http:", host: "localhost:7777", search: "" }), "ws://localhost:7777/ws");
  assert.equal(serverUrl({ protocol: "https:", host: "game.example", search: "" }), "wss://game.example/ws");
  assert.equal(serverUrl({ protocol: "http:", host: "localhost:5173", search: "?server=10.0.0.2:7777" }), "ws://10.0.0.2:7777/ws");
  assert.equal(serverUrl({ protocol: "http:", host: "x", search: "?server=wss://other/ws2" }), "wss://other/ws2");
});

class FakeSocket implements SocketLike {
  binaryType: BinaryType = "blob";
  readyState = 0;
  bufferedAmount = 0;
  sent: Uint8Array[] = [];
  closed = false;
  onopen: ((ev: Event) => unknown) | null = null;
  onmessage: ((ev: MessageEvent) => unknown) | null = null;
  onclose: ((ev: CloseEvent) => unknown) | null = null;
  onerror: ((ev: Event) => unknown) | null = null;
  readonly url: string;
  constructor(url: string) {
    this.url = url;
  }
  send(data: Uint8Array): void {
    this.sent.push(data);
  }
  close(): void {
    this.closed = true;
    this.readyState = 3;
  }
  open(): void {
    this.readyState = 1;
    this.onopen?.(new Event("open"));
  }
  receive(bytes: number[]): void {
    this.onmessage?.({ data: new Uint8Array(bytes).buffer } as MessageEvent);
  }
  drop(reason: string): void {
    this.readyState = 3;
    this.onclose?.({ code: 1006, reason } as CloseEvent);
  }
}

test("transport: connects, queues binary packets, sends, reports close", async () => {
  let socket: FakeSocket | null = null;
  const pending = WebSocketTransport.connect("ws://h/ws", (url) => (socket = new FakeSocket(url)), 1000);
  assert.ok(socket);
  const s = socket as FakeSocket;
  assert.equal(s.binaryType, "arraybuffer");
  s.open();
  const t = await pending;
  assert.ok(t.isOpen);

  s.receive([1, 2]);
  s.onmessage?.({ data: "text frames are ignored" } as MessageEvent);
  s.receive([3]);
  const first = await t.nextPacket(100);
  assert.deepEqual(Array.from(first), [1, 2]);
  assert.deepEqual(t.drain().map((p) => Array.from(p)), [[3]]);
  assert.deepEqual(t.drain(), []);

  t.send(new Uint8Array([9, 9]));
  assert.equal(s.sent.length, 1);
  assert.equal(t.bytesOut, 2);

  let closed = "";
  t.onClose = (r) => (closed = r);
  s.drop("server shut down");
  assert.equal(t.isOpen, false);
  assert.equal(closed, "server shut down");
  assert.equal(t.closeReason, "server shut down");
  t.send(new Uint8Array([1]));
  assert.equal(s.sent.length, 1, "no sends after close");
  await assert.rejects(t.nextPacket(50), /server shut down/);
});

test("transport: droppable packets are skipped while the send buffer is full", async () => {
  let socket: FakeSocket | null = null;
  const pending = WebSocketTransport.connect("ws://h/ws", (url) => (socket = new FakeSocket(url)), 1000);
  const s = socket as unknown as FakeSocket;
  s.open();
  const t = await pending;
  s.bufferedAmount = MAX_BUFFERED_BYTES + 1;
  t.send(new Uint8Array([1]), true);
  assert.equal(s.sent.length, 0, "input dropped under backpressure");
  assert.equal(t.droppedPackets, 1);
  t.send(new Uint8Array([2]));
  assert.equal(s.sent.length, 1, "non-droppable packets (Hello) always go out");
  s.bufferedAmount = 0;
  t.send(new Uint8Array([3]), true);
  assert.equal(s.sent.length, 2);
  assert.equal(t.droppedPackets, 1);
});

test("transport: failed connections and timeouts reject", async () => {
  let socket: FakeSocket | null = null;
  const failing = WebSocketTransport.connect("ws://nowhere/ws", (url) => (socket = new FakeSocket(url)), 1000);
  (socket as unknown as FakeSocket).drop("refused");
  await assert.rejects(failing, /cannot connect/);

  const silent = WebSocketTransport.connect("ws://slow/ws", (url) => new FakeSocket(url), 20);
  await assert.rejects(silent, /no answer/);

  const throwing = WebSocketTransport.connect("bad url", () => {
    throw new Error("SyntaxError");
  });
  await assert.rejects(throwing, /cannot open/);
});
