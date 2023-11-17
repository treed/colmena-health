use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use simple_eyre::eyre::{Result, WrapErr};
use time::OffsetDateTime;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::time::{interval, MissedTickBehavior};

use crate::{CheckInfo, CheckStatus, CheckUpdate};

#[derive(Clone, Serialize, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PostableAlert {
    #[serde(with = "time::serde::rfc3339::option")]
    starts_at: Option<OffsetDateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "time::serde::rfc3339::option")]
    ends_at: Option<OffsetDateTime>,

    labels: HashMap<String, String>,
    annotations: HashMap<String, String>,

    #[serde(rename = "generatorURL")]
    #[serde(skip_serializing_if = "Option::is_none")]
    generator_url: Option<String>,
}

pub struct AlertManagerClient {
    active_alerts: HashMap<usize, PostableAlert>,
    client: reqwest::Client,
    realert_interval: std::time::Duration,
    registry: HashMap<usize, CheckInfo>,
    updates: UnboundedReceiver<CheckUpdate>,
    url: String,
}

impl AlertManagerClient {
    pub fn new(
        base_url: String,
        realert_interval: std::time::Duration,
        registry: HashMap<usize, CheckInfo>,
        updates: UnboundedReceiver<CheckUpdate>,
    ) -> Result<Self> {
        Ok(AlertManagerClient {
            active_alerts: HashMap::new(),
            client: reqwest::ClientBuilder::new()
                .build()
                .wrap_err("Unable to construct http client")?,
            realert_interval,
            registry,
            updates,
            url: format!("{base_url}/alerts"),
        })
    }

    async fn process_update(&mut self, update: CheckUpdate) {
        match update.status {
            CheckStatus::Failed => {
                // The await doesn't really work with entry or_insert
                #[allow(clippy::map_entry)]
                if !self.active_alerts.contains_key(&update.id) {
                    // TODO report this error
                    let info = self.registry.get(&update.id).unwrap();
                    let alert = PostableAlert {
                        starts_at: Some(time::OffsetDateTime::now_utc()),
                        ends_at: None,
                        labels: info.labels.clone(),
                        annotations: info.annotations.clone(),
                        generator_url: None,
                    };

                    self.active_alerts.insert(update.id, alert);
                    self.send_alerts().await;
                }
            }
            CheckStatus::Succeeded => {
                if let Some(alert) = self.active_alerts.get_mut(&update.id) {
                    alert.ends_at = Some(time::OffsetDateTime::now_utc());
                    self.send_alerts().await;

                    self.active_alerts.remove(&update.id);
                }
            }
            _ => {}
        }
    }

    async fn send_alerts(&self) {
        let alerts: Vec<&PostableAlert> = self.active_alerts.values().collect();
        self.client.post(&self.url).json(&alerts).send().await.unwrap(); // TODO report this error
    }

    pub async fn run(mut self) {
        let mut interval = interval(self.realert_interval);
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if !self.active_alerts.is_empty() {
                        self.send_alerts().await;
                    }
                }
                update = self.updates.recv() => {
                    match update {
                        Some(update) => self.process_update(update).await,
                        None => {
                            self.send_alerts().await;
                            return
                        }
                    }
                }
            }
        }
    }
}
