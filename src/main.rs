use futures::stream::futures_unordered::FuturesUnordered;
use futures::StreamExt;
use std::collections::HashMap;
use std::fmt::{self, Debug, Display};
use std::{fs, future};
use std::io::{stdin, Read};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;

use async_process::{Command, Stdio};
use async_trait::async_trait;
use clap::Parser;
use reqwest;
use serde::Deserialize;
use serde_json;
use simple_eyre::eyre::{eyre, Result, WrapErr};
use tokio::time::{sleep, timeout};
use trust_dns_resolver::TokioAsyncResolver;

struct RetryPolicy {
    max_retries: u16,
    initial: Duration,
    multiplier: f64,
}

struct Retrier {
    policy: RetryPolicy,
    last: Option<Duration>,
    attempts: u16,
}

impl Retrier {
    fn new(policy: RetryPolicy) -> Self {
        Retrier {
            policy,
            last: None,
            attempts: 0,
        }
    }

    async fn retry(&mut self) -> Option<u16> {
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

#[async_trait]
trait Checker {
    fn id(&self) -> String;
    async fn check(&mut self) -> Result<()>;
}

struct SshChecker {
    hostname: String,
    command: String,
    debug: Sender<CheckUpdate>,
    ssh: Command,
}

impl SshChecker {
    fn new(hostname: String, command: String, debug: Sender<CheckUpdate>) -> Box<dyn Checker> {
        let mut ssh = Command::new("ssh");

        ssh.arg(hostname.clone())
            .arg(command.clone())
            .kill_on_drop(true)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        Box::new(SshChecker {
            hostname: hostname.clone(),
            command: command.clone(),
            debug: debug.clone(),
            ssh,
        })
    }
}

#[async_trait]
impl Checker for SshChecker {
    fn id(&self) -> String {
        format!("ssh '{}': {}", self.hostname, self.command)
    }

    async fn check(&mut self) -> Result<()> {
        let ssh_cmd = self.ssh.spawn().wrap_err("Unable to spawn ssh command")?;

        let output = ssh_cmd.output().await.wrap_err("Failed to get output from command")?;

        let mut log = String::new();
        log.push_str("Stdout:\n");
        log.push_str(&String::from_utf8_lossy(&output.stdout));
        log.push_str("Stderr:\n");
        log.push_str(&String::from_utf8_lossy(&output.stderr));

        if !output.status.success() {
            let code = match output.status.code() {
                Some(exit_code) => exit_code.to_string(),
                None => "'none'".to_string(),
            };
            return Err(eyre!("Command returned exit code {}\n{}", code, log));
        }

        send_debug(&self.debug, self.id().to_owned(), log);

        return Ok(());
    }
}

struct HttpChecker {
    url: String,
    client: reqwest::Client,
    debug: Sender<CheckUpdate>,
}

impl HttpChecker {
    fn new(url: String, debug: Sender<CheckUpdate>) -> Result<Box<dyn Checker>> {
        let client = reqwest::ClientBuilder::new()
            .timeout(Duration::new(5, 0))
            .build()
            .wrap_err("Unable to construct http client")?;

        Ok(Box::new(HttpChecker { url, client, debug }))
    }
}

#[async_trait]
impl Checker for HttpChecker {
    fn id(&self) -> String {
        format!("http {}", self.url)
    }

    async fn check(&mut self) -> Result<()> {
        send_debug(&self.debug, self.id().to_owned(), "making request".to_owned());
        let response = self
            .client
            .get(self.url.clone())
            .send()
            .await
            .wrap_err("Error making HTTP request")?;

        let status = response.status();
        send_debug(
            &self.debug,
            self.id().to_owned(),
            format!("response status: {:?}", status),
        );

        if !status.is_success() {
            let error = response
                .text()
                .await
                .wrap_err(format!("Received HTTP error '{}' and unable to read body", status))?;

            return Err(eyre!(error.to_string()));
        }

        return Ok(());
    }
}

struct DnsChecker {
    domain: String,
    resolver: TokioAsyncResolver,
}

impl DnsChecker {
    fn new(domain: String) -> Result<Box<dyn Checker>> {
        let resolver = TokioAsyncResolver::tokio_from_system_conf().wrap_err("Unable to construct resolver")?;

        Ok(Box::new(DnsChecker { domain, resolver }))
    }
}

#[async_trait]
impl Checker for DnsChecker {
    fn id(&self) -> String {
        format!("dns '{}'", self.domain)
    }

    async fn check(&mut self) -> Result<()> {
        self.resolver.lookup_ip(self.domain.clone()).await?;

        Ok(())
    }
}

struct CheckConfig {
    checker: Box<dyn Checker>,
    policy: RetryPolicy,
    timeout: Duration,
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

struct CheckUpdate {
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

async fn run_check(mut cfg: CheckConfig, updates: Sender<CheckUpdate>) -> CheckResult {
    let mut retrier = Retrier::new(cfg.policy);

    loop {
        send_update(&updates, &cfg.checker, CheckStatus::Running);

        match timeout(cfg.timeout, cfg.checker.check())
            .await
            .wrap_err("Check timed out")
        {
            Ok(Ok(_)) => {
                send_update(&updates, &cfg.checker, CheckStatus::Succeeded);
                return CheckResult::Success;
            }
            Err(err) | Ok(Err(err)) => {
                send_update(&updates, &cfg.checker, CheckStatus::Waiting(err.to_string()));
            }
        }

        if retrier.retry().await.is_none() {
            send_update(
                &updates,
                &cfg.checker,
                CheckStatus::Failed("Maximum retries reached".to_owned()),
            );
            return CheckResult::Failure;
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "lowercase")]
enum HealthCheck {
    Http { url: String },
    Dns { domain: String },
    Ssh { command: String },
}

fn make_checker(hostname: String, check_def: HealthCheck, updates: Sender<CheckUpdate>) -> Result<Box<dyn Checker>> {
    Ok(match check_def {
        HealthCheck::Http { url } => HttpChecker::new(url, updates)?,
        HealthCheck::Dns { domain } => DnsChecker::new(domain)?,
        HealthCheck::Ssh { command } => SshChecker::new(hostname, command, updates),
    })
}

#[derive(Deserialize, Debug)]
struct Config {
    targets: HashMap<String, Vec<HealthCheck>>,
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
    let config: Config = serde_json::from_str(&config_data)?;

    let checks = FuturesUnordered::new();
    let (tx, rx) = channel::<CheckUpdate>();

    // TODO fix redundancy
    if let Some(targets) = args.targets {
        for target in targets.iter() {
            let target_cfg = config.targets.get(target).ok_or(eyre!("Unknown target: {}", target))?;

            for check_def in target_cfg.iter() {
                let checker = make_checker(target.clone(), (*check_def).clone(), tx.clone())
                    .wrap_err("Failed to instantiate check")?;
                let check = run_check(
                    CheckConfig {
                        checker,
                        policy: RetryPolicy {
                            max_retries: 3,
                            initial: Duration::from_secs(1),
                            multiplier: 1.1,
                        },
                        timeout: Duration::from_secs(10),
                    },
                    tx.clone(),
                );

                checks.push(check);
            }
        }
    } else {
        for (target, target_cfg) in config.targets.iter() {
            for check_def in target_cfg.iter() {
                let checker = make_checker(target.clone(), (*check_def).clone(), tx.clone())
                    .wrap_err("Failed to instantiate check")?;
                let check = run_check(
                    CheckConfig {
                        checker,
                        policy: RetryPolicy {
                            max_retries: 3,
                            initial: Duration::from_secs(1),
                            multiplier: 1.1,
                        },
                        timeout: Duration::from_secs(10),
                    },
                    tx.clone(),
                );

                checks.push(check);
            }
        }
    }

    drop(tx);

    let printer = rt.spawn(print_output(rx, false));

    let failures = rt.block_on(checks.filter(|res| { future::ready(res.is_failure()) }).count());

    rt.block_on(printer)?;

    if failures > 0 {
        return Err(eyre!("{} check(s) failed", failures));
    }

    Ok(())
}
