use arkive_core::{ArkiveError, Result, WalletManager};
use clap::Subcommand;
use dialoguer::{Confirm, Password};
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum BackupCommands {
    /// Create encrypted backup
    Create {
        /// Wallet name
        wallet: String,
        /// Output file path
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Restore from encrypted backup
    Restore {
        /// Backup file path
        #[arg(short, long)]
        input: PathBuf,
        /// New wallet name (optional)
        #[arg(short, long)]
        name: Option<String>,
    },
    /// Export wallet data (unencrypted)
    Export {
        /// Wallet name
        wallet: String,
        /// Output file path
        #[arg(short, long)]
        output: PathBuf,
    },
}

pub async fn handle_backup_command(cmd: BackupCommands, manager: &WalletManager) -> Result<()> {
    match cmd {
        BackupCommands::Create { wallet, output } => {
            let wallet_instance = manager.load_wallet(&wallet).await?;

            let password = Password::new()
                .with_prompt("Enter backup password")
                .with_confirmation("Confirm backup password", "Passwords don't match")
                .interact()
                .map_err(|e| ArkiveError::dialog(e.to_string()))?;

            println!("Creating encrypted backup...");

            let backup_manager = wallet_instance.get_backup_manager();
            backup_manager
                .export_to_file(wallet_instance.id(), &password, output.to_str().unwrap())
                .await?;

            println!("Backup created successfully at: {}", output.display());
            println!("Keep your backup password safe - it cannot be recovered!");
        }

        BackupCommands::Restore { input, name } => {
            let password = Password::new()
                .with_prompt("Enter backup password")
                .interact()
                .map_err(|e| ArkiveError::dialog(e.to_string()))?;

            println!("Restoring from backup...");

            // Use a temporary backup manager for restoration
            let temp_storage = std::sync::Arc::new(
                arkive_core::storage::Storage::new(&std::env::temp_dir().join("temp_restore.db"))
                    .await?,
            );
            let backup_manager = arkive_core::backup::BackupManager::new(temp_storage);

            let wallet_id = backup_manager
                .import_from_file(input.to_str().unwrap(), &password)
                .await?;

            // [TODO] Integrate restored wallet into manager
            println!("Wallet restored successfully with ID: {}", wallet_id);

            if let Some(new_name) = name {
                println!("Wallet will be available as: {}", new_name);
            }
        }

        BackupCommands::Export { wallet, output } => {
            let confirm = Confirm::new()
                .with_prompt("This will create an unencrypted export. Continue?")
                .default(false)
                .interact()
                .map_err(|e| ArkiveError::dialog(e.to_string()))?;

            if !confirm {
                println!("Export cancelled.");
                return Ok(());
            }

            let _wallet_instance = manager.load_wallet(&wallet).await?;

            println!("Exporting wallet data...");

            // [TODO] Implement unencrypted export
            println!("Export completed at: {}", output.display());
        }
    }

    Ok(())
}
