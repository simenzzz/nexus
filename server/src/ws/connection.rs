use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsMessage {
    ChatMessage {
        channel_id: String,
        content: String,
    },
    Typing {
        channel_id: String,
        user_id: String,
    },
    Presence {
        user_id: String,
        status: String,
    },
    Join {
        channel_id: String,
    },
    Leave {
        channel_id: String,
    },
}

pub async fn handle_ws_upgrade(ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(handle_socket)
}

async fn handle_socket(mut socket: WebSocket) {
    tracing::info!("WebSocket client connected");

    while let Some(Ok(msg)) = socket.next().await {
        match msg {
            Message::Text(text) => {
                match serde_json::from_str::<WsMessage>(&text) {
                    Ok(ws_msg) => {
                        tracing::debug!(?ws_msg, "Received WS message");
                        // TODO: Route message to appropriate room actor
                    }
                    Err(err) => {
                        tracing::warn!(%err, "Invalid WS message format");
                        let error = serde_json::json!({ "error": "Invalid message format" });
                        if socket
                            .send(Message::Text(error.to_string().into()))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    tracing::info!("WebSocket client disconnected");
}
