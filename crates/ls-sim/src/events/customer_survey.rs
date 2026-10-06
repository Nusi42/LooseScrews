//! Customer survey (comedy event; `docs/milestone-4-plan.md` §3.2).
//!
//! When the active phase starts a drone parks 2.5 u in front of the
//! leader's view and follows its orientation (it is placed every tick from
//! the leader's pose, the five-star panel facing the leader and about a
//! quarter of the view wide) for 12 s or until shot (15 hp; anyone's shots,
//! the leader's too). The star the destroying shot went through is the
//! leader's rating: the panel is split into five equal columns across its
//! width, 1 star on the leader's left to 5 on the right
//! ([`star_at`]). The rating is announced with an outcome cue; a survey
//! nobody answered ends with rating 0. Then the drone floats off during
//! the 1.5 s ending phase.
//!
//! Drone entity (`ReplicatedKind::Drone`): `state` = `[ROLE_SURVEY,
//! rating (0 none yet, 1-5), phase (working, leaving), hp percent]`, radius
//! = half the panel width. Outcome cue (`CueKind::EventOutcome`): rating
//! 1-5 (source = the shooter, target = the leader) or 0 (no answer).
//!
//! Numbers: `data/events/customer_survey.ron`.

use ls_math::{Quat, Vec3};

use crate::behavior::{Command, Cue, CueKind};
use crate::entity::EntityKind;
use crate::events::drone::{self, DRONE_LEAVING, DRONE_WORKING, ROLE_SURVEY};
use crate::events::{EventCtx, EventKind, EventModule, EventPhase, EventRef};

/// Customer survey.
#[derive(Debug)]
pub struct CustomerSurvey;

/// `EventState::vars`: the rating (0 none), whether the result was told.
const V_RATING: usize = 0;
const V_TOLD: usize = 1;

/// The star (1-5) a shot at `point` went through on a panel at `center`
/// of half width `half_width` facing a viewer whose right is `right`:
/// five equal columns, 1 on the viewer's left. Points beyond the panel's
/// edge count for the outer stars.
pub fn star_at(point: Vec3, center: Vec3, right: Vec3, half_width: f32) -> u8 {
    if half_width <= 0.0 {
        return 3;
    }
    let x = (point - center).dot(right) / (2.0 * half_width) + 0.5;
    let col = ls_math::det::floor(x * 5.0).clamp(0.0, 4.0);
    col as u8 + 1
}

/// The panel's pose for a leader at `pos` with `orientation`: `distance`
/// ahead, turned to face the leader.
pub fn panel_pose(pos: Vec3, orientation: Quat, distance: f32) -> (Vec3, Quat) {
    let rot = orientation
        .mul_quat(Quat::from_axis_angle(Vec3::Y, std::f32::consts::PI))
        .normalize();
    (pos + orientation.forward() * distance, rot)
}

fn tell(e: &mut EventCtx, source: Option<crate::PlayerId>, pos: Vec3, rating: u8) {
    let target = e.event.target;
    e.ctx.cue(Cue {
        kind: CueKind::EventOutcome,
        source,
        target,
        pos,
        value: drone::outcome(EventKind::CustomerSurvey, rating),
    });
    e.event.vars[V_TOLD] = 1;
}

impl EventModule for CustomerSurvey {
    fn kind(&self) -> EventKind {
        EventKind::CustomerSurvey
    }

    fn implemented(&self) -> bool {
        true
    }

    /// Needs a clear leader (`ModeModule::leader`).
    fn eligible(&self, e: &EventRef) -> bool {
        e.leader.is_some()
    }

    /// Names the leader.
    fn on_announce(&self, e: &mut EventCtx) {
        e.event.target = e.leader;
    }

    /// The drone pops up in front of the leader.
    fn on_start(&self, e: &mut EventCtx) {
        let Some(t) = drone::living(&e.ctx.view, e.event.target) else {
            return;
        };
        let (pos, rot) = panel_pose(t.pos, t.orientation, e.def.param("distance", 2.5));
        let mut spawn = drone::prop(
            EntityKind::Drone,
            pos,
            e.def.param("panel_half_width", 1.25),
            e.def.param("hp", 15.0),
            [ROLE_SURVEY, 0, DRONE_WORKING, 100],
        );
        spawn.rot = rot;
        spawn.vel = t.vel;
        e.ctx.spawn_entity(spawn);
    }

    fn on_tick(&self, e: &mut EventCtx) {
        let Some(mut d) = drone::first_prop(&e.ctx.view, EntityKind::Drone) else {
            if e.event.phase == EventPhase::Ending && e.event.vars[V_TOLD] == 0 {
                tell(e, None, e.event.pos, 0);
            }
            return;
        };
        let hp_max = e.def.param("hp", 15.0);
        if d.hp <= 0.0 && e.event.phase == EventPhase::Ending {
            // Shot while floating off: the survey is closed already.
            e.ctx.push(Command::RemoveEntity { id: d.id });
            return;
        }
        if d.hp <= 0.0 {
            // Rated: the star the destroying shot went through.
            let right = -d.rot.right();
            let (point, by) = d.last_hit.map_or((d.pos, None), |(p, by)| (p, Some(by)));
            let star = star_at(point, d.pos, right, d.radius);
            e.event.vars[V_RATING] = star as i64;
            e.event.pos = d.pos;
            tell(e, by, d.pos, star);
            e.ctx.push(Command::RemoveEntity { id: d.id });
            drone::end_now(e);
            return;
        }
        e.event.pos = d.pos;
        if e.event.phase == EventPhase::Ending {
            if e.event.vars[V_TOLD] == 0 {
                tell(e, None, d.pos, 0);
            }
            // Nobody answered: it floats off upward.
            let up = d.rot.up();
            d.vel = up * e.def.param("leave_speed", 4.0);
            d.pos += d.vel * ls_math::DT;
            d.state = [
                ROLE_SURVEY,
                0,
                DRONE_LEAVING,
                drone::hp_percent(d.hp, hp_max),
            ];
            e.ctx.set_entity(&d);
            return;
        }
        if let Some(t) = drone::living(&e.ctx.view, e.event.target) {
            let (pos, rot) = panel_pose(t.pos, t.orientation, e.def.param("distance", 2.5));
            (d.pos, d.rot, d.vel) = (pos, rot, t.vel);
        } else {
            d.vel = Vec3::ZERO;
        }
        d.state = [
            ROLE_SURVEY,
            0,
            DRONE_WORKING,
            drone::hp_percent(d.hp, hp_max),
        ];
        e.ctx.set_entity(&d);
    }

    fn on_end(&self, e: &mut EventCtx) {
        drone::remove_props(e.ctx, EntityKind::Drone);
    }
}
