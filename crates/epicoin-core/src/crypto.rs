//! Hashing and signatures.
//!
//! Every hash is domain-separated with a BLAKE3 derive-key context, so a digest computed for
//! one purpose (a transaction, a block header, the state…) can never be confused with another.

use std::fmt;

use borsh::BorshSerialize;
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use zeroize::Zeroizing;

use crate::types::{Address, Hash, PublicKey, Signature};

/// Domain-separation contexts. Changing any of these is a hard fork.
pub mod domain {
    pub const ADDRESS: &str = "epicoin v1 address";
    pub const TX_SIGNING: &str = "epicoin v1 tx signing";
    pub const TX_ID: &str = "epicoin v1 tx id";
    pub const BLOCK: &str = "epicoin v1 block header";
    pub const GENESIS: &str = "epicoin v1 genesis";
    pub const STATE: &str = "epicoin v1 state";
    pub const MERKLE_LEAF: &str = "epicoin v1 merkle leaf";
    pub const MERKLE_NODE: &str = "epicoin v1 merkle node";
    pub const MERKLE_EMPTY: &str = "epicoin v1 merkle empty";
}

/// Hashes raw bytes under a domain.
pub fn hash_bytes(domain: &str, data: &[u8]) -> Hash {
    let mut hasher = blake3::Hasher::new_derive_key(domain);
    hasher.update(data);
    Hash(*hasher.finalize().as_bytes())
}

/// Hashes the canonical (borsh) encoding of a value under a domain.
pub fn hash_borsh<T: BorshSerialize + ?Sized>(domain: &str, value: &T) -> Hash {
    let mut hasher = blake3::Hasher::new_derive_key(domain);
    // Writing into a hasher cannot fail, and none of our types have fallible encodings.
    value
        .serialize(&mut hasher)
        .expect("borsh serialization into a hasher is infallible");
    Hash(*hasher.finalize().as_bytes())
}

impl PublicKey {
    /// The account address controlled by this key.
    pub fn address(&self) -> Address {
        Address(hash_bytes(domain::ADDRESS, &self.0).0)
    }

    /// Checks `signature` over `digest`. Uses strict verification, which rejects malleable
    /// signatures and weak keys.
    pub fn verify(&self, digest: &Hash, signature: &Signature) -> bool {
        let Ok(key) = VerifyingKey::from_bytes(&self.0) else {
            return false;
        };
        let signature = ed25519_dalek::Signature::from_bytes(&signature.0);
        key.verify_strict(&digest.0, &signature).is_ok()
    }
}

/// An ed25519 key pair. The secret never appears in `Debug` output and is wiped on drop.
#[derive(Clone)]
pub struct Keypair {
    signing: SigningKey,
}

impl Keypair {
    /// Generates a fresh key pair from the operating system's CSPRNG.
    ///
    /// # Panics
    /// Panics if the OS random source is unavailable, which is not recoverable.
    pub fn generate() -> Self {
        let mut seed = Zeroizing::new([0u8; 32]);
        getrandom::fill(seed.as_mut()).expect("operating system random source unavailable");
        Self::from_secret_bytes(&seed)
    }

    pub fn from_secret_bytes(secret: &[u8; 32]) -> Self {
        Self {
            signing: SigningKey::from_bytes(secret),
        }
    }

    /// The 32-byte secret seed. Handle with care: encrypt before storing, never log.
    pub fn secret_bytes(&self) -> Zeroizing<[u8; 32]> {
        Zeroizing::new(self.signing.to_bytes())
    }

    pub fn public_key(&self) -> PublicKey {
        PublicKey(self.signing.verifying_key().to_bytes())
    }

    pub fn address(&self) -> Address {
        self.public_key().address()
    }

    pub fn sign(&self, digest: &Hash) -> Signature {
        Signature(self.signing.sign(&digest.0).to_bytes())
    }
}

impl fmt::Debug for Keypair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Keypair(address={}, secret=<redacted>)", self.address())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_and_verify() {
        let kp = Keypair::generate();
        let digest = hash_bytes("test", b"hello");
        let sig = kp.sign(&digest);
        assert!(kp.public_key().verify(&digest, &sig));
    }

    #[test]
    fn verify_rejects_wrong_message_key_or_signature() {
        let kp = Keypair::generate();
        let other = Keypair::generate();
        let digest = hash_bytes("test", b"hello");
        let sig = kp.sign(&digest);

        assert!(!kp.public_key().verify(&hash_bytes("test", b"hellp"), &sig));
        assert!(!other.public_key().verify(&digest, &sig));

        let mut tampered = sig;
        tampered.0[10] ^= 1;
        assert!(!kp.public_key().verify(&digest, &tampered));
        assert!(!kp.public_key().verify(&digest, &Signature::ZERO));
    }

    #[test]
    fn domains_separate_hashes() {
        assert_ne!(hash_bytes("a", b"x"), hash_bytes("b", b"x"));
        assert_eq!(hash_bytes("a", b"x"), hash_bytes("a", b"x"));
    }

    #[test]
    fn hash_borsh_matches_hash_of_encoding() {
        let value = (42u64, [1u8; 4]);
        let bytes = borsh::to_vec(&value).unwrap();
        assert_eq!(hash_borsh("d", &value), hash_bytes("d", &bytes));
    }

    #[test]
    fn keypair_round_trips_through_secret_bytes() {
        let kp = Keypair::generate();
        let restored = Keypair::from_secret_bytes(&kp.secret_bytes());
        assert_eq!(kp.public_key(), restored.public_key());
        assert_eq!(kp.address(), kp.public_key().address());
    }

    #[test]
    fn debug_never_shows_the_secret() {
        let kp = Keypair::from_secret_bytes(&[0x42; 32]);
        let debug = format!("{kp:?}");
        assert!(debug.contains("redacted"));
        assert!(!debug.contains(&hex::encode([0x42u8; 32])));
    }
}
