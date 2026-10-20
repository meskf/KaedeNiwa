//! Web (WASM) 入口。

fn main() {
    console_error_panic_hook::set_once();
    tracing_wasm::set_as_global_default();
    dioxus::launch(kaede_ui::App);
}
