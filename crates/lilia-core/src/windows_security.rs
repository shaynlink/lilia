//! Windows security helpers ensuring restrictive, current-user-only DACLs
//! on newly created directories and database/backup files.
#![allow(
    unsafe_code,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::cast_possible_truncation
)]

use std::ffi::{c_void, OsStr};
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::{Component, Path, PathBuf};

use windows_sys::Win32::Foundation::{
    LocalFree, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, GENERIC_READ, GENERIC_WRITE,
    INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
    ConvertStringSidToSidW, GetNamedSecurityInfoW, SetNamedSecurityInfoW, SDDL_REVISION_1,
    SE_FILE_OBJECT,
};
use windows_sys::Win32::Security::{
    EqualSid, GetAce, GetSecurityDescriptorControl, GetSecurityDescriptorDacl,
    GetSecurityDescriptorOwner, GetTokenInformation, TokenUser, ACCESS_ALLOWED_ACE, ACE_HEADER,
    DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
    SECURITY_ATTRIBUTES, SE_DACL_PROTECTED, TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateDirectoryW, CreateFileW, FlushFileBuffers, FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

fn wide(value: &OsStr) -> io::Result<Vec<u16>> {
    let mut wide: Vec<_> = value.encode_wide().collect();
    if wide.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path contains NUL character",
        ));
    }
    wide.push(0);
    Ok(wide)
}

#[derive(Debug)]
pub struct LocalMemory(*mut c_void);

impl Drop for LocalMemory {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: only pointers allocated with LocalAlloc construct this guard.
            unsafe {
                LocalFree(self.0);
            }
        }
    }
}

/// Retrieves the current process user SID as a canonical SDDL string (e.g. `S-1-5-21-...`).
pub fn current_user_sid() -> io::Result<String> {
    // SAFETY: Output pointers and buffers are properly sized and aligned.
    unsafe {
        let mut raw_token = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut raw_token) == 0 {
            return Err(io::Error::last_os_error());
        }
        let token = OwnedHandle::from_raw_handle(raw_token);
        let mut length = 0;
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            std::ptr::null_mut(),
            0,
            &raw mut length,
        );
        let min_len = std::mem::size_of::<TOKEN_USER>() as u32;
        if length < min_len {
            return Err(io::Error::last_os_error());
        }
        let mut buffer = vec![0_usize; (length as usize).div_ceil(std::mem::size_of::<usize>())];
        if GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            buffer.as_mut_ptr().cast(),
            length,
            &raw mut length,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let user = &*buffer.as_ptr().cast::<TOKEN_USER>();
        let mut sid_text = std::ptr::null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &raw mut sid_text) == 0 {
            return Err(io::Error::last_os_error());
        }
        let _guard = LocalMemory(sid_text.cast());
        let mut count = 0;
        while *sid_text.add(count) != 0 {
            count += 1;
        }
        String::from_utf16(std::slice::from_raw_parts(sid_text, count))
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid user SID string"))
    }
}

/// RAII wrapper around a Windows `PSECURITY_DESCRIPTOR` allocated via SDDL string parsing.
#[derive(Debug)]
pub struct SecurityDescriptorGuard {
    descriptor: LocalMemory,
}

impl SecurityDescriptorGuard {
    pub fn from_sddl(sddl: &str) -> io::Result<Self> {
        let encoded = wide(OsStr::new(sddl))?;
        let mut descriptor = std::ptr::null_mut();
        // SAFETY: ConvertStringSecurityDescriptorToSecurityDescriptorW allocates via LocalAlloc.
        unsafe {
            if ConvertStringSecurityDescriptorToSecurityDescriptorW(
                encoded.as_ptr(),
                SDDL_REVISION_1,
                &raw mut descriptor,
                std::ptr::null_mut(),
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(Self {
            descriptor: LocalMemory(descriptor),
        })
    }

    #[must_use]
    #[inline]
    pub fn as_ptr(&self) -> *mut c_void {
        self.descriptor.0
    }

    #[must_use]
    pub fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.descriptor.0,
            bInheritHandle: 0,
        }
    }
}

