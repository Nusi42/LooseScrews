//! Mode state and results on the wire (protocol 5): `ls_sim::ModeView`
//! (`World::mode_view`) -> [`ls_net::NetModeState`] and
//! `ls_sim::MatchResults` (`World::results`) -> [`ls_net::MatchResultsMsg`].
//! Everything here is mode-agnostic: the room and the client only see the
//! generic view (`docs/milestone-3-plan.md` §3.1).

use ls_net::mode::NONE;
use ls_net::NetMoodView;
use ls_net::{
    mode_phase, MatchResultsMsg, NetModePlayer, NetModeState, NetObjective, ResultRow, MAX_SHIPS,
};
use ls_sim::mode::ObjectiveView;
use ls_sim::modes::the_squeeze;
use ls_sim::{
    EventKind, EventPhase, MatchPhase, MatchResults, ModeKind, ModeView, Mood, MoodView, PlayerId,
    ShellSphere,
};

fn tick32(t: u64) -> u32 {
    t.min(u32::MAX as u64) as u32
}

/// The wire form of the comedy director's view (protocol 6) with the
/// level's `up` (`World::up`). `wire` maps the target to its wire id.
pub fn mood_to_net(
    m: &MoodView,
    up: ls_math::Vec3,
    wire: impl Fn(PlayerId) -> Option<u8>,
) -> NetMoodView {
    NetMoodView {
        mood: m.mood.index(),
        event: m.event.map_or(NONE, |k| k.index()),
        phase: m.phase.index(),
        target: m.target.and_then(wire).unwrap_or(NONE),
        active_tick: tick32(m.active_tick),
        phase_end_tick: tick32(m.phase_end_tick),
        params: m.params,
        up: if up.is_finite() { up } else { ls_math::Vec3::Y },
    }
}

/// The simulation's view of a replicated mood (client predictor: event
/// movement effects are pure functions of it). The target becomes
/// `PlayerId(wire id)`, the id the client gives its own ship.
pub fn mood_from_net(n: &NetMoodView) -> MoodView {
    MoodView {
        mood: Mood::from_index(n.mood).unwrap_or_default(),
        event: EventKind::from_index(n.event),
        phase: EventPhase::from_index(n.phase).unwrap_or_default(),
        phase_end_tick: n.phase_end_tick as u64,
        active_tick: n.active_tick as u64,
        target: (n.target != NONE).then_some(PlayerId(n.target as u16)),
        params: n.params,
    }
}

/// The wire form of a mode view. `wire` maps simulation players to wire
/// ids; players without one (not connected) are left out. The mood's `up`
/// is world +Y here; the room and the practice core set the world's.
pub fn mode_view_to_net(view: &ModeView, wire: impl Fn(PlayerId) -> Option<u8>) -> NetModeState {
    let (phase, winner) = match view.phase {
        MatchPhase::Running => (mode_phase::RUNNING, NONE),
        MatchPhase::Ended { winner } => (mode_phase::ENDED, winner.unwrap_or(NONE)),
    };
    let mut players: Vec<NetModePlayer> = view
        .players
        .iter()
        .filter_map(|p| {
            let id = wire(p.player).filter(|&id| id < MAX_SHIPS)?;
            Some(NetModePlayer {
                id,
                side: p.side,
                score: p.score,
                lives: p.lives,
                meter: p.meter,
                flags: p.flags,
            })
        })
        .collect();
    players.sort_by_key(|p| p.id);
    players.dedup_by_key(|p| p.id);
    let mut objectives: Vec<NetObjective> = view
        .objectives
        .iter()
        .take(ls_net::MAX_OBJECTIVES)
        .map(|o| NetObjective {
            id: o.id,
            kind: o.kind,
            team: o.team,
            holder: o.holder.and_then(&wire).unwrap_or(NONE),
            state: o.state,
            timer_end: tick32(o.timer_end),
            radius: if o.radius.is_finite() { o.radius } else { 0.0 },
            pos: o.pos,
        })
        .collect();
    objectives.sort_by_key(|o| o.id);
    objectives.dedup_by_key(|o| o.id);
    NetModeState {
        kind: view.kind.index(),
        teams: view.teams,
        phase,
        winner,
        end_tick: tick32(view.end_tick),
        stage: view.stage,
        stage_end_tick: tick32(view.stage_end_tick),
        score_total: view.score_total,
        score_target: view.score_target,
        flags: view.flags,
        team_scores: view
            .team_scores
            .iter()
            .take(ls_net::MAX_MODE_TEAMS)
            .copied()
            .collect(),
        mood: mood_to_net(&view.mood, ls_math::Vec3::Y, &wire),
        players,
        objectives,
    }
}

