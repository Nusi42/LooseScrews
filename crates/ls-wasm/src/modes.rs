//! Mode state, lobby state and match results for the browser (protocol 5,
//! M3) as JSON, so the mode HUD, lobby overlay and results screen read one
//! object each. Field names are camelCase; ids are wire ship ids online.
//!
//! - [`mode_state_json`]: `{kind, mode, teams, running, winner, endTick,
//!   stage, stageEndTick, scoreTotal, scoreTarget, flags, teamScores: [n],
//!   players: [{id, side, score, lives, meter, flags}], objectives: [{id,
//!   kind, team, holder, state, timerEnd, radius, pos: [x, y, z]}]}`;
//!   `winner`, `holder` and `team` are -1 for none, `lives` -1 for no limit,
//!   `mode` is the mode id (`team_brawl`, ...). Protocol 6 adds `mood`:
//!   `{mood, moodId ("calm" | "bored" | "grumpy" | "giddy"), event (-1 =
//!   none), eventId ("" = none), phase, phaseId ("announce" | "active" |
//!   "ending" | "" without an event), target (-1 = none), activeTick,
//!   phaseEndTick, params: [8 bytes], up: [x, y, z]}` (the comedy
//!   director's state and the level's up, `ls_net::NetMoodView`), and
//!   `race` (Belt Up, `null` in other modes): `{heat, heats, phase,
//!   phaseId ("lights" | "racing" | "results" | "done"), phaseEndTick,
//!   racers: [{id, gates, nextGate (-1 once the lap is done), finishTick,
//!   place, holdEndTick, slot}]}` (`ls_net::NetRaceView`; `gates` 8 = the
//!   exit is open; ticks 0 = none).
//! - [`lobby_json`]: `{phase ("lobby" | "countdown" | "match" |
//!   "results"), phaseTicksLeft, roomCode, nextMode, nextModeId,
//!   nextRuleset, nextShareCode, minPlayers, maxPlayers, teams, players:
//!   [{id, team, ready, spectator}]}` (`team` -1 = auto).
//! - [`results_json`]: `{tick, kind, mode, teams, winner, teamScores,
//!   rows: [{id, side, kills, deaths, score, name}], awards: [{event,
//!   player, value}]}` (rows best first; awards: titles comedy events gave,
//!   `event` = the event id such as `"hat_rain"` for the Hat Baron, protocol
//!   6).
//! - [`architect_rules_json`] (M4 C4): `{costs: [8, by stroke index],
//!   minRadius, maxRadius, cooldownSecs, telegraphSecs, maxStrokes,
//!   budgetStart, budgetPerSec, budgetMax, keepClear, wellSecs,
//!   paletteSecs, roundSecs, roundResultsSecs, runnerLives, absurdity (the
//!   rule set's cap; 0 refuses the event strokes), spawns: [[x, y, z]]}`.

use ls_net::mode::{NONE, UNLIMITED_LIVES};
use ls_net::{LobbyPhase, LobbyStateMsg, MatchResultsMsg, NetModeState, NetMoodView, NetRaceView};
use ls_sim::ModeKind;

use crate::json_str;

fn opt(v: u8, none: u8) -> String {
    if v == none {
        "-1".into()
    } else {
        v.to_string()
    }
}

fn num(v: f32) -> String {
    if v.is_finite() {
        format!("{}", (v * 1000.0).round() / 1000.0)
    } else {
        "0".into()
    }
}

fn mode_id(kind: u8) -> String {
    json_str(ModeKind::from_index(kind).map_or("", |k| k.id()))
}

fn object(fields: &[(&str, String)]) -> String {
    let body: Vec<String> = fields.iter().map(|(k, v)| format!("\"{k}\":{v}")).collect();
    format!("{{{}}}", body.join(","))
}

fn array(items: impl IntoIterator<Item = String>) -> String {
    format!("[{}]", items.into_iter().collect::<Vec<_>>().join(","))
}

