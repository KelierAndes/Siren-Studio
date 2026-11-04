//! Group-password crypto for the mesh (F6 / PLAN Phase 3).
//!
//! Design (review pending per the schedule):
//! * one group password, pre-shared with every node (WebUI / CLI flag);
//! * the AES-less PSK is `SHA-256(password)` — 32 bytes, exactly the
//!   ChaCha20-Poly1305 key size;
//! * every Wi-Fi datagram is sealed as
//!   `b"SE" | version(1) | nonce(12) | AEAD(payload)` where the payload is
//!   the complete plain datagram (header + audio) and the AAD is the magic
//!   plus version. 31 B of overhead keeps a 5 ms s16 frame under one MTU;
//! * the Bluetooth carrier is NOT encrypted in this pass: its compact v3
//!   header carries ~62 B payloads and 28 B of AEAD would double the air
//!   time. The tunnel runs over authenticated-encryption-free RFCOMM and is
//!   documented as the weaker carrier until it gets its own budget;
//! * discovery: the advertiser adds `sig = hex(HMAC-SHA256(psk,
//!   id|bit|bt|channel))`. A node holding the password ignores election
//!   credentials (bit/bt/channel) from peers whose signature does not
//!   verify, so a passwordless attacker cannot take the hub election —
//!   and cannot inject audio either, because every frame fails AEAD.

use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    ChaCha20Poly1305, Key, Nonce,
};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

use crate::CoreError;

pub const CRYPTO_MAGIC: [u8; 2] = *b"SE";
pub const CRYPTO_VERSION: u8 = 1;
pub const NONCE_LEN: usize = 12;
pub const TAG_LEN: usize = 16;
/// `SE` + version + nonce + tag.
pub const CRYPTO_OVERHEAD: usize = 2 + 1 + NONCE_LEN + TAG_LEN;

type HmacSha256 = Hmac<Sha256>;

/// Derived pre-shared key. Not printed in logs or snapshots.
#[derive(Clone)]
pub struct MeshKey {
    cipher: ChaCha20Poly1305,
    hmac_key: Vec<u8>,
}

impl std::fmt::Debug for MeshKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MeshKey(***)")
    }
}

impl MeshKey {
    /// Derive the key material from the group password.
    pub fn from_password(password: &str) -> Self {
        let digest = Sha256::digest(password.as_bytes());
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&digest));
        Self {
            cipher,
            hmac_key: digest.to_vec(),
        }
    }

    /// Seal a plain datagram. Random 96-bit nonce: fine for the session
    /// lifetime at mesh packet rates, and keeps senders stateless.
    pub fn seal(&self, plain: &[u8]) -> Result<Vec<u8>, CoreError> {
        let mut nonce_bytes = [0u8; NONCE_LEN];
        fill_random(&mut nonce_bytes)?;
        let nonce = Nonce::from_slice(&nonce_bytes);
        let sealed = self
            .cipher
            .encrypt(nonce, Payload { msg: plain, aad: &aad_bytes() })
            .map_err(|_| CoreError::Backend("AEAD seal failed".into()))?;
        let mut out = Vec::with_capacity(2 + 1 + NONCE_LEN + sealed.len());
        out.extend_from_slice(&CRYPTO_MAGIC);
        out.push(CRYPTO_VERSION);
        out.extend_from_slice(&nonce_bytes);
        out.extend_from_slice(&sealed);
        Ok(out)
    }

    /// Open a sealed datagram; `None` on any failure (wrong key, tampering).
    pub fn open(&self, sealed: &[u8]) -> Option<Vec<u8>> {
        if sealed.len() < 2 + 1 + NONCE_LEN + TAG_LEN {
            return None;
        }
        if sealed[..2] != CRYPTO_MAGIC || sealed[2] != CRYPTO_VERSION {
            return None;
        }
        let nonce = Nonce::from_slice(&sealed[3..3 + NONCE_LEN]);
        self.cipher
            .decrypt(nonce, Payload { msg: &sealed[3 + NONCE_LEN..], aad: &aad_bytes() })
            .ok()
    }

    /// Election credential signature: binds the advertised identity fields
    /// so a passwordless node cannot win the hub election.
    pub fn sign_election(&self, id: &str, bit: Option<u8>, bt: Option<&str>, channel: Option<u8>) -> String {
        let mut mac = HmacSha256::new_from_slice(&self.hmac_key).expect("hmac key");
        mac.update(id.as_bytes());
        mac.update(&[bit.unwrap_or(255)]);
        mac.update(bt.unwrap_or("").as_bytes());
        mac.update(&[channel.unwrap_or(0)]);
        let tag = mac.finalize().into_bytes();
        let mut hex = String::with_capacity(tag.len() * 2);
        for byte in tag.iter() {
            hex.push_str(&format!("{byte:02x}"));
        }
        hex
    }

    pub fn verify_election(
        &self,
        id: &str,
        bit: Option<u8>,
        bt: Option<&str>,
        channel: Option<u8>,
        sig: &str,
    ) -> bool {
        let expected = self.sign_election(id, bit, bt, channel);
        // Constant-time-ish: length check first, then OR over bytes.
        if expected.len() != sig.len() {
            return false;
        }
        let mut diff = 0u8;
        for (a, b) in expected.bytes().zip(sig.bytes()) {
            diff |= a ^ b;
        }
        diff == 0
    }
}

/// The AAD is fixed per protocol version; both sides derive the same bytes.
fn aad_bytes() -> [u8; 3] {
    [CRYPTO_MAGIC[0], CRYPTO_MAGIC[1], CRYPTO_VERSION]
}

/// Fill `buf` with OS randomness.
fn fill_random(buf: &mut [u8]) -> Result<(), CoreError> {
    getrandom::fill(buf).map_err(|e| CoreError::Backend(format!("entropy: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_open_round_trip() {
        let key = MeshKey::from_password("group-pass");
        let plain = b"SR-something-header-and-audio-payload";
        let sealed = key.seal(plain).unwrap();
        assert_eq!(sealed[..2], CRYPTO_MAGIC);
        assert_eq!(sealed.len(), plain.len() + CRYPTO_OVERHEAD);
        let opened = key.open(&sealed).unwrap();
        assert_eq!(opened, plain);
    }

    #[test]
    fn wrong_password_cannot_open() {
        let key = MeshKey::from_password("right");
        let wrong = MeshKey::from_password("wrong");
        let sealed = key.seal(b"payload").unwrap();
        assert!(wrong.open(&sealed).is_none());
    }

    #[test]
    fn tampered_ciphertext_is_rejected() {
        let key = MeshKey::from_password("pass");
        let mut sealed = key.seal(b"payload data").unwrap();
        let last = sealed.len() - 1;
        sealed[last] ^= 0x01;
        assert!(key.open(&sealed).is_none());
    }

    #[test]
    fn election_sig_binds_fields() {
        let key = MeshKey::from_password("pass");
        let sig = key.sign_election("abcdef0123456789", Some(3), Some("AA:BB"), Some(16));
        assert!(key.verify_election("abcdef0123456789", Some(3), Some("AA:BB"), Some(16), &sig));
        assert!(!key.verify_election("0000000000000000", Some(3), Some("AA:BB"), Some(16), &sig));
        assert!(!key.verify_election("abcdef0123456789", Some(4), Some("AA:BB"), Some(16), &sig));
    }
}
