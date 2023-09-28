use std::time::Duration;

use serde::Deserialize;
use serde_with::{serde_as, DurationSeconds};
use tokio::time::sleep;

#[serde_as]
#[derive(Clone, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Policy {
    max_retries: u16,
    #[serde_as(as = "DurationSeconds<f64>")]
    initial: Duration,
    multiplier: f64,
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
