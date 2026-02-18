use aes_gcm::aead::rand_core::RngCore;
use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use base64::{engine::general_purpose::STANDARD, Engine};

const PREFIX: &str = "enc:v1:";

pub fn load_key() -> Option<[u8; 32]> {
    let raw = std::env::var("VIGILO_ENCRYPTION_KEY").ok()?;
    let bytes = STANDARD.decode(raw.trim()).ok()?;
    bytes.try_into().ok()
}

pub fn encrypt(key: &[u8; 32], plaintext: &str) -> Result<String, aes_gcm::Error> {
    let cipher = Aes256Gcm::new(key.into());
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher.encrypt(nonce, plaintext.as_bytes())?;
    let mut payload = nonce_bytes.to_vec();
    payload.extend_from_slice(&ciphertext);
    Ok(format!("{PREFIX}{}", STANDARD.encode(payload)))
}

pub fn decrypt(key: &[u8; 32], ciphertext: &str) -> Option<String> {
    let b64 = ciphertext.strip_prefix(PREFIX)?;
    let payload = STANDARD.decode(b64).ok()?;
    if payload.len() < 12 {
        return None;
    }
    let (nonce_bytes, ct) = payload.split_at(12);
    let cipher = Aes256Gcm::new(key.into());
    let plaintext = cipher.decrypt(Nonce::from_slice(nonce_bytes), ct).ok()?;
    String::from_utf8(plaintext).ok()
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

    fn test_key() -> [u8; 32] {
        [42u8; 32]
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
        let wrong_key = [0u8; 32];
        assert!(decrypt(&wrong_key, &ct).is_none());
    }

    #[test]
    fn non_encrypted_string_not_detected() {
        assert!(!is_encrypted("plaintext"));
    }
}
