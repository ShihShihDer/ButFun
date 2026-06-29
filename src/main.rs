//! ButFun — 蒸汽龐克太空歌劇療癒多人世界
//! Phase 0 權威伺服器骨架：靜態前端 + WebSocket 即時多人移動 + 遊戲內建議箱。
//!
//! 詳見 docs/GAME_DESIGN.md。

mod achievement;
mod active_skill;
mod affliction;
mod angler_bond;
mod auth;
mod class;
mod director;
mod combat;
mod compost;
mod guard;
mod dodge;
mod charged_strike;
mod wayfaring;
mod town_blocs;
mod town_share;
mod world_grove;
mod daily_recap;
mod craft_ceremony;
mod player_title;
mod activity_chain;
mod meditation;
mod busking;
mod busking_ensemble;
mod busking_repertoire;
mod kite;
mod firefly_lantern;
mod apiary;
mod companion_revive;
mod meal_buff;
mod meal_share;
mod dish_mastery;
mod onboarding;
mod idle_nudge;
mod town_project;
mod town_project_store;
mod visit_streak;
mod visit_streak_store;
mod welcome_kit;
mod welcome_kit_store;
mod observatory;
mod ether_surge;
mod meteor_shower;
mod night_aether_springs;
mod wandering_merchant;
mod wayposts;
mod bottle_drift;
mod campfire;
mod coop_build;
mod ship_repair;
mod snowman;
mod combat_mark;
mod connections;
mod equipment;
mod refinement;
mod crafting;
mod crop_demand;
mod crop_raid;
mod crop_rotation;
mod crop_variety;
mod crops;
mod daynight;
mod daynight_store;
mod db;
mod dynamic_price;
mod economy;
mod enemy_field;
mod field;
mod field_store;
mod field_thrive;
mod game;
mod gather;
mod gather_field;
mod guild;
mod market;
mod moon;
mod npc;
mod npc_chat;
mod npc_agent;
mod npc_agent_wire;
mod inventory;
mod inventory_store;
mod journey;
mod postcard;
mod postcard_mail;
mod tile_store;
mod tiles;
mod vehicle; // Phase 1-E 蒸汽載具 MVP·北極星「載具」垂直切片
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
// AI 生態世界 voxel 基底（切片①）：全隔離的方塊世界，並行於現有世界、互不干涉。
mod voxel;
mod voxel_ws;
mod pet;
mod pet_fetch;
mod pet_forage; // ROADMAP 484 寵物撈寶·把逗寵物接物接進羈絆→成長→回饋循環
mod pet_follow;
mod pet_greeting;
mod pet_personality;
mod pet_play;
mod fish_school;
mod fish_size;
mod fishing;
mod fishing_bite;
mod mining_vein;
mod prospecting; // ROADMAP 562 勘礦造詣·越掘越懂礦脈（採礦個人養成曲線）
mod cooking_steps;
mod aether_draw;
mod woodcutting;
mod skipstone; // ROADMAP 475 打水漂·水域第一個玩的動詞
mod skip_treasure; // ROADMAP 483 打水漂撈寶·把水漂接進經濟／風險循環
mod coop_labour; // ROADMAP 414 並肩協作·結伴勞動默契加成
mod constellation;
mod ancient_inscription;
mod field_guide;
mod terrain_atlas;
mod sky_codex;
mod ranching;
mod farm_crops;
mod star_crystal;
mod trade_route;
mod workshop;
mod bounty_board;
mod bounty_harvest;
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
mod boss_slam;
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
mod world_glimpse; // ROADMAP 445 世界此刻一瞥·登入畫面映出當下時辰/季節/天氣
mod player_log;
mod player_emote;
mod high_five;
mod emote_resonance;
mod player_cheer;
mod popularity_gathering;
mod weather;
mod wind; // ROADMAP 430 微風拂過微縮世界·世界級風場
mod rainbow;
mod reconcile;
mod return_hook;
mod region_name;
mod wayfinding;
mod social_dynamics;
mod soil_vitality;
mod friends;
mod party;
mod sprinkler;
mod village_well; // ROADMAP 640 禱告驅動·故鄉古井（應諾娃之禱，定時滋潤公田）
mod village_tea_stall; // ROADMAP 641 禱告驅動·故鄉茶棚（應露娜之禱，定時出爐熱茶溫暖全鎮）
mod resident_home; // ROADMAP 642 禱告驅動·居民木屋（應居民之禱，為他們蓋起溫暖的家）
mod harvest_festival; // ROADMAP 646 禱告驅動·豐收節慶典（應露娜之禱，廣場定期升起彩旗慶典）
mod field_spring;     // ROADMAP 647 禱告驅動·田邊清泉（應諾娃之禱，農田北坡天然清泉常流不息）
mod warehouse;
mod perishable;
mod home_interior;
mod home_furniture;
mod home_decor;
mod resident_npc;
mod resident_chat;
mod resident_bonds;
mod resident_care_back;
mod town_prosperity;
mod community_gathering;
mod season;
mod seasonal_harvest_award;
mod session_champions;
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
mod item_rarity;
mod element_affinity;
mod kill_streak;
mod weakpoint;
mod world_tally; // ROADMAP 495 今日世界戰報·廣場石板第一次有了「全服今天做了什麼」
mod world_tally_milestone; // ROADMAP 498 全服里程碑喝采——計數突破門檻時廣場 NPC 鼓舞一句
mod world_wonder; // ROADMAP 524 世界奇觀首探·五處隱藏秘境散落世界各角
mod world_boss;   // ROADMAP 525 世界守護者降臨·超強守護者周期現身荒野，協力擊敗全服皆獎
mod rain_regen;   // ROADMAP 496 草原細雨庇護·天氣首次影響戰鬥——細雨中戶外玩家緩緩回血
mod rain_harvest; // ROADMAP 502 雨天豐澤·細雨中收成每株多 +1 乙太
mod gold_rush;         // ROADMAP 521 黃金礦脈爭奪戰·每 30 分鐘週期性競技採礦事件
mod auction;           // ROADMAP 522 星際拍賣行·每 2 小時全服競標傳說遺物
mod fishing_contest;   // ROADMAP 523 萬尾釣魚大賽·每 45 分鐘全服釣魚競速，比總體長
mod monument;          // ROADMAP 526 旅人紀念碑·廣場石碑銘記守護者首殺/奇觀首探/釣魚冠軍/礦脈冠軍
mod guardian_blessing; // ROADMAP 533 守護者元素祝福·擊敗守護者的參戰玩家獲元素光環持續 2 小時

