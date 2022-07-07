use futures::stream::futures_unordered::FuturesUnordered;
use futures::StreamExt;
use std::collections::HashMap;
use std::error::Error;
use std::fmt::{Debug, Display, Formatter, Result as FmtResult};
use std::fs;
use std::io::{stdin, Read};
use std::time::Duration;

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

impl From<CheckResult> for Result<String, String> {
    fn from(result: CheckResult) -> Result<String, String> {
        if result.failure {
            Err(format!("Checking {}: Failure:\n{}", result.description, result.log))
        } else {
            Ok(format!("Checking {}: Success!", result.description))
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(tag = "type", rename_all="lowercase")]
enum HealthCheck {
    Http { url: String },
    Dns { domain: String },
}

impl HealthCheck {
    async fn do_check(&self, hostname: String) -> CheckResult {
	    match self {
	        HealthCheck::Http { url } => {
                let mut result = CheckResult::new(format!("url '{}' for '{}'", url, hostname));

		        let client = match reqwest::ClientBuilder::new()
                    .timeout(Duration::new(3, 0))
                    .build() {
                    Ok(res) => res,
                    Err(err) => {
                        result.log.push_str(&format!("Unable to construct http client: {}", err.to_string()));
                        result.failure = true;
                        return result;
                    }
                };
		        let response = match client.get(url).send().await {
                    Ok(resp) => resp,
                    Err(error) => {
                        result.log.push_str(&error.to_string());
                        result.failure = true;
                        return result;
                    }
                };

                if !response.status().is_success() {
                    let error = match response.text().await {
                        Ok(body) => body,
                        Err(text) => format!("Unable to read result: {}", text),
                    };

                    result.log.push_str(&error);
                    result.failure = true;
                    return result;
                }

                return result;
	        }
	        HealthCheck::Dns { domain } => {
                let mut result = CheckResult::new(format!("domain '{}' for '{}'", domain, hostname));

		        let resolver = match TokioAsyncResolver::tokio_from_system_conf() {
                    Ok(res) => res,
                    Err(err) => {
                        result.log.push_str(&format!("Unable to construct resolver: {}", err.to_string()));
                        result.failure = true;
                        return result;
                    }
                };
                if let Err(error) = resolver.lookup_ip(domain).await {
                    result.log.push_str(&error.to_string());
                    result.failure = true;
                }

                return result;
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
        let (plural, verb) = if self.number == 1 { ("", "was") } else { ("s", "were") };
        write!(f, "There {} {} failed check{}", verb, self.number, plural)
    }
}
impl Display for ChecksFailedError {
    fn fmt(&self, f: &mut Formatter) -> FmtResult {
        let (plural, verb) = if self.number == 1 { ("", "was") } else { ("s", "were") };
        write!(f, "There {} {} failed check{}", verb, self.number, plural)
    }
}

#[derive(Parser, Debug)]
struct Args {
    #[clap(long="on")]
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
            let target_cfg = config.targets.get(target)
                .ok_or(UnknownTargetError{target: target.clone()})?;

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
            let result = result;
            if result.failure {
                failures += 1;
            }
            print!("{}\n", result);
        }
        failures
    });

    if failures > 0 {
        return Err(Box::new(ChecksFailedError{ number: failures }))
    }
    Ok(())
}
