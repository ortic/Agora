//! Encryption of secrets at rest (provider API keys): AES-256-GCM with a key derived from
//! `AGORA_SECRET_KEY`. Stored as `nonce (12 bytes) || ciphertext`.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use sha2::{Digest, Sha256};

/// Used when `AGORA_SECRET_KEY` is unset (development only; the server warns at start-up).
const DEV_KEY: &str = "agora-development-key-do-not-use-in-production";

#[derive(Clone)]
pub struct Secrets {
    cipher: Aes256Gcm,
}

impl Secrets {
    pub fn new(secret: &str) -> Self {
        let secret = if secret.trim().is_empty() {
            tracing::warn!("AGORA_SECRET_KEY is not set: provider API keys are encrypted with a built-in development key");
            DEV_KEY
        } else {
            secret
        };
        let digest = Sha256::digest(format!("agora/secrets/v1/{secret}").as_bytes());
        let key = Key::<Aes256Gcm>::from_slice(&digest[..]);
        Secrets { cipher: Aes256Gcm::new(key) }
    }

    pub fn encrypt(&self, plain: &str) -> anyhow::Result<Vec<u8>> {
        let mut nonce = [0u8; 12];
        rand::fill(&mut nonce);
        let ct = self.cipher.encrypt(Nonce::from_slice(&nonce), plain.as_bytes()).map_err(|_| anyhow::anyhow!("encryption failed"))?;
        let mut out = nonce.to_vec();
        out.extend(ct);
        Ok(out)
    }

    pub fn decrypt(&self, data: &[u8]) -> anyhow::Result<String> {
        anyhow::ensure!(data.len() > 12, "stored secret is too short");
        let plain = self
            .cipher
            .decrypt(Nonce::from_slice(&data[..12]), &data[12..])
            .map_err(|_| anyhow::anyhow!("stored secret can't be decrypted (was AGORA_SECRET_KEY changed?)"))?;
        Ok(String::from_utf8(plain)?)
    }
}

/// What the UI shows of a key: its last four characters.
pub fn hint(key: &str) -> String {
    let tail: String = key.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    format!("…{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_wrong_key() {
        let a = Secrets::new("one");
        let enc = a.encrypt("sk-secret-1234").unwrap();
        assert_eq!(a.decrypt(&enc).unwrap(), "sk-secret-1234");
        assert!(Secrets::new("two").decrypt(&enc).is_err());
        assert_ne!(enc, a.encrypt("sk-secret-1234").unwrap(), "fresh nonce every time");
        assert_eq!(hint("sk-secret-1234"), "…1234");
    }
}
