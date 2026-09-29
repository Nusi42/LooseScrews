// Turns the sampled InputState into the flat controls array ls-wasm's
// `Game.frame` expects (see wasm/api.ts). Look deltas become radians here;
// ls-wasm spreads them over the 60 Hz ticks that run this frame. Weapon keys
// are passed as intents (number key, wheel step, X press): ls-wasm resolves
// them against the carried weapons (crates/ls-wasm/src/inventory.rs).

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
  gadget: 1 << 6,
} as const;

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
 * Per-session control state: the horizon assist toggle and the look
 * settings. Weapon selection is resolved by ls-wasm (it knows what the ship
 * carries and keeps a key press until a tick sends it).
 */
export class ControlState {
  horizonAssist: boolean;
  settings: ControlSettings;
  private readonly out = new Float32Array(CONTROLS_LEN);

  constructor(horizonAssist = true, settings: ControlSettings = DEFAULT_CONTROL_SETTINGS) {
    this.horizonAssist = horizonAssist;
    this.settings = { ...settings };
  }

  /** Applies toggles from `input` and returns the controls for `Game.frame` (reused buffer). */
  update(input: InputState): Float32Array {
    if (input.toggleHorizonAssist) this.horizonAssist = !this.horizonAssist;
    const s = this.settings;
    let buttons = 0;
    if (input.boost) buttons |= BUTTONS.boost;
    if (input.firePrimary) buttons |= BUTTONS.firePrimary;
    if (input.fireSecondary) buttons |= BUTTONS.fireSecondary;
    if (input.scavenge) buttons |= BUTTONS.scavenge;
    if (input.overdrive) buttons |= BUTTONS.overdrive;
    if (this.horizonAssist) buttons |= BUTTONS.horizonAssist;
    if (input.gadget) buttons |= BUTTONS.gadget;
    const o = this.out;
    o[0] = clampUnit(input.thrustForward);
    o[1] = clampUnit(input.thrustRight);
    o[2] = clampUnit(input.thrustUp);
    o[3] = clampUnit(input.roll);
    // Mouse right turns right; mouse up (negative movementY) raises the nose.
    o[4] = input.lookX * s.lookSensitivity;
    o[5] = -input.lookY * s.lookSensitivity * (s.invertY ? -1 : 1);
    o[6] = buttons;
    o[7] = 0; // direct weapon pick (unused; keys below are resolved by ls-wasm)
    o[8] = Math.max(0, Math.min(10, Math.trunc(input.weaponKey) || 0));
    o[9] = Math.sign(input.cyclePrimary) || 0;
    o[10] = input.cycleSecondary ? 1 : 0;
    return o;
  }
}

function clampUnit(v: number): number {
  return Number.isFinite(v) ? Math.max(-1, Math.min(1, v)) : 0;
}
