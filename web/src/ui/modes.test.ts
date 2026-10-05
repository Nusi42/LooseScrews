// M3 client: mode state / lobby / results parsing, the match flow (lobby →
// match → results → next match), the lobby clock, the lobby card, results
// screen and mode HUD models, objective marker projection, side colours,
// the spectator camera and match transitions through GameSession.

import assert from "node:assert/strict";
import { test } from "node:test";
import { emptyInput, type InputState } from "../input/input.ts";
import * as mat4 from "../math/mat4.ts";
import * as q from "../math/quat.ts";
import { LobbyClock, matchFlow } from "../game/matchFlow.ts";
import {
  emptyModeState,
  isOut,
  modeInfo,
  OBJECTIVE,
  parseLobbyState,
  parseModeState,
  parseResults,
  PLAYER_FLAG,
  secsUntil,
  VIEW_FLAG,
  type LobbyState,
  type MatchResults,
  type ModeState,
} from "../game/modeState.ts";
import { buildScene, TEAM_HULL } from "../game/scene.ts";
import { GameSession, type PacketLink } from "../game/session.ts";
import { cssColor, ffaColor, FFA_PALETTE, NEUTRAL_SIDE, sideColor, TEAM_PALETTE } from "../game/sides.ts";
import { SpectatorCamera } from "../game/spectator.ts";
import { parseShips } from "../game/views.ts";
import { GameStub } from "../wasm/stub.ts";
import { lobbyModel, nextTeamPick } from "./lobby.ts";
import { modeHudModel, objectiveMarkers, ordinal, projectMarker, type ModeHudInput } from "./modeHud.ts";
import { practiceResults, resultsModel } from "./results.ts";

const MODE_JSON =
  '{"kind":4,"mode":"hot_reactor","teams":0,"running":true,"winner":-1,"endTick":36000,"stage":0,' +
  '"stageEndTick":0,"scoreTotal":0,"scoreTarget":0,"flags":8,"teamScores":[],' +
  '"players":[{"id":3,"side":3,"score":2,"lives":2,"meter":50,"flags":4},{"id":1,"side":1,"score":1,"lives":-1,"meter":0,"flags":1}],' +
  '"objectives":[{"id":1,"kind":4,"team":-1,"holder":3,"state":0,"timerEnd":1200,"radius":1.5,"pos":[1,-2.5,3.25]}]}';

const LOBBY_JSON =
  '{"phase":"countdown","phaseTicksLeft":600,"roomCode":"","nextMode":1,"nextModeId":"team_brawl",' +
  '"nextRuleset":"team_brawl","nextShareCode":"LS-1","minPlayers":4,"maxPlayers":32,"teams":2,' +
  '"players":[{"id":0,"team":-1,"ready":true,"spectator":false},{"id":2,"team":1,"ready":false,"spectator":false}]}';

const RESULTS_JSON =
  '{"tick":99,"kind":1,"mode":"team_brawl","teams":2,"winner":1,"teamScores":[3,15],' +
  '"rows":[{"id":2,"side":1,"kills":7,"deaths":1,"score":7,"name":"Rita"},{"id":0,"side":0,"kills":3,"deaths":4,"score":3,"name":"Max"}]}';

function lobby(over: Partial<LobbyState> = {}): LobbyState {
  return { ...(parseLobbyState(LOBBY_JSON) as LobbyState), ...over };
}

function mode(over: Partial<ModeState> = {}): ModeState {
  return { ...emptyModeState(), ...over };
}

