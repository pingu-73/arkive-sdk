use arkive_core::{ArkiveError, Result, WalletManager};
use bitcoin::Network;
use clap::Subcommand;
use comfy_table::{presets::UTF8_FULL, Table};
use dialoguer::{Confirm, Password};

#[derive(Subcommand)]
pub enum WalletCommands {
    /// Create a new wallet
    Create {
        /// Wallet name
        name: String,
        /// Bitcoin network (regtest, signet, mutinynet)
        #[arg(short, long, default_value = "regtest")]
        network: String,
    },
    /// Import a wallet from mnemonic
    Import {
        /// Wallet name
        name: String,
        /// Bitcoin network (regtest, signet, mutinynet)
        #[arg(short, long, default_value = "regtest")]
        network: String,
        /// Mnemonic phrase (will prompt if not provided)
        #[arg(short, long)]
        mnemonic: Option<String>,
    },
    /// List all wallets
    List,
    /// Show wallet information
    Info {
        /// Wallet name
        name: String,
    },
    /// Delete a wallet
    Delete {
        /// Wallet name
        name: String,
        /// Skip confirmation prompt
        #[arg(short, long)]
        force: bool,
    },
}

pub async fn handle_wallet_command(cmd: WalletCommands, manager: &WalletManager) -> Result<()> {
    match cmd {
        WalletCommands::Create { name, network } => {
            let (network, is_mutinynet) = parse_network(&network)?;

            println!("Creating wallet '{}'...", name);
            let (wallet, mnemonic) = if is_mutinynet {
                manager.create_wallet_mutinynet(&name).await?
            } else {
                manager.create_wallet(&name, network).await?
            };

            println!("Wallet created successfully!");
            println!();
            println!("IMPORTANT: Save your mnemonic phrase securely!");
            println!("Mnemonic: {}", mnemonic);
            println!();
            println!("Wallet Details:");
            println!("  Name: {}", wallet.name());
            println!("  ID: {}", wallet.id());
            println!("  Network: {:?}", wallet.network_display());

            // Get addresses
            if let Ok(onchain_addr) = wallet.get_onchain_address().await {
                println!(" On-chain Address: {}", onchain_addr.address);
            }
            if let Ok(ark_addr) = wallet.get_ark_address().await {
                println!(" Ark Address: {}", ark_addr.address);
            }
        }

        WalletCommands::Import {
            name,
            network,
            mnemonic,
        } => {
            let (network, is_mutinynet) = parse_network(&network)?;

            let mnemonic = if let Some(m) = mnemonic {
                m
            } else {
                Password::new()
                    .with_prompt("Enter mnemonic phrase")
                    .interact()
                    .map_err(|e| ArkiveError::dialog(e.to_string()))?
            };

            println!("Importing wallet '{}'...", name);
            let wallet = if is_mutinynet {
                manager.import_wallet_mutinynet(&name, &mnemonic).await?
            } else {
                manager.import_wallet(&name, &mnemonic, network).await?
            };

            println!("Wallet imported successfully!");
            println!("  Name: {}", wallet.name());
            println!("  ID: {}", wallet.id());
            println!("  Network: {:?}", wallet.network_display());
        }

        WalletCommands::List => {
            let wallets = manager.list_wallets().await?;

            if wallets.is_empty() {
                println!("No wallets found.");
                println!("Create a new wallet with: arkive wallet create <name>");
                return Ok(());
            }

            let mut table = Table::new();
            table.load_preset(UTF8_FULL);
            table.set_header(vec!["Name", "Network", "Status"]);

            for wallet_name in wallets {
                match manager.load_wallet(&wallet_name).await {
                    Ok(wallet) => {
                        table.add_row(vec![wallet.name(), &wallet.network_display(), "Available"]);
                    }
                    Err(_) => {
                        table.add_row(vec![&wallet_name, "Unknown", "Error"]);
                    }
                }
            }

            println!("{}", table);
        }

        WalletCommands::Info { name } => {
            let wallet = manager.load_wallet(&name).await?;

            println!("Wallet Information:");
            println!("  Name: {}", wallet.name());
            println!("  ID: {}", wallet.id());
            println!("  Network: {:?}", wallet.network_display());
            println!();

            // Get addresses
            println!("Addresses:");
            if let Ok(onchain_addr) = wallet.get_onchain_address().await {
                println!("  On-chain: {}", onchain_addr.address);
            }
            if let Ok(ark_addr) = wallet.get_ark_address().await {
                println!("  Ark: {}", ark_addr.address);
            }
            if let Ok(boarding_addr) = wallet.get_boarding_address().await {
                println!("  Boarding: {}", boarding_addr.address);
            }

            // Get balance
            println!();
            if let Ok(balance) = wallet.balance().await {
                println!("Balance:");
                println!("  Confirmed: {} sats", balance.confirmed.to_sat());
                println!("  Pending: {} sats", balance.pending.to_sat());
                println!("  Total: {} sats", balance.total.to_sat());
            }
        }

        WalletCommands::Delete { name, force } => {
            if !force {
                let confirm = Confirm::new()
                    .with_prompt(format!("Are you sure you want to delete wallet '{}'? This action cannot be undone.", name))
                    .default(false)
                    .interact()
                    .map_err(|e| ArkiveError::dialog(e.to_string()))?;

                if !confirm {
                    println!("Deletion cancelled.");
                    return Ok(());
                }
            }

            manager.delete_wallet(&name).await?;
            println!("Wallet '{}' deleted successfully.", name);
        }
    }

    Ok(())
}

fn parse_network(network: &str) -> Result<(Network, bool)> {
    match network.to_lowercase().as_str() {
        "signet" => Ok((Network::Signet, false)),
        "mutinynet" => Ok((Network::Signet, true)),
        "regtest" => Ok((Network::Regtest, false)),
        _ => Err(ArkiveError::config(format!(
            "Invalid network: {}. Supported networks: signet, mutinynet, regtest",
            network
        ))),
    }
}
