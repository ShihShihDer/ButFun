//! ButFun — 蒸汽龐克太空歌劇療癒多人世界
//! Phase 0 權威伺服器骨架：靜態前端 + WebSocket 即時多人移動 + 遊戲內建議箱。
//!
//! 詳見 docs/GAME_DESIGN.md。

mod auth;
mod connections;
mod crafting;
mod crops;
mod daynight;
mod db;
mod field;
mod game;
mod gather;
mod gather_field;
mod inventory;
mod plot_registry;
mod plots;
mod positions;
mod protocol;
mod state;
mod suggestions;
mod tools;
mod users;
mod vehicle;
mod ws;

use std::net::SocketAddr;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;

use state::AppState;
use suggestions::NewSuggestion;

#[tokio::main]
async fn main() {
    // 開發/正式上線都從 .env 載入秘密(systemd 會用 EnvironmentFile,本機 cargo run 用 dotenvy)。
    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "butfun_server=info,tower_http=warn".into()),
        )
        .init();

    // Phase 0-E 跨重啟持久化：有 DATABASE_URL 就連 Postgres、套 migration、把玩家位置
    // 載回；沒設則退回 JSONL/記憶體模式（見 db.rs / positions.rs）。連得到但 migration 失敗
    // 視為設定錯誤、直接中止（不要默默跑沒持久化的記憶體模式,免得又像換版洗檔那樣丟資料）。
    let positions = match db::connect().await.expect("Postgres 連線或 migration 失敗") {
        Some(pool) => {
            tracing::info!("Postgres 已連線、migration 已套用；玩家位置走 DB 持久化");
            positions::PositionStore::from_pool(pool).await
        }
        None => {
            tracing::warn!("未設 DATABASE_URL；玩家位置走 JSONL 退回層（本機/測試模式）");
            positions::PositionStore::new()
        }
    };

    let app_state = AppState::with_positions(positions);
    if app_state.auth.is_some() {
        tracing::info!("Google OAuth 已啟用(/auth/google/start)");
    } else {
        tracing::warn!("Google OAuth 未設定;走訪客模式(設好 GOOGLE_CLIENT_ID/SECRET/REDIRECT_URI/BUTFUN_SESSION_SECRET 即啟用)");
    }

    // 啟動權威遊戲迴圈。
    game::spawn(app_state.clone());

    let app = Router::new()
        .route("/healthz", get(health))
        .route("/ws", get(ws::ws_handler))
        // 只收建議（POST），刻意不開公開的 GET 清單：建議是玩家送回的回饋（含自選
        // 署名），維護者本就直接讀 `data/suggestions.jsonl` 三角化。先前
        // `GET /api/suggestions` 是未驗身公開端點，會把全部玩家建議整包吐給任何人，
        // 而前端從不消費它（`web/game.js` 只 POST）——等於線上一個沒人用卻能被任意
        // `curl` 撈走所有玩家回饋（含自填署名）的資料曝露點。移除以收口；日後若要做
        // 後台檢視，再走驗身（見 `SuggestionStore::list`）。
        .route("/api/suggestions", post(post_suggestion))
        // 登入相關路由
        .merge(auth::auth_router())
        // 其餘路徑交給靜態前端（web/）。
        .fallback_service(ServeDir::new("web"))
        .layer(TraceLayer::new_for_http())
        .with_state(app_state);

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3000);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("無法綁定連接埠");
    tracing::info!("ButFun 伺服器啟動於 http://{addr}");

    axum::serve(listener, app)
        .await
        .expect("伺服器執行失敗");
}

async fn health() -> &'static str {
    "ok"
}

/// 收到一則玩家建議。內容清乾淨後若為空（全空白 / 全控制字元）回 400、不存——
/// 擋空的判斷下沉到 `add`（依實際會被存下的內容），不是只對 raw 輸入 `trim`。
async fn post_suggestion(
    State(app): State<AppState>,
    Json(new): Json<NewSuggestion>,
) -> impl IntoResponse {
    match app.suggestions.add(new) {
        Some(saved) => (StatusCode::CREATED, Json(saved)).into_response(),
        None => (StatusCode::BAD_REQUEST, "建議內容不可為空").into_response(),
    }
}

// 註：刻意不再提供 `list_suggestions` HTTP handler——建議清單不對外公開（見上方路由註解）。
