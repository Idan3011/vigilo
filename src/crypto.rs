use aes_gcm::aead::rand_core::RngCore;
use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use zeroize::{Zeroize, ZeroizeOnDrop};

const PREFIX: &str = "enc:v1:";

/// AES-256 key wrapper that zeroizes memory on drop.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct EncryptionKey([u8; 32]);

impl EncryptionKey {
    pub fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Returns the path to the on-disk key file: `~/.vigilo/encryption.key`
pub fn key_file_path() -> std::path::PathBuf {
    crate::models::vigilo_path("encryption.key")
}

/// Try loading key from: env var → key file → None.
pub fn load_key() -> Option<EncryptionKey> {
    if let Some(key) = load_key_from_env() {
        return Some(key);
    }
    load_key_from_file()
}

/// Load key, or auto-generate and persist one if none exists.
/// Used by the MCP server to ensure encryption is always active.
pub fn load_or_create_key() -> Option<EncryptionKey> {
    if let Some(key) = load_key() {
        return Some(key);
    }
    match generate_and_save_key() {
        Ok(key) => {
            eprintln!(
                "[vigilo] auto-generated encryption key → {}",
                key_file_path().display()
            );
            Some(key)
        }
        Err(e) => {
            eprintln!("[vigilo] warning: could not create encryption key: {e}");
            eprintln!("[vigilo] events will be stored in plaintext");
            None
        }
    }
}

fn load_key_from_env() -> Option<EncryptionKey> {
    let raw = std::env::var("VIGILO_ENCRYPTION_KEY").ok()?;
    let bytes = STANDARD.decode(raw.trim()).ok()?;
    let arr: [u8; 32] = bytes.try_into().ok()?;
    Some(EncryptionKey::new(arr))
}

/// Load key from `~/.vigilo/encryption.key`.
pub fn load_key_from_file() -> Option<EncryptionKey> {
    let path = key_file_path();
    let raw = std::fs::read_to_string(&path).ok()?;
    let bytes = STANDARD.decode(raw.trim()).ok()?;
    let arr: [u8; 32] = bytes.try_into().ok()?;
    Some(EncryptionKey::new(arr))
}

/// Generate a new AES-256 key, save it to `~/.vigilo/encryption.key` with mode 600.
pub fn generate_and_save_key() -> std::io::Result<EncryptionKey> {
    let mut key = [0u8; 32];
    OsRng.fill_bytes(&mut key);
    let b64 = STANDARD.encode(key);

    let path = key_file_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, format!("{b64}\n"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }

    Ok(EncryptionKey::new(key))
}

pub fn encrypt(key: &EncryptionKey, plaintext: &str) -> Result<String, aes_gcm::Error> {
    let cipher = Aes256Gcm::new(key.as_bytes().into());
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher.encrypt(nonce, plaintext.as_bytes())?;
    let mut payload = nonce_bytes.to_vec();
    payload.extend_from_slice(&ciphertext);
    Ok(format!("{PREFIX}{}", STANDARD.encode(payload)))
}

pub fn decrypt(key: &EncryptionKey, ciphertext: &str) -> Option<String> {
    let b64 = ciphertext.strip_prefix(PREFIX)?;
    let payload = STANDARD.decode(b64).ok()?;
    if payload.len() < 12 {
        return None;
    }
    let (nonce_bytes, ct) = payload.split_at(12);
    let cipher = Aes256Gcm::new(key.as_bytes().into());
    let plaintext = cipher.decrypt(Nonce::from_slice(nonce_bytes), ct).ok()?;
    String::from_utf8(plaintext).ok()
}

pub fn encrypt_for_ledger(
    encryption_key: Option<&EncryptionKey>,
    arguments: &serde_json::Value,
    outcome: &crate::models::Outcome,
    diff: &Option<String>,
) -> Result<(serde_json::Value, crate::models::Outcome, Option<String>), aes_gcm::Error> {
    let key = match encryption_key {
        Some(k) => k,
        None => return Ok((arguments.clone(), outcome.clone(), diff.clone())),
    };
    let enc_args = serde_json::json!(encrypt(key, &arguments.to_string())?);
    let enc_outcome = match outcome {
        crate::models::Outcome::Ok { result } => crate::models::Outcome::Ok {
            result: serde_json::json!(encrypt(key, &result.to_string())?),
        },
        crate::models::Outcome::Err { .. } => outcome.clone(),
    };
    let enc_diff = match diff.as_deref() {
        Some(d) => Some(encrypt(key, d)?),
        None => None,
    };
    Ok((enc_args, enc_outcome, enc_diff))
}

pub fn is_encrypted(s: &str) -> bool {
    s.starts_with(PREFIX)
}

pub fn generate_key_b64() -> String {
    let mut key = [0u8; 32];
    OsRng.fill_bytes(&mut key);
    STANDARD.encode(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> EncryptionKey {
        EncryptionKey::new([42u8; 32])
    }

    #[test]
    fn round_trip() {
        let key = test_key();
        let ct = encrypt(&key, "hello world").unwrap();
        assert!(is_encrypted(&ct));
        assert_eq!(decrypt(&key, &ct).unwrap(), "hello world");
    }

    #[test]
    fn wrong_key_returns_none() {
        let key = test_key();
        let ct = encrypt(&key, "secret").unwrap();
        let wrong_key = EncryptionKey::new([0u8; 32]);
        assert!(decrypt(&wrong_key, &ct).is_none());
    }

    #[test]
    fn non_encrypted_string_not_detected() {
        assert!(!is_encrypted("plaintext"));
    }

    #[test]
    fn generate_key_b64_produces_valid_32_byte_key() {
        let b64 = generate_key_b64();
        let bytes = STANDARD.decode(&b64).unwrap();
        assert_eq!(bytes.len(), 32);
    }

    #[test]
    fn generate_key_b64_is_unique() {
        let k1 = generate_key_b64();
        let k2 = generate_key_b64();
        assert_ne!(k1, k2);
    }

    #[test]
    fn encrypt_decrypt_empty_string() {
        let key = test_key();
        let ct = encrypt(&key, "").unwrap();
        assert!(is_encrypted(&ct));
        assert_eq!(decrypt(&key, &ct).unwrap(), "");
    }

    #[test]
    fn decrypt_short_payload_returns_none() {
        let key = test_key();
        // Payload shorter than 12-byte nonce
        let short = STANDARD.encode([1u8; 5]);
        let ct = format!("{PREFIX}{short}");
        assert!(decrypt(&key, &ct).is_none());
    }

    #[test]
    fn decrypt_without_prefix_returns_none() {
        let key = test_key();
        assert!(decrypt(&key, "not-encrypted-at-all").is_none());
    }

    #[test]
    fn decrypt_invalid_base64_returns_none() {
        let key = test_key();
        assert!(decrypt(&key, &format!("{PREFIX}!!!invalid-base64!!!")).is_none());
    }

    #[test]
    fn is_encrypted_detects_prefix() {
        assert!(is_encrypted("enc:v1:something"));
        assert!(!is_encrypted("enc:v2:something"));
        assert!(!is_encrypted(""));
    }

    #[test]
    fn generate_and_save_key_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let key_path = dir.path().join(".vigilo").join("encryption.key");

        std::env::set_var("HOME", dir.path().to_str().unwrap());
        let key = generate_and_save_key().unwrap();
        std::env::remove_var("HOME");

        assert_eq!(key.as_bytes().len(), 32);
        assert!(key_path.exists(), "key file should exist at {key_path:?}");

        let raw = std::fs::read_to_string(&key_path).unwrap();
        let bytes = STANDARD.decode(raw.trim()).unwrap();
        let loaded: [u8; 32] = bytes.try_into().unwrap();
        assert_eq!(*key.as_bytes(), loaded);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&key_path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn load_key_from_file_returns_none_for_missing() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", dir.path().to_str().unwrap());
        let result = load_key_from_file();
        std::env::remove_var("HOME");
        assert!(result.is_none());
    }

    #[test]
    fn load_key_from_file_returns_none_for_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let key_path = dir.path().join(".vigilo").join("encryption.key");
        std::fs::create_dir_all(key_path.parent().unwrap()).unwrap();
        std::fs::write(&key_path, "not-valid-base64!!!").unwrap();

        std::env::set_var("HOME", dir.path().to_str().unwrap());
        let result = load_key_from_file();
        std::env::remove_var("HOME");
        assert!(result.is_none());
    }

    #[test]
    fn load_or_create_key_generates_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        std::env::remove_var("VIGILO_ENCRYPTION_KEY");
        std::env::set_var("HOME", dir.path().to_str().unwrap());

        let key = load_or_create_key().unwrap();
        assert_eq!(key.as_bytes().len(), 32);

        let key_path = dir.path().join(".vigilo").join("encryption.key");
        assert!(key_path.exists());
        let raw = std::fs::read_to_string(&key_path).unwrap();
        let bytes = STANDARD.decode(raw.trim()).unwrap();
        let loaded: [u8; 32] = bytes.try_into().unwrap();
        assert_eq!(*key.as_bytes(), loaded);

        std::env::remove_var("HOME");
    }
}