/// The `Snapshot::scores` list of a view (kept for protocol-4-era readers):
/// the team scores (clamped to `0..=u32::MAX`), then `score_total`. Paint
/// the Void with two teams gives `[team 0 cells, team 1 cells, surface
/// cells]` exactly as before.
pub fn legacy_scores(view: &ModeView) -> Vec<u32> {
    view.team_scores
        .iter()
        .map(|&s| s.clamp(0, u32::MAX as i64) as u32)
        .chain(std::iter::once(view.score_total.min(u32::MAX as u64) as u32))
        .collect()
}

/// The wire form of match results at `tick`. `wire` maps players to wire
/// ids and `name` gives a wire id's name; rows of players without a wire id
/// (left the match) are left out.
pub fn results_to_net(
    results: &MatchResults,
    tick: u64,
    wire: impl Fn(PlayerId) -> Option<u8>,
    name: impl Fn(u8) -> String,
) -> MatchResultsMsg {
    MatchResultsMsg {
        tick: tick32(tick),
        kind: results.kind.index(),
        teams: results.teams,
        winner: results.winner.unwrap_or(NONE),
        team_scores: results
            .team_scores
            .iter()
            .take(ls_net::MAX_MODE_TEAMS)
            .copied()
            .collect(),
        rows: results
            .rows
            .iter()
            .filter_map(|r| {
                let id = wire(r.player).filter(|&id| id < MAX_SHIPS)?;
                let mut n = name(id);
                truncate_utf8(&mut n, ls_net::MAX_PLAYER_NAME);
                Some(ResultRow {
                    id,
                    side: r.side,
                    kills: r.kills,
                    deaths: r.deaths,
                    score: r.score,
                    name: n,
                })
            })
            .take(MAX_SHIPS as usize)
            .collect(),
    }
}

/// The Squeeze's wall at tick `tick` from a replicated mode state (`None`
/// in other modes or before the walls rise). Zone centres are whole units
/// on the server (`ls_sim::modes::the_squeeze`), so snapping the quantized
/// marker positions recovers them exactly and the client's shell equals the
/// server's at the same tick.
pub fn squeeze_shell(mode: &NetModeState, tick: u64) -> Option<ShellSphere> {
    if mode.kind != ModeKind::TheSqueeze.index() || mode == &NetModeState::default() {
        return None;
    }
    let views: Vec<ObjectiveView> = mode
        .objectives
        .iter()
        .map(|o| ObjectiveView {
            id: o.id,
            kind: o.kind,
            pos: the_squeeze::snap(o.pos),
            radius: o.radius,
            holder: None,
            timer_end: o.timer_end as u64,
            team: o.team,
            state: o.state,
        })
        .collect();
    the_squeeze::ShellSchedule::from_objectives(&views)?.shell_at(tick)
}

