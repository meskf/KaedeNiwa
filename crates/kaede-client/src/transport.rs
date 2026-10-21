//! 网络抽象 + 实现。
//!
//! - [`MockTransport`]：内存，测试用
//! - [`WebSocketTransport`]：native 用 tokio-tungstenite，wasm 用 web-sys

#![cfg_attr(target_arch = "wasm32", allow(clippy::type_complexity))]
// wasm 分支：Rc<RefCell<Option<Closure<dyn FnMut(...)>>>> 是 wasm-bindgen 标准模式

use async_trait::async_trait;
use kaede_protocol::{ClientMessage, ServerMessage};

use crate::error::ClientError;

/// 单个连接的收发接口。
#[async_trait]
pub trait Transport: Send + Sync {
    async fn send(&self, msg: ClientMessage) -> Result<(), ClientError>;
    async fn recv(&self) -> Result<ServerMessage, ClientError>;
    async fn close(&self) -> Result<(), ClientError>;
}

// ====================================================================
// MockTransport
// ====================================================================

use async_channel::{Receiver, Sender};
use std::sync::Mutex;

pub struct MockTransport {
    outbound: Sender<ClientMessage>,
    inbound: Mutex<Receiver<ServerMessage>>,
}

impl MockTransport {
    pub fn new() -> Self {
        let (out_tx, _out_rx) = async_channel::unbounded();
        let (_in_tx, in_rx) = async_channel::unbounded();
        Self {
            outbound: out_tx,
            inbound: Mutex::new(in_rx),
        }
    }

    pub fn pair() -> (Self, MockServerSide) {
        let (c2s_tx, c2s_rx) = async_channel::unbounded();
        let (s2c_tx, s2c_rx) = async_channel::unbounded();
        let client = Self {
            outbound: c2s_tx,
            inbound: Mutex::new(s2c_rx),
        };
        let server = MockServerSide {
            rx: c2s_rx,
            tx: s2c_tx,
        };
        (client, server)
    }
}

pub struct MockServerSide {
    rx: Receiver<ClientMessage>,
    tx: Sender<ServerMessage>,
}

impl MockServerSide {
    pub async fn recv(&self) -> Option<ClientMessage> {
        self.rx.recv().await.ok()
    }

    pub async fn send(&self, msg: ServerMessage) -> Result<(), ClientError> {
        self.tx
            .send(msg)
            .await
            .map_err(|e| ClientError::Transport(e.to_string()))
    }
}

impl Default for MockTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Transport for MockTransport {
    async fn send(&self, msg: ClientMessage) -> Result<(), ClientError> {
        self.outbound
            .send(msg)
            .await
            .map_err(|e| ClientError::Transport(e.to_string()))
    }

    async fn recv(&self) -> Result<ServerMessage, ClientError> {
        let rx = self
            .inbound
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        rx.recv().await.map_err(|_| ClientError::Closed)
    }

    async fn close(&self) -> Result<(), ClientError> {
        self.outbound.close();
        self.inbound
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .close();
        Ok(())
    }
}

// ====================================================================
// WebSocketTransport —— native
// ====================================================================

#[cfg(not(target_arch = "wasm32"))]
pub use native_ws::WebSocketTransport;

#[cfg(not(target_arch = "wasm32"))]
mod native_ws {
    use super::*;
    use futures_util::stream::{SplitSink, SplitStream};
    use futures_util::{SinkExt, StreamExt};
    use kaede_protocol::WireMessage;
    use tokio::net::TcpStream;
    use tokio::sync::Mutex;
    use tokio_tungstenite::tungstenite::Message;
    use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

    type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

    pub struct WebSocketTransport {
        sink: Mutex<SplitSink<WsStream, Message>>,
        source: Mutex<SplitStream<WsStream>>,
    }

    impl WebSocketTransport {
        pub async fn connect(url: &str) -> Result<Self, ClientError> {
            let (ws, _) = connect_async(url)
                .await
                .map_err(|e| ClientError::Transport(format!("ws connect {}: {}", url, e)))?;
            let (sink, source) = ws.split();
            Ok(Self {
                sink: Mutex::new(sink),
                source: Mutex::new(source),
            })
        }
    }