/// See the module docs.
pub fn mode_state_json(m: &NetModeState) -> String {
    let players = m.players.iter().map(|p| {
        object(&[
            ("id", p.id.to_string()),
            ("side", p.side.to_string()),
            ("score", p.score.to_string()),
            ("lives", opt(p.lives, UNLIMITED_LIVES)),
            ("meter", p.meter.to_string()),
            ("flags", p.flags.to_string()),
        ])
    });
    let objectives = m.objectives.iter().map(|o| {
        object(&[
            ("id", o.id.to_string()),
            ("kind", o.kind.to_string()),
            ("team", opt(o.team, NONE)),
            ("holder", opt(o.holder, NONE)),
            ("state", o.state.to_string()),
            ("timerEnd", o.timer_end.to_string()),
            ("radius", num(o.radius)),
            ("pos", array([num(o.pos.x), num(o.pos.y), num(o.pos.z)])),
        ])
    });
    object(&[
        ("kind", m.kind.to_string()),
        ("mode", mode_id(m.kind)),
        ("teams", m.teams.to_string()),
        ("running", m.is_running().to_string()),
        ("winner", opt(m.winner, NONE)),
        ("endTick", m.end_tick.to_string()),
        ("stage", m.stage.to_string()),
        ("stageEndTick", m.stage_end_tick.to_string()),
        ("scoreTotal", m.score_total.to_string()),
        ("scoreTarget", m.score_target.to_string()),
        ("flags", m.flags.to_string()),
        (
            "teamScores",
            array(m.team_scores.iter().map(|s| s.to_string())),
        ),
        ("players", array(players)),
        ("objectives", array(objectives)),
        ("mood", mood_json(&m.mood)),
        ("race", m.race.as_ref().map_or("null".into(), race_json)),
    ])
}

/// Heat phase ids by `ls_net::race_phase` value.
pub const RACE_PHASE_IDS: [&str; 4] = ["lights", "racing", "results", "done"];

/// The `race` object of [`mode_state_json`] (Belt Up).
pub fn race_json(r: &NetRaceView) -> String {
    let racers = r.racers.iter().map(|x| {
        let next = if x.gates < 8 {
            ((x.slot as u32 + 1 + x.gates as u32) % 8).to_string()
        } else {
            "-1".into()
        };
        object(&[
            ("id", x.id.to_string()),
            ("gates", x.gates.to_string()),
            ("nextGate", next),
            ("finishTick", x.finish_tick.to_string()),
            ("place", x.place.to_string()),
            ("holdEndTick", x.hold_end_tick.to_string()),
            ("slot", x.slot.to_string()),
        ])
    });
    object(&[
        ("heat", r.heat.to_string()),
        ("heats", r.heats.to_string()),
        ("phase", r.phase.to_string()),
        (
            "phaseId",
            json_str(RACE_PHASE_IDS.get(r.phase as usize).copied().unwrap_or("")),
        ),
        ("phaseEndTick", r.phase_end_tick.to_string()),
        ("racers", array(racers)),
    ])
}

/// Mood ids by index (`ls_sim` moods, append only).
pub const MOOD_IDS: [&str; 4] = ["calm", "bored", "grumpy", "giddy"];
/// Event phase ids by `ls_net::mood_phase` value.
pub const MOOD_PHASE_IDS: [&str; 3] = ["announce", "active", "ending"];

/// The `mood` object of [`mode_state_json`].
pub fn mood_json(m: &NetMoodView) -> String {
    let up = |v: f32| {
        if v.is_finite() {
            format!("{v}")
        } else {
            "0".into()
        }
    };
    object(&[
        ("mood", m.mood.to_string()),
        (
            "moodId",
            json_str(MOOD_IDS.get(m.mood as usize).copied().unwrap_or("")),
        ),
        ("event", opt(m.event, NONE)),
        ("eventId", json_str(&crate::event_id(m.event))),
        ("phase", m.phase.to_string()),
        (
            "phaseId",
            json_str(if m.event == NONE {
                ""
            } else {
                MOOD_PHASE_IDS.get(m.phase as usize).copied().unwrap_or("")
            }),
        ),
        ("target", opt(m.target, NONE)),
        ("activeTick", m.active_tick.to_string()),
        ("phaseEndTick", m.phase_end_tick.to_string()),
        ("params", array(m.params.iter().map(|b| b.to_string()))),
        ("up", array([up(m.up.x), up(m.up.y), up(m.up.z)])),
    ])
}

