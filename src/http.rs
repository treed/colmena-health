use std::sync::mpsc::Sender;
use std::time::Duration;

use async_trait::async_trait;
use merge::Merge;
use reqwest;
use serde::Deserialize;
use simple_eyre::eyre::{eyre, Error as EyreError, Result, WrapErr};

use crate::{send_debug, CheckUpdate, Checker as CheckerTrait};

#[derive(Clone, Deserialize, Debug, Merge)]
pub struct OptionalConfig {
    url: Option<String>,
    // TODO expected status codes
}

impl Default for OptionalConfig {
    fn default() -> Self {
        OptionalConfig { url: None }
    }
}

#[derive(Debug)]
pub struct Config {
    url: String,
}

impl TryFrom<OptionalConfig> for Config {
    type Error = EyreError;

    fn try_from(cfg: OptionalConfig) -> Result<Config> {
        // could use .ok_or, but it's unstable
        // https://github.com/rust-lang/rust/issues/91930
        let url = match cfg.url {
            Some(url) => url,
            None => return Err(eyre!("'url' is a required field for http checks")),
        };

        Ok(Config { url })
    }
}

pub struct Checker {
    config: Config,
    client: reqwest::Client,
    debug: Sender<CheckUpdate>,
}

impl Checker {
    pub fn new(config: Config, debug: Sender<CheckUpdate>) -> Result<Box<dyn CheckerTrait>> {
        let client = reqwest::ClientBuilder::new()
            .timeout(Duration::new(5, 0))
            .build()
            .wrap_err("Unable to construct http client")?;

        Ok(Box::new(Checker { config, client, debug }))
    }
}

#[async_trait]
impl CheckerTrait for Checker {
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
