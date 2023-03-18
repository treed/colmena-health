use async_trait::async_trait;
use merge::Merge;
use serde::Deserialize;
use simple_eyre::eyre::{eyre, Error as EyreError, Result, WrapErr};
use trust_dns_resolver::TokioAsyncResolver;

use crate::{Checker as CheckerTrait, UpdateChan};

#[derive(Clone, Default, Deserialize, Debug, Merge)]
pub struct OptionalConfig {
    domain: Option<String>,
    // TODO add record type, possibly expected result
}

#[derive(Debug)]
pub struct Config {
    domain: String,
}

impl TryFrom<OptionalConfig> for Config {
    type Error = EyreError;

    fn try_from(cfg: OptionalConfig) -> Result<Config> {
        // could use .ok_or, but it's unstable
        // https://github.com/rust-lang/rust/issues/91930
        let domain = match cfg.domain {
            Some(domain) => domain,
            None => return Err(eyre!("'domain' is a required field for dns checks")),
        };

        Ok(Config { domain })
    }
}

pub struct Checker {
    id: usize,
    config: Config,
    resolver: TokioAsyncResolver,
}

impl Checker {
    pub fn new(id: usize, config: Config) -> Result<Self> {
        let resolver = TokioAsyncResolver::tokio_from_system_conf().wrap_err("Unable to construct resolver")?;

        Ok(Checker { id, config, resolver })
    }
}

#[async_trait]
impl CheckerTrait for Checker {
    fn id(&self) -> usize {
        self.id
    }

    fn name(&self) -> String {
        format!("dns {}", self.config.domain)
    }

    async fn check(&self, _updates: &UpdateChan) -> Result<()> {
        self.resolver.lookup_ip(self.config.domain.clone()).await?;

        Ok(())
    }
}
