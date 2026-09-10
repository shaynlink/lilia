use super::*;

#[path = "cross_account.rs"]
mod cross_account;

fn assert_safe_ancestors(security: &UserSecurity, parent: &Path) {
    for path in parent.ancestors() {
        let file = security.open(path, false).unwrap();
        if let Err(error) = security.validate_security(&file, false) {
            panic!("unsafe test ancestor {}: {error}", path.display());
        }
    }
}

fn private_parent(security: &UserSecurity, path: &Path) {
    let encoded = wide(path.as_os_str()).unwrap();
    let attributes = security.attributes();
    // SAFETY: valid creation buffers; fixture belongs to this test only.
    assert_ne!(
        unsafe { CreateDirectoryW(encoded.as_ptr(), &raw const attributes) },
        0
    );
}

#[test]
fn tokens_are_private_and_rotate_without_truncating_old_files() {
    let temporary = tempfile::tempdir().unwrap();
    let security = UserSecurity::new().unwrap();
    assert_safe_ancestors(&security, temporary.path());
    let parent = temporary.path().join("private");
    private_parent(&security, &parent);
    let path = parent.join("token");
    let first = security.credentials(&path).unwrap();
    assert_eq!(hex::decode(&first.token).unwrap().len(), 32);
    security
        .validate_private(&security.open(&path, false).unwrap())
        .unwrap();
    let second = security.credentials(&path).unwrap();
    assert_ne!(first.token, second.token);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), second.token);
    assert!(std::fs::rename(&parent, temporary.path().join("moved")).is_err());
    assert!(!format!("{second:?}").contains(&second.token));
    drop(first);
    drop(second);
    std::fs::rename(parent, temporary.path().join("moved")).unwrap();
}

#[test]
fn unsafe_token_inputs_are_rejected_without_modification() {
    let temporary = tempfile::tempdir().unwrap();
    let security = UserSecurity::new().unwrap();
    let parent = temporary.path().join("private");
    private_parent(&security, &parent);
    let path = parent.join("token");
    let original = security.credentials(&path).unwrap();
    let alias = parent.join("alias");
    std::fs::hard_link(&path, &alias).unwrap();
    assert!(security.credentials(&alias).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original.token);
    let public = temporary.path().join("public-token");
    std::fs::write(&public, b"preserve").unwrap();
    assert!(security.credentials(&public).is_err());
    assert_eq!(std::fs::read(public).unwrap(), b"preserve");
    for input in [
        r"C:\private\..\escape",
        r"C:\private\token:stream",
        r"C:\private\token.",
        r"C:\private\token ",
        r"C:\private.\token",
        r"\\?\C:\private\token.",
        r"\\server\share\token",
    ] {
        assert!(credential_path(Path::new(input)).is_err(), "{input}");
    }
}

#[test]
fn rejects_mutable_ancestors_before_creating_private_children() {
    let temporary = tempfile::tempdir().unwrap();
    let security = UserSecurity::new().unwrap();
    let sddl = wide(OsStr::new("D:P(A;;FA;;;WD)")).unwrap();
    let mut descriptor = std::ptr::null_mut();
    // SAFETY: valid SDDL and output; this deliberately insecure descriptor is only
    // used to create the test-owned rejection fixture, never a real credential.
    assert_ne!(
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &raw mut descriptor,
                std::ptr::null_mut(),
            )
        },
        0
    );
    let public = UserSecurity {
        descriptor: LocalMemory(descriptor),
    };
    let parent = temporary.path().join("mutable");
    private_parent(&public, &parent);
    assert_eq!(
        security
            .credentials(&parent.join("private/token"))
            .unwrap_err()
            .to_string(),
        "token ancestor permits mutation by another user"
    );
    assert!(!parent.join("private").exists());
}

#[tokio::test]
async fn pipe_has_private_dacl_and_namespace_stays_claimed() {
    use tokio::net::windows::named_pipe::ClientOptions;
    let security = UserSecurity::new().unwrap();
    let path = PathBuf::from(format!(r"\\.\pipe\lilia-test-{}", uuid::Uuid::new_v4()));
    let first = security.pipe(&path, true).unwrap();
    security.validate_private(&first).unwrap();
    assert!(security.pipe(&path, true).is_err());
    let client = ClientOptions::new().open(&path).unwrap();
    first.connect().await.unwrap();
    let next = security.pipe(&path, false).unwrap();
    drop(client);
    drop(first);
    assert!(security.pipe(&path, true).is_err());
    let _client = ClientOptions::new().open(&path).unwrap();
    next.connect().await.unwrap();
    for name in [r"\\remote\pipe\lilia", r"\\.\pipe\nested\lilia"] {
        assert!(security.pipe(Path::new(name), true).is_err());
    }
}

#[tokio::test]
async fn default_pipe_security_is_not_accepted_as_user_only() {
    let security = UserSecurity::new().unwrap();
    let path = PathBuf::from(format!(
        r"\\.\pipe\lilia-default-test-{}",
        uuid::Uuid::new_v4()
    ));
    let default_pipe = ServerOptions::new()
        .first_pipe_instance(true)
        .create(&path)
        .unwrap();
    assert!(security.validate_private(&default_pipe).is_err());
}
