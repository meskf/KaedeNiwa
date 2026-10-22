//! Web (WASM) 入口。

fn main() {
    console_error_panic_hook::set_once();
    // tracing-wasm 默认 TRACE：每次 VDOM diff 都会打一条巨大的 console 日志，
    // 调试构建下足以让页面卡死。Web 端只保留 INFO 及以上。
    let mut log_config = tracing_wasm::WASMLayerConfigBuilder::new();
    log_config.set_max_level(tracing::Level::INFO);
    tracing_wasm::set_as_global_default_with_config(log_config.build());
    dioxus::launch(kaede_ui::App);
}
