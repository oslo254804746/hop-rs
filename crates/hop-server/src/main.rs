mod control_api;
mod local_cli;
mod manifest_io;
mod ssh;
mod transfer;
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
    load_master_key, load_or_create_master_key, ApplyOptions, AssetAccessMode, AuthType, Catalog,
    CatalogError, HopConfig, MasterKey, NewAsset, ASSET_PROTOCOL_SSH, ASSET_PROTOCOL_TCP,
};
use serde::Serialize;
use tracing::{info, warn};

use crate::transfer::{ConflictPolicy, TransferFormat, TransferKind};

#[derive(Debug, Parser)]
#[command(
    name = "hop-server",
    version,
    about = "Hop lightweight SSH jump server"
)]
struct Cli {
    #[arg(long, global = true, env = "HOP_CONFIG")]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    Serve,
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    Apply {
        #[arg(short = 'f', long = "file", required = true)]
        files: Vec<PathBuf>,
        #[arg(long)]
        source: String,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        prune: bool,
        #[arg(long)]
        base_revision: Option<i64>,
        #[arg(long)]
        json: bool,
    },
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

#[derive(Debug, Subcommand)]
enum ConfigCommand {
    Validate {
        #[arg(long)]
        json: bool,
    },
    Diff {
        #[arg(short = 'f', long = "file", required = true)]
        files: Vec<PathBuf>,
        #[arg(long)]
        source: String,
        #[arg(long)]
        prune: bool,
        #[arg(long)]
        json: bool,
    },
    Status {
        #[arg(long)]
        json: bool,
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
    Tcp,
}

impl AssetProtocolArg {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Ssh => ASSET_PROTOCOL_SSH,
            Self::Tcp => ASSET_PROTOCOL_TCP,
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
    let cli = Cli::parse();
    let configured_log_level = cli
        .config
        .as_deref()
        .and_then(|path| HopConfig::load(Some(path)).ok())
        .map(|config| config.runtime.log_level)
        .unwrap_or_else(|| HopConfig::default().runtime.log_level);
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .or_else(|_| tracing_subscriber::EnvFilter::try_new(configured_log_level))
        .context("invalid runtime.log_level")?;
    tracing_subscriber::fmt().with_env_filter(env_filter).init();

