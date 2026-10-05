// Ping probe for the server list (M3 package B7): the browser measures each
// listed server itself, in parallel with a cap (plan §3.1, "Directory
// service"). A probe opens a WebSocket to the server's game endpoint and
// closes it as soon as it is open, without sending a Hello (the server drops
// the silent connection; no player joins). Opening a WebSocket takes the
// TCP handshake plus the HTTP upgrade, two round trips, so the ping is half
// the time to `open`. A probe that fails or takes longer than the timeout
// reports `null` (shown as "-").

import { WS_PATH, type SocketFactory, type SocketLike } from "./transport.ts";

/** Probes running at once. */
export const PING_CONCURRENCY = 6;
/** A probe gives up after this long. */
export const PING_TIMEOUT_MS = 3000;

/** WebSocket URL of a server's game endpoint for a `host:port` address (`wss:` from https pages). */
export function wsUrlFor(address: string, pageProtocol: string): string {
  const scheme = pageProtocol === "https:" ? "wss:" : "ws:";
  return `${scheme}//${address}${WS_PATH}`;
}

/** HTTP base URL of a server (its `/api`), same scheme as the page. */
export function httpBaseFor(address: string, pageProtocol: string): string {
  const scheme = pageProtocol === "https:" ? "https:" : "http:";
  return `${scheme}//${address}`;
}

export interface ProbeOptions {
  /** WebSocket factory (tests); default: the browser's WebSocket. */
  socketFactory?: SocketFactory;
  /** Clock in ms; default: performance.now. */
  now?: () => number;
  timeoutMs?: number;
}

const browserSocket: SocketFactory = (url) => new WebSocket(url) as unknown as SocketLike;

/** One ping in ms (half the WebSocket open time, rounded), or null on failure or timeout. */
export function probe(url: string, opts: ProbeOptions = {}): Promise<number | null> {
  const factory = opts.socketFactory ?? browserSocket;
  const now = opts.now ?? (() => performance.now());
  const timeoutMs = opts.timeoutMs ?? PING_TIMEOUT_MS;
  return new Promise((resolve) => {
    let done = false;
    let socket: SocketLike | null = null;
    const finish = (ms: number | null): void => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      if (socket) {
        socket.onopen = null;
        socket.onerror = null;
        socket.onclose = null;
        try {
          socket.close(1000, "ping");
        } catch {
          /* already closed */
        }
      }
      resolve(ms);
    };
    const timer = setTimeout(() => finish(null), timeoutMs);
    const t0 = now();
    try {
      socket = factory(url);
    } catch {
      finish(null);
      return;
    }
    socket.onopen = () => finish(Math.max(1, Math.round((now() - t0) / 2)));
    socket.onerror = () => finish(null);
    socket.onclose = () => finish(null);
  });
}

/**
 * Pings every address with at most `concurrency` probes at once; `onResult`
 * is told each result as it arrives. Resolves with all results by address.
 */
export async function pingAll(
  addresses: readonly string[],
  ping: (address: string) => Promise<number | null>,
  concurrency = PING_CONCURRENCY,
  onResult?: (address: string, ms: number | null) => void,
): Promise<Map<string, number | null>> {
  const out = new Map<string, number | null>();
  const queue = [...new Set(addresses)];
  const worker = async (): Promise<void> => {
    for (;;) {
      const address = queue.shift();
      if (address === undefined) return;
      let ms: number | null = null;
      try {
        ms = await ping(address);
      } catch {
        ms = null;
      }
      out.set(address, ms);
      onResult?.(address, ms);
    }
  };
  const n = Math.max(1, Math.min(concurrency, queue.length));
  await Promise.all(Array.from({ length: n }, () => worker()));
  return out;
}

/** Ping as shown in the list. */
export function formatPing(ms: number | null | undefined): string {
  if (ms === undefined) return "…";
  return ms === null ? "-" : `${ms} ms`;
}
