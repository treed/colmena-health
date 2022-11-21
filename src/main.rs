use futures::stream::futures_unordered::FuturesUnordered;
use futures::StreamExt;
use std::fmt::{self, Debug, Display};
use std::io::{stdin, Read};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;
use std::{fs, future};

use async_process::{Command, Stdio};
use async_trait::async_trait;
use clap::Parser;
use merge::Merge;
use reqwest;
use serde::Deserialize;
use serde_json;
use simple_eyre::eyre::{eyre, Error as EyreError, Result, WrapErr};
use tokio::time::{sleep, timeout as tokio_timeout};
use trust_dns_resolver::TokioAsyncResolver;

#[derive(Clone, Deserialize, Debug, Merge)]
struct OptionalRetryPolicy {
    max_retries: Option<u16>,
    initial: Option<f64>,
    multiplier: Option<f64>,
}

impl Default for OptionalRetryPolicy {
    fn default() -> Self {
        OptionalRetryPolicy {
            max_retries: Some(3),
            initial: Some(1.0),
            multiplier: Some(1.1),
        }
    }
}

#[derive(Clone, Deserialize, Debug)]
struct RetryPolicy {
    max_retries: u16,
    initial: Duration,
    multiplier: f64,
}

impl TryFrom<OptionalRetryPolicy> for RetryPolicy {
    type Error = EyreError;

    fn try_from(policy: OptionalRetryPolicy) -> Result<RetryPolicy> {
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

        Ok(RetryPolicy {
            max_retries,
            initial,
            multiplier,
        })
    }
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
    config: SshConfig,
    debug: Sender<CheckUpdate>,
    ssh: Command,
}

impl SshChecker {
    fn new(config: SshConfig, debug: Sender<CheckUpdate>) -> Box<dyn Checker> {
        let mut ssh = Command::new("ssh");

        ssh.arg(config.hostname.clone())
            .arg(config.command.clone())
            .kill_on_drop(true)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        Box::new(SshChecker {
            config,
            debug: debug.clone(),
            ssh,
        })
    }
}

#[async_trait]
impl Checker for SshChecker {
    fn id(&self) -> String {
        format!("ssh '{}': {}", self.config.hostname, self.config.command)
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
    config: HttpConfig,
    client: reqwest::Client,
    debug: Sender<CheckUpdate>,
}

impl HttpChecker {
    fn new(config: HttpConfig, debug: Sender<CheckUpdate>) -> Result<Box<dyn Checker>> {
        let client = reqwest::ClientBuilder::new()
            .timeout(Duration::new(5, 0))
            .build()
            .wrap_err("Unable to construct http client")?;

        Ok(Box::new(HttpChecker { config, client, debug }))
    }
}

#[async_trait]
impl Checker for HttpChecker {
    fn id(&self) -> String {
        format!("http {}", self.config.url)
    }

    async fn check(&mut self) -> Result<()> {
        send_debug(&self.debug, self.id().to_owned(), "making request".to_owned());
        let response = self
            .client
            .get(self.config.url.clone())
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
    config: DnsConfig,
    resolver: TokioAsyncResolver,
}

impl DnsChecker {
    fn new(config: DnsConfig) -> Result<Box<dyn Checker>> {
        let resolver = TokioAsyncResolver::tokio_from_system_conf().wrap_err("Unable to construct resolver")?;

        Ok(Box::new(DnsChecker { config, resolver }))
    }
}

#[async_trait]
impl Checker for DnsChecker {
    fn id(&self) -> String {
        format!("dns '{}'", self.config.domain)
    }

