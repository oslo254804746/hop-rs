use anyhow::{bail, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecCommand {
    Version,
    ListAssets,
    Connect { asset: String },
}

pub fn parse_exec_command(bytes: &[u8]) -> Result<ExecCommand> {
    let raw = std::str::from_utf8(bytes)?.trim();
    match raw {
        "hop-version" => Ok(ExecCommand::Version),
        "hop-list-assets" => Ok(ExecCommand::ListAssets),
        _ => {
            let Some(asset) = raw.strip_prefix("hop-connect ") else {
                bail!("unsupported exec command");
            };
            let asset = asset.trim();
            if asset.is_empty() || asset.contains(char::is_whitespace) {
                bail!("hop-connect requires a single asset name");
            }
            Ok(ExecCommand::Connect {
                asset: asset.to_string(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_supported_exec_commands_only() {
        assert_eq!(parse_exec_command(b"hop-version").unwrap(), ExecCommand::Version);
        assert_eq!(parse_exec_command(b"hop-list-assets").unwrap(), ExecCommand::ListAssets);
        assert_eq!(
            parse_exec_command(b"hop-connect web-prod-01").unwrap(),
            ExecCommand::Connect {
                asset: "web-prod-01".to_string()
            }
        );
        assert!(parse_exec_command(b"admin-delete-all").is_err());
        assert!(parse_exec_command(b"hop-connect web prod").is_err());
    }
}
