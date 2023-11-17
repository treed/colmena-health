use std::collections::HashMap;
use std::fmt::{self, Debug, Display};
use std::fs;
use std::io::{stdin, Read};
use std::rc::Rc;
use std::time::Duration;

use alert::run_alerts;
use async_trait::async_trait;
use clap::Parser;
use log::{debug, error};
use simple_eyre::eyre::{Result, WrapErr};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use tokio::time::timeout as tokio_timeout;

use report::run_report;

mod alert;
mod alertmanager;
mod config;
mod dns;
mod http;
mod report;
mod retry;
mod select;
mod ssh;

#[async_trait]
pub trait Checker {
    fn id(&self) -> usize;
    fn name(&self) -> String;
    async fn check(&self, updates: &UpdateChan) -> Result<()>;
}

enum CheckStatus {
    // Currently Running
    Running,
    // Waiting for Retry
    Retrying,
    // Waiting to Run
    Waiting(Duration, String),
    // Check succeeded
    Succeeded,
    // Check failed
    Failed,
}

impl Display for CheckStatus {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            CheckStatus::Running => write!(f, "Running"),
            CheckStatus::Retrying => write!(f, "Waiting to retry"),
            CheckStatus::Waiting(secs, waiting_for) => write!(f, "Waiting {:?} for {}", secs, waiting_for),
            CheckStatus::Succeeded => write!(f, "Succeeded"),
            CheckStatus::Failed => write!(f, "Failed:"),
        }
    }
}

#[derive(Clone)]
pub struct CheckInfo {
    name: String,
    labels: HashMap<String, String>,
    annotations: HashMap<String, String>,
}

pub struct CheckUpdate {
    id: usize,
    status: CheckStatus,
    msg: Option<String>,
}

#[derive(Clone)]
pub struct UpdateChan {
    id: usize,
    updates: UnboundedSender<CheckUpdate>,
}

impl UpdateChan {
    fn new(id: usize, updates: UnboundedSender<CheckUpdate>) -> Self {
        UpdateChan { id, updates }
    }

    fn send<M>(&self, status: CheckStatus, msg: M)
    where
        M: Into<Option<String>>,
    {
        if self
            .updates
            .send(CheckUpdate {
                id: self.id,
                status,
                msg: msg.into(),
            })
            .is_err()
        {
            error!("Unable to send CheckUpdate for ID {}", self.id);
        }
    }
}

enum CheckResult {
    Success,
    Failure,
}

impl CheckResult {
    fn is_failure(&self) -> bool {
        if let CheckResult::Failure = self {
            return true;
        }

        false
    }
}

#[derive(Clone)]
pub struct RunnableCheck {
    alert_policy: alert::Policy,
    checker: Rc<dyn Checker>,
    retry_policy: retry::Policy,
    timeout: Duration,
    updates: UpdateChan,
}

async fn run_check(check: RunnableCheck) -> CheckResult {
    let mut retrier = retry::Retrier::new(check.retry_policy.clone());
    debug!("Running check - {}", check.checker.name());

    loop {
        check.updates.send(CheckStatus::Running, None);

        match tokio_timeout(check.timeout, check.checker.check(&check.updates))
            .await
            .wrap_err("Check timed out")
        {
            Ok(Ok(_)) => {
                check.updates.send(CheckStatus::Succeeded, None);
                return CheckResult::Success;
            }
            Err(err) | Ok(Err(err)) => {
                check.updates.send(CheckStatus::Retrying, err.to_string());
            }
        }

        if retrier.retry().await.is_none() {
            check
                .updates
                .send(CheckStatus::Failed, "Maximum retries reached".to_owned());
            return CheckResult::Failure;
        }
    }
}
#[derive(Parser, Debug)]
struct Args {
    /// A label-based query selector, e.g. hostname:web-1,web-2
    #[clap(short, long)]
    select: Option<String>,
    /// The configuration file containing check definitions
    config_file: String,
    /// Enable alerting mode
    #[clap(long)]
    alert: bool,
}

fn main() -> Result<()> {
    simple_eyre::install()?;

    std_logger::Config::logfmt().init();

    let args = Args::parse();

    let label_selector: Option<select::Term> = match args.select {
        Some(sel) => Some(sel.parse()?),
        None => None,
    };

    let config_data = if args.config_file == "-" {
        let mut buf = String::new();
        stdin().read_to_string(&mut buf)?;
        buf
    } else {
        fs::read_to_string(args.config_file)?
    };
    let config: config::Config = serde_json::from_str(&config_data)?;

    let mut checks = Vec::new();
    let (tx, rx) = unbounded_channel::<CheckUpdate>();

    let mut check_registry = HashMap::new();

    for (id, check_def) in config.checks.into_iter().enumerate() {
        if let Some(ref sel) = label_selector {
            if !sel.matches(&check_def.labels) {
                continue;
            }
        }

        let checker = check_def.config.clone().into_check(id)?;
        check_registry.insert(
            id,
            CheckInfo {
                name: checker.name(),
                labels: check_def.labels.clone(),
                annotations: check_def.annotations.clone(),
            },
        );

        let runnable = RunnableCheck {
            alert_policy: check_def.alert_policy,
            checker,
            retry_policy: check_def.retry_policy,
            timeout: check_def.check_timeout,
            updates: UpdateChan::new(id, tx.clone()),
        };

        checks.push(runnable);
    }

    drop(tx);

    if args.alert {
        if let Some(alert_cfg) = config.alerting {
            run_alerts(checks, check_registry, rx, alert_cfg)?;
        }
    } else {
        run_report(checks, check_registry, rx)?;
    }

    Ok(())
}

#[test]
fn verify_cli() {
    use clap::CommandFactory;
    Args::command().debug_assert();
}
