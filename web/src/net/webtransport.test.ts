import assert from "node:assert/strict";
import { test } from "node:test";
import { connectTransport, transportKind, transportLabel, transportPreference, type FetchLike } from "./connect.ts";
import type { SocketLike } from "./transport.ts";
import {
  certHashUrl,
  frame,
  FrameDecoder,
  hexToBytes,
  parseCertHash,
  WebTransportTransport,
  webTransportUrl,
  type WebTransportFactory,
  type WebTransportLike,
  type WebTransportOptionsLike,
} from "./webtransport.ts";

const HEX = "ab".repeat(31) + "01";

test("cert hash JSON, URLs and hex", () => {
  const h = hexToBytes(HEX);
  assert.ok(h);
  assert.equal(h[0], 0xab);
  assert.equal(h[31], 0x01);
  assert.equal(hexToBytes("abc"), null);
  assert.equal(hexToBytes("zz".repeat(32)), null);

  const info = parseCertHash({ algorithm: "sha-256", hash: HEX, port: 7777, path: "/wt", expires: 123 });
  assert.ok(info);
  assert.deepEqual(Array.from(info.hash), Array.from(h));
  assert.equal(info.port, 7777);
  assert.equal(info.path, "/wt");
  assert.equal(info.expires, 123);
  assert.equal(parseCertHash({ algorithm: "md5", hash: HEX, port: 1 }), null);
  assert.equal(parseCertHash({ algorithm: "sha-256", hash: HEX, port: 0 }), null);
  assert.equal(parseCertHash({ algorithm: "sha-256", hash: HEX }), null);
  assert.equal(parseCertHash("nope"), null);
  assert.equal(parseCertHash({ algorithm: "sha-256", hash: HEX, port: 9 })?.path, "/wt", "default path");

  assert.equal(certHashUrl("ws://localhost:7777/ws"), "http://localhost:7777/cert-hash");
  assert.equal(certHashUrl("wss://game.example/ws"), "https://game.example/cert-hash");
  assert.equal(webTransportUrl("ws://10.0.0.2:7777/ws", info), "https://10.0.0.2:7777/wt");
  assert.equal(webTransportUrl("ws://[::1]:7777/ws", { ...info, port: 9000 }), "https://[::1]:9000/wt");
});

test("frames survive arbitrary chunking; oversized frames are refused", () => {
  const packets = [new Uint8Array([]), new Uint8Array([1]), new Uint8Array(300).map((_, i) => i & 0xff)];
  const wire = new Uint8Array(packets.reduce((n, p) => n + 4 + p.byteLength, 0));
  let at = 0;
  for (const p of packets) {
    const f = frame(p);
    wire.set(f, at);
    at += f.byteLength;
  }
  assert.deepEqual(Array.from(wire.slice(4, 9)), [0, 0, 0, 1, 1], "big-endian length");
  for (const step of [1, 3, 4, 7, wire.byteLength]) {
    const d = new FrameDecoder();
    const got: number[][] = [];
    for (let i = 0; i < wire.byteLength; i += step) {
      for (const f of d.push(wire.slice(i, i + step))) got.push(Array.from(f));
    }
    assert.deepEqual(got, packets.map((p) => Array.from(p)), `step ${step}`);
    assert.equal(d.pending, 0);
  }
  const small = new FrameDecoder(8);
  assert.throws(() => small.push(frame(new Uint8Array(9))), /exceeds/);
});

/** A fake browser WebTransport session backed by real web streams. */
class FakeWt implements WebTransportLike {
  readonly url: string;
  readonly options: WebTransportOptionsLike;
  ready: Promise<unknown>;
  closed: Promise<{ closeCode?: number; reason?: string } | undefined>;
  datagrams: { readable: ReadableStream<Uint8Array>; writable: WritableStream<Uint8Array>; maxDatagramSize: number };
  sentDatagrams: Uint8Array[] = [];
  sentStream: Uint8Array[] = [];
  closeCalls: { closeCode?: number; reason?: string }[] = [];
  streamClosed = false;
  private dgramCtl!: ReadableStreamDefaultController<Uint8Array>;
  private streamCtl!: ReadableStreamDefaultController<Uint8Array>;
  private resolveReady!: () => void;
  private rejectReady!: (e: Error) => void;
  private resolveClosed!: (info: { closeCode?: number; reason?: string }) => void;
  private rejectClosed!: (e: Error) => void;

  constructor(url: string, options: WebTransportOptionsLike, maxDatagramSize = 1000) {
    this.url = url;
    this.options = options;
    this.ready = new Promise((res, rej) => {
      this.resolveReady = () => res(undefined);
      this.rejectReady = rej;
    });
    this.closed = new Promise((res, rej) => {
      this.resolveClosed = res;
      this.rejectClosed = rej;
    });
    this.datagrams = {
      readable: new ReadableStream({ start: (c) => void (this.dgramCtl = c) }),
      writable: new WritableStream({ write: (chunk) => void this.sentDatagrams.push(chunk) }),
      maxDatagramSize,
    };
  }