use std::net::SocketAddr;

use axum::extract::State;
use axum::http::header;
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
    let (positions, inventories, fields, daynight_store, users, suggestions, tile_store, land_plot_store, npc_memory_store, friends, guilds, sprinkler_persist, sprinkler_preload, tp_store, visit_streaks, welcome_kits) =
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
                let visit_streaks = visit_streak_store::VisitStreakStore::from_pool(pool.clone()).await;
                let welcome_kits = welcome_kit_store::WelcomeKitStore::from_pool(pool.clone()).await;
                let (sp, sp_rows) = sprinkler::SprinklerPersist::from_pool(pool).await;
                (positions, inventories, fields, daynight_store, users, suggestions, tile_store, land_plot_store, npc_memory_store, friends, guilds, sp, sp_rows, tp_store, visit_streaks, welcome_kits)
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
                    visit_streak_store::VisitStreakStore::new(),
                    welcome_kit_store::WelcomeKitStore::new(),
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
        visit_streaks,
        welcome_kits,
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
        // 首頁與 index.html：經後端動態注入 game.js 的內容雜湊版本，並回 no-cache，
        // 讓前端部署後玩家立刻拿到新版（根治「快取卡 4h」——見 serve_index）。
        // 必須放在 fallback_service(ServeDir) 之前才會優先命中。
        .route("/", get(serve_index))
        .route("/index.html", get(serve_index))
        // 3D 試驗場頁（/3d/、/play3d/）的 index 同樣回 no-cache，根治玩家被快取卡住舊
        // 前端的問題（過去走 ServeDir，index 無 no-cache → 卡住舊 main.js?v=）。
        // main.js 仍交給下方 ServeDir，維持 ?v= 版本快取。必須放在 fallback 之前才優先命中。
        .route("/3d/", get(serve_3d_index))
        .route("/3d/index.html", get(serve_3d_index))
        .route("/play3d/", get(serve_play3d_index))
        .route("/play3d/index.html", get(serve_play3d_index))
        // AI 生態世界 voxel 基底（切片①）：新頁 /voxel/ + 獨立 WS /voxel/ws，全隔離、
        // additive，與現有 2D/3D 協定零交集（見 voxel.rs / voxel_ws.rs）。
        .route("/voxel/", get(serve_voxel_index))
        .route("/voxel/index.html", get(serve_voxel_index))
        .route("/voxel/ws", get(voxel_ws::voxel_ws_handler))
        // 其餘路徑（game.js、assets、wasm…）交給靜態前端（web/）。game.js 維持可
        // 快取——它的 URL 帶內容雜湊，內容一變 URL 就變，CF/瀏覽器自然抓新版。
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