test("mode state, lobby and results parse from ls-wasm JSON; broken input is safe", () => {
  const m = parseModeState(MODE_JSON);
  assert.equal(m.mode, "hot_reactor");
  assert.equal(m.teams, 0);
  assert.equal(m.flags & VIEW_FLAG.lives, VIEW_FLAG.lives);
  assert.deepEqual(m.players[0], { id: 3, side: 3, score: 2, lives: 2, meter: 50, flags: 4 });
  assert.equal(m.players[1]!.lives, -1);
  assert.deepEqual(m.objectives[0], { id: 1, kind: OBJECTIVE.reactor, team: -1, holder: 3, state: 0, timerEnd: 1200, radius: 1.5, pos: { x: 1, y: -2.5, z: 3.25 } });
  assert.equal(isOut(m.players[1]), true, "eliminated");
  assert.equal(isOut(m.players[0]), false);
  assert.deepEqual(parseModeState("not json"), emptyModeState());
  assert.deepEqual(parseModeState(undefined), emptyModeState());
  assert.equal(parseModeState('{"players":[{"id":"x"}],"objectives":7}').players[0]!.id, 0);

  const l = parseLobbyState(LOBBY_JSON);
  assert.ok(l);
  assert.equal(l.phase, "countdown");
  assert.equal(l.players[1]!.team, 1);
  assert.equal(parseLobbyState('{"phase":"later"}'), null, "unknown phase");
  assert.equal(parseLobbyState(undefined), null);

  const r = parseResults(RESULTS_JSON);
  assert.ok(r);
  assert.equal(r.winner, 1);
  assert.equal(r.rows[0]!.name, "Rita");
  assert.equal(parseResults("[1,2]"), null);

  assert.equal(modeInfo("the_squeeze").stageLabel, "Phase");
  assert.equal(modeInfo("new_mode").name, "New Mode", "unknown ids still get a name");
  assert.equal(secsUntil(1260, 1200), 1);
  assert.equal(secsUntil(0, 1200), 0, "no timer");
  assert.equal(secsUntil(100, 1200), 0, "past");
});

test("match flow: lobby → countdown → match → results → next match, spectating when out", () => {
  const own = 0;
  const running = mode({ teams: 2 });
  const flow = (l: LobbyState | null, m: ModeState, r: MatchResults | null, online = true) => matchFlow({ online, lobby: l, mode: m, results: r, own });
  let f = flow(lobby({ phase: "lobby" }), running, null);
  assert.deepEqual(f, { stage: "warmup", showLobby: true, showResults: false, spectating: false });
  f = flow(lobby({ phase: "countdown" }), running, null);
  assert.equal(f.stage, "countdown");
  assert.equal(f.showLobby, true);
  f = flow(lobby({ phase: "match" }), running, null);
  assert.deepEqual(f, { stage: "match", showLobby: false, showResults: false, spectating: false });
  // The match ended: results show once the server's table arrived.
  const ended = mode({ teams: 2, running: false, winner: 1 });
  assert.equal(flow(lobby({ phase: "match" }), ended, null).showResults, false, "waits for MatchResults");
  const results = parseResults(RESULTS_JSON);
  f = flow(lobby({ phase: "results" }), ended, results);
  assert.equal(f.stage, "results");
  assert.equal(f.showResults, true);
  // Next match's warm-up after the transition (results cleared by the new Welcome).
  f = flow(lobby({ phase: "lobby" }), running, null);
  assert.equal(f.showResults, false);
  assert.equal(f.showLobby, true);
  // A server without a lobby (warm-up off): results from the mode end.
  assert.equal(flow(null, ended, results).showResults, true);
  // Practice: no lobby, results from the local mode state.
  assert.equal(flow(null, ended, null, false).showResults, true);

  // Out of an elimination mode: spectating.
  const out = mode({ teams: 0, players: [{ id: own, side: 0, score: 0, lives: 0, meter: 0, flags: PLAYER_FLAG.eliminated }] });
  assert.equal(flow(lobby({ phase: "match" }), out, null).spectating, true);
  // A late joiner flagged as spectator in the lobby, not listed in the mode yet.
  const spec = lobby({ phase: "match", players: [{ id: own, team: -1, ready: false, spectator: true }] });
  assert.equal(flow(spec, running, null).spectating, true);
});