  open(): void {
    this.resolveReady();
  }
  fail(msg: string): void {
    this.rejectReady(new Error(msg));
    this.rejectClosed(new Error(msg));
  }
  createBidirectionalStream(): Promise<{ readable: ReadableStream<Uint8Array>; writable: WritableStream<Uint8Array> }> {
    return Promise.resolve({
      readable: new ReadableStream({ start: (c) => void (this.streamCtl = c) }),
      writable: new WritableStream({
        write: (chunk) => void this.sentStream.push(chunk),
        close: () => void (this.streamClosed = true),
      }),
    });
  }
  close(info: { closeCode?: number; reason?: string } = {}): void {
    this.closeCalls.push(info);
  }
  serverStream(packet: number[]): void {
    this.streamCtl.enqueue(frame(new Uint8Array(packet)));
  }
  serverDatagram(packet: number[]): void {
    this.dgramCtl.enqueue(new Uint8Array(packet));
  }
  serverClose(reason: string): void {
    this.resolveClosed({ closeCode: 0, reason });
  }
}

const tick = (): Promise<void> => new Promise((r) => setTimeout(r, 5));

function fakeFactory(onCreate: (wt: FakeWt) => void): WebTransportFactory {
  return (url, options) => {
    const wt = new FakeWt(url, options);
    onCreate(wt);
    return wt;
  };
}

test("webtransport: handshake on the stream, datagrams after the Welcome, sends by channel", async () => {
  let fake: FakeWt | null = null;
  const pending = WebTransportTransport.connect("https://h:7777/wt", hexToBytes(HEX)!, fakeFactory((w) => (fake = w)), 1000);
  const wt = fake as unknown as FakeWt;
  assert.equal(wt.url, "https://h:7777/wt");
  assert.equal(wt.options.serverCertificateHashes[0]?.algorithm, "sha-256");
  assert.equal(wt.options.serverCertificateHashes[0]?.value.byteLength, 32);
  wt.open();
  const t = await pending;
  assert.equal(t.kind, "webtransport");
  assert.equal(transportLabel(t), "WebTransport");
  assert.ok(t.isOpen);

  // Hello: reliable -> framed on the stream.
  t.send(new Uint8Array([1, 2, 3]));
  await tick();
  assert.deepEqual(wt.sentStream.map((c) => Array.from(c)), [[0, 0, 0, 3, 1, 2, 3]]);

  // A datagram before the Welcome cannot be decoded: dropped.
  wt.serverDatagram([9]);
  await tick();
  wt.serverStream([7, 7]);
  const welcome = await t.nextPacket(500);
  assert.deepEqual(Array.from(welcome), [7, 7]);
  wt.serverDatagram([5]);
  wt.serverStream([6]);
  await tick();
  assert.deepEqual(t.drain().map((p) => Array.from(p)).sort(), [[5], [6]]);
  assert.equal(t.datagramsIn, 1);

  // Inputs (droppable) go as datagrams unless too large.
  t.send(new Uint8Array([4]), true);
  t.send(new Uint8Array(2000), true);
  await tick();
  assert.deepEqual(wt.sentDatagrams.map((c) => Array.from(c)), [[4]]);
  assert.equal(wt.sentStream.length, 2);
  assert.equal(wt.sentStream[1]?.byteLength, 2004);
  assert.equal(t.datagramsOut, 1);

  let closed = "";
  t.onClose = (r) => (closed = r);
  wt.serverClose("server shutting down");
  await tick();
  assert.equal(t.isOpen, false);
  assert.equal(closed, "server shutting down");
  t.send(new Uint8Array([1]), true);
  assert.equal(wt.sentDatagrams.length, 1, "no sends after close");
});

test("webtransport: client close flushes the stream, then closes the session", async () => {
  let fake: FakeWt | null = null;
  const pending = WebTransportTransport.connect("https://h:1/wt", hexToBytes(HEX)!, fakeFactory((w) => (fake = w)), 1000);
  const wt = fake as unknown as FakeWt;
  wt.open();
  const t = await pending;
  t.send(new Uint8Array([9]));
  t.close("bye");
  await tick();
  assert.equal(t.closeReason, "bye");
  assert.ok(wt.streamClosed);
  assert.equal(wt.sentStream.length, 1);
  assert.deepEqual(wt.closeCalls, [{ closeCode: 0, reason: "bye" }]);
  t.close("again");
  await tick();
  assert.equal(wt.closeCalls.length, 1);
});