/// See the module docs.
pub fn lobby_json(l: &LobbyStateMsg) -> String {
    let phase = match l.phase {
        LobbyPhase::Lobby => "lobby",
        LobbyPhase::Countdown => "countdown",
        LobbyPhase::Match => "match",
        LobbyPhase::Results => "results",
    };
    let players = l.players.iter().map(|p| {
        object(&[
            ("id", p.id.to_string()),
            ("team", opt(p.team, NONE)),
            ("ready", p.ready.to_string()),
            ("spectator", p.spectator.to_string()),
        ])
    });
    object(&[
        ("phase", json_str(phase)),
        ("phaseTicksLeft", l.phase_ticks_left.to_string()),
        ("roomCode", json_str(&l.room_code)),
        ("nextMode", l.next_mode.to_string()),
        ("nextModeId", mode_id(l.next_mode)),
        ("nextRuleset", json_str(&l.next_ruleset)),
        ("nextShareCode", json_str(&l.next_share_code)),
        ("minPlayers", l.min_players.to_string()),
        ("maxPlayers", l.max_players.to_string()),
        ("teams", l.teams.to_string()),
        ("players", array(players)),
    ])
}

/// See the module docs.
pub fn results_json(r: &MatchResultsMsg) -> String {
    let rows = r.rows.iter().map(|row| {
        object(&[
            ("id", row.id.to_string()),
            ("side", row.side.to_string()),
            ("kills", row.kills.to_string()),
            ("deaths", row.deaths.to_string()),
            ("score", row.score.to_string()),
            ("name", json_str(&row.name)),
        ])
    });
    let awards = r.awards.iter().map(|a| {
        let id = ls_sim::EventKind::from_index(a.event).map_or("", |k| k.id());
        object(&[
            ("event", json_str(id)),
            ("player", a.player.to_string()),
            ("value", a.value.to_string()),
        ])
    });
    object(&[
        ("tick", r.tick.to_string()),
        ("kind", r.kind.to_string()),
        ("mode", mode_id(r.kind)),
        ("teams", r.teams.to_string()),
        ("winner", opt(r.winner, NONE)),
        (
            "teamScores",
            array(r.team_scores.iter().map(|s| s.to_string())),
        ),
        ("rows", array(rows)),
        ("awards", array(awards)),
    ])
}

