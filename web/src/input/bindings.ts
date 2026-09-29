// Key and mouse bindings from the design doc (docs/design/game-design-doc.md,
// "Controls"). Uses KeyboardEvent.code, so bindings follow physical key
// positions on every layout (QWERTZ, AZERTY, ...).

export type Action =
  | "forward"
  | "back"
  | "left"
  | "right"
  | "up"
  | "down"
  | "rollLeft"
  | "rollRight"
  | "boost"
  | "scavenge"
  | "overdrive"
  | "toggleHorizonAssist"
  | "weapon1"
  | "weapon2"
  | "weapon3"
  | "weapon4"
  // Interface keys (work with or without pointer lock).
  | "scoreboard"
  | "toggleDebug";

export const KEY_BINDINGS: Readonly<Record<string, Action>> = {
  KeyW: "forward",
  KeyS: "back",
  KeyA: "left",
  KeyD: "right",
  Space: "up",
  ControlLeft: "down",
  ControlRight: "down",
  // Alternate for "down": browsers reserve Ctrl+W/Ctrl+T etc. and never pass
  // them to pages, so Ctrl+W while diving would close the tab.
  KeyC: "down",
  KeyQ: "rollLeft",
  KeyE: "rollRight",
  ShiftLeft: "boost",
  ShiftRight: "boost",
  KeyR: "scavenge",
  // Overdrive gamble: hold to pay missing weapon energy with hull (2 hull per 1 energy).
  KeyF: "overdrive",
  KeyH: "toggleHorizonAssist",
  // Weapon selection (see WEAPON_KEYS in game/controls.ts for the order).
  Digit1: "weapon1",
  Digit2: "weapon2",
  Digit3: "weapon3",
  Digit4: "weapon4",
  // Hold for the scoreboard; F3 toggles the debug overlay.
  Tab: "scoreboard",
  F3: "toggleDebug",
};

/** Actions handled even while the pointer is not locked (overlays, not flying). */
export const UI_ACTIONS: ReadonlySet<Action> = new Set<Action>(["scoreboard", "toggleDebug"]);

/** Weapon-slot actions in key order (Digit1 = index 0). */
export const WEAPON_ACTIONS: readonly Action[] = ["weapon1", "weapon2", "weapon3", "weapon4"];

/** Mouse buttons (MouseEvent.button): 0 = left = primary, 2 = right = secondary. */
export const MOUSE_PRIMARY = 0;
export const MOUSE_SECONDARY = 2;

export interface Axes {
  /** +1 forward (W), -1 back (S) */
  forward: number;
  /** +1 right (D), -1 left (A) */
  right: number;
  /** +1 up (Space), -1 down (Ctrl) */
  up: number;
  /** +1 roll right / clockwise (E), -1 roll left (Q) */
  roll: number;
}

/** Converts the set of held actions into thrust/roll axes in -1..1. */
export function axesFromActions(held: ReadonlySet<Action>): Axes {
  const axis = (pos: Action, neg: Action): number => (held.has(pos) ? 1 : 0) - (held.has(neg) ? 1 : 0);
  return {
    forward: axis("forward", "back"),
    right: axis("right", "left"),
    up: axis("up", "down"),
    roll: axis("rollRight", "rollLeft"),
  };
}
