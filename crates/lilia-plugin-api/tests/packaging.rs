mod common;

use std::fs;

use lilia_plugin_api::{
    discover_installed, find_installed, generate_keypair, install_verified, official_release_key,
    official_release_signing_key, pack_plugin, sign_package, verify_package, Capability,
    PackOptions,
};

const LIBRARY_CONTENT: &[u8] = b"lilia native binary test content";

#[test]
fn generates_valid_ed25519_keypair() {
    let (sk1, vk1) = generate_keypair();
    let (sk2, vk2) = generate_keypair();

    assert_eq!(sk1.verifying_key(), vk1);
    assert_eq!(sk2.verifying_key(), vk2);
    // Two random generations must not produce the same key
    assert_ne!(vk1.as_bytes(), vk2.as_bytes());
}

#[test]
fn pack_and_sign_lifecycle_end_to_end() {
    let temp = tempfile::tempdir().expect("tempdir");
    let lib_name = common::library_name();
    let raw_lib_path = temp.path().join(lib_name);
    fs::write(&raw_lib_path, LIBRARY_CONTENT).expect("write library");

    let package_dir = temp.path().join("pkg-out");
    let options = PackOptions {
        name: "test-plugin".into(),
        version: "1.2.3".into(),
        capabilities: vec![Capability::Codec],
        engine_requirement: None,
    };

    let packed_path = pack_plugin(&raw_lib_path, &package_dir, &options).expect("pack plugin");
    assert_eq!(packed_path, package_dir);

    // Verify manifest.json exists and is valid
    assert!(package_dir.join("manifest.json").exists());
    assert!(package_dir.join(lib_name).exists());
    // Signature not yet present
    assert!(!package_dir.join("manifest.sig").exists());

    // Generate signer key and sign
    let (signing_key, verifying_key) = generate_keypair();
    let sig_b64 = sign_package(&package_dir, &signing_key).expect("sign package");
    assert!(!sig_b64.is_empty());
    assert!(package_dir.join("manifest.sig").exists());

    // Verification must now succeed
    let manifest = verify_package(&package_dir, &[verifying_key], false).expect("verify");
    assert_eq!(manifest.name, "test-plugin");
    assert_eq!(manifest.version, "1.2.3");
    assert_eq!(manifest.capabilities, vec![Capability::Codec]);
}

#[test]
fn official_release_key_signs_and_verifies() {
    let temp = tempfile::tempdir().expect("tempdir");
    let lib_name = common::library_name();
    let raw_lib_path = temp.path().join(lib_name);
    fs::write(&raw_lib_path, LIBRARY_CONTENT).expect("write library");

    let package_dir = temp.path().join("official-pkg");
    let options = PackOptions {
        name: "lilia-plugin-official".into(),
        version: "0.1.0-alpha.1".into(),
        capabilities: vec![Capability::Sql],
        engine_requirement: None,
    };

    pack_plugin(&raw_lib_path, &package_dir, &options).expect("pack");
    let official_sk = official_release_signing_key();
    let official_vk = official_release_key();

    sign_package(&package_dir, &official_sk).expect("sign with official key");

    // Verification against official_vk succeeds
    let manifest = verify_package(&package_dir, &[official_vk], false).expect("verify official");
    assert_eq!(manifest.name, "lilia-plugin-official");
}

#[test]
fn install_force_flag_replaces_existing_installation() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("installed_root");

    let (signing_key, verifying_key) = generate_keypair();
    let lib_name = common::library_name();
    let raw_lib_path = temp.path().join(lib_name);
    fs::write(&raw_lib_path, LIBRARY_CONTENT).expect("write library");

    let package_dir = temp.path().join("my-package");
    let options = PackOptions {
        name: "my-tool".into(),
        version: "2.0.0".into(),
        capabilities: vec![Capability::Kv],
        engine_requirement: None,
    };
    pack_plugin(&raw_lib_path, &package_dir, &options).expect("pack");
    sign_package(&package_dir, &signing_key).expect("sign");

    // First install succeeds
    let published = install_verified(&package_dir, &root, &[verifying_key], false, false)
        .expect("first install");
    assert!(published.exists());

    // Second install without force fails
    let err = install_verified(&package_dir, &root, &[verifying_key], false, false)
        .expect_err("should reject overwrite");
    assert!(err.to_string().contains("already installed"));

    // Install with force = true succeeds
    let reinstalled = install_verified(&package_dir, &root, &[verifying_key], false, true)
        .expect("force install");
    assert!(reinstalled.exists());
}

#[test]
fn discover_and_find_installed_plugins() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("plugin_root");

    let (signing_key, verifying_key) = generate_keypair();
    let lib_name = common::library_name();
    let raw_lib_path = temp.path().join(lib_name);
    fs::write(&raw_lib_path, LIBRARY_CONTENT).expect("write library");

    // Package and install plugin 1: lilia-plugin-codec
    let pkg1 = temp.path().join("pkg1");
    pack_plugin(
        &raw_lib_path,
        &pkg1,
        &PackOptions {
            name: "lilia-plugin-codec".into(),
            version: "1.0.0".into(),
            capabilities: vec![Capability::Codec],
            engine_requirement: None,
        },
    )
    .expect("pack 1");
    sign_package(&pkg1, &signing_key).expect("sign 1");
    install_verified(&pkg1, &root, &[verifying_key], false, false).expect("install 1");

    // Package and install plugin 2: lilia-plugin-sql
    let pkg2 = temp.path().join("pkg2");
    pack_plugin(
        &raw_lib_path,
        &pkg2,
        &PackOptions {
            name: "lilia-plugin-sql".into(),
            version: "1.0.0".into(),
            capabilities: vec![Capability::Sql],
            engine_requirement: None,
        },
    )
    .expect("pack 2");
    sign_package(&pkg2, &signing_key).expect("sign 2");
    install_verified(&pkg2, &root, &[verifying_key], false, false).expect("install 2");

    // Test discover_installed
    let installed = discover_installed(&root).expect("discover");
    assert_eq!(installed.len(), 2);
    assert_eq!(installed[0].name, "lilia-plugin-codec");
    assert_eq!(installed[1].name, "lilia-plugin-sql");

    // Test find_installed:
    // 1. By exact name
    let found_name = find_installed(&root, "lilia-plugin-codec")
        .expect("find")
        .unwrap();
    assert_eq!(found_name.name, "lilia-plugin-codec");

    // 2. By short name (without lilia-plugin-)
    let found_short = find_installed(&root, "codec").expect("find short").unwrap();
    assert_eq!(found_short.name, "lilia-plugin-codec");

    // 3. By capability
    let found_cap = find_installed(&root, "sql").expect("find cap").unwrap();
    assert_eq!(found_cap.name, "lilia-plugin-sql");

    // 4. Non-existent returns None
    assert!(find_installed(&root, "non_existent")
        .expect("find none")
        .is_none());
}
