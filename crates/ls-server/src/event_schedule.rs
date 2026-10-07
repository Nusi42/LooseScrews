//! `--event-schedule <FILE|SPEC>` (M4 plan §3.1): forces comedy events at
//! given times of every match, for end-to-end runs and the browser smoke
//! only (the server logs a warning when it is set). The events ignore the
//! Absurdity, rule set bans, mode vetoes, `enabled` and eligibility
//! (`World::schedule_event`).
//!
//! A schedule is a list of `<seconds> <event>` items, one per line in a
//! file (`#` starts a comment) or separated by commas on the command line
//! (`--event-schedule "5 duck_parade, 40 next"`; a colon works as the
//! separator too: `5:duck_parade`). `<seconds>` counts from the start of
//! each match room (warm-up rooms get no events); `<event>` is an event id
//! (`data/events/<id>.ron`) or `next`, which takes the events in
//! `EventKind` order, one per `next` item, continuing across matches and
//! rooms, so a rotation of short matches forces every event in turn.
//!
//! An event scheduled while another runs starts as soon as that one ended.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use ls_math::Tick;
use ls_sim::{EventKind, World};

/// One forced event of the schedule.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScheduledEvent {
    /// Seconds after the match room's first tick.
    pub secs: f64,
    /// The event; `None` = `next` (the next event in `EventKind` order).
    pub kind: Option<EventKind>,
}

/// A parsed `--event-schedule`, shared by every room of the server (the
/// `next` cursor continues across matches and rooms).
#[derive(Debug, Default)]
pub struct EventSchedule {
    pub events: Vec<ScheduledEvent>,
    next: AtomicUsize,
}

impl Clone for EventSchedule {
    fn clone(&self) -> Self {
        EventSchedule {
            events: self.events.clone(),
            next: AtomicUsize::new(self.next.load(Ordering::Relaxed)),
        }
    }
}

impl PartialEq for EventSchedule {
    fn eq(&self, other: &Self) -> bool {
        self.events == other.events
    }
}

impl EventSchedule {
    /// Parses a schedule (file text or a command line spec, see the module
    /// docs).
    pub fn parse(text: &str) -> Result<EventSchedule, String> {
        let mut events = Vec::new();
        for raw in text.split(['\n', ',']) {
            let item = raw.split('#').next().unwrap_or("").trim();
            if item.is_empty() {
                continue;
            }
            let mut parts = item
                .split(|c: char| c.is_whitespace() || c == ':')
                .filter(|p| !p.is_empty());
            let (Some(secs), Some(id), None) = (parts.next(), parts.next(), parts.next()) else {
                return Err(format!(
                    "bad item {item:?} (want `<seconds> <event id or next>`)"
                ));
            };
            let secs: f64 = secs
                .parse()
                .ok()
                .filter(|s: &f64| s.is_finite() && (0.0..=86_400.0).contains(s))
                .ok_or_else(|| format!("bad seconds {secs:?} in {item:?}"))?;
            let kind = if id == "next" {
                None
            } else {
                Some(EventKind::from_id(id).ok_or_else(|| {
                    let ids: Vec<&str> = EventKind::ALL.iter().map(|k| k.id()).collect();
                    format!("unknown event {id:?} (known: next, {})", ids.join(", "))
                })?)
            };
            events.push(ScheduledEvent { secs, kind });
        }
        if events.is_empty() {
            return Err("the event schedule is empty".into());
        }
        events.sort_by(|a, b| a.secs.total_cmp(&b.secs));
        Ok(EventSchedule {
            events,
            next: AtomicUsize::new(0),
        })
    }

    /// `--event-schedule`'s value: a file if one exists at that path, else
    /// an inline spec.
    pub fn load(arg: &str) -> Result<EventSchedule, String> {
        let path = Path::new(arg);
        if path.is_file() {
            let text = std::fs::read_to_string(path)
                .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            EventSchedule::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
        } else {
            EventSchedule::parse(arg)
        }
    }

    /// Schedules the events in a new match room's world (at its tick 0);
    /// returns what was scheduled (`next` resolved), for the log.
    pub fn apply(&self, world: &mut World) -> Vec<(u64, EventKind)> {
        let base = world.tick();
        let mut out = Vec::with_capacity(self.events.len());
        for e in &self.events {
            let kind = e.kind.unwrap_or_else(|| {
                let i = self.next.fetch_add(1, Ordering::Relaxed);
                EventKind::ALL[i % EventKind::ALL.len()]
            });
            let at = base + Tick::ticks_for_secs(e.secs as f32).max(1);
            world.schedule_event(at, kind);
            out.push((at, kind));
        }
        out
    }
}

/// The shared schedule of a server (`None`: no `--event-schedule`).
pub type SharedSchedule = Option<Arc<EventSchedule>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_files_and_inline_specs() {
        let file = "# comment\n5 duck_parade\n\n  40.5   next # trailing\n2 hat_rain\n";
        let s = EventSchedule::parse(file).unwrap();
        assert_eq!(
            s.events,
            vec![
                ScheduledEvent {
                    secs: 2.0,
                    kind: Some(EventKind::HatRain)
                },
                ScheduledEvent {
                    secs: 5.0,
                    kind: Some(EventKind::DuckParade)
                },
                ScheduledEvent {
                    secs: 40.5,
                    kind: None
                },
            ]
        );
        let inline = EventSchedule::parse("5:duck_parade, 40.5 next,2 hat_rain").unwrap();
        assert_eq!(inline, s);
    }

    #[test]
    fn refuses_bad_items() {
        for bad in [
            "",
            "#only",
            "5",
            "x duck_parade",
            "-1 duck_parade",
            "5 no_such",
            "5 a b",
        ] {
            assert!(EventSchedule::parse(bad).is_err(), "{bad:?}");
        }
        let e = EventSchedule::parse("5 ducks").unwrap_err();
        assert!(e.contains("duck_parade"), "{e}");
    }

    #[test]
    fn next_walks_every_event_across_matches() {
        let s = EventSchedule::parse("1 next, 2 next").unwrap();
        let mut seen = Vec::new();
        for _ in 0..9 {
            let cfg = ls_sim::WorldConfig::paint_the_void(
                7,
                ls_sim::MatchSetup {
                    spawns: vec![ls_sim::SpawnPoint {
                        pos: ls_math::Vec3::ZERO,
                        team: 0,
                        orientation: ls_math::Quat::IDENTITY,
                    }],
                    ..ls_sim::MatchSetup::default()
                },
            );
            let mut w = World::new(cfg).unwrap();
            let got = s.apply(&mut w);
            assert_eq!(got.len(), 2);
            assert_eq!(got[0].0, 60);
            assert_eq!(got[1].0, 120);
            seen.extend(got.into_iter().map(|(_, k)| k));
        }
        assert_eq!(
            seen,
            EventKind::ALL.to_vec(),
            "18 `next` items: every event once"
        );
    }
}
