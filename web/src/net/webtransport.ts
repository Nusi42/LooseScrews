// WebTransport transport to ls-server (HTTP/3 over QUIC on a UDP port).
//
// The server's certificate is self-signed; it is trusted by its SHA-256 hash
// (`serverCertificateHashes`), which the server publishes at GET /cert-hash
// on its HTTP port together with the WebTransport port and path:
//   {"algorithm":"sha-256","hash":"<64 hex>","port":7777,"path":"/wt","expires":<unix s>}
//
// Wire mapping (see crates/ls-server/src/wt.rs):
// - reliable packets: one bidirectional stream opened by the client; every
//   packet is a frame of a 4-byte big-endian length plus the encoded ls-net
//   Message. The first frame is the Hello, the server answers on the stream.
// - unreliable packets (inputs, pings; snapshots and pongs from the server)
//   are datagrams, one Message each. Packets larger than the datagram limit
//   go on the stream. Datagrams that arrive before the first stream packet
//   (the Welcome) are dropped: they cannot be decoded before it.
//
// The class has the same surface as WebSocketTransport, so the session does
// not care which one it got (see connect.ts).

import { MAX_BUFFERED_BYTES, TransportError } from "./transport.ts";

/** URL path of the WebTransport session on the server. */
export const WT_PATH = "/wt";

/** Stream frame header: 4-byte big-endian length. */
export const FRAME_HEADER = 4;

/** Largest frame accepted from the server (a Welcome with a long edit log). */
export const MAX_SERVER_FRAME = 32 * 1024 * 1024;

/** Datagram limit assumed when the browser does not report one (QUIC minimum). */
const FALLBACK_MAX_DATAGRAM = 1100;

/** What GET /cert-hash says. */
export interface CertHashInfo {
  /** SHA-256 of the server certificate (32 bytes). */
  hash: Uint8Array;
  /** UDP port of the WebTransport endpoint. */
  port: number;
  /** Session path, e.g. "/wt". */
  path: string;
  /** End of the certificate's validity, Unix seconds. */
  expires: number;
}

/** 64 hex characters to 32 bytes; null if malformed. */
export function hexToBytes(hex: string): Uint8Array | null {
  if (!/^[0-9a-fA-F]{64}$/.test(hex)) return null;
  const out = new Uint8Array(32);
  for (let i = 0; i < 32; i++) out[i] = Number.parseInt(hex.slice(2 * i, 2 * i + 2), 16);
  return out;
}

/** Validates the JSON body of GET /cert-hash. */
export function parseCertHash(json: unknown): CertHashInfo | null {
  if (typeof json !== "object" || json === null) return null;
  const o = json as Record<string, unknown>;
  if (o["algorithm"] !== "sha-256" || typeof o["hash"] !== "string") return null;
  const hash = hexToBytes(o["hash"]);
  const port = o["port"];
  if (!hash || typeof port !== "number" || !Number.isInteger(port) || port < 1 || port > 65535) return null;
  const path = typeof o["path"] === "string" && o["path"].startsWith("/") ? o["path"] : WT_PATH;
  const expires = typeof o["expires"] === "number" ? o["expires"] : 0;
  return { hash, port, path, expires };
}

/**
 * The HTTP URL of /cert-hash on the server a WebSocket URL points at:
 * `ws://host:7777/ws` -> `http://host:7777/cert-hash` (wss -> https).
 */
export function certHashUrl(wsUrl: string): string {
  const u = new URL(wsUrl);
  const scheme = u.protocol === "wss:" || u.protocol === "https:" ? "https:" : "http:";
  return `${scheme}//${u.host}/cert-hash`;
}

/** The WebTransport URL: the WebSocket URL's host with the published port and path. */
export function webTransportUrl(wsUrl: string, info: CertHashInfo): string {
  const host = new URL(wsUrl).hostname; // IPv6 stays bracketed
  return `https://${host}:${info.port}${info.path}`;
}

/** Appends a length prefix. */
export function frame(packet: Uint8Array): Uint8Array {
  const out = new Uint8Array(FRAME_HEADER + packet.byteLength);
  new DataView(out.buffer).setUint32(0, packet.byteLength, false);
  out.set(packet, FRAME_HEADER);
  return out;
}

/** Reassembles length-prefixed frames from stream chunks of any size. */
export class FrameDecoder {
  private buf = new Uint8Array(0);
  private readonly max: number;

  constructor(max = MAX_SERVER_FRAME) {
    this.max = max;
  }

