//! 系统服务管理器（systemd / SCM）检测 + 命令生成。
//!
//! 面板不直接执行 sudo 命令，只生成命令让用户复制到终端执行。
//! 这样避开 root 权限问题，也符合"默认安全"原则。

use serde::Serialize;

use crate::error::ApiError;

#[derive(Serialize)]
pub struct ServiceStatus {
    pub platform: String,
    pub unit_exists: bool,
    pub current_mode: String,
    pub hint: String,
}

pub fn detect() -> ServiceStatus {
    #[cfg(target_os = "linux")]
    {
        let unit_path = "/etc/systemd/system/kaede-server.service";
        let exists = std::path::Path::new(unit_path).exists();
        let (mode, hint) = if exists {
            let content = std::fs::read_to_string(unit_path).unwrap_or_default();
            let mode = if content.contains("Restart=always") {
                "keep_alive"
            } else if content.contains("Restart=no") {
                "on_demand"
            } else {
                "unknown"
            };
            (
                mode.to_string(),
                "点击「切换模式」获取需要执行的命令".to_string(),
            )
        } else {
            (
                "not_installed".to_string(),
                "尚未安装 systemd unit。请先用面板生成的模板创建 unit 文件。".to_string(),
            )
        };
        ServiceStatus {
            platform: "linux".to_string(),
            unit_exists: exists,
            current_mode: mode,
            hint,
        }
    }
    #[cfg(target_os = "windows")]
    {
        ServiceStatus {
            platform: "windows".to_string(),
            unit_exists: false,
            current_mode: "unknown".to_string(),
            hint: "请使用 sc.exe 配置 Windows 服务；MVP 暂不做 SCM 自动探测".to_string(),
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        ServiceStatus {
            platform: "other".to_string(),
            unit_exists: false,
            current_mode: "unsupported".to_string(),
            hint: "此平台暂不支持面板管理服务模式".to_string(),
        }
    }
}

pub fn command_for_mode(mode: &str) -> Result<(String, String), ApiError> {
    match mode {
        "on_demand" | "keep_alive" => {
            #[cfg(target_os = "linux")]
            {
                let restart = if mode == "keep_alive" { "always" } else { "no" };
                let cmd = format!(
                    "# 1) 编辑 /etc/systemd/system/kaede-server.service，把 [Service] 段里的 Restart= 行改成：\n                     #    Restart={restart}\n                     # 2) 执行以下命令：\n                     sudo systemctl daemon-reload\n                     sudo systemctl restart kaede-server\n                     # 3) 验证：\n                     systemctl show kaede-server -p Restart",
                    restart = restart
                );
                Ok((cmd, "需要 sudo 权限执行".to_string()))
            }
            #[cfg(not(target_os = "linux"))]
            {
                let _ = mode;
                Err(ApiError::BadRequest(
                    "此平台暂不支持自动切换服务模式".into(),
                ))
            }
        }
        _ => Err(ApiError::BadRequest(
            "mode must be 'on_demand' or 'keep_alive'".into(),
        )),
    }
}
