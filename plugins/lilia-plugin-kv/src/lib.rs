use lilia_plugin_api::PluginDescriptorV1;

const NAME: &[u8] = b"lilia-kv\0";

#[no_mangle]
pub extern "C" fn lilia_plugin_v1() -> PluginDescriptorV1 {
    PluginDescriptorV1::new(NAME.as_ptr().cast(), 1)
}
