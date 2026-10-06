//! Tired announcer (comedy event; `docs/milestone-4-plan.md` §3.2).
//!
//! For 30 s every kill plays a 3 s kill-cam for everyone (picture in
//! picture, the killer's view from the client's buffered snapshots)
//! narrated by the announcer. The simulation decides the schedule, so
//! every client shows the same kill-cams at the same time: each kill in
//! the active phase emits a [`CueKind::KillCam`] cue (killer, victim, where
//! the victim died) whose `value` is the delay until its kill-cam starts.
//! Kill-cams play one after another; a kill that would wait longer than
//! `max_wait_secs` (two kill-cams) is narrated only (`value` =
//! [`NARRATED_ONLY`]). Kills by the level (no killer) get kill-cams too
//! (the client shows the victim's view).
//!
//! State: `vars[0]` = tick the kill-cam queue is free again, `vars[1]` =
//! kills seen, `vars[2]` = kill-cams scheduled.
//!
//! Numbers: `data/events/tired_announcer.ron` (`killcam_secs`,
//! `max_wait_secs`).

use ls_math::Tick;

use crate::behavior::{Cue, CueKind};
use crate::events::{EventCtx, EventKind, EventModule, EventPhase};

/// Tired announcer.
#[derive(Debug)]
pub struct TiredAnnouncer;

/// `value` of a kill-cam cue whose kill is narrated without a kill-cam.
pub const NARRATED_ONLY: u16 = u16::MAX;

impl EventModule for TiredAnnouncer {
    fn kind(&self) -> EventKind {
        EventKind::TiredAnnouncer
    }

    fn implemented(&self) -> bool {
        true
    }

    fn on_tick(&self, e: &mut EventCtx) {
        if e.event.phase != EventPhase::Active || e.kills.is_empty() {
            return;
        }
        let now = e.now();
        let cam = Tick::ticks_for_secs(e.def.param("killcam_secs", 3.0)).max(1);
        let max_wait = Tick::ticks_for_secs(e.def.param("max_wait_secs", 6.0));
        let kills = e.kills.to_vec();
        for k in kills {
            e.event.vars[1] += 1;
            let free = (e.event.vars[0].max(0) as u64).max(now);
            let wait = free - now;
            let value = if wait <= max_wait {
                e.event.vars[0] = (free + cam) as i64;
                e.event.vars[2] += 1;
                wait.min(NARRATED_ONLY as u64 - 1) as u16
            } else {
                NARRATED_ONLY
            };
            e.ctx.cue(Cue {
                kind: CueKind::KillCam,
                source: k.killer,
                target: Some(k.victim),
                pos: k.pos,
                value,
            });
        }
    }
}
