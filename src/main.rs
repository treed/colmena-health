use config::CheckConfig;
use futures::stream::futures_unordered::FuturesUnordered;
use futures::StreamExt;
use std::fmt::{self, Debug, Display};
use std::io::{stdin, Read};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;
use std::{fs, future};

use async_trait::async_trait;
use clap::Parser;
use simple_eyre::eyre::{eyre, Result, WrapErr};
use tokio::time::timeout as tokio_timeout;

mod config;
mod dns;
mod http;
mod retry;
mod ssh;

#[async_trait]
pub trait Checker {
    fn id(&self) -> String;
    async fn check(&mut self) -> Result<()>;
}

enum CheckStatus {
    Running,
    Waiting(String),
    Succeeded,
    Failed(String),
}

impl Display for CheckStatus {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            CheckStatus::Running => write!(f, "Running"),
            CheckStatus::Waiting(last_failure) => write!(f, "Waiting after failure: {}", last_failure),
            CheckStatus::Succeeded => write!(f, "Succeeded"),
            CheckStatus::Failed(reason) => write!(f, "Failed: {}", reason),
        }
    }
}

enum CheckMessage {
    Status(CheckStatus),
    Debug(String),
}

impl Display for CheckMessage {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            CheckMessage::Status(status) => write!(f, "status update: {}", status),
            CheckMessage::Debug(debug) => write!(f, "debug: {}", debug),
        }
    }
}

pub struct CheckUpdate {
    id: String,
    msg: CheckMessage,
}

impl CheckUpdate {
    fn is_debug(&self) -> bool {
        if let CheckMessage::Debug(_) = &self.msg {
            return true;
        }
        return false;
    }
}

impl Display for CheckUpdate {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "[{}] {}", self.id, self.msg)
    }
}

async fn print_output(rx: Receiver<CheckUpdate>, verbose: bool) {
    while let Ok(update) = rx.recv() {
        if verbose || !update.is_debug() {
            print!("{}\n", update);
        }
    }
}

fn send_update(updates: &Sender<CheckUpdate>, checker: &Box<dyn Checker>, status: CheckStatus) {
    if let Err(_) = updates.send(CheckUpdate {
        id: checker.id().to_owned(),
        msg: CheckMessage::Status(status),
    }) {
        // TODO handle this error; I guess print to stderr?
    }
}

fn send_debug(updates: &Sender<CheckUpdate>, id: String, debug_msg: String) {
    if let Err(_) = updates.send(CheckUpdate {
        id,
        msg: CheckMessage::Debug(debug_msg),
    }) {
        // TODO handle this error; I guess print to stderr?
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

async fn run_check(
    mut checker: Box<dyn Checker>,
    policy: retry::Policy,
    timeout: Duration,
    updates: Sender<CheckUpdate>,
) -> CheckResult {
    let mut retrier = retry::Retrier::new(policy);

    loop {
        send_update(&updates, &checker, CheckStatus::Running);

        match tokio_timeout(timeout, checker.check())
            .await
            .wrap_err("Check timed out")
        {
            Ok(Ok(_)) => {
                send_update(&updates, &checker, CheckStatus::Succeeded);
                return CheckResult::Success;
            }
            Err(err) | Ok(Err(err)) => {
                send_update(&updates, &checker, CheckStatus::Waiting(err.to_string()));
            }
        }

        if retrier.retry().await.is_none() {
            send_update(
                &updates,
                &checker,
                CheckStatus::Failed("Maximum retries reached".to_owned()),
            );
            return CheckResult::Failure;
        }
    }
}
#[derive(Parser, Debug)]
struct Args {
    #[clap(long = "on")]
    targets: Option<Vec<String>>,
    config_file: String,
}

fn main() -> Result<()> {
    simple_eyre::install()?;

    let args = Args::parse();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_time()
        .enable_io()
        .worker_threads(4)
        .build()?;

    let config_data = if args.config_file == "-" {
        let mut buf = String::new();
        stdin().read_to_string(&mut buf)?;
        buf
    } else {
        fs::read_to_string(args.config_file)?
    };
    let config: config::Config = serde_json::from_str(&config_data)?;
    let config_defaults = config.defaults.unwrap_or_default();

    let checks = FuturesUnordered::new();
    let (tx, rx) = channel::<CheckUpdate>();

    for check_def in config.checks.into_iter() {
        let checker = match check_def.config {
            CheckConfig::Http(http_config) => {
                http::Checker::new(config::prepare(config_defaults.http.clone(), http_config)?, tx.clone())?
            }
            CheckConfig::Dns(dns_config) => {
                dns::Checker::new(config::prepare(config_defaults.dns.clone(), dns_config)?)?
            }
            CheckConfig::Ssh(ssh_config) => {
                ssh::Checker::new(config::prepare(config_defaults.ssh.clone(), ssh_config)?, tx.clone())
            }
        };
        let check = run_check(
            checker,
            config::prepare(
                config_defaults.retry_policy.clone(),
                check_def.retry_policy.unwrap_or_else(retry::OptionalPolicy::new_empty),
            )?,
            Duration::from_secs_f64(check_def.check_timeout.unwrap_or(10.0)),
            tx.clone(),
        );

        checks.push(check);
    }

    drop(tx);

    let printer = rt.spawn(print_output(rx, false));

    let failures = rt.block_on(checks.filter(|res| future::ready(res.is_failure())).count());

    rt.block_on(printer)?;

    if failures > 0 {
        return Err(eyre!("{} check(s) failed", failures));
    }

    Ok(())
}
