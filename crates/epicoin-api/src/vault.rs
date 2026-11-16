//! Encryption at rest of the custodial wallet keys.
//!
//! Each wallet secret is sealed with XChaCha20-Poly1305 under a 32-byte master key that only
//! exists in the process environment. The 24-byte nonce is random per encryption; the owner's
//! user id is bound as associated data, so a ciphertext copied onto another user's row does
//! not decrypt.
//!
//! Rotation: the vault can hold a *previous* master key next to the current one. Each
//! ciphertext records the fingerprint of the key that sealed it, so old rows stay readable
//! while `rotate-master-key` re-seals them under the current key.

use std::fmt;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use zeroize::Zeroizing;

const NONCE_LEN: usize = 24;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum VaultError {
    #[error("the master key must be 64 hex characters (32 bytes)")]
    BadMasterKey,
    #[error("no master key with fingerprint {0} is configured")]
    UnknownKey(String),
    #[error("decryption failed: wrong key or corrupted data")]
    Decrypt,
    #[error("encryption failed")]
    Encrypt,
}

/// A 32-byte master key. Never printed, wiped on drop.
pub struct MasterKey {
    bytes: Zeroizing<[u8; 32]>,
    id: String,
}

impl MasterKey {
    pub fn from_hex(hex_key: &str) -> Result<Self, VaultError> {
        let mut bytes = Zeroizing::new([0u8; 32]);
        hex::decode_to_slice(hex_key.trim(), bytes.as_mut())
            .map_err(|_| VaultError::BadMasterKey)?;
        Ok(Self::from_bytes(bytes))
    }

    pub fn from_bytes(bytes: Zeroizing<[u8; 32]>) -> Self {
        // A public fingerprint, safe to store next to ciphertexts and to log.
        let digest = blake3::derive_key("epicoin v1 master key fingerprint", bytes.as_ref());
        Self {
            id: hex::encode(&digest[..6]),
            bytes,
        }
    }

    /// Generates a new random master key, as hex. For setup tooling.
    pub fn generate_hex() -> Zeroizing<String> {
        let mut bytes = Zeroizing::new([0u8; 32]);
        getrandom::fill(bytes.as_mut()).expect("operating system random source unavailable");
        Zeroizing::new(hex::encode(bytes.as_ref()))
    }

    /// Public fingerprint of the key.
    pub fn id(&self) -> &str {
        &self.id
    }

    fn cipher(&self) -> XChaCha20Poly1305 {
        let key: &[u8; 32] = &self.bytes;
        XChaCha20Poly1305::new(key.into())
    }
}

impl fmt::Debug for MasterKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MasterKey(id={}, <redacted>)", self.id)
    }
}

/// A sealed secret and the fingerprint of the key that sealed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sealed {
    pub key_id: String,
    /// `nonce || ciphertext`.
    pub bytes: Vec<u8>,
}

#[derive(Debug)]
pub struct Vault {
    current: MasterKey,
    previous: Option<MasterKey>,
}

impl Vault {
    pub fn new(current: MasterKey, previous: Option<MasterKey>) -> Self {
        Self { current, previous }
    }

    /// Fingerprint of the key new secrets are sealed with.
    pub fn current_key_id(&self) -> &str {
        self.current.id()
    }

