use arkive_core::{Result, WalletManager};
use clap::Subcommand;
use comfy_table::{presets::UTF8_FULL, Table};

#[derive(Subcommand)]
pub enum ArkCommands {
    /// List VTXOs
    Vtxos {
        /// Wallet name
        wallet: String,
    },
    /// Participate in a round
    Round {
        /// Wallet name
        wallet: String,
    },
    /// Sync wallet with Ark server
    Sync {
        /// Wallet name
        wallet: String,
    },
}

pub async fn handle_ark_command(cmd: ArkCommands, manager: &WalletManager) -> Result<()> {
    match cmd {
        ArkCommands::Vtxos { wallet } => {
            let wallet = manager.load_wallet(&wallet).await?;

            println!("VTXOs for wallet '{}':", wallet.name());

            let vtxos = wallet.list_vtxos().await?;

            if vtxos.is_empty() {
                println!("No VTXOs found.");
                return Ok(());
            }

            let mut table = Table::new();
            table.load_preset(UTF8_FULL);
            table.set_header(vec![
                "Outpoint",
                "Amount (sats)",
                "Status",
                "Expiry",
                "Address",
            ]);

            for vtxo in vtxos {
                table.add_row(vec![
                    &format!("{}...", &vtxo.outpoint[..16]),
                    &vtxo.amount.to_sat().to_string(),
                    &format!("{:?}", vtxo.status),
                    &vtxo.expiry.format("%Y-%m-%d %H:%M").to_string(),
                    &format!("{}...", &vtxo.address[..20]),
                ]);
            }

            println!("{}", table);
        }

        ArkCommands::Round { wallet } => {
            let wallet = manager.load_wallet(&wallet).await?;

            println!("Participating in round for wallet '{}'...", wallet.name());

            match wallet.participate_in_round().await {
                Ok(Some(round_txid)) => {
                    println!("Successfully participated in round!");
                    println!("Round transaction ID: {}", round_txid);
                }
                Ok(None) => {
                    println!("No round participation needed at this time.");
                }
                Err(e) => {
                    println!("Failed to participate in round: {}", e);
                    return Err(e);
                }
            }
        }

        ArkCommands::Sync { wallet } => {
            let wallet = manager.load_wallet(&wallet).await?;

            println!("Syncing wallet '{}'...", wallet.name());

            match wallet.sync().await {
                Ok(_) => {
                    println!("Wallet synced successfully!");

                    // show updated balance
                    if let Ok(balance) = wallet.balance().await {
                        println!("Updated balance:");
                        println!("  Confirmed: {} sats", balance.confirmed.to_sat());
                        println!("  Pending: {} sats", balance.pending.to_sat());
                        println!("  Total: {} sats", balance.total.to_sat());
                    }
                }
                Err(e) => {
                    println!("Sync failed: {}", e);
                    return Err(e);
                }
            }
        }
    }

    Ok(())
}
