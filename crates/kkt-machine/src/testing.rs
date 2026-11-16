//! A stand-in [`Machine`]: the facts it is given.

use crate::{Env, Machine, Platform};

/// A machine whose facts are fixed when it is made.
#[derive(Clone, Debug)]
pub struct FixedMachine {
    pub env: Env,
    pub platform: Platform,
    pub process_id: u32,
}

impl Machine for FixedMachine {
    fn env(&self) -> Env {
        self.env.clone()
    }
    fn platform(&self) -> Platform {
        self.platform
    }
    fn process_id(&self) -> u32 {
        self.process_id
    }
}
