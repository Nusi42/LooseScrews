// Turns the sampled InputState into the flat controls array ls-wasm's
// `Game.frame` expects (see wasm/api.ts). Look deltas become radians here;
// ls-wasm spreads them over the 60 Hz ticks that run this frame.

import type { InputState } from "../input/input.ts";
import { CONTROLS_LEN } from "../wasm/api.ts";

/** `ls_sim::Buttons` bits (crates/ls-sim/src/input.rs). */
export const BUTTONS = {
  boost: 1 << 0,
  firePrimary: 1 << 1,
  fireSecondary: 1 << 2,
  scavenge: 1 << 3,
  overdrive: 1 << 4,
  horizonAssist: 1 << 5,
} as const;

/** Weapon selected by keys 1-4, by ls-sim weapon id (data/weapons/*.ron). */
export const WEAPON_KEYS: readonly string[] = ["scrap_laser", "paint_thrower", "rivet_gun", "bolt_rocket"];

export interface ControlSettings {
  /** Radians per CSS pixel of mouse movement. */
  lookSensitivity: number;
  invertY: boolean;
}

export const DEFAULT_CONTROL_SETTINGS: ControlSettings = {
  lookSensitivity: 0.0022,
  invertY: false,
};

/**
 * Per-session control state: horizon assist toggle and the sticky weapon
 * selection (re-sent every frame so a key press never gets lost on a frame
 * that runs no tick; the server ignores weapons the ship does not carry).
 */
export class ControlState {
  horizonAssist: boolean;
  /** ls-sim weapon id + 1 of the last selection, 0 = none yet. */
  weaponSelect = 0;
  settings: ControlSettings;
  /** Weapon string ids in ls-sim id order (from ls-wasm `weaponIds()`). */
  private readonly weaponIds: readonly string[];
  private readonly out = new Float32Array(CONTROLS_LEN);

  constructor(weaponIds: readonly string[], horizonAssist = true, settings: ControlSettings = DEFAULT_CONTROL_SETTINGS) {
    this.weaponIds = weaponIds;
    this.horizonAssist = horizonAssist;
    this.settings = { ...settings };
  }

  /** Applies toggles from `input` and returns the controls for `Game.frame` (reused buffer). */
  update(input: InputState): Float32Array {
    if (input.toggleHorizonAssist) this.horizonAssist = !this.horizonAssist;
    if (input.weaponKey > 0) {
      const id = this.weaponIds.indexOf(WEAPON_KEYS[input.weaponKey - 1] ?? "");
      if (id >= 0) this.weaponSelect = id + 1;
    }
    const s = this.settings;
    let buttons = 0;
    if (input.boost) buttons |= BUTTONS.boost;
    if (input.firePrimary) buttons |= BUTTONS.firePrimary;
    if (input.fireSecondary) buttons |= BUTTONS.fireSecondary;
    if (input.scavenge) buttons |= BUTTONS.scavenge;
    if (input.overdrive) buttons |= BUTTONS.overdrive;
    if (this.horizonAssist) buttons |= BUTTONS.horizonAssist;
    const o = this.out;
    o[0] = clampUnit(input.thrustForward);
    o[1] = clampUnit(input.thrustRight);
    o[2] = clampUnit(input.thrustUp);
    o[3] = clampUnit(input.roll);
    // Mouse right turns right; mouse up (negative movementY) raises the nose.
    o[4] = input.lookX * s.lookSensitivity;
    o[5] = -input.lookY * s.lookSensitivity * (s.invertY ? -1 : 1);
    o[6] = buttons;
    o[7] = this.weaponSelect;
    return o;
  }
}

function clampUnit(v: number): number {
  return Number.isFinite(v) ? Math.max(-1, Math.min(1, v)) : 0;
}
