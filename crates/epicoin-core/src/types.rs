//! Fixed-size byte types used everywhere on chain.

use std::fmt;
use std::str::FromStr;

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Amount in the smallest unit. 1 EPC = [`UNITS_PER_EPC`] units. Never a float.
pub type Amount = u64;

/// Number of smallest units in one EPC.
pub const UNITS_PER_EPC: Amount = 100;

/// Formats an amount of units as EPC with two decimals, e.g. `1250` → `"12.50"`.
pub fn format_epc(amount: Amount) -> String {
    format!("{}.{:02}", amount / UNITS_PER_EPC, amount % UNITS_PER_EPC)
}

/// Parses a decimal EPC amount (`"12"`, `"12.5"`, `"12.50"`) into units, exactly — no
/// floating point involved. Returns `None` for anything else, including more than two
/// decimals or a value that does not fit.
pub fn parse_epc(text: &str) -> Option<Amount> {
    let (whole, fraction) = match text.split_once('.') {
        Some((whole, fraction)) => (whole, fraction),
        None => (text, ""),
    };
    let all_digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    if whole.is_empty() || !all_digits(whole) || !all_digits(fraction) || fraction.len() > 2 {
        return None;
    }
    if text.contains('.') && fraction.is_empty() {
        return None;
    }
    let cents = match fraction.len() {
        0 => 0,
        1 => fraction.parse::<Amount>().ok()? * 10,
        _ => fraction.parse::<Amount>().ok()?,
    };
    whole
        .parse::<Amount>()
        .ok()?
        .checked_mul(UNITS_PER_EPC)?
        .checked_add(cents)
}

/// Error returned when parsing a hex-encoded fixed-size value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("expected {expected} hex-encoded bytes")]
pub struct ParseHexError {
    pub expected: usize,
}

macro_rules! bytes_newtype {
    ($(#[$meta:meta])* $name:ident, $len:expr) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, BorshSerialize, BorshDeserialize)]
        pub struct $name(pub [u8; $len]);

        impl $name {
            pub const LEN: usize = $len;
            pub const ZERO: Self = Self([0u8; $len]);

            pub fn as_bytes(&self) -> &[u8; $len] {
                &self.0
            }

            pub fn to_hex(&self) -> String {
                hex::encode(self.0)
            }

            pub fn from_hex(s: &str) -> Result<Self, ParseHexError> {
                let mut out = [0u8; $len];
                hex::decode_to_slice(s, &mut out).map_err(|_| ParseHexError { expected: $len })?;
                Ok(Self(out))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.to_hex())
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($name), self.to_hex())
            }
        }

        impl FromStr for $name {
            type Err = ParseHexError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Self::from_hex(s)
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(&self.to_hex())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let s = <String as Deserialize>::deserialize(deserializer)?;
                Self::from_hex(&s).map_err(serde::de::Error::custom)
            }
        }
    };
}

bytes_newtype!(
    /// A 32-byte BLAKE3 digest.
    Hash,
    32
);

bytes_newtype!(
    /// An account address: the BLAKE3 hash of an ed25519 public key.
    Address,
    32
);

bytes_newtype!(
    /// An ed25519 public key.
    PublicKey,
    32
);

bytes_newtype!(
    /// An ed25519 signature.
    Signature,
    64
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epc_formatting() {
        assert_eq!(format_epc(0), "0.00");
        assert_eq!(format_epc(5), "0.05");
        assert_eq!(format_epc(1_250), "12.50");
        assert_eq!(format_epc(100), "1.00");
    }

    #[test]
    fn epc_parsing_is_exact() {
        assert_eq!(parse_epc("12"), Some(1_200));
        assert_eq!(parse_epc("12.5"), Some(1_250));
        assert_eq!(parse_epc("12.50"), Some(1_250));
        assert_eq!(parse_epc("0.07"), Some(7));
        assert_eq!(parse_epc("0"), Some(0));
        for bad in [
            "", ".", "12.", ".5", "1.234", "-1", "+1", "1e3", "1,5", " 1", "abc",
        ] {
            assert_eq!(parse_epc(bad), None, "{bad:?} should not parse");
        }
        assert_eq!(parse_epc("184467440737095516.15"), Some(u64::MAX));
        assert_eq!(parse_epc("184467440737095516.16"), None);
        for amount in [0, 1, 99, 100, 101, 123_456_789] {
            assert_eq!(parse_epc(&format_epc(amount)), Some(amount));
        }
    }

    #[test]
    fn hex_round_trip() {
        let h = Hash([0xab; 32]);
        assert_eq!(h.to_hex().len(), 64);
        assert_eq!(Hash::from_hex(&h.to_hex()), Ok(h));
        assert_eq!(h.to_string().parse::<Hash>(), Ok(h));
    }

    #[test]
    fn hex_rejects_wrong_length_and_garbage() {
        assert!(Hash::from_hex("abcd").is_err());
        assert!(Hash::from_hex(&"zz".repeat(32)).is_err());
        assert!(Hash::from_hex(&"ab".repeat(33)).is_err());
    }

    #[test]
    fn json_is_a_hex_string() {
        let a = Address([1; 32]);
        let json = serde_json::to_string(&a).unwrap();
        assert_eq!(json, format!("\"{}\"", "01".repeat(32)));
        assert_eq!(serde_json::from_str::<Address>(&json).unwrap(), a);
    }

    #[test]
    fn borsh_is_raw_bytes() {
        let s = Signature([7; 64]);
        assert_eq!(borsh::to_vec(&s).unwrap(), vec![7u8; 64]);
    }
}
