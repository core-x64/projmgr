use std::fs;
use std::path::Path;

use aes_gcm::aead::rand_core::{OsRng, RngCore};
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use sha2::{Digest, Sha256};
use zeroize::Zeroize;

use crate::data::AppResult;

const MAGIC: &[u8] = b"UNITENV1";

fn derive_key(password: &[u8]) -> [u8; 32] {
    let digest = Sha256::digest(password);
    let mut key = [0_u8; 32];
    key.copy_from_slice(&digest);
    key
}

pub fn encrypt_env_file(path: &Path, password: String) -> AppResult<()> {
    let mut password_bytes = password.into_bytes();
    let mut plaintext =
        fs::read(path).map_err(|e| format!("Failed to read {} for encryption: {e}", path.display()))?;
    if plaintext.is_empty() {
        password_bytes.zeroize();
        return Err(format!("{} is empty; refusing to encrypt empty content", path.display()));
    }

    let key_bytes = derive_key(&password_bytes);
    let cipher = Aes256Gcm::new_from_slice(&key_bytes)
        .map_err(|e| format!("Cipher initialization failed: {e}"))?;

    let mut nonce_bytes = [0_u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let mut ciphertext = cipher
        .encrypt(nonce, plaintext.as_ref())
        .map_err(|e| format!("Encryption failed for {}: {e}", path.display()))?;

    let mut out = Vec::with_capacity(MAGIC.len() + nonce_bytes.len() + ciphertext.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&nonce_bytes);
    out.append(&mut ciphertext);

    fs::write(path, &out).map_err(|e| format!("Failed to write encrypted data to {}: {e}", path.display()))?;

    out.zeroize();
    plaintext.zeroize();
    password_bytes.zeroize();
    Ok(())
}

pub fn decrypt_env_file(path: &Path, password: String) -> AppResult<()> {
    let mut password_bytes = password.into_bytes();
    let mut blob =
        fs::read(path).map_err(|e| format!("Failed to read {} for decryption: {e}", path.display()))?;

    let min_size = MAGIC.len() + 12;
    if blob.len() < min_size || &blob[..MAGIC.len()] != MAGIC {
        password_bytes.zeroize();
        blob.zeroize();
        return Err(format!("{} is not in UNIT encrypted format", path.display()));
    }

    let nonce_start = MAGIC.len();
    let nonce_end = nonce_start + 12;
    let nonce = Nonce::from_slice(&blob[nonce_start..nonce_end]);
    let ciphertext = &blob[nonce_end..];

    let key_bytes = derive_key(&password_bytes);
    let cipher = Aes256Gcm::new_from_slice(&key_bytes)
        .map_err(|e| format!("Cipher initialization failed: {e}"))?;

    let mut plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| format!("Decryption failed for {}: {e}", path.display()))?;

    fs::write(path, &plaintext)
        .map_err(|e| format!("Failed to write decrypted data to {}: {e}", path.display()))?;

    plaintext.zeroize();
    blob.zeroize();
    password_bytes.zeroize();
    Ok(())
}
