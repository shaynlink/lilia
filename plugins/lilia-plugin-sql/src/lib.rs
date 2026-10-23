#![allow(unsafe_code)]

use lilia_plugin_api::{Capability, PluginDescriptorV1, SqlDescriptorV1};

static PLUGIN_NAME: &[u8] = b"lilia-sql\0";
static SQL_NAME: &[u8] = b"sqlite-standard\0";

static SQL_DESCRIPTOR: SqlDescriptorV1 = SqlDescriptorV1 {
    name: SQL_NAME.as_ptr().cast(),
    init: None,
    validate_query: None,
};

#[no_mangle]
pub extern "C" fn lilia_plugin_v1() -> PluginDescriptorV1 {
    PluginDescriptorV1::with_sql(
        PLUGIN_NAME.as_ptr().cast(),
        Capability::Sql.bit(),
        &raw const SQL_DESCRIPTOR,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_declares_sql_capability() {
        let descriptor = lilia_plugin_v1();
        assert_eq!(descriptor.capabilities, Capability::Sql.bit());
        assert!(!descriptor.sql.is_null());
    }
}
