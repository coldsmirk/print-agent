use std::sync::{Arc, Mutex};

use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

use crate::config::AppConfig;
use crate::printer;

#[derive(Debug, Deserialize)]
struct PrintRequest {
    doc_type: String,
    file_data: Option<String>,
    file_url: Option<String>,
    file_format: Option<String>,
    duplex: Option<bool>,
    copies: Option<u32>,
}

#[derive(Debug, Serialize)]
struct PrintResponse {
    success: bool,
    message: String,
}

pub async fn run(config: Arc<Mutex<AppConfig>>) {
    let port = config.lock().unwrap().port;
    let addr = format!("127.0.0.1:{port}");

    let listener = match TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("Failed to bind WebSocket server on {addr}: {e}");
            return;
        }
    };
    tracing::info!("WebSocket server listening on ws://{addr}");

    while let Ok((stream, peer)) = listener.accept().await {
        tracing::info!("New connection from {peer}");
        let config = config.clone();

        tokio::spawn(async move {
            let ws = match tokio_tungstenite::accept_async(stream).await {
                Ok(ws) => ws,
                Err(e) => {
                    tracing::error!("WebSocket handshake failed: {e}");
                    return;
                }
            };

            let (mut write, mut read) = ws.split();

            while let Some(Ok(msg)) = read.next().await {
                match msg {
                    Message::Text(text) => {
                        let response = handle_print_request(&text, &config).await;
                        let resp_json = serde_json::to_string(&response).unwrap();
                        if write.send(Message::Text(resp_json.into())).await.is_err() {
                            break;
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }

            tracing::info!("Connection from {peer} closed");
        });
    }
}

async fn handle_print_request(text: &str, config: &Arc<Mutex<AppConfig>>) -> PrintResponse {
    let req: PrintRequest = match serde_json::from_str(text) {
        Ok(r) => r,
        Err(e) => {
            return PrintResponse {
                success: false,
                message: format!("Invalid JSON: {e}"),
            };
        }
    };

    tracing::info!(
        "Print request: doc_type={}, has_file_data={}, has_file_url={}, duplex={:?}, copies={:?}",
        req.doc_type,
        req.file_data.is_some(),
        req.file_url.is_some(),
        req.duplex,
        req.copies,
    );

    let (printer_name, default_settings) = {
        let cfg = config.lock().unwrap();
        match cfg.bindings.iter().find(|b| b.doc_type == req.doc_type) {
            Some(b) => (b.printer.clone(), b.settings.clone()),
            None => {
                return PrintResponse {
                    success: false,
                    message: format!("未找到文档类型 '{}' 的打印机配置", req.doc_type),
                };
            }
        }
    };

    let duplex = req.duplex.unwrap_or(default_settings.duplex);
    let copies = req.copies.unwrap_or(default_settings.copies);

    let file_bytes = if let Some(data) = &req.file_data {
        match base64::engine::general_purpose::STANDARD.decode(data) {
            Ok(bytes) => bytes,
            Err(e) => {
                return PrintResponse {
                    success: false,
                    message: format!("file_data Base64 解码失败: {e}"),
                };
            }
        }
    } else if let Some(url) = &req.file_url {
        match fetch_file(url).await {
            Ok(bytes) => bytes,
            Err(e) => {
                return PrintResponse {
                    success: false,
                    message: format!("获取文件失败: {e}"),
                };
            }
        }
    } else {
        return PrintResponse {
            success: false,
            message: "未提供 file_data 或 file_url".into(),
        };
    };

    let file_format = req.file_format.as_deref().unwrap_or("PDF");

    match printer::print_document(&printer_name, &file_bytes, file_format, duplex, copies) {
        Ok(()) => {
            tracing::info!("Print job submitted to {printer_name}");
            PrintResponse {
                success: true,
                message: format!("打印任务已提交到 {printer_name}"),
            }
        }
        Err(e) => {
            tracing::error!("Print failed: {e}");
            PrintResponse {
                success: false,
                message: format!("打印失败: {e}"),
            }
        }
    }
}

async fn fetch_file(url: &str) -> anyhow::Result<Vec<u8>> {
    if url.starts_with("http://") || url.starts_with("https://") {
        let resp = reqwest::get(url).await?;
        Ok(resp.bytes().await?.to_vec())
    } else {
        Ok(tokio::fs::read(url).await?)
    }
}
