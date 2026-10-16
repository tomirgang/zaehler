//! Geheime Tokens (Einladungen, später API-Tokens). Gespeichert wird nur der
//! SHA-256; das Token selbst erscheint genau einmal in der Antwort.

use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Erzeugtes Token mit seinem Hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Secret {
    /// 32 Zufallsbytes, hexadezimal (64 Zeichen)
    pub token: String,
    pub hash: [u8; 32],
}

/// 32 Bytes aus dem Zufallsgenerator des Betriebssystems (zwei UUIDv4 über
/// `getrandom`, wie das CSRF-Token des Web-Adapters).
#[must_use]
pub fn generate() -> Secret {
    let mut bytes = [0u8; 32];
    bytes[..16].copy_from_slice(Uuid::new_v4().as_bytes());
    bytes[16..].copy_from_slice(Uuid::new_v4().as_bytes());
    let token = to_hex(&bytes);
    let hash = hash(&token);
    Secret { token, hash }
}

/// Hexadezimale Darstellung (Kleinbuchstaben).
#[must_use]
pub fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// Hash aus seiner hexadezimalen Darstellung.
#[must_use]
pub fn hash_from_hex(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 || !hex.is_ascii() {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

/// SHA-256 eines Tokens.
#[must_use]
pub fn hash(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    // @test SPEC_TEN_INVITES
    #[test]
    fn tokens_are_random_and_hashed() {
        let a = generate();
        let b = generate();
        assert_eq!(a.token.len(), 64);
        assert_ne!(a.token, b.token);
        assert_eq!(hash(&a.token), a.hash);
        assert_ne!(a.hash, b.hash);
        assert_eq!(hash_from_hex(&to_hex(&a.hash)), Some(a.hash));
        assert_eq!(hash_from_hex("zz"), None);
    }
}
