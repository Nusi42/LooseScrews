//! Event registry and data tests.

use super::*;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

#[test]
fn registry_is_in_kind_order_with_unique_ids() {
    assert_eq!(MODULES.len(), EventKind::ALL.len());
    for (i, (entry, kind)) in MODULES.iter().zip(EventKind::ALL).enumerate() {
        assert_eq!(entry.kind, kind);
        assert_eq!(entry.module.kind(), kind, "{}", entry.id);
        assert_eq!(kind.index() as usize, i);
        assert_eq!(EventKind::from_index(i as u8), Some(kind));
        assert_eq!(EventKind::from_id(entry.id), Some(kind));
        assert_eq!(kind.id(), entry.id);
        assert_eq!(entry.file, format!("events/{}.ron", entry.id));
        assert!(
            MODULES[..i].iter().all(|m| m.id != entry.id),
            "duplicate {}",
            entry.id
        );
    }
    assert_eq!(EventKind::from_index(18), None);
    assert_eq!(EventKind::from_id("tea_party"), None);
    assert_eq!(event_ids().len(), 18);
    // The rule set code carries 5-bit event indices.
    assert!(EventKind::ALL.len() <= 31);
}

#[test]
fn event_data_is_valid_and_matches_the_plan() {
    let table = EventTable::builtin();
    let mut moods = std::collections::BTreeMap::new();
    for (kind, def) in table.iter() {
        assert_eq!(def.id, kind.id());
        assert!(def.enabled, "{}", def.id);
        assert!(!def.name.is_empty());
        assert!(
            def.active_secs > 0.0 && def.active_secs <= 45.0,
            "{}",
            def.id
        );
        for m in &def.moods {
            moods.entry(*m).or_insert_with(Vec::new).push(kind);
        }
    }
    // §3.1: the moods' events.
    use crate::director::Mood;
    assert_eq!(
        moods[&Mood::Bored],
        vec![
            EventKind::BoredLevel,
            EventKind::MovingDay,
            EventKind::LevelHiccups
        ]
    );
    assert_eq!(
        moods[&Mood::Grumpy],
        vec![
            EventKind::TaxDrone,
            EventKind::CatPaw,
            EventKind::CustomerSurvey
        ]
    );
    assert_eq!(
        moods[&Mood::Giddy],
        vec![
            EventKind::RubberChickens,
            EventKind::PizzaDelivery,
            EventKind::DuckParade,
            EventKind::HatRain,
            EventKind::DiscoInferno
        ]
    );
    assert!(!moods.contains_key(&Mood::Calm));
    // §3.2 durations (active phase).
    for (kind, secs) in [
        (EventKind::TaxDrone, 25.0),
        (EventKind::GravityCoffeeBreak, 20.0),
        (EventKind::RubberChickens, 12.0),
        (EventKind::TiredAnnouncer, 30.0),
        (EventKind::WrongGenre, 10.0),
        (EventKind::GossipWall, 15.0),
        (EventKind::PizzaDelivery, 30.0),
        (EventKind::EverythingIsCake, 20.0),
        (EventKind::DuckParade, 45.0),
        (EventKind::HatRain, 25.0),
        (EventKind::CustomerSurvey, 12.0),
        (EventKind::DiscoInferno, 20.0),
        (EventKind::BigHeadDay, 15.0),
        (EventKind::MimicPickups, 20.0),
        (EventKind::LevelHiccups, 15.0),
    ] {
        assert_eq!(table.get(kind).active_secs, secs, "{kind:?}");
    }
    // Targeted events name a player.
    for kind in [
        EventKind::TaxDrone,
        EventKind::PizzaDelivery,
        EventKind::CustomerSurvey,
    ] {
        assert!(table.get(kind).targeted);
    }
}

#[test]
fn bad_data_is_rejected() {
    let ok = MODULES[EventKind::BigHeadDay as usize].data;
    assert!(EventDef::from_ron("x", ok).is_ok());
    for (from, to) in [
        ("id: \"big_head_day\"", "id: \"tea_party\""),
        ("active_secs: 15.0", "active_secs: 0.0"),
        ("weight: 1.0", "weight: -1.0"),
    ] {
        assert!(ok.contains(from), "{from}");
        let bad = ok.replace(from, to);
        assert!(EventDef::from_ron("x", &bad).is_err(), "{to}");
    }
    // A missing event file.
    let files: Vec<(&str, &str)> = MODULES[1..].iter().map(|m| (m.file, m.data)).collect();
    assert!(matches!(
        EventTable::from_files(files),
        Err(SimError::UnknownEvent(_))
    ));
}

/// Events built so far (wave B replaces the stubs); the rest stay
/// announce-only.
const BUILT: &[EventKind] = &[
    // B1: world and movement.
    EventKind::BoredLevel,
    EventKind::LevelHiccups,
    EventKind::GravityCoffeeBreak,
    EventKind::WrongGenre,
    EventKind::BigHeadDay,
    EventKind::RubberChickens,
    EventKind::MovingDay,
    EventKind::EverythingIsCake,
    // B2: visitors.
    EventKind::TaxDrone,
    EventKind::PizzaDelivery,
    EventKind::CustomerSurvey,
    EventKind::MimicPickups,
    EventKind::GossipWall,
    // B3: spectacles.
    EventKind::CatPaw,
    EventKind::TiredAnnouncer,
    EventKind::DuckParade,
    EventKind::HatRain,
    EventKind::DiscoInferno,
];

#[test]
fn stubs_are_announce_only() {
    for m in &MODULES {
        assert_eq!(m.module.implemented(), BUILT.contains(&m.kind), "{}", m.id);
    }
}

#[test]
fn phases_and_moods_have_stable_indices() {
    for (i, p) in [EventPhase::Announce, EventPhase::Active, EventPhase::Ending]
        .into_iter()
        .enumerate()
    {
        assert_eq!(p.index() as usize, i);
        assert_eq!(EventPhase::from_index(i as u8), Some(p));
    }
    assert_eq!(EventPhase::from_index(3), None);
    use crate::director::Mood;
    for (i, m) in Mood::ALL.into_iter().enumerate() {
        assert_eq!(m.index() as usize, i);
        assert_eq!(Mood::from_index(i as u8), Some(m));
    }
    assert_eq!(Mood::from_index(4), None);
}