test("lobby clock counts the phase down between LobbyState updates", () => {
  const c = new LobbyClock();
  const l = lobby({ phaseTicksLeft: 600 });
  assert.equal(c.secsLeft(l, 1000), 10);
  assert.equal(c.secsLeft(l, 4000), 7, "counted down locally");
  assert.equal(c.secsLeft(l, 20_000), 0, "never negative");
  // A new state (another player readied) restarts from its ticks.
  const l2 = lobby({ phaseTicksLeft: 300, players: [] });
  assert.equal(c.secsLeft(l2, 20_000), 5);
  assert.equal(c.secsLeft(lobby({ phase: "lobby", phaseTicksLeft: 0 }), 21_000), 0, "open-ended");
  assert.equal(c.secsLeft(null, 22_000), 0);
});

test("lobby model: mode card, ready count, team picks and the countdown", () => {
  const names = new Map([
    [0, "Max"],
    [2, "Rita"],
  ]);
  const m = lobbyModel(lobby({ phase: "lobby", phaseTicksLeft: 1800 }), 2, names, 21.2);
  assert.equal(m.kicker, "Warm-up");
  assert.equal(m.mode, "Team Brawl");
  assert.match(m.blurb, /kill target/);
  assert.equal(m.format, "2 teams");
  assert.equal(m.world, "LS-1");
  assert.equal(m.countdown, "0:22");
  assert.equal(m.countdownLabel, "Countdown in");
  assert.match(m.status, /Waiting for 2 more pilots/);
  assert.equal(m.players[0]!.you, true, "own row first");
  assert.equal(m.players[0]!.name, "Rita");
  assert.equal(m.players[0]!.team, "Blue");
  assert.equal(m.players[1]!.team, "Auto");
  assert.equal(m.ownReady, false);
  assert.deepEqual(m.teamOptions.map((t) => [t.value, t.label, t.selected]), [
    [-1, "Auto", false],
    [0, "Orange", false],
    [1, "Blue", true],
  ]);
  assert.match(m.keys, /Enter ready · T team/);

  const four = lobbyModel(lobby({ phase: "lobby", minPlayers: 2, players: [{ id: 0, team: -1, ready: true, spectator: false }, { id: 2, team: -1, ready: false, spectator: false }] }), 0, names, 0);
  assert.match(four.status, /1 of 2 ready/);
  assert.equal(four.countdown, "");
  assert.equal(four.ownReady, true);
  const ffa = lobbyModel(lobby({ phase: "countdown", teams: 0, nextModeId: "the_squeeze" }), 0, names, 4.5);
  assert.equal(ffa.kicker, "Match starting");
  assert.equal(ffa.format, "Free-for-all");
  assert.equal(ffa.teamOptions.length, 0);
  assert.equal(ffa.players[0]!.team, "");
  assert.equal(ffa.countdown, "0:05");
  assert.doesNotMatch(ffa.keys, /T team/);

  assert.equal(nextTeamPick(-1, 2), 0);
  assert.equal(nextTeamPick(0, 2), 1);
  assert.equal(nextTeamPick(1, 2), -1, "back to auto");
  assert.equal(nextTeamPick(3, 4), -1);
  assert.equal(nextTeamPick(-1, 0), -1, "free-for-all has no picks");
});