    match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => serve(cli.config).await,
        Command::Config { command } => run_config_command(cli.config, command).await,
        Command::Apply {
            files,
            source,
            dry_run,
            prune,
            base_revision,
            json,
        } => {
            run_apply_command(
                cli.config,
                files,
                source,
                ApplyOptions {
                    base_revision,
                    prune,
                    dry_run,
                },
                json,
            )
            .await
        }
        Command::Key { command } => {
            let (db, _, _) = open_runtime(cli.config).await?;
            match command {
                KeyCommand::Add {
                    name,
                    public_key,
                    public_key_file,
                } => local_cli::add_key(&db, name, public_key, public_key_file).await,
                KeyCommand::List => local_cli::list_keys(&db).await,
                KeyCommand::Deactivate { id } => local_cli::set_key_active(&db, &id, false).await,
                KeyCommand::Activate { id } => local_cli::set_key_active(&db, &id, true).await,
                KeyCommand::Access { command } => match command {
                    KeyAccessCommand::Show { id } => local_cli::show_key_access(&db, &id).await,
                    KeyAccessCommand::Set {
                        id,
                        mode,
                        asset_ids,
                    } => local_cli::set_key_access(&db, &id, mode.into(), asset_ids).await,
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
                    local_cli::add_credential(
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
                CredentialCommand::List => local_cli::list_credentials(&db).await,
                CredentialCommand::Delete { id } => local_cli::delete_credential(&db, &id).await,
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
                    local_cli::add_asset(
                        &db,
                        NewAsset {
                            name,
                            protocol: protocol.as_str().to_string(),
                            hostname,
                            port,
                            description,
                            tags,
                            credential_id,
                        },
                    )
                    .await
                }
                AssetCommand::List => local_cli::list_assets(&db).await,
                AssetCommand::Delete { id } => local_cli::delete_asset(&db, &id).await,
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

async fn run_config_command(config_path: Option<PathBuf>, command: ConfigCommand) -> Result<()> {
    match command {
        ConfigCommand::Validate { json } => {
            load_config(config_path)?;
            let output = ValidationOutput { valid: true };
            if json {
                println!("{}", serde_json::to_string(&output)?);
            } else {
                println!("startup config valid");
            }
            Ok(())
        }
        ConfigCommand::Diff {
            files,
            source,
            prune,
            json,
        } => {
            let manifest = match manifest_io::load_manifest_scope(&files) {
                Ok(manifest) => manifest,
                Err(error) => {
                    return finish_catalog_result::<hop_core::ApplySummary>(Err(error), json)
                }
            };
            let (catalog, master_key) = open_catalog_read_only(config_path).await?;
            let result = catalog.diff(&manifest, &source, &master_key, prune).await;
            finish_catalog_result(result, json)
        }
        ConfigCommand::Status { json } => {
            let catalog = open_catalog_db_read_only(config_path).await?;
            finish_catalog_result(catalog.status().await, json)
        }
    }
}

async fn run_apply_command(
    config_path: Option<PathBuf>,
    files: Vec<PathBuf>,
    source: String,
    options: ApplyOptions,
    json: bool,
) -> Result<()> {
    let manifest = match manifest_io::load_manifest_scope(&files) {
        Ok(manifest) => manifest,
        Err(error) => return finish_catalog_result::<hop_core::ApplySummary>(Err(error), json),
    };
    let (catalog, master_key) = if options.dry_run {
        open_catalog_read_only(config_path).await?
    } else {
        open_catalog(config_path).await?
    };
    let result = catalog
        .apply(&manifest, &source, &master_key, options)
        .await;
    if !options.dry_run {
        if let Err(error) = &result {
            let _ = catalog.record_apply_failure(&source, "cli", error).await;
        }
    }
    finish_catalog_result(result, json)
}

#[derive(Debug, Serialize)]
struct ValidationOutput {
    valid: bool,
}

fn finish_catalog_result<T>(result: std::result::Result<T, CatalogError>, json: bool) -> Result<()>
where
    T: Serialize + std::fmt::Debug,
{
    match result {
        Ok(output) => {
            if json {
                println!("{}", serde_json::to_string(&output)?);
            } else {
                print_human_catalog_output(&output)?;
            }
            Ok(())
        }
        Err(error) => {
            if json {
                eprintln!("{}", serde_json::to_string(&error)?);
            }
            Err(error.into())
        }
    }
}

fn print_human_catalog_output<T>(output: &T) -> Result<()>
where
    T: Serialize + std::fmt::Debug,
{
    let value = serde_json::to_value(output)?;
    if let Some(valid) = value.get("valid").and_then(serde_json::Value::as_bool) {
        println!("manifest {}", if valid { "valid" } else { "invalid" });
    } else if let (Some(base), Some(next)) = (
        value
            .get("base_revision")
            .and_then(serde_json::Value::as_i64),
        value
            .get("new_revision")
            .and_then(serde_json::Value::as_i64),
    ) {
        println!("catalog revision {base} -> {next}");
        for field in ["created", "updated", "deleted", "orphaned", "unchanged"] {
            let count = value
                .get(field)
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            println!("{field}={count}");
        }
    } else {
        println!("{}", serde_json::to_string_pretty(&value)?);
    }
    Ok(())
}

async fn open_catalog(config_path: Option<PathBuf>) -> Result<(Catalog, Arc<MasterKey>)> {
    let config = load_config(config_path)?;
    fs::create_dir_all(&config.data_dir)
        .with_context(|| format!("create data directory {}", config.data_dir.display()))?;
    let catalog = Catalog::connect(config.database_path()).await?;
    let master_key = Arc::new(load_or_create_master_key(&config.master_key_path())?);
    Ok((catalog, master_key))
}

async fn open_catalog_db_read_only(config_path: Option<PathBuf>) -> Result<Catalog> {
    let config = load_config(config_path)?;
    Catalog::connect_read_only(config.database_path())
        .await
        .map_err(Into::into)
}

async fn open_catalog_read_only(config_path: Option<PathBuf>) -> Result<(Catalog, Arc<MasterKey>)> {
    let config = load_config(config_path)?;
    let catalog = Catalog::connect_read_only(config.database_path()).await?;
    let master_key = Arc::new(load_master_key(&config.master_key_path())?);
    Ok((catalog, master_key))
}

fn load_config(config_path: Option<PathBuf>) -> Result<HopConfig> {
    let config = match config_path {
        Some(path) => HopConfig::load(Some(&path))
            .with_context(|| format!("load config {}", path.display()))?,
        None => HopConfig::load(None)?,
    };
    Ok(config)
}

async fn export_data(
    db: &Catalog,
    kind: TransferKind,
    format: TransferFormat,
    output: Option<PathBuf>,
) -> Result<()> {
    let payload = match kind {
        TransferKind::Assets => {
            let assets = db.list_assets().await?;
            transfer::export_assets(&assets, format)?
        }
        TransferKind::Credentials => {
            let credentials = db.list_credentials().await?;
            transfer::export_credentials(&credentials, format)?
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
    db: &Catalog,
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
        TransferKind::Assets => transfer::import_assets(db, &payload, format, on_conflict).await?,
        TransferKind::Credentials => {
            transfer::import_credentials(db, &payload, format, on_conflict).await?
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
    let ssh_bind = config.ssh_bind_addr()?;
    if config.api.enabled {
        config.api_bind_addr()?;
    }
    fs::create_dir_all(&config.runtime.temp_dir).with_context(|| {
        format!(
            "create runtime temp directory {}",
            config.runtime.temp_dir.display()
        )
    })?;
    let pruned = db
        .prune_finished_sessions(config.runtime.session_retention_days)
        .await
        .context("apply runtime session retention")?;
    if pruned > 0 {
        info!(pruned, "removed expired finished sessions");
    } else if config.runtime.session_retention_days == 0 {
        warn!("automatic finished-session retention is disabled");
    }
    let summary = apply_startup_resources(&db, &master_key, &config).await?;
    if summary.new_revision != summary.base_revision {
        info!(
            created = summary.created,
            updated = summary.updated,
            deleted = summary.deleted,
            revision = summary.new_revision,
            "applied startup-config resources"
        );
    }
    let active_sessions = ssh::session_registry::ActiveSessionRegistry::default();
    info!("starting hop-server SSH listener on {ssh_bind}");
    let api_token = config
        .api
        .enabled
        .then(|| config.api.token.as_ref().filter(|token| !token.is_empty()))
        .flatten();
    if config.api.enabled && api_token.is_none() {
        warn!("Control API disabled because api.token is missing or empty; SSH remains available");
    }
    if config.api.enabled && api_token.is_some_and(|token| token.is_placeholder()) {
        warn!("api.token still uses the insecure 'change-me' placeholder; replace it before exposing the panel");
    }
    if let Some(token) = api_token {
        let api_bind = config.api_bind_addr()?;
        let api_state = control_api::ControlApiState::new(
            db.clone(),
            master_key.clone(),
            token.expose(),
            active_sessions.clone(),
        )?;
        info!("starting Hop Control API on {api_bind}");
        let cors_allowlist = config.api.cors_allowlist.clone();
        let api = control_api::serve(api_bind, api_state, &cors_allowlist);
        let ssh = ssh::server::serve_ssh(ssh_bind, config, db, master_key, active_sessions);
        tokio::try_join!(api, ssh)?;
        Ok(())
    } else {
        info!("Control API disabled; no HTTP listener will be created");
        let ssh = ssh::server::serve_ssh(ssh_bind, config, db, master_key, active_sessions);
        ssh.await?;
        Ok(())
    }
}

async fn apply_startup_resources(
    catalog: &Catalog,
    master_key: &MasterKey,
    config: &HopConfig,
) -> Result<hop_core::ApplySummary> {
    catalog
        .apply(
            &config.startup_manifest(),
            "startup-config",
            master_key,
            ApplyOptions {
                prune: true,
                ..ApplyOptions::default()
            },
        )
        .await
        .map_err(|error| anyhow::anyhow!("startup config apply failed: {error}"))
}

async fn open_runtime(
    config_path: Option<PathBuf>,
) -> Result<(Catalog, HopConfig, Arc<MasterKey>)> {
    let config = match config_path {
        Some(path) => HopConfig::load(Some(&path))
            .with_context(|| format!("load config {}", path.display()))?,
        None => HopConfig::load(None)?,
    };
    fs::create_dir_all(&config.data_dir)
        .with_context(|| format!("create data directory {}", config.data_dir.display()))?;
    let db = Catalog::connect(config.database_path()).await?;
    let master_key = Arc::new(load_or_create_master_key(&config.master_key_path())?);
    Ok((db, config, master_key))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PUBLIC_KEY: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJdD7y3aLq454yWBdwLWbieU1ebz9/cu7/QEXn9OIeZJ test";

    #[test]
    fn config_argument_honors_hop_config_environment_variable() {
        use clap::CommandFactory;

        let command = Cli::command();
        let config = command
            .get_arguments()
            .find(|argument| argument.get_id() == "config")
            .unwrap();
        assert_eq!(config.get_env(), Some(std::ffi::OsStr::new("HOP_CONFIG")));
    }

    #[test]
    fn config_and_apply_commands_parse_v0_2_contract() {
        let validate = Cli::try_parse_from([
            "hop-server",
            "--config",
            "hop.yaml",
            "config",
            "validate",
            "--json",
        ])
        .unwrap();
        assert!(matches!(
            validate.command,
            Some(Command::Config {
                command: ConfigCommand::Validate { json: true }
            })
        ));

        let apply = Cli::try_parse_from([
            "hop-server",
            "apply",
            "-f",
            "resources.toml",
            "--source",
            "home",
            "--dry-run",
            "--prune",
            "--base-revision",
            "4",
            "--json",
        ])
        .unwrap();
        assert!(matches!(
            apply.command,
            Some(Command::Apply {
                dry_run: true,
                prune: true,
                base_revision: Some(4),
                json: true,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn apply_command_initializes_and_updates_the_v0_2_catalog() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("hop.db");
        let password = directory.path().join("password");
        let public_key = directory.path().join("id.pub");
        let manifest = directory.path().join("resources.yaml");
        let config = directory.path().join("config.toml");
        fs::write(&password, "test-password").unwrap();
        fs::write(&public_key, PUBLIC_KEY).unwrap();
        fs::write(
            &manifest,
            format!(
                r#"api_version: hop/v1alpha1
credentials:
  root:
    type: password
    username: root
    password:
      file: {}
assets:
  server:
    type: ssh
    host: 192.0.2.10
    port: 22
    credential: root
access:
  laptop:
    public_key:
      file: {}
"#,
                password.display(),
                public_key.display()
            ),
        )
        .unwrap();
        fs::write(&config, format!("data_dir = {:?}\n", directory.path())).unwrap();

        run_apply_command(
            Some(config),
            vec![manifest],
            "home".to_string(),
            ApplyOptions::default(),
            false,
        )
        .await
        .unwrap();

        let catalog = Catalog::connect(&database).await.unwrap();
        assert_eq!(catalog.revision().await.unwrap(), 1);
        assert_eq!(catalog.list_assets().await.unwrap().len(), 1);
        assert_eq!(catalog.list_authorized_keys().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn disabled_api_creates_no_http_listener_and_enabled_api_binds() {
        let directory = tempfile::tempdir().unwrap();
        let occupied = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api_address = occupied.local_addr().unwrap();
        let config_path = directory.path().join("config.toml");
        let write_config = |enabled: bool, token: Option<&str>| {
            let token = token
                .map(|token| format!("token = {token:?}\n"))
                .unwrap_or_default();
            fs::write(
                &config_path,
                format!(
                    "listen = '127.0.0.1:0'\ndata_dir = {:?}\n\n[api]\nenabled = {enabled}\nlisten = '{api_address}'\n{token}",
                    directory.path(),
                ),
            )
            .unwrap();
        };

        write_config(false, None);
        let disabled = tokio::time::timeout(
            std::time::Duration::from_millis(150),
            serve(Some(config_path.clone())),
        )
        .await;
        assert!(
            disabled.is_err(),
            "disabled API must not try the occupied HTTP port"
        );

        write_config(false, Some("configured-but-disabled"));
        let disabled_with_token = tokio::time::timeout(
            std::time::Duration::from_millis(150),
            serve(Some(config_path.clone())),
        )
        .await;
        assert!(
            disabled_with_token.is_err(),
            "disabled API must ignore a configured token and leave SSH running"
        );

        write_config(true, None);
        let missing_token = tokio::time::timeout(
            std::time::Duration::from_millis(150),
            serve(Some(config_path.clone())),
        )
        .await;
        assert!(
            missing_token.is_err(),
            "missing API token must leave SSH running without binding HTTP"
        );

        write_config(true, Some(""));
        let empty_token = tokio::time::timeout(
            std::time::Duration::from_millis(150),
            serve(Some(config_path.clone())),
        )
        .await;
        assert!(
            empty_token.is_err(),
            "empty API token must leave SSH running without binding HTTP"
        );

        write_config(true, Some("management-token"));
        let enabled =
            tokio::time::timeout(std::time::Duration::from_secs(2), serve(Some(config_path)))
                .await
                .expect("enabled API bind failure should return promptly");
        assert!(
            enabled.is_err(),
            "enabled API must try the occupied HTTP port"
        );
    }

    #[tokio::test]
    async fn startup_config_apply_is_atomic_idempotent_and_prunes_only_its_resources() {
        let directory = tempfile::tempdir().unwrap();
        let config_path = directory.path().join("hop.yaml");
        let write_config = |password: Option<&str>, host: &str| {
            let resources = password
                .map(|password| {
                    format!(
                        r#"credentials:
  root:
    username: root
    password: "{password}"
assets:
  nas:
    host: {host}
    credential: root
access_keys:
  laptop:
    public_key: "{PUBLIC_KEY}"
    assets: [nas]
"#
                    )
                })
                .unwrap_or_default();
            fs::write(
                &config_path,
                format!(
                    "listen: 127.0.0.1:0\ndata_dir: {}\n{resources}",
                    directory.path().display()
                ),
            )
            .unwrap();
        };

        write_config(Some("first-password"), "192.0.2.10");
        let config = HopConfig::load(Some(&config_path)).unwrap();
        let catalog = Catalog::connect(config.database_path()).await.unwrap();
        let master_key = load_or_create_master_key(&config.master_key_path()).unwrap();
        let first = apply_startup_resources(&catalog, &master_key, &config)
            .await
            .unwrap();
        assert_eq!((first.created, first.new_revision), (3, 1));
        assert_eq!(
            catalog
                .resource_ownership("asset", &catalog.list_assets().await.unwrap()[0].id)
                .await
                .unwrap(),
            hop_core::ResourceOwnership::Config
        );

        let same = apply_startup_resources(&catalog, &master_key, &config)
            .await
            .unwrap();
        assert_eq!(same.new_revision, 1);

        write_config(Some("second-password"), "192.0.2.11");
        let changed = HopConfig::load(Some(&config_path)).unwrap();
        let changed = apply_startup_resources(&catalog, &master_key, &changed)
            .await
            .unwrap();
        assert_eq!((changed.updated, changed.new_revision), (2, 2));

        let local = catalog
            .add_asset(NewAsset::new("local-console", "192.0.2.30", 22))
            .await
            .unwrap();
        write_config(None, "unused");
        let empty = HopConfig::load(Some(&config_path)).unwrap();
        let pruned = apply_startup_resources(&catalog, &master_key, &empty)
            .await
            .unwrap();
        assert_eq!(pruned.deleted, 3);
        assert_eq!(
            catalog
                .list_assets()
                .await
                .unwrap()
                .into_iter()
                .map(|asset| asset.name)
                .collect::<Vec<_>>(),
            vec!["local-console"]
        );
        assert_eq!(
            catalog
                .resource_ownership("asset", &local.id)
                .await
                .unwrap(),
            hop_core::ResourceOwnership::Local
        );
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
    fn asset_add_accepts_only_ssh_and_tcp_protocol_options() {
        let cli = Cli::try_parse_from([
            "hop-server",
            "asset",
            "add",
            "--name",
            "desktop",
            "--protocol",
            "tcp",
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

        assert_eq!(protocol.as_str(), ASSET_PROTOCOL_TCP);
        assert_eq!(port, 3389);

        assert!(Cli::try_parse_from([
            "hop-server",
            "asset",
            "add",
            "--name",
            "desktop",
            "--protocol",
            "rdp",
            "--hostname",
            "10.0.2.20",
            "--port",
            "3389",
        ])
        .is_err());
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
        let db = Catalog::in_memory().await.unwrap();
        db.add_asset(hop_core::NewAsset::new("web", "10.0.0.1", 22))
            .await
            .unwrap();
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            file.path(),
            "name,hostname,port,description,tags,credential_id,protocol\nweb,10.0.0.2,22,,,,ssh\n",
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
