//! Team Brawl: team deathmatch with respawns; 2 to 4 teams, or free-for-all
//! deathmatch with `teams: 0` (the duel mode). First side to the kill target
//! wins; on time the most kills win (draw if equal).
//!
//! Settings: `ruleset::TeamBrawlRules` (`teams`, `match_secs`,
//! `respawn_secs`, `kill_target`, `team_kill_penalty`). Design:
//! `docs/milestone-3-plan.md` §3.2. Default rule sets:
//! `data/rulesets/team_brawl.ron`, `instagib_brawl.ron` (with the Instagib
//! mutator) and `duel.ron` (free-for-all, first to 10).
//!
//! Rules:
//! - A kill scores one point for the killer (and their team). A suicide
//!   (including a death without a killer) costs the victim's side
//!   `team_kill_penalty` points; a team kill costs the killer's side the
//!   same.
//! - Kill target: `kill_target`, or with `kill_target: 0` it scales with the
//!   players: `max(15, 5 x players / sides)`, capped at 50 (sides = teams,
//!   or the players themselves in free-for-all, so free-for-all plays to
//!   15). It is set when players join and never drops during a match, so a
//!   player leaving cannot end it.
//! - The first side to the target wins at once; on time the best score wins
//!   (team score, or player score in free-for-all) and a tie is a draw.
//! - Respawns after `respawn_secs` (instant under Instagib); no lives, so
//!   late joiners play at once on the smaller team.

use crate::mode::{player_flags, ModeKind};
use crate::modes::{add_score, generic_end, Kill, ModeModule, ModeMut, ModeRef, Outcome};
use crate::ruleset::{ModeRules, TeamBrawlRules};

/// Smallest scaled kill target.
pub const KILL_TARGET_MIN: u32 = 15;
/// Scaled kill target per player and side.
pub const KILL_TARGET_PER_PLAYER: u32 = 5;
/// Largest scaled kill target.
pub const KILL_TARGET_MAX: u32 = 50;

/// The kill target of a match with `players` playing: the rule set's
/// `kill_target`, or (0) `max(15, 5 x players / sides)` capped at 50.
pub fn kill_target(rules: &TeamBrawlRules, players: usize) -> u32 {
    if rules.kill_target > 0 {
        return u32::from(rules.kill_target);
    }
    let players = players as u32;
    let sides = if rules.teams == 0 {
        players.max(1)
    } else {
        u32::from(rules.teams)
    };
    (KILL_TARGET_PER_PLAYER * players / sides).clamp(KILL_TARGET_MIN, KILL_TARGET_MAX)
}

/// The Team Brawl mode module.
#[derive(Debug)]
pub struct TeamBrawl;

fn settings(rules: &ModeRules) -> TeamBrawlRules {
    match rules {
        ModeRules::TeamBrawl(r) => *r,
        _ => TeamBrawlRules::default(),
    }
}

impl ModeModule for TeamBrawl {
    fn kind(&self) -> ModeKind {
        ModeKind::TeamBrawl
    }

    fn implemented(&self) -> bool {
        true
    }

