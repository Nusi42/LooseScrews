//! Mode state and results on the wire (protocol 5): `ls_sim::ModeView` <->
//! [`ls_net::NetModeState`], `ls_sim::MatchResults` ->
//! [`ls_net::MatchResultsMsg`], and the kill/death tally the room keeps for
//! the results table. Everything here is mode-agnostic: the room and the
//! client only see the generic view (`docs/milestone-3-plan.md` §3.1).

use std::collections::BTreeMap;

use ls_net::mode::NONE;
use ls_net::{
    mode_phase, MatchResultsMsg, NetModePlayer, NetModeState, NetObjective, ResultRow, MAX_SHIPS,
};
use ls_sim::mode::{view_flags, MAX_OBJECTIVES, MAX_VIEW_PLAYERS};
use ls_sim::{
    Event, MatchPhase, MatchResults, ModeKind, ModeView, ObjectiveView, PlayerId, PlayerModeView,
    World,
};

/// The replicated mode state of `world`.
///
/// Built from `World::mode()` (the generic `ModeState`) the way
/// `ls_sim`'s `ModeView` documents it: players by score (highest first,
/// then by id, at most 32), the first 8 objectives, Paint the Void scores
/// as painted cells with the surface as `score_total`.
pub fn mode_view(world: &World) -> ModeView {
    let m = world.mode();
    let teams = m.teams;
    let paint = m.kind == ModeKind::PaintTheVoid;
    let n_teams = if paint {
        teams.clamp(2, ls_sim::mode::MAX_TEAMS)
    } else {
        teams.min(ls_sim::mode::MAX_TEAMS)
    } as usize;
    let team_scores: Vec<i64> = if paint {
        m.coverage.team_cells[..n_teams]
            .iter()
            .map(|&c| c.min(i64::MAX as u64) as i64)
            .collect()
    } else {
        m.team_scores[..n_teams].to_vec()
    };
    let mut players: Vec<PlayerModeView> = world
        .ships()
        .map(|s| {
            let pm = m.player(s.player).copied().unwrap_or_default();
            PlayerModeView {
                player: s.player,
                side: s.team,
                score: pm.score,
                lives: pm.lives,
                meter: pm.meter,
                flags: pm.flags,
            }
        })
        .collect();
    players.sort_by(|a, b| b.score.cmp(&a.score).then(a.player.cmp(&b.player)));
    players.truncate(MAX_VIEW_PLAYERS);
    let objectives: Vec<ObjectiveView> = m
        .objectives
        .iter()
        .take(MAX_OBJECTIVES)
        .map(|o| o.view())
        .collect();
    ModeView {
        kind: m.kind,
        teams,
        phase: m.phase,
        end_tick: m.end_tick,
        stage: m.stage,
        stage_end_tick: m.stage_end_tick,
        team_scores,
        score_total: if paint { m.coverage.surface_cells } else { 0 },
        score_target: m.score_target,
        players,
        objectives,
        flags: m.flags | if paint { view_flags::PAINT_SCORES } else { 0 },
    }
}

fn tick32(t: u64) -> u32 {
    t.min(u32::MAX as u64) as u32
}

/// The wire form of a mode view. `wire` maps simulation players to wire
/// ids; players without one (not connected) are left out.
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

/// Kills and deaths per player over a match, counted from the simulation's
/// `Killed` events (mode-agnostic; the room keeps one).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tally {
    entries: BTreeMap<PlayerId, (u32, u32)>,
}

impl Tally {
    /// Counts the kills and deaths among `events`. A suicide counts as a
    /// death, not a kill.
    pub fn record(&mut self, events: &[Event]) {
        for e in events {
            if let Event::Killed { victim, killer, .. } = e {
                self.entries.entry(*victim).or_default().1 += 1;
                if let Some(k) = killer.filter(|k| k != victim) {
                    self.entries.entry(k).or_default().0 += 1;
                }
            }
        }
    }

    pub fn kills(&self, p: PlayerId) -> u32 {
        self.entries.get(&p).map_or(0, |e| e.0)
    }

    pub fn deaths(&self, p: PlayerId) -> u32 {
        self.entries.get(&p).map_or(0, |e| e.1)
    }
}

/// The result of the match in `world`: winner and team scores from the
/// mode view, one row per player (best first: score, then kills, then
/// fewer deaths, then id) with kills and deaths from `tally`.
pub fn match_results(world: &World, tally: &Tally) -> MatchResults {
    let view = mode_view(world);
    let mut rows: Vec<ls_sim::ResultRow> = world
        .ships()
        .map(|s| ls_sim::ResultRow {
            player: s.player,
            side: s.team,
            kills: tally.kills(s.player),
            deaths: tally.deaths(s.player),
            score: world.mode().player(s.player).map_or(0, |p| p.score),
        })
        .collect();
    rows.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then(b.kills.cmp(&a.kills))
            .then(a.deaths.cmp(&b.deaths))
            .then(a.player.cmp(&b.player))
    });
    let (ended, winner) = match view.phase {
        MatchPhase::Running => (false, None),
        MatchPhase::Ended { winner } => (true, winner),
    };
    MatchResults {
        kind: view.kind,
        teams: view.teams,
        ended,
        winner,
        team_scores: view.team_scores,
        rows,
    }
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
    use ls_sim::mode::{objective_kind, player_flags, NO_SIDE};
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
    fn tally_counts_kills_and_deaths() {
        let mut t = Tally::default();
        let kill = |victim: u16, killer: Option<u16>| Event::Killed {
            victim: PlayerId(victim),
            killer: killer.map(PlayerId),
            weapon: None,
        };
        t.record(&[kill(1, Some(2)), kill(2, Some(2)), kill(3, None)]);
        t.record(&[kill(1, Some(2))]);
        assert_eq!((t.kills(PlayerId(2)), t.deaths(PlayerId(2))), (2, 1));
        assert_eq!((t.kills(PlayerId(1)), t.deaths(PlayerId(1))), (0, 2));
        assert_eq!(t.deaths(PlayerId(3)), 1);
        assert_eq!(t.kills(PlayerId(9)), 0);
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
