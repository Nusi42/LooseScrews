//! Interpolation of remote ships between snapshots, rendered a fixed delay
//! (default [`INTERP_DELAY_TICKS`] = 100 ms) behind the newest server time.

use std::collections::VecDeque;

use ls_net::NetShip;

/// Render delay behind the estimated server time, in ticks (6 ticks = 100 ms,
/// three snapshot intervals at 30 Hz, so one lost snapshot does not starve it).
pub const INTERP_DELAY_TICKS: f64 = 6.0;
/// Longest extrapolation past the newest snapshot, in ticks.
pub const MAX_EXTRAPOLATION_TICKS: f64 = 6.0;
/// A ship that moved further than this between two snapshots teleported
/// (respawn); it is not interpolated across the jump.
pub const TELEPORT_DISTANCE: f32 = 8.0;

/// Snapshots of remote ships, by server tick.
#[derive(Debug, Clone)]
pub struct InterpolationBuffer {
    frames: VecDeque<(u32, Vec<NetShip>)>,
    capacity: usize,
}

impl Default for InterpolationBuffer {
    fn default() -> Self {
        InterpolationBuffer::new(32)
    }
}

impl InterpolationBuffer {
    pub fn new(capacity: usize) -> Self {
        InterpolationBuffer {
            frames: VecDeque::new(),
            capacity: capacity.max(2),
        }
    }

    /// Adds the ships of a snapshot. Out-of-order snapshots are inserted in
    /// place; duplicates are ignored.
    pub fn push(&mut self, tick: u32, ships: Vec<NetShip>) {
        let i = self.frames.partition_point(|(t, _)| *t < tick);
        if self.frames.get(i).is_some_and(|(t, _)| *t == tick) {
            return;
        }
        self.frames.insert(i, (tick, ships));
        while self.frames.len() > self.capacity {
            self.frames.pop_front();
        }
    }

    pub fn newest_tick(&self) -> Option<u32> {
        self.frames.back().map(|(t, _)| *t)
    }

    pub fn oldest_tick(&self) -> Option<u32> {
        self.frames.front().map(|(t, _)| *t)
    }

    /// Ship states at (fractional) server tick `t`: linear interpolation of
    /// position and velocity, normalized lerp of orientation. Before the
    /// oldest frame the oldest is used; past the newest, positions are
    /// extrapolated with the velocity for at most [`MAX_EXTRAPOLATION_TICKS`].
    /// Flags and status come from the earlier frame.
    pub fn sample(&self, t: f64) -> Vec<NetShip> {
        let Some((newest, ships)) = self.frames.back() else {
            return Vec::new();
        };
        if t >= *newest as f64 {
            let dt = ((t - *newest as f64).min(MAX_EXTRAPOLATION_TICKS) / ls_math::TICK_HZ as f64)
                as f32;
            return ships
                .iter()
                .map(|s| NetShip {
                    pos: s.pos + s.vel * dt,
                    ..*s
                })
                .collect();
        }
        let j = self.frames.partition_point(|(tick, _)| (*tick as f64) <= t);
        if j == 0 {
            return self.frames[0].1.clone();
        }
        let (ta, a) = &self.frames[j - 1];
        let (tb, b) = &self.frames[j];
        let f = ((t - *ta as f64) / (*tb as f64 - *ta as f64)) as f32;
        a.iter()
            .map(|sa| match b.iter().find(|sb| sb.id == sa.id) {
                Some(sb) if sa.pos.distance(sb.pos) <= TELEPORT_DISTANCE => NetShip {
                    pos: sa.pos.lerp(sb.pos, f),
                    vel: sa.vel.lerp(sb.vel, f),
                    rot: sa.rot.nlerp(sb.rot, f),
                    ..*sa
                },
                Some(sb) if f >= 0.5 => *sb,
                _ => *sa,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ls_math::{Quat, Vec3};
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn ship(id: u8, x: f32) -> NetShip {
        NetShip {
            id,
            pos: Vec3::new(x, 0.0, 0.0),
            vel: Vec3::new(60.0, 0.0, 0.0),
            rot: Quat::IDENTITY,
            flags: 1,
            ext: 0,
            status: None,
        }
    }

    #[test]
    fn interpolates_extrapolates_and_orders() {
        let mut b = InterpolationBuffer::default();
        b.push(4, vec![ship(1, 4.0)]);
        b.push(0, vec![ship(1, 0.0)]);
        b.push(2, vec![ship(1, 2.0)]);
        b.push(2, vec![ship(1, 99.0)]); // duplicate ignored
        assert_eq!(b.oldest_tick(), Some(0));
        assert_eq!(b.sample(1.0)[0].pos.x, 1.0);
        assert_eq!(b.sample(3.5)[0].pos.x, 3.5);
        assert_eq!(b.sample(-3.0)[0].pos.x, 0.0);
        // 60 u/s for 2 ticks past the newest frame.
        assert!((b.sample(6.0)[0].pos.x - 6.0).abs() < 1e-5);
        // Extrapolation is capped.
        assert!((b.sample(100.0)[0].pos.x - 10.0).abs() < 1e-4);
    }

    #[test]
    fn teleports_are_not_interpolated() {
        let mut b = InterpolationBuffer::default();
        b.push(0, vec![ship(1, 0.0)]);
        b.push(2, vec![ship(1, 50.0)]);
        assert_eq!(b.sample(0.5)[0].pos.x, 0.0);
        assert_eq!(b.sample(1.5)[0].pos.x, 50.0);
    }
}
