use std::{collections::HashMap, time::Duration};

use futures::stream::FuturesUnordered;
use futures::StreamExt;
use serde::Deserialize;
use serde_with::{serde_as, DurationSeconds};
use simple_eyre::eyre::Result;
use tokio::{sync::mpsc::UnboundedReceiver, time::sleep};

use crate::{run_check, CheckStatus, CheckUpdate, RunnableCheck};

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

async fn report_alerts(registry: HashMap<usize, String>, mut updates: UnboundedReceiver<CheckUpdate>) {
    let unknown = "unknown check".to_owned();

    while let Some(update) = updates.recv().await {
        let name = registry.get(&update.id).unwrap_or(&unknown);

        println!("{}: {}", name, update.status);

        if let Some(msg) = update.msg {
            for line in msg.lines() {
                println!("    {}", line);
            }
        }
    }
}

pub fn run_alerts(
    checks: Vec<RunnableCheck>,
    registry: HashMap<usize, String>,
    rx: UnboundedReceiver<CheckUpdate>,
) -> Result<()> {
    let checks: FuturesUnordered<_> = checks.into_iter().map(run_check_for_alerts).collect();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_time()
        .enable_io()
        .worker_threads(4)
        .build()?;

    let printer = rt.spawn(report_alerts(registry, rx));

    rt.block_on(checks.count());

    rt.block_on(printer)?;
    Ok(())
}
