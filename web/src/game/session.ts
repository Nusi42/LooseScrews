// One running game: the ls-wasm `Game` (online or practice) plus, online, the
// transport to the server. Each animation frame: feed received packets in,
// step with the sampled controls, send what comes out, and read back the
// state to draw.

import { emptyInput, type InputState } from "../input/input.ts";
import type { HudValues } from "../ui/hud.ts";
import type { GameHandle, HudStateHandle, WasmApi } from "../wasm/api.ts";
import { WebSocketTransport, type SocketFactory } from "../net/transport.ts";
import { ControlState } from "./controls.ts";
import {
  parsePose,
  parseProjectiles,
  parseShips,
  type LocalPose,
  type ProjectileState,
  type ShipState,
} from "./views.ts";

/** What the session needs from a transport (WebSocketTransport, or a fake in tests). */
export interface PacketLink {
  readonly isOpen: boolean;
  readonly closeReason: string | null;
  /** `droppable`: may be skipped under send backpressure (inputs, pings). */
  send(packet: Uint8Array, droppable?: boolean): void;
  drain(): Uint8Array[];
  close(reason?: string): void;
}

export interface FrameResult {
  pose: LocalPose | null;
  ships: ShipState[];
  projectiles: ProjectileState[];
  hud: Partial<HudValues>;
  /** New kill-feed lines. */
  feed: string[];
  /** Set once when the online session ends (server closed, connection lost, desync). */
  disconnected: string | null;
  /** With `disconnected`: the server finished the match and starts a new one (rejoin). */
  matchRestart: boolean;
}

/** Number of idle target dummies in practice. */
export const PRACTICE_DUMMIES = 3;

/** Maps the wasm HUD object to HudValues and frees it. */
export function hudValues(h: HudStateHandle): Partial<HudValues> {
  try {
    return {
      hull: h.hull,
      hullMax: h.hullMax,
      energy: h.energy,
      energyMax: h.energyMax,
      boost: h.boost,
      boostMax: h.boostMax,
      scrap: h.scrap,
      weapon: h.weapon || "—",
      ammo: h.ammo,
      secondary: h.secondary,
      paint: [h.paint0, h.paint1],
      timeLeft: h.timeLeft,
      status: h.status,
    };
  } finally {
    h.free();
  }
}

/** A random 64-bit value for practice share codes (the WASM side never touches OS RNG). */
export function entropy(): [number, number] {
  const v = new Uint32Array(2);
  crypto.getRandomValues(v);
  return [v[0] ?? 0, v[1] ?? 0];
}

export class GameSession {
  readonly game: GameHandle;
  readonly link: PacketLink | null;
  readonly controls: ControlState;
  readonly weaponIds: readonly string[];
  /** Server `Disconnect` reason that means "match over, rejoin" (see ls_game::MATCH_RESTART_REASON). */
  readonly restartReason: string | null;
  private ended: string | null = null;
  private disposed = false;

  constructor(
    game: GameHandle,
    link: PacketLink | null,
    weaponIds: readonly string[],
    horizonAssist = true,
    restartReason: string | null = null,
  ) {
    this.game = game;
    this.link = link;
    this.weaponIds = weaponIds;
    this.restartReason = restartReason;
    this.controls = new ControlState(weaponIds, horizonAssist);
  }

  /** Offline practice on `code` (null = a fresh random level). */
  static practice(api: WasmApi, code: string | null, horizonAssist = true): GameSession {
    const [lo, hi] = entropy();
    const level = code ?? api.randomShareCode(lo, hi, 45, 35);
    return new GameSession(api.Game.practice(level, PRACTICE_DUMMIES), null, api.weaponIds(), horizonAssist);
  }

  /**
   * Connects to `url`, sends Hello and builds the game from the server's
   * Welcome. Rejects with the server's reason on Reject.
   */
  static async connect(
    api: WasmApi,
    url: string,
    name: string,
    horizonAssist = true,
    factory?: SocketFactory,
  ): Promise<GameSession> {
    const link = await WebSocketTransport.connect(url, factory);
    try {
      link.send(api.helloPacket(name, -1));
      const welcome = await link.nextPacket();
      const game = api.Game.fromWelcome(welcome);
      return new GameSession(game, link, api.weaponIds(), horizonAssist, api.matchRestartReason());
    } catch (err) {
      link.close("handshake failed");
      throw err;
    }
  }

  get isOnline(): boolean {
    return this.link !== null;
  }

  get hasEnded(): boolean {
    return this.ended !== null;
  }

  frame(dt: number, input: InputState): FrameResult {
    let disconnected: string | null = null;
    let matchRestart = false;
    if (this.link && this.ended === null) {
      for (const packet of this.link.drain()) {
        try {
          const reason = this.game.handlePacket(packet);
          if (reason !== undefined && reason === this.restartReason) {
            matchRestart = true;
            disconnected = "The match is over. The server is starting a new match on the same level.";
          } else if (reason !== undefined) {
            disconnected = `The server ended the session: ${reason}`;
          }
        } catch (err) {
          disconnected = `Lost sync with the server: ${err instanceof Error ? err.message : String(err)}`;
        }
        if (disconnected) break;
      }
      if (!disconnected && !this.link.isOpen) {
        disconnected = `Disconnected: ${this.link.closeReason ?? "connection closed"}`;
      }
      if (disconnected) {
        this.ended = disconnected;
        this.link.close(disconnected);
      }
    }

    const controls = this.controls.update(input);
    const out = this.game.frame(dt, controls);
    if (this.link && this.ended === null) {
      // Inputs and pings: each Input repeats the newest commands, so a
      // packet skipped under backpressure loses nothing that is still useful.
      for (const p of out) if (p instanceof Uint8Array) this.link.send(p, true);
    }

    return {
      pose: parsePose(this.game.localPose()),
      ships: parseShips(this.game.remoteShips()),
      projectiles: parseProjectiles(this.game.projectiles()),
      hud: { ...hudValues(this.game.hud()), horizonAssist: this.controls.horizonAssist },
      feed: this.game.takeFeed(),
      disconnected,
      matchRestart,
    };
  }

  /**
   * Keeps an online session alive while the tab is hidden (no animation
   * frames): drains packets and sends idle input plus the periodic ping, so
   * the server neither times the player out nor keeps repeating the last
   * held buttons. Returns the disconnect result if the session ended.
   */
  idleFrame(dt: number): FrameResult | null {
    if (!this.link || this.ended !== null || this.disposed) return null;
    const r = this.frame(dt, emptyInput());
    return r.disconnected ? r : null;
  }

  /** Team of a ship id (own ship included), for paint colours. */
  teamOf(ships: readonly ShipState[]): (id: number) => number | undefined {
    const player = this.game.player;
    const team = this.game.team;
    return (id) => (id === player ? team : ships.find((s) => s.id === id)?.team);
  }

  /** Closes the link and frees the wasm game (idempotent). */
  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.link?.close();
    this.game.free();
  }

  get isDisposed(): boolean {
    return this.disposed;
  }
}
