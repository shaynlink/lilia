use super::*;
use std::os::unix::fs::symlink;

#[cfg(target_os = "macos")]
fn acl(path: &Path, rule: &str) {
    assert!(std::process::Command::new("/bin/chmod")
        .args(["+a", rule])
        .arg(path)
        .status()
        .unwrap()
        .success());
}

#[cfg(target_os = "macos")]
#[test]
fn rejects_macos_acl_grants_despite_private_mode_bits() {
    let (_guard, path) = directory();
    acl(&path, "everyone allow list,search,add_file,add_subdirectory,delete_child,read,write,file_inherit,directory_inherit");
    assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o700);
    let error = create_token(&path.join("token")).unwrap_err();
    assert!(error.to_string().contains("ACL grants"));
    assert!(!path.join("token").exists());
    assert!(create_token(&path.join("child/token")).is_err());
    assert!(!path.join("child").exists());
}

#[cfg(target_os = "macos")]
#[test]
fn rejects_token_acl_grants_and_allows_deny_only_acls() {
    let (_guard, path) = directory();
    let token_path = path.join("token");
    let original = create_token(&token_path).unwrap();
    acl(&token_path, "everyone allow read,write");
    assert_eq!(fs::metadata(&token_path).unwrap().mode() & 0o777, 0o600);
    assert!(create_token(&token_path).is_err());
    assert_eq!(fs::read_to_string(&token_path).unwrap(), original);
    acl(&path, "everyone deny delete");
    let permitted = create_token(&path.join("other-token"));
    assert!(std::process::Command::new("/bin/chmod")
        .arg("-N")
        .arg(&path)
        .status()
        .unwrap()
        .success());
    assert!(permitted.is_ok());
}

fn directory() -> (tempfile::TempDir, PathBuf) {
    let guard = tempfile::tempdir().unwrap();
    let path = guard.path().canonicalize().unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    (guard, path)
}

#[test]
fn token_is_private_rotates_and_does_not_modify_old_inode() {
    let (_guard, path) = directory();
    let token_path = path.join("token");
    let first = create_token(&token_path).unwrap();
    assert_eq!(hex::decode(&first).unwrap().len(), 32);
    assert_eq!(fs::read_to_string(&token_path).unwrap(), first);
    assert_eq!(fs::metadata(&token_path).unwrap().mode() & 0o777, 0o600);
    let mut old = fs::File::open(&token_path).unwrap();
    let second = create_token(&token_path).unwrap();
    assert_ne!(first, second);
    assert_eq!(fs::read_to_string(&token_path).unwrap(), second);
    let mut old_value = String::new();
    std::io::Read::read_to_string(&mut old, &mut old_value).unwrap();
    assert_eq!(old_value, first);
    assert_eq!(fs::read_dir(path).unwrap().count(), 1);
}

#[test]
fn rejects_token_aliases_directories_and_public_files() {
    let (_guard, path) = directory();
    let target = path.join("target");
    fs::write(&target, "original").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
    let alias = path.join("alias");
    fs::hard_link(&target, &alias).unwrap();
    assert!(create_token(&alias).is_err());
    let link = path.join("dangling");
    symlink(path.join("absent"), &link).unwrap();
    assert!(create_token(&link).is_err());
    assert!(!path.join("absent").exists());
    let child = path.join("child");
    fs::create_dir(&child).unwrap();
    assert!(create_token(&child).is_err());
    let public = path.join("public");
    fs::write(&public, "public original").unwrap();
    fs::set_permissions(&public, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(create_token(&public).is_err());
    assert_eq!(fs::read_to_string(target).unwrap(), "original");
    assert_eq!(fs::read_to_string(public).unwrap(), "public original");
}

#[test]
fn checks_ancestors_and_preserves_existing_permissions() {
    let (_guard, path) = directory();
    let public = path.join("public");
    fs::create_dir(&public).unwrap();
    fs::set_permissions(&public, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(create_token(&public.join("token")).is_err());
    assert_eq!(fs::metadata(&public).unwrap().mode() & 0o777, 0o755);
    let token = public.join("new/private/token");
    create_token(&token).unwrap();
    assert_eq!(
        fs::metadata(public.join("new")).unwrap().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(public.join("new/private")).unwrap().mode() & 0o777,
        0o700
    );
    fs::set_permissions(&public, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(create_token(&token).is_err());
    fs::set_permissions(&public, fs::Permissions::from_mode(0o755)).unwrap();
    let alias = path.join("directory-link");
    symlink(public.join("new/private"), &alias).unwrap();
    assert!(create_token(&alias.join("token")).is_err());
    assert!(create_token(&path.join("../escaped-token")).is_err());
}

#[tokio::test]
async fn binding_preserves_existing_entries_and_live_listener() {
    let (_guard, path) = directory();
    let endpoint = path.join("daemon.sock");
    fs::write(&endpoint, "not a socket").unwrap();
    assert!(ListenerGuard::bind(&endpoint).is_err());
    assert_eq!(fs::read_to_string(&endpoint).unwrap(), "not a socket");
    fs::remove_file(&endpoint).unwrap();
    symlink(path.join("missing"), &endpoint).unwrap();
    assert!(ListenerGuard::bind(&endpoint).is_err());
    assert!(!path.join("missing").exists());
    assert!(fs::symlink_metadata(&endpoint)
        .unwrap()
        .file_type()
        .is_symlink());
    fs::remove_file(&endpoint).unwrap();
    let first = ListenerGuard::bind(&endpoint).unwrap();
    assert!(ListenerGuard::bind(&endpoint).is_err());
    assert_eq!(fs::metadata(&endpoint).unwrap().mode() & 0o777, 0o600);
    let _client = tokio::net::UnixStream::connect(&endpoint).await.unwrap();
    let (server, _) = first.listener.accept().await.unwrap();
    assert!(same_user(server.peer_cred().unwrap().uid()));
    assert!(!same_user(
        rustix::process::geteuid().as_raw().wrapping_add(1)
    ));
    drop(first);
    assert!(!endpoint.exists());
}

#[tokio::test]
async fn cleanup_never_unlinks_a_replacement_entry() {
    let (_guard, path) = directory();
    let endpoint = path.join("daemon.sock");
    let first = ListenerGuard::bind(&endpoint).unwrap();
    fs::remove_file(&endpoint).unwrap();
    let second = ListenerGuard::bind(&endpoint).unwrap();
    drop(first);
    assert!(endpoint.exists());
    fs::remove_file(&endpoint).unwrap();
    fs::write(&endpoint, "replacement").unwrap();
    drop(second);
    assert_eq!(fs::read_to_string(endpoint).unwrap(), "replacement");
}
