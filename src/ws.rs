//! WebSocket 連線處理：每名玩家一條連線。
//!
//! 流程：升級連線 → 等第一則 `Join` → 建立權威玩家 → 送 `Welcome` →
//! 一邊把廣播（快照 / 聊天）轉發給此客戶端，一邊讀取此客戶端的輸入更新權威狀態。

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use uuid::Uuid;

use crate::protocol::{ClientMsg, ServerMsg};
use crate::state::{AppState, Input, Player, WORLD_HEIGHT, WORLD_WIDTH};

pub async fn ws_handler(ws: WebSocketUpgrade, State(app): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, app))
}

async fn handle_socket(socket: WebSocket, app: AppState) {
    let (mut sender, mut receiver) = socket.split();

    // 等待第一則訊息，必須是 Join。
    let join = loop {
        match receiver.next().await {
            Some(Ok(Message::Text(text))) => {
                match serde_json::from_str::<ClientMsg>(&text) {
                    Ok(ClientMsg::Join { name, species }) => break (name, species),
                    Ok(_) => continue, // Join 之前忽略其他訊息
                    Err(e) => {
                        tracing::debug!("無法解析進場訊息：{e}");
                        continue;
                    }
                }
            }
            Some(Ok(Message::Close(_))) | None => return,
            Some(Ok(_)) => continue,
            Some(Err(_)) => return,
        }
    };

    let (name, species) = join;
    let id = Uuid::new_v4();
    let player = Player {
        id,
        name: sanitize(&name, "拓荒者"),
        // MVP 只有「地球人」起源，其他種族之後當資料逐個加。
        species: if species.trim().is_empty() {
            "terran".to_string()
        } else {
            species.trim().to_string()
        },
        x: WORLD_WIDTH / 2.0,
        y: WORLD_HEIGHT / 2.0,
        input: Input::default(),
    };

    {
        let mut players = app.players.write().unwrap();
        players.insert(id, player.clone());
    }
    tracing::info!(player = %player.name, %id, "玩家進場");

    // 先送 Welcome。
    let welcome = ServerMsg::Welcome {
        id,
        world: app.world_info(),
    };
    if let Ok(text) = serde_json::to_string(&welcome) {
        if sender.send(Message::Text(text)).await.is_err() {
            cleanup(&app, id).await;
            return;
        }
    }

    // 轉發任務：把廣播（快照 / 聊天）推給這個客戶端。
    let mut rx = app.tx.subscribe();
    let forward = tokio::spawn(async move {
        while let Ok(msg) = rx.recv().await {
            if sender.send(Message::Text(msg)).await.is_err() {
                break;
            }
        }
    });

    // 讀取迴圈：更新此玩家的輸入意圖、處理聊天。
    while let Some(Ok(msg)) = receiver.next().await {
        match msg {
            Message::Text(text) => match serde_json::from_str::<ClientMsg>(&text) {
                Ok(ClientMsg::Input {
                    up,
                    down,
                    left,
                    right,
                }) => {
                    if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                        p.input = Input {
                            up,
                            down,
                            left,
                            right,
                        };
                    }
                }
                Ok(ClientMsg::Chat { text }) => {
                    let text = text.trim();
                    if !text.is_empty() {
                        let chat = ServerMsg::Chat {
                            from: player.name.clone(),
                            text: text.chars().take(200).collect(),
                        };
                        if let Ok(json) = serde_json::to_string(&chat) {
                            let _ = app.tx.send(json);
                        }
                    }
                }
                Ok(ClientMsg::Join { .. }) => {} // 已進場，忽略
                Err(e) => tracing::debug!("無法解析客戶端訊息：{e}"),
            },
            Message::Close(_) => break,
            _ => {}
        }
    }

    forward.abort();
    cleanup(&app, id).await;
    tracing::info!(player = %player.name, %id, "玩家離線");
}

async fn cleanup(app: &AppState, id: Uuid) {
    app.players.write().unwrap().remove(&id);
    let left = ServerMsg::PlayerLeft { id };
    if let Ok(json) = serde_json::to_string(&left) {
        let _ = app.tx.send(json);
    }
}

/// 清理玩家輸入的名字：去頭尾空白、限制長度、空字串給預設。
fn sanitize(raw: &str, fallback: &str) -> String {
    let trimmed: String = raw.trim().chars().take(24).collect();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed
    }
}
