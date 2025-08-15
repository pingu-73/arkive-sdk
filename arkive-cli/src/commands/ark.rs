use arkive_core::{Result, WalletManager};
use clap::Subcommand;
use comfy_table::{presets::UTF8_FULL, Table};

#[derive(Subcommand)]
pub enum ArkCommands {
    /// List VTXOs
    Vtxos {
        /// Wallet name
        wallet: String,
        /// Filter by status (all, spendable, preconfirmed, rercoverable)
        #[arg(short, long, default_value = " all ")]
        filter: String,
    },
    /// Participate in a round
    Round {
        /// Wallet name
        wallet: String,
    },
    BatchSwap {
        /// Wallet name
        wallet: String,
        /// Specific VTXO outpoints to swap (optional)
        #[arg(long)]
        vtxos: Option<Vec<String>>,
    },
    /// List batch swaps
    Swaps {
        /// Wallet name
        wallet: String,
    },
    /// Get batch swap details
    SwapInfo {
        /// Wallet name
        wallet: String,
        /// Batch swap ID
        swap_id: String,
    },
    /// Auto-manage VTXOs (batch swap expiring ones)
    AutoManage {
        /// Wallet name
        wallet: String,
        /// Hours threshold for expiry (default: 24)
        #[arg(short, long, default_value = "24")]
        threshold: i64,
    },
    /// Show VTXO statistics
    Stats {
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
        ArkCommands::Vtxos { wallet, filter } => {
            let wallet = manager.load_wallet(&wallet).await?;

            println!("VTXOs for wallet '{}':", wallet.name());

            let vtxos = match filter.as_str() {
                "spendable" => wallet.get_spendable_vtxos().await?,
                "preconfirmed" => wallet.get_preconfirmed_vtxos().await?,
                "recoverable" => wallet.get_recoverable_vtxos().await?,
                _ => {
                    println!("Warning: Unknown filter '{}', showing all VTXOs", filter);
                    wallet.list_vtxos().await?
                }
            };

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
                "Preconf",
                "Recover",
                "Batch",
            ]);