    #[async_trait]
    impl Transport for WebSocketTransport {
        async fn send(&self, msg: ClientMessage) -> Result<(), ClientError> {
            let bytes = msg
                .encode()
                .map_err(|e| ClientError::Protocol(e.to_string()))?;
            let mut sink = self.sink.lock().await;
            sink.send(Message::Binary(bytes))
                .await
                .map_err(|e| ClientError::Transport(format!("ws send: {}", e)))
        }

        async fn recv(&self) -> Result<ServerMessage, ClientError> {
            let mut source = self.source.lock().await;
            loop {
                match source.next().await {
                    Some(Ok(Message::Binary(b))) => {
                        return ServerMessage::decode(&b)
                            .map_err(|e| ClientError::Protocol(e.to_string()));
                    }
                    Some(Ok(Message::Text(_))) => {
                        // 与 server 端对称：只接受 Binary CBOR。
                        return Err(ClientError::Protocol(
                            "text frames not supported; use binary CBOR".into(),
                        ));
                    }
                    Some(Ok(Message::Ping(_)))
                    | Some(Ok(Message::Pong(_)))
                    | Some(Ok(Message::Frame(_))) => continue,
                    Some(Ok(Message::Close(_))) | None => return Err(ClientError::Closed),
                    Some(Err(e)) => {
                        return Err(ClientError::Transport(format!("ws recv: {}", e)));
                    }
                }
            }
        }

        async fn close(&self) -> Result<(), ClientError> {
            let mut sink = self.sink.lock().await;
            let _ = sink.close().await;
            Ok(())
        }
    }
}

// ====================================================================
// WebSocketTransport —— wasm
// ====================================================================

#[cfg(target_arch = "wasm32")]
pub use wasm_ws::WebSocketTransport;

#[cfg(target_arch = "wasm32")]
mod wasm_ws {
    use super::*;
    use kaede_protocol::WireMessage;
    use std::cell::RefCell;
    use std::rc::Rc;
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;
    use web_sys::{BinaryType, MessageEvent, WebSocket};

    type InMsg = Result<Vec<u8>, String>;

    pub struct WebSocketTransport {
        ws: WebSocket,
        incoming: async_channel::Receiver<InMsg>,
        _onopen: Rc<RefCell<Option<Closure<dyn FnMut()>>>>,
        _onmessage: Rc<RefCell<Option<Closure<dyn FnMut(MessageEvent)>>>>,
        _onerror: Rc<RefCell<Option<Closure<dyn FnMut()>>>>,
        _onclose: Rc<RefCell<Option<Closure<dyn FnMut()>>>>,
    }

    // WASM 单线程；async_trait 要求 Send + Sync。
    // ⚠️ 若未来迁移到 wasm-bindgen-rayon 多线程，必须重新审查此 impl。
    unsafe impl Send for WebSocketTransport {}
    unsafe impl Sync for WebSocketTransport {}

    impl WebSocketTransport {
        /// P0-7：连接并**等待 open**（最多 10s）。
        ///
        /// 旧实现在 CONNECTING 状态就返回，调用方立即 send(ClientHello) 会抛
        /// `InvalidStateError`。新实现等 open 成功 / error / 超时。
        pub async fn connect(url: &str) -> Result<Self, ClientError> {
            let ws = WebSocket::new(url)
                .map_err(|e| ClientError::Transport(format!("WebSocket::new: {:?}", e)))?;
            ws.set_binary_type(BinaryType::Arraybuffer);

            let (in_tx, in_rx) = async_channel::unbounded::<InMsg>();
            let (open_tx, open_rx) = async_channel::bounded::<()>(1);
            let (err_tx, err_rx) = async_channel::bounded::<String>(1);

            // onopen
            let onopen = Closure::<dyn FnMut()>::new({
                let tx = open_tx.clone();
                move || {
                    let _ = tx.try_send(());
                }
            });
            ws.set_onopen(Some(onopen.as_ref().unchecked_ref()));

            // onmessage
            let onmessage = Closure::<dyn FnMut(MessageEvent)>::new({
                let in_tx = in_tx.clone();
                move |e: MessageEvent| {
                    let data = e.data();
                    if let Ok(ab) = data.clone().dyn_into::<js_sys::ArrayBuffer>() {
                        let arr = js_sys::Uint8Array::new(&ab);
                        let _ = in_tx.try_send(Ok(arr.to_vec()));
                    } else if let Some(s) = data.as_string() {
                        let preview: String = s.chars().take(64).collect();
                        let _ = in_tx.try_send(Err(format!(
                            "text frames not supported; use binary CBOR (got: {})",
                            preview
                        )));
                    }
                }
            });
            ws.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));

