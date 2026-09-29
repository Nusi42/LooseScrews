// WebSocket transport to ls-server: one binary WebSocket message per encoded
// ls-net Message, both directions. Incoming packets are queued and drained by
// the frame loop (the poll-based shape of ls-net's `Transport` trait).
// WebSocket is reliable and ordered, so "unreliable" packets go the same way.
//
// Send-side backpressure: when the uplink stalls, packets pile up in the
// browser's socket buffer and later arrive in a burst, too late to be used.
// Droppable packets (inputs and pings, which the frame loop resends anyway:
// every Input carries the newest three commands) are skipped while more than
// MAX_BUFFERED_BYTES wait; the handshake is never dropped.

/** Path of the game endpoint on the server. */
export const WS_PATH = "/ws";

/** Buffered outgoing bytes above which droppable packets are skipped. */
export const MAX_BUFFERED_BYTES = 16 * 1024;

/**
 * WebSocket URL for the page location: `ws://host/ws` over http,
 * `wss://host/ws` over https. `?server=host:port` (or a full ws:// URL)
 * overrides the host, e.g. to play from the Vite dev server against another
 * machine.
 */
export function serverUrl(loc: { protocol: string; host: string; search: string }): string {
  const override = new URLSearchParams(loc.search).get("server");
  const scheme = loc.protocol === "https:" ? "wss:" : "ws:";
  if (override) {
    if (/^wss?:\/\//i.test(override)) return override;
    return `${scheme}//${override}${WS_PATH}`;
  }
  return `${scheme}//${loc.host}${WS_PATH}`;
}

/** The part of the WebSocket API the transport uses (lets tests inject a fake). */
export interface SocketLike {
  binaryType: BinaryType;
  readonly readyState: number;
  /** Bytes queued by `send` but not yet transmitted. */
  readonly bufferedAmount: number;
  send(data: Uint8Array): void;
  close(code?: number, reason?: string): void;
  onopen: ((ev: Event) => unknown) | null;
  onmessage: ((ev: MessageEvent) => unknown) | null;
  onclose: ((ev: CloseEvent) => unknown) | null;
  onerror: ((ev: Event) => unknown) | null;
}

export type SocketFactory = (url: string) => SocketLike;

const OPEN = 1;

export class TransportError extends Error {
  override name = "TransportError";
}

export class WebSocketTransport {
  private readonly socket: SocketLike;
  private readonly inbox: Uint8Array[] = [];
  private closedReason: string | null = null;
  /** Called once when the connection closes (after the inbox is drained by the caller). */
  onClose: ((reason: string) => void) | null = null;
  bytesIn = 0;
  bytesOut = 0;
  /** Droppable packets skipped under backpressure. */
  droppedPackets = 0;

  private constructor(socket: SocketLike) {
    this.socket = socket;
    socket.onmessage = (ev) => {
      const data: unknown = ev.data;
      if (data instanceof ArrayBuffer) {
        this.bytesIn += data.byteLength;
        this.inbox.push(new Uint8Array(data));
      }
      // Text frames are not part of the protocol; ignore them.
    };
    socket.onclose = (ev) => this.markClosed(ev.reason || `connection closed (code ${ev.code})`);
    socket.onerror = () => this.markClosed("connection error");
  }

  /** Opens `url` and resolves once connected (rejects on failure or timeout). */
  static connect(url: string, factory: SocketFactory = defaultFactory, timeoutMs = 8000): Promise<WebSocketTransport> {
    return new Promise((resolve, reject) => {
      let socket: SocketLike;
      try {
        socket = factory(url);
      } catch (err) {
        reject(new TransportError(`cannot open ${url}: ${err instanceof Error ? err.message : String(err)}`));
        return;
      }
      socket.binaryType = "arraybuffer";
      const timer = setTimeout(() => {
        socket.close();
        reject(new TransportError(`no answer from ${url}`));
      }, timeoutMs);
      socket.onopen = () => {
        clearTimeout(timer);
        socket.onopen = null;
        resolve(new WebSocketTransport(socket));
      };
      const fail = (): void => {
        clearTimeout(timer);
        reject(new TransportError(`cannot connect to ${url}`));
      };
      socket.onerror = fail;
      socket.onclose = fail;
    });
  }

  get isOpen(): boolean {
    return this.closedReason === null && this.socket.readyState === OPEN;
  }

  get closeReason(): string | null {
    return this.closedReason;
  }

  /**
   * Sends one packet. `droppable` packets (inputs, pings) are skipped while
   * the socket's send buffer holds more than MAX_BUFFERED_BYTES.
   */
  send(packet: Uint8Array, droppable = false): void {
    if (!this.isOpen) return;
    if (droppable && this.socket.bufferedAmount > MAX_BUFFERED_BYTES) {
      this.droppedPackets += 1;
      return;
    }
    this.bytesOut += packet.byteLength;
    this.socket.send(packet);
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
    this.markClosed(reason);
    try {
      this.socket.close(1000, reason.slice(0, 120));
    } catch {
      /* already closed */
    }
  }

  private markClosed(reason: string): void {
    if (this.closedReason !== null) return;
    this.closedReason = reason;
    this.onClose?.(reason);
  }
}

function defaultFactory(url: string): SocketLike {
  return new WebSocket(url) as unknown as SocketLike;
}