/// 把 HTML 裡的 `game.js?v=...` 版本字串換成「game.js 內容的 sha256 前 12 個 hex 字元」。
///
/// 根治「前端部署後玩家約 4h 看不到新版」的快取 bug：原本 index.html 寫死
/// `game.js?v=20260610-leaderboard`（手動、卡在 6/10、沒人更新），而 `/` 走純
/// `ServeDir` 靜態 serve、URL 不隨 game.js 內容變→Cloudflare(HIT,max-age 14400)＋
/// 瀏覽器快取會持續送舊的 game.js。改成內容雜湊後，game.js 內容一變版本字串就變→
/// URL 一變→CF/瀏覽器自然抓新版，立刻到位。
///
/// 穩健替換：找每一處 `game.js?v=` 後面到下一個 `"` 為止那段，整段換成新雜湊；
/// 找不到 `game.js?v=` 就原樣返回（不硬塞）。抽成純函式好測。
fn inject_gamejs_version(html: &str, gamejs: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(gamejs);
    // 取前 12 個 hex 字元（6 bytes）當版本——夠長到不會碰撞、夠短到 URL 乾淨。
    let version: String = digest.iter().take(6).map(|b| format!("{b:02x}")).collect();

    let needle = "game.js?v=";
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(pos) = rest.find(needle) {
        // 寫入 needle（含）之前的內容 + needle 本身。
        let after = pos + needle.len();
        out.push_str(&rest[..after]);
        // 從 needle 之後找下一個 `"`，那之間是舊版本字串，整段換成新雜湊。
        let tail = &rest[after..];
        match tail.find('"') {
            Some(q) => {
                out.push_str(&version);
                rest = &tail[q..]; // 保留 `"` 起繼續掃（可能有多處）
            }
            // 沒有結尾 `"`（理論上不會）：保守起見原樣接上、停止替換。
            None => {
                out.push_str(tail);
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

/// 啟動時算一次並快取的首頁 HTML（game.js 版本已換成內容雜湊）。
/// 用 `LazyLock` 確保只讀檔/算雜湊一次，避免每請求摸大檔。
/// server cwd＝repo 根，相對路徑 `web/game.js`、`web/index.html` 可讀；deploy 重啟
/// server → 每次部署自動重算雜湊。讀檔失敗時退回原樣 index.html（不 panic、不擋服務）。
static INDEX_HTML: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    let html = match std::fs::read_to_string("web/index.html") {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("讀 web/index.html 失敗，serve_index 將回空白：{e}");
            return String::new();
        }
    };
    match std::fs::read("web/game.js") {
        Ok(gamejs) => {
            let injected = inject_gamejs_version(&html, &gamejs);
            tracing::info!("serve_index：已把 index.html 的 game.js 版本注入為內容雜湊");
            injected
        }
        // 讀不到 game.js（理論上不會）：退回原樣 index.html，至少首頁能出。
        Err(e) => {
            tracing::warn!("讀 web/game.js 失敗，index.html 沿用原版本字串：{e}");
            html
        }
    }
});

/// 首頁 handler：回「已注入 game.js 內容雜湊」的 index.html，並帶 no-cache 標頭。
/// HTML 永遠新鮮（極小、無快取）；game.js 的 URL 隨內容雜湊變，照舊可被快取。
async fn serve_index() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache, must-revalidate"),
        ],
        INDEX_HTML.as_str(),
    )
}

