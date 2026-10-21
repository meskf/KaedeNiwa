//! KaedeNiwa Dioxus UI（重做版 v2）。

mod app;
mod attachment;
mod content;
mod css;
mod dm_view;
mod friends_view;
mod i18n;
mod icons;
mod login;
mod logo;
mod member_picker;
mod members;
mod mobile_pages;
mod mobile_shell;
mod mobile_voice;
mod observer;
mod search;
mod server_modal;
mod settings_modal;
mod shell;
mod sidebar;
mod state;
mod theme;
mod video_picker;
#[cfg(target_arch = "wasm32")]
mod voice_web;

pub use app::App;
pub use state::View;
pub use theme::Mode;
