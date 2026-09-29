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
  | "toggleHorizonAssist";

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
  KeyH: "toggleHorizonAssist",
};

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
