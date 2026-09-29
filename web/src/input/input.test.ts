import assert from "node:assert/strict";
import { test } from "node:test";
import * as q from "../math/quat.ts";
import { axesFromActions, KEY_BINDINGS, type Action } from "./bindings.ts";
import { FlightCamera } from "./flightCamera.ts";
import { emptyInput, type InputState } from "./input.ts";

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
  };
  for (const [code, action] of Object.entries(expected)) assert.equal(KEY_BINDINGS[code], action, code);
});

test("axes combine opposing keys to zero", () => {
  const axes = axesFromActions(new Set<Action>(["forward", "back", "right", "up", "rollLeft"]));
  assert.deepEqual(axes, { forward: 0, right: 1, up: 1, roll: -1 });
});

function input(over: Partial<InputState>): InputState {
  return { ...emptyInput(), pointerLocked: true, ...over };
}

function fly(cam: FlightCamera, state: InputState, seconds: number, boosting = false): void {
  const dt = 1 / 60;
  const steps = Math.round(seconds / dt);
  for (let i = 0; i < steps; i++) cam.step(state, dt, boosting);
}

test("thrust is capped per axis: 12 u/s straight, 28.5 u/s boosted 3-axis diagonal", () => {
  const cam = new FlightCamera();
  cam.horizonAssist = false;
  fly(cam, input({ thrustForward: 1 }), 2);
  assert.ok(Math.abs(cam.speed - 12) < 1e-6, `straight cruise ${cam.speed}`);
  assert.ok(cam.velocity.z < 0, "forward is -Z");

  const diag = new FlightCamera();
  diag.horizonAssist = false;
  fly(diag, input({ thrustForward: 1, thrustRight: 1, thrustUp: 1 }), 2, true);
  assert.ok(Math.abs(diag.speed - 16.5 * Math.sqrt(3)) < 1e-4, `diagonal boost ${diag.speed}`);

  fly(diag, input({}), 2);
  assert.ok(diag.speed < 1e-6, "releasing thrust brings the ship to rest");
});

test("mouse right yaws right, mouse up pitches up", () => {
  const yaw = new FlightCamera();
  yaw.step(input({ lookX: 100 }), 1 / 60, false);
  assert.ok(q.forward(yaw.orientation).x > 0.1);
  const pitch = new FlightCamera();
  pitch.step(input({ lookY: -100 }), 1 / 60, false);
  assert.ok(q.forward(pitch.orientation).y > 0.1);
});

test("E rolls right (right wing down), Q rolls left", () => {
  const cam = new FlightCamera();
  cam.horizonAssist = false;
  fly(cam, input({ roll: 1 }), 0.2);
  assert.ok(q.right(cam.orientation).y < -0.1);
  const left = new FlightCamera();
  left.horizonAssist = false;
  fly(left, input({ roll: -1 }), 0.2);
  assert.ok(q.right(left.orientation).y > 0.1);
});

test("horizon assist levels the roll unless disabled; H toggles it", () => {
  const cam = new FlightCamera();
  cam.orientation = q.fromAxisAngle({ x: 0, y: 0, z: 1 }, 0.8); // banked left
  fly(cam, input({}), 4);
  assert.ok(Math.abs(cam.bankAngle()) < 0.02, `bank ${cam.bankAngle()}`);

  const off = new FlightCamera();
  off.orientation = q.fromAxisAngle({ x: 0, y: 0, z: 1 }, 0.8);
  off.step(input({ toggleHorizonAssist: true }), 1 / 60, false);
  assert.equal(off.horizonAssist, false);
  fly(off, input({}), 2);
  assert.ok(Math.abs(off.bankAngle() - 0.8) < 1e-4);
});
