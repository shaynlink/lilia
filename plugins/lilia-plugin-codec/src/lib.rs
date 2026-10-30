#![allow(unsafe_code)]

use std::sync::RwLock;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::Engine;
use lilia_plugin_api::{Capability, CodecDescriptorV1, PluginDescriptorV1};
use sha2::{Digest, Sha256};

static PLUGIN_NAME: &[u8] = b"lilia-codec\0";
static CODEC_NAME: &[u8] = b"aes-256-gcm\0";

static CIPHER: RwLock<Option<Aes256Gcm>> = RwLock::new(None);

fn get_or_init_cipher() -> Option<Aes256Gcm> {
    if let Ok(guard) = CIPHER.read() {
        if let Some(ref cipher) = *guard {
            return Some(cipher.clone());
        }
    }

    let key_bytes = if let Ok(key_str) = std::env::var("LILIA_CODEC_KEY") {
        if key_str.len() == 64 {
            hex::decode(&key_str).ok()?
        } else if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(&key_str) {
            if decoded.len() == 32 {
                decoded
            } else {
                key_str.into_bytes()
            }
        } else {
            key_str.into_bytes()
        }
    } else {
        // Deterministic development fallback key
        Sha256::digest(b"liliadb-dev-codec-key").to_vec()
    };

    if key_bytes.len() != 32 {
        return None;
    }

    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key_bytes));
    if let Ok(mut guard) = CIPHER.write() {
        *guard = Some(cipher.clone());
    }
    Some(cipher)
}

/// Initialize the AES-256-GCM cipher with a 256-bit (32-byte) key.
///
/// # Safety
/// The `key` pointer must be non-null and valid for reads of `key_len` bytes.
pub unsafe extern "C" fn codec_init(key: *const u8, key_len: usize) -> i32 {
    if key.is_null() || key_len != 32 {
        return -1;
    }
    let key_slice = unsafe { std::slice::from_raw_parts(key, key_len) };
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key_slice));
    if let Ok(mut guard) = CIPHER.write() {
        *guard = Some(cipher);
        0
    } else {
        -1
    }
}

/// Encrypt a plaintext buffer using AES-256-GCM with a random 96-bit nonce.
///
/// # Safety
/// - If `input_len > 0`, `input` must be valid for reads of `input_len` bytes.
/// - `out_len` must be a valid, aligned, non-null pointer to `usize`.
/// - If `out_cap > 0`, `out` must be valid for writes of `out_cap` bytes.
pub unsafe extern "C" fn codec_encode(
    input: *const u8,
    input_len: usize,
    out: *mut u8,
    out_cap: usize,
    out_len: *mut usize,
) -> i32 {
    if (input.is_null() && input_len > 0) || out_len.is_null() {
        return -1;
    }
    let Some(cipher) = get_or_init_cipher() else {
        return -1;
    };
    let in_slice = if input_len == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(input, input_len) }
    };

    let nonce_bytes: [u8; 12] = rand::random();
    let nonce = Nonce::from_slice(&nonce_bytes);

    let Ok(ciphertext) = cipher.encrypt(nonce, in_slice) else {
        return -1;
    };

    let total_len = 12 + ciphertext.len();
    unsafe {
        *out_len = total_len;
    }
    if out.is_null() || out_cap < total_len {
        return 1;
    }

    unsafe {
        std::ptr::copy_nonoverlapping(nonce_bytes.as_ptr(), out, 12);
        std::ptr::copy_nonoverlapping(ciphertext.as_ptr(), out.add(12), ciphertext.len());
    }
    0
}

/// Decrypt an AES-256-GCM ciphertext buffer prefixed by a 96-bit nonce.
///
/// # Safety
/// - `input` must be valid for reads of `input_len` bytes.
/// - `out_len` must be a valid, aligned, non-null pointer to `usize`.
/// - If `out_cap > 0`, `out` must be valid for writes of `out_cap` bytes.
pub unsafe extern "C" fn codec_decode(
    input: *const u8,
    input_len: usize,
    out: *mut u8,
    out_cap: usize,
    out_len: *mut usize,
) -> i32 {
    if input.is_null() || out_len.is_null() || input_len < 28 {
        return -1;
    }
    let in_slice = unsafe { std::slice::from_raw_parts(input, input_len) };
    let nonce = Nonce::from_slice(&in_slice[..12]);
    let ciphertext = &in_slice[12..];

    let Some(cipher) = get_or_init_cipher() else {
        return -1;
    };

    let Ok(plaintext) = cipher.decrypt(nonce, ciphertext) else {
        return -1;
    };

    unsafe {
        *out_len = plaintext.len();
    }
    if out.is_null() || out_cap < plaintext.len() {
        return 1;
    }

    unsafe {
        std::ptr::copy_nonoverlapping(plaintext.as_ptr(), out, plaintext.len());
    }
    0
}

static CODEC_DESCRIPTOR: CodecDescriptorV1 = CodecDescriptorV1 {
    name: CODEC_NAME.as_ptr().cast(),
    init: Some(codec_init),
    encode: Some(codec_encode),
    decode: Some(codec_decode),
};

