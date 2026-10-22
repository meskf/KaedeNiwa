//! 内存日志环形缓冲。
//!
//! 挂一个 tracing layer 到 subscriber，把每条 event 推进 ring buffer。
//! 管理面板的 /api/logs 从这里读，避免读文件 + 不用管日志轮转。

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

const MAX_LOG_ENTRIES: usize = 500;

/// 该 target 下的日志不进入环形缓冲（用于一条必须打印但不应经 /api/logs 扩散的日志）。
pub const SECRET_TARGET: &str = "kaede_admin_token";

#[derive(Clone, Serialize)]
pub struct LogEntry {
    pub ts: i64,
    pub level: String,
    pub target: String,
    pub message: String,
}

#[derive(Clone)]
pub struct LogBuffer {
    inner: Arc<Mutex<VecDeque<LogEntry>>>,
}

impl LogBuffer {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(VecDeque::with_capacity(MAX_LOG_ENTRIES))),
        }
    }

    pub fn push(&self, entry: LogEntry) {
        let mut q = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if q.len() >= MAX_LOG_ENTRIES {
            q.pop_front();
        }
        q.push_back(entry);
    }

    pub fn snapshot(&self, limit: usize) -> Vec<LogEntry> {
        let q = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let n = q.len().min(limit);
        q.iter().rev().take(n).cloned().collect()
    }

    pub fn layer(&self) -> LogLayer {
        LogLayer {
            inner: self.inner.clone(),
        }
    }
}

impl Default for LogBuffer {
    fn default() -> Self {
        Self::new()
    }
}

pub struct LogLayer {
    inner: Arc<Mutex<VecDeque<LogEntry>>>,
}

impl<S> Layer<S> for LogLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        if event.metadata().target() == SECRET_TARGET {
            return;
        }
        struct MessageVisitor {
            message: String,
        }
        impl Visit for MessageVisitor {
            fn record_str(&mut self, field: &Field, value: &str) {
                if field.name() == "message" {
                    self.message = value.to_string();
                }
            }
            fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
                if field.name() == "message" && self.message.is_empty() {
                    self.message = format!("{:?}", value);
                }
            }
        }
        let mut visitor = MessageVisitor {
            message: String::new(),
        };
        event.record(&mut visitor);
        if visitor.message.is_empty() {
            return;
        }
        let entry = LogEntry {
            ts: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0),
            level: event.metadata().level().to_string(),
            target: event.metadata().target().to_string(),
            message: visitor.message,
        };
        let mut q = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if q.len() >= MAX_LOG_ENTRIES {
            q.pop_front();
        }
        q.push_back(entry);
    }
}
