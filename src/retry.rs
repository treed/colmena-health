use std::time::Duration;

use merge::Merge;
use serde::Deserialize;
use simple_eyre::eyre::{eyre, Error as EyreError, Result};
use tokio::time::sleep;

#[derive(Clone, Deserialize, Debug, Merge)]
pub struct OptionalPolicy {
    max_retries: Option<u16>,
    initial: Option<f64>,
    multiplier: Option<f64>,
}

impl Default for OptionalPolicy {
    fn default() -> Self {
        OptionalPolicy {
            max_retries: Some(3),
            initial: Some(1.0),
            multiplier: Some(1.1),
        }
    }
}

impl OptionalPolicy {
    pub fn new_empty() -> OptionalPolicy {
        OptionalPolicy {
            max_retries: None,
            initial: None,
            multiplier: None,
        }
    }
}

#[derive(Clone, Deserialize, Debug)]
pub struct Policy {
    max_retries: u16,
    initial: Duration,
    multiplier: f64,
}

impl TryFrom<OptionalPolicy> for Policy {
    type Error = EyreError;

    fn try_from(policy: OptionalPolicy) -> Result<Policy> {
        // could use .ok_or, but it's unstable
        // https://github.com/rust-lang/rust/issues/91930
        let max_retries = match policy.max_retries {
            Some(max_retries) => max_retries,
            None => return Err(eyre!("'max_retries' is a required field for ssh checks")),
        };

        let initial = match policy.initial {
            Some(initial) => Duration::from_secs_f64(initial),
            None => return Err(eyre!("'initial' is a required field for ssh checks")),
        };

        let multiplier = match policy.multiplier {
            Some(multiplier) => multiplier,
            None => return Err(eyre!("'multiplier' is a required field for ssh checks")),
        };

        Ok(Policy {
            max_retries,
            initial,
            multiplier,
        })
    }
}

pub struct Retrier {
    policy: Policy,
    last: Option<Duration>,
    attempts: u16,
}

impl Retrier {
    pub fn new(policy: Policy) -> Self {
        Retrier {
            policy,
            last: None,
            attempts: 0,
        }
    }

    pub async fn retry(&mut self) -> Option<u16> {
        if self.attempts >= self.policy.max_retries {
            return None;
        }

        let dur = match self.last {
            None => self.policy.initial,
            Some(last_dur) => last_dur.mul_f64(self.policy.multiplier),
        };

        sleep(dur).await;

        self.last = Some(dur);
        self.attempts += 1;

        Some(self.attempts)
    }
}
