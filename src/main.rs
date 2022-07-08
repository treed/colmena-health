use futures::stream::futures_unordered::FuturesUnordered;
use futures::StreamExt;
use std::collections::HashMap;
use std::error::Error;
use std::fmt::{Debug, Display, Formatter, Result as FmtResult};
use std::fs;
use std::io::{stdin, Read};
use std::time::Duration;

use async_process::{Command, Stdio};
use clap::Parser;
use reqwest;
use serde::Deserialize;
use serde_json;
use trust_dns_resolver::TokioAsyncResolver;

#[derive(Debug)]
struct CheckResult {
    description: String,
    log: String,
    failure: bool,
}

impl CheckResult {
    fn new(description: String) -> Self {
        CheckResult {
            description,
            failure: false,
            log: String::new(),
        }
    }
}

impl CheckResult {
    fn fail(mut self, message: String) -> CheckResult {
        self.failure = true;
        self.log.push_str(&message);
        self
    }

    fn err<S, E>(&mut self, message: S, error: E) -> String
    where
        S: Into<String>,
        E: std::error::Error,
    {
        self.failure = true;
        self.log
            .push_str(&format!("{}: {}", message.into(), error.to_string()));
        format!("{}", self)
    }
}

impl Display for CheckResult {
    fn fmt(&self, f: &mut Formatter) -> FmtResult {
        write!(f, "Checking {}: ", self.description)?;
        if self.failure {
            write!(f, "Failed:\n{}", self.log)?;
        } else {
            write!(f, "Success")?;
        }
        Ok(())
    }
}

impl Into<Result<String, String>> for CheckResult {
    fn into(self) -> Result<String, String> {
        let output = format!("{}", self);
        if self.failure {
            Err(output)
        } else {
            Ok(output)
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(tag = "type", rename_all = "lowercase")]
enum HealthCheck {
    Http { url: String },
    Dns { domain: String },
    Ssh { command: String },
}

impl HealthCheck {
    async fn do_check(&self, hostname: String) -> Result<String, String> {
        match self {
            HealthCheck::Http { url } => {
                let mut result = CheckResult::new(format!("url '{}' for '{}'", url, hostname));

                let client = reqwest::ClientBuilder::new()
                    .timeout(Duration::new(5, 0))
                    .build()
                    .map_err(|err| result.err("Unable to construct http client", err))?;

                let response = client
                    .get(url)
                    .send()
                    .await
                    .map_err(|err| result.err("Error making HTTP request", err))?;

                if !response.status().is_success() {
                    let error = response
                        .text()
                        .await
                        .map_err(|err| result.err("Unable to read result", err))?;

                    return result.fail(error).into();
                }

                return result.into();
            }
            HealthCheck::Dns { domain } => {
                let mut result =
                    CheckResult::new(format!("domain '{}' for '{}'", domain, hostname));

                let resolver = TokioAsyncResolver::tokio_from_system_conf()
                    .map_err(|err| result.err("Unable to construct resolver", err))?;

                if let Err(error) = resolver.lookup_ip(domain).await {
                    result.log.push_str(&error.to_string());
                    result.failure = true;
                }

                return result.into();
            }
            HealthCheck::Ssh { command } => {
                let mut result = CheckResult::new(format!(
                    "via ssh to {} with command '{}'",
                    hostname, command
                ));

                let ssh = Command::new("ssh")
                    .arg(hostname)
                    .arg(command)
                    .kill_on_drop(true)
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .map_err(|err| result.err("Unable to spawn ssh command", err))?;

                let output = ssh
                    .output()
                    .await
                    .map_err(|err| result.err("Failed to get output from command", err))?;

                if !output.status.success() {
                    result.failure = true;
                    let code = match output.status.code() {
                        Some(exit_code) => exit_code.to_string(),
                        None => "'none'".to_string(),
                    };
                    result
                        .log
                        .push_str(&format!("Command returned exit code {}\n", code));
                }

                result.log.push_str("Stdout:\n");
                result
                    .log
                    .push_str(&String::from_utf8_lossy(&output.stdout));
                result.log.push_str("Stderr:\n");
                result
                    .log
                    .push_str(&String::from_utf8_lossy(&output.stderr));

                return result.into();
            }
        }
    }
}

#[derive(Deserialize, Debug)]
struct Config {
    targets: HashMap<String, Vec<HealthCheck>>,
}

struct UnknownTargetError {
    target: String,
}

impl Error for UnknownTargetError {}
impl Debug for UnknownTargetError {
    fn fmt(&self, f: &mut Formatter) -> FmtResult {
        write!(f, "Unknown target: {}", self.target)
    }
}
impl Display for UnknownTargetError {
    fn fmt(&self, f: &mut Formatter) -> FmtResult {
        write!(f, "Unknown target: {}", self.target)
    }
}

struct ChecksFailedError {
    number: usize,
}

impl Error for ChecksFailedError {}
impl Debug for ChecksFailedError {
    fn fmt(&self, f: &mut Formatter) -> FmtResult {
        let (plural, verb) = if self.number == 1 {
            ("", "was")
        } else {
            ("s", "were")
        };
        write!(f, "There {} {} failed check{}", verb, self.number, plural)
    }
}
impl Display for ChecksFailedError {
    fn fmt(&self, f: &mut Formatter) -> FmtResult {
        let (plural, verb) = if self.number == 1 {
            ("", "was")
        } else {
            ("s", "were")
        };
        write!(f, "There {} {} failed check{}", verb, self.number, plural)
    }
}

#[derive(Parser, Debug)]
struct Args {
    #[clap(long = "on")]
    targets: Option<Vec<String>>,
    config_file: String,
}

fn main() -> Result<(), Box<dyn std::error::Error + 'static>> {
    let args = Args::parse();
    let rt = tokio::runtime::Runtime::new().unwrap();

    let config_data = if args.config_file == "-" {
        let mut buf = String::new();
        stdin().read_to_string(&mut buf)?;
        buf
    } else {
        fs::read_to_string(args.config_file)?
    };
    let config: Config = serde_json::from_str(&config_data)?;

    let mut checks = FuturesUnordered::new();

    if let Some(targets) = args.targets {
        for target in targets.iter() {
            let target_cfg = config.targets.get(target).ok_or(UnknownTargetError {
                target: target.clone(),
            })?;

            for check in target_cfg.iter() {
                checks.push(check.do_check(target.clone()));
            }
        }
    } else {
        for (target, target_cfg) in config.targets.iter() {
            for check in target_cfg.iter() {
                checks.push(check.do_check(target.clone()));
            }
        }
    }

    let failures = rt.block_on(async {
        let mut failures = 0;
        while let Some(result) = checks.next().await {
            match result {
                Ok(log) => print!("{}\n", log),
                Err(log) => {
                    failures += 1;
                    print!("{}\n", log);
                }
            }
        }
        failures
    });

    if failures > 0 {
        return Err(Box::new(ChecksFailedError { number: failures }));
    }
    Ok(())
}
