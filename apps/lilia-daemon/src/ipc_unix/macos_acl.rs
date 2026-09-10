//! Narrow bindings to macOS sys/acl.h: reject grants instead of attempting to
//! reinterpret ACL principals, inheritance or permission ordering.
#![allow(unsafe_code)]

use std::ffi::{c_char, c_int, c_uint, c_void, CString};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

const ACL_TYPE_EXTENDED: c_uint = 0x100;
const ACL_EXTENDED_DENY: c_uint = 2;
const ACL_FIRST_ENTRY: c_int = 0;
const ACL_NEXT_ENTRY: c_int = -1;

extern "C" {
    fn acl_get_file(path: *const c_char, kind: c_uint) -> *mut c_void;
    fn acl_valid(acl: *mut c_void) -> c_int;
    fn acl_get_entry(acl: *mut c_void, entry_id: c_int, entry: *mut *mut c_void) -> c_int;
    fn acl_get_tag_type(entry: *mut c_void, tag: *mut c_uint) -> c_int;
    fn acl_free(object: *mut c_void) -> c_int;
}

struct Acl(*mut c_void);

impl Drop for Acl {
    fn drop(&mut self) {
        // SAFETY: this non-null allocation came from acl_get_file, is owned by
        // this guard, and no entry pointer survives the guard.
        unsafe {
            acl_free(self.0);
        }
    }
}

pub(super) fn reject_grants(path: &Path) -> io::Result<()> {
    let encoded_path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid ACL path"))?;
    // SAFETY: path is NUL terminated and live during the call; the documented
    // macOS extended ACL selector matches sys/acl.h.
    let raw = unsafe { acl_get_file(encoded_path.as_ptr(), ACL_TYPE_EXTENDED) };
    if raw.is_null() {
        let error = io::Error::last_os_error();
        // macOS filesec_get_property also reports ENOENT when an existing
        // object's ACL property is absent. Never confuse this with a missing
        // object or a symlink: metadata must still confirm a real file/directory.
        if error.kind() == io::ErrorKind::NotFound {
            let metadata = std::fs::symlink_metadata(path)?;
            if metadata.is_file() || metadata.is_dir() {
                return Ok(());
            }
        }
        return Err(error);
    }
    let acl = Acl(raw);
    // SAFETY: acl owns a live ACL allocation.
    if unsafe { acl_valid(acl.0) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let mut selector = ACL_FIRST_ENTRY;
    loop {
        let mut entry = std::ptr::null_mut();
        // SAFETY: acl is valid, selector is FIRST or NEXT, entry is a writable
        // pointer slot; returned entry descriptors remain owned by acl.
        if unsafe { acl_get_entry(acl.0, selector, &raw mut entry) } != 0 {
            let error = io::Error::last_os_error();
            // macOS returns EINVAL at end of a valid ACL (including empty ACLs),
            // unlike the POSIX/Linux iteration convention.
            return if error.raw_os_error() == Some(rustix::io::Errno::INVAL.raw_os_error()) {
                Ok(())
            } else {
                Err(error)
            };
        }
        let mut tag = 0;
        // SAFETY: successful get_entry produced an entry in the live ACL; tag
        // is an initialized, aligned output slot of acl_tag_t's enum ABI.
        if unsafe { acl_get_tag_type(entry, &raw mut tag) } != 0 {
            return Err(io::Error::last_os_error());
        }
        if tag != ACL_EXTENDED_DENY {
            return Err(super::denied("IPC paths must not contain macOS ACL grants"));
        }
        selector = ACL_NEXT_ENTRY;
    }
}