test("results model: team victory, defeat, free-for-all podium, paint shares, practice", () => {
  const r = parseResults(RESULTS_JSON) as MatchResults;
  const base = { own: 2, ownSide: 1, online: true, modeFlags: 0, scoreTotal: 0, lobby: lobby({ phase: "results", nextModeId: "the_squeeze" }), secsLeft: 7.2 };
  let m = resultsModel({ results: r, ...base });
  assert.equal(m.title, "Victory");
  assert.equal(m.outcome, "win");
  assert.equal(m.kicker, "Team Brawl · Match over");
  assert.equal(m.subtitle, "Blue wins · 3 – 15");
  assert.deepEqual(m.teams.map((t) => [t.name, t.score, Math.round(t.fill), t.winner, t.yours]), [
    ["Orange", "3", 20, false, false],
    ["Blue", "15", 100, true, true],
  ]);
  assert.equal(m.podium.length, 0);
  assert.equal(m.rows[0]!.you, true);
  assert.equal(m.footer, "Next: The Squeeze in 8 s");
  m = resultsModel({ results: r, ...base, own: 0, ownSide: 0 });
  assert.equal(m.title, "Defeat");
  m = resultsModel({ results: r, ...base, own: 9, ownSide: 9 });
  assert.equal(m.title, "Blue wins", "a spectator sees the winner");
  m = resultsModel({ results: { ...r, winner: -1 }, ...base });
  assert.equal(m.title, "Draw");
  assert.equal(m.winnerColor, "");
  m = resultsModel({ results: r, ...base, lobby: null, secsLeft: 0 });
  assert.equal(m.footer, "Starting the next match…");

  // Paint the Void: shares of the paintable surface.
  m = resultsModel({ results: { ...r, mode: "paint_the_void", teamScores: [100, 300] }, ...base, modeFlags: VIEW_FLAG.paintScores, scoreTotal: 1000 });
  assert.equal(m.subtitle, "Blue painted the most · 10.0% – 30.0%");
  assert.deepEqual(m.teams.map((t) => t.score), ["10.0%", "30.0%"]);

  // Free-for-all: podium of the top three, the winner by side.
  const ffa: MatchResults = {
    tick: 1,
    kind: 2,
    mode: "the_squeeze",
    teams: 0,
    winner: 5,
    teamScores: [],
    rows: [
      { id: 5, side: 5, kills: 4, deaths: 0, score: 4, name: "Ada" },
      { id: 1, side: 1, kills: 2, deaths: 1, score: 2, name: "Bo" },
      { id: 3, side: 3, kills: 1, deaths: 1, score: 1, name: "" },
      { id: 7, side: 7, kills: 0, deaths: 1, score: 0, name: "Di" },
    ],
  };
  m = resultsModel({ results: ffa, ...base, own: 1, ownSide: 1 });
  assert.equal(m.title, "Defeat");
  assert.equal(m.subtitle, "Ada wins with 4");
  assert.deepEqual(m.podium.map((p) => p.name), ["Ada", "Bo", "Pilot 4"]);
  assert.equal(m.rows[1]!.you, true);
  assert.equal(m.rows[0]!.team, "");
  m = resultsModel({ results: ffa, ...base, own: 5, ownSide: 5 });
  assert.equal(m.title, "Victory");
  assert.match(m.subtitle, /You win with 4/);

  // Practice: from the local mode state and the kill tally.
  const pm = mode({ mode: "team_brawl", teams: 2, running: false, winner: 0, teamScores: [5, 2], players: [{ id: 0, side: 0, score: 5, lives: -1, meter: 0, flags: 0 }] });
  const pr = practiceResults(pm, [{ id: 0, name: "You", team: 0, alive: true, kills: 5, deaths: 1, isYou: true }], new Map());
  assert.deepEqual(pr.rows[0], { id: 0, side: 0, kills: 5, deaths: 1, score: 5, name: "You" });
  m = resultsModel({ results: pr, ...base, own: 0, ownSide: 0, online: false, lobby: null });
  assert.equal(m.title, "Victory");
  assert.match(m.footer, /Practice over/);
});

function hudInput(over: Partial<ModeHudInput>): ModeHudInput {
  return {
    mode: emptyModeState(),
    stage: "match",
    own: 0,
    ownSide: 0,
    nowTick: 1000,
    timeLeft: 125,
    names: new Map([
      [0, "Me"],
      [3, "Ada"],
      [5, "Bo"],
    ]),
    ownPos: { x: 0, y: 0, z: 0 },
    spectating: false,
    following: null,
    ...over,
  };
}

