use arkive_core::WalletManager;
use bitcoin::Network;
use tempfile::tempdir;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    // Create temp dir
    let temp_dir = tempdir()?;
    println!("Using temporary directory: {:?}", temp_dir.path());

    // Initialize wallet manager
    let manager = WalletManager::new(temp_dir.path()).await?;

    println!("Creating wallet...");
    let (wallet, mnemonic) = manager.create_wallet("example-wallet", Network::Regtest).await?;
    
    println!("Wallet created!");
    println!("Mnemonic: {}", mnemonic);
    println!("Wallet ID: {}", wallet.id());

    // Get addresses
    let onchain_addr = wallet.get_onchain_address().await?;
    let ark_addr = wallet.get_ark_address().await?;
    
    println!("\nAddresses:");
    println!("On-chain: {}", onchain_addr.address);
    println!("Ark: {}", ark_addr.address);

    // Check balance
    let balance = wallet.balance().await?;
    println!("\nBalance:");
    println!("Confirmed: {} sats", balance.confirmed.to_sat());
    println!("Pending: {} sats", balance.pending.to_sat());

    // List wallets
    let wallets = manager.list_wallets().await?;
    println!("\nAvailable wallets: {:?}", wallets);

    // Get transaction history
    let history = wallet.transaction_history().await?;
    println!("\nTransaction history: {} transactions", history.len());

    println!("\nExample completed successfully!");

    Ok(())
}