  /** Adds a chunk and returns the frames it completed. Throws on an oversized frame. */
  push(chunk: Uint8Array): Uint8Array[] {
    if (this.buf.byteLength === 0) {
      this.buf = chunk.slice();
    } else {
      const joined = new Uint8Array(this.buf.byteLength + chunk.byteLength);
      joined.set(this.buf);
      joined.set(chunk, this.buf.byteLength);
      this.buf = joined;
    }
    const frames: Uint8Array[] = [];
    let at = 0;
    const view = new DataView(this.buf.buffer, this.buf.byteOffset, this.buf.byteLength);
    while (this.buf.byteLength - at >= FRAME_HEADER) {
      const len = view.getUint32(at, false);
      if (len > this.max) throw new TransportError(`stream frame of ${len} bytes exceeds ${this.max}`);
      if (this.buf.byteLength - at - FRAME_HEADER < len) break;
      frames.push(this.buf.slice(at + FRAME_HEADER, at + FRAME_HEADER + len));
      at += FRAME_HEADER + len;
    }
    this.buf = this.buf.slice(at);
    return frames;
  }

  /** Bytes buffered but not yet returned. */
  get pending(): number {
    return this.buf.byteLength;
  }
}

// --- the part of the browser WebTransport API the transport uses -------------

export interface BidiStreamLike {
  readable: ReadableStream<Uint8Array>;
  writable: WritableStream<Uint8Array>;
}

export interface WebTransportLike {
  readonly ready: Promise<unknown>;
  readonly closed: Promise<{ closeCode?: number; reason?: string } | undefined>;
  readonly datagrams: {
    readonly readable: ReadableStream<Uint8Array>;
    readonly writable: WritableStream<Uint8Array>;
    readonly maxDatagramSize?: number;
  };
  createBidirectionalStream(): Promise<BidiStreamLike>;
  close(info?: { closeCode?: number; reason?: string }): void;
}

export interface WebTransportOptionsLike {
  serverCertificateHashes: { algorithm: "sha-256"; value: Uint8Array }[];
}

export type WebTransportFactory = (url: string, options: WebTransportOptionsLike) => WebTransportLike;

/** The browser's WebTransport, or null where it is missing (old browsers, non-secure pages, Node). */
export function browserWebTransport(): WebTransportFactory | null {
  const ctor = (globalThis as { WebTransport?: new (url: string, options: WebTransportOptionsLike) => WebTransportLike })
    .WebTransport;
  if (typeof ctor !== "function") return null;
  return (url, options) => new ctor(url, options);
}

export class WebTransportTransport {
  readonly kind = "webtransport";
  private readonly wt: WebTransportLike;
  private readonly streamWriter: WritableStreamDefaultWriter<Uint8Array>;
  private readonly datagramWriter: WritableStreamDefaultWriter<Uint8Array>;
  private readonly inbox: Uint8Array[] = [];
  private closedReason: string | null = null;
  /** The first stream packet (the Welcome) arrived; datagrams are usable from now on. */
  private welcomed = false;
  /** Bytes handed to the stream but not yet written. */
  private pendingStreamBytes = 0;
  /** Called once when the connection closes. */
  onClose: ((reason: string) => void) | null = null;
  bytesIn = 0;
  bytesOut = 0;
  /** Droppable packets skipped under backpressure. */
  droppedPackets = 0;
  /** Packets sent as datagrams (the rest went on the stream). */
  datagramsOut = 0;
  datagramsIn = 0;

  private constructor(wt: WebTransportLike, stream: BidiStreamLike) {
    this.wt = wt;
    this.streamWriter = stream.writable.getWriter();
    this.datagramWriter = wt.datagrams.writable.getWriter();
    void this.readStream(stream.readable);
    void this.readDatagrams(wt.datagrams.readable);
    wt.closed.then(
      (info) => this.markClosed(info?.reason || `connection closed (code ${info?.closeCode ?? 0})`),
      () => this.markClosed("connection lost"),
    );
  }

  /**
   * Opens a session at `url` trusting the certificate with `certHash` and
   * opens the packet stream. Rejects on failure or timeout.
   */
  static async connect(
    url: string,
    certHash: Uint8Array,
    factory: WebTransportFactory,
    timeoutMs = 5000,
  ): Promise<WebTransportTransport> {
    let wt: WebTransportLike;
    try {
      wt = factory(url, { serverCertificateHashes: [{ algorithm: "sha-256", value: certHash }] });
    } catch (err) {
      throw new TransportError(`cannot open ${url}: ${err instanceof Error ? err.message : String(err)}`);
    }
    // A failed session rejects `closed` too; keep that from surfacing as unhandled.
    wt.closed.catch(() => undefined);
    let timer: ReturnType<typeof setTimeout> | undefined;
    const timeout = new Promise<never>((_, reject) => {
      timer = setTimeout(() => reject(new TransportError(`no answer from ${url}`)), timeoutMs);
    });
    try {
      const stream = await Promise.race([
        wt.ready.then(() => wt.createBidirectionalStream()),
        timeout,
      ]);
      return new WebTransportTransport(wt, stream);
    } catch (err) {
      try {
        wt.close();
      } catch {
        /* never opened */
      }
      if (err instanceof TransportError) throw err;
      throw new TransportError(`cannot connect to ${url}: ${err instanceof Error ? err.message : String(err)}`);
    } finally {
      clearTimeout(timer);
    }
  }

