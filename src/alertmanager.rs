use std::collections::HashMap;

use log::{error, info};
use serde::{Deserialize, Serialize};
use simple_eyre::eyre::{Result, WrapErr};
use time::OffsetDateTime;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::time::{interval, MissedTickBehavior};

use crate::alert::Config as AlertConfig;
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
    alert_config: AlertConfig,
    client: reqwest::Client,
    registry: HashMap<usize, CheckInfo>,
    updates: UnboundedReceiver<CheckUpdate>,
    url: String,
}

impl AlertManagerClient {
    pub fn new(
        alert_config: AlertConfig,
        registry: HashMap<usize, CheckInfo>,
        updates: UnboundedReceiver<CheckUpdate>,
    ) -> Result<Self> {
        Ok(AlertManagerClient {
            active_alerts: HashMap::new(),
            // having url out of order avoids a copy
            url: format!("{}/alerts", &alert_config.base_url),
            alert_config,
            client: reqwest::ClientBuilder::new()
                .build()
                .wrap_err("Unable to construct http client")?,
            registry,
            updates,
        })
    }

    async fn process_update(&mut self, update: CheckUpdate) {
        match update.status {
            CheckStatus::Failed => {
                // The await doesn't really work with entry or_insert
                #[allow(clippy::map_entry)]
                if !self.active_alerts.contains_key(&update.id) {
                    if let Some(info) = self.registry.get(&update.id) {
                        let mut alert = PostableAlert {
                            starts_at: Some(time::OffsetDateTime::now_utc()),
                            ends_at: None,
                            labels: info.labels.clone(),
                            annotations: info.annotations.clone(),
                            generator_url: None,
                        };

                        if self.alert_config.allow_output_annotation {
                            // Combining these ifs is an unstable feature
                            if let Some(ref output) = update.msg {
                                alert.annotations.insert("output".to_owned(), output.clone());
                            };
                        }

                        info!("Check failed - {}", info.name);
                        self.active_alerts.insert(update.id, alert);
                        self.send_alerts().await;
                    } else {
                        error!(
                            "Tried to send an alert for id {}, which was not in the registry; skipping transmission",
                            update.id
                        );
                    }
                }
            }
            CheckStatus::Succeeded => {
                if let Some(alert) = self.active_alerts.get_mut(&update.id) {
                    alert.ends_at = Some(time::OffsetDateTime::now_utc());
                    info!("Check passing again: {:?}", alert.labels);

                    self.send_alerts().await;
                    self.active_alerts.remove(&update.id);
                }
            }
            _ => {}
        }
    }

    async fn send_alerts(&self) {
        let alerts: Vec<&PostableAlert> = self.active_alerts.values().collect();
        if let Err(e) = self.client.post(&self.url).json(&alerts).send().await {
            error!("Failure sending alerts: {}", e);
        }
    }

    pub async fn run(mut self) {
        let mut interval = interval(self.alert_config.realert_interval);
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
