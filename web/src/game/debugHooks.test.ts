import assert from "node:assert/strict";
import { test } from "node:test";

import { installGameHooks, type GameDebug, type HookSession } from "./debugHooks.ts";
import { emptyModeState } from "./modeState.ts";

function session(online: boolean, sent: Uint8Array[]): HookSession {
  return {
    isOnline: online,
    link: online ? { send: (p) => void sent.push(p) } : null,
    game: {
      player: 3,
      localPose: () => Float32Array.from([1, 2, 3, 0, 0, 0, 1, 0, 0, 0, 1]),
      architectActionPacket: (stroke, x) => Uint8Array.from([17, stroke, x]),
      entities: () => new Float32Array(0),
      raycast: () => 12,
    },
    names: { entityKinds: ["debris"] },
    modeState: () => emptyModeState(),
  };
}

test("__lsGame reads the session and sends architect strokes online only", () => {
  const sent: Uint8Array[] = [];
  const target: { __lsGame?: GameDebug } = {};
  const dbg = installGameHooks(session(true, sent), target);
  assert.equal(target.__lsGame, dbg);
  assert.equal(dbg.playerId(), 3);
  assert.deepEqual(dbg.pose()?.position, { x: 1, y: 2, z: 3 });
  assert.equal(dbg.modeState().mood.eventId, "");
  assert.deepEqual(dbg.entities(), []);
  assert.equal(dbg.raycast({ x: 0, y: 0, z: 0 }, { x: 0, y: 0, z: -1 }, 50), 12);
  assert.ok(dbg.architectStroke(3, 9, 0, 0, 16));
  assert.deepEqual([...sent[0]!], [17, 3, 9]);
  const offline = installGameHooks(session(false, sent), target);
  assert.equal(offline.architectStroke(3, 9, 0, 0, 16), false);
  assert.equal(sent.length, 1);
});