fn truncate_utf8(s: &mut String, max: usize) {
    if s.len() > max {
        let mut cut = max;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ls_math::Vec3;
    use ls_sim::mode::view_flags;
    use ls_sim::mode::{objective_kind, player_flags, NO_SIDE};
    use ls_sim::{ModeKind, ObjectiveView, PlayerModeView};
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn view() -> ModeView {
        ModeView {
            kind: ModeKind::HotReactor,
            teams: 0,
            phase: MatchPhase::Ended { winner: Some(7) },
            end_tick: 36_000,
            stage: 1,
            stage_end_tick: 400,
            team_scores: vec![],
            score_total: 0,
            score_target: 3,
            players: vec![
                PlayerModeView {
                    player: PlayerId(7),
                    side: 7,
                    score: 5,
                    lives: 2,
                    meter: 0,
                    flags: player_flags::CARRIER,
                },
                PlayerModeView {
                    player: PlayerId(3),
                    side: 3,
                    score: 1,
                    lives: 0,
                    meter: 0,
                    flags: player_flags::ELIMINATED,
                },
                PlayerModeView {
                    player: PlayerId(9),
                    side: 9,
                    score: 0,
                    lives: 3,
                    meter: 0,
                    flags: 0,
                },
            ],
            objectives: vec![ObjectiveView {
                id: 2,
                kind: objective_kind::REACTOR,
                pos: Vec3::new(10.0, 20.0, 30.0),
                radius: 15.0,
                holder: Some(PlayerId(7)),
                timer_end: 1234,
                team: NO_SIDE,
                state: 0,
            }],
            flags: view_flags::LIVES,
            mood: Default::default(),
        }
    }

    #[test]
    fn mode_view_maps_to_wire_ids() {
        // Player 9 is not connected (no wire id).
        let wire = |p: PlayerId| match p.0 {
            7 => Some(1),
            3 => Some(0),
            _ => None,
        };
        let net = mode_view_to_net(&view(), wire);
        assert_eq!(net.kind, ModeKind::HotReactor.index());
        assert_eq!((net.phase, net.winner), (mode_phase::ENDED, 7));
        assert_eq!(net.players.len(), 2);
        assert_eq!(net.players[0].id, 0, "sorted by wire id");
        assert_eq!(net.player(1).unwrap().score, 5);
        assert_eq!(net.player(1).unwrap().flags, player_flags::CARRIER);
        assert_eq!(net.objectives[0].holder, 1);
        assert_eq!(net.objectives[0].timer_end, 1234);
        assert_eq!(net.flags, view_flags::LIVES);
        // Through the wire and back.
        let q = ls_net::Quantizer::default();
        let back = q.unpack_mode(&q.pack_mode(&net));
        assert_eq!(back.players, net.players);
        assert!((back.objectives[0].pos - net.objectives[0].pos).length() < 0.01);
        // Running, draw, no holder.
        let mut v = view();
        v.phase = MatchPhase::Running;
        v.objectives[0].holder = None;
        let net = mode_view_to_net(&v, wire);
        assert_eq!((net.phase, net.winner), (mode_phase::RUNNING, NONE));
        assert_eq!(net.objectives[0].holder, NONE);
    }

    #[test]
    fn results_to_wire() {
        let r = MatchResults {
            kind: ModeKind::TeamBrawl,
            teams: 2,
            ended: true,
            winner: None,
            team_scores: vec![10, 10],
            rows: vec![
                ls_sim::ResultRow {
                    player: PlayerId(4),
                    side: 1,
                    kills: 10,
                    deaths: 3,
                    score: 10,
                },
                ls_sim::ResultRow {
                    player: PlayerId(5),
                    side: 0,
                    kills: 1,
                    deaths: 9,
                    score: 1,
                },
            ],
        };
        let msg = results_to_net(
            &r,
            9000,
            |p| (p.0 == 4).then_some(2),
            |id| format!("pilot {id} with a very long name that goes on"),
        );
        assert_eq!(msg.winner, NONE, "draw");
        assert_eq!(msg.rows.len(), 1, "players who left are dropped");
        assert_eq!((msg.rows[0].id, msg.rows[0].kills), (2, 10));
        assert!(msg.rows[0].name.len() <= ls_net::MAX_PLAYER_NAME);
        let bytes = ls_net::Message::MatchResults(Box::new(msg.clone()))
            .encode()
            .unwrap();
        assert_eq!(
            ls_net::Message::decode(&bytes).unwrap(),
            ls_net::Message::MatchResults(Box::new(msg))
        );
    }
}
