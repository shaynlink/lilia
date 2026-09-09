use std::fs;

use base64::Engine;
use ed25519_dalek::{Signer, SigningKey};
use lilia_plugin_api::{verify_package, Capability, PluginManifest, ABI_MAJOR, ABI_MINOR};
use sha2::{Digest, Sha256};

#[test]
fn verifies_signature_and_rejects_tampered_library() {
    let directory = tempfile::tempdir().expect("temporary package");
    let library = if cfg!(target_os = "windows") {
        "plugin.dll"
    } else {
        "plugin.so"
    };
    let bytes = b"trusted native plugin";
    fs::write(directory.path().join(library), bytes).expect("write library");
    let manifest = PluginManifest {
        name: "trusted-plugin".into(),
        version: "1.0.0".into(),
        abi_major: ABI_MAJOR,
        abi_minor: ABI_MINOR,
        engine_requirement: "^0.1".into(),
        target: format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS),
        library: library.into(),
        library_sha256: hex::encode(Sha256::digest(bytes)),
        capabilities: vec![Capability::Kv],
    };
    fs::write(
        directory.path().join("manifest.json"),
        manifest.canonical_bytes().expect("manifest JSON"),
    )
    .expect("write manifest");
    let signing_key = SigningKey::from_bytes(&[7; 32]);
    let signature = signing_key.sign(&manifest.canonical_bytes().expect("manifest JSON"));
    fs::write(
        directory.path().join("manifest.sig"),
        base64::engine::general_purpose::STANDARD.encode(signature.to_bytes()),
    )
    .expect("write signature");

    verify_package(directory.path(), &[signing_key.verifying_key()], false)
        .expect("valid signed package");
    fs::write(directory.path().join(library), b"tampered").expect("tamper library");
    let error = verify_package(directory.path(), &[signing_key.verifying_key()], false)
        .expect_err("tampering must fail");
    assert!(error.to_string().contains("checksum"));
}
