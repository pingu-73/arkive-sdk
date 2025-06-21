pub mod config;
pub mod instance;
pub mod manager;

pub use config::WalletConfig;
pub use instance::ArkWallet;
pub use manager::WalletManager;

use crate::error::{ArkiveError, Result};
use bip39::{Language, Mnemonic};
use bitcoin::key::Keypair;
use bitcoin::secp256k1::{Secp256k1, SecretKey};

pub fn generate_mnemonic() -> Result<String> {
    let mut rng = bip39::rand::thread_rng();
    let mnemonic = Mnemonic::generate_in_with(&mut rng, Language::English, 24)
        .map_err(|e| ArkiveError::internal(format!("Failed to generate mnemonic: {}", e)))?;
    Ok(mnemonic.to_string())
}

pub fn mnemonic_to_keypair(mnemonic: &str, network: bitcoin::Network) -> Result<Keypair> {
    let mnemonic = Mnemonic::parse_in(Language::English, mnemonic)
        .map_err(|e| ArkiveError::config(format!("Invalid mnemonic: {}", e)))?;

    let seed = mnemonic.to_seed("");
    let secp = Secp256k1::new();

    let master_key = bitcoin::bip32::Xpriv::new_master(network, &seed)
        .map_err(|e| ArkiveError::internal(format!("Failed to derive master key: {}", e)))?;

    let path = bitcoin::bip32::DerivationPath::from_str("m/84'/0'/0'/0/0")
        .map_err(|e| ArkiveError::config(format!("Invalid derivation path: {}", e)))?;

    let child_key = master_key
        .derive_priv(&secp, &path)
        .map_err(|e| ArkiveError::internal(format!("Failed to derive child key: {}", e)))?;

    let secret_key = SecretKey::from_slice(&child_key.private_key.secret_bytes())
        .map_err(|e| ArkiveError::internal(format!("Invalid secret key: {}", e)))?;

    let keypair = Keypair::from_secret_key(&secp, &secret_key);
    Ok(keypair)
}

use std::str::FromStr;
