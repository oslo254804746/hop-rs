mod config;
mod ssh_exec;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "hop", version, about = "Local Hop SSH wrapper")]
struct Cli {
    #[arg(long, env = "HOP_HOST", default_value = "hop")]
    host: String,

    #[arg(long, env = "HOP_PORT", default_value_t = 2222)]
    port: u16,

    #[arg(long, env = "HOP_USER", default_value = "hop")]
    user: String,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    Ls,
    Connect { asset: String },
    Version,
    SshConfig,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let target = config::SshTarget::new(cli.user, cli.host, cli.port);
    match cli.command {
        None => ssh_exec::interactive_shell(&target),
        Some(Command::Ls) => ssh_exec::exec(&target, "hop-list-assets", false),
        Some(Command::Connect { asset }) => {
            ssh_exec::exec(&target, &format!("hop-connect {asset}"), true)
        }
        Some(Command::Version) => ssh_exec::exec(&target, "hop-version", false),
        Some(Command::SshConfig) => {
            print!("{}", target.ssh_config());
            Ok(())
        }
    }
}
