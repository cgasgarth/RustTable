wasmtime::component::bindgen!({
    path: "wit",
    world: "extension",
    imports: { default: async },
    exports: { default: async },
    require_store_data_send: true,
});
