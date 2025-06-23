mod commands;
mod config;

use arkive_core::{ArkiveError, WalletManager};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser)]
#[command(name = "arkive")]
#[command(about = "ARKive SDK - Bitcoin and Ark protocol wallet")]
#[command(version)]
struct Cli {
    /// Data directory for wallet storage
    #[arg(short, long, global = true)]
    data_dir: Option<PathBuf>,

    /// Verbose output
    #[arg(short, long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Wallet management commands
    #[command(subcommand)]
    Wallet(commands::WalletCommands),

    /// Transaction commands
    #[command(subcommand)]
    Transaction(commands::TransactionCommands),

    /// Balance and address commands
    #[command(subcommand)]
    Balance(commands::BalanceCommands),

    /// Ark-specific commands
    #[command(subcommand)]
    Ark(commands::ArkCommands),

    /// Backup and restore commands
    #[command(subcommand)]
    Backup(commands::BackupCommands),

    /// Multi-device sync commands
    #[command(subcommand)]
    Sync(commands::SyncCommands),
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    // Initialize logging
    let log_level = if cli.verbose { "debug" } else { "info" };
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(format!(
            "arkive={}",
            log_level
        )))
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Get data directory
    let data_dir = cli.data_dir.unwrap_or_else(|| {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("arkive")
    });

    // Ensure data directory exists
    tokio::fs::create_dir_all(&data_dir).await?;

    // Initialize wallet manager
    let manager = WalletManager::new(&data_dir).await?;

    // Execute command
    let result = match cli.command {
        Commands::Wallet(cmd) => commands::handle_wallet_command(cmd, &manager).await,
        Commands::Transaction(cmd) => commands::handle_transaction_command(cmd, &manager).await,
        Commands::Balance(cmd) => commands::handle_balance_command(cmd, &manager).await,
        Commands::Ark(cmd) => commands::handle_ark_command(cmd, &manager).await,
        Commands::Backup(cmd) => commands::handle_backup_command(cmd, &manager).await,
        Commands::Sync(cmd) => commands::handle_sync_command(cmd, &manager).await,
    };

    if let Err(e) = result {
        match e {
            ArkiveError::WalletNotFound { name } => {
                eprintln!("Error: Wallet '{}' not found", name);
                eprintln!("Use 'arkive wallet list' to see available wallets");
            }
            ArkiveError::InsufficientFunds { need, available } => {
                eprintln!("Error: Insufficient funds");
                eprintln!("Need: {} sats, Available: {} sats", need, available);
            }
            ArkiveError::InvalidAddress(addr) => {
                eprintln!("Error: Invalid address: {}", addr);
            }
            _ => {
                eprintln!("Error: {}", e);
            }
        }
        std::process::exit(1);
    }

    Ok(())
}
