use std::fs;
use std::path::Path;

use base64::Engine;
use ed25519_dalek::{Signer, SigningKey};
use lilia_plugin_api::{host_target, Capability, PluginManifest, ABI_MAJOR, ABI_MINOR};
use sha2::{Digest, Sha256};

pub fn library_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "plugin.dll"
    } else if cfg!(target_os = "macos") {
        "plugin.dylib"
    } else {
        "plugin.so"
    }
}

pub fn manifest(library_bytes: &[u8]) -> PluginManifest {
    PluginManifest {
        name: "trusted-plugin".into(),
        version: "1.0.0".into(),
        abi_major: ABI_MAJOR,
        abi_minor: ABI_MINOR,
        engine_requirement: ">=0.1.0-alpha.1,<0.2.0".into(),
        target: host_target().into(),
        library: library_name().into(),
        library_sha256: hex::encode(Sha256::digest(library_bytes)),
        capabilities: vec![Capability::Kv],
    }
}

pub fn write_signed_package(
    directory: &Path,
    manifest: &PluginManifest,
    library_bytes: &[u8],
    signing_key: &SigningKey,
) {
    fs::write(directory.join(&manifest.library), library_bytes).expect("write library");
    let manifest_bytes = manifest.canonical_bytes().expect("manifest JSON");
    fs::write(directory.join("manifest.json"), &manifest_bytes).expect("write manifest");
    let signature = signing_key.sign(&manifest_bytes);
    fs::write(
        directory.join("manifest.sig"),
        base64::engine::general_purpose::STANDARD.encode(signature.to_bytes()),
    )
    .expect("write signature");
}