            for vtxo in vtxos {
                #[allow(unused_variables)]
                let status_display = match vtxo.status {
                    arkive_core::types::VtxoStatus::Preconfirmed => "⏳ Preconf",
                    arkive_core::types::VtxoStatus::Unconfirmed => "🔄 Unconf",
                    arkive_core::types::VtxoStatus::Confirmed => "✅ Conf",
                    arkive_core::types::VtxoStatus::Pending => "🟡 Pend",
                    arkive_core::types::VtxoStatus::Spent => "❌ Spent",
                    arkive_core::types::VtxoStatus::Expired => "⏰ Exp",
                    arkive_core::types::VtxoStatus::Replaced => "🔄 Repl",
                };

                let preconf_indicator = if vtxo.is_preconfirmed { "✓" } else { "-" };
                let recover_indicator = if vtxo.is_recoverable { "✓" } else { "-" };

                table.add_row(vec![
                    &format!("{}...", &vtxo.outpoint[..16]),
                    &vtxo.amount.to_sat().to_string(),
                    &format!("{:?}", vtxo.status),
                    &vtxo.expiry.format("%Y-%m-%d %H:%M").to_string(),
                    &format!("{}...", &vtxo.address[..20]),
                    preconf_indicator,
                    recover_indicator,
                    &vtxo.batch_id.as_deref().unwrap_or("-"),
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

        ArkCommands::BatchSwap { wallet, vtxos } => {
            let wallet = manager.load_wallet(&wallet).await?;

            println!("Initiating batch swap for wallet '{}'...", wallet.name());

            if let Some(ref vtxo_list) = vtxos {
                println!("Swapping specific VTXOs: {:?}", vtxo_list);
            } else {
                println!("Auto-selecting VTXOs that need swapping...");
            }

            match wallet.batch_swap(vtxos).await {
                Ok(Some(swap_id)) => {
                    println!("Batch swap initiated successfully!");
                    println!("Swap ID: {}", swap_id);
                    println!(
                        "Use 'arkive ark swap-info {} {}' to check status",
                        wallet.name(),
                        swap_id
                    );
                }
                Ok(None) => {
                    println!("No VTXOs need batch swap at this time.");
                }
                Err(e) => {
                    println!("Failed to initiate batch swap: {}", e);
                    return Err(e);
                }
            }
        }

        ArkCommands::Swaps { wallet } => {
            let wallet = manager.load_wallet(&wallet).await?;

            println!("Batch swaps for wallet '{}':", wallet.name());

            let swaps = wallet.list_batch_swaps().await?;

            if swaps.is_empty() {
                println!("No batch swaps found.");
                return Ok(());
            }

            let mut table = Table::new();
            table.load_preset(UTF8_FULL);
            table.set_header(vec![
                "Swap ID",
                "Status",
                "Input VTXOs",
                "Output VTXOs",
                "Created",
                "Expires",
                "Commitment TX",
            ]);

            for swap in swaps {
                let status_display = match swap.status {
                    arkive_core::types::BatchSwapStatus::Pending => "🟡 Pending",
                    arkive_core::types::BatchSwapStatus::Signed => "✍️ Signed",
                    arkive_core::types::BatchSwapStatus::Committed => "📝 Committed",
                    arkive_core::types::BatchSwapStatus::Confirmed => "✅ Confirmed",
                    arkive_core::types::BatchSwapStatus::Failed => "❌ Failed",
                };

                table.add_row(vec![
                    &swap.swap_id[..8],
                    status_display,
                    &swap.input_vtxos.len().to_string(),
                    &swap.output_vtxos.len().to_string(),
                    &swap.created_at.format("%Y-%m-%d %H:%M").to_string(),
                    &swap.expires_at.format("%Y-%m-%d %H:%M").to_string(),
                    &swap.commitment_txid.as_deref().unwrap_or("-")[..16],
                ]);
            }

            println!("{}", table);
        }

        ArkCommands::SwapInfo { wallet, swap_id } => {
            let wallet = manager.load_wallet(&wallet).await?;

            println!("Batch swap details for '{}':", swap_id);

            match wallet.get_batch_swap_info(&swap_id).await? {
                Some(swap) => {
                    println!("Swap ID: {}", swap.swap_id);
                    println!("Status: {:?}", swap.status);
                    println!(
                        "Created: {}",
                        swap.created_at.format("%Y-%m-%d %H:%M:%S UTC")
                    );
                    println!(
                        "Expires: {}",
                        swap.expires_at.format("%Y-%m-%d %H:%M:%S UTC")
                    );

                    if let Some(commitment_txid) = &swap.commitment_txid {
                        println!("Commitment TX: {}", commitment_txid);
                    }

                    println!("\nInput VTXOs ({}):", swap.input_vtxos.len());
                    for (i, vtxo) in swap.input_vtxos.iter().enumerate() {
                        println!("  {}: {}", i + 1, vtxo);
                    }

                    if !swap.output_vtxos.is_empty() {
                        println!("\nOutput VTXOs ({}):", swap.output_vtxos.len());
                        for (i, vtxo) in swap.output_vtxos.iter().enumerate() {
                            println!("  {}: {}", i + 1, vtxo);
                        }
                    }
                }
                None => {
                    println!("Batch swap '{}' not found.", swap_id);
                }
            }
        }

        ArkCommands::AutoManage { wallet, threshold } => {
            let wallet = manager.load_wallet(&wallet).await?;

            println!("Auto-managing VTXOs for wallet '{}'...", wallet.name());
            println!("Checking for VTXOs expiring within {} hours", threshold);

            match wallet.auto_manage_vtxos(threshold).await {
                Ok(Some(swap_id)) => {
                    println!("Auto-management completed!");
                    println!("Initiated batch swap: {}", swap_id);
                }
                Ok(None) => {
                    println!("No VTXOs need management at this time.");
                }
                Err(e) => {
                    println!("Auto-management failed: {}", e);
                    return Err(e);
                }
            }
        }

        ArkCommands::Stats { wallet } => {
            let wallet = manager.load_wallet(&wallet).await?;

            println!("VTXO statistics for wallet '{}':", wallet.name());

            let stats = wallet.get_vtxo_statistics().await?;

            println!("\n📊 Overall Statistics:");
            println!(
                "  Total VTXOs: {} ({} sats)",
                stats.total_count,
                stats.total_value.to_sat()
            );

            println!("\n📈 By Status:");
            println!(
                "  Confirmed: {} ({} sats)",
                stats.confirmed_count,
                stats.confirmed_value.to_sat()
            );
            println!(
                "  Preconfirmed: {} ({} sats)",
                stats.preconfirmed_count,
                stats.preconfirmed_value.to_sat()
            );
            println!(
                "  Pending: {} ({} sats)",
                stats.pending_count,
                stats.pending_value.to_sat()
            );
            println!(
                "  Spent: {} ({} sats)",
                stats.spent_count,
                stats.spent_value.to_sat()
            );
            println!(
                "  Expired: {} ({} sats)",
                stats.expired_count,
                stats.expired_value.to_sat()
            );

            println!("\n🔄 Special Categories:");
            println!(
                "  Recoverable: {} ({} sats)",
                stats.recoverable_count,
                stats.recoverable_value.to_sat()
            );
            println!(
                "  Expiring Soon (24h): {} ({} sats)",
                stats.expiring_soon_count,
                stats.expiring_soon_value.to_sat()
            );

            if stats.expiring_soon_count > 0 {
                println!(
                    "\n⚠️  Warning: {} VTXOs are expiring within 24 hours!",
                    stats.expiring_soon_count
                );
                println!(
                    "   Consider running: arkive ark auto-manage {}",
                    wallet.name()
                );
            }

            if stats.preconfirmed_count > 0 {
                println!(
                    "\n💡 Info: {} preconfirmed VTXOs can be batch swapped for confirmation",
                    stats.preconfirmed_count
                );
                println!("   Run: arkive ark batch-swap {}", wallet.name());
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

                    if let Ok(preconfirmed) = wallet.get_preconfirmed_vtxos().await {
                        if !preconfirmed.is_empty() {
                            println!(
                                "\n💡 Found {} preconfirmed VTXOs that can be batch swapped",
                                preconfirmed.len()
                            );
                        }
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
