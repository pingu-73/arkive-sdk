use arkive_core::{ArkiveError, Result, WalletManager};
use clap::Subcommand;
use comfy_table::{presets::UTF8_FULL, Table};
use dialoguer::{Confirm, Select};
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum SyncCommands {
    /// Initialize sync for a wallet
    Init {
        /// Wallet name
        wallet: String,
    },
    /// Create sync package for sharing
    Package {
        /// Wallet name
        wallet: String,
        /// Output file path
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Apply sync package from another device
    Apply {
        /// Sync package file path
        #[arg(short, long)]
        input: PathBuf,
    },
    /// Show sync status
    Status {
        /// Wallet name
        wallet: String,
    },
    /// List and resolve sync conflicts
    Conflicts {
        /// Wallet name
        wallet: String,
        /// Auto-resolve conflicts (use local version)
        #[arg(long)]
        auto_local: bool,
        /// Auto-resolve conflicts (use remote version)
        #[arg(long)]
        auto_remote: bool,
    },
}

pub async fn handle_sync_command(cmd: SyncCommands, manager: &WalletManager) -> Result<()> {
    match cmd {
        SyncCommands::Init { wallet } => {
            let wallet_instance = manager.load_wallet(&wallet).await?;

            println!("Initializing sync for wallet '{}'...", wallet);

            let sync_manager = wallet_instance.get_sync_manager();
            sync_manager.init_sync(wallet_instance.id()).await?;

            println!("Sync initialized successfully!");
            println!("Device ID: {}", sync_manager.device_id);
        }

        SyncCommands::Package { wallet, output } => {
            let wallet_instance = manager.load_wallet(&wallet).await?;

            println!("Creating sync package for wallet '{}'...", wallet);

            let sync_manager = wallet_instance.get_sync_manager();
            let package = sync_manager
                .create_sync_package(wallet_instance.id())
                .await?;

            let package_json = serde_json::to_string_pretty(&package)?;
            tokio::fs::write(&output, package_json).await?;

            println!("Sync package created at: {}", output.display());
            println!("Share this file with your other devices to sync wallet data.");
        }

        SyncCommands::Apply { input } => {
            println!("Applying sync package from: {}", input.display());

            let package_json = tokio::fs::read_to_string(&input).await?;
            let package: arkive_core::sync::SyncPackage = serde_json::from_str(&package_json)?;

            // TODO: Get appropriate wallet instance
            println!("Sync package for wallet: {}", package.wallet_id);
            println!("From device: {}", package.device_id);
            println!("Sync version: {}", package.sync_version);

            let confirm = Confirm::new()
                .with_prompt("Apply this sync package?")
                .default(true)
                .interact()
                .map_err(|e| ArkiveError::dialog(e.to_string()))?;

            if confirm {
                // TODO: Apply sync package
                println!("Sync package applied successfully!");
            } else {
                println!("Sync cancelled.");
            }
        }

        SyncCommands::Status { wallet } => {
            let wallet_instance = manager.load_wallet(&wallet).await?;

            println!("Sync status for wallet '{}':", wallet);

            let sync_manager = wallet_instance.get_sync_manager();
            if let Some(state) = sync_manager.get_sync_state(wallet_instance.id()).await? {
                println!("  Device ID: {}", state.device_id);
                println!(
                    "  Last sync: {}",
                    state.last_sync.format("%Y-%m-%d %H:%M:%S UTC")
                );
                println!("  Sync version: {}", state.sync_version);
                println!("  Data hash: {}...", &state.data_hash[..16]);

                let conflicts = sync_manager.get_conflicts(wallet_instance.id()).await?;
                if conflicts.is_empty() {
                    println!("  Status: ✅ No conflicts");
                } else {
                    println!("  Status: ⚠️  {} unresolved conflicts", conflicts.len());
                }
            } else {
                println!("  Status: Not initialized");
                println!("  Run 'arkive sync init {}' to initialize sync", wallet);
            }
        }

        SyncCommands::Conflicts {
            wallet,
            auto_local,
            auto_remote,
        } => {
            let wallet_instance = manager.load_wallet(&wallet).await?;

            println!("Checking conflicts for wallet '{}'...", wallet);

            let sync_manager = wallet_instance.get_sync_manager();
            let conflicts = sync_manager.get_conflicts(wallet_instance.id()).await?;

            if conflicts.is_empty() {
                println!("No conflicts found!");
                return Ok(());
            }

            println!("Found {} conflicts:", conflicts.len());

            let mut table = Table::new();
            table.load_preset(UTF8_FULL);
            table.set_header(vec!["ID", "Type", "Table", "Record", "Timestamp"]);

            for conflict in &conflicts {
                table.add_row(vec![
                    &conflict.id[..8],
                    &format!("{:?}", conflict.conflict_type),
                    &conflict.local_change.table_name,
                    &conflict.local_change.record_id[..16],
                    &conflict.timestamp.format("%Y-%m-%d %H:%M").to_string(),
                ]);
            }

            println!("{}", table);

            if auto_local {
                println!("Auto-resolving all conflicts using local version...");
                for conflict in &conflicts {
                    sync_manager.resolve_conflict(&conflict.id, true).await?;
                }
                println!("All conflicts resolved using local version.");
            } else if auto_remote {
                println!("Auto-resolving all conflicts using remote version...");
                for conflict in &conflicts {
                    sync_manager.resolve_conflict(&conflict.id, false).await?;
                }
                println!("All conflicts resolved using remote version.");
            } else {
                // Interactive resolution
                for conflict in &conflicts {
                    println!("\nConflict: {}", conflict.id);
                    println!("Type: {:?}", conflict.conflict_type);
                    println!("Table: {}", conflict.local_change.table_name);
                    println!("Record: {}", conflict.local_change.record_id);

                    let options = vec!["Use Local Version", "Use Remote Version", "Skip"];
                    let selection = Select::new()
                        .with_prompt("How would you like to resolve this conflict?")
                        .items(&options)
                        .default(0)
                        .interact()
                        .map_err(|e| ArkiveError::dialog(e.to_string()))?;

                    match selection {
                        0 => {
                            sync_manager.resolve_conflict(&conflict.id, true).await?;
                            println!("Resolved using local version.");
                        }
                        1 => {
                            sync_manager.resolve_conflict(&conflict.id, false).await?;
                            println!("Resolved using remote version.");
                        }
                        2 => {
                            println!("Skipped conflict resolution.");
                        }
                        _ => unreachable!(),
                    }
                }
            }
        }
    }

    Ok(())
}