/// Creates all missing parent directories for the given path, applying a protected,
/// current-user-only DACL (`O:{sid}D:P(A;OICI;FA;;;{sid})`) to every newly created directory.
/// Pre-existing directories are left untouched with their original ACLs intact.
pub fn create_dir_all_secure(path: &Path) -> io::Result<()> {
    if path.as_os_str().is_empty() || path.exists() {
        return Ok(());
    }

    let sid = current_user_sid()?;
    // O:{sid} -> Owner is current user
    // D:P     -> Protected DACL: blocks inheritance from parent directory
    // (A;OICI;FA;;;{sid}) -> Access Allowed, Object Inherit + Container Inherit, File All Access
    let sddl = format!("O:{sid}D:P(A;OICI;FA;;;{sid})");
    let sec = SecurityDescriptorGuard::from_sddl(&sddl)?;
    let attributes = sec.attributes();

    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        if matches!(component, Component::Prefix(_)) {
            continue;
        }
        if current.exists() {
            continue;
        }
        let encoded = wide(current.as_os_str())?;
        // SAFETY: Path string is null-terminated and attributes pointer is valid for the call duration.
        unsafe {
            if CreateDirectoryW(encoded.as_ptr(), &raw const attributes) == 0 {
                let err = io::Error::last_os_error();
                if err.raw_os_error() == Some(ERROR_ALREADY_EXISTS.cast_signed())
                    && current.is_dir()
                {
                    continue;
                }
                return Err(err);
            }
        }
        if let Some(parent) = current.parent() {
            if !parent.as_os_str().is_empty() {
                sync_directory(parent)?;
            }
        }
        sync_directory(&current)?;
    }
    Ok(())
}

/// Applies a protected, current-user-only DACL to an existing file (`O:{sid}D:P(A;;FA;;;{sid})`).
pub fn secure_file(path: &Path) -> io::Result<()> {
    let sid = current_user_sid()?;
    let sddl = format!("O:{sid}D:P(A;;FA;;;{sid})");
    apply_sddl(path, &sddl)
}

/// Applies a protected, current-user-only DACL to an existing directory (`O:{sid}D:P(A;OICI;FA;;;{sid})`).
pub fn secure_directory(path: &Path) -> io::Result<()> {
    let sid = current_user_sid()?;
    let sddl = format!("O:{sid}D:P(A;OICI;FA;;;{sid})");
    apply_sddl(path, &sddl)
}

fn apply_sddl(path: &Path, sddl: &str) -> io::Result<()> {
    let sec = SecurityDescriptorGuard::from_sddl(sddl)?;
    let encoded = wide(path.as_os_str())?;

    // SAFETY: Extract owner and DACL from the parsed security descriptor, then set them atomically.
    unsafe {
        let mut owner = std::ptr::null_mut();
        let mut owner_defaulted = 0;
        if GetSecurityDescriptorOwner(sec.as_ptr(), &raw mut owner, &raw mut owner_defaulted) == 0 {
            return Err(io::Error::last_os_error());
        }

        let mut dacl = std::ptr::null_mut();
        let mut dacl_present = 0;
        let mut dacl_defaulted = 0;
        if GetSecurityDescriptorDacl(
            sec.as_ptr(),
            &raw mut dacl_present,
            &raw mut dacl,
            &raw mut dacl_defaulted,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }

        let status = SetNamedSecurityInfoW(
            encoded.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION
                | DACL_SECURITY_INFORMATION
                | PROTECTED_DACL_SECURITY_INFORMATION,
            owner,
            std::ptr::null_mut(),
            dacl,
            std::ptr::null_mut(),
        );

        if status != 0 {
            return Err(io::Error::from_raw_os_error(status.cast_signed()));
        }
    }

    Ok(())
}

