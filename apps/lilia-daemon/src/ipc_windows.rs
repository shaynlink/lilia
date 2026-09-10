//! Windows security attributes are supplied at object creation, never patched
//! onto a token after writing. FFI ownership and pointer lifetimes stay here.
#![allow(unsafe_code)]

use std::ffi::{c_void, OsStr};
use std::fs::File;
use std::io::{self, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::{Component, Path, PathBuf, Prefix};

use rand::RngCore;
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use windows_sys::Win32::Foundation::{LocalFree, GENERIC_ALL, GENERIC_WRITE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
    ConvertStringSidToSidW, GetSecurityInfo, SDDL_REVISION_1, SE_FILE_OBJECT,
};
use windows_sys::Win32::Security::{
    EqualSid, GetAce, GetSecurityDescriptorControl, GetSecurityDescriptorOwner,
    GetTokenInformation, IsWellKnownSid, TokenUser, WinBuiltinAdministratorsSid, WinLocalSystemSid,
    ACCESS_ALLOWED_ACE, ACE_HEADER, DACL_SECURITY_INFORMATION, INHERIT_ONLY_ACE,
    OWNER_SECURITY_INFORMATION, SECURITY_ATTRIBUTES, SE_DACL_PROTECTED, TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateDirectoryW, CreateFileW, GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    CREATE_NEW, DELETE, FILE_ALL_ACCESS, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_DELETE_CHILD, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_READ_ATTRIBUTES, FILE_READ_DATA, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_WRITE_ATTRIBUTES,
    FILE_WRITE_DATA, FILE_WRITE_EA, OPEN_EXISTING, READ_CONTROL, WRITE_DAC, WRITE_OWNER,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

#[cfg(test)]
#[path = "ipc_windows/tests.rs"]
mod tests;

const ANCESTOR_MUTATION_RIGHTS: u32 = GENERIC_ALL
    | GENERIC_WRITE
    | WRITE_DAC
    | WRITE_OWNER
    | DELETE
    | FILE_DELETE_CHILD
    | FILE_WRITE_ATTRIBUTES
    | FILE_WRITE_EA
    | FILE_WRITE_DATA;

fn denied(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}

fn wide(value: &OsStr) -> io::Result<Vec<u16>> {
    let mut value: Vec<_> = value.encode_wide().collect();
    if value.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path contains NUL",
        ));
    }
    value.push(0);
    Ok(value)
}

#[derive(Debug)]
struct LocalMemory(*mut c_void);

fn parse_sid(value: &str) -> io::Result<LocalMemory> {
    let encoded = wide(OsStr::new(value))?;
    let mut sid = std::ptr::null_mut();
    // SAFETY: valid string and output slot. The returned allocation is guarded.
    if unsafe { ConvertStringSidToSidW(encoded.as_ptr(), &raw mut sid) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(LocalMemory(sid))
}

impl Drop for LocalMemory {
    fn drop(&mut self) {
        // SAFETY: only successful APIs allocating with LocalAlloc construct this guard.
        unsafe {
            LocalFree(self.0);
        }
    }
}

#[derive(Debug)]
pub(crate) struct UserSecurity {
    descriptor: LocalMemory,
}

pub(crate) struct Credentials {
    pub(crate) token: String,
    // No FILE_SHARE_DELETE on these handles: ancestors cannot be renamed or
    // replaced throughout the daemon lifetime, not merely during validation.
    _parents: Vec<File>,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials").finish_non_exhaustive()
    }
}