/// See the module docs.
pub fn architect_rules_json(
    r: &ls_sim::ruleset::TheArchitectRules,
    absurdity: u8,
    spawns: &[ls_math::Vec3],
) -> String {
    use ls_sim::modes::the_architect::StrokeKind;
    let costs = StrokeKind::ALL.iter().map(|k| k.cost(r).to_string());
    let spawns = spawns
        .iter()
        .map(|p| format!("[{},{},{}]", num(p.x), num(p.y), num(p.z)));
    object(&[
        ("costs", array(costs)),
        ("minRadius", num(r.min_radius)),
        ("maxRadius", num(r.max_radius)),
        ("cooldownSecs", num(r.stroke_cooldown_secs)),
        ("telegraphSecs", num(r.telegraph_secs)),
        ("maxStrokes", r.max_strokes.to_string()),
        ("budgetStart", r.budget_start.to_string()),
        ("budgetPerSec", num(r.budget_per_sec)),
        ("budgetMax", r.budget_max.to_string()),
        ("keepClear", num(r.keep_clear)),
        ("wellSecs", num(r.well_secs)),
        ("paletteSecs", num(r.palette_secs)),
        ("roundSecs", num(r.round_secs)),
        ("roundResultsSecs", num(r.round_results_secs)),
        ("runnerLives", r.runner_lives.to_string()),
        ("absurdity", absurdity.to_string()),
        ("spawns", array(spawns)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use ls_math::Vec3;
    use ls_net::{LobbyPlayer, NetModePlayer, NetObjective, ResultRow};
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn race_as_json() {
        let m = NetModeState {
            kind: ModeKind::BeltUp.index(),
            race: Some(NetRaceView {
                heat: 1,
                heats: 3,
                phase: ls_net::race_phase::RACING,
                phase_end_tick: 9000,
                racers: vec![
                    ls_net::NetRacer {
                        id: 2,
                        gates: 3,
                        finish_tick: 0,
                        place: 1,
                        hold_end_tick: 0,
                        slot: 6,
                    },
                    ls_net::NetRacer {
                        id: 4,
                        gates: 8,
                        finish_tick: 8800,
                        place: 2,
                        hold_end_tick: 8700,
                        slot: 0,
                    },
                ],
            }),
            ..Default::default()
        };
        let j = mode_state_json(&m);
        assert!(j.contains("\"mode\":\"belt_up\""), "{j}");
        assert!(
            j.ends_with(
                "\"race\":{\"heat\":1,\"heats\":3,\"phase\":1,\"phaseId\":\"racing\",\"phaseEndTick\":9000,\"racers\":[{\"id\":2,\"gates\":3,\"nextGate\":2,\"finishTick\":0,\"place\":1,\"holdEndTick\":0,\"slot\":6},{\"id\":4,\"gates\":8,\"nextGate\":-1,\"finishTick\":8800,\"place\":2,\"holdEndTick\":8700,\"slot\":0}]}}"
            ),
            "{j}"
        );
        assert!(mode_state_json(&NetModeState::default()).ends_with("\"race\":null}"));
    }

    #[test]
    fn mode_state_as_json() {
        let m = NetModeState {
            kind: ModeKind::HotReactor.index(),
            teams: 0,
            phase: ls_net::mode_phase::RUNNING,
            winner: NONE,
            end_tick: 36_000,
            team_scores: vec![],
            players: vec![NetModePlayer {
                id: 3,
                side: 3,
                score: -1,
                lives: UNLIMITED_LIVES,
                meter: 50,
                flags: 4,
            }],
            objectives: vec![NetObjective {
                id: 1,
                kind: 4,
                team: NONE,
                holder: 3,
                state: 0,
                timer_end: 1200,
                radius: 1.5,
                pos: Vec3::new(1.0, -2.5, 3.25),
            }],
            ..Default::default()
        };
        let j = mode_state_json(&m);
        assert!(
            j.starts_with("{\"kind\":4,\"mode\":\"hot_reactor\",\"teams\":0,"),
            "{j}"
        );
        assert!(j.contains("\"running\":true,\"winner\":-1,"), "{j}");
        assert!(j.contains(
            "\"players\":[{\"id\":3,\"side\":3,\"score\":-1,\"lives\":-1,\"meter\":50,\"flags\":4}]"
        ));
        assert!(j.contains("\"holder\":3,"));
        assert!(j.contains("\"team\":-1,"));
        assert!(j.contains("\"radius\":1.5,\"pos\":[1,-2.5,3.25]"), "{j}");
        assert!(j.contains("\"teamScores\":[]"));
    }

    #[test]
    fn architect_rules_as_json() {
        let r = ls_sim::ruleset::TheArchitectRules::default();
        let j = architect_rules_json(&r, 100, &[ls_math::Vec3::new(1.0, 2.5, -3.0)]);
        assert!(j.contains("\"costs\":[25,15,20,30,35,60,15,80]"), "{j}");
        assert!(j.contains("\"minRadius\":12,\"maxRadius\":24,"), "{j}");
        assert!(
            j.contains("\"absurdity\":100,\"spawns\":[[1,2.5,-3]]"),
            "{j}"
        );
    }

    #[test]
    fn lobby_and_results_as_json() {
        let l = LobbyStateMsg {
            phase: LobbyPhase::Countdown,
            phase_ticks_left: 600,
            room_code: "R-\"x\"".into(),
            next_mode: ModeKind::TeamBrawl.index(),
            next_ruleset: "team_brawl".into(),
            next_share_code: "LS-1".into(),
            min_players: 4,
            max_players: 32,
            teams: 2,
            players: vec![LobbyPlayer {
                id: 0,
                team: NONE,
                ready: true,
                spectator: false,
            }],
        };
        let j = lobby_json(&l);
        assert!(j.starts_with("{\"phase\":\"countdown\",\"phaseTicksLeft\":600,"));
        assert!(j.contains("\"roomCode\":\"R-\\\"x\\\"\""), "{j}");
        assert!(j.contains("\"nextMode\":1,\"nextModeId\":\"team_brawl\""));
        assert!(j.contains("{\"id\":0,\"team\":-1,\"ready\":true,\"spectator\":false}"));
        let r = MatchResultsMsg {
            tick: 99,
            kind: 1,
            teams: 2,
            winner: 1,
            team_scores: vec![3, 15],
            rows: vec![ResultRow {
                id: 2,
                side: 1,
                kills: 7,
                deaths: 1,
                score: 7,
                name: "Rita".into(),
            }],
            awards: vec![ls_net::NetAward {
                event: ls_sim::EventKind::HatRain.index(),
                player: 2,
                value: 5,
            }],
        };
        let j = results_json(&r);
        assert_eq!(
            j,
            "{\"tick\":99,\"kind\":1,\"mode\":\"team_brawl\",\"teams\":2,\"winner\":1,\
             \"teamScores\":[3,15],\"rows\":[{\"id\":2,\"side\":1,\"kills\":7,\"deaths\":1,\
             \"score\":7,\"name\":\"Rita\"}],\"awards\":[{\"event\":\"hat_rain\",\"player\":2,\"value\":5}]}"
        );
    }
}