/// Validates that an existing file or directory has a protected DACL granting access
/// exclusively to the current process user.
pub fn is_private_dacl(path: &Path) -> io::Result<bool> {
    let encoded = wide(path.as_os_str())?;
    unsafe {
        let mut owner = std::ptr::null_mut();
        let mut dacl = std::ptr::null_mut();
        let mut descriptor = std::ptr::null_mut();
        let status = GetNamedSecurityInfoW(
            encoded.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &raw mut owner,
            std::ptr::null_mut(),
            &raw mut dacl,
            std::ptr::null_mut(),
            &raw mut descriptor,
        );
        if status != 0 {
            return Err(io::Error::from_raw_os_error(status.cast_signed()));
        }
        let _guard = LocalMemory(descriptor);

        let mut control = 0;
        let mut revision = 0;
        if GetSecurityDescriptorControl(descriptor, &raw mut control, &raw mut revision) == 0 {
            return Err(io::Error::last_os_error());
        }

        // Must be protected against inherited permissions
        if control & SE_DACL_PROTECTED == 0 {
            return Ok(false);
        }

        // Must have a valid DACL
        let Some(acl) = dacl.as_ref() else {
            return Ok(false);
        };

        let sid_str = current_user_sid()?;
        let encoded_sid = wide(OsStr::new(&sid_str))?;
        let mut expected_sid = std::ptr::null_mut();
        if ConvertStringSidToSidW(encoded_sid.as_ptr(), &raw mut expected_sid) == 0 {
            return Err(io::Error::last_os_error());
        }
        let _sid_guard = LocalMemory(expected_sid);

        if EqualSid(owner, expected_sid) == 0 {
            return Ok(false);
        }

        if acl.AceCount == 0 {
            return Ok(false);
        }

        for i in 0..acl.AceCount {
            let mut ace = std::ptr::null_mut();
            if GetAce(dacl, u32::from(i), &raw mut ace) == 0 {
                return Err(io::Error::last_os_error());
            }
            let Some(header) = ace.cast::<ACE_HEADER>().as_ref() else {
                return Ok(false);
            };
            if header.AceType != 0 {
                return Ok(false);
            }
            let Some(allowed_ace) = ace.cast::<ACCESS_ALLOWED_ACE>().as_ref() else {
                return Ok(false);
            };
            let ace_sid = (&raw const allowed_ace.SidStart).cast_mut().cast();
            if EqualSid(ace_sid, expected_sid) == 0 {
                return Ok(false);
            }
        }

        Ok(true)
    }
}

/// Flushes directory metadata to disk using `CreateFileW` with `FILE_FLAG_BACKUP_SEMANTICS`
/// and `FlushFileBuffers`.
pub fn sync_directory(path: &Path) -> io::Result<()> {
    if path.as_os_str().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path cannot be empty",
        ));
    }
    let encoded = wide(path.as_os_str())?;

    // SAFETY: Path is null-terminated UTF-16. CreateFileW is called with FILE_FLAG_BACKUP_SEMANTICS
    // to open a directory handle, and OwnedHandle guarantees CloseHandle is called on drop.
    unsafe {
        let mut raw_handle = CreateFileW(
            encoded.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        );

        if raw_handle == INVALID_HANDLE_VALUE {
            let err = io::Error::last_os_error();
            if err.raw_os_error() == Some(ERROR_ACCESS_DENIED.cast_signed()) {
                raw_handle = CreateFileW(
                    encoded.as_ptr(),
                    GENERIC_WRITE,
                    FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                    std::ptr::null(),
                    OPEN_EXISTING,
                    FILE_FLAG_BACKUP_SEMANTICS,
                    std::ptr::null_mut(),
                );
            }
        }

        if raw_handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }

        let handle = OwnedHandle::from_raw_handle(raw_handle);
        if FlushFileBuffers(handle.as_raw_handle()) == 0 {
            return Err(io::Error::last_os_error());
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sync_directory_success_and_errors() {
        let temp = std::env::temp_dir().join(format!("lilia-sync-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp).unwrap();
        assert!(sync_directory(&temp).is_ok());

        // Empty path error
        assert!(sync_directory(Path::new("")).is_err());

        // Non-existent path error
        let nonexistent = temp.join("does_not_exist");
        assert!(sync_directory(&nonexistent).is_err());

        std::fs::remove_dir_all(&temp).unwrap();
    }
}
