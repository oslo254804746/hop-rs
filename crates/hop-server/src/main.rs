mod admin;
mod ssh;
mod tui;

use std::{path::PathBuf, sync::Arc};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use hop_core::{load_or_create_master_key, AuthType, HopConfig, HopDb, MasterKey};
use tracing::info;

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

#[derive(Debug, Subcommand)]
enum AssetCommand {
    Add {
        #[arg(long)]
        name: String,
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
                    private_key_file,
                    passphrase,
                } => {
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
                    hostname,
                    port,
                    description,
                    tags,
                    credential_id,
                } => {
                    admin::local_cli::add_asset(
                        &db,
                        name,
                        hostname,
                        port,
                        description,
                        tags,
                        credential_id,
                    )
                    .await
                }
                AssetCommand::List => admin::local_cli::list_assets(&db).await,
                AssetCommand::Delete { id } => admin::local_cli::delete_asset(&db, &id).await,
            }
        }
    }
}

async fn serve(config_path: Option<PathBuf>) -> Result<()> {
    let (db, config, master_key) = open_runtime(config_path).await?;
    if let Some(password) = admin::bootstrap::ensure_admin_password(&db).await? {
        println!("Initial Hop admin password: {password}");
        println!("Open http://{} to finish setup.", config.server.admin_bind);
    }
    let ssh_bind = config.ssh_bind_addr()?;
    let admin_bind = config.admin_bind_addr()?;
    let admin = admin::routes::serve_admin(admin_bind, db.clone(), master_key.clone());
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
    let admin_bind = config.admin_bind_addr()?;
    if !admin_bind.ip().is_loopback() {
        bail!("admin_bind must use a loopback address for MVP Admin Web: {admin_bind}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admin_bind_must_be_loopback() {
        let mut config = HopConfig::default();
        assert!(validate_admin_bind(&config).is_ok());

        config.server.admin_bind = "[::1]:8080".to_string();
        assert!(validate_admin_bind(&config).is_ok());

        config.server.admin_bind = "0.0.0.0:8080".to_string();
        assert!(validate_admin_bind(&config).is_err());

        config.server.admin_bind = "[::]:8080".to_string();
        assert!(validate_admin_bind(&config).is_err());

        config.server.admin_bind = "192.168.1.10:8080".to_string();
        assert!(validate_admin_bind(&config).is_err());
    }
}