  get isOpen(): boolean {
    return this.closedReason === null;
  }

  get closeReason(): string | null {
    return this.closedReason;
  }

  /** Current datagram limit. */
  get maxDatagramSize(): number {
    return this.wt.datagrams.maxDatagramSize ?? FALLBACK_MAX_DATAGRAM;
  }

  /**
   * Sends one packet. `droppable` packets (inputs, pings) are the
   * unreliable ones: they go as datagrams (the stream if too large) and are
   * skipped while the stream or datagram queue is backed up. Everything else
   * (the Hello) goes on the stream and is never dropped. Contract: callers
   * set `droppable` only for ls-net `Channel::Unreliable` messages (Input,
   * Ping); a droppable reliable message would become lossy here while
   * staying reliable over WebSocket.
   */
  send(packet: Uint8Array, droppable = false): void {
    if (!this.isOpen) return;
    if (droppable && packet.byteLength <= this.maxDatagramSize) {
      const desired = this.datagramWriter.desiredSize;
      if (desired !== null && desired <= 0) {
        this.droppedPackets += 1;
        return;
      }
      this.bytesOut += packet.byteLength;
      this.datagramsOut += 1;
      // Datagram writes only fail once the session is gone (seen via `closed`).
      this.datagramWriter.write(packet).catch(() => undefined);
      return;
    }
    if (droppable && this.pendingStreamBytes > MAX_BUFFERED_BYTES) {
      this.droppedPackets += 1;
      return;
    }
    const framed = frame(packet);
    this.bytesOut += packet.byteLength;
    this.pendingStreamBytes += framed.byteLength;
    this.streamWriter.write(framed).then(
      () => {
        this.pendingStreamBytes -= framed.byteLength;
      },
      () => this.markClosed("stream write failed"),
    );
  }

  /** Takes every packet received since the last call, oldest first. */
  drain(): Uint8Array[] {
    return this.inbox.splice(0, this.inbox.length);
  }

  /** Resolves with the next packet (used for the handshake). */
  nextPacket(timeoutMs = 10000): Promise<Uint8Array> {
    return new Promise((resolve, reject) => {
      const started = Date.now();
      const poll = (): void => {
        const p = this.inbox.shift();
        if (p) return resolve(p);
        if (this.closedReason !== null) return reject(new TransportError(this.closedReason));
        if (Date.now() - started > timeoutMs) return reject(new TransportError("the server did not answer"));
        setTimeout(poll, 10);
      };
      poll();
    });
  }

  close(reason = "client closed"): void {
    const wasOpen = this.closedReason === null;
    this.markClosed(reason);
    if (!wasOpen) return;
    // Let queued stream data (a final Disconnect) go out, then close.
    this.streamWriter.close().catch(() => undefined).finally(() => {
      try {
        this.wt.close({ closeCode: 0, reason: reason.slice(0, 120) });
      } catch {
        /* already closed */
      }
    });
  }

  private async readStream(readable: ReadableStream<Uint8Array>): Promise<void> {
    const reader = readable.getReader();
    const decoder = new FrameDecoder();
    try {
      for (;;) {
        const { value, done } = await reader.read();
        if (done) break;
        this.bytesIn += value.byteLength;
        for (const f of decoder.push(value)) {
          this.welcomed = true;
          this.inbox.push(f);
        }
      }
      this.markClosed("the server closed the connection");
    } catch (err) {
      this.markClosed(err instanceof TransportError ? err.message : "connection lost");
    }
  }

  private async readDatagrams(readable: ReadableStream<Uint8Array>): Promise<void> {
    const reader = readable.getReader();
    try {
      for (;;) {
        const { value, done } = await reader.read();
        if (done) return;
        this.bytesIn += value.byteLength;
        if (!this.welcomed) continue;
        this.datagramsIn += 1;
        this.inbox.push(value);
      }
    } catch {
      /* the session ended; `closed` reports why */
    }
  }

  private markClosed(reason: string): void {
    if (this.closedReason !== null) return;
    this.closedReason = reason;
    this.onClose?.(reason);
  }
}
