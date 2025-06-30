mod commands;

use arkive_core::WalletManager;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser)]
#[command(name = "coinflip")]
#[command(about = "Zero-Collateral Lottery CLI 2-player Betting")]
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
    /// Create a new 2-player lottery game
    Create {
        /// Wallet name to use
        wallet: String,
        /// Bet amount in satoshis
        amount: u64,
    },
    /// Join an existing game
    Join {
        /// Wallet name to use
        wallet: String,
        /// Game ID to join
        game_id: String,
    },
    /// Place bet in the game
    Bet {
        /// Wallet name
        wallet: String,
        /// Game ID
        game_id: String,
    },
    /// Commit to a game
    Commit {
        /// Wallet name
        wallet: String,
        /// Game ID
        game_id: String,
    },
    /// Reveal commitment
    Reveal {
        /// Wallet name
        wallet: String,
        /// Game ID
        game_id: String,
        /// Secret (hex encoded)
        secret: String,
    },
    /// Show game status
    Status {
        /// Game ID
        game_id: String,
    },
    /// List active games
    List,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    // Initialize logging
    let log_level = if cli.verbose { "debug" } else { "info" };
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(format!(
            "coinflip={},arkive_lottery={}",
            log_level, log_level
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
    let wallet_manager = WalletManager::new(&data_dir).await?;

    // Execute command
    let result = match cli.command {
        Commands::Create { wallet, amount } => {
            commands::create_game(&wallet_manager, &wallet, amount).await
        }
        Commands::Join { wallet, game_id } => {
            commands::join_game(&wallet_manager, &wallet, &game_id).await
        }
        Commands::Bet { wallet, game_id } => {
            commands::place_bet(&wallet_manager, &wallet, &game_id).await
        }
        Commands::Commit { wallet, game_id } => {
            commands::commit_to_game(&wallet_manager, &wallet, &game_id).await
        }
        Commands::Reveal {
            wallet,
            game_id,
            secret,
        } => commands::reveal_commitment(&wallet_manager, &wallet, &game_id, &secret).await,
        Commands::Status { game_id } => commands::show_game_status(&game_id).await,
        Commands::List => commands::list_games().await,
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }

    Ok(())
}
