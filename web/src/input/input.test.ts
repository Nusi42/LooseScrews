import assert from "node:assert/strict";
import { test } from "node:test";
import { axesFromActions, KEY_BINDINGS, type Action } from "./bindings.ts";
import { emptyInput } from "./input.ts";

test("design-doc controls are bound", () => {
  const expected: Record<string, Action> = {
    KeyW: "forward",
    KeyS: "back",
    KeyA: "left",
    KeyD: "right",
    Space: "up",
    ControlLeft: "down",
    KeyQ: "rollLeft",
    KeyE: "rollRight",
    ShiftLeft: "boost",
    KeyR: "scavenge",
    KeyF: "overdrive",
    Digit1: "weapon1",
    Digit4: "weapon4",
  };
  for (const [code, action] of Object.entries(expected)) assert.equal(KEY_BINDINGS[code], action, code);
});

test("axes combine opposing keys to zero", () => {
  const axes = axesFromActions(new Set<Action>(["forward", "back", "right", "up", "rollLeft"]));
  assert.deepEqual(axes, { forward: 0, right: 1, up: 1, roll: -1 });
});

test("an empty input sample holds nothing", () => {
  const e = emptyInput();
  assert.equal(e.weaponKey, 0);
  assert.equal(e.overdrive, false);
  assert.equal(e.thrustForward, 0);
});
