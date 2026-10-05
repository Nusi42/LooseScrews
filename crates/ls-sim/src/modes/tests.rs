//! Mode framework tests: every mode builds a world and ends, teams 0-4
//! spawn and balance, lives and elimination, spectators, generic scoring,
//! the replicated view and the results table.

use ls_math::terrain::BoxRoomTerrain;
use ls_math::{Quat, Vec3};

use crate::mode::{player_flags, view_flags, MatchPhase, ModeKind, LIVES_UNLIMITED};
use crate::modes::{module, LATE_JOIN_GRACE_TICKS, MODULES};
use crate::*;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

type Room = PaintedTerrain<BoxRoomTerrain>;

/// Two spawn points per team for four teams, around the room's centre.
fn setup() -> MatchSetup {
    let mut spawns = Vec::new();
    for team in 0..4u8 {
        for k in 0..2 {
            let a = team as f32 * 1.5 + k as f32 * 0.4;
            spawns.push(SpawnPoint {
                pos: Vec3::new(
                    20.0 * ls_math::det::cos(a),
                    k as f32 * 4.0,
                    20.0 * ls_math::det::sin(a),
                ),
                team,
                orientation: Quat::IDENTITY,
            });
        }
    }
    MatchSetup {
        spawns,
        pickup_points: vec![],
        instability: 0,
        heist: None,
    }
}

fn room() -> Room {
    PaintedTerrain::new(BoxRoomTerrain::centered(Vec3::new(60.0, 30.0, 60.0)), 1.0)
}

fn world(rules: RuleSet) -> World {
    let mut cfg = WorldConfig::paint_the_void(7, setup());
    cfg.rules = rules;
    World::new(cfg).unwrap()
}

fn rules_for(kind: ModeKind, teams: Option<u8>, match_secs: f32) -> RuleSet {
    let mut r = RuleSet::default_for(kind);
    r.mode.set_field("match_secs", match_secs);
    if let Some(t) = teams {
        r.mode.set_field("teams", t as f32);
    }
    r
}

/// Kills `victim` without an attacker (a suicide for scoring).
fn kill(w: &mut World, t: &mut Room, victim: PlayerId) {
    w.hook_for_test(t, |ctx| {
        ctx.push(Command::AdjustHull {
            target: victim,
            delta: -10_000.0,
            can_kill: true,
        })
    });
}

/// `from` hits `to` for `amount`.
fn hit(w: &mut World, t: &mut Room, from: PlayerId, to: PlayerId, amount: f32) {
    let src = DamageSource {
        attacker: Some(from),
        team: w.ship(from).unwrap().team,
        weapon: None,
    };
    w.hook_for_test(t, |ctx| ctx.damage(to, amount, Vec3::ZERO, src, true));
}

#[test]
fn registry_covers_every_mode_in_order() {
    assert_eq!(MODULES.len(), ModeKind::ALL.len());
    for (i, k) in ModeKind::ALL.iter().enumerate() {
        assert_eq!(MODULES[i].kind, *k);
        assert_eq!(module(*k).kind(), *k);
    }
    assert!(module(ModeKind::PaintTheVoid).implemented());
}

#[test]
fn every_mode_builds_a_world_and_ends_by_timer() {
    for kind in ModeKind::ALL {
        let rules = rules_for(kind, None, 4.0);
        let res = rules.resolve(&WeaponTable::builtin());
        assert!(res.is_ok(), "{kind:?}: {res:?}");
        let mut w = world(rules);
        assert_eq!(w.mode().kind, kind);
        let mut t = room();
        let ids: Vec<PlayerId> = (0..4)
            .map(|_| w.add_player(None, ShipType::Tinker).unwrap())
            .collect();
        assert!(ids.iter().all(|&id| w.ship(id).unwrap().alive), "{kind:?}");
        let mut ended = None;
        for _ in 0..4 * 60 + 1 {
            for e in w.step(&mut t, &[]) {
                if let Event::MatchEnded { winner, .. } = e {
                    assert!(ended.is_none(), "{kind:?} ended twice");
                    ended = Some((w.tick(), winner));
                }
            }
        }
        // Nobody scored: a draw at the timer.
        assert_eq!(ended, Some((4 * 60, None)), "{kind:?}");
        assert_eq!(w.mode().phase, MatchPhase::Ended { winner: None });
        assert!(
            w.step(&mut t, &[]).is_empty(),
            "{kind:?}: nothing moves after the end"
        );
        let res = w.results();
        assert!(res.ended && res.winner.is_none());
        assert_eq!(res.rows.len(), 4);
    }
}

