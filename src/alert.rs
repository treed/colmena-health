use std::{collections::HashMap, time::Duration};

use futures::stream::FuturesUnordered;
use futures::StreamExt;
use serde::Deserialize;
use serde_with::{serde_as, DurationSeconds};
use simple_eyre::eyre::Result;
use tokio::{sync::mpsc::UnboundedReceiver, time::sleep};

use crate::{alertmanager, run_check, CheckInfo, CheckStatus, CheckUpdate, RunnableCheck};

#[serde_as]
#[derive(Clone, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    #[serde(rename = "baseURL")]
    pub base_url: String,
    #[serde_as(as = "DurationSeconds<f64>")]
    pub realert_interval: Duration,
    pub allow_output_annotation: bool,
}

#[serde_as]
#[derive(Clone, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Policy {
    #[serde_as(as = "DurationSeconds<f64>")]
    check_interval: Duration,
    #[serde_as(as = "DurationSeconds<f64>")]
    recheck_interval: Duration,
}

pub async fn run_check_for_alerts(check: RunnableCheck) {
    let policy: Policy = check.alert_policy.clone();

    loop {
        loop {
            let result = run_check(check.clone()).await;
            if !result.is_failure() {
                break;
            }

            check.updates.send(
                CheckStatus::Waiting(policy.recheck_interval, "recheck".to_owned()),
                None,
            );
            sleep(policy.recheck_interval).await;
        }
        check.updates.send(
            CheckStatus::Waiting(policy.check_interval, "next check".to_owned()),
            None,
        );
        sleep(policy.check_interval).await;
    }
}

pub fn run_alerts(
    checks: Vec<RunnableCheck>,
    registry: HashMap<usize, CheckInfo>,
    rx: UnboundedReceiver<CheckUpdate>,
    cfg: Config,
) -> Result<()> {
    let checks: FuturesUnordered<_> = checks.into_iter().map(run_check_for_alerts).collect();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_time()
        .enable_io()
        .worker_threads(4)
        .build()?;

    let printer = rt.spawn(alertmanager::AlertManagerClient::new(cfg, registry, rx)?.run());

    rt.block_on(checks.count());

    rt.block_on(printer)?;
    Ok(())
}