    fn team_counts(&self) -> &'static [u8] {
        &[0, 2, 3, 4]
    }

    /// The kill target follows the most players the match has had.
    fn on_join(&self, m: &mut ModeMut, _player: crate::PlayerId) {
        let players = m
            .state
            .players
            .values()
            .filter(|p| p.flags & player_flags::SPECTATOR == 0)
            .count();
        let target = kill_target(&settings(m.rules), players);
        m.state.score_target = m.state.score_target.max(target);
    }

    fn on_kill(&self, m: &mut ModeMut, kill: &Kill) {
        let penalty = i32::from(settings(m.rules).team_kill_penalty);
        if kill.is_suicide() {
            add_score(m, kill.victim, -penalty);
        } else if let Some(k) = kill.killer {
            if kill.is_team_kill() {
                add_score(m, k, -penalty);
            } else {
                add_score(m, k, 1);
            }
        }
    }

    fn end_check(&self, m: &ModeRef) -> Option<Outcome> {
        generic_end(m)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    //! Team Brawl scenario tests (kill target, scoring, penalties, timer,
    //! respawn), its default rule sets, and the per-mode replay goldens.
    //! The helpers are shared with the Instagib tests.

    use ls_math::terrain::BoxRoomTerrain;
    use ls_math::{Pcg32, Quat, Vec3};

    use super::*;
    use crate::mode::{MatchPhase, LIVES_UNLIMITED};
    use crate::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    pub(crate) type Room = PaintedTerrain<BoxRoomTerrain>;

    pub(crate) fn room() -> Room {
        PaintedTerrain::new(BoxRoomTerrain::centered(Vec3::new(60.0, 30.0, 60.0)), 1.0)
    }

    /// Two spawn points per team for four teams, around the room's centre,
    /// and eight pickup points.
    pub(crate) fn setup() -> MatchSetup {
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
                    orientation: Quat::from_axis_angle(Vec3::Y, a),
                });
            }
        }
        MatchSetup {
            spawns,
            pickup_points: (0..8)
                .map(|i| Vec3::new(-14.0 + 4.0 * i as f32, -3.0, 0.0))
                .collect(),
            instability: 0,
            heist: None,
        }
    }

    pub(crate) fn world(rules: RuleSet) -> World {
        let mut cfg = WorldConfig::paint_the_void(7, setup());
        cfg.rules = rules;
        World::new(cfg).unwrap()
    }

    pub(crate) fn brawl(teams: u8) -> RuleSet {
        let mut r = RuleSet::default_for(ModeKind::TeamBrawl);
        r.mode.set_field("teams", teams as f32);
        r
    }

    /// `from` deals `amount` to `to` (`weapon`: the shot's weapon id).
    pub(crate) fn hit_with(
        w: &mut World,
        t: &mut Room,
        from: PlayerId,
        to: PlayerId,
        amount: f32,
        weapon: Option<WeaponId>,
        direct: bool,
    ) -> Vec<Event> {
        let src = DamageSource {
            attacker: Some(from),
            team: w.ship(from).unwrap().team,
            weapon,
        };
        w.hook_for_test(t, |ctx| ctx.damage(to, amount, Vec3::ZERO, src, direct))
            .1
    }

    /// `from` kills `to` outright.
    pub(crate) fn frag(w: &mut World, t: &mut Room, from: PlayerId, to: PlayerId) {
        hit_with(w, t, from, to, 10_000.0, None, true);
    }

    /// `victim` dies without an attacker (a suicide for scoring).
    pub(crate) fn suicide(w: &mut World, t: &mut Room, victim: PlayerId) {
        w.hook_for_test(t, |ctx| {
            ctx.push(Command::AdjustHull {
                target: victim,
                delta: -10_000.0,
                can_kill: true,
            })
        });
    }

    /// Steps until every dead ship is back (Team Brawl's 3 s respawn).
    pub(crate) fn respawn_all(w: &mut World, t: &mut Room) -> Vec<Event> {
        let mut ev = Vec::new();
        for _ in 0..181 {
            ev.extend(w.step(t, &[]));
        }
        ev
    }

    pub(crate) fn ended(events: &[Event]) -> Option<Option<u8>> {
        events.iter().find_map(|e| match e {
            Event::MatchEnded { winner, .. } => Some(*winner),
            _ => None,
        })
    }

    /// Scripted inputs: random buttons and thrust, aiming at the nearest
    /// enemy; depends only on the world state and the RNG.
    fn scripted_inputs(
        rng: &mut Pcg32,
        w: &World,
        ids: &[PlayerId],
        tick: u64,
    ) -> Vec<(PlayerId, InputCmd)> {
        ids.iter()
            .map(|&id| {
                let (pitch, yaw) = aim_at_nearest_enemy(w, id);
                let mut buttons = Buttons::HORIZON_ASSIST;
                for (b, p) in [
                    (Buttons::BOOST, 0.3),
                    (Buttons::FIRE_PRIMARY, 0.5),
                    (Buttons::FIRE_SECONDARY, 0.05),
                    (Buttons::SCAVENGE, 0.2),
                    (Buttons::GADGET, 0.03),
                ] {
                    buttons.set(b, rng.chance(p));
                }
                let axis = |r: &mut Pcg32| (r.below(3) as f32) - 1.0;
                let thrust = Vec3::new(axis(rng), axis(rng), axis(rng));
                (
                    id,
                    InputCmd {
                        tick,
                        thrust,
                        look_pitch: pitch + rng.range_f32(-0.02, 0.02),
                        look_yaw: yaw + rng.range_f32(-0.02, 0.02),
                        roll: rng.range_f32(-1.0, 1.0),
                        buttons,
                        weapon_select: 0,
                    },
                )
            })
            .collect()
    }

    fn aim_at_nearest_enemy(w: &World, id: PlayerId) -> (f32, f32) {
        let Some(me) = w.ship(id) else {
            return (0.0, 0.0);
        };
        let target = w
            .ships()
            .filter(|s| s.alive && s.team != me.team)
            .min_by(|a, b| {
                a.pos
                    .distance_squared(me.pos)
                    .total_cmp(&b.pos.distance_squared(me.pos))
            });
        let Some(target) = target else {
            return (0.0, 0.0);
        };
        let local = me.orientation.conjugate().rotate(target.pos - me.pos);
        let pitch = ls_math::det::atan2(local.y, -local.z).clamp(-0.1, 0.1);
        let yaw = ls_math::det::atan2(-local.x, -local.z).clamp(-0.1, 0.1);
        (pitch, yaw)
    }

    /// A scripted 8-player match (two ships of every type) under `rules`:
    /// runs `ticks` ticks (or until the match ends) and returns the state
    /// hash every 60 ticks, all events and the world.
    pub(crate) fn replay(rules: RuleSet, seed: u64, ticks: u64) -> (Vec<u64>, Vec<Event>, World) {
        let mut cfg = WorldConfig::paint_the_void(seed, setup());
        cfg.rules = rules;
        let mut w = World::new(cfg).unwrap();
        let mut t = room();
        let ids: Vec<PlayerId> = ShipType::ALL
            .iter()
            .chain(ShipType::ALL.iter())
            .map(|&ty| w.add_player(None, ty).unwrap())
            .collect();
        let mut rng = Pcg32::new(seed, 11);
        let mut hashes = Vec::new();
        let mut events = Vec::new();
        for tick in 1..=ticks {
            let inputs = scripted_inputs(&mut rng, &w, &ids, tick);
            events.extend(w.step(&mut t, &inputs));
            if tick % 60 == 0 {
                hashes.push(w.state_hash());
            }
            if !w.mode().is_running() {
                break;
            }
        }
        (hashes, events, w)
    }

    pub(crate) fn kills(events: &[Event]) -> usize {
        events
            .iter()
            .filter(|e| matches!(e, Event::Killed { .. }))
            .count()
    }

    pub(crate) fn builtin(file: &str) -> RuleSet {
        let (name, text) = crate::ruleset::BUILTIN_RULESETS
            .iter()
            .find(|(f, _)| *f == file)
            .unwrap();
        RuleSet::from_ron(name, text).unwrap()
    }

    // ----- kill target -----------------------------------------------------

    #[test]
    fn kill_target_scales_with_players_and_sides() {
        let r = |teams: u8, kill_target: u16| TeamBrawlRules {
            teams,
            kill_target,
            ..TeamBrawlRules::default()
        };
        // max(15, 5 x players / teams), capped at 50.
        assert_eq!(kill_target(&r(2, 0), 4), 15);
        assert_eq!(kill_target(&r(2, 0), 8), 20);
        assert_eq!(kill_target(&r(2, 0), 9), 22);
        assert_eq!(kill_target(&r(2, 0), 12), 30);
        assert_eq!(kill_target(&r(4, 0), 32), 40);
        assert_eq!(kill_target(&r(2, 0), 20), 50);
        assert_eq!(kill_target(&r(2, 0), 32), 50, "capped");
        assert_eq!(kill_target(&r(3, 0), 9), 15);
        assert_eq!(kill_target(&r(2, 0), 0), 15);
        // Free-for-all: every player is a side.
        assert_eq!(kill_target(&r(0, 0), 2), 15);
        assert_eq!(kill_target(&r(0, 0), 32), 15);
        // An explicit target wins.
        assert_eq!(kill_target(&r(0, 10), 2), 10);
        assert_eq!(kill_target(&r(2, 60), 32), 60);
    }

    #[test]
    fn kill_target_follows_joins_and_never_drops() {
        let mut w = world(brawl(2));
        assert_eq!(w.mode().score_target, 0);
        let ids: Vec<PlayerId> = (0..12)
            .map(|_| w.add_player(None, ShipType::Tinker).unwrap())
            .collect();
        assert_eq!(w.mode().score_target, 30);
        assert_eq!(w.mode_view().score_target, 30);
        for id in &ids[..8] {
            w.remove_player(*id);
        }
        w.add_player(None, ShipType::Tinker).unwrap();
        assert_eq!(w.mode().score_target, 30, "a leaving player cannot end it");
        // Explicit target.
        let mut r = brawl(2);
        r.mode.set_field("kill_target", 3.0);
        let mut w = world(r);
        for _ in 0..12 {
            w.add_player(None, ShipType::Tinker).unwrap();
        }
        assert_eq!(w.mode().score_target, 3);
    }

    #[test]
    fn first_team_to_the_kill_target_wins() {
        let mut r = brawl(3);
        r.mode.set_field("kill_target", 3.0);
        let mut w = world(r);
        let mut t = room();
        let ids: Vec<PlayerId> = (0..3)
            .map(|_| w.add_player(None, ShipType::Tinker).unwrap())
            .collect();
        let (a, b, c) = (ids[0], ids[1], ids[2]);
        assert_eq!(w.ship(b).unwrap().team, 1);
        let mut ev = Vec::new();
        for (killer, victim) in [(b, a), (b, c), (a, c)] {
            frag(&mut w, &mut t, killer, victim);
            // Wait for the respawns.
            for _ in 0..200 {
                ev.extend(w.step(&mut t, &[]));
            }
        }
        assert_eq!(ended(&ev), None, "two kills are not three");
        assert_eq!(w.mode().team_scores[..3], [1, 2, 0]);
        frag(&mut w, &mut t, b, c);
        let ev = w.step(&mut t, &[]);
        assert_eq!(ended(&ev), Some(Some(1)));
        assert_eq!(w.mode().phase, MatchPhase::Ended { winner: Some(1) });
        let res = w.results();
        assert_eq!(res.winner, Some(1));
        assert_eq!(res.team_scores, vec![1, 3, 0]);
        assert_eq!(res.rows[0].player, b);
        assert_eq!((res.rows[0].kills, res.rows[0].score), (3, 3));
    }

    #[test]
    fn free_for_all_first_player_to_the_target_wins() {
        let mut r = brawl(0);
        r.mode.set_field("kill_target", 2.0);
        let mut w = world(r);
        let mut t = room();
        let ids: Vec<PlayerId> = (0..4)
            .map(|_| w.add_player(None, ShipType::Tinker).unwrap())
            .collect();
        frag(&mut w, &mut t, ids[2], ids[0]);
        assert_eq!(ended(&w.step(&mut t, &[])), None);
        frag(&mut w, &mut t, ids[2], ids[1]);
        let side = w.ship(ids[2]).unwrap().team;
        assert_eq!(ended(&w.step(&mut t, &[])), Some(Some(side)));
        let v = w.mode_view();
        assert!(v.team_scores.is_empty());
        assert_eq!(v.winner_player().unwrap().player, ids[2]);
        assert_eq!(v.players[0].score, 2);
    }

    // ----- scoring -----------------------------------------------------------

    #[test]
    fn suicides_and_team_kills_cost_the_penalty() {
        let mut r = brawl(2);
        r.mode.set_field("team_kill_penalty", 2.0);
        let mut w = world(r);
        let mut t = room();
        let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
        let a2 = w.add_player(Some(0), ShipType::Tinker).unwrap();
        let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
        frag(&mut w, &mut t, a, b);
        assert_eq!(w.mode().team_scores[..2], [1, 0]);
        respawn_all(&mut w, &mut t);
        // Team kill: the killer's team (and the killer) pay.
        frag(&mut w, &mut t, a2, a);
        assert_eq!(w.mode().team_scores[..2], [-1, 0]);
        assert_eq!(w.mode().player(a2).unwrap().score, -2);
        assert_eq!(w.ship(a2).unwrap().kills, 0, "team kills are no kills");
        respawn_all(&mut w, &mut t);
        // Suicide: the victim's team pays.
        suicide(&mut w, &mut t, b);
        assert_eq!(w.mode().team_scores[..2], [-1, -2]);
        assert_eq!(w.mode().player(b).unwrap().score, -2);
        // Free-for-all: a suicide costs the player.
        let mut w = world(brawl(0));
        let p = w.add_player(None, ShipType::Tinker).unwrap();
        suicide(&mut w, &mut t, p);
        assert_eq!(w.mode().player(p).unwrap().score, -1, "default penalty 1");
    }

    #[test]
    fn on_time_the_most_kills_win_and_a_tie_is_a_draw() {
        for (kills_b, winner) in [(1, Some(0)), (2, None)] {
            let mut r = brawl(2);
            r.mode.set_field("match_secs", 20.0);
            let mut w = world(r);
            let mut t = room();
            let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
            let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
            let mut ev = Vec::new();
            for k in 0..2 {
                frag(&mut w, &mut t, a, b);
                if k < kills_b {
                    frag(&mut w, &mut t, b, a);
                }
                ev.extend(respawn_all(&mut w, &mut t));
            }
            for _ in 0..20 * 60 {
                ev.extend(w.step(&mut t, &[]));
            }
            assert_eq!(ended(&ev), Some(winner), "b killed {kills_b}");
            assert_eq!(w.tick(), 20 * 60);
        }
    }

    #[test]
    fn ships_respawn_after_the_respawn_time_and_late_joiners_play() {
        let mut w = world(brawl(2));
        let mut t = room();
        let a = w.add_player(None, ShipType::Tinker).unwrap();
        let b = w.add_player(None, ShipType::Tinker).unwrap();
        assert_eq!(w.mode().player(a).unwrap().lives, LIVES_UNLIMITED);
        w.step(&mut t, &[]);
        frag(&mut w, &mut t, a, b);
        let died = w.tick();
        let mut respawned = None;
        for _ in 0..400 {
            for e in w.step(&mut t, &[]) {
                if matches!(e, Event::Respawned { player, .. } if player == b) {
                    respawned.get_or_insert(w.tick());
                }
            }
        }
        assert_eq!(respawned, Some(died + 180), "3 s");
        // Late joiners play at once (no lives in Team Brawl).
        for _ in 0..crate::modes::LATE_JOIN_GRACE_TICKS {
            w.step(&mut t, &[]);
        }
        let late = w.add_player(None, ShipType::Tinker).unwrap();
        assert!(w.ship(late).unwrap().alive && !w.is_spectator(late));
    }

    // ----- default rule sets -------------------------------------------------

    #[test]
    fn default_rule_sets_load_resolve_and_round_trip_as_codes() {
        let weapons = WeaponTable::builtin();
        let tb = builtin("rulesets/team_brawl.ron");
        assert_eq!(tb, RuleSet::default_for(ModeKind::TeamBrawl));
        assert_eq!(tb.name, "Team Brawl");
        let ModeRules::TeamBrawl(m) = tb.mode else {
            panic!("{:?}", tb.mode)
        };
        assert_eq!(
            (m.teams, m.match_secs, m.respawn_secs, m.kill_target),
            (2, 600.0, 3.0, 0)
        );
        assert_eq!(m.team_kill_penalty, 1);
        assert_eq!(tb.friendly_fire_percent, 50);
        let res = tb.resolve(&weapons).unwrap();
        let laser = weapons.find("scrap_laser").unwrap();
        let rocket = weapons.find("bolt_rocket").unwrap();
        assert_eq!(res.loadout_primaries, vec![laser]);
        assert_eq!(res.loadout_secondaries, vec![(rocket, 3)]);
        assert!(!res.pickup_table.is_empty(), "standard pickups");
        assert!(tb.availability("paint_thrower").is_banned());
        assert!(tb.mutators.is_empty());

        let duel = builtin("rulesets/duel.ron");
        let ModeRules::TeamBrawl(d) = duel.mode else {
            panic!("{:?}", duel.mode)
        };
        assert_eq!((d.teams, d.kill_target, d.match_secs), (0, 10, 300.0));
        duel.resolve(&weapons).unwrap();

        let ig = builtin("rulesets/instagib_brawl.ron");
        assert_eq!(ig.mode.kind(), ModeKind::TeamBrawl);
        assert_eq!(ig.mutators, vec![Mutator::Instagib]);
        assert_eq!(ig.effective(), ig, "the file is already the rewrite");
        ig.resolve(&weapons).unwrap();

        for r in [tb, duel, ig] {
            let code = r.to_code().unwrap();
            assert!(code.starts_with("LR"), "{code}");
            let back = RuleSet::from_code(&code).unwrap();
            assert_eq!(back.mode, r.mode, "{}", r.name);
            assert_eq!(back.mutators, r.mutators, "{}", r.name);
            assert_eq!(
                back.resolve(&weapons).unwrap(),
                r.resolve(&weapons).unwrap(),
                "{}",
                r.name
            );
        }
    }

    // ----- full matches and replays ------------------------------------------

    /// Every default rule set plays a full scripted 8-player match to a
    /// result (kill target or timer).
    #[test]
    fn default_rule_sets_play_a_full_match_to_a_result() {
        for file in [
            "rulesets/team_brawl.ron",
            "rulesets/instagib_brawl.ron",
            "rulesets/duel.ron",
        ] {
            let mut r = builtin(file);
            r.mode.set_field("match_secs", 90.0);
            let (_, ev, w) = replay(r, 99, 90 * 60 + 1);
            assert!(!w.mode().is_running(), "{file}: no result");
            assert!(kills(&ev) > 0, "{file}: nobody died");
            let res = w.results();
            assert!(res.ended);
            assert_eq!(res.rows.len(), 8);
            if let Some(winner) = res.winner {
                let target = w.mode().score_target as i64;
                let best = if w.mode().teams > 0 {
                    res.team_scores[winner as usize]
                } else {
                    res.rows[0].score as i64
                };
                assert!(best >= target || w.tick() == 90 * 60, "{file}");
            }
        }
    }

    /// Golden hashes of the 60 s Team Brawl replays (8 ships: the Team
    /// Brawl and Instagib Brawl rule sets with 2 teams, the duel rule set
    /// free-for-all; kill target 255 so they run the full minute). Must be identical on native and wasm32. Changing Team
    /// Brawl, Instagib, these rule sets or any simulation behaviour they use
    /// changes them: update the constants in the same change.
    const TEAM_BRAWL_REPLAY_GOLDEN: u64 = 0x4324_af1e_2f87_4773;
    const INSTAGIB_BRAWL_REPLAY_GOLDEN: u64 = 0x61ed_c96b_b3c7_4e0a;
    const DUEL_REPLAY_GOLDEN: u64 = 0xbf16_09a8_44fe_9b5c;

    #[test]
    fn replays_match_their_golden_hashes() {
        for (file, golden) in [
            ("rulesets/team_brawl.ron", TEAM_BRAWL_REPLAY_GOLDEN),
            ("rulesets/instagib_brawl.ron", INSTAGIB_BRAWL_REPLAY_GOLDEN),
            ("rulesets/duel.ron", DUEL_REPLAY_GOLDEN),
        ] {
            // A high kill target keeps the match running for all 60 s.
            let mut rules = builtin(file);
            rules.mode.set_field("kill_target", 255.0);
            let (h1, e1, w) = replay(rules.clone(), 1234, 3600);
            let (h2, e2, _) = replay(rules, 1234, 3600);
            assert_eq!(h1, h2, "{file}");
            assert_eq!(e1, e2, "{file}");
            assert_eq!(w.tick(), 3600, "{file}: 60 s without an early end");
            assert!(kills(&e1) > 0, "{file}: the replay has fights");
            let last = *h1.last().unwrap();
            assert_eq!(last, golden, "{file}: replay hash {last:#018x}");
        }
    }
}