test("webtransport: failed and silent sessions reject", async () => {
  let fake: FakeWt | null = null;
  const failing = WebTransportTransport.connect("https://h:1/wt", hexToBytes(HEX)!, fakeFactory((w) => (fake = w)), 1000);
  (fake as unknown as FakeWt).fail("certificate rejected");
  await assert.rejects(failing, /certificate rejected/);
  const silent = WebTransportTransport.connect("https://h:2/wt", hexToBytes(HEX)!, fakeFactory(() => {}), 20);
  await assert.rejects(silent, /no answer/);
  const throwing = WebTransportTransport.connect("https://h:3/wt", hexToBytes(HEX)!, () => {
    throw new Error("SyntaxError");
  });
  await assert.rejects(throwing, /cannot open/);
});

class OpenSocket implements SocketLike {
  binaryType: BinaryType = "blob";
  readyState = 0;
  bufferedAmount = 0;
  onopen: ((ev: Event) => unknown) | null = null;
  onmessage: ((ev: MessageEvent) => unknown) | null = null;
  onclose: ((ev: CloseEvent) => unknown) | null = null;
  onerror: ((ev: Event) => unknown) | null = null;
  readonly url: string;
  constructor(url: string) {
    this.url = url;
    setTimeout(() => {
      this.readyState = 1;
      this.onopen?.(new Event("open"));
    }, 1);
  }
  send(): void {}
  close(): void {
    this.readyState = 3;
  }
}

function certFetch(status = 200, body: unknown = { algorithm: "sha-256", hash: HEX, port: 7778, path: "/wt", expires: 0 }): FetchLike & { urls: string[] } {
  const urls: string[] = [];
  const f = (url: string) => {
    urls.push(url);
    return Promise.resolve({ ok: status === 200, status, json: () => Promise.resolve(body) });
  };
  return Object.assign(f, { urls });
}

test("connectTransport prefers WebTransport and falls back to WebSocket", async () => {
  const wsUrls: string[] = [];
  const socketFactory = (url: string): SocketLike => {
    wsUrls.push(url);
    return new OpenSocket(url);
  };
  const quiet = { onFallback: () => {} };

  // WebTransport available and offered.
  const fetch = certFetch();
  const opened: FakeWt[] = [];
  const t = await connectTransport("ws://example:7777/ws", {
    fetch,
    socketFactory,
    webTransportFactory: fakeFactory((w) => {
      opened.push(w);
      w.open();
    }),
  });
  assert.equal(transportKind(t), "webtransport");
  assert.deepEqual(fetch.urls, ["http://example:7777/cert-hash"]);
  assert.equal(opened[0]?.url, "https://example:7778/wt");
  assert.equal(wsUrls.length, 0);

  // No WebTransport in this browser.
  let reason = "";
  const a = await connectTransport("ws://example:7777/ws", {
    socketFactory,
    webTransportFactory: null,
    onFallback: (r) => (reason = r),
  });
  assert.equal(transportLabel(a), "WebSocket");
  assert.match(reason, /no WebTransport/);

  // Server without WebTransport (404).
  const b = await connectTransport("ws://example:7777/ws", {
    ...quiet,
    fetch: certFetch(404),
    socketFactory,
    webTransportFactory: fakeFactory((w) => w.open()),
  });
  assert.equal(b.kind, "websocket");

  // Session refused (e.g. certificate).
  const c = await connectTransport("ws://example:7777/ws", {
    ...quiet,
    fetch: certFetch(),
    socketFactory,
    webTransportFactory: fakeFactory((w) => w.fail("handshake failed")),
  });
  assert.equal(c.kind, "websocket");

  // Garbage from /cert-hash.
  const d = await connectTransport("ws://example:7777/ws", {
    ...quiet,
    fetch: certFetch(200, { hello: 1 }),
    socketFactory,
    webTransportFactory: fakeFactory((w) => w.open()),
  });
  assert.equal(d.kind, "websocket");

  // ?transport=ws skips WebTransport entirely.
  const wsOnly = certFetch();
  const e = await connectTransport("ws://example:7777/ws", {
    prefer: "ws",
    fetch: wsOnly,
    socketFactory,
    webTransportFactory: fakeFactory((w) => w.open()),
  });
  assert.equal(e.kind, "websocket");
  assert.equal(wsOnly.urls.length, 0);

  // ?transport=wt does not fall back.
  await assert.rejects(
    connectTransport("ws://example:7777/ws", { prefer: "wt", fetch: certFetch(404), socketFactory, webTransportFactory: fakeFactory(() => {}) }),
    /WebTransport: the server does not offer WebTransport/,
  );
  assert.equal(wsUrls.length, 5);
});

test("transport preference and labels", () => {
  assert.equal(transportPreference({ search: "" }), "auto");
  assert.equal(transportPreference({ search: "?transport=ws" }), "ws");
  assert.equal(transportPreference({ search: "?server=x:1&transport=WT" }), "wt");
  assert.equal(transportPreference({ search: "?transport=webtransport" }), "wt");
  assert.equal(transportPreference({ search: "?transport=carrier-pigeon" }), "auto");
  assert.equal(transportLabel(null), "offline");
  assert.equal(transportKind(undefined), null);
});
