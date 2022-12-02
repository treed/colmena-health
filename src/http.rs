use std::sync::mpsc::Sender;

use async_trait::async_trait;
use merge::Merge;
use serde::Deserialize;
use simple_eyre::eyre::{eyre, Error as EyreError, Result, WrapErr};

use crate::{send_debug, CheckUpdate, Checker as CheckerTrait};

#[derive(Clone, Default, Deserialize, Debug, Merge)]
pub struct OptionalConfig {
    url: Option<String>,
    // TODO expected status codes
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
        send_debug(&self.debug, self.id(), "making request".to_owned());
        let response = self
            .client
            .get(self.config.url.clone())
            .send()
            .await
            .wrap_err("Error making HTTP request")?;

        let status = response.status();
        send_debug(&self.debug, self.id(), format!("response status: {:?}", status));

        if !status.is_success() {
            let error = response
                .text()
                .await
                .wrap_err(format!("Received HTTP error '{}' and unable to read body", status))?;

            return Err(eyre!(error));
        }

        return Ok(());
    }
}
