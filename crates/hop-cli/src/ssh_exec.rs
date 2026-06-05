use std::{
    io::{self, Write},
    process::{Command, Stdio},
};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use crate::config::{SshTarget, PUBLIC_KEY_ONLY_AUTH_OPTIONS};

#[derive(Debug, Clone, Deserialize)]
struct ListedAsset {
    name: String,
    hostname: String,
    port: i64,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    credential_id: Option<String>,
}

pub fn interactive_shell(target: &SshTarget) -> Result<()> {
    run_status(&mut build_ssh_command(target, None, false))
}

pub fn exec(target: &SshTarget, command: &str, allocate_tty: bool) -> Result<()> {
    run_status(&mut build_ssh_command(target, Some(command), allocate_tty))
}

pub fn list_assets(target: &SshTarget) -> Result<()> {
    let output = run_output(&mut build_ssh_command(
        target,
        Some("hop-list-assets"),
        false,
    ))?;
    let assets: Vec<ListedAsset> =
        serde_json::from_slice(&output).context("parse hop-list-assets response")?;
    io::stdout().write_all(format_asset_table(&assets).as_bytes())?;
    Ok(())
}

fn build_ssh_command(target: &SshTarget, command: Option<&str>, allocate_tty: bool) -> Command {
    let mut ssh = Command::new("ssh");
    ssh.arg("-p").arg(target.port.to_string());
    for (name, value) in PUBLIC_KEY_ONLY_AUTH_OPTIONS {
        ssh.arg("-o").arg(format!("{name}={value}"));
    }
    if command.is_some() && !allocate_tty {
        ssh.arg("-n");
        ssh.arg("-o").arg("BatchMode=yes");
        ssh.arg("-o").arg("LogLevel=ERROR");
    }
    if allocate_tty {
        ssh.arg("-tt");
    }
    ssh.arg(target.destination());
    if let Some(command) = command {
        ssh.arg(command);
    }
    if command.is_some() && !allocate_tty {
        ssh.stdin(Stdio::null());
    } else {
        ssh.stdin(Stdio::inherit());
    }
    ssh.stdout(Stdio::inherit()).stderr(Stdio::inherit());
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

fn run_output(command: &mut Command) -> Result<Vec<u8>> {
    let output = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;
    if output.status.success() {
        return Ok(output.stdout);
    }

    let mut message = if let Some(code) = output.status.code() {
        ssh_failure_message(code)
    } else {
        format!("ssh exited with status {}", output.status)
    };
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stderr.trim().is_empty() {
        message.push_str("\n\nssh stderr:\n");
        message.push_str(stderr.trim_end());
    }
    bail!("{message}")
}

fn format_asset_table(assets: &[ListedAsset]) -> String {
    if assets.is_empty() {
        return "No assets found.\n".to_string();
    }

    let headers = ["NAME", "ADDRESS", "CREDENTIAL", "TAGS", "DESCRIPTION"];
    let rows = assets
        .iter()
        .map(|asset| {
            [
                asset.name.clone(),
                format!("{}:{}", asset.hostname, asset.port),
                if asset.credential_id.is_some() {
                    "yes".to_string()
                } else {
                    "no".to_string()
                },
                if asset.tags.is_empty() {
                    "-".to_string()
                } else {
                    asset.tags.join(", ")
                },
                asset
                    .description
                    .as_deref()
                    .map(str::trim)
                    .filter(|description| !description.is_empty())
                    .unwrap_or("-")
                    .to_string(),
            ]
        })
        .collect::<Vec<_>>();

    let mut widths = headers.map(str::len);
    for row in &rows {
        for (index, column) in row.iter().enumerate() {
            widths[index] = widths[index].max(column.len());
        }
    }

    let mut output = String::new();
    push_table_row(&mut output, &headers.map(str::to_string), &widths);
    push_table_row(&mut output, &widths.map(|width| "-".repeat(width)), &widths);
    for row in &rows {
        push_table_row(&mut output, row, &widths);
    }
    output
}

fn push_table_row(output: &mut String, columns: &[String; 5], widths: &[usize; 5]) {
    for (index, column) in columns.iter().enumerate() {
        if index > 0 {
            output.push_str("  ");
        }
        output.push_str(column);
        for _ in column.len()..widths[index] {
            output.push(' ');
        }
    }
    output.push('\n');
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
    fn non_tty_exec_does_not_wait_for_terminal_input_or_show_banner() {
        let target = SshTarget::new("hop".to_string(), "127.0.0.1".to_string(), 2222);
        let command = build_ssh_command(&target, Some("hop-list-assets"), false);
        let args = arg_strings(&command);

        assert!(args.iter().any(|arg| arg == "-n"));
        assert!(args.windows(2).any(|pair| pair == ["-o", "LogLevel=ERROR"]));
    }

    #[test]
    fn tty_exec_does_not_force_batch_mode() {
        let target = SshTarget::new("hop".to_string(), "127.0.0.1".to_string(), 2222);
        let command = build_ssh_command(&target, Some("hop-connect web-prod-01"), true);
        let args = arg_strings(&command);

        assert!(!args.windows(2).any(|pair| pair == ["-o", "BatchMode=yes"]));
    }

    #[test]
    fn asset_list_output_is_a_readable_table() {
        let assets = vec![ListedAsset {
            name: "test".to_string(),
            hostname: "10.225.37.43".to_string(),
            port: 22,
            description: None,
            tags: Vec::new(),
            credential_id: Some("credential-id".to_string()),
        }];

        let output = format_asset_table(&assets);

        assert!(output.contains("NAME"));
        assert!(output.contains("ADDRESS"));
        assert!(output.contains("CREDENTIAL"));
        assert!(output.contains("test"));
        assert!(output.contains("10.225.37.43:22"));
        assert!(output.contains("yes"));
        assert!(!output.trim_start().starts_with('['));
        assert!(!output.contains("credential_id"));
    }

    #[test]
    fn empty_asset_list_says_there_are_no_assets() {
        assert_eq!(format_asset_table(&[]), "No assets found.\n");
    }
}
