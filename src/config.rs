use std::collections::HashMap;

use serde::Deserialize;

use simple_eyre::eyre::Result;

use crate::{dns, http, retry, ssh, Checker as CheckerTrait};

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CheckDefinition {
    pub retry_policy: retry::Policy,
    pub check_timeout: f64,
    pub labels: HashMap<String, String>,

    #[serde(flatten)]
    pub config: CheckConfig,
}

#[derive(Clone, Deserialize, Debug)]
#[serde(tag = "type", content = "params", rename_all = "lowercase")]
pub enum CheckConfig {
    Http(http::Config),
    Dns(dns::Config),
    Ssh(ssh::Config),
}

impl CheckConfig {
    pub fn into_check(self, id: usize) -> Result<Box<dyn CheckerTrait>> {
        Ok(match self {
            CheckConfig::Http(http_config) => Box::new(http::Checker::new(id, http_config)?),
            CheckConfig::Dns(dns_config) => Box::new(dns::Checker::new(id, dns_config)?),
            CheckConfig::Ssh(ssh_config) => Box::new(ssh::Checker::new(id, ssh_config)),
        })
    }
}

#[derive(Deserialize, Debug)]
pub struct Config {
    pub checks: Vec<CheckDefinition>,
}
