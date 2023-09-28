use async_process::{Command, Stdio};
use async_trait::async_trait;
use serde::Deserialize;
use simple_eyre::eyre::{eyre, Result, WrapErr};

use crate::{CheckStatus, Checker as CheckerTrait, UpdateChan};

#[derive(Clone, Deserialize, Debug)]
pub struct Config {
    command: String,
    hostname: String,
    user: Option<String>,
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
