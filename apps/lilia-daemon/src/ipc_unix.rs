use std::fs::{self, Metadata};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};

use rand::RngCore;
use tokio::net::UnixListener;

#[cfg(test)]
mod tests;

pub(crate) fn same_user(uid: u32) -> bool {
    uid == rustix::process::geteuid().as_raw()
}

fn denied(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}

/// Walk from the root without following symlinks. Every ancestor must resist
/// replacement by another unprivileged user; the final directory is user-only.
fn private_path(path: &Path) -> io::Result<PathBuf> {
    if path
        .components()
        .any(|part| matches!(part, Component::ParentDir))
    {
        return Err(denied("IPC paths must not contain parent traversal"));
    }
    let absolute = std::path::absolute(path)?;
    let parent = absolute
        .parent()
        .ok_or_else(|| denied("IPC path requires a parent"))?;
    if absolute.file_name().is_none() {
        return Err(denied("IPC path requires a filename"));
    }
    let mut current = PathBuf::new();
    for component in parent.components() {
        current.push(component.as_os_str());
        let metadata = match fs::symlink_metadata(&current) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let mut builder = fs::DirBuilder::new();
                builder.mode(0o700);
                match builder.create(&current) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(error),
                }
                fs::symlink_metadata(&current)?
            }
            Err(error) => return Err(error),
        };
        if !metadata.is_dir() || (metadata.uid() != 0 && !same_user(metadata.uid())) {
            return Err(denied(
                "IPC ancestors must be real directories owned by root or the current user",
            ));
        }
        // Root-owned sticky temporary directories prevent other users from
        // replacing our private child, while allowing ordinary /tmp use via its real path.
        let sticky_root = metadata.uid() == 0 && metadata.mode() & 0o1000 != 0;
        if metadata.mode() & 0o022 != 0 && !sticky_root {
            return Err(denied("IPC ancestor is writable by another user"));
        }
        if current == parent && (!same_user(metadata.uid()) || metadata.mode() & 0o077 != 0) {
            return Err(denied(
                "IPC parent must be owned by the current user with mode 0700",
            ));
        }
    }
    Ok(absolute)
}

// Octal masks make the POSIX permission policy explicit.
#[allow(clippy::verbose_bit_mask)]
fn validate_token(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata)
            if metadata.is_file()
                && same_user(metadata.uid())
                && metadata.mode() & 0o077 == 0
                && metadata.nlink() == 1 =>
        {
            Ok(())
        }
        Ok(_) => Err(denied(
            "existing token must be a private, owned regular file without links",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

pub(crate) fn create_token(path: &Path) -> io::Result<String> {
    let path = private_path(path)?;
    validate_token(&path)?;
    let parent = path
        .parent()
        .ok_or_else(|| denied("token requires a parent"))?;
    // tempfile opens with create_new and mode 0600, before any secret is written.
    let mut staging = tempfile::Builder::new()
        .prefix(".lilia-token-")
        .tempfile_in(parent)?;
    let mut bytes = [0_u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    let token = hex::encode(bytes);
    staging.write_all(token.as_bytes())?;
    staging.as_file().sync_all()?;
    validate_token(&path)?;
    staging.persist(&path).map_err(|error| error.error)?;
    fs::File::open(parent)?.sync_all()?;
    Ok(token)
}

#[derive(Debug)]
pub(crate) struct ListenerGuard {
    pub(crate) listener: UnixListener,
    path: PathBuf,
    identity: Metadata,
}

impl ListenerGuard {
    pub(crate) fn bind(path: &Path) -> io::Result<Self> {
        let path = private_path(path)?;
        // Some Unix kernels follow dangling symlinks during bind. Check the entry
        // without following links first, inside the validated private parent.
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "IPC endpoint already exists; inspect it before removal",
                ))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        // Stale sockets require deliberate operator cleanup, not automatic unlink.
        let listener = UnixListener::bind(&path)?;
        let identity = fs::symlink_metadata(&path)?;
        let guard = Self {
            listener,
            path,
            identity,
        };
        fs::set_permissions(&guard.path, fs::Permissions::from_mode(0o600))?;
        Ok(guard)
    }
}

impl Drop for ListenerGuard {
    fn drop(&mut self) {
        if let Ok(metadata) = fs::symlink_metadata(&self.path) {
            if metadata.file_type().is_socket()
                && same_user(metadata.uid())
                && metadata.dev() == self.identity.dev()
                && metadata.ino() == self.identity.ino()
            {
                let _ = fs::remove_file(&self.path);
            }
        }
    }
}