    async fn check(&mut self) -> Result<()> {
        self.resolver.lookup_ip(self.config.domain.clone()).await?;

        Ok(())
    }
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

async fn run_check(
    mut checker: Box<dyn Checker>,
    policy: RetryPolicy,
    timeout: Duration,
    updates: Sender<CheckUpdate>,
) -> CheckResult {
    let mut retrier = Retrier::new(policy);

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

#[derive(Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "lowercase")]
enum HealthCheck {
    Http { url: String },
    Dns { domain: String },
    Ssh { command: String },
}

#[derive(Deserialize, Debug)]
struct CheckDefinition {
    retry_policy: Option<OptionalRetryPolicy>,

    #[serde(flatten)]
    config: CheckConfig,
}

#[derive(Clone, Deserialize, Debug)]
#[serde(tag = "type", content = "params", rename_all = "lowercase")]
enum CheckConfig {
    Http(OptionalHttpConfig),
    Dns(OptionalDnsConfig),
    Ssh(OptionalSshConfig),
}

#[derive(Clone, Deserialize, Debug, Merge)]
struct OptionalSshConfig {
    command: Option<String>,
    hostname: Option<String>,
}

impl Default for OptionalSshConfig {
    fn default() -> Self {
        OptionalSshConfig {
            command: None,
            hostname: None,
        }
    }
}

#[derive(Debug)]
struct SshConfig {
    command: String,
    hostname: String,
}

impl TryFrom<OptionalSshConfig> for SshConfig {
    type Error = EyreError;

    fn try_from(cfg: OptionalSshConfig) -> Result<SshConfig> {
        // could use .ok_or, but it's unstable
        // https://github.com/rust-lang/rust/issues/91930
        let command = match cfg.command {
            Some(command) => command,
            None => return Err(eyre!("'command' is a required field for ssh checks")),
        };

        let hostname = match cfg.hostname {
            Some(hostname) => hostname,
            None => return Err(eyre!("'hostname' is a required field for ssh checks")),
        };

        Ok(SshConfig {
            command,
            hostname,
        })
    }
}

#[derive(Clone, Deserialize, Debug, Merge)]
struct OptionalDnsConfig {
    domain: Option<String>,
    // TODO add record type, possibly expected result
}

impl Default for OptionalDnsConfig {
    fn default() -> Self {
        OptionalDnsConfig { domain: None }
    }
}

#[derive(Debug)]
struct DnsConfig {
    domain: String,
}

impl TryFrom<OptionalDnsConfig> for DnsConfig {
    type Error = EyreError;

    fn try_from(cfg: OptionalDnsConfig) -> Result<DnsConfig> {
        // could use .ok_or, but it's unstable
        // https://github.com/rust-lang/rust/issues/91930
        let domain = match cfg.domain {
            Some(domain) => domain,
            None => return Err(eyre!("'domain' is a required field for dns checks")),
        };

        Ok(DnsConfig { domain })
    }
}

#[derive(Clone, Deserialize, Debug, Merge)]
struct OptionalHttpConfig {
    url: Option<String>,
    // TODO expected status codes
}

impl Default for OptionalHttpConfig {
    fn default() -> Self {
        OptionalHttpConfig { url: None }
    }
}

#[derive(Debug)]
struct HttpConfig {
    url: String,
}

impl TryFrom<OptionalHttpConfig> for HttpConfig {
    type Error = EyreError;

    fn try_from(cfg: OptionalHttpConfig) -> Result<HttpConfig> {
        // could use .ok_or, but it's unstable
        // https://github.com/rust-lang/rust/issues/91930
        let url = match cfg.url {
            Some(url) => url,
            None => return Err(eyre!("'url' is a required field for http checks")),
        };

        Ok(HttpConfig { url })
    }
}

#[derive(Deserialize, Debug, Default, Merge)]
struct ConfigDefaults {
    ssh: Option<OptionalSshConfig>,
    dns: Option<OptionalDnsConfig>,
    http: Option<OptionalHttpConfig>,
    retry_policy: Option<OptionalRetryPolicy>,
}

#[derive(Deserialize, Debug)]
struct Config {
    defaults: Option<ConfigDefaults>,
    checks: Vec<CheckDefinition>,
}

#[derive(Parser, Debug)]
struct Args {
    #[clap(long = "on")]
    targets: Option<Vec<String>>,
    config_file: String,
}

fn merge_configs<T>(global: Option<T>, mut check: T) -> T
where
    T: Merge + Default,
{
    if let Some(mut g) = global {
        g.merge(T::default());
        check.merge(g);
    } else {
        check.merge(T::default());
    }

    check
}

fn prepare_config<T, F>(global: Option<T>, mut check: T) -> Result<F>
where
    T: Merge + Default,
    F: TryFrom<T, Error = EyreError>,
{
    if let Some(mut g) = global {
        g.merge(T::default());
        check.merge(g);
    } else {
        check.merge(T::default());
    }

    F::try_from(check)
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
    let config_defaults = config.defaults.unwrap_or_default();

    let checks = FuturesUnordered::new();
    let (tx, rx) = channel::<CheckUpdate>();

    for check_def in config.checks.into_iter() {
        let checker = match check_def.config {
            CheckConfig::Http(http_config) => {
                HttpChecker::new(prepare_config(config_defaults.http.clone(), http_config)?, tx.clone())?
            }
            CheckConfig::Dns(dns_config) => DnsChecker::new(prepare_config(config_defaults.dns.clone(), dns_config)?)?,
            CheckConfig::Ssh(ssh_config) => {
                SshChecker::new(prepare_config(config_defaults.ssh.clone(), ssh_config)?, tx.clone())
            }
        };
        let check = run_check(
            checker,
            RetryPolicy::try_from(merge_configs(
                config_defaults.retry_policy.clone(),
                check_def.retry_policy.clone().unwrap_or_default(),
            ))?,
            Duration::from_secs(10),
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
