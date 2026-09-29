// Picks the transport to the server: WebTransport first where the browser
// has it and the server offers it (GET /cert-hash), WebSocket otherwise or
// when WebTransport fails to connect. `?transport=ws|wt` forces one.
//
// Integration (the session and HUD live elsewhere):
//   const link = await connectTransport(serverUrl(location), { prefer: transportPreference(location) });
//   hud: transportLabel(link)  -> "WebTransport" | "WebSocket"

import { WebSocketTransport, type SocketFactory, TransportError } from "./transport.ts";
import {
  browserWebTransport,
  certHashUrl,
  parseCertHash,
  WebTransportTransport,
  webTransportUrl,
  type WebTransportFactory,
} from "./webtransport.ts";

export type TransportKind = "webtransport" | "websocket";

/** `auto`: WebTransport if possible, else WebSocket. */
export type TransportPreference = "auto" | "ws" | "wt";

/** Either transport; both have the same surface. */
export type Transport = WebSocketTransport | WebTransportTransport;

export type FetchLike = (url: string, init?: { signal?: AbortSignal }) => Promise<{
  ok: boolean;
  status: number;
  json(): Promise<unknown>;
}>;

export interface ConnectOptions {
  prefer?: TransportPreference;
  /** WebSocket factory (tests). */
  socketFactory?: SocketFactory;
  /** WebTransport factory; default: the browser's, if it has one. `null` disables WebTransport. */
  webTransportFactory?: WebTransportFactory | null;
  /** fetch for /cert-hash (tests). */
  fetch?: FetchLike;
  /** Limit for fetching /cert-hash. */
  certHashTimeoutMs?: number;
  /** Limit for opening the WebTransport session before falling back. */
  webTransportTimeoutMs?: number;
  /** Limit for the WebSocket connection. */
  webSocketTimeoutMs?: number;
  /** Told why WebTransport was not used (default: console.info). */
  onFallback?: (reason: string) => void;
}

/** `?transport=ws` or `?transport=wt` in the page URL; `auto` otherwise. */
export function transportPreference(loc: { search: string }): TransportPreference {
  const v = new URLSearchParams(loc.search).get("transport")?.toLowerCase();
  if (v === "ws" || v === "websocket") return "ws";
  if (v === "wt" || v === "webtransport") return "wt";
  return "auto";
}

/** Human-readable transport name for the HUD / debug overlay ("offline" without a link). */
export function transportLabel(link: unknown): string {
  const kind = (link as { kind?: unknown } | null)?.kind;
  if (kind === "webtransport") return "WebTransport";
  if (kind === "websocket") return "WebSocket";
  return "offline";
}

/** The active transport's kind, or null without a link. */
export function transportKind(link: unknown): TransportKind | null {
  const kind = (link as { kind?: unknown } | null)?.kind;
  return kind === "webtransport" || kind === "websocket" ? kind : null;
}

/**
 * Connects to the server behind `wsUrl` (the WebSocket URL, e.g. from
 * `serverUrl(location)`), preferring WebTransport; see the file comment.
 * Rejects only if the chosen/fallback transport cannot connect.
 */
export async function connectTransport(wsUrl: string, opts: ConnectOptions = {}): Promise<Transport> {
  const prefer = opts.prefer ?? "auto";
  const onFallback = opts.onFallback ?? ((reason: string) => console.info(`WebTransport not used: ${reason}; using WebSocket`));
  if (prefer !== "ws") {
    try {
      return await connectWebTransport(wsUrl, opts);
    } catch (err) {
      const reason = err instanceof Error ? err.message : String(err);
      if (prefer === "wt") throw new TransportError(`WebTransport: ${reason}`);
      onFallback(reason);
    }
  }
  return WebSocketTransport.connect(wsUrl, opts.socketFactory, opts.webSocketTimeoutMs);
}

async function connectWebTransport(wsUrl: string, opts: ConnectOptions): Promise<WebTransportTransport> {
  const factory = opts.webTransportFactory === undefined ? browserWebTransport() : opts.webTransportFactory;
  if (!factory) throw new TransportError("this browser or page has no WebTransport");
  const fetchFn = opts.fetch ?? defaultFetch();
  if (!fetchFn) throw new TransportError("no fetch");
  const url = certHashUrl(wsUrl);
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), opts.certHashTimeoutMs ?? 3000);
  let info;
  try {
    const r = await fetchFn(url, { signal: controller.signal });
    if (!r.ok) throw new TransportError(`the server does not offer WebTransport (${url}: ${r.status})`);
    info = parseCertHash(await r.json());
  } catch (err) {
    if (err instanceof TransportError) throw err;
    throw new TransportError(`cannot read ${url}: ${err instanceof Error ? err.message : String(err)}`);
  } finally {
    clearTimeout(timer);
  }
  if (!info) throw new TransportError(`unexpected answer from ${url}`);
  if (info.expires > 0 && info.expires * 1000 < Date.now()) throw new TransportError("the server certificate has expired");
  return WebTransportTransport.connect(webTransportUrl(wsUrl, info), info.hash, factory, opts.webTransportTimeoutMs);
}

function defaultFetch(): FetchLike | null {
  const f = (globalThis as { fetch?: FetchLike }).fetch;
  return typeof f === "function" ? (url, init) => f(url, init) : null;
}
