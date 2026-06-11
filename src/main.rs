//! ButFun — 蒸汽龐克太空歌劇療癒多人世界
//! Phase 0 權威伺服器骨架：靜態前端 + WebSocket 即時多人移動 + 遊戲內建議箱。
//!
//! 詳見 docs/GAME_DESIGN.md。

mod achievement;
mod active_skill;
mod auth;
mod class;
mod director;
mod combat;
mod connections;
mod equipment;
mod refinement;
mod crafting;
mod crops;
mod daynight;
mod daynight_store;
mod db;
mod dynamic_price;
mod economy;
mod enemy_field;
mod field;
mod field_store;
mod game;
mod gather;
mod gather_field;
mod guild;
mod market;
mod npc;
mod npc_chat;
mod inventory;
mod inventory_store;
mod tile_store;
mod tiles;
mod vitals;
mod land_plot;
mod land_plot_store;
mod plot_registry;
mod plots;
mod daily_quest;
mod positions;
mod profile;
mod protocol;
mod quest;
mod state;
mod suggestions;
mod tools;
mod users;
mod world_event;
mod ws;
mod pet;
mod fishing;
mod ranching;
mod farm_crops;
mod star_crystal;
mod trade_route;
mod workshop;
mod bounty_board;
mod expedition;
mod procurement;
mod farm_fair;
mod npc_lifecycle;
mod npc_schedule;
mod npc_memory_store;
mod npc_factions;
mod npc_needs;
mod npc_proactive;
mod npc_relations;
mod village_chief;
mod traveler_npc;
mod boss_roar;
mod plaza_talk;
mod npc_dawn_call;
mod npc_dusk_call;
mod npc_noon_bell;
mod world_log;
mod player_log;

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
    // 在啟動當下定錨 uptime 起點（LazyLock 首次存取才初始化，不在這摸一下會變成「第一次
    // 有人打 /api/status 才開始計時」）。
    let _ = *SERVER_START;

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "butfun_server=info,tower_http=warn".into()),
        )
        .init();

    // Phase 0-E 跨重啟持久化：有 DATABASE_URL 就連 Postgres、套 migration、把玩家位置
    // 載回；沒設則退回 JSONL/記憶體模式（見 db.rs / positions.rs）。連得到但 migration 失敗
    // 視為設定錯誤、直接中止（不要默默跑沒持久化的記憶體模式,免得又像換版洗檔那樣丟資料）。
    // 位置、背包、農地共用同一個連線池（PgPool 內部是 Arc,clone 便宜）：三個 store 各自獨立
    // 載回 / flush,沒有寫入順序耦合（見 0002_inventories.sql / 0003_fields.sql 為何不設外鍵）。
    let (positions, inventories, fields, daynight_store, users, suggestions, tile_store, land_plot_store, npc_memory_store) =
        match db::connect()
            .await
            .expect("Postgres 連線或 migration 失敗")
        {
            Some(pool) => {
                tracing::info!(
                    "Postgres 已連線、migration 已套用；\
                     玩家位置/背包/農地/日夜時刻/帳號/建議/地形差異/NPC記憶走 DB 持久化"
                );
                let positions = positions::PositionStore::from_pool(pool.clone()).await;
                let inventories = inventory_store::InventoryStore::from_pool(pool.clone()).await;
                let fields = field_store::FieldStore::from_pool(pool.clone()).await;
                let daynight_store = daynight_store::DayNightStore::from_pool(pool.clone()).await;
                let users = users::UserStore::from_pool(pool.clone()).await;
                let suggestions = suggestions::SuggestionStore::from_pool(pool.clone()).await;
                let tile_store = tile_store::TileStore::from_pool(pool.clone()).await;
                let land_plot_store = land_plot_store::LandPlotStore::from_pool(pool.clone()).await;
                let npc_memory_store = npc_memory_store::NpcMemoryStore::from_pool(pool).await;
                (positions, inventories, fields, daynight_store, users, suggestions, tile_store, land_plot_store, npc_memory_store)
            }
            None => {
                tracing::warn!(
                    "未設 DATABASE_URL；玩家位置/背包/農地/日夜時刻/帳號/建議/地形差異/NPC記憶走記憶體模式"
                );
                (
                    positions::PositionStore::new(),
                    inventory_store::InventoryStore::new(),
                    field_store::FieldStore::new(),
                    daynight_store::DayNightStore::new(),
                    users::UserStore::new(),
                    suggestions::SuggestionStore::new(),
                    tile_store::TileStore::new(),
                    land_plot_store::LandPlotStore::new(),
                    npc_memory_store::NpcMemoryStore::new(),
                )
            }
        };

    let app_state = AppState::with_stores(
        positions,
        inventories,
        fields,
        daynight_store,
        users,
        suggestions,
        tile_store,
        land_plot_store,
        npc_memory_store,
    );
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
        // 官網（/site/）的伺服器狀態小工具：只吐「線上人數 + 開機秒數」兩個彙總數字，
        // 不含任何玩家身分/位置資訊（公開端點，最小揭露原則）。
        .route("/api/status", get(api_status))
        // 官網即時世界小窗：吐「故鄉星球玩家的去識別化座標 + 城鎮幾何」，讓官網畫
        // 俯瞰活地圖（看得到有人在動）。只回座標數字、不含任何玩家身分（最小揭露）。
        .route("/api/worldview", get(api_worldview))
        // 登入相關路由
        .merge(auth::auth_router())
        // 個人資料編輯(改顯示名)——需登入,見 profile.rs
        .merge(profile::profile_router())
        // 其餘路徑交給靜態前端（web/）。
        .fallback_service(ServeDir::new("web"))
        .layer(TraceLayer::new_for_http())
        .with_state(app_state.clone());

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3000);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("無法綁定連接埠");
    tracing::info!("ButFun 伺服器啟動於 http://{addr}");

    // 優雅關機:收到 SIGTERM(deploy 重啟)或 Ctrl-C 時,先停收新連線,再把全部狀態最後
    // flush 一次,才退出。否則換版重啟會丟掉上次週期 flush 之後、線上玩家最多約 10 秒的進度
    // (見 game::flush_all)。flush 是冪等 upsert,多寫一次永遠安全。
    let flush_state = app_state.clone();
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("伺服器執行失敗");
    tracing::info!("收到關機訊號;退出前最後一次落地玩家狀態…");
    game::flush_all(&flush_state).await;
    tracing::info!("狀態已落地,伺服器關閉");
}

