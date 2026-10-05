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

test("overlay keys: Tab holds the scoreboard, F3 toggles the debug overlay", async () => {
  const { UI_ACTIONS } = await import("./bindings.ts");
  assert.equal(KEY_BINDINGS["Tab"], "scoreboard");
  assert.equal(KEY_BINDINGS["F3"], "toggleDebug");
  assert.ok(UI_ACTIONS.has("scoreboard") && UI_ACTIONS.has("toggleDebug"));
  assert.ok(!UI_ACTIONS.has("forward"), "flight keys still need pointer lock");
  const e = emptyInput();
  assert.equal(e.scoreboard, false);
  assert.equal(e.toggleDebug, false);
});

test("focused controls keep their keys while the pointer is free", async () => {
  const { controlOwnsKey } = await import("./input.ts");
  const button = { tagName: "BUTTON" };
  const text = { tagName: "INPUT", type: "text" };
  const check = { tagName: "INPUT", type: "checkbox" };
  // Buttons and checkboxes: Enter activates, Tab moves the focus; letters stay game keys.
  for (const t of [button, check, { tagName: "A" }]) {
    assert.ok(controlOwnsKey(t, "Enter") && controlOwnsKey(t, "NumpadEnter") && controlOwnsKey(t, "Tab"));
    assert.ok(!controlOwnsKey(t, "KeyT") && !controlOwnsKey(t, "KeyO"));
  }
  // Text fields take every key but F3.
  for (const t of [text, { tagName: "textarea" }, { tagName: "DIV", isContentEditable: true }]) {
    assert.ok(controlOwnsKey(t, "KeyT") && controlOwnsKey(t, "Enter") && controlOwnsKey(t, "Tab"));
    assert.ok(!controlOwnsKey(t, "F3"));
  }
  // The page itself (nothing focused): the overlay keys.
  for (const t of [null, undefined, { tagName: "BODY" }, { tagName: "CANVAS" }, {}]) {
    assert.ok(!controlOwnsKey(t, "Enter") && !controlOwnsKey(t, "Tab"));
  }
});

test("Enter on a focused lobby button does not also toggle Ready", async () => {
  const { InputController } = await import("./input.ts");
  const g = globalThis as Record<string, unknown>;
  const saved = { window: g["window"], document: g["document"] };
  const win = new EventTarget();
  const doc = Object.assign(new EventTarget(), { pointerLockElement: null, exitPointerLock: () => {} });
  g["window"] = win;
  g["document"] = doc;
  try {
    const input = new InputController(new EventTarget() as unknown as HTMLElement);
    const key = (code: string, type: string, target: object | null): Event => {
      const e = Object.assign(new Event(type, { cancelable: true }), { code, repeat: false });
      if (target) Object.defineProperty(e, "target", { value: target });
      win.dispatchEvent(e);
      return e;
    };
    // Focus on a lobby button: the browser clicks it; the game ignores the key.
    const onButton = key("Enter", "keydown", { tagName: "BUTTON" });
    key("Enter", "keyup", { tagName: "BUTTON" });
    assert.equal(onButton.defaultPrevented, false);
    assert.equal(input.sample().toggleReady, false);
    // Nothing focused: Enter is the Ready key, as before.
    const onPage = key("Enter", "keydown", { tagName: "BODY" });
    assert.equal(onPage.defaultPrevented, true);
    assert.equal(input.sample().toggleReady, true);
    input.dispose();
  } finally {
    g["window"] = saved.window;
    g["document"] = saved.document;
  }
});
