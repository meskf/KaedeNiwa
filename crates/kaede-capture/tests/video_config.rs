//! C-5 回归测试：VideoConfig 的 width/height 字段。
//!
//! 这些测试不依赖 GStreamer runtime —— 只验证 struct 构造与默认值。

use kaede_capture::VideoConfig;

#[test]
fn video_config_default_keeps_native_resolution() {
    let cfg = VideoConfig::default();
    assert_eq!(cfg.width, None, "default must keep native resolution");
    assert_eq!(cfg.height, None, "default must keep native resolution");
}

#[test]
fn video_config_with_custom_resolution() {
    let cfg = VideoConfig {
        width: Some(1280),
        height: Some(720),
        ..Default::default()
    };
    assert_eq!(cfg.width, Some(1280));
    assert_eq!(cfg.height, Some(720));
    assert_eq!(cfg.fps, 60, "other fields should stay default");
}

#[test]
fn video_config_partial_resolution() {
    // 只有 width 或只有 height：不触发 videoscale
    let cfg = VideoConfig {
        width: Some(1280),
        ..Default::default()
    };
    assert_eq!(cfg.width, Some(1280));
    assert_eq!(cfg.height, None);
}
