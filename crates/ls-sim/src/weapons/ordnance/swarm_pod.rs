//! Swarm Pod: ordnance secondary. 30 u/s; releases 6 homing blobs on impact. Malus: blobs are
//! weak alone. Lasting effect: scatter of small craters.
//!
//! Numbers: `data/weapons/swarm_pod.ron` (`params`: `blobs`, `blob_speed`, `blob_damage`,
//! `blob_lifetime_secs`, `blob_crater_radius`, `turn_rate`).
//!
//! Status: TODO(wave-B). Fallback: dumbfire rocket.
//! To implement: On impact release `params.blobs` homing blobs (`sub_projectile` with
//! `params.blob_*`), each leaving a small crater; blobs home like the Seeker.
//! Then return `true` from `implemented` and add scenario tests for the bonus, malus and
//! lasting effect.

use crate::weapons::WeaponBehavior;

/// Behaviour of the Swarm Pod.
#[derive(Debug)]
pub struct SwarmPod;

impl WeaponBehavior for SwarmPod {
    fn id(&self) -> &'static str {
        "swarm_pod"
    }
}