/// 把 3D 頁 HTML 裡的 `main.js?v=...` 版本字串換成 main.js 內容的 sha256 前 12 hex，
/// 並在 `</head>` 前插入 `<script>window.__BUILD__="<hash>";</script>`，
/// 讓前端 JS 可在 `?debug=1` 的偵錯 HUD 裡顯示版本號，一眼確認是最新版。
///
/// 邏輯與 `inject_gamejs_version` 完全對稱：同樣算 sha256、同樣只換 `?v=` 後到 `"` 之間；
/// 差別僅在 needle 是 `main.js?v=`，並額外注入 `window.__BUILD__`。
/// 抽成純函式便於測試（不碰磁碟）。
fn inject_mainjs_version(html: &str, mainjs: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(mainjs);
    // 前 12 hex（6 bytes）——與 game.js 雜湊長度一致，夠長不碰撞、夠短 URL 乾淨。
    let version: String = digest.iter().take(6).map(|b| format!("{b:02x}")).collect();

    // 第一步：替換所有 main.js?v=<舊版> 為 main.js?v=<hash>（邏輯同 inject_gamejs_version）。
    let needle = "main.js?v=";
    let mut out = String::with_capacity(html.len() + 80);
    let mut rest = html;
    while let Some(pos) = rest.find(needle) {
        let after = pos + needle.len();
        out.push_str(&rest[..after]);
        let tail = &rest[after..];
        match tail.find('"') {
            Some(q) => {
                out.push_str(&version);
                rest = &tail[q..]; // 保留 `"` 繼續掃（避免漏掉多處）
            }
            None => {
                out.push_str(tail);
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);

    // 第二步：在 </head> 前插入 window.__BUILD__，供前端偵錯 HUD 讀版本號。
    let build_tag = format!("<script>window.__BUILD__=\"{version}\";</script>");
    if let Some(pos) = out.find("</head>") {
        out.insert_str(pos, &build_tag);
    }
    out
}

/// `/3d/`、`/3d/index.html` 的 handler：每次請求即時讀檔並注入 main.js 內容雜湊版本。
///
/// 根治「前端改動後玩家看到舊版」的問題：
/// - 舊做法：啟動時讀 index.html 一次（LazyLock）+ 手動 `?v=N`
///   → 改前端不重啟看不到新版；手動版本號忘記改就永久卡舊版。
/// - 新做法：每次請求讀 index.html + 算 web/3d/main.js 的 sha256，
///   把 `main.js?v=<任何舊值>` 換成 `main.js?v=<雜湊>`，並注入 `window.__BUILD__`；
///   前端改了、雜湊就變、URL 就變、CF/瀏覽器就抓新版——無需重啟伺服器。
/// index.html 帶 no-cache，main.js 本體仍走 ServeDir 可被快取。
async fn serve_3d_index() -> impl IntoResponse {
    let html = match std::fs::read_to_string("web/3d/index.html") {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("讀 web/3d/index.html 失敗：{e}");
            String::new()
        }
    };
    let body = match std::fs::read("web/3d/main.js") {
        Ok(mainjs) => {
            tracing::debug!("serve_3d_index：已把 index.html 的 main.js 版本注入為內容雜湊");
            inject_mainjs_version(&html, &mainjs)
        }
        Err(e) => {
            tracing::warn!("讀 web/3d/main.js 失敗，index.html 沿用原版本字串：{e}");
            html
        }
    };
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache, must-revalidate"),
        ],
        body,
    )
}

