mod common;

use std::fs;
use std::sync::{Arc, Barrier};
use std::thread;

use ed25519_dalek::SigningKey;
use lilia_plugin_api::{install_verified, verify_package};

const LIBRARY: &[u8] = b"installable native plugin";

#[test]
fn installs_from_private_staging_and_reverifies_published_copy() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let source = directory.path().join("source");
    let root = directory.path().join("plugins");
    fs::create_dir(&source).expect("source directory");
    let signing_key = SigningKey::from_bytes(&[11; 32]);
    let manifest = common::manifest(LIBRARY);
    common::write_signed_package(&source, &manifest, LIBRARY, &signing_key);

    let installed = install_verified(&source, &root, &[signing_key.verifying_key()], false)
        .expect("install package");
    assert_eq!(
        installed.file_name().and_then(|name| name.to_str()),
        Some("package")
    );
    verify_package(&installed, &[signing_key.verifying_key()], false)
        .expect("published package remains verifiable");
    fs::write(source.join(common::library_name()), b"changed source").expect("mutate source");
    verify_package(&installed, &[signing_key.verifying_key()], false)
        .expect("installed copy is independent");

    let entries = fs::read_dir(&root)
        .expect("read plugin root")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(entries.len(), 1, "staging and lock files must be removed");
    assert!(!entries[0].starts_with('.'));
}

#[test]
fn refuses_to_overwrite_an_existing_installation() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let source = directory.path().join("source");
    let root = directory.path().join("plugins");
    fs::create_dir(&source).expect("source directory");
    let signing_key = SigningKey::from_bytes(&[12; 32]);
    let manifest = common::manifest(LIBRARY);
    common::write_signed_package(&source, &manifest, LIBRARY, &signing_key);
    let key = signing_key.verifying_key();
    let installed = install_verified(&source, &root, &[key], false).expect("first install");
    let before = fs::read(installed.join(common::library_name())).expect("installed bytes");

    install_verified(&source, &root, &[key], false).expect_err("second install must fail");
    assert_eq!(
        fs::read(installed.join(common::library_name())).expect("installed bytes"),
        before
    );
}

#[test]
fn concurrent_installers_publish_exactly_one_package() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let source = directory.path().join("source");
    let root = directory.path().join("plugins");
    fs::create_dir(&source).expect("source directory");
    let signing_key = SigningKey::from_bytes(&[13; 32]);
    let manifest = common::manifest(LIBRARY);
    common::write_signed_package(&source, &manifest, LIBRARY, &signing_key);
    let barrier = Arc::new(Barrier::new(2));
    let mut workers = Vec::new();
    for _ in 0..2 {
        let source = source.clone();
        let root = root.clone();
        let barrier = Arc::clone(&barrier);
        let key = signing_key.verifying_key();
        workers.push(thread::spawn(move || {
            barrier.wait();
            install_verified(&source, &root, &[key], false)
        }));
    }
    let results = workers
        .into_iter()
        .map(|worker| worker.join().expect("worker"))
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);
    let installed = results.into_iter().find_map(Result::ok).expect("winner");
    verify_package(&installed, &[signing_key.verifying_key()], false).expect("winner is valid");
}

#[cfg(unix)]
#[test]
fn rejects_public_or_symlinked_install_roots() {
    use std::os::unix::fs::{symlink, PermissionsExt};

    let directory = tempfile::tempdir().expect("temporary directory");
    let source = directory.path().join("source");
    fs::create_dir(&source).expect("source directory");
    let signing_key = SigningKey::from_bytes(&[14; 32]);
    let manifest = common::manifest(LIBRARY);
    common::write_signed_package(&source, &manifest, LIBRARY, &signing_key);

    let public_root = directory.path().join("public");
    fs::create_dir(&public_root).expect("public root");
    fs::set_permissions(&public_root, fs::Permissions::from_mode(0o755)).expect("permissions");
    install_verified(&source, &public_root, &[signing_key.verifying_key()], false)
        .expect_err("public root must fail");

    let private_root = directory.path().join("private");
    fs::create_dir(&private_root).expect("private root");
    let linked_root = directory.path().join("linked");
    symlink(&private_root, &linked_root).expect("root symlink");
    install_verified(&source, &linked_root, &[signing_key.verifying_key()], false)
        .expect_err("symlink root must fail");
}