/// 等待關機訊號:Unix 上同時聽 SIGTERM(systemd/deploy 重啟用)與 Ctrl-C;
/// 非 Unix 只聽 Ctrl-C。任一觸發即返回,交還主流程做最後 flush。
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            // 裝不上 SIGTERM 處理器極罕見;退而只靠 Ctrl-C,別讓伺服器起不來。
            Err(_) => std::future::pending::<()>().await,
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
}

async fn health() -> &'static str {
    "ok"
}

/// 行程啟動時刻（算 uptime 用）。`LazyLock` 在 main 啟動早期第一次被讀到時定錨。
static SERVER_START: std::sync::LazyLock<std::time::Instant> =
    std::sync::LazyLock::new(std::time::Instant::now);

/// 官網狀態小工具用的公開彙總：線上人數 + 開機秒數。刻意不含玩家名單/位置等
/// 任何個體資訊（公開端點，最小揭露）。
async fn api_status(State(app): State<AppState>) -> impl IntoResponse {
    let online = app.players.read().map(|p| p.len()).unwrap_or(0);
    Json(serde_json::json!({
        "online": online,
        "uptime_secs": SERVER_START.elapsed().as_secs(),
    }))
}

/// 官網即時世界小窗的資料源。回故鄉星球（home）線上玩家的「去識別化座標」
/// （只有 x/y 數字，**不含 id / 名字 / 任何身分**——多人公開世界裡位置本就互相可見，
/// 這裡比照最小揭露只給點）＋ 城鎮幾何（世界像素的中心與半徑），讓官網畫俯瞰活地圖。
async fn api_worldview(State(app): State<AppState>) -> impl IntoResponse {
    let players: Vec<[f32; 2]> = app
        .players
        .read()
        .map(|m| {
            m.values()
                .filter(|p| p.planet == state::PLANET_HOME)
                .map(|p| [p.x, p.y])
                .collect()
        })
        .unwrap_or_default();
    let towns: Vec<serde_json::Value> = world_core::TOWNS
        .iter()
        .map(|t| {
            let px = (t.cgx as f32 + 0.5) * world_core::TILE_PX;
            let py = (t.cgy as f32 + 0.5) * world_core::TILE_PX;
            let half = t.half_tiles as f32 * world_core::TILE_PX;
            serde_json::json!({ "x": px, "y": py, "half": half, "name": t.name })
        })
        .collect();
    Json(serde_json::json!({ "players": players, "towns": towns }))
}

/// 收到一則玩家建議。內容清乾淨後若為空（全空白 / 全控制字元）回 400、不存——
/// 擋空的判斷下沉到 `add`（依實際會被存下的內容），不是只對 raw 輸入 `trim`。
async fn post_suggestion(
    State(app): State<AppState>,
    Json(new): Json<NewSuggestion>,
) -> impl IntoResponse {
    match app.suggestions.add(new).await {
        Some(saved) => (StatusCode::CREATED, Json(saved)).into_response(),
        None => (StatusCode::BAD_REQUEST, "建議內容不可為空").into_response(),
    }
}

// 註：刻意不再提供 `list_suggestions` HTTP handler——建議清單不對外公開（見上方路由註解）。
