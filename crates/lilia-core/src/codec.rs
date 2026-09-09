use crate::Result;

/// Extension point for at-rest value encoding.
pub trait ValueCodec: Send + Sync + std::fmt::Debug {
    fn name(&self) -> &'static str;

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
    fn name(&self) -> &'static str {
        "identity"
    }

    fn encode(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        Ok(plaintext.to_vec())
    }

    fn decode(&self, encoded: &[u8]) -> Result<Vec<u8>> {
        Ok(encoded.to_vec())
    }
}