impl UserSecurity {
    pub(crate) fn new() -> io::Result<Self> {
        // SAFETY: all output slots are aligned and initialized; token information
        // is kept in aligned storage until its SID has been converted. Every owned
        // HANDLE and LocalAlloc allocation is released exactly once by a guard.
        unsafe {
            let mut raw = std::ptr::null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut raw) == 0 {
                return Err(io::Error::last_os_error());
            }
            let token = OwnedHandle::from_raw_handle(raw);
            let mut length = 0;
            GetTokenInformation(
                token.as_raw_handle(),
                TokenUser,
                std::ptr::null_mut(),
                0,
                &raw mut length,
            );
            if length < u32::try_from(std::mem::size_of::<TOKEN_USER>()).unwrap() {
                return Err(io::Error::last_os_error());
            }
            let mut buffer =
                vec![0_usize; (length as usize).div_ceil(std::mem::size_of::<usize>())];
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
            let _sid_memory = LocalMemory(sid_text.cast());
            let mut count = 0;
            while *sid_text.add(count) != 0 {
                count += 1;
            }
            let sid = String::from_utf16(std::slice::from_raw_parts(sid_text, count))
                .map_err(|_| denied("invalid process SID"))?;
            let sddl = wide(OsStr::new(&format!("O:{sid}D:P(A;;FA;;;{sid})")))?;
            let mut descriptor = std::ptr::null_mut();
            if ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &raw mut descriptor,
                std::ptr::null_mut(),
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(Self {
                descriptor: LocalMemory(descriptor),
            })
        }
    }

    fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: u32::try_from(std::mem::size_of::<SECURITY_ATTRIBUTES>()).unwrap(),
            lpSecurityDescriptor: self.descriptor.0,
            bInheritHandle: 0,
        }
    }

    pub(crate) fn pipe(&self, path: &Path, first: bool) -> io::Result<NamedPipeServer> {
        let name = path
            .to_str()
            .ok_or_else(|| denied("invalid local pipe name"))?;
        let suffix = name
            .strip_prefix(r"\\.\pipe\")
            .ok_or_else(|| denied("only local named pipes are allowed"))?;
        if suffix.is_empty() || suffix.len() > 200 || suffix.contains(['\\', '/', '\0', ':']) {
            return Err(denied("invalid local pipe name"));
        }
        let mut attributes = self.attributes();
        // SAFETY: both SECURITY_ATTRIBUTES and its descriptor remain live through
        // the synchronous CreateNamedPipe call; Windows copies the security data.
        unsafe {
            ServerOptions::new()
                .first_pipe_instance(first)
                .reject_remote_clients(true)
                .create_with_security_attributes_raw(path, (&raw mut attributes).cast())
        }
    }

    fn open(&self, path: &Path, create: bool) -> io::Result<File> {
        let path = wide(path.as_os_str())?;
        let attributes = self.attributes();
        // SAFETY: valid NUL-terminated path and live descriptor; only a successful
        // owned handle is transferred to File. OPEN_REPARSE_POINT avoids following
        // the final component, subsequently checked through the returned handle.
        unsafe {
            let handle = CreateFileW(
                path.as_ptr(),
                // A metadata-only handle does not participate in share-access
                // checks. READ_DATA (LIST_DIRECTORY on folders) pins the name.
                READ_CONTROL
                    | FILE_READ_ATTRIBUTES
                    | FILE_READ_DATA
                    | if create { GENERIC_WRITE } else { 0 },
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                &raw const attributes,
                if create { CREATE_NEW } else { OPEN_EXISTING },
                FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
                std::ptr::null_mut(),
            );
            if handle == INVALID_HANDLE_VALUE {
                return Err(io::Error::last_os_error());
            }
            Ok(File::from_raw_handle(handle))
        }
    }

    fn validate_private(&self, file: &impl AsRawHandle) -> io::Result<()> {
        self.validate_security(file, true)
    }

    fn validate_security(&self, file: &impl AsRawHandle, private: bool) -> io::Result<()> {
        // SAFETY: GetSecurityInfo returns an allocated, valid self-relative SD;
        // owner and ACL/ACE pointers point into it and are only read while it lives.
        unsafe {
            let mut owner = std::ptr::null_mut();
            let mut acl = std::ptr::null_mut();
            let mut descriptor = std::ptr::null_mut();
            let status = GetSecurityInfo(
                file.as_raw_handle(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                &raw mut owner,
                std::ptr::null_mut(),
                &raw mut acl,
                std::ptr::null_mut(),
                &raw mut descriptor,
            );
            if status != 0 {
                return Err(io::Error::from_raw_os_error(status.cast_signed()));
            }
            let _memory = LocalMemory(descriptor);
            let mut expected = std::ptr::null_mut();
            let mut defaulted = 0;
            let mut control = 0;
            let mut revision = 0;
            if GetSecurityDescriptorOwner(self.descriptor.0, &raw mut expected, &raw mut defaulted)
                == 0
                || GetSecurityDescriptorControl(descriptor, &raw mut control, &raw mut revision)
                    == 0
            {
                return Err(io::Error::last_os_error());
            }
            // Windows Modules Installer owns standard system ancestors (including
            // drive roots). Trust this exact privileged service SID, not all services.
            let installer =
                parse_sid("S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464")?;
            let trusted = |sid| {
                EqualSid(sid, expected) != 0
                    || EqualSid(sid, installer.0) != 0
                    || IsWellKnownSid(sid, WinLocalSystemSid) != 0
                    || IsWellKnownSid(sid, WinBuiltinAdministratorsSid) != 0
            };
            let acl_ref = acl
                .as_ref()
                .ok_or_else(|| denied("missing token path DACL"))?;
            if owner.is_null()
                || !trusted(owner)
                || (private
                    && (EqualSid(owner, expected) == 0
                        || control & SE_DACL_PROTECTED == 0
                        || acl_ref.AceCount != 1))
            {
                return Err(denied(
                    "token directory/file must have a protected current-user-only DACL",
                ));
            }
            for index in 0..u32::from(acl_ref.AceCount) {
                let mut ace = std::ptr::null_mut();
                if GetAce(acl, index, &raw mut ace) == 0 {
                    return Err(io::Error::last_os_error());
                }
                let header = ace
                    .cast::<ACE_HEADER>()
                    .as_ref()
                    .ok_or_else(|| denied("missing token path ACL entry"))?;
                // Deny entries cannot expand access. Unknown/object/callback ACEs
                // are refused rather than attempting to reinterpret their layout.
                if !private && header.AceType == 1 {
                    continue;
                }
                if header.AceType != 0
                    || usize::from(header.AceSize) < std::mem::size_of::<ACCESS_ALLOWED_ACE>()
                {
                    return Err(denied("unsupported token path ACL entry"));
                }
                let ace = ace
                    .cast::<ACCESS_ALLOWED_ACE>()
                    .as_ref()
                    .ok_or_else(|| denied("missing token path ACL entry"))?;
                let sid = (&raw const ace.SidStart).cast_mut().cast();
                if private {
                    if ace.Header.AceFlags != 0
                        || ![FILE_ALL_ACCESS, GENERIC_ALL].contains(&ace.Mask)
                        || EqualSid(sid, expected) == 0
                    {
                        return Err(denied(
                            "token directory/file grants access beyond the current user",
                        ));
                    }
                } else if u32::from(ace.Header.AceFlags) & INHERIT_ONLY_ACE == 0
                    && ace.Mask & ANCESTOR_MUTATION_RIGHTS != 0
                    && !trusted(sid)
                {
                    return Err(denied("token ancestor permits mutation by another user"));
                }
            }
            Ok(())
        }
    }

    pub(crate) fn credentials(&self, path: &Path) -> io::Result<Credentials> {
        let path = credential_path(path)?;
        let parent = path
            .parent()
            .ok_or_else(|| denied("token requires a parent"))?;
        let mut parents = Vec::new();
        let mut current = PathBuf::new();
        for component in parent.components() {
            current.push(component.as_os_str());
            if matches!(component, Component::Prefix(_)) {
                continue;
            }
            let file = match self.open(&current, false) {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    let encoded = wide(current.as_os_str())?;
                    let attributes = self.attributes();
                    // SAFETY: buffers and security descriptor live through creation.
                    if unsafe { CreateDirectoryW(encoded.as_ptr(), &raw const attributes) } == 0 {
                        return Err(io::Error::last_os_error());
                    }
                    self.open(&current, false)?
                }
                Err(error) => return Err(error),
            };
            let info = information(&file)?;
            if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
                || info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0
            {
                return Err(denied(
                    "token ancestors must be real directories, not reparse points",
                ));
            }
            if current == parent {
                self.validate_private(&file)?;
            } else {
                // Delete sharing alone does not stop in-place junction conversion.
                // Refuse untrusted write-data/write-attributes and ACL mutation rights.
                self.validate_security(&file, false)?;
            }
            parents.push(file);
        }
        match self.open(&path, false) {
            Ok(existing) => {
                let info = information(&existing)?;
                if info.dwFileAttributes & (FILE_ATTRIBUTE_REPARSE_POINT | FILE_ATTRIBUTE_DIRECTORY)
                    != 0
                    || info.nNumberOfLinks != 1
                {
                    return Err(denied("token must be an unlinked regular file"));
                }
                self.validate_private(&existing)?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let temporary = parent.join(format!(".lilia-token-{}", uuid::Uuid::new_v4()));
        let mut file = self.open(&temporary, true)?;
        let _cleanup = PendingFile(temporary.clone());
        let written = (|| -> io::Result<String> {
            self.validate_private(&file)?;
            let mut bytes = [0_u8; 32];
            rand::rng().fill_bytes(&mut bytes);
            let token = hex::encode(bytes);
            file.write_all(token.as_bytes())?;
            file.sync_all()?;
            Ok(token)
        })();
        // Close before either publication or cleanup: delete sharing was denied.
        drop(file);
        let token = written?;
        std::fs::rename(&temporary, &path)?;
        Ok(Credentials {
            token,
            _parents: parents,
        })
    }
}

fn information(file: &File) -> io::Result<BY_HANDLE_FILE_INFORMATION> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: handle is live and output points to a correctly sized aligned struct.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &raw mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(info)
}

fn credential_path(path: &Path) -> io::Result<PathBuf> {
    // GetFullPathName (used by absolute) can erase a trailing dot/space. Reject
    // ambiguous input before normalization, then check the resolved path too.
    validate_components(path)?;
    let path = std::path::absolute(path)?;
    validate_components(&path)?;
    if !matches!(path.components().next(), Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)))
    {
        return Err(denied("token path must be on a local drive"));
    }
    if path.file_name().is_none() {
        return Err(denied("token requires a filename"));
    }
    Ok(path)
}

fn validate_components(path: &Path) -> io::Result<()> {
    if path
        .components()
        .any(|part| matches!(part, Component::ParentDir))
    {
        return Err(denied("token path must not contain traversal"));
    }
    for part in path.components() {
        if let Component::Normal(name) = part {
            let name = name.to_str().ok_or_else(|| denied("invalid token path"))?;
            if name.contains(':') || name.ends_with(['.', ' ']) {
                return Err(denied(
                    "token path must not contain streams or ambiguous components",
                ));
            }
        }
    }
    Ok(())
}

struct PendingFile(PathBuf);

impl Drop for PendingFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
