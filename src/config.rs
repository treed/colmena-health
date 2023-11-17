use std::time::Duration;
use std::{collections::HashMap, rc::Rc};

use serde::Deserialize;
use serde_with::{serde_as, DurationSeconds};

use simple_eyre::eyre::Result;

use crate::{alert, dns, http, retry, ssh, Checker as CheckerTrait};

#[serde_as]
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CheckDefinition {
    pub retry_policy: retry::Policy,
    #[serde_as(as = "DurationSeconds<f64>")]
    pub check_timeout: Duration,
    pub labels: HashMap<String, String>,
    pub annotations: HashMap<String, String>,
    pub alert_policy: alert::Policy,

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
    pub fn into_check(self, id: usize) -> Result<Rc<dyn CheckerTrait>> {
        Ok(match self {
            CheckConfig::Http(http_config) => Rc::new(http::Checker::new(id, http_config)?),
            CheckConfig::Dns(dns_config) => Rc::new(dns::Checker::new(id, dns_config)?),
            CheckConfig::Ssh(ssh_config) => Rc::new(ssh::Checker::new(id, ssh_config)),
        })
    }
}

#[derive(Deserialize, Debug)]
pub struct Config {
    pub alerting: Option<alert::Config>,
    pub checks: Vec<CheckDefinition>,
}