#[test]
fn teams_two_to_four_balance_and_spawn_at_their_points() {
    for teams in 2..=4u8 {
        let mut w = world(rules_for(ModeKind::TeamBrawl, Some(teams), 60.0));
        assert_eq!(w.teams(), teams);
        let ids: Vec<PlayerId> = (0..9)
            .map(|_| w.add_player(None, ShipType::Tinker).unwrap())
            .collect();
        let mut counts = vec![0usize; teams as usize];
        for (i, id) in ids.iter().enumerate() {
            let s = w.ship(*id).unwrap();
            assert_eq!(s.team, (i % teams as usize) as u8, "the smallest team");
            counts[s.team as usize] += 1;
            let sp = setup()
                .spawns
                .into_iter()
                .find(|sp| sp.pos == s.pos)
                .expect("at a spawn point");
            assert_eq!(sp.team, s.team, "at its own team's spawn point");
        }
        let (min, max) = (counts.iter().min().unwrap(), counts.iter().max().unwrap());
        assert!(max - min <= 1, "{teams} teams: {counts:?}");
        // Explicit teams: in range only.
        assert!(w.add_player(Some(teams - 1), ShipType::Tinker).is_ok());
        assert!(matches!(
            w.add_player(Some(teams), ShipType::Tinker),
            Err(SimError::InvalidTeam(_))
        ));
        // The view has one score per team.
        assert_eq!(w.mode_view().team_scores.len(), teams as usize);
    }
}

#[test]
fn unsupported_team_counts_are_rejected() {
    let weapons = WeaponTable::builtin();
    for (kind, teams) in [
        (ModeKind::PaintTheVoid, 0u8),
        (ModeKind::PaintTheVoid, 1),
        (ModeKind::PaintTheVoid, 5),
        (ModeKind::TeamBrawl, 1),
        (ModeKind::TheSqueeze, 2),
        (ModeKind::Heist, 3),
        (ModeKind::HotReactor, 2),
        (ModeKind::LastColorStanding, 4),
    ] {
        let r = rules_for(kind, Some(teams), 60.0);
        assert!(r.resolve(&weapons).is_err(), "{kind:?} with {teams} teams");
    }
    for (kind, teams) in [
        (ModeKind::PaintTheVoid, 4u8),
        (ModeKind::TeamBrawl, 0),
        (ModeKind::TeamBrawl, 3),
        (ModeKind::Heist, 0),
    ] {
        let r = rules_for(kind, Some(teams), 60.0);
        assert!(r.resolve(&weapons).is_ok(), "{kind:?} with {teams} teams");
    }
}

#[test]
fn free_for_all_gives_every_ship_its_own_side() {
    let mut w = world(rules_for(ModeKind::TeamBrawl, Some(0), 60.0));
    assert_eq!(w.teams(), 0);
    let ids: Vec<PlayerId> = (0..5)
        .map(|_| w.add_player(None, ShipType::Tinker).unwrap())
        .collect();
    let sides: Vec<u8> = ids.iter().map(|id| w.ship(*id).unwrap().team).collect();
    assert_eq!(sides, vec![0, 1, 2, 3, 4]);
    // Spawns spread out: no two ships share a spawn point.
    for (i, a) in ids.iter().enumerate() {
        for b in &ids[i + 1..] {
            assert_ne!(w.ship(*a).unwrap().pos, w.ship(*b).unwrap().pos);
        }
    }
    // A freed side is reused; explicit teams are refused.
    w.remove_player(ids[1]);
    let again = w.add_player(None, ShipType::Tinker).unwrap();
    assert_eq!(w.ship(again).unwrap().team, 1);
    assert!(matches!(
        w.add_player(Some(0), ShipType::Tinker),
        Err(SimError::InvalidTeam(0))
    ));
    assert!(w.mode_view().team_scores.is_empty());
}

