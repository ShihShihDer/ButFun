//! ButFun — 蒸汽龐克太空歌劇療癒多人世界
//! Phase 0 權威伺服器骨架：靜態前端 + WebSocket 即時多人移動 + 遊戲內建議箱。
//!
//! 詳見 docs/GAME_DESIGN.md。

mod auth;
mod connections;
mod crops;
mod field;
mod game;
mod positions;
mod protocol;
mod state;
mod suggestions;
mod users;
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
use suggestions::{NewSuggestion, Suggestion};

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

    let app_state = AppState::new();
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
        .route(
            "/api/suggestions",
            post(post_suggestion).get(list_suggestions),
        )
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

/// 列出所有玩家建議（最新在前）。
async fn list_suggestions(State(app): State<AppState>) -> Json<Vec<Suggestion>> {
    Json(app.suggestions.list())
}
