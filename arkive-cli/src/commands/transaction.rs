use arkive_core::{ArkiveError, Result, WalletManager};
use bitcoin::Amount;
use clap::Subcommand;
use comfy_table::{presets::UTF8_FULL, Table};

#[derive(Subcommand)]
pub enum TransactionCommands {
    /// Send on-chain Bitcoin transaction
    SendOnchain {
        /// Wallet name
        wallet: String,
        /// Recipient address
        address: String,
        /// Amount in satoshis
        amount: u64,
    },
    /// Send Ark transaction
    SendArk {
        /// Wallet name
        wallet: String,
        /// Recipient Ark address
        address: String,
        /// Amount in satoshis
        amount: u64,
    },
    /// Show transaction history
    History {
        /// Wallet name
        wallet: String,
        /// Number of transactions to show
        #[arg(short, long, default_value = "10")]
        limit: usize,
    },
    /// Estimate transaction fee
    EstimateFee {
        /// Wallet name
        wallet: String,
        /// Transaction type (onchain, ark)
        #[arg(short, long)]
        tx_type: String,
        /// Recipient address
        address: String,
        /// Amount in satoshis
        amount: u64,
    },
}

pub async fn handle_transaction_command(
    cmd: TransactionCommands,
    manager: &WalletManager,
) -> Result<()> {
    match cmd {
        TransactionCommands::SendOnchain {
            wallet,
            address,
            amount,
        } => {
            let wallet = manager.load_wallet(&wallet).await?;
            let amount = Amount::from_sat(amount);

            // Check balance
            let balance = wallet.onchain_balance().await?;
            if balance < amount {
                return Err(ArkiveError::InsufficientFunds {
                    need: amount.to_sat(),
                    available: balance.to_sat(),
                });
            }

            println!(
                "Sending {} sats to {} via on-chain transaction...",
                amount.to_sat(),
                address
            );

            // Estimate fee first
            if let Ok(fee) = wallet.estimate_onchain_fee(&address, amount).await {
                println!("Estimated fee: {} sats", fee.to_sat());

                if balance < amount + fee {
                    return Err(ArkiveError::InsufficientFunds {
                        need: (amount + fee).to_sat(),
                        available: balance.to_sat(),
                    });
                }
            }

            match wallet.send_onchain(&address, amount).await {
                Ok(txid) => {
                    println!("Transaction sent successfully!");
                    println!("Transaction ID: {}", txid);
                }
                Err(e) => {
                    println!("Transaction failed: {}", e);
                    return Err(e);
                }
            }
        }

        TransactionCommands::SendArk {
            wallet,
            address,
            amount,
        } => {
            let wallet = manager.load_wallet(&wallet).await?;
            let amount = Amount::from_sat(amount);

            // Check Ark balance
            let (confirmed, _pending) = wallet.ark_balance().await?;
            if confirmed < amount {
                return Err(ArkiveError::InsufficientFunds {
                    need: amount.to_sat(),
                    available: confirmed.to_sat(),
                });
            }

            println!(
                "Sending {} sats to {} via Ark transaction...",
                amount.to_sat(),
                address
            );

            // Estimate fee
            if let Ok(fee) = wallet.estimate_ark_fee(amount).await {
                println!("Estimated fee: {} sats", fee.to_sat());
            }

            match wallet.send_ark(&address, amount).await {
                Ok(txid) => {
                    println!("Ark transaction sent successfully!");
                    println!("Transaction ID: {}", txid);
                }
                Err(e) => {
                    println!("Transaction failed: {}", e);
                    return Err(e);
                }
            }
        }

        TransactionCommands::History { wallet, limit } => {
            let wallet = manager.load_wallet(&wallet).await?;
            println!("Transaction history for wallet '{}':", wallet.name());

            let transactions = wallet.transaction_history().await?;

            if transactions.is_empty() {
                println!("No transactions found.");
                return Ok(());
            }

            let mut table = Table::new();
            table.load_preset(UTF8_FULL);
            table.set_header(vec!["Date", "Type", "Amount", "Status", "TXID", "Round"]);

            for tx in transactions.iter().take(limit) {
                let amount_str = if tx.amount >= 0 {
                    format!("+{} sats", tx.amount)
                } else {
                    format!("{} sats", tx.amount)
                };

                let round_display = tx
                    .ark_round_id
                    .as_ref()
                    .map(|id| id.replace("round_", ""))
                    .unwrap_or_else(|| "-".to_string());

                table.add_row(vec![
                    &tx.timestamp.format("%Y-%m-%d %H:%M").to_string(),
                    &format!("{:?}", tx.tx_type),
                    &amount_str,
                    &format!("{:?}", tx.status),
                    &tx.txid[..16],
                    &round_display,
                ]);
            }

            println!("{}", table);

            if transactions.len() > limit {
                println!(
                    "\nShowing {} of {} transactions. Use --limit to see more.",
                    limit,
                    transactions.len()
                );
            }
        }

        TransactionCommands::EstimateFee {
            wallet,
            tx_type,
            address,
            amount,
        } => {
            let wallet = manager.load_wallet(&wallet).await?;
            let amount = Amount::from_sat(amount);

            match tx_type.as_str() {
                "onchain" => match wallet.estimate_onchain_fee(&address, amount).await {
                    Ok(fee) => {
                        println!("On-chain transaction fee estimate:");
                        println!("  Amount: {} sats", amount.to_sat());
                        println!("  Fee: {} sats", fee.to_sat());
                        println!("  Total: {} sats", (amount + fee).to_sat());
                    }
                    Err(e) => {
                        println!("Failed to estimate fee: {}", e);
                    }
                },
                "ark" => match wallet.estimate_ark_fee(amount).await {
                    Ok(fee) => {
                        println!("Ark transaction fee estimate:");
                        println!("  Amount: {} sats", amount.to_sat());
                        println!("  Fee: {} sats", fee.to_sat());
                        println!("  Total: {} sats", (amount + fee).to_sat());
                    }
                    Err(e) => {
                        println!("Failed to estimate fee: {}", e);
                    }
                },
                _ => {
                    return Err(ArkiveError::config(
                        "Invalid transaction type. Use 'onchain' or 'ark'",
                    ));
                }
            }
        }
    }

    Ok(())
}