#[test]
fn free_for_all_keeps_a_departed_players_side_while_its_shots_live() {
    let mut w = world(rules_for(ModeKind::TeamBrawl, Some(0), 60.0));
    let mut t = room();
    let a = w.add_player(None, ShipType::Tinker).unwrap();
    let _b = w.add_player(None, ShipType::Tinker).unwrap();
    let side_a = w.ship(a).unwrap().team;
    // a leaves a mine-like entity behind (owned, on its side).
    let mut mine = EntitySpawn::debris(Vec3::ZERO, Vec3::ZERO, 0.5, 0.0);
    mine.owner = Some(a);
    mine.team = side_a;
    w.spawn_entity(mine);
    w.remove_player(a);
    let c = w.add_player(None, ShipType::Tinker).unwrap();
    assert_ne!(
        w.ship(c).unwrap().team,
        side_a,
        "a's side is still in play through its entity"
    );
    // a's leftovers hurt c in full (no friendly-fire scaling).
    let hull = w.ship(c).unwrap().hull;
    let src = DamageSource {
        attacker: Some(a),
        team: side_a,
        weapon: None,
    };
    w.hook_for_test(&mut t, |ctx| ctx.damage(c, 10.0, Vec3::ZERO, src, true));
    assert_eq!(w.ship(c).unwrap().hull, hull - 10.0);
}

#[test]
fn free_for_all_has_no_friendly_fire_and_credits_kills() {
    let mut r = rules_for(ModeKind::TeamBrawl, Some(0), 60.0);
    r.friendly_fire_percent = 10;
    let mut w = world(r);
    let mut t = room();
    let a = w.add_player(None, ShipType::Tinker).unwrap();
    let b = w.add_player(None, ShipType::Tinker).unwrap();
    let hull = w.ship(b).unwrap().hull;
    hit(&mut w, &mut t, a, b, 10.0);
    assert_eq!(w.ship(b).unwrap().hull, hull - 10.0, "full damage");
    hit(&mut w, &mut t, a, b, 10_000.0);
    assert_eq!(w.ship(a).unwrap().kills, 1);
    assert_eq!(w.mode().player(a).unwrap().score, 1);
    let v = w.mode_view();
    assert_eq!(v.players[0].player, a, "best score first");
}

#[test]
fn generic_scoring_kills_suicides_and_team_kills() {
    let mut w = world(rules_for(ModeKind::TeamBrawl, Some(2), 60.0));
    let mut t = room();
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    let a2 = w.add_player(Some(0), ShipType::Tinker).unwrap();
    let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
    hit(&mut w, &mut t, a, b, 10_000.0);
    assert_eq!(w.mode().team_scores[..2], [1, 0]);
    assert_eq!(w.mode().player(a).unwrap().score, 1);
    // Team kill: the killer's team loses a point.
    hit(&mut w, &mut t, a2, a, 10_000.0);
    assert_eq!(w.mode().team_scores[..2], [0, 0]);
    // Suicide (no attacker): the victim's team loses a point.
    kill(&mut w, &mut t, a2);
    assert_eq!(w.mode().team_scores[..2], [-1, 0]);
    // Mode state is part of the hash.
    let h = w.state_hash();
    w.mode_mut().team_scores[0] += 1;
    assert_ne!(w.state_hash(), h);
}

