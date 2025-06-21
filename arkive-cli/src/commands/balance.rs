use arkive_core::{Result, WalletManager};
use clap::Subcommand;
use comfy_table::{presets::UTF8_FULL, Table};

#[derive(Subcommand)]
pub enum BalanceCommands {
    /// Show wallet balance
    Show {
        /// Wallet name
        wallet: String,
    },
    /// Show detailed balance breakdown
    Detail {
        /// Wallet name
        wallet: String,
    },
    /// Show addresses for receiving funds
    Address {
        /// Wallet name
        wallet: String,
        /// Address type (onchain, ark, boarding)
        #[arg(short, long)]
        address_type: Option<String>,
    },
}

pub async fn handle_balance_command(cmd: BalanceCommands, manager: &WalletManager) -> Result<()> {
    match cmd {
        BalanceCommands::Show { wallet } => {
            let wallet = manager.load_wallet(&wallet).await?;

            println!("Balance for wallet '{}':", wallet.name());

            let balance = wallet.balance().await?;
            println!(
                "  Confirmed: {} sats ({:.8} BTC)",
                balance.confirmed.to_sat(),
                balance.confirmed.to_btc()
            );
            println!(
                "  Pending: {} sats ({:.8} BTC)",
                balance.pending.to_sat(),
                balance.pending.to_btc()
            );
            println!(
                "  Total: {} sats ({:.8} BTC)",
                balance.total.to_sat(),
                balance.total.to_btc()
            );
        }

        BalanceCommands::Detail { wallet } => {
            let wallet = manager.load_wallet(&wallet).await?;

            println!("Detailed balance for wallet '{}':", wallet.name());
            println!();

            // On-chain balance
            if let Ok(onchain_balance) = wallet.onchain_balance().await {
                println!("On-chain Balance:");
                println!(
                    "  Amount: {} sats ({:.8} BTC)",
                    onchain_balance.to_sat(),
                    onchain_balance.to_btc()
                );
                println!();
            }

            // Ark balance
            if let Ok((confirmed, pending)) = wallet.ark_balance().await {
                println!("Ark Balance:");
                println!(
                    "  Confirmed: {} sats ({:.8} BTC)",
                    confirmed.to_sat(),
                    confirmed.to_btc()
                );
                println!(
                    "  Pending: {} sats ({:.8} BTC)",
                    pending.to_sat(),
                    pending.to_btc()
                );
                println!();
            }

            // VTXOs
            if let Ok(vtxos) = wallet.list_vtxos().await {
                if !vtxos.is_empty() {
                    println!("VTXOs:");
                    let mut table = Table::new();
                    table.load_preset(UTF8_FULL);
                    table.set_header(vec!["Outpoint", "Amount (sats)", "Status", "Expiry"]);

                    for vtxo in vtxos {
                        table.add_row(vec![
                            &vtxo.outpoint[..16], // truncated for display
                            &vtxo.amount.to_sat().to_string(),
                            &format!("{:?}", vtxo.status),
                            &vtxo.expiry.format("%Y-%m-%d %H:%M").to_string(),
                        ]);
                    }

                    println!("{}", table);
                }
            }
        }

        BalanceCommands::Address {
            wallet,
            address_type,
        } => {
            let wallet = manager.load_wallet(&wallet).await?;

            match address_type.as_deref() {
                Some("onchain") => {
                    if let Ok(addr) = wallet.get_onchain_address().await {
                        println!("On-chain address: {}", addr.address);
                    }
                }
                Some("ark") => {
                    if let Ok(addr) = wallet.get_ark_address().await {
                        println!("Ark address: {}", addr.address);
                    }
                }
                Some("boarding") => {
                    if let Ok(addr) = wallet.get_boarding_address().await {
                        println!("Boarding address: {}", addr.address);
                    }
                }
                _ => {
                    println!("Addresses for wallet '{}':", wallet.name());

                    if let Ok(onchain_addr) = wallet.get_onchain_address().await {
                        println!("  On-chain: {}", onchain_addr.address);
                    }
                    if let Ok(ark_addr) = wallet.get_ark_address().await {
                        println!("  Ark: {}", ark_addr.address);
                    }
                    if let Ok(boarding_addr) = wallet.get_boarding_address().await {
                        println!("  Boarding: {}", boarding_addr.address);
                    }
                }
            }
        }
    }

    Ok(())
}
