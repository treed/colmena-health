use async_process::{Command, Stdio};
use async_trait::async_trait;
use merge::Merge;
use serde::Deserialize;
use simple_eyre::eyre::{eyre, Error as EyreError, Result, WrapErr};

use crate::{CheckStatus, Checker as CheckerTrait, UpdateChan};

#[derive(Clone, Deserialize, Debug, Merge)]
pub struct OptionalConfig {
    command: Option<String>,
    hostname: Option<String>,
    user: Option<String>,
}

impl Default for OptionalConfig {
    fn default() -> Self {
        OptionalConfig {
            command: None,
            hostname: None,
            user: Some("root".to_owned()),
        }
    }
}

#[derive(Debug)]
pub struct Config {
    command: String,
    hostname: String,
    user: Option<String>,
}

impl TryFrom<OptionalConfig> for Config {
    type Error = EyreError;

    fn try_from(cfg: OptionalConfig) -> Result<Config> {
        // could use .ok_or, but it's unstable
        // https://github.com/rust-lang/rust/issues/91930
        let command = match cfg.command {
            Some(command) => command,
            None => return Err(eyre!("'command' is a required field for ssh checks")),
        };

        let hostname = match cfg.hostname {
            Some(hostname) => hostname,
            None => return Err(eyre!("'hostname' is a required field for ssh checks")),
        };

        Ok(Config {
            command,
            hostname,
            user: cfg.user,
        })
    }
}

pub struct Checker {
    id: usize,
    config: Config,
}

impl Checker {
    pub fn new(id: usize, config: Config) -> Self {
        Checker { id, config }
    }
}

#[async_trait]
impl CheckerTrait for Checker {
    fn id(&self) -> usize {
        self.id
    }

    fn name(&self) -> String {
        format!("ssh {}: '{}'", self.config.hostname, self.config.command)
    }

    async fn check(&self, updates: &UpdateChan) -> Result<()> {
        let mut ssh = Command::new("ssh");
        ssh.kill_on_drop(true)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        ssh.arg(self.config.hostname.clone());

        if let Some(ref user) = self.config.user {
            ssh.arg(format!("-l{}", user));
        }

        ssh.arg(self.config.command.clone());

        let ssh_cmd = ssh.spawn().wrap_err("Unable to spawn ssh command")?;

        let output = ssh_cmd.output().await.wrap_err("Failed to get output from command")?;

        let mut log = String::new();
        log.push_str("Stdout:\n");
        log.push_str(&String::from_utf8_lossy(&output.stdout));
        log.push_str("Stderr:\n");
        log.push_str(&String::from_utf8_lossy(&output.stderr));

        if !output.status.success() {
            let code = match output.status.code() {
                Some(exit_code) => exit_code.to_string(),
                None => "'none'".to_string(),
            };
            return Err(eyre!("Command returned exit code {}\n{}", code, log));
        }

        updates.send(CheckStatus::Running, log);

        return Ok(());
    }
}