test("mode HUD model: team scoreline, free-for-all leaders, stage, lives, meter, alerts, instagib", () => {
  // Team Brawl, 4 teams: chips per team towards the kill target.
  let h = modeHudModel(hudInput({ mode: mode({ mode: "team_brawl", teams: 4, teamScores: [3, 9, 1, 0], scoreTarget: 15 }), ownSide: 1 }));
  assert.equal(h.title, "Team Brawl");
  assert.equal(h.timer, "2:05");
  assert.equal(h.target, "First to 15");
  assert.deepEqual(h.chips.map((c) => [c.name, c.score, Math.round(c.fill), c.yours, c.leading]), [
    ["Orange", "3", 20, false, false],
    ["Blue", "9", 60, true, true],
    ["Lime", "1", 7, false, false],
    ["Magenta", "0", 0, false, false],
  ]);
  assert.equal(h.lives, -1);
  assert.equal(h.meter, -1);

  // Paint the Void: paint shares.
  h = modeHudModel(hudInput({ mode: mode({ mode: "paint_the_void", teams: 2, teamScores: [50, 25], scoreTotal: 500, flags: VIEW_FLAG.paintScores }) }));
  assert.deepEqual(h.chips.map((c) => c.score), ["10.0%", "5.0%"]);

  // Free-for-all with lives and meter, own standing.
  const ffa = mode({
    mode: "last_color_standing",
    teams: 0,
    flags: VIEW_FLAG.lives | VIEW_FLAG.meter,
    players: [
      { id: 3, side: 3, score: 4, lives: 3, meter: 100, flags: 0 },
      { id: 5, side: 5, score: 2, lives: 3, meter: 66, flags: 0 },
      { id: 0, side: 0, score: 1, lives: 2, meter: 32, flags: 0 },
      { id: 9, side: 9, score: 0, lives: 3, meter: 0, flags: PLAYER_FLAG.spectator },
    ],
  });
  h = modeHudModel(hudInput({ mode: ffa }));
  assert.deepEqual(h.chips.map((c) => c.name), ["Ada", "Bo", "Me"]);
  assert.equal(h.chips[0]!.leading, true);
  assert.equal(h.chips[2]!.yours, true);
  assert.equal(h.ownLine, "You · 3rd of 3 · 1");
  assert.equal(h.lives, 2);
  assert.equal(h.meter, 32);

  // The Squeeze: stage line, outside the safe zone.
  const squeeze = mode({
    mode: "the_squeeze",
    teams: 0,
    stage: 2,
    stageEndTick: 1000 + 12 * 60,
    objectives: [{ id: 0, kind: OBJECTIVE.safeZone, team: -1, holder: -1, state: 0, timerEnd: 0, radius: 10, pos: { x: 50, y: 0, z: 0 } }],
  });
  h = modeHudModel(hudInput({ mode: squeeze }));
  assert.equal(h.stage, "Phase 2 · 0:12");
  assert.deepEqual(h.alerts.map((a) => a.level), ["danger"]);
  assert.match(h.alerts[0]!.text, /Outside the safe zone/);
  assert.equal(modeHudModel(hudInput({ mode: squeeze, ownPos: { x: 45, y: 0, z: 0 } })).alerts.length, 0, "inside");

  // Holding the reactor / carrying the core.
  const reactor = mode({ mode: "hot_reactor", teams: 0, objectives: [{ id: 1, kind: OBJECTIVE.reactor, team: -1, holder: 0, state: 0, timerEnd: 1000 + 90, radius: 1, pos: { x: 0, y: 0, z: 0 } }] });
  h = modeHudModel(hudInput({ mode: reactor }));
  assert.match(h.alerts[0]!.text, /You hold the reactor! .* 1\.5 s/);
  const core = mode({ mode: "heist", teams: 2, teamScores: [0, 0], objectives: [{ id: 2, kind: OBJECTIVE.core, team: -1, holder: 0, state: 0, timerEnd: 0, radius: 1, pos: { x: 0, y: 0, z: 0 } }] });
  assert.match(modeHudModel(hudInput({ mode: core })).alerts[0]!.text, /You carry the core/);

  // Spectating, warm-up, ended, instagib.
  h = modeHudModel(hudInput({ mode: ffa, spectating: true, following: 3 }));
  assert.deepEqual(h.alerts.map((a) => [a.level, /Following Ada/.test(a.text)]), [["info", true]]);
  h = modeHudModel(hudInput({ mode: ffa, stage: "warmup" }));
  assert.equal(h.title, "Warm-up");
  assert.equal(h.timer, "");
  assert.equal(h.chips.length, 0);
  assert.equal(h.lives, -1);
  assert.equal(modeHudModel(hudInput({ mode: mode({ running: false }) })).timer, "");
  assert.equal(modeHudModel(hudInput({ timeLeft: 20 })).timerUrgent, true);
  assert.equal(modeHudModel(hudInput({ mode: mode({ flags: VIEW_FLAG.instagib }) })).instagib, true);
  assert.deepEqual([1, 2, 3, 4, 11, 12, 13, 21, 22, 101].map(ordinal), ["1st", "2nd", "3rd", "4th", "11th", "12th", "13th", "21st", "22nd", "101st"]);
});

