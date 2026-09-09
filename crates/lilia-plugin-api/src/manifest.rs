use std::ffi::c_char;

use serde::{Deserialize, Serialize};

pub const ABI_MAJOR: u16 = 1;
pub const ABI_MINOR: u16 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
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
