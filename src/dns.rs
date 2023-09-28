use async_trait::async_trait;
use serde::Deserialize;
use simple_eyre::eyre::{Result, WrapErr};
use trust_dns_resolver::TokioAsyncResolver;

use crate::{Checker as CheckerTrait, UpdateChan};

#[derive(Clone, Default, Deserialize, Debug)]
pub struct Config {
    domain: String,
    // TODO add record type, possibly expected result
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