#[test]
fn score_target_ends_the_match() {
    let mut w = world(rules_for(ModeKind::TeamBrawl, Some(2), 60.0));
    let mut t = room();
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
    w.mode_mut().score_target = 1;
    hit(&mut w, &mut t, b, a, 10_000.0);
    let ev = w.step(&mut t, &[]);
    assert!(ev.iter().any(|e| matches!(
        e,
        Event::MatchEnded {
            winner: Some(1),
            ..
        }
    )));
    let res = w.results();
    assert_eq!(res.winner, Some(1));
    assert_eq!(res.rows[0].player, b);
    assert_eq!(res.team_scores, vec![0, 1]);
}

#[test]
fn one_life_eliminates_and_the_last_side_wins() {
    // The Squeeze (stub): one life each, free-for-all.
    let mut w = world(rules_for(ModeKind::TheSqueeze, None, 600.0));
    let mut t = room();
    let ids: Vec<PlayerId> = (0..3)
        .map(|_| w.add_player(None, ShipType::Tinker).unwrap())
        .collect();
    assert_eq!(w.mode().player(ids[0]).unwrap().lives, 1);
    assert!(w.mode_view().flags & view_flags::LIVES != 0);
    w.step(&mut t, &[]);
    kill(&mut w, &mut t, ids[0]);
    let ev = w.step(&mut t, &[]);
    assert!(ev.contains(&Event::Eliminated { player: ids[0] }));
    assert!(w.is_spectator(ids[0]));
    assert!(w.mode().is_running(), "two left");
    // No respawn, ever.
    for _ in 0..600 {
        w.step(&mut t, &[]);
    }
    assert!(!w.ship(ids[0]).unwrap().alive);
    kill(&mut w, &mut t, ids[1]);
    let ev = w.step(&mut t, &[]);
    let side = w.ship(ids[2]).unwrap().team;
    assert!(ev.contains(&Event::MatchEnded {
        winner: Some(side),
        team_cells: [0; 4]
    }));
    let view = w.mode_view();
    assert_eq!(view.winner_player().unwrap().player, ids[2]);
    assert!(view
        .players
        .iter()
        .filter(|r| r.player != ids[2])
        .all(|r| r.flags & player_flags::ELIMINATED != 0 && r.lives == 0));
}

#[test]
fn unlimited_lives_respawn() {
    let mut w = world(rules_for(ModeKind::TeamBrawl, Some(2), 60.0));
    let mut t = room();
    let a = w.add_player(None, ShipType::Tinker).unwrap();
    assert_eq!(w.mode().player(a).unwrap().lives, LIVES_UNLIMITED);
    kill(&mut w, &mut t, a);
    let ev: Vec<Event> = (0..200).flat_map(|_| w.step(&mut t, &[])).collect();
    assert!(ev
        .iter()
        .any(|e| matches!(e, Event::Respawned { player, .. } if *player == a)));
    assert!(!ev.iter().any(|e| matches!(e, Event::Eliminated { .. })));
}

#[test]
fn late_joiners_spectate_in_elimination_modes() {
    let mut w = world(rules_for(ModeKind::TheSqueeze, None, 600.0));
    let mut t = room();
    let early = w.add_player(None, ShipType::Tinker).unwrap();
    for _ in 0..LATE_JOIN_GRACE_TICKS + 1 {
        w.step(&mut t, &[]);
    }
    let late = w.add_player(None, ShipType::Tinker).unwrap();
    assert!(w.ship(early).unwrap().alive);
    assert!(!w.ship(late).unwrap().alive);
    assert!(w.is_spectator(late));
    assert!(
        w.set_spectator(late, false).is_err(),
        "not until the next match"
    );
    for _ in 0..300 {
        w.step(&mut t, &[]);
    }
    assert!(!w.ship(late).unwrap().alive, "spectators never respawn");
    // Modes with respawns let late joiners play.
    let mut w = world(rules_for(ModeKind::TeamBrawl, Some(2), 600.0));
    for _ in 0..LATE_JOIN_GRACE_TICKS + 1 {
        w.step(&mut t, &[]);
    }
    let p = w.add_player(None, ShipType::Tinker).unwrap();
    assert!(w.ship(p).unwrap().alive && !w.is_spectator(p));
}

