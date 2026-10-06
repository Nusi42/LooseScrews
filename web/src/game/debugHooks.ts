/**
 * `window.__lsGame` (M4 C6): a small read-mostly handle on the running
 * session for scripts (the browser smoke's moods, architect and race
 * phases, gallery and review scripts). It reads what the HUD reads (the
 * mode state, the own pose, the player id) and can send one kind of
 * message a player can send from the UI anyway: an Architect stroke
 * (`ArchitectAction`, validated by the server like any other), so the smoke
 * can play an Architect round before and besides the stroke menu.
 */

import type { ModeState } from "./modeState.ts";
import { parseEntities, parsePose, type EntityState, type LocalPose } from "./views.ts";

/** What the hooks need of a session (GameSession). */
export interface HookSession {
  readonly isOnline: boolean;
  readonly link: { send(packet: Uint8Array, droppable?: boolean): void } | null;
  readonly game: {
    architectActionPacket?(stroke: number, x: number, y: number, z: number, radius: number): Uint8Array | undefined;
    readonly player: number;
    localPose(): Float32Array | undefined;
    beltSite?(): string | undefined;
    entities(): Float32Array;
    raycast?(ox: number, oy: number, oz: number, dx: number, dy: number, dz: number, max: number): number;
  };
  readonly names: { readonly entityKinds: readonly string[] };
  modeState(): ModeState;
}

export interface GameDebug {
  readonly online: boolean;
  /** The own player id (wire id). */
  playerId(): number;
  modeState(): ModeState;
  pose(): LocalPose | null;
  /** The asteroid belt as JSON (ls-wasm `beltSite`), undefined on classic levels. */
  belt(): string | undefined;
  /** Replicated entities (pickups, debris, event props: drone, duck, hat, catPaw, gossipFace, discoBall, mimic, ...). */
  entities(): EntityState[];
  /** Distance to the first rock along a ray, -1 for none within `max`. */
  raycast(o: { x: number; y: number; z: number }, d: { x: number; y: number; z: number }, max: number): number;
  /** Sends an Architect stroke (`StrokeKind` index, centre, radius); false offline or without the packet. */
  architectStroke(kind: number, x: number, y: number, z: number, radius: number): boolean;
}

/** Installs `window.__lsGame` for `session` (replacing the previous one). */
export function installGameHooks(
  session: HookSession,
  target: { __lsGame?: GameDebug } | null = typeof window === "undefined" ? null : (window as unknown as { __lsGame?: GameDebug }),
): GameDebug {
  const debug: GameDebug = {
    online: session.isOnline,
    playerId: () => session.game.player,
    modeState: () => session.modeState(),
    pose: () => parsePose(session.game.localPose()),
    belt: () => session.game.beltSite?.(),
    entities: () => parseEntities(session.game.entities(), session.names.entityKinds),
    raycast: (o, d, max) => session.game.raycast?.(o.x, o.y, o.z, d.x, d.y, d.z, max) ?? -1,
    architectStroke: (kind, x, y, z, radius) => {
      const packet = session.game.architectActionPacket?.(kind, x, y, z, radius);
      if (!packet || !session.link) return false;
      session.link.send(packet, false);
      return true;
    },
  };
  if (target) target.__lsGame = debug;
  return debug;
}
