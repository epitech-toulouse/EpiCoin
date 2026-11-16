//! Passwords and random tokens.

use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use argon2::Argon2;

pub const MIN_PASSWORD_LEN: usize = 10;
pub const MAX_PASSWORD_LEN: usize = 128;

/// Checks the password policy. Returns a message for the user when it is not met.
pub fn check_password_policy(password: &str) -> Result<(), &'static str> {
    let length = password.chars().count();
    if length < MIN_PASSWORD_LEN {
        return Err("Le mot de passe doit contenir au moins 10 caractères.");
    }
    if length > MAX_PASSWORD_LEN {
        return Err("Le mot de passe ne peut pas dépasser 128 caractères.");
    }
    Ok(())
}

/// Hashes a password with Argon2id (random salt), as a PHC string.
pub fn hash_password(password: &str) -> Result<String, argon2::password_hash::Error> {
    Ok(Argon2::default()
        .hash_password(password.as_bytes())?
        .to_string())
}

/// Verifies a password against a PHC string. Any malformed hash counts as a mismatch.
pub fn verify_password(password: &str, phc: &str) -> bool {
    match PasswordHash::new(phc) {
        Ok(parsed) => Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

/// A random 256-bit token, hex-encoded: session cookies, invitation and reset links.
pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("operating system random source unavailable");
    hex::encode(bytes)
}

/// What is stored in the database instead of a token: a leak of the table does not give
/// usable sessions or links. Tokens are 256-bit random values, so a fast hash is enough.
pub fn token_hash(token: &str) -> Vec<u8> {
    blake3::derive_key("epicoin v1 auth token", token.as_bytes()).to_vec()
}

/// Compares two strings without leaking where they differ.
pub fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_hash_verifies_only_the_right_password() {
        let phc = hash_password("correct horse battery").unwrap();
        assert!(phc.starts_with("$argon2id$"));
        assert!(verify_password("correct horse battery", &phc));
        assert!(!verify_password("correct horse batterz", &phc));
        assert!(!verify_password("", &phc));
        assert!(!verify_password("anything", "not a phc string"));
    }

    #[test]
    fn same_password_hashes_differently() {
        assert_ne!(
            hash_password("same password").unwrap(),
            hash_password("same password").unwrap()
        );
    }

    #[test]
    fn password_policy() {
        assert!(check_password_policy("123456789").is_err());
        assert!(check_password_policy("1234567890").is_ok());
        assert!(check_password_policy(&"x".repeat(128)).is_ok());
        assert!(check_password_policy(&"x".repeat(129)).is_err());
        // Counted in characters, not bytes.
        assert!(check_password_policy("éééééééééé").is_ok());
    }

    #[test]
    fn tokens_are_random_and_hashed_deterministically() {
        let a = random_token();
        let b = random_token();
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
        assert_eq!(token_hash(&a), token_hash(&a));
        assert_ne!(token_hash(&a), token_hash(&b));
        assert_ne!(token_hash(&a), a.as_bytes());
    }

    #[test]
    fn constant_time_comparison() {
        assert!(constant_time_eq("abc", "abc"));
        assert!(!constant_time_eq("abc", "abd"));
        assert!(!constant_time_eq("abc", "abcd"));
        assert!(constant_time_eq("", ""));
    }
}
