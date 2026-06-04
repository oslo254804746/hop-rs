use std::process::{Command, Stdio};

use anyhow::{bail, Result};

use crate::config::SshTarget;

pub fn interactive_shell(target: &SshTarget) -> Result<()> {
    run_status(
        Command::new("ssh")
            .arg("-p")
            .arg(target.port.to_string())
            .arg(target.destination())
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit()),
    )
}

pub fn exec(target: &SshTarget, command: &str, allocate_tty: bool) -> Result<()> {
    let mut ssh = Command::new("ssh");
    ssh.arg("-p")
        .arg(target.port.to_string())
        .arg(target.destination());
    if allocate_tty {
        ssh.arg("-tt");
    }
    ssh.arg(command)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    run_status(&mut ssh)
}

fn run_status(command: &mut Command) -> Result<()> {
    let status = command.status()?;
    if status.success() {
        Ok(())
    } else {
        bail!("ssh exited with status {status}")
    }
}
