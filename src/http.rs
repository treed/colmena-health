use async_trait::async_trait;
use serde::Deserialize;
use simple_eyre::eyre::{eyre, Result, WrapErr};

use crate::{CheckStatus, Checker as CheckerTrait, UpdateChan};

#[derive(Clone, Default, Deserialize, Debug)]

pub struct Config {
    url: String,
    // TODO expected status codes
}

pub struct Checker {
    id: usize,
    config: Config,
    client: reqwest::Client,
}

impl Checker {
    pub fn new(id: usize, config: Config) -> Result<Self> {
        let client = reqwest::ClientBuilder::new()
            .build()
            .wrap_err("Unable to construct http client")?;

        Ok(Checker { id, config, client })
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

    async fn check(&self, updates: &UpdateChan) -> Result<()> {
        updates.send(CheckStatus::Running, "making request".to_owned());

        let response = self
            .client
            .get(self.config.url.clone())
            .send()
            .await
            .wrap_err("Error making HTTP request")?;

        let status = response.status();
        updates.send(CheckStatus::Running, format!("response status: {:?}", status));

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
