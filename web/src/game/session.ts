// One running game: the ls-wasm `Game` (online or practice) plus, online, the
// transport to the server. Each animation frame: feed received packets in,
// step with the sampled controls, send what comes out, and read back the
// state to draw.

import { emptyInput, type InputState } from "../input/input.ts";
import type { HudValues } from "../ui/hud.ts";
import type { GameHandle, HudStateHandle, WasmApi } from "../wasm/api.ts";
import { connectTransport, transportLabel } from "../net/connect.ts";
import { ControlState, type ControlSettings } from "./controls.ts";
import {
  emptyGadget,
  parseCharge,
  parseCues,
  parseShipChoices,
  type ChargeMeter,
  type Cue,
  type GadgetState,
  type ShipChoice,
} from "./mechanics.ts";
import {
  parseDebugStats,
  parseMatchInfo,
  parseScoreboard,
  type DebugStats,
  type MatchInfo,
  type ScoreRow,
} from "./stats.ts";
import {
  emptyLoadout,
  loadoutState,
  parseEntities,
  parsePlayers,
  parsePose,
  parseProjectiles,
  parseShips,
  parseVisuals,
  type EntityState,
  type LoadoutState,
  type LocalPose,
  type PlayerState,
  type ProjectileState,
  type ShipState,
  type VisualDef,
} from "./views.ts";

/** What the session needs from a transport (WebSocketTransport / WebTransportTransport, or a fake in tests). */
export interface PacketLink {
  readonly isOpen: boolean;
  readonly closeReason: string | null;
  /**
   * `droppable`: may be skipped under send backpressure (inputs, pings):
   * the transport skips it while its send buffer is backed up instead of
   * queueing without bound. Each Input repeats the newest commands, so a
   * skipped one loses nothing still useful. Only set it for ls-net
   * `Channel::Unreliable` messages (Input, Ping): WebTransport sends
   * droppable packets as datagrams, which may also be lost in transit.
   */
  send(packet: Uint8Array, droppable?: boolean): void;
  drain(): Uint8Array[];
  close(reason?: string): void;
  /** Droppable packets skipped under backpressure (if the transport counts them). */
  readonly droppedPackets?: number;
}

/** A freshly opened link that can wait for the handshake reply. */
export interface HandshakeLink extends PacketLink {
  nextPacket(timeoutMs?: number): Promise<Uint8Array>;
}

/** Opens a link to `url` (default: `connectTransport`, WebTransport with WebSocket fallback). */
export type LinkOpener = (url: string) => Promise<HandshakeLink>;

