//! Opt-in native test: CI supplies a disposable, non-administrator local account.
use super::*;
use windows_sys::Win32::Security::{
    ImpersonateLoggedOnUser, LogonUserW, RevertToSelf, LOGON32_LOGON_INTERACTIVE,
    LOGON32_PROVIDER_DEFAULT,
};

struct Impersonation(std::marker::PhantomData<std::rc::Rc<()>>);

fn assert_access_denied<T>(result: io::Result<T>) {
    // Never format an unexpected successful value: it could contain the token.
    match result {
        Err(error) => assert_eq!(error.raw_os_error(), Some(5)),
        Ok(_) => panic!("private object unexpectedly accessible to another account"),
    }
}

impl Drop for Impersonation {
    fn drop(&mut self) {
        // SAFETY: this non-Send guard never leaves the impersonating thread.
        // Continuing under a different identity after failed reversion is unsafe.
        if unsafe { RevertToSelf() } == 0 {
            std::process::abort();
        }
    }
}

fn logon_other_user(security: &UserSecurity) -> OwnedHandle {
    let user = wide(&std::env::var_os("LILIA_TEST_USER").expect("test account required")).unwrap();
    let mut password =
        wide(&std::env::var_os("LILIA_TEST_PASSWORD").expect("test password required")).unwrap();
    let domain = wide(OsStr::new(".")).unwrap();
    let mut token = std::ptr::null_mut();
    // SAFETY: valid NUL-terminated inputs and output; handle ownership is transferred.
    let success = unsafe {
        LogonUserW(
            user.as_ptr(),
            domain.as_ptr(),
            password.as_ptr(),
            LOGON32_LOGON_INTERACTIVE,
            LOGON32_PROVIDER_DEFAULT,
            &raw mut token,
        )
    };
    password.fill(0);
    assert_ne!(
        success,
        0,
        "local account logon failed: {}",
        io::Error::last_os_error()
    );
    // SAFETY: LogonUser succeeded and returned an owned token.
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let mut length = 0;
    // SAFETY: sizing call, followed by aligned storage for the complete TOKEN_USER.
    unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            std::ptr::null_mut(),
            0,
            &raw mut length,
        )
    };
    let mut buffer = vec![0_usize; (length as usize).div_ceil(std::mem::size_of::<usize>())];
    unsafe {
        assert_ne!(
            GetTokenInformation(
                token.as_raw_handle(),
                TokenUser,
                buffer.as_mut_ptr().cast(),
                length,
                &raw mut length
            ),
            0
        );
        let user = &*buffer.as_ptr().cast::<TOKEN_USER>();
        let mut owner = std::ptr::null_mut();
        let mut defaulted = 0;
        assert_ne!(
            GetSecurityDescriptorOwner(security.descriptor.0, &raw mut owner, &raw mut defaulted),
            0
        );
        assert_eq!(
            EqualSid(user.User.Sid, owner),
            0,
            "fixture must use a different SID"
        );
    }
    token
}

#[test]
#[ignore = "requires a disposable second local account; run tools/test-windows-accounts.ps1"]
fn second_account_cannot_open_private_pipe_or_token() {
    use tokio::net::windows::named_pipe::ClientOptions;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .build()
        .unwrap();
    let _runtime = runtime.enter();
    let security = UserSecurity::new().unwrap();
    let token = logon_other_user(&security);
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("private/token");
    let credentials = security.credentials(&path).unwrap();
    let pipe_name = PathBuf::from(format!(r"\\.\pipe\lilia-private-{}", uuid::Uuid::new_v4()));
    let _pipe = security.pipe(&pipe_name, true).unwrap();

    // A deliberately public file/pipe proves the second identity can reach these
    // locations: private denials must not be caused by a broken test environment.
    let sddl = wide(OsStr::new("D:P(A;;FA;;;WD)")).unwrap();
    let mut descriptor = std::ptr::null_mut();
    // SAFETY: test-only public descriptor, guarded immediately after conversion.
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
    let public_path = temporary.path().join("public-control");
    drop(public.open(&public_path, true).unwrap());
    let public_name = PathBuf::from(format!(r"\\.\pipe\lilia-public-{}", uuid::Uuid::new_v4()));
    let _public_pipe = public.pipe(&public_name, true).unwrap();

    // No await/spawn occurs while impersonating: every access runs on this thread.
    assert_ne!(unsafe { ImpersonateLoggedOnUser(token.as_raw_handle()) }, 0);
    let identity = Impersonation(std::marker::PhantomData);
    assert!(
        std::fs::read(&public_path).is_ok(),
        "public file control must be readable"
    );
    let public_client = ClientOptions::new()
        .open(&public_name)
        .expect("public pipe control must connect");
    assert_access_denied(std::fs::read(&path));
    assert_access_denied(ClientOptions::new().open(&pipe_name));
    drop(public_client);
    drop(identity);

    // Positive control on the very same private objects after identity restoration.
    assert!(std::fs::read(&path).is_ok(), "owner must still read token");
    let _client = ClientOptions::new()
        .open(&pipe_name)
        .expect("owner must still connect");
    drop(credentials);
}
