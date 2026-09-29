//! Input sequence numbering and redundant input messages.

use std::collections::VecDeque;

use ls_net::message::MAX_INPUT_CMDS;
use ls_net::{InputMsg, NetInput};
use ls_sim::InputCmd;

use crate::convert::{input_to_net, net_to_input};

/// Numbers input commands with consecutive ticks and builds [`InputMsg`]s
/// carrying the newest command plus the two before it.
#[derive(Debug, Clone)]
pub struct InputSequencer {
    next_tick: u64,
    recent: VecDeque<NetInput>,
}

impl InputSequencer {
    /// The first command will be for `first_tick`.
    pub fn new(first_tick: u64) -> Self {
        InputSequencer {
            next_tick: first_tick,
            recent: VecDeque::with_capacity(MAX_INPUT_CMDS),
        }
    }

    /// Tick the next command will get.
    pub fn next_tick(&self) -> u64 {
        self.next_tick
    }

    /// Stamps `controls` with the next tick and quantizes it. Returns the
    /// command exactly as the server will decode it; predict with that one.
    pub fn next(&mut self, controls: InputCmd) -> InputCmd {
        let cmd = InputCmd {
            tick: self.next_tick,
            ..controls
        };
        self.next_tick += 1;
        let net = input_to_net(&cmd);
        if self.recent.len() == MAX_INPUT_CMDS {
            self.recent.pop_back();
        }
        self.recent.push_front(net);
        net_to_input(&net)
    }

    /// Input message with the newest commands (newest first).
    pub fn message(&self, ack_snapshot: Option<u32>) -> Option<InputMsg> {
        if self.recent.is_empty() {
            return None;
        }
        Some(InputMsg {
            ack_snapshot,
            cmds: self.recent.iter().copied().collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ls_math::Vec3;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn numbering_and_redundancy() {
        let mut s = InputSequencer::new(10);
        assert!(s.message(None).is_none());
        let a = s.next(InputCmd::idle(0).with_thrust(Vec3::new(0.5, 0.0, 0.0)));
        assert_eq!(a.tick, 10);
        for _ in 0..4 {
            s.next(InputCmd::default());
        }
        let m = s.message(Some(3)).unwrap();
        assert_eq!(m.ack_snapshot, Some(3));
        let ticks: Vec<u32> = m.cmds.iter().map(|c| c.tick).collect();
        assert_eq!(ticks, vec![14, 13, 12]);
        assert_eq!(s.next(InputCmd::default()).tick, 15);
        // The message still encodes (ticks strictly decreasing, <= 255 apart).
        ls_net::Message::Input(s.message(None).unwrap())
            .encode()
            .unwrap();
    }
}
