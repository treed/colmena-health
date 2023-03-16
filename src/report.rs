use std::{collections::HashMap, future};

use futures::stream::FuturesUnordered;
use futures::StreamExt;
use simple_eyre::eyre::{eyre, Result};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::{run_check, CheckUpdate, RunnableCheck};

async fn print_verbose(registry: HashMap<usize, String>, mut rx: UnboundedReceiver<CheckUpdate>) {
    let unknown = "unknown check".to_owned();

    while let Some(update) = rx.recv().await {
        let name = registry.get(&update.id).unwrap_or(&unknown);

        println!("{}: {}", name, update.status);

        if let Some(msg) = update.msg {
            for line in msg.lines() {
                println!("    {}", line);
            }
        }
    }
}

pub fn run_report(
    checks: Vec<RunnableCheck>,
    registry: HashMap<usize, String>,
    rx: UnboundedReceiver<CheckUpdate>,
) -> Result<()> {
    let checks: FuturesUnordered<_> = checks.into_iter().map(run_check).collect();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_time()
        .enable_io()
        .worker_threads(4)
        .build()?;

    let printer = rt.spawn(print_verbose(registry, rx));

    let failures = rt.block_on(checks.filter(|res| future::ready(res.is_failure())).count());

    rt.block_on(printer)?;

    if failures > 0 {
        return Err(eyre!("{} check(s) failed", failures));
    }

    Ok(())
}
