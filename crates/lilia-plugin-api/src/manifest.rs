use std::ffi::c_char;

use serde::{Deserialize, Serialize};

pub const ABI_MAJOR: u16 = 1;
pub const ABI_MINOR: u16 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Kv,
    Json,
    Sql,
    Document,
    Graph,
    Codec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub abi_major: u16,
    pub abi_minor: u16,
    pub engine_requirement: String,
    pub target: String,
    pub library: String,
    pub library_sha256: String,
    pub capabilities: Vec<Capability>,
}

impl PluginManifest {
    /// Canonical compact JSON bytes used as the Ed25519 signing payload.
    ///
    /// # Errors
    /// Returns an error only if serialization of this fixed schema fails.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }
}

impl Capability {
    #[must_use]
    pub const fn bit(self) -> u64 {
        match self {
            Self::Kv => 1 << 0,
            Self::Json => 1 << 1,
            Self::Sql => 1 << 2,
            Self::Document => 1 << 3,
            Self::Graph => 1 << 4,
            Self::Codec => 1 << 5,
        }
    }
}

/// Exact Rust target triple supported by this distribution build.
#[must_use]
pub const fn host_target() -> &'static str {
    if cfg!(all(target_arch = "aarch64", target_os = "macos")) {
        "aarch64-apple-darwin"
    } else if cfg!(all(target_arch = "x86_64", target_os = "macos")) {
        "x86_64-apple-darwin"
    } else if cfg!(all(
        target_arch = "aarch64",
        target_os = "linux",
        target_env = "gnu"
    )) {
        "aarch64-unknown-linux-gnu"
    } else if cfg!(all(
        target_arch = "x86_64",
        target_os = "linux",
        target_env = "gnu"
    )) {
        "x86_64-unknown-linux-gnu"
    } else if cfg!(all(
        target_arch = "x86_64",
        target_os = "windows",
        target_env = "msvc"
    )) {
        "x86_64-pc-windows-msvc"
    } else {
        "unsupported-target"
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct PluginDescriptorV1 {
    pub abi_major: u16,
    pub abi_minor: u16,
    pub capabilities: u64,
    pub name: *const c_char,
}

impl PluginDescriptorV1 {
    pub const fn new(name: *const c_char, capabilities: u64) -> Self {
        Self {
            abi_major: ABI_MAJOR,
            abi_minor: ABI_MINOR,
            capabilities,
            name,
        }
    }
}