test("objective markers: projected on screen, clamped to the edge behind or beside the camera", () => {
  // Camera at the origin looking down -Z.
  const proj = mat4.perspective(Math.PI / 2, 1, 0.1, 1000);
  const vp = mat4.multiply(proj, mat4.viewFromPose({ x: 0, y: 0, z: 0 }, q.identity()));
  const ahead = projectMarker(vp, { x: 0, y: 0, z: -10 }, 800, 800);
  assert.equal(ahead.edge, false);
  assert.ok(Math.abs(ahead.x - 400) < 1e-6 && Math.abs(ahead.y - 400) < 1e-6, "centre");
  const right = projectMarker(vp, { x: 100, y: 0, z: -10 }, 800, 800);
  assert.equal(right.edge, true);
  assert.ok(right.x > 700 && right.x <= 800, `${right.x}`);
  assert.ok(Math.abs(right.angle) < 1e-6, "arrow points right");
  const behind = projectMarker(vp, { x: -5, y: 0, z: 10 }, 800, 800);
  assert.equal(behind.edge, true);
  assert.ok(behind.x < 100, `behind-left stays on the left (${behind.x})`);

  const m = mode({
    teams: 2,
    objectives: [
      { id: 1, kind: OBJECTIVE.core, team: -1, holder: 3, state: 0, timerEnd: 0, radius: 1, pos: { x: 0, y: 0, z: -20 } },
      { id: 2, kind: OBJECTIVE.extraction, team: 1, holder: -1, state: 0, timerEnd: 0, radius: 5, pos: { x: 0, y: 0, z: -50 } },
      { id: 3, kind: OBJECTIVE.reactor, team: -1, holder: 0, state: 0, timerEnd: 0, radius: 1, pos: { x: 0, y: 0, z: -5 } },
      { id: 4, kind: OBJECTIVE.safeZone, team: -1, holder: -1, state: 0, timerEnd: 0, radius: 100, pos: { x: 0, y: 0, z: -30 } },
      { id: 5, kind: 255, team: -1, holder: -1, state: 0, timerEnd: 0, radius: 1, pos: { x: 0, y: 0, z: -5 } },
    ],
  });
  const markers = objectiveMarkers(m, 0, new Map([[3, "Ada"]]), 0, vp, { x: 0, y: 0, z: 0 }, 800, 800);
  assert.deepEqual(markers.map((k) => [k.id, k.label, k.detail]), [
    [1, "Core · Ada", "20 u"],
    [2, "Extraction", "50 u"],
    [4, "Safe zone", "inside"],
  ], "the own reactor and unknown kinds have no marker");
  assert.equal(markers[1]!.color, cssColor(TEAM_PALETTE[1]!));
  assert.equal(objectiveMarkers(mode({ running: false, objectives: m.objectives }), 0, new Map(), 0, vp, { x: 0, y: 0, z: 0 }, 800, 800).length, 0);
});

