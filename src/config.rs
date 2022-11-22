use merge::Merge;
use serde::Deserialize;
use simple_eyre::eyre::{Error as EyreError, Result};

use crate::{dns, http, retry, ssh};

#[derive(Deserialize, Debug)]
pub struct CheckDefinition {
    pub retry_policy: Option<retry::OptionalPolicy>,
    pub check_timeout: Option<f64>,

    #[serde(flatten)]
    pub config: CheckConfig,
}

#[derive(Clone, Deserialize, Debug)]
#[serde(tag = "type", content = "params", rename_all = "lowercase")]
pub enum CheckConfig {
    Http(http::OptionalConfig),
    Dns(dns::OptionalConfig),
    Ssh(ssh::OptionalConfig),
}

#[derive(Deserialize, Debug, Default, Merge)]
pub struct ConfigDefaults {
    pub ssh: Option<ssh::OptionalConfig>,
    pub dns: Option<dns::OptionalConfig>,
    pub http: Option<http::OptionalConfig>,
    pub retry_policy: Option<retry::OptionalPolicy>,
}

#[derive(Deserialize, Debug)]
pub struct Config {
    pub defaults: Option<ConfigDefaults>,
    pub checks: Vec<CheckDefinition>,
}

pub fn prepare<T, F>(global: Option<T>, mut check: T) -> Result<F>
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