#[no_mangle]
pub extern "C" fn lilia_plugin_v1() -> PluginDescriptorV1 {
    PluginDescriptorV1::with_codec(
        PLUGIN_NAME.as_ptr().cast(),
        Capability::Codec.bit(),
        &raw const CODEC_DESCRIPTOR,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_declares_codec_capability() {
        let descriptor = lilia_plugin_v1();
        assert_eq!(descriptor.capabilities, Capability::Codec.bit());
        assert!(!descriptor.codec.is_null());
        let codec = unsafe { &*descriptor.codec };
        assert!(codec.encode.is_some());
        assert!(codec.decode.is_some());
    }

    #[test]
    fn aes_gcm_encode_decode_roundtrip_and_tamper_detection() {
        let key = [42u8; 32];
        assert_eq!(unsafe { codec_init(key.as_ptr(), key.len()) }, 0);

        let plaintext = b"LiliaDB highly secret at-rest payload";
        let mut encoded = vec![0u8; 256];
        let mut enc_len = 0usize;

        let res = unsafe {
            codec_encode(
                plaintext.as_ptr(),
                plaintext.len(),
                encoded.as_mut_ptr(),
                encoded.len(),
                &raw mut enc_len,
            )
        };
        assert_eq!(res, 0);
        assert!(enc_len >= plaintext.len() + 28);
        encoded.truncate(enc_len);

        // Ciphertext should not contain plaintext
        assert!(!encoded.windows(plaintext.len()).any(|w| w == plaintext));

        // Decrypt
        let mut decoded = vec![0u8; 256];
        let mut dec_len = 0usize;
        let res_dec = unsafe {
            codec_decode(
                encoded.as_ptr(),
                encoded.len(),
                decoded.as_mut_ptr(),
                decoded.len(),
                &raw mut dec_len,
            )
        };
        assert_eq!(res_dec, 0);
        decoded.truncate(dec_len);
        assert_eq!(&decoded, plaintext);

        // Tampering with ciphertext should fail authentication
        let mut tampered = encoded.clone();
        tampered[15] ^= 0xFF;
        let res_tampered = unsafe {
            codec_decode(
                tampered.as_ptr(),
                tampered.len(),
                decoded.as_mut_ptr(),
                decoded.len(),
                &raw mut dec_len,
            )
        };
        assert_eq!(res_tampered, -1);
    }

    #[test]
    fn sqlite_storage_with_aes_codec_integration() {
        use lilia_core::{BatchOperation, ValueCodec};
        use lilia_storage_sqlite::{Database, DatabaseOptions};
        use serde_json::json;
        use std::sync::Arc;

        #[derive(Debug)]
        struct AesCodecWrapper;
        impl ValueCodec for AesCodecWrapper {
            #[allow(clippy::unnecessary_literal_bound)]
            fn name(&self) -> &str {
                "aes-256-gcm"
            }
            fn encode(&self, plaintext: &[u8]) -> lilia_core::Result<Vec<u8>> {
                let mut out = vec![0u8; plaintext.len() + 64];
                let mut out_len = 0;
                let res = unsafe {
                    codec_encode(
                        plaintext.as_ptr(),
                        plaintext.len(),
                        out.as_mut_ptr(),
                        out.len(),
                        &raw mut out_len,
                    )
                };
                assert_eq!(res, 0);
                out.truncate(out_len);
                Ok(out)
            }
            fn decode(&self, ciphertext: &[u8]) -> lilia_core::Result<Vec<u8>> {
                let mut out = vec![0u8; ciphertext.len()];
                let mut out_len = 0;
                let res = unsafe {
                    codec_decode(
                        ciphertext.as_ptr(),
                        ciphertext.len(),
                        out.as_mut_ptr(),
                        out.len(),
                        &raw mut out_len,
                    )
                };
                if res != 0 {
                    return Err(lilia_core::LiliaError::new(
                        lilia_core::ErrorCode::Corrupt,
                        "decryption failed",
                        false,
                    ));
                }
                out.truncate(out_len);
                Ok(out)
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("encrypted.lilia");
        let codec = Arc::new(AesCodecWrapper);
        let options = DatabaseOptions::durable(&db_path).with_codec(codec);
        let db = Database::open(options).unwrap();

        db.batch(&[
            BatchOperation::KvSet {
                namespace: "vault".into(),
                key: b"secret_key".to_vec(),
                value: b"top_secret_aes_payload".to_vec(),
                if_version: None,
                expires_at_ms: None,
            },
            BatchOperation::JsonPut {
                space: "identities".into(),
                id: "user42".into(),
                value: json!({"role": "admin", "token": "xyz"}),
                if_version: None,
            },
        ])
        .unwrap();

        let kv_entry = db.kv_get("vault", b"secret_key").unwrap().unwrap();
        assert_eq!(kv_entry.value, b"top_secret_aes_payload");

        let json_entry = db.json_get("identities", "user42").unwrap().unwrap();
        assert_eq!(json_entry.value["role"], "admin");
        assert_eq!(json_entry.value["token"], "xyz");

        drop(db);

        // Verify SQLite raw database contains encrypted bytes and envelopes
        let raw_conn = rusqlite::Connection::open(&db_path).unwrap();
        let raw_kv: Vec<u8> = raw_conn
            .query_row(
                "SELECT value FROM _lilia_kv WHERE namespace = 'vault' AND key = ?1",
                [b"secret_key"],
                |row| row.get(0),
            )
            .unwrap();
        assert_ne!(raw_kv, b"top_secret_aes_payload");

        let raw_json: String = raw_conn
            .query_row(
                "SELECT value FROM _lilia_json WHERE space = 'identities' AND id = 'user42'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!raw_json.contains("top_secret_aes_payload"));
        assert!(!raw_json.contains("admin"));
        assert!(raw_json.contains("$codec"));
        assert!(raw_json.contains("$data"));
    }
}
