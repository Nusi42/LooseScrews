// Mouse-look steering for smoke scripts (moods, race): the mouse movement that turns the own
// ship's nose toward a point, over the fake pointer lock (web/src/game/controls.ts: mouse right
// turns right, mouse up raises the nose, `lookSensitivity` rad per px).

/** `look` sensitivity (web/src/game/controls.ts DEFAULT_CONTROL_SETTINGS.lookSensitivity, rad per px). */
export const LOOK_RAD_PER_PX = 0.0022;

/** `v` rotated by the inverse of unit quaternion `q` (world -> ship frame; forward is -Z). */
export function toLocal(q, v) {
  const x = -q.x;
  const y = -q.y;
  const z = -q.z;
  const w = q.w;
  const tx = 2 * (y * v.z - z * v.y);
  const ty = 2 * (z * v.x - x * v.z);
  const tz = 2 * (x * v.y - y * v.x);
  return {
    x: v.x + w * tx + (y * tz - z * ty),
    y: v.y + w * ty + (z * tx - x * tz),
    z: v.z + w * tz + (x * ty - y * tx),
  };
}

/** Mouse movement (px, capped) turning `pose` toward `target`, and the angle left (rad). */
export function lookDelta(pose, target, maxPx = 70) {
  const p = pose.position;
  const d = toLocal(pose.orientation, { x: target.x - p.x, y: target.y - p.y, z: target.z - p.z });
  const yaw = Math.atan2(d.x, -d.z);
  const pitch = Math.atan2(d.y, Math.hypot(d.x, d.z));
  const clamp = (v) => Math.max(-maxPx, Math.min(maxPx, v));
  return { dx: clamp(yaw / LOOK_RAD_PER_PX), dy: clamp(-pitch / LOOK_RAD_PER_PX), angle: Math.hypot(yaw, pitch) };
}

/**
 * Mouse look by `dx`, `dy` px: a synthetic `mousemove` with that movement (the game reads only
 * `movementX/Y` under the pointer lock), so the cursor never runs off the page.
 */
export function look(page, dx, dy) {
  return page.evaluate(
    (x, y) => document.dispatchEvent(new MouseEvent("mousemove", { movementX: Math.round(x), movementY: Math.round(y), bubbles: true })),
    dx,
    dy,
  );
}

/**
 * Turns the page's ship toward `target` (world point) with mouse moves until within `tol` rad
 * or `ms` ran out. Returns the angle left (rad; Infinity without a pose).
 */
export async function aimAt(page, target, { tol = 0.05, ms = 2500 } = {}) {
  const end = Date.now() + ms;
  for (;;) {
    const pose = await page.evaluate(() => window.__lsGame?.pose() ?? null);
    if (!pose) return Infinity;
    const d = lookDelta(pose, target, 40);
    if (d.angle < tol || Date.now() > end) return d.angle;
    await look(page, 0.5 * d.dx, 0.5 * d.dy);
    await new Promise((r) => setTimeout(r, 50));
  }
}