test("side colours: four distinct team colours, free-for-all sides above 3 are not neutral", () => {
  assert.equal(TEAM_PALETTE.length, 4);
  assert.deepEqual(sideColor(1, 2), TEAM_PALETTE[1]);
  assert.deepEqual(sideColor(3, 4), TEAM_PALETTE[3]);
  assert.deepEqual(sideColor(9, 4), NEUTRAL_SIDE, "no team 9");
  const ffa = Array.from({ length: 32 }, (_, s) => sideColor(s, 0));
  for (const c of ffa) assert.notDeepEqual(c, NEUTRAL_SIDE);
  const distinct = new Set(ffa.map((c) => c.map((v) => v.toFixed(3)).join(",")));
  assert.equal(distinct.size, 32, "32 free-for-all pilots, 32 colours");
  assert.deepEqual(ffaColor(0), FFA_PALETTE[0]);
  assert.deepEqual(ffaColor(254), NEUTRAL_SIDE);
  assert.equal(cssColor([1, 0.5, 0]), "rgb(255, 128, 0)");
  assert.equal(TEAM_HULL, TEAM_PALETTE);

  // The scene colours free-for-all ships by side.
  const ships = parseShips(new Float32Array([5, 5, 1, 0, 0, 0, 0, 0, 0, 0, 1, 1, 0.5, 0, 6, 6, 1, 0, 3, 0, 0, 0, 0, 0, 1, 1, 0.5, 0]));
  const scene = buildScene(ships, [], [], [], () => undefined, 0, { teams: 0 });
  const hulls = scene.opaque["ship"] ?? [];
  assert.equal(hulls.length, 2);
  assert.deepEqual(hulls[0]!.color, ffaColor(5));
  assert.notDeepEqual(hulls[0]!.color, hulls[1]!.color);
  const teamScene = buildScene(ships, [], [], [], () => undefined, 0);
  assert.deepEqual((teamScene.opaque["ship"] ?? [])[0]!.color, NEUTRAL_SIDE, "side 5 in a team match is neutral");
});

function input(over: Partial<InputState>): InputState {
  return { ...emptyInput(), pointerLocked: true, ...over };
}

test("spectator camera: free flight with mouse look, follows ships on click", () => {
  const cam = new SpectatorCamera({ position: { x: 0, y: 0, z: 0 }, orientation: q.identity() });
  let v = cam.update(1, input({ thrustForward: 1 }), [], 0.01);
  assert.ok(Math.abs(v.position.z + 22) < 1e-6, `flies forward (-Z): ${v.position.z}`);
  v = cam.update(0.5, input({ thrustForward: 1, boost: true }), [], 0.01);
  assert.ok(Math.abs(v.position.z + 22 + 33) < 1e-6, "boost triples the speed");
  // Mouse right turns right: forward swings towards +X.
  v = cam.update(0, input({ lookX: 100 }), [], 0.01);
  assert.ok(q.forward(v.orientation).x > 0.5);
  // Bounds keep it near the level.
  cam.update(100, input({ thrustForward: 1 }), [], 0, [-10, -10, -10, 10, 10, 10]);
  assert.ok(Math.abs(cam.position.x) <= 30 && Math.abs(cam.position.z) <= 30);

  const ships = parseShips(new Float32Array([4, 4, 1, 0, 100, 0, 0, 0, 0, 0, 1, 1, 0.5, 0, 2, 2, 1, 0, -100, 0, 0, 0, 0, 0, 1, 1, 0.5, 0]));
  cam.update(0.1, input({ firePrimary: true }), ships, 0.01);
  assert.equal(cam.following, 2, "left click follows the first ship by id");
  cam.update(0.1, input({ firePrimary: true }), ships, 0.01);
  assert.equal(cam.following, 2, "holding does not cycle");
  cam.update(0.1, input({}), ships, 0.01);
  for (let n = 0; n < 60; n++) cam.update(0.05, input({ firePrimary: n === 0 }), ships, 0.01);
  assert.equal(cam.following, 4, "the next click follows the next ship");
  assert.ok(Math.abs(cam.position.x - 100) < 1 && cam.position.z > 4, `chases from behind: ${JSON.stringify(cam.position)}`);
  cam.update(0.1, input({ fireSecondary: true }), ships, 0.01);
  assert.equal(cam.following, null, "right click: free camera");
  cam.update(0.1, input({ firePrimary: true }), [], 0.01);
  assert.equal(cam.following, null, "nobody to follow");
});