export interface FrameResult {
  pose: LocalPose | null;
  ships: ShipState[];
  projectiles: ProjectileState[];
  entities: EntityState[];
  players: PlayerState[];
  hud: Partial<HudValues>;
  /** New kill-feed lines. */
  feed: string[];
  /** Game cues since the last frame (effects, banners). */
  cues: Cue[];
  /** Charge in progress (Fusion Charger, Polite Laser), for the charge meter. */
  charge: ChargeMeter | null;
  /** The own gadget. */
  gadget: GadgetState;
  /** Own Paint Thrower colour (1 red, 2 blue, 3 yellow) while it is the selected primary; 0 otherwise. */
  paintColor: number;
  /** Seconds until the own weapons are back online (Borrowed Time), 0 = online. */
  weaponsOffline: number;
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

/** Name tables from ls-wasm that decode the entity and visual arrays. */
export interface SceneNames {
  /** `entityKindNames()`: entity kind names by wire byte. */
  entityKinds: readonly string[];
  /** `visualShapeNames()`: shape names by shape index. */
  shapes: readonly string[];
}

export function sceneNames(api: Pick<WasmApi, "entityKindNames" | "visualShapeNames">): SceneNames {
  return { entityKinds: api.entityKindNames(), shapes: api.visualShapeNames() };
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
  readonly names: SceneNames;
  /** The match's visual table (constant for a match; indexed by visual id). */
  readonly visuals: readonly VisualDef[];
  /** Server `Disconnect` reason that means "match over, rejoin" (see ls_game::MATCH_RESTART_REASON). */
  readonly restartReason: string | null;
  /** Gadget string ids by gadget id (cloak shimmer, shield bubble). */
  gadgetIds: readonly string[] = [];
  private ended: string | null = null;
  private disposed = false;
  /** Parsed ship choices (see shipChoices()); null = refresh. */
  private choices: ShipChoice[] | null = null;
  /** Own ship alive at the previous frame (a change refreshes the ship choices). */
  private wasAlive: boolean | null = null;
  /** Monotonic clock in ms (times pings for the debug overlay). */
  clock: () => number = () => performance.now();

  constructor(
    game: GameHandle,
    link: PacketLink | null,
    names: SceneNames,
    horizonAssist = true,
    restartReason: string | null = null,
  ) {
    this.game = game;
    this.link = link;
    this.names = names;
    this.visuals = parseVisuals(game.visuals(), names.shapes);
    this.restartReason = restartReason;
    this.controls = new ControlState(horizonAssist);
  }

  /** Offline practice on `code` (null = a fresh random level), flying `shipType` if given. */
  static practice(api: WasmApi, code: string | null, horizonAssist = true, shipType?: number): GameSession {
    const [lo, hi] = entropy();
    const level = code ?? api.randomShareCode(lo, hi, 45, 35);
    const game =
      shipType !== undefined && api.Game.practiceAs
        ? api.Game.practiceAs(level, PRACTICE_DUMMIES, shipType)
        : api.Game.practice(level, PRACTICE_DUMMIES);
    const session = new GameSession(game, null, sceneNames(api), horizonAssist);
    session.gadgetIds = api.gadgetIds?.() ?? [];
    return session;
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
    open: LinkOpener = (u) => connectTransport(u),
    controls?: ControlSettings,
    shipType?: number,
  ): Promise<GameSession> {
    const link = await open(url);
    try {
      const hello =
        shipType !== undefined && api.helloPacketAs ? api.helloPacketAs(name, -1, shipType) : api.helloPacket(name, -1);
      link.send(hello);
      const welcome = await link.nextPacket();
      const game = api.Game.fromWelcome(welcome);
      const session = new GameSession(game, link, sceneNames(api), horizonAssist, api.matchRestartReason());
      session.gadgetIds = api.gadgetIds?.() ?? [];
      if (controls) session.controls.settings = { ...controls };
      return session;
    } catch (err) {
      link.close("handshake failed");
      throw err;
    }
  }

  get isOnline(): boolean {
    return this.link !== null;
  }

  /** Active transport name for the debug overlay ("WebTransport" / "WebSocket"); null offline. */
  get transportName(): string | null {
    return this.link ? transportLabel(this.link) : null;
  }

  /** Packets the transport skipped under send backpressure (null if it does not count them). */
  get droppedSends(): number | null {
    const n = this.link?.droppedPackets;
    return typeof n === "number" ? n : null;
  }

  /** Debug counters (ping, prediction, predicted shots). */
  debugStats(): DebugStats {
    return parseDebugStats(this.disposed ? [] : this.game.debugStats());
  }

  /** Scoreboard rows (sorted by team, then kills) with player names. */
  scoreboard(): ScoreRow[] {
    return this.disposed ? [] : parseScoreboard(this.game.scoreboard(), this.game.scoreboardNames());
  }

  /** Carried weapons and gadget for the weapon panel. */
  loadout(): LoadoutState {
    return this.disposed ? emptyLoadout() : loadoutState(this.game.loadout());
  }

  /** Match result and team paint cells. */
  matchInfo(): MatchInfo {
    return parseMatchInfo(this.disposed ? [] : this.game.matchInfo());
  }

  get hasEnded(): boolean {
    return this.ended !== null;
  }

  frame(dt: number, input: InputState): FrameResult {
    let disconnected: string | null = null;
    let matchRestart = false;
    if (this.link && this.ended === null) {
      this.game.setClockMs(this.clock());
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

    const pose = parsePose(this.game.localPose());
    const alive = pose ? pose.alive : null;
    if (alive !== this.wasAlive) {
      this.wasAlive = alive;
      this.choices = null;
    }
    return {
      pose,
      ships: parseShips(this.game.remoteShips()),
      projectiles: parseProjectiles(this.game.projectiles()),
      entities: parseEntities(this.game.entities(), this.names.entityKinds),
      players: parsePlayers(this.game.playerInfo(), this.game.playerNames()),
      hud: { ...hudValues(this.game.hud()), horizonAssist: this.controls.horizonAssist, loadout: this.loadout() },
      feed: this.game.takeFeed(),
      cues: parseCues(this.game.takeCues?.()),
      charge: parseCharge(this.game.charge?.()),
      gadget: this.gadget(),
      paintColor: this.game.paintColor ?? 0,
      weaponsOffline: this.game.weaponsOfflineSecs ?? 0,
      disconnected,
      matchRestart,
    };
  }

  /** The own gadget (name, on/off, cooldown). */
  gadget(): GadgetState {
    const h = this.disposed ? undefined : this.game.gadget?.();
    if (!h) return emptyGadget();
    try {
      return {
        id: h.id,
        key: h.key,
        name: h.name,
        active: h.active,
        toggle: h.toggle,
        cooldownLeft: h.cooldownLeft,
        cooldownSecs: h.cooldownSecs,
        energyCost: h.energyCost,
        drainPerSec: h.drainPerSec,
      };
    } finally {
      h.free();
    }
  }

  /** Ship types with numbers, rule set permission and loadout preview. */
  shipChoices(): ShipChoice[] {
    if (this.disposed) return [];
    // Cached: the choices change only on a pick or when the ship dies or
    // respawns (the new type flies then), not every frame.
    this.choices ??= parseShipChoices(this.game.shipChoices?.());
    return this.choices;
  }

  /**
   * Picks the ship type for the next respawn (online: sends SelectShip
   * reliably). Returns an error text, or null on success.
   */
  selectShip(shipType: number): string | null {
    if (this.disposed || !this.game.selectShip) return "Ship selection is not available";
    this.choices = null;
    try {
      const packet = this.game.selectShip(shipType);
      if (packet && this.link && this.ended === null) this.link.send(packet, false);
      return null;
    } catch (err) {
      return err instanceof Error ? err.message : String(err);
    }
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
