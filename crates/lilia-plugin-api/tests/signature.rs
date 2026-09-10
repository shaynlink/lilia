mod common;

use std::fs;

use ed25519_dalek::SigningKey;
use lilia_plugin_api::{verify_package, Capability};

const LIBRARY: &[u8] = b"trusted native plugin";

#[test]
fn verifies_signature_and_rejects_tampered_library() {
    let directory = tempfile::tempdir().expect("temporary package");
    let signing_key = SigningKey::from_bytes(&[7; 32]);
    let manifest = common::manifest(LIBRARY);
    common::write_signed_package(directory.path(), &manifest, LIBRARY, &signing_key);

    verify_package(directory.path(), &[signing_key.verifying_key()], false)
        .expect("valid signed package");
    fs::write(directory.path().join(common::library_name()), b"tampered").expect("tamper library");
    let error = verify_package(directory.path(), &[signing_key.verifying_key()], false)
        .expect_err("tampering must fail");
    assert!(error.to_string().contains("checksum"));
}

#[test]
fn rejects_noncanonical_manifest_even_when_signature_matches_schema() {
    use base64::Engine;
    use ed25519_dalek::Signer;

    let directory = tempfile::tempdir().expect("temporary package");
    let signing_key = SigningKey::from_bytes(&[8; 32]);
    let manifest = common::manifest(LIBRARY);
    fs::write(directory.path().join(common::library_name()), LIBRARY).expect("write library");
    let pretty = serde_json::to_vec_pretty(&manifest).expect("pretty manifest");
    fs::write(directory.path().join("manifest.json"), &pretty).expect("write manifest");
    fs::write(
        directory.path().join("manifest.sig"),
        base64::engine::general_purpose::STANDARD.encode(signing_key.sign(&pretty).to_bytes()),
    )
    .expect("write signature");

    let error = verify_package(directory.path(), &[signing_key.verifying_key()], false)
        .expect_err("noncanonical manifest must fail");
    assert!(error.to_string().contains("canonical"));
}

#[test]
fn rejects_incompatible_and_ambiguous_manifest_fields() {
    let signing_key = SigningKey::from_bytes(&[9; 32]);
    let cases = [
        ("not-semver", "version"),
        ("engine-mismatch", "support this engine"),
        ("target-mismatch", "target"),
        ("uppercase-hash", "lowercase"),
        ("empty-capabilities", "nonempty"),
        ("duplicate-capabilities", "unique"),
    ];
    for (case, expected) in cases {
        let directory = tempfile::tempdir().expect("temporary package");
        let mut manifest = common::manifest(LIBRARY);
        match case {
            "not-semver" => manifest.version = "version-one".into(),
            "engine-mismatch" => manifest.engine_requirement = ">=99.0.0".into(),
            "target-mismatch" => manifest.target = "x86_64-unknown-example".into(),
            "uppercase-hash" => manifest.library_sha256.make_ascii_uppercase(),
            "empty-capabilities" => manifest.capabilities.clear(),
            "duplicate-capabilities" => manifest.capabilities.push(Capability::Kv),
            _ => unreachable!(),
        }
        common::write_signed_package(directory.path(), &manifest, LIBRARY, &signing_key);
        let error = verify_package(directory.path(), &[signing_key.verifying_key()], false)
            .expect_err(case);
        assert!(error.to_string().contains(expected), "{case}: {error}");
    }
}

#[test]
fn unsigned_packages_require_an_explicit_verification_policy() {
    let directory = tempfile::tempdir().expect("temporary package");
    let manifest = common::manifest(LIBRARY);
    fs::write(directory.path().join(common::library_name()), LIBRARY).expect("write library");
    fs::write(
        directory.path().join("manifest.json"),
        manifest.canonical_bytes().expect("manifest JSON"),
    )
    .expect("write manifest");

    verify_package(directory.path(), &[], false).expect_err("unsigned package must fail closed");
    verify_package(directory.path(), &[], true).expect("development policy may allow unsigned");
}

#[test]
fn rejects_unlisted_package_entries() {
    let directory = tempfile::tempdir().expect("temporary package");
    let signing_key = SigningKey::from_bytes(&[15; 32]);
    let manifest = common::manifest(LIBRARY);
    common::write_signed_package(directory.path(), &manifest, LIBRARY, &signing_key);
    fs::write(directory.path().join("hidden-payload"), b"not declared")
        .expect("write unexpected entry");

    let error = verify_package(directory.path(), &[signing_key.verifying_key()], false)
        .expect_err("unexpected files must fail");
    assert!(error.to_string().contains("unexpected entry"));
}

#[cfg(unix)]
#[test]
fn rejects_symlinked_package_entries() {
    use std::os::unix::fs::symlink;

    let parent = tempfile::tempdir().expect("temporary parent");
    let package = parent.path().join("package");
    fs::create_dir(&package).expect("create package");
    let signing_key = SigningKey::from_bytes(&[10; 32]);
    let manifest = common::manifest(LIBRARY);
    common::write_signed_package(&package, &manifest, LIBRARY, &signing_key);
    let external = parent.path().join("external-library");
    fs::write(&external, LIBRARY).expect("external library");
    fs::remove_file(package.join(common::library_name())).expect("remove library");
    symlink(external, package.join(common::library_name())).expect("symlink library");

    let error = verify_package(&package, &[signing_key.verifying_key()], false)
        .expect_err("symlink must fail");
    assert!(error.to_string().contains("regular file"));
}
