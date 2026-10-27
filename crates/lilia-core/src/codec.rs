use crate::Result;

/// Extension point for at-rest value encoding.
pub trait ValueCodec: Send + Sync + std::fmt::Debug {
    fn name(&self) -> &str;

    /// Encode plaintext before persistence.
    ///
    /// # Errors
    /// Returns a codec-specific error when encoding cannot be completed.
    fn encode(&self, plaintext: &[u8]) -> Result<Vec<u8>>;

    /// Decode a persisted value.
    ///
    /// # Errors
    /// Returns a codec-specific error when decoding or authentication fails.
    fn decode(&self, encoded: &[u8]) -> Result<Vec<u8>>;
}

#[derive(Debug, Default)]
pub struct IdentityCodec;

impl ValueCodec for IdentityCodec {
    #[allow(clippy::unnecessary_literal_bound)]
    fn name(&self) -> &str {
        "identity"
    }

    fn encode(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        Ok(plaintext.to_vec())
    }

    fn decode(&self, encoded: &[u8]) -> Result<Vec<u8>> {
        Ok(encoded.to_vec())
    }
}

/// AES-256-GCM AEAD at-rest encryption codec.
///
/// Encoded payload layout: `[12-byte random nonce][ciphertext with 16-byte tag]`.
#[derive(Clone)]
pub struct Aes256GcmCodec {
    cipher: aes_gcm::Aes256Gcm,
}

impl std::fmt::Debug for Aes256GcmCodec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Aes256GcmCodec")
            .field("algorithm", &"aes-256-gcm")
            .finish()
    }
}

impl Aes256GcmCodec {
    const NONCE_LEN: usize = 12;
    const TAG_LEN: usize = 16;
    const MIN_ENCODED_LEN: usize = Self::NONCE_LEN + Self::TAG_LEN;

    #[must_use]
    pub fn from_key(key: [u8; 32]) -> Self {
        use aes_gcm::KeyInit;
        Self {
            cipher: aes_gcm::Aes256Gcm::new(aes_gcm::Key::<aes_gcm::Aes256Gcm>::from_slice(&key)),
        }
    }

    #[must_use]
    pub fn from_passphrase(passphrase: &str) -> Self {
        use sha2::{Digest, Sha256};
        let hash = Sha256::digest(passphrase.as_bytes());
        Self::from_key(hash.into())
    }
}

impl ValueCodec for Aes256GcmCodec {
    #[allow(clippy::unnecessary_literal_bound)]
    fn name(&self) -> &str {
        "aes-256-gcm"
    }

    fn encode(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        use crate::{ErrorCode, LiliaError};
        use aes_gcm::aead::Aead;

        let mut nonce_bytes = [0u8; Self::NONCE_LEN];
        rand::Rng::fill(&mut rand::rng(), &mut nonce_bytes);
        let nonce = aes_gcm::Nonce::from_slice(&nonce_bytes);

        let ciphertext = self.cipher.encrypt(nonce, plaintext).map_err(|e| {
            LiliaError::new(
                ErrorCode::Storage,
                format!("AES-256-GCM encryption failed: {e}"),
                false,
            )
        })?;

        let mut encoded = Vec::with_capacity(Self::NONCE_LEN + ciphertext.len());
        encoded.extend_from_slice(&nonce_bytes);
        encoded.extend_from_slice(&ciphertext);
        Ok(encoded)
    }

    fn decode(&self, encoded: &[u8]) -> Result<Vec<u8>> {
        use crate::{ErrorCode, LiliaError};
        use aes_gcm::aead::Aead;

        if encoded.len() < Self::MIN_ENCODED_LEN {
            return Err(LiliaError::new(
                ErrorCode::Corrupt,
                "encoded buffer too short for AES-256-GCM payload",
                false,
            ));
        }

        let nonce = aes_gcm::Nonce::from_slice(&encoded[..Self::NONCE_LEN]);
        let ciphertext = &encoded[Self::NONCE_LEN..];

        self.cipher.decrypt(nonce, ciphertext).map_err(|_| {
            LiliaError::new(
                ErrorCode::Corrupt,
                "AES-256-GCM decryption / authentication tag verification failed",
                false,
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identity_codec_roundtrip() {
        let codec = IdentityCodec;
        let data = b"hello liliadb";
        let encoded = codec.encode(data).unwrap();
        assert_eq!(encoded, data);
        let decoded = codec.decode(&encoded).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_aes_codec_roundtrip() {
        let codec = Aes256GcmCodec::from_passphrase("test-secret-passphrase");
        let plaintext = b"multi-model encrypted state at rest with zero data leaks";
        let encoded = codec.encode(plaintext).unwrap();
        assert_ne!(&encoded, plaintext);
        assert!(encoded.len() >= plaintext.len() + 28);

        let decoded = codec.decode(&encoded).unwrap();
        assert_eq!(&decoded, plaintext);
    }

    #[test]
    fn test_aes_codec_tampering_rejected() {
        let codec = Aes256GcmCodec::from_passphrase("test-secret-passphrase");
        let plaintext = b"tamper test payload";
        let mut encoded = codec.encode(plaintext).unwrap();
        // Flip one byte in ciphertext
        let len = encoded.len();
        encoded[len - 1] ^= 0xFF;
        let result = codec.decode(&encoded);
        assert!(result.is_err());
    }
}