/// `/play3d/`、`/play3d/index.html` 的 handler：同 `serve_3d_index`，對 web/play3d/main.js 算雜湊。
async fn serve_play3d_index() -> impl IntoResponse {
    let html = match std::fs::read_to_string("web/play3d/index.html") {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("讀 web/play3d/index.html 失敗：{e}");
            String::new()
        }
    };
    let body = match std::fs::read("web/play3d/main.js") {
        Ok(mainjs) => {
            tracing::debug!("serve_play3d_index：已把 index.html 的 main.js 版本注入為內容雜湊");
            inject_mainjs_version(&html, &mainjs)
        }
        Err(e) => {
            tracing::warn!("讀 web/play3d/main.js 失敗，index.html 沿用原版本字串：{e}");
            html
        }
    };
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache, must-revalidate"),
        ],
        body,
    )
}

/// `/voxel/`、`/voxel/index.html` 的 handler：同 `serve_3d_index`，對 web/voxel/main.js 算雜湊。
/// AI 生態世界 voxel 基底的新前端頁，與現有頁完全並行、互不影響。
async fn serve_voxel_index() -> impl IntoResponse {
    let html = match std::fs::read_to_string("web/voxel/index.html") {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("讀 web/voxel/index.html 失敗：{e}");
            String::new()
        }
    };
    let body = match std::fs::read("web/voxel/main.js") {
        Ok(mainjs) => {
            tracing::debug!("serve_voxel_index：已把 index.html 的 main.js 版本注入為內容雜湊");
            inject_mainjs_version(&html, &mainjs)
        }
        Err(e) => {
            tracing::warn!("讀 web/voxel/main.js 失敗，index.html 沿用原版本字串：{e}");
            html
        }
    };
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache, must-revalidate"),
        ],
        body,
    )
}

/// 行程啟動時刻（算 uptime 用）。`LazyLock` 在 main 啟動早期第一次被讀到時定錨。
static SERVER_START: std::sync::LazyLock<std::time::Instant> =
    std::sync::LazyLock::new(std::time::Instant::now);

