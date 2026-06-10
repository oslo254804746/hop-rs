mod admin;
mod ssh;
mod tui;

use std::{
    fs,
    io::{self, Read},
    path::PathBuf,
    sync::Arc,
};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use hop_core::{
    load_or_create_master_key, AssetAccessMode, AuthType, HopConfig, HopDb, MasterKey, NewAsset,
    ASSET_PRESET_RDP, ASSET_PROTOCOL_SSH, ASSET_PROTOCOL_TCP,
};
use tracing::{info, warn};

use crate::admin::transfer::{ConflictPolicy, TransferFormat, TransferKind};

#[derive(Debug, Parser)]
#[command(
    name = "hop-server",
    version,
    about = "Hop lightweight SSH jump server"
)]
struct Cli {
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    Serve,
    ResetAdmin,
    Key {
        #[command(subcommand)]
        command: KeyCommand,
    },
    Credential {
        #[command(subcommand)]
        command: CredentialCommand,
    },
    Asset {
        #[command(subcommand)]
        command: AssetCommand,
    },
    Export {
        #[arg(long, value_enum, default_value = "assets")]
        kind: TransferKindArg,
        #[arg(long, value_enum, default_value = "json")]
        format: TransferFormatArg,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    Import {
        #[arg(long, value_enum, default_value = "assets")]
        kind: TransferKindArg,
        #[arg(long)]
        file: PathBuf,
        #[arg(long, value_enum)]
        format: Option<TransferFormatArg>,
        #[arg(long, value_enum, default_value = "skip")]
        on_conflict: ConflictPolicyArg,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum TransferKindArg {
    Assets,
    Credentials,
}

impl From<TransferKindArg> for TransferKind {
    fn from(value: TransferKindArg) -> Self {
        match value {
            TransferKindArg::Assets => TransferKind::Assets,
            TransferKindArg::Credentials => TransferKind::Credentials,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum TransferFormatArg {
    Csv,
    Json,
}

impl From<TransferFormatArg> for TransferFormat {
    fn from(value: TransferFormatArg) -> Self {
        match value {
            TransferFormatArg::Csv => TransferFormat::Csv,
            TransferFormatArg::Json => TransferFormat::Json,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ConflictPolicyArg {
    Skip,
    Overwrite,
    Error,
}

impl From<ConflictPolicyArg> for ConflictPolicy {
    fn from(value: ConflictPolicyArg) -> Self {
        match value {
            ConflictPolicyArg::Skip => ConflictPolicy::Skip,
            ConflictPolicyArg::Overwrite => ConflictPolicy::Overwrite,
            ConflictPolicyArg::Error => ConflictPolicy::Error,
        }
    }
}

#[derive(Debug, Subcommand)]
enum KeyCommand {
    Add {
        #[arg(long)]
        name: String,
        #[arg(long)]
        public_key: Option<String>,
        #[arg(long)]
        public_key_file: Option<PathBuf>,
    },
    List,
    Deactivate {
        id: String,
    },
    Activate {
        id: String,
    },
    Access {
        #[command(subcommand)]
        command: KeyAccessCommand,
    },
}

#[derive(Debug, Subcommand)]
enum KeyAccessCommand {
    Show {
        id: String,
    },
    Set {
        id: String,
        #[arg(long, value_enum)]
        mode: AssetAccessModeArg,
        #[arg(long = "asset-id")]
        asset_ids: Vec<String>,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum AssetAccessModeArg {
    All,
    Restricted,
}

impl From<AssetAccessModeArg> for AssetAccessMode {
    fn from(value: AssetAccessModeArg) -> Self {
        match value {
            AssetAccessModeArg::All => AssetAccessMode::All,
            AssetAccessModeArg::Restricted => AssetAccessMode::Restricted,
        }
    }
}

#[derive(Debug, Clone, ValueEnum)]
enum AuthTypeArg {
    Password,
    Key,
    KeyPassphrase,
}

impl From<AuthTypeArg> for AuthType {
    fn from(value: AuthTypeArg) -> Self {
        match value {
            AuthTypeArg::Password => AuthType::Password,
            AuthTypeArg::Key => AuthType::Key,
            AuthTypeArg::KeyPassphrase => AuthType::KeyWithPassphrase,
        }
    }
}

#[derive(Debug, Subcommand)]
enum CredentialCommand {
    Add {
        #[arg(long)]
        name: String,
        #[arg(long)]
        username: String,
        #[arg(long, value_enum)]
        auth_type: AuthTypeArg,
        #[arg(long)]
        password: Option<String>,
        #[arg(long, conflicts_with = "password")]
        password_stdin: bool,
        #[arg(long)]
        private_key_file: Option<PathBuf>,
        #[arg(long)]
        passphrase: Option<String>,
    },
    List,
    Delete {
        id: String,
    },
}

#[derive(Debug, Clone, ValueEnum)]
enum AssetProtocolArg {
    Ssh,
    Rdp,
    Tcp,
    Vnc,
    Mysql,
    Postgres,
    Redis,
}

impl AssetProtocolArg {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Ssh => ASSET_PROTOCOL_SSH,
            Self::Rdp => ASSET_PRESET_RDP,
            Self::Tcp => ASSET_PROTOCOL_TCP,
            Self::Vnc => "vnc",
            Self::Mysql => "mysql",
            Self::Postgres => "postgres",
            Self::Redis => "redis",
        }
    }
}

#[derive(Debug, Subcommand)]
enum AssetCommand {
    Add {
        #[arg(long)]
        name: String,
        #[arg(long, value_enum, default_value = "ssh")]
        protocol: AssetProtocolArg,
        #[arg(long)]
        hostname: String,
        #[arg(long, default_value_t = 22)]
        port: i64,
        #[arg(long)]
        description: Option<String>,
        #[arg(long, value_delimiter = ',')]
        tags: Vec<String>,
        #[arg(long)]
        credential_id: Option<String>,
    },
    List,
    Delete {
        id: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => serve(cli.config).await,
        Command::ResetAdmin => {
            let (db, _, _) = open_runtime(cli.config).await?;
            let password = admin::bootstrap::reset_admin_password(&db).await?;
            println!("New Hop admin password: {password}");
            Ok(())
        }
        Command::Key { command } => {
            let (db, _, _) = open_runtime(cli.config).await?;
            match command {
                KeyCommand::Add {
                    name,
                    public_key,
                    public_key_file,
                } => admin::local_cli::add_key(&db, name, public_key, public_key_file).await,
                KeyCommand::List => admin::local_cli::list_keys(&db).await,
                KeyCommand::Deactivate { id } => {
                    admin::local_cli::set_key_active(&db, &id, false).await
                }
                KeyCommand::Activate { id } => {
                    admin::local_cli::set_key_active(&db, &id, true).await
                }
                KeyCommand::Access { command } => match command {
                    KeyAccessCommand::Show { id } => {
                        admin::local_cli::show_key_access(&db, &id).await
                    }
                    KeyAccessCommand::Set {
                        id,
                        mode,
                        asset_ids,
                    } => admin::local_cli::set_key_access(&db, &id, mode.into(), asset_ids).await,
                },
            }
        }
        Command::Credential { command } => {
            let (db, _, master_key) = open_runtime(cli.config).await?;
            match command {
                CredentialCommand::Add {
                    name,
                    username,
                    auth_type,
                    password,
                    password_stdin,
                    private_key_file,
                    passphrase,
                } => {
                    let password = read_stdin_secret_arg(password, password_stdin)?;
                    admin::local_cli::add_credential(
                        &db,
                        &master_key,
                        name,
                        username,
                        auth_type.into(),
                        password,
                        private_key_file,
                        passphrase,
                    )
                    .await
                }
                CredentialCommand::List => admin::local_cli::list_credentials(&db).await,
                CredentialCommand::Delete { id } => {
                    admin::local_cli::delete_credential(&db, &id).await
                }
            }
        }
        Command::Asset { command } => {
            let (db, _, _) = open_runtime(cli.config).await?;
            match command {
                AssetCommand::Add {
                    name,
                    protocol,
                    hostname,
                    port,
                    description,
                    tags,
                    credential_id,
                } => {
                    admin::local_cli::add_asset(
                        &db,
                        NewAsset {
                            name,
                            protocol: protocol.as_str().to_string(),
                            preset: None,
                            hostname,
                            port,
                            description,
                            tags,
                            credential_id,
                        },
                    )
                    .await
                }
                AssetCommand::List => admin::local_cli::list_assets(&db).await,
                AssetCommand::Delete { id } => admin::local_cli::delete_asset(&db, &id).await,
            }
        }
        Command::Export {
            kind,
            format,
            output,
        } => {
            let (db, _, _) = open_runtime(cli.config).await?;
            export_data(&db, kind.into(), format.into(), output).await
        }
        Command::Import {
            kind,
            file,
            format,
            on_conflict,
        } => {
            let (db, _, _) = open_runtime(cli.config).await?;
            import_data(
                &db,
                kind.into(),
                file,
                format.map(Into::into),
                on_conflict.into(),
            )
            .await
        }
    }
}

async fn export_data(
    db: &HopDb,
    kind: TransferKind,
    format: TransferFormat,
    output: Option<PathBuf>,
) -> Result<()> {
    let payload = match kind {
        TransferKind::Assets => {
            let assets = db.list_assets().await?;
            admin::transfer::export_assets(&assets, format)?
        }
        TransferKind::Credentials => {
            let credentials = db.list_credentials().await?;
            admin::transfer::export_credentials(&credentials, format)?
        }
    };

    if let Some(path) = output {
        fs::write(&path, payload).with_context(|| format!("write {}", path.display()))?;
        println!("exported {} to {}", export_kind_name(kind), path.display());
    } else {
        print!("{payload}");
    }
    Ok(())
}

async fn import_data(
    db: &HopDb,
    kind: TransferKind,
    file: PathBuf,
    format: Option<TransferFormat>,
    on_conflict: ConflictPolicy,
) -> Result<()> {
    let format = format
        .or_else(|| TransferFormat::from_path(&file))
        .context("cannot infer import format from file extension; pass --format")?;
    let payload = fs::read_to_string(&file).with_context(|| format!("read {}", file.display()))?;
    let summary = match kind {
        TransferKind::Assets => {
            admin::transfer::import_assets(db, &payload, format, on_conflict).await?
        }
        TransferKind::Credentials => {
            admin::transfer::import_credentials(db, &payload, format, on_conflict).await?
        }
    };
    println!(
        "imported={} skipped={} overwritten={} errors={}",
        summary.imported,
        summary.skipped,
        summary.overwritten,
        summary.errors.len()
    );
    let error_count = summary.errors.len();
    for error in &summary.errors {
        eprintln!("import error: {error}");
    }
    if error_count > 0 {
        bail!("import completed with {error_count} error(s)");
    }
    Ok(())
}

fn export_kind_name(kind: TransferKind) -> &'static str {
    match kind {
        TransferKind::Assets => "assets",
        TransferKind::Credentials => "credentials",
    }
}

fn read_stdin_secret_arg(value: Option<String>, read_stdin: bool) -> Result<Option<String>> {
    if !read_stdin {
        return Ok(value);
    }

    let mut raw = String::new();
    io::stdin()
        .read_to_string(&mut raw)
        .context("read --password-stdin")?;
    Ok(Some(normalize_stdin_secret(&raw)))
}

fn normalize_stdin_secret(value: &str) -> String {
    value.trim_end_matches(&['\r', '\n'][..]).to_string()
}

async fn serve(config_path: Option<PathBuf>) -> Result<()> {
    let (db, config, master_key) = open_runtime(config_path).await?;
    if let Some(password) = admin::bootstrap::ensure_admin_password(&db).await? {
        println!("Initial Hop admin password: {password}");
        println!("Open http://{} to finish setup.", config.server.admin_bind);
    }
    let ssh_bind = config.ssh_bind_addr()?;
    let admin_bind = config.admin_bind_addr()?;
    if let Some(message) = admin_bind_exposure_warning(&config)? {
        warn!("{message}");
    }
    let admin = admin::routes::serve_admin(admin_bind, ssh_bind, db.clone(), master_key.clone());
    let ssh = ssh::server::serve_ssh(ssh_bind, config, db, master_key);
    info!("starting hop-server");
    tokio::try_join!(admin, ssh)?;
    Ok(())
}

async fn open_runtime(config_path: Option<PathBuf>) -> Result<(HopDb, HopConfig, Arc<MasterKey>)> {
    let config = match config_path {
        Some(path) => HopConfig::load(Some(&path))
            .with_context(|| format!("load config {}", path.display()))?,
        None => HopConfig::load(None)?,
    };
    validate_admin_bind(&config)?;
    let db = HopDb::connect(&config.database.path).await?;
    let master_key = Arc::new(load_or_create_master_key(&config.security.secret_key_file)?);
    Ok((db, config, master_key))
}

fn validate_admin_bind(config: &HopConfig) -> Result<()> {
    config.admin_bind_addr()?;
    Ok(())
}

fn admin_bind_exposure_warning(config: &HopConfig) -> Result<Option<String>> {
    let admin_bind = config.admin_bind_addr()?;
    if admin_bind.ip().is_loopback() {
        return Ok(None);
    }
    Ok(Some(format!(
        "Admin Web is listening on {admin_bind}; protect it with a firewall, VPN, \
         host-local port mapping, or trusted management network"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admin_bind_accepts_loopback() {
        let mut config = HopConfig::default();
        assert!(validate_admin_bind(&config).is_ok());

        config.server.admin_bind = "[::1]:8080".to_string();
        assert!(validate_admin_bind(&config).is_ok());
    }

    #[test]
    fn admin_bind_allows_non_loopback_when_configured() {
        let mut config = HopConfig::default();
        config.server.admin_bind = "0.0.0.0:8080".to_string();
        assert!(validate_admin_bind(&config).is_ok());

        config.server.admin_bind = "[::]:8080".to_string();
        assert!(validate_admin_bind(&config).is_ok());

        config.server.admin_bind = "192.168.1.10:8080".to_string();
        assert!(validate_admin_bind(&config).is_ok());
    }

    #[test]
    fn non_loopback_admin_bind_gets_warning() {
        let mut config = HopConfig::default();
        assert!(admin_bind_exposure_warning(&config).unwrap().is_none());

        config.server.admin_bind = "0.0.0.0:8080".to_string();
        let warning = admin_bind_exposure_warning(&config).unwrap().unwrap();
        assert!(warning.contains("0.0.0.0:8080"));
        assert!(warning.contains("firewall"));
    }

    #[test]
    fn credential_add_accepts_password_stdin_flag() {
        let cli = Cli::try_parse_from([
            "hop-server",
            "credential",
            "add",
            "--name",
            "deploy",
            "--username",
            "deploy",
            "--auth-type",
            "password",
            "--password-stdin",
        ])
        .unwrap();

        let Some(Command::Credential {
            command:
                CredentialCommand::Add {
                    password,
                    password_stdin,
                    ..
                },
        }) = cli.command
        else {
            panic!("expected credential add");
        };

        assert!(password.is_none());
        assert!(password_stdin);
    }

    #[test]
    fn import_and_export_commands_parse_bulk_options() {
        let export_cli = Cli::try_parse_from([
            "hop-server",
            "export",
            "--format",
            "csv",
            "--output",
            "assets.csv",
        ])
        .unwrap();
        assert!(matches!(
            export_cli.command,
            Some(Command::Export {
                format: TransferFormatArg::Csv,
                output: Some(_),
                ..
            })
        ));

        let import_cli = Cli::try_parse_from([
            "hop-server",
            "import",
            "--file",
            "assets.csv",
            "--on-conflict",
            "skip",
        ])
        .unwrap();
        assert!(matches!(
            import_cli.command,
            Some(Command::Import {
                on_conflict: ConflictPolicyArg::Skip,
                ..
            })
        ));
    }

    #[test]
    fn asset_add_parses_rdp_protocol_option() {
        let cli = Cli::try_parse_from([
            "hop-server",
            "asset",
            "add",
            "--name",
            "win-rdp",
            "--protocol",
            "rdp",
            "--hostname",
            "10.0.2.20",
            "--port",
            "3389",
        ])
        .unwrap();

        let Some(Command::Asset {
            command: AssetCommand::Add { protocol, port, .. },
        }) = cli.command
        else {
            panic!("expected asset add");
        };

        assert_eq!(protocol.as_str(), ASSET_PRESET_RDP);
        assert_eq!(port, 3389);
    }

    #[test]
    fn key_access_commands_parse_repeated_asset_ids() {
        let cli = Cli::try_parse_from([
            "hop-server",
            "key",
            "access",
            "set",
            "key-1",
            "--mode",
            "restricted",
            "--asset-id",
            "asset-1",
            "--asset-id",
            "asset-2",
        ])
        .unwrap();

        let Some(Command::Key {
            command:
                KeyCommand::Access {
                    command:
                        KeyAccessCommand::Set {
                            mode, asset_ids, ..
                        },
                },
        }) = cli.command
        else {
            panic!("expected key access set");
        };
        assert!(matches!(mode, AssetAccessModeArg::Restricted));
        assert_eq!(asset_ids, vec!["asset-1", "asset-2"]);
    }

    #[tokio::test]
    async fn import_data_returns_error_when_summary_has_errors() {
        let db = HopDb::in_memory().await.unwrap();
        db.add_asset(hop_core::NewAsset::new("web", "10.0.0.1", 22))
            .await
            .unwrap();
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            file.path(),
            "name,hostname,port,description,tags,credential_id\nweb,10.0.0.2,22,,,\n",
        )
        .unwrap();

        let err = import_data(
            &db,
            TransferKind::Assets,
            file.path().to_path_buf(),
            Some(TransferFormat::Csv),
            ConflictPolicy::Error,
        )
        .await
        .unwrap_err();

        assert!(err.to_string().contains("import completed with 1 error"));
    }

    #[test]
    fn stdin_secret_strips_trailing_newlines_only() {
        assert_eq!(normalize_stdin_secret(" secret \r\n"), " secret ");
    }
}