class Link implements PacketLink {
  isOpen = true;
  closeReason: string | null = null;
  inbox: Uint8Array[] = [];
  sent: Array<[number, boolean]> = [];
  send(p: Uint8Array, droppable = false): void {
    this.sent.push([p[0] ?? -1, droppable]);
  }
  drain(): Uint8Array[] {
    return this.inbox.splice(0);
  }
  close(): void {
    this.isOpen = false;
  }
}

/** A core that switches to the next match when it receives packet 9. */
class TransitionGame extends GameStub {
  matches = 0;
  pending = false;
  lobbyText: string | undefined = undefined;
  override handlePacket(p: Uint8Array): string | undefined {
    if (p[0] === 9) {
      this.matches++;
      this.pending = true;
    }
    return undefined;
  }
  override visuals(): Float32Array {
    // One visual per match so far (shape 0, white, scale 1).
    return new Float32Array(Array.from({ length: this.matches + 1 }, () => [0, 1, 1, 1, 1]).flat());
  }
  takeMatchTransition(): boolean {
    const t = this.pending;
    this.pending = false;
    return t;
  }
  modeState(): string {
    return MODE_JSON;
  }
  lobbyState(): string | undefined {
    return this.lobbyText;
  }
  results(): string | undefined {
    return undefined;
  }
  readyPacket(ready: boolean): Uint8Array {
    return new Uint8Array([ready ? 21 : 20]);
  }
  pickTeamPacket(team: number): Uint8Array {
    return new Uint8Array([30 + team]);
  }
  spectatePacket(s: boolean): Uint8Array {
    return new Uint8Array([s ? 41 : 40]);
  }
}

test("session: match transitions on one connection refresh visuals and survive hidden-tab frames", () => {
  const game = new TransitionGame();
  const link = new Link();
  const s = new GameSession(game, link, { entityKinds: [], shapes: ["bolt"] });
  assert.equal(s.visuals.length, 1);
  let r = s.frame(1 / 60, input({}));
  assert.equal(r.transition, false);
  assert.equal(r.mode.mode, "hot_reactor");
  assert.equal(r.lobby, null);
  assert.equal(r.results, null);

  // Three transitions in a row, no reconnect: each reported once.
  for (let n = 1; n <= 3; n++) {
    link.inbox.push(new Uint8Array([9]));
    r = s.frame(1 / 60, input({}));
    assert.equal(r.transition, true, `transition ${n}`);
    assert.equal(r.disconnected, null);
    assert.equal(s.matchCount, n);
    assert.equal(s.visuals.length, n + 1, "visual table of the new match");
    assert.equal(s.frame(1 / 60, input({})).transition, false, "reported once");
  }
  // A transition during a hidden-tab frame is kept for the next real frame.
  link.inbox.push(new Uint8Array([9]));
  assert.equal(s.idleFrame(0.25), null);
  assert.equal(s.frame(1 / 60, input({})).transition, true);

  // Lobby requests go out reliably.
  game.lobbyText = LOBBY_JSON;
  assert.equal(s.frame(1 / 60, input({})).lobby?.phase, "countdown");
  link.sent = [];
  assert.equal(s.setReady(true), true);
  assert.equal(s.pickTeam(1), true);
  assert.equal(s.setSpectate(false), true);
  assert.deepEqual(link.sent, [
    [21, false],
    [31, false],
    [40, false],
  ]);
  s.dispose();
  assert.equal(s.setReady(true), false, "nothing is sent after dispose");
});

test("session: practice has no lobby and sends nothing", () => {
  const game = new GameStub();
  game.isOnline = false;
  const s = new GameSession(game, null, { entityKinds: [], shapes: [] });
  const r = s.frame(1 / 60, input({}));
  assert.equal(r.transition, false);
  assert.deepEqual(r.mode, emptyModeState(), "a core without modeState gives the empty state");
  assert.equal(s.setReady(true), false);
});
