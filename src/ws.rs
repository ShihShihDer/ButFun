//! WebSocket 連線處理：每名玩家一條連線。
//!
//! 流程：升級連線 → 等第一則 `Join` → 建立權威玩家 → 送 `Welcome` →
//! 一邊把廣播（快照 / 聊天）轉發給此客戶端，一邊讀取此客戶端的輸入更新權威狀態。

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use uuid::Uuid;

use crate::auth::user_id_from_cookies;
use crate::field::{FarmOutcome, Field};
use crate::protocol::{ClientMsg, ServerMsg};
use crate::state::{AppState, Input, Player, WORLD_HEIGHT, WORLD_WIDTH};

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    State(app): State<AppState>,
) -> impl IntoResponse {
    // 在升級前先從 cookies 拿到已驗身的 user_id(若有)。
    let authed_uid = app
        .auth
        .as_ref()
        .and_then(|cfg| user_id_from_cookies(&headers, &cfg.session_secret));
    ws.on_upgrade(move |socket| handle_socket(socket, app, authed_uid))
}

async fn handle_socket(socket: WebSocket, app: AppState, authed_uid: Option<Uuid>) {
    let (mut sender, mut receiver) = socket.split();

    // 已登入 → player.id = user.id(同帳號重連即同玩家);name/species 從 user 來,可以
    // 直接建場、不必等客戶端 Join。
    // 訪客 → 等第一則 Join,uid 隨機(localStorage 名字僅在那個瀏覽器留)。
    let player = if let Some(uid) = authed_uid {
        let user = match app.users.get(uid) {
            Some(u) => u,
            None => return, // cookie 對得上但人不在了:直接斷
        };
        // 同帳號重連 → 回到離線前的位置（沒有歷史就地圖中央）。
        let (x, y) = crate::positions::spawn_at(app.positions.recall(user.id));
        Player {
            id: user.id,
            name: user.name,
            species: user.species,
            x,
            y,
            input: Input::default(),
            ether: 0,
        }
    } else {
        // 等 Join
        let join = loop {
            match receiver.next().await {
                Some(Ok(Message::Text(text))) => match serde_json::from_str::<ClientMsg>(&text) {
                    Ok(ClientMsg::Join { name, species }) => break (name, species),
                    Ok(_) => continue,
                    Err(e) => {
                        tracing::debug!("無法解析進場訊息：{e}");
                        continue;
                    }
                },
                Some(Ok(Message::Close(_))) | None => return,
                Some(Ok(_)) => continue,
                Some(Err(_)) => return,
            }
        };
        let (name, species) = join;
        Player {
            id: Uuid::new_v4(),
            name: crate::users::sanitize_name(&name),
            species: crate::users::sanitize_species(&species),
            x: WORLD_WIDTH / 2.0,
            y: WORLD_HEIGHT / 2.0,
            input: Input::default(),
            ether: 0,
        }
    };
    let id = player.id;

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
            cleanup(&app, id, authed_uid.is_some()).await;
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
                Ok(ClientMsg::Farm { x, y }) => {
                    // 點到農地外的座標 cell_at 會回 None，直接忽略。
                    if let Some((col, row)) = Field::cell_at(x, y) {
                        // 權威伺服器：只接受「玩家確實站在農地（或緊鄰邊緣）」的照顧動作，
                        // 不讓客戶端用任意座標隔空遙控這片共享農地。讀鎖在本句結束即釋放，
                        // 之後才取農地與玩家的寫鎖，避免互鎖。
                        let at_field = app
                            .players
                            .read()
                            .unwrap()
                            .get(&id)
                            .map(|p| crate::field::within_field_reach(p.x, p.y))
                            .unwrap_or(false);
                        if at_field {
                            let outcome = app.field.write().unwrap().interact(col, row);
                            if let FarmOutcome::Harvested(ether) = outcome {
                                if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                                    p.ether = p.ether.saturating_add(ether);
                                    tracing::info!(player = %p.name, ether = p.ether, "收成乙太");
                                }
                            }
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
    cleanup(&app, id, authed_uid.is_some()).await;
    tracing::info!(player = %player.name, %id, "玩家離線");
}

/// 玩家離線清理。`persist_pos` 為真（已登入玩家）時，先把最後位置記下來，
/// 讓同帳號下次重連回到原位。
async fn cleanup(app: &AppState, id: Uuid, persist_pos: bool) {
    let removed = app.players.write().unwrap().remove(&id);
    if persist_pos {
        if let Some(p) = removed {
            app.positions.remember(id, p.x, p.y);
        }
    }
    let left = ServerMsg::PlayerLeft { id };
    if let Ok(json) = serde_json::to_string(&left) {
        let _ = app.tx.send(json);
    }
}
