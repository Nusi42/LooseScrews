// Pointer-lock keyboard/mouse input, exposed as an InputState sampled once per frame.

import {
  axesFromActions,
  KEY_BINDINGS,
  MOUSE_PRIMARY,
  MOUSE_SECONDARY,
  UI_ACTIONS,
  WEAPON_ACTIONS,
  type Action,
} from "./bindings.ts";

export interface InputState {
  /** Thrust axes in ship space, each -1..1 (per-axis, so diagonals add up). */
  thrustForward: number;
  thrustRight: number;
  thrustUp: number;
  /** +1 roll right (E), -1 roll left (Q). */
  roll: number;
  /** Mouse movement in CSS pixels accumulated since the previous sample. */
  lookX: number;
  lookY: number;
  boost: boolean;
  firePrimary: boolean;
  fireSecondary: boolean;
  scavenge: boolean;
  /** Overdrive gamble held (F). */
  overdrive: boolean;
  /** True for exactly one sample after H is pressed. */
  toggleHorizonAssist: boolean;
  /** Weapon key pressed since the previous sample: 1-4, or 0 for none. */
  weaponKey: number;
  pointerLocked: boolean;
  /** Scoreboard key (Tab) held. */
  scoreboard: boolean;
  /** True for exactly one sample after F3 is pressed. */
  toggleDebug: boolean;
}

export function emptyInput(): InputState {
  return {
    thrustForward: 0,
    thrustRight: 0,
    thrustUp: 0,
    roll: 0,
    lookX: 0,
    lookY: 0,
    boost: false,
    firePrimary: false,
    fireSecondary: false,
    scavenge: false,
    overdrive: false,
    toggleHorizonAssist: false,
    weaponKey: 0,
    pointerLocked: false,
    scoreboard: false,
    toggleDebug: false,
  };
}

type KeyboardLock = { lock?: (codes?: string[]) => Promise<void>; unlock?: () => void };

export class InputController {
  private readonly target: HTMLElement;
  private readonly held = new Set<Action>();
  private readonly edges = new Set<Action>();
  private lookX = 0;
  private lookY = 0;
  private primary = false;
  private secondary = false;
  private locked = false;
  private readonly abort = new AbortController();
  /** Called whenever pointer lock is gained or lost. */
  onLockChange: ((locked: boolean) => void) | null = null;

  /** `target` is the element that captures the pointer (normally the game canvas). */
  constructor(target: HTMLElement) {
    this.target = target;
    const opts = { signal: this.abort.signal };
    window.addEventListener("keydown", (e) => this.onKey(e, true), opts);
    window.addEventListener("keyup", (e) => this.onKey(e, false), opts);
    window.addEventListener("blur", () => this.releaseAll(), opts);
    document.addEventListener("mousemove", (e) => this.onMouseMove(e), opts);
    document.addEventListener("mousedown", (e) => this.onMouseButton(e, true), opts);
    document.addEventListener("mouseup", (e) => this.onMouseButton(e, false), opts);
    document.addEventListener("pointerlockchange", () => this.onPointerLockChange(), opts);
    target.addEventListener("contextmenu", (e) => e.preventDefault(), opts);
    target.addEventListener("click", () => this.requestLock(), opts);
  }

  get pointerLocked(): boolean {
    return this.locked;
  }

  /** Requests pointer lock (must run inside a user gesture such as a click). */
  requestLock(): void {
    if (this.locked) return;
    const el = this.target as HTMLElement & {
      requestPointerLock(options?: { unadjustedMovement?: boolean }): Promise<void> | void;
    };
    // Raw (unaccelerated) mouse movement where supported; fall back otherwise.
    const attempt = (options?: { unadjustedMovement?: boolean }): Promise<void> =>
      Promise.resolve().then(() => el.requestPointerLock(options));
    attempt({ unadjustedMovement: true })
      .catch(() => attempt())
      .catch(() => {
        /* the user can click again; nothing else to do */
      });
  }

  exitLock(): void {
    if (this.locked) document.exitPointerLock();
  }

  /** Returns the current input and resets the accumulated mouse deltas and one-shot edges. */
  sample(): InputState {
    const axes = axesFromActions(this.held);
    const state: InputState = {
      thrustForward: axes.forward,
      thrustRight: axes.right,
      thrustUp: axes.up,
      roll: axes.roll,
      lookX: this.lookX,
      lookY: this.lookY,
      boost: this.held.has("boost"),
      firePrimary: this.primary,
      fireSecondary: this.secondary,
      scavenge: this.held.has("scavenge"),
      overdrive: this.held.has("overdrive"),
      toggleHorizonAssist: this.edges.has("toggleHorizonAssist"),
      weaponKey: WEAPON_ACTIONS.findIndex((a) => this.edges.has(a)) + 1,
      pointerLocked: this.locked,
      scoreboard: this.held.has("scoreboard"),
      toggleDebug: this.edges.has("toggleDebug"),
    };
    this.lookX = 0;
    this.lookY = 0;
    this.edges.clear();
    return state;
  }

  dispose(): void {
    this.abort.abort();
    this.exitLock();
  }

  private onKey(e: KeyboardEvent, down: boolean): void {
    const action = KEY_BINDINGS[e.code];
    if (action && UI_ACTIONS.has(action)) {
      // Overlay keys work without pointer lock too; keep Tab from moving
      // focus and F3 from opening the browser's find bar.
      e.preventDefault();
      if (down) {
        if (!e.repeat && !this.held.has(action)) this.edges.add(action);
        this.held.add(action);
      } else {
        this.held.delete(action);
      }
      return;
    }
    if (!action || !this.locked) {
      if (!down && action) this.held.delete(action);
      return;
    }
    // Stop the page from scrolling on Space and swallow Ctrl combos we can intercept.
    e.preventDefault();
    if (down) {
      if (!e.repeat && !this.held.has(action)) this.edges.add(action);
      this.held.add(action);
    } else {
      this.held.delete(action);
    }
  }

  private onMouseMove(e: MouseEvent): void {
    if (!this.locked) return;
    this.lookX += e.movementX;
    this.lookY += e.movementY;
  }

  private onMouseButton(e: MouseEvent, down: boolean): void {
    if (!this.locked && down) return;
    if (e.button === MOUSE_PRIMARY) this.primary = down;
    else if (e.button === MOUSE_SECONDARY) this.secondary = down;
  }

  private onPointerLockChange(): void {
    this.locked = document.pointerLockElement === this.target;
    const keyboard = (navigator as Navigator & { keyboard?: KeyboardLock }).keyboard;
    if (this.locked) {
      // Keyboard lock only takes effect in fullscreen; it lets Ctrl/Escape reach the game there.
      keyboard?.lock?.(["ControlLeft", "ControlRight", "KeyW", "KeyS", "KeyA", "KeyD"]).catch(() => {});
    } else {
      keyboard?.unlock?.();
      this.releaseAll();
    }
    this.onLockChange?.(this.locked);
  }

  private releaseAll(): void {
    this.held.clear();
    this.primary = false;
    this.secondary = false;
    this.lookX = 0;
    this.lookY = 0;
  }
}