    /// Seals `secret` under the current master key. `context` (the owner's id) must be given
    /// again to open it.
    pub fn seal(&self, secret: &[u8], context: &[u8]) -> Result<Sealed, VaultError> {
        let mut nonce = [0u8; NONCE_LEN];
        getrandom::fill(&mut nonce).map_err(|_| VaultError::Encrypt)?;
        let ciphertext = self
            .current
            .cipher()
            .encrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: secret,
                    aad: context,
                },
            )
            .map_err(|_| VaultError::Encrypt)?;
        let mut bytes = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        bytes.extend_from_slice(&nonce);
        bytes.extend_from_slice(&ciphertext);
        Ok(Sealed {
            key_id: self.current.id().to_owned(),
            bytes,
        })
    }

    /// Opens a secret sealed by the current or the previous master key.
    pub fn open(&self, sealed: &Sealed, context: &[u8]) -> Result<Zeroizing<Vec<u8>>, VaultError> {
        let key = [Some(&self.current), self.previous.as_ref()]
            .into_iter()
            .flatten()
            .find(|key| key.id() == sealed.key_id)
            .ok_or_else(|| VaultError::UnknownKey(sealed.key_id.clone()))?;
        if sealed.bytes.len() < NONCE_LEN {
            return Err(VaultError::Decrypt);
        }
        let (nonce, ciphertext) = sealed.bytes.split_at(NONCE_LEN);
        let nonce: [u8; NONCE_LEN] = nonce.try_into().map_err(|_| VaultError::Decrypt)?;
        key.cipher()
            .decrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: ciphertext,
                    aad: context,
                },
            )
            .map(Zeroizing::new)
            .map_err(|_| VaultError::Decrypt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(byte: u8) -> MasterKey {
        MasterKey::from_bytes(Zeroizing::new([byte; 32]))
    }

    #[test]
    fn seal_then_open_round_trips() {
        let vault = Vault::new(key(1), None);
        let sealed = vault.seal(b"wallet secret", b"user-1").unwrap();
        assert_eq!(sealed.key_id, vault.current_key_id());
        assert_eq!(
            vault.open(&sealed, b"user-1").unwrap().as_slice(),
            b"wallet secret"
        );
    }

    #[test]
    fn ciphertext_does_not_contain_the_secret_and_is_randomized() {
        let vault = Vault::new(key(1), None);
        let secret = [0xAB; 32];
        let a = vault.seal(&secret, b"u").unwrap();
        let b = vault.seal(&secret, b"u").unwrap();
        assert_ne!(a.bytes, b.bytes, "nonces must differ");
        assert!(!a.bytes.windows(8).any(|w| w == [0xAB; 8]));
        assert_eq!(a.bytes.len(), 24 + 32 + 16);
    }

    #[test]
    fn wrong_context_wrong_key_or_tampering_fail() {
        let vault = Vault::new(key(1), None);
        let sealed = vault.seal(b"secret", b"user-1").unwrap();

        assert_eq!(vault.open(&sealed, b"user-2"), Err(VaultError::Decrypt));

        let other = Vault::new(key(2), None);
        assert!(matches!(
            other.open(&sealed, b"user-1"),
            Err(VaultError::UnknownKey(_))
        ));
        // Even when lying about the key id.
        let mut relabeled = sealed.clone();
        relabeled.key_id = other.current_key_id().to_owned();
        assert_eq!(other.open(&relabeled, b"user-1"), Err(VaultError::Decrypt));

        let mut tampered = sealed.clone();
        *tampered.bytes.last_mut().unwrap() ^= 1;
        assert_eq!(vault.open(&tampered, b"user-1"), Err(VaultError::Decrypt));

        let mut truncated = sealed;
        truncated.bytes.truncate(10);
        assert_eq!(vault.open(&truncated, b"user-1"), Err(VaultError::Decrypt));
    }

    #[test]
    fn previous_key_still_opens_during_rotation() {
        let old = Vault::new(key(1), None);
        let sealed = old.seal(b"secret", b"u").unwrap();

        let rotating = Vault::new(key(2), Some(key(1)));
        let opened = rotating.open(&sealed, b"u").unwrap();
        let resealed = rotating.seal(&opened, b"u").unwrap();
        assert_eq!(resealed.key_id, rotating.current_key_id());
        assert_ne!(resealed.key_id, sealed.key_id);

        // Once rotation is done, the old key can be removed.
        let rotated = Vault::new(key(2), None);
        assert_eq!(rotated.open(&resealed, b"u").unwrap().as_slice(), b"secret");
        assert!(rotated.open(&sealed, b"u").is_err());
    }

    #[test]
    fn master_key_parsing_and_redaction() {
        assert_eq!(
            MasterKey::from_hex("abcd").unwrap_err(),
            VaultError::BadMasterKey
        );
        let hex_key = MasterKey::generate_hex();
        assert_eq!(hex_key.len(), 64);
        let key = MasterKey::from_hex(&hex_key).unwrap();
        let debug = format!("{key:?}");
        let debug = format!("{debug} {:?}", Vault::new(key, None));
        assert!(!debug.contains(hex_key.as_str()));
        assert!(debug.contains("redacted"));
    }
}