#[test]
fn spectating_by_choice_and_back() {
    let mut w = world(rules_for(ModeKind::TeamBrawl, Some(2), 600.0));
    let mut t = room();
    let a = w.add_player(None, ShipType::Tinker).unwrap();
    w.set_spectator(a, true).unwrap();
    assert!(!w.ship(a).unwrap().alive && w.is_spectator(a));
    assert_eq!(w.ship(a).unwrap().deaths, 0, "no death");
    for _ in 0..300 {
        w.step(&mut t, &[]);
    }
    assert!(!w.ship(a).unwrap().alive);
    w.set_spectator(a, false).unwrap();
    let ev = w.step(&mut t, &[]);
    assert!(ev
        .iter()
        .any(|e| matches!(e, Event::Respawned { player, .. } if *player == a)));
    assert!(w.set_spectator(PlayerId(99), true).is_err());
}

#[test]
fn paint_the_void_view_carries_coverage() {
    let mut w = world(RuleSet::paint_the_void());
    let mut t = room();
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    let pos = w.ship(a).unwrap().pos;
    w.hook_for_test(&mut t, |ctx| {
        ctx.paint(pos + Vec3::new(0.0, -15.0, 0.0), 4.0, 1)
    });
    for _ in 0..30 {
        w.step(&mut t, &[]);
    }
    let v = w.mode_view();
    assert_eq!(v.kind, ModeKind::PaintTheVoid);
    assert!(v.flags & view_flags::PAINT_SCORES != 0);
    assert_eq!(v.team_scores.len(), 2);
    assert!(v.team_scores[1] > 0 && v.team_scores[0] == 0, "{v:?}");
    assert!(v.score_total > 0);
    assert_eq!(v.players.len(), 1);
    assert_eq!(v.end_tick, 300 * 60);
}

#[test]
fn instagib_sets_the_view_flag() {
    let mut r = rules_for(ModeKind::TeamBrawl, Some(2), 60.0);
    r.mutators.push(Mutator::Instagib);
    let w = world(r);
    assert!(w.mode_view().flags & view_flags::INSTAGIB != 0);
}

#[test]
fn derived_default_rule_sets_resolve_without_the_paint_thrower() {
    let weapons = WeaponTable::builtin();
    for kind in ModeKind::ALL {
        let r = RuleSet::default_for(kind);
        assert_eq!(r.mode.kind(), kind);
        r.resolve(&weapons).unwrap();
        if kind != ModeKind::PaintTheVoid {
            assert!(r.availability("paint_thrower").is_banned(), "{kind:?}");
            assert_eq!(r.name, kind.name());
        }
    }
    assert_eq!(
        RuleSet::default_for(ModeKind::PaintTheVoid),
        RuleSet::paint_the_void()
    );
}

#[test]
fn deferred_edits_apply_at_their_tick() {
    let mut w = world(rules_for(ModeKind::TeamBrawl, Some(2), 60.0));
    let mut t = room();
    let center = Vec3::new(0.0, -15.0, 0.0);
    let (_, ev) = w.hook_for_test(&mut t, |ctx| {
        ctx.later(3, |ctx| ctx.crater_exact(center, 2.0))
    });
    assert!(ev.is_empty());
    let h = w.state_hash();
    let mut got = Vec::new();
    for _ in 0..4 {
        let tick = w.tick() + 1;
        for e in w.step(&mut t, &[]) {
            if let Event::TerrainEdit(edit) = e {
                got.push((tick, edit));
            }
        }
    }
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].0, 3, "now (0) + 3");
    assert_ne!(w.state_hash(), h);
}
