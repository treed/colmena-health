use tokio::sync::mpsc::UnboundedSender;

use async_trait::async_trait;
use merge::Merge;
use serde::Deserialize;
use simple_eyre::eyre::{eyre, Error as EyreError, Result, WrapErr};

use crate::{send_update, CheckStatus, CheckUpdate, Checker as CheckerTrait};

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
    id: usize,
    config: Config,
    client: reqwest::Client,
    updates: UnboundedSender<CheckUpdate>,
}

impl Checker {
    pub fn new(id: usize, config: Config, updates: UnboundedSender<CheckUpdate>) -> Result<Self> {
        let client = reqwest::ClientBuilder::new()
            .build()
            .wrap_err("Unable to construct http client")?;

        Ok(Checker {
            id,
            config,
            client,
            updates,
        })
    }
}

#[async_trait]
impl CheckerTrait for Checker {
    fn id(&self) -> usize {
        self.id
    }

    fn name(&self) -> String {
        format!("http {}", self.config.url)
    }

    async fn check(&self) -> Result<()> {
        send_update(
            &self.updates,
            self.id(),
            CheckStatus::Running,
            "making request".to_owned(),
        );

        let response = self
            .client
            .get(self.config.url.clone())
            .send()
            .await
            .wrap_err("Error making HTTP request")?;

        let status = response.status();
        send_update(
            &self.updates,
            self.id(),
            CheckStatus::Running,
            format!("response status: {:?}", status),
        );

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
