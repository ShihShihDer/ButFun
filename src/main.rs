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
mod town_project;
mod town_project_store;
mod observatory;
mod meteor_shower;
mod night_aether_springs;
mod wandering_merchant;
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
mod moon;
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
mod appearance;
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
mod pet_follow;
mod fishing;
mod field_guide;
mod terrain_atlas;
mod sky_codex;
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
mod npc_gather;
mod npc_needs;
mod npc_proactive;
mod npc_relations;
mod village_chief;
mod traveler_npc;
mod boss_roar;
mod boss_ai;
mod plaza_talk;
mod npc_dawn_call;
mod npc_dusk_call;
mod npc_noon_bell;
mod npc_night_watch;
mod daytime_talk;
mod lunch_chatter;
mod lunch_gift;
mod lunch_regular;
mod npc_bounty;
mod npc_defeat_reaction;
mod npc_level_greet;
mod npc_recognition;
mod npc_commission;
mod npc_expedition_boost;
mod npc_workshop_boost;
mod npc_treasury;
mod npc_deal;
mod npc_stock;
mod supply_chain;
mod world_log;
mod player_log;
mod player_emote;
mod high_five;
mod emote_resonance;
mod player_cheer;
mod popularity_gathering;
mod weather;
mod friends;
mod party;
mod sprinkler;
mod warehouse;
mod perishable;
mod home_interior;
mod home_furniture;
mod resident_npc;
mod resident_chat;
mod town_prosperity;
mod community_gathering;
mod season;
mod seasonal_nodes;
mod wildlife;
mod species_relations;
mod stat_points;
mod skill_mastery;
mod civic_vote;
mod town_memory;
mod invasion;
mod monster_colony;
mod eco_pressure;
mod eco_report;
mod eco_bounty;
mod eco_festival;

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
    let (positions, inventories, fields, daynight_store, users, suggestions, tile_store, land_plot_store, npc_memory_store, friends, guilds, sprinkler_persist, sprinkler_preload, tp_store) =
        match db::connect()
            .await
            .expect("Postgres 連線或 migration 失敗")
        {
            Some(pool) => {
                tracing::info!(
                    "Postgres 已連線、migration 已套用；\
                     玩家位置/背包/農地/日夜時刻/帳號/建議/地形差異/NPC記憶/好友/公會/灑水器/工程走 DB 持久化"
                );
                let positions = positions::PositionStore::from_pool(pool.clone()).await;
                let inventories = inventory_store::InventoryStore::from_pool(pool.clone()).await;
                let fields = field_store::FieldStore::from_pool(pool.clone()).await;
                let daynight_store = daynight_store::DayNightStore::from_pool(pool.clone()).await;
                let users = users::UserStore::from_pool(pool.clone()).await;
                let suggestions = suggestions::SuggestionStore::from_pool(pool.clone()).await;
                let tile_store = tile_store::TileStore::from_pool(pool.clone()).await;
                let land_plot_store = land_plot_store::LandPlotStore::from_pool(pool.clone()).await;
                let npc_memory_store = npc_memory_store::NpcMemoryStore::from_pool(pool.clone()).await;
                let friends = friends::FriendStore::from_pool(pool.clone()).await;
                let guilds = guild::GuildStore::from_pool(pool.clone()).await;
                let tp_store = town_project_store::TownProjectStore::from_pool(pool.clone()).await;
                let (sp, sp_rows) = sprinkler::SprinklerPersist::from_pool(pool).await;
                (positions, inventories, fields, daynight_store, users, suggestions, tile_store, land_plot_store, npc_memory_store, friends, guilds, sp, sp_rows, tp_store)
            }
            None => {
                tracing::warn!(
                    "未設 DATABASE_URL；玩家位置/背包/農地/日夜時刻/帳號/建議/地形差異/NPC記憶/好友/公會/灑水器/工程走記憶體模式"
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
                    friends::FriendStore::new(),
                    guild::GuildStore::new(),
                    sprinkler::SprinklerPersist::new(),
                    vec![],
                    town_project_store::TownProjectStore::new(),
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
        friends,
        guilds,
        sprinkler_persist,
        sprinkler_preload,
        tp_store,
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
        // 經濟儀表（ROADMAP 108）：商隊金庫餘額 + 注入/支付累計統計；
        // 只彙總數字、不含個資（公開端點，供維護者調參）。
        .route("/api/economy", get(api_economy))
        // 登入相關路由
        .merge(auth::auth_router())
        // 個人資料編輯(改顯示名)——需登入,見 profile.rs
        .merge(profile::profile_router())
        // 外觀自訂(捏臉)——需登入,見 appearance.rs
        .merge(appearance::appearance_router())
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

/// 經濟儀表（ROADMAP 108）：彙總商隊金庫與乙太流量資訊，供維護者調參用。
/// 只回彙總數字，不含玩家身分或個別玩家乙太（最小揭露原則）。
async fn api_economy(State(app): State<AppState>) -> impl IntoResponse {
    let snap = app.npc_treasury.read().unwrap().snapshot();
    let online = app.players.read().map(|p| p.len()).unwrap_or(0);
    // 線上玩家乙太總量（匿名加總，不含身分）
    let online_ether_total: u64 = app.players.read()
        .map(|p| p.values().map(|pl| pl.ether as u64).sum())
        .unwrap_or(0);
    let uptime_secs = SERVER_START.elapsed().as_secs();

    let treasury: serde_json::Value = {
        let mut m = serde_json::Map::new();
        for (name, balance, max) in &snap.merchants {
            m.insert(name.to_string(), serde_json::json!({ "balance": balance, "max": max }));
        }
        m.into()
    };

    let net = snap.lifetime_injected as i64
        - snap.lifetime_paid_to_players as i64
        - snap.lifetime_supply_cost as i64;

    Json(serde_json::json!({
        "treasury": treasury,
        "faucet": {
            "lifetime_injected": snap.lifetime_injected,
            "restock_interval_secs": crate::npc_treasury::RESTOCK_INTERVAL_SECS,
        },
        "drain": {
            "lifetime_paid_to_players": snap.lifetime_paid_to_players,
            "lifetime_supply_cost": snap.lifetime_supply_cost,
        },
        "net_ether_delta": net,
        "online_players": online,
        "online_ether_total": online_ether_total,
        "uptime_secs": uptime_secs,
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
/// 建議箱每 IP 速率限制（H3 安全強化）：防匿名腳本無限 POST 灌爆 suggestions 表 / 撐爆磁碟。
/// Cloudflare tunnel 後真實 IP 在 `CF-Connecting-IP`；近似計數（每分鐘窗、每 IP ≤ 3 則）。
fn suggest_rate_ok(ip: &str) -> bool {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static RL: OnceLock<Mutex<HashMap<String, (u64, u32)>>> = OnceLock::new();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let min = now / 60;
    let mut map = RL.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap();
    if map.len() > 20000 {
        map.clear(); // 防 map 無限長大
    }
    let e = map.entry(ip.to_string()).or_insert((min, 0));
    if e.0 != min {
        *e = (min, 0);
    }
    e.1 += 1;
    e.1 <= 3
}

async fn post_suggestion(
    State(app): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(new): Json<NewSuggestion>,
) -> impl IntoResponse {
    // H3：每 IP 速率限制。Cloudflare tunnel 後真實 IP 在 CF-Connecting-IP（退而求其次 X-Forwarded-For）。
    let ip = headers
        .get("cf-connecting-ip")
        .or_else(|| headers.get("x-forwarded-for"))
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(',').next().unwrap_or(s).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    if !suggest_rate_ok(&ip) {
        return (StatusCode::TOO_MANY_REQUESTS, "建議送太頻繁了，請稍後再試").into_response();
    }
    match app.suggestions.add(new).await {
        Some(saved) => (StatusCode::CREATED, Json(saved)).into_response(),
        None => (StatusCode::BAD_REQUEST, "建議內容不可為空").into_response(),
    }
}

// 註：刻意不再提供 `list_suggestions` HTTP handler——建議清單不對外公開（見上方路由註解）。