/// 官網狀態小工具用的公開彙總：線上人數 + 開機秒數。刻意不含玩家名單/位置等
/// 任何個體資訊（公開端點，最小揭露）。
async fn api_status(State(app): State<AppState>) -> impl IntoResponse {
    let online = app.players.read().map(|p| p.len()).unwrap_or(0);
    // ROADMAP 445：彙整「世界此刻」一瞥（時辰／季節／天氣），讓登入畫面映出當下世界。
    // 全是全域世界狀態（公開、本就互相可見），不含任何玩家身分／座標，守最小揭露。
    let phase = app
        .daynight
        .read()
        .map(|d| d.phase())
        .unwrap_or(crate::daynight::Phase::Day);
    let season = app
        .season
        .read()
        .map(|s| s.current)
        .unwrap_or(crate::season::Season::Spring);
    let weather = app
        .weather
        .read()
        .map(|w| w.weather_type)
        .unwrap_or(crate::weather::WeatherType::Clear);
    let glimpse = crate::world_glimpse::compose(phase, season, weather, online);
    Json(serde_json::json!({
        "online": online,
        "uptime_secs": SERVER_START.elapsed().as_secs(),
        "glimpse": {
            "theme": glimpse.theme,
            "headline": glimpse.headline,
            "subline": glimpse.subline,
        },
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

#[cfg(test)]
mod tests {
    use super::{inject_gamejs_version, inject_mainjs_version};

    /// sha256(content) 前 12 hex 字元——測試用的期望版本算法（與函式一致）。
    fn expected_version(gamejs: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        Sha256::digest(gamejs)
            .iter()
            .take(6)
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    #[test]
    fn 替換舊版本字串為內容雜湊() {
        let html = r#"<html><body><script src="game.js?v=20260610-leaderboard"></script></body></html>"#;
        let gamejs = b"console.log('hello butfun');";
        let out = inject_gamejs_version(html, gamejs);

        let ver = expected_version(gamejs);
        // 新版本字串應出現，舊的應消失。
        assert!(out.contains(&format!("game.js?v={ver}")), "應注入內容雜湊版本: {out}");
        assert!(!out.contains("20260610-leaderboard"), "舊版本字串應被換掉: {out}");
        // 雜湊取 12 個 hex 字元。
        assert_eq!(ver.len(), 12);
    }

    #[test]
    fn 雜湊隨內容變而變() {
        let html = r#"<script src="game.js?v=old"></script>"#;
        let a = inject_gamejs_version(html, b"version A");
        let b = inject_gamejs_version(html, b"version B");
        assert_ne!(a, b, "不同 game.js 內容應產生不同版本字串");

        // 同內容應穩定（同 HTML 同內容 → 同輸出）。
        let a2 = inject_gamejs_version(html, b"version A");
        assert_eq!(a, a2, "相同內容應產生相同版本字串");
    }

    #[test]
    fn 替換多處且保留其餘html() {
        let html = r#"<a href="game.js?v=x">a</a> mid <script src="game.js?v=y"></script>"#;
        let gamejs = b"abc";
        let out = inject_gamejs_version(html, gamejs);
        let ver = expected_version(gamejs);
        // 兩處都換成同一雜湊。
        let count = out.matches(&format!("game.js?v={ver}")).count();
        assert_eq!(count, 2, "兩處 game.js?v= 都應被替換: {out}");
        // 其餘文字（mid）原樣保留。
        assert!(out.contains(" mid "), "非版本內容應保留: {out}");
    }

    #[test]
    fn 沒有版本字串時原樣返回() {
        let html = "<html>no script here</html>";
        let out = inject_gamejs_version(html, b"whatever");
        assert_eq!(out, html, "沒有 game.js?v= 應原樣返回");
    }

    // ---- inject_mainjs_version 測試 ----

    #[test]
    fn mainjs_替換舊版本字串為內容雜湊() {
        let html = r#"<html><head></head><body><script type="module" src="main.js?v=17"></script></body></html>"#;
        let mainjs = b"console.log('butfun 3d');";
        let out = inject_mainjs_version(html, mainjs);
        let ver = expected_version(mainjs);
        // main.js?v= 應被換成內容雜湊。
        assert!(out.contains(&format!("main.js?v={ver}")), "應注入內容雜湊版本: {out}");
        assert!(!out.contains("?v=17"), "舊版本字串應被換掉: {out}");
        // window.__BUILD__ 應被注入。
        assert!(out.contains(&format!("window.__BUILD__=\"{ver}\"")), "應注入 window.__BUILD__: {out}");
        // 注入點在 </head> 之前。
        let build_pos = out.find("window.__BUILD__").expect("找不到 __BUILD__");
        let head_pos = out.find("</head>").expect("找不到 </head>");
        assert!(build_pos < head_pos, "__BUILD__ 應在 </head> 之前: {out}");
    }

    #[test]
    fn mainjs_雜湊隨內容變而變() {
        let html = r#"<html><head></head><body><script type="module" src="main.js?v=1"></script></body></html>"#;
        let a = inject_mainjs_version(html, b"version A");
        let b = inject_mainjs_version(html, b"version B");
        assert_ne!(a, b, "不同 main.js 內容應產生不同版本字串");
    }

    #[test]
    fn mainjs_沒有版本字串時仍注入build標籤() {
        // 沒有 main.js?v= 的 HTML：版本替換跳過，但 __BUILD__ 仍應注入（有 </head>）。
        let html = "<html><head></head><body>no script</body></html>";
        let out = inject_mainjs_version(html, b"js content");
        let ver = expected_version(b"js content");
        // 沒有 main.js?v= 可替換，原樣通過。
        assert!(!out.contains("main.js?v="), "沒有 main.js?v= 不應憑空插入: {out}");
        // __BUILD__ 仍應注入。
        assert!(out.contains(&format!("window.__BUILD__=\"{ver}\"")), "即使無 main.js?v= 也應注入 __BUILD__: {out}");
    }
}
