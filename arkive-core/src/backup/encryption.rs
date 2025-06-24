#![allow(dead_code)]
use crate::backup::EncryptedBackup;
use crate::error::{ArkiveError, Result};
use bip39::rand::{rngs::OsRng, RngCore};
use chrono::Utc;
use sha2::{Digest, Sha256};

// ChaCha20Poly1305 for authenticated encryption
use chacha20poly1305::{
    aead::{Aead, AeadCore, KeyInit},
    ChaCha20Poly1305, Key, Nonce,
};

const SALT_SIZE: usize = 32;
const NONCE_SIZE: usize = 12;

/// Encrypt data with password using ChaCha20Poly1305
pub fn encrypt_data(data: &[u8], password: &str) -> Result<EncryptedBackup> {
    // Generate random salt
    let mut salt = [0u8; SALT_SIZE];
    OsRng.fill_bytes(&mut salt);

    // Derive key from password using PBKDF2
    let key = derive_key(password, &salt)?;

    // Generate random nonce
    let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);

    // Create cipher
    let cipher = ChaCha20Poly1305::new(&key);

    // Encrypt data
    let encrypted_data = cipher
        .encrypt(&nonce, data)
        .map_err(|e| ArkiveError::internal(format!("Encryption failed: {}", e)))?;

    // Calculate checksum
    let checksum = calculate_checksum(&encrypted_data);

    Ok(EncryptedBackup {
        version: 1,
        encryption_method: "ChaCha20Poly1305".to_string(),
        salt: salt.to_vec(),
        nonce: nonce.to_vec(),
        encrypted_data,
        checksum,
        created_at: Utc::now(),
    })
}

/// Decrypt data with password
pub fn decrypt_data(backup: &EncryptedBackup, password: &str) -> Result<Vec<u8>> {
    // Verify checksum
    let calculated_checksum = calculate_checksum(&backup.encrypted_data);
    if calculated_checksum != backup.checksum {
        return Err(ArkiveError::internal("Backup checksum verification failed"));
    }

    // Derive key from password
    let key = derive_key(password, &backup.salt)?;

    // Create cipher
    let cipher = ChaCha20Poly1305::new(&key);

    // Create nonce
    let nonce = Nonce::from_slice(&backup.nonce);

    // Decrypt data
    let decrypted_data = cipher
        .decrypt(nonce, backup.encrypted_data.as_ref())
        .map_err(|e| ArkiveError::internal(format!("Decryption failed: {}", e)))?;

    Ok(decrypted_data)
}

/// Derive encryption key from password using PBKDF2
fn derive_key(password: &str, salt: &[u8]) -> Result<Key> {
    use pbkdf2::pbkdf2_hmac;
    use sha2::Sha256;

    let mut key = [0u8; 32];
    pbkdf2_hmac::<Sha256>(password.as_bytes(), salt, 100_000, &mut key);
    Ok(*Key::from_slice(&key))
}

/// Calculate SHA256 checksum
fn calculate_checksum(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt() {
        let data = b"test data for encryption";
        let password = "test_password_123";

        let encrypted = encrypt_data(data, password).unwrap();
        let decrypted = decrypt_data(&encrypted, password).unwrap();

        assert_eq!(data, decrypted.as_slice());
    }

    #[test]
    fn test_wrong_password() {
        let data = b"test data for encryption";
        let password = "test_password_123";
        let wrong_password = "wrong_password";

        let encrypted = encrypt_data(data, password).unwrap();
        let result = decrypt_data(&encrypted, wrong_password);

        assert!(result.is_err());
    }
}
