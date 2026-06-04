use std::process::{Command, Stdio};

use anyhow::{bail, Result};

use crate::config::{SshTarget, PUBLIC_KEY_ONLY_AUTH_OPTIONS};

pub fn interactive_shell(target: &SshTarget) -> Result<()> {
    run_status(&mut build_ssh_command(target, None, false))
}

pub fn exec(target: &SshTarget, command: &str, allocate_tty: bool) -> Result<()> {
    run_status(&mut build_ssh_command(target, Some(command), allocate_tty))
}

fn build_ssh_command(target: &SshTarget, command: Option<&str>, allocate_tty: bool) -> Command {
    let mut ssh = Command::new("ssh");
    ssh.arg("-p").arg(target.port.to_string());
    for (name, value) in PUBLIC_KEY_ONLY_AUTH_OPTIONS {
        ssh.arg("-o").arg(format!("{name}={value}"));
    }
    if command.is_some() && !allocate_tty {
        ssh.arg("-o").arg("BatchMode=yes");
    }
    if allocate_tty {
        ssh.arg("-tt");
    }
    ssh.arg(target.destination());
    if let Some(command) = command {
        ssh.arg(command);
    }
    ssh.stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    ssh
}

fn run_status(command: &mut Command) -> Result<()> {
    let status = command.status()?;
    if status.success() {
        Ok(())
    } else if let Some(code) = status.code() {
        bail!("{}", ssh_failure_message(code))
    } else {
        bail!("ssh exited with status {status}")
    }
}

fn ssh_failure_message(exit_code: i32) -> String {
    if exit_code != 255 {
        return format!("ssh exited with status exit code: {exit_code}");
    }

    [
        "ssh exited with status exit code: 255",
        "",
        "Hop SSH connection failed.",
        "If OpenSSH reported \"Connection refused\", check that hop-server is running and that --host/--port point at its SSH bind address.",
        "If OpenSSH reported \"Permission denied (publickey...)\", Hop login uses authorized SSH keys on the Hop server; credentials are target credentials used after `hop connect <asset>` or the TUI selects an asset.",
        "",
        "For public-key login, add your public key on the server, then retry:",
        "  hop-server --config config.toml key add --name \"your laptop\" --public-key-file ~/.ssh/id_ed25519.pub",
    ]
    .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arg_strings(command: &Command) -> Vec<String> {
        command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn ssh_commands_use_public_key_only_auth_for_hop_login() {
        let target = SshTarget::new("hop".to_string(), "127.0.0.1".to_string(), 2222);
        let command = build_ssh_command(&target, Some("hop-list-assets"), false);
        let args = arg_strings(&command);

        assert!(args
            .windows(2)
            .any(|pair| pair == ["-o", "PreferredAuthentications=publickey"]));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["-o", "PasswordAuthentication=no"]));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["-o", "KbdInteractiveAuthentication=no"]));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["-o", "HostbasedAuthentication=no"]));
        assert_eq!(args.last().map(String::as_str), Some("hop-list-assets"));
    }

    #[test]
    fn ssh_exit_255_help_explains_keys_are_not_credentials() {
        let message = ssh_failure_message(255);

        assert!(message.contains("Hop SSH connection failed"));
        assert!(message.contains("Connection refused"));
        assert!(message.contains("authorized SSH keys"));
        assert!(message.contains("credentials are target credentials"));
        assert!(message.contains("hop-server"));
        assert!(message.contains("key add"));
    }

    #[test]
    fn tty_flag_is_an_ssh_option_not_part_of_remote_command() {
        let target = SshTarget::new("hop".to_string(), "127.0.0.1".to_string(), 2222);
        let command = build_ssh_command(&target, Some("hop-connect web-prod-01"), true);
        let args = arg_strings(&command);

        let tty_index = args.iter().position(|arg| arg == "-tt").unwrap();
        let destination_index = args.iter().position(|arg| arg == "hop@127.0.0.1").unwrap();
        let remote_command_index = args
            .iter()
            .position(|arg| arg == "hop-connect web-prod-01")
            .unwrap();

        assert!(tty_index < destination_index);
        assert!(destination_index < remote_command_index);
    }

    #[test]
    fn non_tty_exec_uses_batch_mode_to_fail_instead_of_prompting() {
        let target = SshTarget::new("hop".to_string(), "127.0.0.1".to_string(), 2222);
        let command = build_ssh_command(&target, Some("hop-list-assets"), false);
        let args = arg_strings(&command);

        assert!(args.windows(2).any(|pair| pair == ["-o", "BatchMode=yes"]));
    }

    #[test]
    fn tty_exec_does_not_force_batch_mode() {
        let target = SshTarget::new("hop".to_string(), "127.0.0.1".to_string(), 2222);
        let command = build_ssh_command(&target, Some("hop-connect web-prod-01"), true);
        let args = arg_strings(&command);

        assert!(!args.windows(2).any(|pair| pair == ["-o", "BatchMode=yes"]));
    }
}