            // onerror
            let onerror = Closure::<dyn FnMut()>::new({
                let err_tx = err_tx.clone();
                let in_tx = in_tx.clone();
                move || {
                    let _ = err_tx.try_send("websocket error event".to_string());
                    in_tx.close();
                }
            });
            ws.set_onerror(Some(onerror.as_ref().unchecked_ref()));

            // onclose
            let onclose = Closure::<dyn FnMut()>::new({
                let err_tx = err_tx.clone();
                let in_tx = in_tx.clone();
                move || {
                    let _ = err_tx.try_send("websocket closed before open".to_string());
                    in_tx.close();
                }
            });
            ws.set_onclose(Some(onclose.as_ref().unchecked_ref()));

            // 等 open / error / 10s 超时（浏览器对不可达主机可能长时间不回调）
            let wait = futures_lite::future::or(
                futures_lite::future::or(
                    async { open_rx.recv().await.ok().map(|_| Ok::<(), String>(())) },
                    async { err_rx.recv().await.ok().map(Err::<(), String>) },
                ),
                async {
                    gloo_timers::future::TimeoutFuture::new(10_000).await;
                    Some(Err::<(), String>("ws open timeout (10s)".into()))
                },
            )
            .await;

            match wait {
                Some(Ok(())) => {}
                Some(Err(e)) => {
                    return Err(ClientError::Connection(format!("ws open failed: {}", e)));
                }
                None => {
                    return Err(ClientError::Connection("ws open wait cancelled".into()));
                }
            }

            if ws.ready_state() != WebSocket::OPEN {
                return Err(ClientError::Connection(format!(
                    "ws not open after wait (ready_state={})",
                    ws.ready_state()
                )));
            }

            drop(in_tx);

            Ok(Self {
                ws,
                incoming: in_rx,
                _onopen: Rc::new(RefCell::new(Some(onopen))),
                _onmessage: Rc::new(RefCell::new(Some(onmessage))),
                _onerror: Rc::new(RefCell::new(Some(onerror))),
                _onclose: Rc::new(RefCell::new(Some(onclose))),
            })
        }
    }

    #[async_trait]
    impl Transport for WebSocketTransport {
        async fn send(&self, msg: ClientMessage) -> Result<(), ClientError> {
            let bytes = msg
                .encode()
                .map_err(|e| ClientError::Protocol(e.to_string()))?;
            self.ws
                .send_with_u8_array(bytes.as_slice())
                .map_err(|e| ClientError::Transport(format!("ws send: {:?}", e)))
        }

        async fn recv(&self) -> Result<ServerMessage, ClientError> {
            match self.incoming.recv().await {
                Ok(Ok(bytes)) => {
                    ServerMessage::decode(&bytes).map_err(|e| ClientError::Protocol(e.to_string()))
                }
                Ok(Err(e)) => Err(ClientError::Protocol(e)),
                Err(_) => Err(ClientError::Closed),
            }
        }

        async fn close(&self) -> Result<(), ClientError> {
            let _ = self.ws.close();
            Ok(())
        }
    }
}

// ====================================================================
// 测试
// ====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_pair_roundtrip() {
        let (client, server) = MockTransport::pair();
        client.send(ClientMessage::Join).await.unwrap();
        let got = server.recv().await.unwrap();
        assert!(matches!(got, ClientMessage::Join));
        server
            .send(ServerMessage::Joined { peers: vec![] })
            .await
            .unwrap();
        let got = client.recv().await.unwrap();
        assert!(matches!(got, ServerMessage::Joined { .. }));
    }
}
