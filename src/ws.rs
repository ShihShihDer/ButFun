//! WebSocket 連線處理：每名玩家一條連線。
//!
//! 流程：升級連線 → 等第一則 `Join` → 建立權威玩家 → 送 `Welcome` →
//! 一邊把廣播（快照 / 聊天）轉發給此客戶端，一邊讀取此客戶端的輸入更新權威狀態。

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use tokio::sync::broadcast::error::RecvError;
use uuid::Uuid;

use crate::auth::user_id_from_cookies;
use crate::dynamic_price::unix_secs;
use crate::field::{FarmOutcome, Field};
use crate::market::MarketListing;
use crate::npc;
use crate::protocol::{ClientMsg, ServerMsg};
use crate::state::{AppState, Input, Player, WORLD_HEIGHT, WORLD_WIDTH};
use world_core;

/// 一則聊天訊息的最長字元數。聊天會廣播給所有玩家，這條是「公開輸入邊界」的集中
/// 常數（對齊建議內容 1000 / 署名 24 / 玩家名 24 的同類上限）。
const MAX_CHAT_CHARS: usize = 200;

/// 整理一則進來的聊天輸入：先濾掉控制字元（換行 / 歸位 / NUL 等——聊天是單行輸入，
/// 這些只會來自壞客戶端，放行會讓廣播出多行或破壞顯示／偽造介面的內容）、去頭尾空白、
/// 依「字元」(非位元組，中文不被切壞)截到上限。清乾淨後變空（全空白 / 全控制字元）回
/// `None`，呼叫端據此不廣播空訊息。抽成純函式以便測試，與訪客名字 / 建議的輸入加固一致。
fn sanitize_chat(text: &str) -> Option<String> {
    let cleaned: String = text
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .chars()
        .take(MAX_CHAT_CHARS)
        .collect();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

/// 轉發迴圈從 broadcast 收訊息時遇到錯誤，該繼續還是收掉這條連線。
#[derive(Debug, PartialEq, Eq)]
enum ForwardAction {
    /// 跳過、繼續轉發後續廣播。
    Skip,
    /// 結束轉發、收掉這條連線。
    Stop,
}

/// 把一個 broadcast `RecvError` 分類成轉發迴圈的動作。抽成純函式以便測試。
///
/// `Lagged` 只代表「這個客戶端一時跟不上廣播速度」（手機網路抖、分頁切到背景），
/// tokio 已替它丟掉最舊的快照、之後 `recv` 會接著給最新的——跳過繼續轉即可，
/// **不該因此把玩家踢下線**（對一個手機上玩的療癒多人世界尤其重要）。下一則
/// 快照 15 分之一秒就到，畫面自然追回，無需重連。
/// 只有 `Closed`（伺服器端關了廣播頻道、要收攤）才結束轉發。
fn forward_action(err: &RecvError) -> ForwardAction {
    match err {
        RecvError::Lagged(_) => ForwardAction::Skip,
        RecvError::Closed => ForwardAction::Stop,
    }
}

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
        // 同帳號重連 → 回到離線前的位置與乙太(沒有歷史就地圖中央、乙太 0)。
        // 真正的 recall **延後到 players 寫鎖內**(見下方 acquire 區塊),避免和
        // cleanup 的 remember 之間出現 race window(refresh 時舊連線 cleanup 與
        // 新連線進場兩個 async 任務交錯,recall 若在鎖外搶先跑會拿到 None,
        // 玩家被瞬移回地圖中央)。此處只是建占位 Player,位置/乙太會在鎖內覆寫。
        Player {
            id: user.id,
            name: user.name,
            species: user.species,
            x: WORLD_WIDTH / 2.0,
            y: WORLD_HEIGHT / 2.0,
            input: Input::default(),
            ether: 0,
            inventory: crate::inventory::Inventory::new(),
            vitals: crate::vitals::Vitals::new(),
            wallet: crate::economy::PlotWallet::new(),
            attack_cooldown: 0.0,
            exp: 0,
            codex: 0,
            atlas: 0,
            skylog: 0,
            cheers: 0,
            planet: crate::state::PLANET_HOME.to_string(),
            masteries: crate::class::Masteries::new(),
            // 重連還原：工會成員資料 keyed by uid 存在 GuildStore，登入玩家重連時從中還原
            // 工會標籤——否則一刷新就「看起來不在工會」（guild_tag 被建成 None，已知 bug）。
            guild_tag: app.guilds.tag_of(user.id),
            party_id: None,
            hair_style: user.hair_style,
            skin_tone: user.skin_tone,
            goggle_color: user.goggle_color,
            costume: user.costume,
            achievements: crate::achievement::AchievementSet::new(),
            kill_count: 0,
            refine_attempt_count: 0,
            equipment: crate::equipment::EquipmentSlots::default(),
            skill_cooldowns: crate::active_skill::SkillCooldowns::default(),
            pending_warcry: false,
            pending_bounty: false,
            pending_precision: false,
            pending_haggle: false,
            auto_skills: std::collections::HashSet::new(),
            stats: crate::stat_points::StatPoints::default(),
            skill_masteries: crate::skill_mastery::SkillMasteries::default(),
            pet: None,
            pet_x: crate::state::WORLD_WIDTH / 2.0,
            pet_y: crate::state::WORLD_HEIGHT / 2.0,
            pet_playing: false,
            pet_fetch: None,
            pet_fetching: false,
            fish_cooldown: 0.0,
            fish_attempt_count: 0,
            fishing: None,
            mine_cooldown: 0.0,
            mine_attempt_count: 0,
            mining: None,
            traced_constellations: 0,
            toast_cooldown: 0.0,
            toast_count: 0,
            high_five_offer: 0,
            recent_emote: None,
            cheer_offer: 0,
            cheer_cooldowns: std::collections::HashMap::new(),
            trade_cargo: None,
            trade_cooldowns: crate::trade_route::TradeCooldowns::new(),
            workshop_active: None,
            workshop_cooldown: 0.0,
            bounty_active: None,
            bounty_cooldown: 0.0,
            expedition_active: None,
            expedition_cooldown: 0.0,
            procurement_active: None,
            procurement_cooldown: 0.0,
            farm_fair_active: None,
            farm_fair_cooldown: 0.0,
            warehouse: crate::warehouse::Warehouse::default(),
            decay_timers: crate::perishable::PerishableDecayState::new(),
            indoor_plot_id: None,
            indoor_x: 0.0,
            indoor_y: 0.0,
            inventory_extra_kinds: 0,
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
            inventory: crate::inventory::Inventory::new(),
            vitals: crate::vitals::Vitals::new(),
            wallet: crate::economy::PlotWallet::new(),
            attack_cooldown: 0.0,
            exp: 0,
            codex: 0,
            atlas: 0,
            skylog: 0,
            cheers: 0,
            planet: crate::state::PLANET_HOME.to_string(),
            masteries: crate::class::Masteries::new(),
            guild_tag: None,
            party_id: None,
            hair_style: 0,
            skin_tone: 0,
            goggle_color: 0,
            costume: 0,
            achievements: crate::achievement::AchievementSet::new(),
            kill_count: 0,
            refine_attempt_count: 0,
            equipment: crate::equipment::EquipmentSlots::default(),
            skill_cooldowns: crate::active_skill::SkillCooldowns::default(),
            pending_warcry: false,
            pending_bounty: false,
            pending_precision: false,
            pending_haggle: false,
            auto_skills: std::collections::HashSet::new(),
            stats: crate::stat_points::StatPoints::default(),
            skill_masteries: crate::skill_mastery::SkillMasteries::default(),
            pet: None,
            pet_x: crate::state::WORLD_WIDTH / 2.0,
            pet_y: crate::state::WORLD_HEIGHT / 2.0,
            pet_playing: false,
            pet_fetch: None,
            pet_fetching: false,
            fish_cooldown: 0.0,
            fish_attempt_count: 0,
            fishing: None,
            mine_cooldown: 0.0,
            mine_attempt_count: 0,
            mining: None,
            traced_constellations: 0,
            toast_cooldown: 0.0,
            toast_count: 0,
            high_five_offer: 0,
            recent_emote: None,
            cheer_offer: 0,
            cheer_cooldowns: std::collections::HashMap::new(),
            trade_cargo: None,
            trade_cooldowns: crate::trade_route::TradeCooldowns::new(),
            workshop_active: None,
            workshop_cooldown: 0.0,
            bounty_active: None,
            bounty_cooldown: 0.0,
            expedition_active: None,
            expedition_cooldown: 0.0,
            procurement_active: None,
            procurement_cooldown: 0.0,
            farm_fair_active: None,
            farm_fair_cooldown: 0.0,
            warehouse: crate::warehouse::Warehouse::default(),
            decay_timers: crate::perishable::PerishableDecayState::new(),
            indoor_plot_id: None,
            indoor_x: 0.0,
            indoor_y: 0.0,
            inventory_extra_kinds: 0,
        }
    };
    let id = player.id;

    // 登記這條連線。同帳號（同 id）開多個分頁／裝置時，只有第一條連線建立玩家、從記憶
    // 位置進場；之後的連線共用既有權威狀態（不用舊存檔覆蓋當前位置，避免畫面瞬移）。
    // 鎖序固定「先 players 再 conns」，與 cleanup 一致，避免死鎖。
    //
    // recall 也在這裡(鎖內)做，跟 cleanup 的 remember 用同一把 players 寫鎖排序，
    // 消除 refresh 時「新連線 recall 早於舊連線 remember」的 race window。
    // 讀取既有地塊序號(已登入才有)。不再進場就自動分配,對齊 ③ Slice D「自己攢乙太買地」。
    let plot_index = authed_uid.and_then(|uid| app.plots.index_of(uid));
    {
        let mut players = app.players.write().unwrap();
        if app.connections.acquire(id) {
            // 第一條連線:讀記憶位置(已登入玩家才記),把占位 Player 的位置/乙太覆寫掉。
            let mut p = player.clone();
            if let Some(uid) = authed_uid {
                // 背包與位置各自獨立記憶:有存檔就帶回採集/打怪/收成囤的素材,沒有就維持空背包。
                if let Some(inv) = app.inventories.recall(uid) {
                    p.inventory = inv;
                }
                // 裝備槽（ROADMAP 36）：有存檔就帶回；否則依背包自動裝最強（向後相容遷移）。
                // 首次遷移時必須同步從背包扣除，否則 unequip 後會複製道具。
                p.equipment = app.inventories.recall_equipment(uid)
                    .unwrap_or_else(|| {
                        let slots = crate::equipment::auto_equip_best(&p.inventory);
                        if let Some(w) = slots.weapon   { p.inventory.take(w, 1); }
                        if let Some(a) = slots.armor    { p.inventory.take(a, 1); }
                        if let Some(ac) = slots.accessory { p.inventory.take(ac, 1); }
                        slots
                    });
                let saved = app.positions.recall(uid);
                match saved {
                    // 有歷史位置 → 回到離線前的地方。
                    Some(s) => {
                        let (x, y) = crate::positions::spawn_at(Some((s.x, s.y)));
                        p.x = x;
                        p.y = y;
                        p.ether = s.ether;
                        // 農地擴張格數：超上限時視為無效，重設為 0（載入防線）。
                        let mut w = crate::economy::PlotWallet::from_expansions(s.wallet_expansions);
                        if !w.is_loadable() {
                            w = crate::economy::PlotWallet::new();
                        }
                        p.wallet = w;
                        p.exp = s.exp;
                        // 五條熟練度從 DB 還原（ROADMAP 38）。
                        p.masteries = s.masteries;
                        // 屬性加點從 DB 還原（ROADMAP 152）。
                        p.stats = s.stats;
                        // 技能使用型熟練度從 DB 還原（ROADMAP 153）。
                        p.skill_masteries = s.skill_masteries;
                        // 生態圖鑑（333）／探索圖鑑（336）／天象圖鑑（337）的蒐集進度從 DB 還原。
                        // 三者本就持久化、且重連後仍要與面板／稱號一致，不還原會讓蒐集進度一重連歸零。
                        p.codex = s.codex;
                        p.atlas = s.atlas;
                        p.skylog = s.skylog;
                        // 喝采人氣（341）從 DB 還原——人氣是長駐的社交身份，不還原會讓名牌徽記一重連歸零。
                        p.cheers = s.cheers;
                        // 根據存檔等級 + 戰士熟練度 + HP 加點校正最大血量（Vitals 不持久化，重連給滿血）。
                        let base_hp = crate::vitals::level_max_hp(p.level())
                            + crate::class::hp_bonus(&p.masteries)
                            + p.stats.hp * crate::stat_points::HP_PER_POINT;
                        p.vitals.set_max_hp_full(base_hp);
                    }
                    // 第一次進場、沒有歷史位置 → 落在自己那塊地的中心。
                    None => {
                        if let Some(idx) = plot_index {
                            let (ox, oy) = crate::plots::plot_origin(idx);
                            p.x = ox + crate::plots::PLOT_WIDTH / 2.0;
                            p.y = oy + crate::plots::PLOT_HEIGHT / 2.0;
                        }
                        p.ether = 0;
                    }
                }
                // 乙太寶箱背包加成（ROADMAP 155）：重連同一 session 時從 home_furnishings 重新同步，
                // 避免 inventory_extra_kinds 停在初始值 0 而家具面板仍顯示寶箱的不一致狀態。
                if app.home_furnishings.read().unwrap()
                    .get(&uid).map(|h| h.has_chest()).unwrap_or(false)
                {
                    p.inventory_extra_kinds = crate::home_furniture::CHEST_CAPACITY_BONUS as u32;
                }
            }
            players.insert(id, p);
        }
        // 不是第一條連線:既有玩家記錄保留(同帳號其他分頁仍在用),不動。
    }

    // 已登入玩家擁有自己的一塊地（Phase 0-G-O1 per-player）：依序號與已購擴張格數建立那塊地。
    // `entry` 冪等,多分頁/重連重複呼叫不會覆蓋既有作物。訪客(隨機 id、不持久)刻意不分地。
    if let (Some(uid), Some(index)) = (authed_uid, plot_index) {
        let expansions = app.players.read().unwrap()
            .get(&uid)
            .map(|p| p.wallet.expansions())
            .unwrap_or(0);
        app.fields
            .write()
            .unwrap()
            .entry(uid)
            .or_insert_with(|| Field::for_plot_expanded(index, expansions));
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

    // 登入玩家：進場後立刻送一次好友清單（讓前端開好友面板即可看到資料）。
    if let Some(uid) = authed_uid {
        let friend_msg = build_friend_list_msg(&app, uid);
        if let Ok(text) = serde_json::to_string(&friend_msg) {
            // 連線剛建立、forward task 尚未啟動，直接透過 sender 送。
            let _ = sender.send(Message::Text(text)).await;
        }
    }

    // 轉發任務：把兩條廣播推給這個客戶端。
    // 快照（高頻、會淹）走 tx；聊天（低頻、一次性、漏了就永久看不到）走獨立的 tx_chat，
    // 這樣追快照造成的 Lagged 不會把同段時間捲過的聊天一起丟掉。兩條各自用 forward_action
    // 判斷 Lagged（跳過、不踢人）/ Closed（結束）。
    // ③ 無限世界（切片 C）：從 tx 收到的是 Arc<ServerMsg>，依玩家當下位置做 AOI 剔除後才序列化。
    // tx_direct：單播通道——讓讀取迴圈把僅給本玩家看的訊息（如 TravelResult）推給 forward task。
    let (tx_direct, mut rx_direct) = tokio::sync::mpsc::channel::<String>(16);
    // ROADMAP 95 密語路由：登記本連線的直達通道，讓其他玩家的密語可精準單播。
    app.whisper_senders.write().unwrap().insert(id, tx_direct.clone());
    let mut rx = app.tx.subscribe();
    let mut rx_chat = app.tx_chat.subscribe();
    let app_for_forward = app.clone();
    let forward = tokio::spawn(async move {
        loop {
            tokio::select! {
                r = rx.recv() => {
                    match r {
                        Ok(msg) => {
                            // 依玩家權威位置做 AOI 剔除。
                            let filtered = match &*msg {
                                ServerMsg::Snapshot { tick, players, fields, nodes, enemies, daynight, listings, npcs, terrain, world_event, horde_event, quests, land_plots, ranch_plots, farm_crop_plots, star_crystals, village_buff_remaining_secs, village_treasury, weather, sprinklers, gathering_secs, active_help_requests, resident_moods, town_prosperity_level, town_project, star_forecast_secs, star_forecast_bonus, meteor_shower_secs, dust_nodes, wandering_merchant_secs, wandering_catalog, merchant_quests, current_season, season_remaining_secs, wildlife, carion_orbs, colonies, species_attitudes, seasonal_nodes, home_furniture: _, home_style: _, civic_vote, civic_effect_secs, civic_effect_kind, invasion, night_spring_nodes, monster_species_attitudes, monster_colony_views, eco_pressure_value, alpha_monsters, eco_bounty, ancient_alpha, expedition_target, eco_festival } => {
                                    let (px, py) = {
                                        let ps = app_for_forward.players.read().unwrap();
                                        ps.get(&id).map(|p| (p.x, p.y)).unwrap_or((0.0, 0.0))
                                    };
                                    const AOI_RADIUS_SQ: f32 = 2000.0 * 2000.0;

                                    let filter_pos = |x: f32, y: f32| {
                                        let dx = x - px;
                                        let dy = y - py;
                                        dx * dx + dy * dy <= AOI_RADIUS_SQ
                                    };

                                    ServerMsg::Snapshot {
                                        tick: *tick,
                                        players: players.iter().filter(|p| p.id == id || filter_pos(p.x, p.y)).cloned().collect(),
                                        fields: fields.iter().filter(|f| f.owner == id || filter_pos(f.origin_x + (f.cols as f32 * f.tile_size)/2.0, f.origin_y + (f.rows as f32 * f.tile_size)/2.0)).cloned().collect(),
                                        nodes: nodes.iter().filter(|n| filter_pos(n.x, n.y)).cloned().collect(),
                                        enemies: enemies.iter().filter(|e| filter_pos(e.x, e.y)).cloned().collect(),
                                        daynight: daynight.clone(),
                                        listings: listings.iter().filter(|l| filter_pos(l.x, l.y)).cloned().collect(),
                                        // NPC 全部送出（靜態且位置固定在新手村，一定在 AOI 內）
                                        npcs: npcs.clone(),
                                        // C-2：依格中心世界座標做 AOI 剔除，不廣播超出視野的挖掘差異。
                                        terrain: terrain.iter().filter(|d| {
                                            let (wx, wy) = crate::tiles::cell_center(d.cx, d.cy, d.tx, d.ty);
                                            filter_pos(wx, wy)
                                        }).cloned().collect(),
                                        // 世界事件全服廣播（裂縫座標不做 AOI 剔除，讓玩家知道在哪裡）。
                                        world_event: world_event.clone(),
                                        // 獸潮攻城事件全服廣播（攻城點座標讓所有玩家知道）。
                                        horde_event: horde_event.clone(),
                                        // 社群任務全服廣播（所有玩家看同一套任務進度）。
                                        quests: quests.clone(),
                                        // 城外地塊全部送出（20 塊量小；地塊都在主城附近）。
                                        land_plots: land_plots.clone(),
                                        // 牧場狀態全部送出（稀疏，通常很少地塊有雞）。
                                        ranch_plots: ranch_plots.clone(),
                                        // 農地作物狀態全部送出（稀疏，通常很少地塊有種植）。
                                        farm_crop_plots: farm_crop_plots.clone(),
                                        // 夜採星晶礦脈：夜間節點依 AOI 剔除，白天空陣列直接傳。
                                        star_crystals: star_crystals.iter().filter(|c| filter_pos(c.x, c.y)).cloned().collect(),
                                        // 村落節慶加成全服廣播（直接轉送原值）。
                                        village_buff_remaining_secs: *village_buff_remaining_secs,
                                        // 村庫餘額全服廣播（里長面板需要）。
                                        village_treasury: *village_treasury,
                                        // 天氣狀態全服廣播（ROADMAP 93）。
                                        weather: weather.clone(),
                                        // 灑水器：依農地位置做 AOI 剔除（ROADMAP 112）。
                                        sprinklers: sprinklers.iter().filter(|s| filter_pos(s.wx, s.wy)).cloned().collect(),
                                        // 廣場聚會剩餘秒數（ROADMAP 124）：全服廣播。
                                        gathering_secs: *gathering_secs,
                                        // 互助請求居民清單（ROADMAP 125）：全服廣播。
                                        active_help_requests: active_help_requests.clone(),
                                        // 居民心情（ROADMAP 126）：全服廣播（量小，5-12 居民）。
                                        resident_moods: resident_moods.clone(),
                                        // 城鎮繁榮等級：全服廣播。
                                        town_prosperity_level: *town_prosperity_level,
                                        // 城鎮大工程：全服廣播。
                                        town_project: town_project.clone(),
                                        // 天文台星象預報（ROADMAP 132）：全服廣播。
                                        star_forecast_secs: *star_forecast_secs,
                                        star_forecast_bonus: star_forecast_bonus.clone(),
                                        // 流星雨（ROADMAP 133）：全服廣播。
                                        meteor_shower_secs: *meteor_shower_secs,
                                        dust_nodes: dust_nodes.iter().filter(|n| filter_pos(n.wx, n.wy)).cloned().collect(),
                                        // 旅行商人（ROADMAP 135）：全服廣播。
                                        wandering_merchant_secs: *wandering_merchant_secs,
                                        wandering_catalog: wandering_catalog.clone(),
                                        // 旅行商人限時委託（ROADMAP 136）：全服廣播。
                                        merchant_quests: merchant_quests.clone(),
                                        // 季節循環（ROADMAP 137）：全服廣播。
                                        current_season: current_season.clone(),
                                        season_remaining_secs: *season_remaining_secs,
                                        // 野生動物（ROADMAP 140）：依 AOI 剔除。
                                        wildlife: wildlife.iter().filter(|w| filter_pos(w.x, w.y)).cloned().collect(),
                                        // 乙太微粒（ROADMAP 142）：依 AOI 剔除。
                                        carion_orbs: carion_orbs.iter().filter(|o| filter_pos(o.x, o.y)).cloned().collect(),
                                        // 物種聚落（ROADMAP 143）：靜態資料，全部送出。
                                        colonies: colonies.clone(),
                                        // 物種關係（ROADMAP 144）：全服廣播（量少，5 物種）。
                                        species_attitudes: species_attitudes.clone(),
                                        expedition_target: *expedition_target,
                                        // 季節性野外採集節點（ROADMAP 154）：全服廣播（量少，最多 3 顆）。
                                        seasonal_nodes: seasonal_nodes.clone(),
                                        // 住家家具（ROADMAP 155）：只在玩家自己室內時送出本人家具。
                                        home_furniture: {
                                            let ps = app_for_forward.players.read().unwrap();
                                            if let Some(p) = ps.get(&id) {
                                                if p.indoor_plot_id.is_some() {
                                                    app_for_forward.home_furnishings.read().unwrap()
                                                        .get(&id)
                                                        .map(|f| f.views())
                                                        .unwrap_or_default()
                                                } else {
                                                    vec![]
                                                }
                                            } else {
                                                vec![]
                                            }
                                        },
                                        // 居家風格（ROADMAP 325）：只在玩家自己室內時送出本人風格代碼。
                                        home_style: {
                                            let ps = app_for_forward.players.read().unwrap();
                                            match ps.get(&id) {
                                                Some(p) if p.indoor_plot_id.is_some() => Some(
                                                    app_for_forward.home_furnishings.read().unwrap()
                                                        .get(&id)
                                                        .map(|f| f.style())
                                                        .unwrap_or_default()
                                                        .code()
                                                        .to_string()
                                                ),
                                                _ => None,
                                            }
                                        },
                                        // 公民投票（ROADMAP 156）：全服廣播（投票視圖 + 效果狀態）。
                                        civic_vote: civic_vote.clone(),
                                        civic_effect_secs: *civic_effect_secs,
                                        civic_effect_kind: civic_effect_kind.clone(),
                                        // 城鎮入侵警報（ROADMAP 158）：全服廣播入侵狀態。
                                        invasion: invasion.clone(),
                                        // 夜間乙太泉（ROADMAP 162）：全服廣播（量少，最多 5 顆）。
                                        night_spring_nodes: night_spring_nodes.clone(),
                                        // 怪物物種態度（ROADMAP 163）：全服廣播。
                                        monster_species_attitudes: monster_species_attitudes.clone(),
                                        // 怪物巢穴（ROADMAP 164）：全服廣播（量少，5 個巢穴）。
                                        monster_colony_views: monster_colony_views.clone(),
                                        // 生態壓力值（ROADMAP 167）：全服廣播。
                                        eco_pressure_value: *eco_pressure_value,
                                        // 巢穴 Alpha（ROADMAP 168）：全服廣播。
                                        alpha_monsters: alpha_monsters.clone(),
                                        // 生態清剿委託（ROADMAP 172）：全服廣播。
                                        eco_bounty: eco_bounty.clone(),
                                        // 傳說古 Alpha（ROADMAP 173）：全服廣播（唯一世界頭目）。
                                        ancient_alpha: ancient_alpha.clone(),
                                        // 生態豐收節（ROADMAP 178）：全服廣播（全城同步慶典）。
                                        eco_festival: eco_festival.clone(),
                                    }
                                }
                                other => other.clone(),
                            };
                            
                            match serde_json::to_string(&filtered) {
                                Ok(json) => {
                                    if sender.send(Message::Text(json)).await.is_err() {
                                        break;
                                    }
                                }
                                Err(_) => continue,
                            }
                        }
                        Err(e) => match forward_action(&e) {
                            ForwardAction::Skip => continue,
                            ForwardAction::Stop => break,
                        },
                    }
                }
                r = rx_chat.recv() => match r {
                    Ok(msg) => {
                        if sender.send(Message::Text(msg)).await.is_err() {
                            break;
                        }
                    }
                    Err(e) => match forward_action(&e) {
                        ForwardAction::Skip => continue,
                        ForwardAction::Stop => break,
                    },
                },
                // 單播直達訊息（如 TravelResult）：由讀取迴圈產生後透過 tx_direct 推來。
                Some(json) = rx_direct.recv() => {
                    if sender.send(Message::Text(json)).await.is_err() {
                        break;
                    }
                },
            }
        }
    });

    // H2 安全：每連線訊息限流（令牌桶）——防單一惡意客戶端每秒灌數千訊息 DoS（CPU + 廣播放大）。
    let mut rl_win = std::time::Instant::now();
    let mut rl_n: u32 = 0;
    let mut rl_chat_win = std::time::Instant::now();
    let mut rl_chat_n: u32 = 0;
    // 表情動作限流（ROADMAP 338）：每則 emote 走 broadcast 放大給全服，比照 chat 從嚴。
    let mut rl_emote_win = std::time::Instant::now();
    let mut rl_emote_n: u32 = 0;
    // 擊掌意願限流（ROADMAP 339）：比照 emote 從嚴（每秒至多 3 次，超量靜默丟棄）。
    let mut rl_hifive_win = std::time::Instant::now();
    let mut rl_hifive_n: u32 = 0;
    // 喝采意願限流（ROADMAP 341）：比照 emote／擊掌從嚴（每秒至多 3 次，超量靜默丟棄）——
    // 連線層擋封包洪流，每對象 60s 冷卻另由 game.rs 把關，雙重防洗榜。
    let mut rl_cheer_win = std::time::Instant::now();
    let mut rl_cheer_n: u32 = 0;
    // 逗玩接物限流（ROADMAP 345）：比照 emote／擊掌／喝采從嚴（每秒至多 3 次，超量靜默丟棄）——
    // 擋封包洪流；一趟接物未結束前不重複開新接物另由 game.rs 把關（`pet_fetch.is_none()` 才丟得出）。
    let mut rl_petfetch_win = std::time::Instant::now();
    let mut rl_petfetch_n: u32 = 0;
    // 讀取迴圈：更新此玩家的輸入意圖、處理聊天。
    while let Some(Ok(msg)) = receiver.next().await {
        // H2：訊息總量限流（每秒上限）。合法操作（移動/動作）遠低於此；超量靜默丟棄。
        if rl_win.elapsed().as_secs() >= 1 {
            rl_win = std::time::Instant::now();
            rl_n = 0;
        }
        rl_n += 1;
        if rl_n > 60 {
            continue;
        }
        match msg {
            Message::Text(text) => match serde_json::from_str::<ClientMsg>(&text) {
                Ok(ClientMsg::Input {
                    up,
                    down,
                    left,
                    right,
                    run,
                }) => {
                    if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                        p.input = Input {
                            up,
                            down,
                            left,
                            right,
                            run,
                        };
                    }
                }
                Ok(ClientMsg::Chat { text }) => {
                    // H2：聊天額外限流（每則 chat 走 broadcast 放大給全服，更嚴）。超量靜默丟棄。
                    if rl_chat_win.elapsed().as_secs() >= 1 {
                        rl_chat_win = std::time::Instant::now();
                        rl_chat_n = 0;
                    }
                    rl_chat_n += 1;
                    if rl_chat_n > 3 {
                        continue;
                    }
                    // 清過控制字元 / 截長後若還有內容才廣播（集中在 sanitize_chat，可測）。
                    if let Some(text) = sanitize_chat(&text) {
                        // 讀**線上即時**名(不是進場時擷取的舊名):改名後不重連、聊天 from 也立刻是新名。
                        let (from, my_guild_tag) = {
                            let ps = app.players.read().unwrap();
                            let (name, tag) = ps.get(&id)
                                .map(|p| (p.name.clone(), p.guild_tag.clone()))
                                .unwrap_or_else(|| (player.name.clone(), None));
                            (name, tag)
                        };
                        // `/g ` 前綴 → 公會頻道聊天（只廣播給同公會成員，via tx_chat 帶 guild_tag）。
                        if let Some(guild_text) = text.strip_prefix("/g ").map(str::to_string) {
                            if let Some(ref tag) = my_guild_tag {
                                let msg = ServerMsg::GuildChat {
                                    guild_tag: tag.clone(),
                                    from: from.clone(),
                                    text: guild_text,
                                };
                                if let Ok(json) = serde_json::to_string(&msg) {
                                    let _ = app.tx_chat.send(json);
                                }
                            } else {
                                // 不在公會，提示加入。
                                let err = ServerMsg::Chat { from: "系統".into(), text: "你目前不在任何公會（輸入 /g 文字 發送公會聊天）".into() };
                                if let Ok(json) = serde_json::to_string(&err) {
                                    let _ = tx_direct.try_send(json);
                                }
                            }
                        } else if let Some(rest) = text.strip_prefix("/w ") {
                            // `/w 名字 訊息` → 密語（ROADMAP 95）：只送寄件人+收件人，不廣播全服。
                            let mut parts = rest.splitn(2, ' ');
                            let to_name = parts.next().unwrap_or("").trim().to_string();
                            let whisper_text = parts.next().unwrap_or("").trim().to_string();
                            if to_name.is_empty() || whisper_text.is_empty() {
                                let err = ServerMsg::Chat {
                                    from: "系統".into(),
                                    text: "用法：/w 玩家名字 訊息".into(),
                                };
                                if let Ok(json) = serde_json::to_string(&err) {
                                    let _ = tx_direct.try_send(json);
                                }
                            } else if to_name == from {
                                let err = ServerMsg::Chat {
                                    from: "系統".into(),
                                    text: "不能密語自己哦".into(),
                                };
                                if let Ok(json) = serde_json::to_string(&err) {
                                    let _ = tx_direct.try_send(json);
                                }
                            } else {
                                // 依顯示名找目標玩家 id（線上才找得到）。
                                let target_id = app.players.read().unwrap()
                                    .iter()
                                    .find(|(_, p)| p.name == to_name)
                                    .map(|(uid, _)| *uid);
                                if let Some(target_id) = target_id {
                                    let msg = ServerMsg::Whisper {
                                        from: from.clone(),
                                        to: to_name.clone(),
                                        text: whisper_text,
                                    };
                                    if let Ok(json) = serde_json::to_string(&msg) {
                                        // 送給收件人（在線才有 sender）。
                                        let target_tx = app.whisper_senders.read().unwrap()
                                            .get(&target_id).cloned();
                                        if let Some(target_tx) = target_tx {
                                            let _ = target_tx.try_send(json.clone());
                                        }
                                        // 回顯給寄件人（讓他確認訊息送出）。
                                        let _ = tx_direct.try_send(json);
                                    }
                                } else {
                                    let err = ServerMsg::Chat {
                                        from: "系統".into(),
                                        text: format!("「{to_name}」目前不在線"),
                                    };
                                    if let Ok(json) = serde_json::to_string(&err) {
                                        let _ = tx_direct.try_send(json);
                                    }
                                }
                            }
                        } else if let Some(invite_name) = text.strip_prefix("/invite ") {
                            // `/invite 玩家名` → 邀請加入隊伍（ROADMAP 97）的聊天捷徑。
                            let invite_name = invite_name.trim().to_string();
                            let Some(uid) = authed_uid else { continue; };
                            let target_id = {
                                let ps = app.players.read().unwrap();
                                ps.values().find(|p| p.name == invite_name).map(|p| p.id)
                            };
                            let Some(target_id) = target_id else {
                                let err = ServerMsg::Chat { from: "系統".into(), text: format!("找不到在線玩家「{invite_name}」") };
                                if let Ok(j) = serde_json::to_string(&err) { let _ = tx_direct.try_send(j); }
                                continue;
                            };
                            if target_id == uid {
                                let err = ServerMsg::Chat { from: "系統".into(), text: "不能邀請自己哦".into() };
                                if let Ok(j) = serde_json::to_string(&err) { let _ = tx_direct.try_send(j); }
                                continue;
                            }
                            let my_party_id = app.parties.party_of(uid).unwrap_or_else(|| {
                                let pid = app.parties.create(uid);
                                if let Some(p) = app.players.write().unwrap().get_mut(&uid) { p.party_id = Some(pid); }
                                pid
                            });
                            match app.parties.invite(my_party_id, target_id) {
                                None => {
                                    let err = ServerMsg::Chat { from: "系統".into(), text: format!("「{invite_name}」已在隊伍中，無法邀請") };
                                    if let Ok(j) = serde_json::to_string(&err) { let _ = tx_direct.try_send(j); }
                                }
                                Some(_) => {
                                    let invite_msg = ServerMsg::PartyInvite { from_name: from.clone() };
                                    if let Ok(j) = serde_json::to_string(&invite_msg) {
                                        let senders = app.whisper_senders.read().unwrap();
                                        if let Some(tx) = senders.get(&target_id) { let _ = tx.try_send(j); }
                                    }
                                    let ok = ServerMsg::Chat { from: "系統".into(), text: format!("已傳送隊伍邀請給「{invite_name}」") };
                                    if let Ok(j) = serde_json::to_string(&ok) { let _ = tx_direct.try_send(j); }
                                }
                            }
                        } else if let Some(party_text) = text.strip_prefix("/p ") {
                            // `/p 訊息` → 隊伍頻道聊天（ROADMAP 97）：只送給隊伍成員。
                            let party_text = party_text.trim().to_string();
                            if party_text.is_empty() {
                                let err = ServerMsg::Chat { from: "系統".into(), text: "用法：/p 訊息（隊伍聊天）".into() };
                                if let Ok(j) = serde_json::to_string(&err) { let _ = tx_direct.try_send(j); }
                            } else if let Some(pid) = authed_uid.and_then(|u| app.parties.party_of(u)) {
                                let msg = ServerMsg::PartyChat { from: from.clone(), text: party_text };
                                let members = app.parties.members(pid);
                                let senders = app.whisper_senders.read().unwrap();
                                for m in &members {
                                    if let Some(tx) = senders.get(m) {
                                        if let Ok(j) = serde_json::to_string(&msg) { let _ = tx.try_send(j); }
                                    }
                                }
                            } else {
                                let err = ServerMsg::Chat { from: "系統".into(), text: "你目前不在任何隊伍（輸入 /p 訊息 發送隊伍聊天）".into() };
                                if let Ok(j) = serde_json::to_string(&err) { let _ = tx_direct.try_send(j); }
                            }
                        } else {
                            let chat = ServerMsg::Chat { from, text };
                            if let Ok(json) = serde_json::to_string(&chat) {
                                // 走聊天專用頻道，不與高頻快照爭緩衝、不被 Lagged 一起丟。
                                let _ = app.tx_chat.send(json);
                            }
                        }
                    }
                }
                Ok(ClientMsg::Emote { kind }) => {
                    // 表情動作（ROADMAP 338）：玩家↔玩家的即時情緒表態。
                    // 限流（比照 chat：每秒至多 3 次，超量靜默丟棄，不懲罰多按）。
                    if rl_emote_win.elapsed().as_secs() >= 1 {
                        rl_emote_win = std::time::Instant::now();
                        rl_emote_n = 0;
                    }
                    rl_emote_n += 1;
                    if rl_emote_n > 3 {
                        continue;
                    }
                    // 查白名單：只接受固定表情，未知 kind 靜默忽略（玩家送不出任意內容）。
                    if let Some(idx) = crate::player_emote::index_of(&kind) {
                        let glyph = crate::player_emote::glyph_at(idx).unwrap_or("❓");
                        // 讀玩家自己的**權威座標 + 即時名**（改名後不重連也對），同時點亮
                        // 「最近表情」倒數——供 game.rs 每幀偵測表情共鳴（ROADMAP 340）。
                        let loc = {
                            let mut ps = app.players.write().unwrap();
                            ps.get_mut(&id).map(|p| {
                                p.recent_emote =
                                    Some((idx, crate::emote_resonance::RESONANCE_WINDOW));
                                (p.name.clone(), p.x, p.y)
                            })
                        };
                        if let Some((from_name, wx, wy)) = loc {
                            let _ = app.tx.send(std::sync::Arc::new(ServerMsg::PlayerEmote {
                                from_id: id,
                                from_name,
                                glyph: glyph.to_string(),
                                wx,
                                wy,
                                display_secs: crate::player_emote::EMOTE_DISPLAY_SECS,
                            }));
                        }
                    }
                }
                Ok(ClientMsg::HighFive) => {
                    // 擊掌意願（ROADMAP 339）：玩家伸手想擊掌。這裡只點亮一個短暫的意願，
                    // 真正的配對與特效廣播交給 game.rs 每幀做（同區、靠得夠近、也正在比的兩人配成對）。
                    // 限流（比照 emote：每秒至多 3 次，超量靜默丟棄）。
                    if rl_hifive_win.elapsed().as_secs() >= 1 {
                        rl_hifive_win = std::time::Instant::now();
                        rl_hifive_n = 0;
                    }
                    rl_hifive_n += 1;
                    if rl_hifive_n > 3 {
                        continue;
                    }
                    // 在該玩家身上點亮擊掌意願倒數（訪客沒登入也行——配對只看在場玩家座標）。
                    if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                        p.high_five_offer = crate::high_five::OFFER_TICKS;
                    }
                }
                Ok(ClientMsg::Cheer) => {
                    // 喝采意願（ROADMAP 341）：玩家替附近玩家鼓掌。這裡只點亮一個短暫的意願，
                    // 真正的挑對象、加人氣、迸特效交給 game.rs 每幀做（同區、最近、過了冷卻）。
                    // 限流（比照擊掌：每秒至多 3 次，超量靜默丟棄）。
                    if rl_cheer_win.elapsed().as_secs() >= 1 {
                        rl_cheer_win = std::time::Instant::now();
                        rl_cheer_n = 0;
                    }
                    rl_cheer_n += 1;
                    if rl_cheer_n > 3 {
                        continue;
                    }
                    // 點亮喝采意願倒數（訪客沒登入也能鼓掌——挑對象只看在場玩家座標；但人氣只記在
                    // 對象身上、且對象需是已登入玩家才持久化得了，訪客互喝采重啟即逝、無妨）。
                    if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                        p.cheer_offer = crate::player_cheer::OFFER_TICKS;
                    }
                }
                Ok(ClientMsg::PlayWithPet { dx, dy }) => {
                    // 逗玩接物（ROADMAP 345）：玩家朝面前丟出玩具，寵物衝去叼回。這裡只在寵物身上
                    // 開一趟接物（玩具落點＋追逐階段），真正的「衝去叼→叼回」推進交給 game.rs 每幀做。
                    // 限流（比照喝采：每秒至多 3 次，超量靜默丟棄，不懲罰多按）。
                    if rl_petfetch_win.elapsed().as_secs() >= 1 {
                        rl_petfetch_win = std::time::Instant::now();
                        rl_petfetch_n = 0;
                    }
                    rl_petfetch_n += 1;
                    if rl_petfetch_n > 3 {
                        continue;
                    }
                    // 有寵物、在室外、且目前沒有正在進行的接物，才丟得出玩具（用玩家自己的權威座標
                    // 算落點，防隔空丟）。一趟接物未叼回前不重複開新的（天然防洗螢幕）。
                    if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                        if p.pet.is_some() && p.indoor_plot_id.is_none() && p.pet_fetch.is_none() {
                            let (tx, ty) = crate::pet_fetch::throw_spot(p.x, p.y, dx, dy);
                            p.pet_fetch = Some(crate::pet_fetch::PetFetch {
                                toy_x: tx,
                                toy_y: ty,
                                phase: crate::pet_fetch::FetchPhase::Chasing,
                            });
                            p.pet_fetching = true;
                        }
                    }
                }
                Ok(ClientMsg::Farm { x, y }) => {
                    // 被打趴時不能耕種——倒地定身，等復原傳回新手村再繼續。
                    if app.players.read().unwrap().get(&id).map(|p| p.vitals.is_downed()).unwrap_or(false) {
                        continue;
                    }
                    // 農地互動：先嘗試自己的私有農地；座標不在私有地內則嘗試公共農地。
                    // 私有地：只有擁有者能互動（`id` 即 uid，訪客沒有地塊 → 取不到 → 不能耕種）。
                    // 公共地：任何已登入玩家均可互動（軟劫掠：誰先採誰得）。
                    // 每把鎖各自取各自放，同一時間至多持一把，沿用「不互鎖」的鎖序。
                    let player_pos = app.players.read().unwrap().get(&id).map(|p| (p.x, p.y));

                    // 嘗試私有農地（若座標在其中回 Some(outcome)，否則 None）。
                    let own_outcome: Option<FarmOutcome> = {
                        let mut fields = app.fields.write().unwrap();
                        match fields.get_mut(&id) {
                            Some(field) => match field.cell_at(x, y) {
                                Some((col, row))
                                    if player_pos
                                        .map(|(px, py)| field.within_reach(px, py))
                                        .unwrap_or(false) =>
                                {
                                    Some(field.interact(col, row))
                                }
                                // 座標不在私有地（或太遠）→ 留給公共農地試試。
                                _ => None,
                            },
                            None => None,
                        }
                    };

                    // 若私有地沒命中，且玩家已登入，嘗試公共農地。
                    let outcome = if let Some(o) = own_outcome {
                        o
                    } else if authed_uid.is_some() {
                        let mut pf = app.pub_field.write().unwrap();
                        match pf.cell_at(x, y) {
                            Some((col, row))
                                if player_pos
                                    .map(|(px, py)| pf.within_reach(px, py))
                                    .unwrap_or(false) =>
                            {
                                pf.interact(col, row)
                            }
                            _ => FarmOutcome::Nothing,
                        }
                    } else {
                        FarmOutcome::Nothing
                    };

                    if let FarmOutcome::Harvested(ether) = outcome {
                        if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                            let bonus = crate::class::harvest_ether_bonus(&p.masteries);
                            p.ether = p.ether.saturating_add(ether).saturating_add(bonus);
                            p.masteries.gain_farmer(1); // 農夫熟練度（ROADMAP 38）
                            tracing::info!(player = %p.name, ether = p.ether, bonus, "農地收成乙太");
                        }
                    }
                }
                Ok(ClientMsg::Gather) => {
                    // 被打趴時不能採集——倒地定身，等復原傳回新手村再繼續。
                    if app.players.read().unwrap().get(&id).map(|p| p.vitals.is_downed()).unwrap_or(false) {
                        continue;
                    }
                    // 採集：用玩家**自己的權威位置**判定 GATHER_REACH 內最近的可採節點(防隔空採集,
                    // 客戶端送的座標只是觸發點、不採信)。採到的種類 `.into()` 轉成背包物品加進背包。
                    // 每把鎖各自取各自放(先讀玩家位置、再寫節點、再寫玩家背包),同時至多持一把,不互鎖。
                    let player_pos = app.players.read().unwrap().get(&id).map(|p| (p.x, p.y));
                    let gathered = player_pos
                        .and_then(|(px, py)| app.nodes.write().unwrap().gather_near(px, py));
                    if let Some((kind, amount)) = gathered {
                        // 豐饒術自動施放（ROADMAP 151）：設定自動且冷卻到期就自動觸發。
                        {
                            use crate::active_skill::ActiveSkillKind;
                            let should_auto = app.players.read().unwrap().get(&id).map(|p| {
                                p.auto_skills.contains("bounty")
                                    && !p.pending_bounty
                                    && p.skill_cooldowns.get(ActiveSkillKind::Bounty) == 0.0
                                    && ActiveSkillKind::Bounty.is_unlocked(&p.masteries)
                            }).unwrap_or(false);
                            if should_auto {
                                if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                                    p.pending_bounty = true;
                                    // 自動施放：熟練度縮短冷卻（ROADMAP 153）。
                                    let cd = p.skill_masteries.effective_cooldown(ActiveSkillKind::Bounty, ActiveSkillKind::Bounty.cooldown_secs());
                                    p.skill_cooldowns.set(ActiveSkillKind::Bounty, cd);
                                    p.skill_masteries.increment(ActiveSkillKind::Bounty);
                                }
                            }
                        }
                        // 天氣採集加成（ROADMAP 93）：對應生態域的天氣條件下採集 +1。
                        let weather_bonus: u32 = {
                            let biome_str = player_pos.map(|(px, py)| {
                                match world_core::biome_at(px as f64, py as f64) {
                                    world_core::Biome::Meadow => "meadow",
                                    world_core::Biome::Forest => "forest",
                                    world_core::Biome::Rocky => "rocky",
                                    world_core::Biome::Sand => "sand",
                                    world_core::Biome::Water => "water",
                                }
                            }).unwrap_or("");
                            if app.weather.read().unwrap().is_gather_bonus_biome(biome_str) { 1 } else { 0 }
                        };
                        let mut gather_level_up: Option<(String, u32)> = None;
                        if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                            let item: crate::inventory::ItemKind = kind.into();
                            // 工具效用(1-D):背包有鎬子/強化鎬就採更多(乘工具倍率)——
                            // 給合成出的工具一個用處,接上「採集→合成工具→採更快」迴圈。
                            let mult = crate::tools::gather_speed_multiplier(&p.inventory);
                            // 豐饒術（ROADMAP 45）：下次採集額外 +3 個；熟練加成再加（ROADMAP 153）。
                            let bounty_bonus = if p.pending_bounty {
                                p.pending_bounty = false;
                                crate::active_skill::BOUNTY_BONUS_QTY + p.skill_masteries.bounty_bonus_qty()
                            } else { 0 };
                            // 寵物採集加成（ROADMAP 46）：飄舞精靈每次額外 +1 物品。
                            let pet_gather = p.pet.map(|pk| pk.bonus_gather_qty()).unwrap_or(0);
                            // 星象預報豐收星象（ROADMAP 132）：採集每次額外 +1 物品。
                            let forecast_gather = {
                                let obs = app.observatory.read().unwrap();
                                if obs.is_active() && obs.current_bonus == crate::observatory::StarForecastBonus::GatherExtra {
                                    crate::observatory::StarForecastBonus::gather_extra_qty()
                                } else { 0 }
                            };
                            let (added, _wh, _drop) = p.add_item_overflow(item, amount * mult + bounty_bonus + pet_gather + weather_bonus + forecast_gather);
                            // 採集得 exp（鼓勵探索）；村落節慶加成 +30%（ROADMAP 64）；廣場聚會加成 +20%（ROADMAP 124）；繁榮紅利 +15/+30%（ROADMAP 129）。
                            let village_gather_pct = {
                                let lock = app.village_buff_until.read().unwrap();
                                lock.as_ref()
                                    .map(|&expiry| if std::time::Instant::now() < expiry { crate::village_chief::EVENT_EXP_BONUS_PCT } else { 0 })
                                    .unwrap_or(0)
                            };
                            let gathering_pct = if app.community_gathering.read().unwrap().is_active() {
                                crate::community_gathering::GATHERING_EXP_BONUS_PCT
                            } else { 0 };
                            // 星象預報吉星高照（ROADMAP 132）：EXP +25%。
                            let forecast_exp_pct = {
                                let obs = app.observatory.read().unwrap();
                                if obs.is_active() && obs.current_bonus == crate::observatory::StarForecastBonus::ExpBoost {
                                    crate::observatory::StarForecastBonus::exp_bonus_pct()
                                } else { 0 }
                            };
                            // 護符被動 EXP 加成：星際守護符 +15% > 星光護符 +10%（ROADMAP 133/134）。
                            let star_amulet_pct: u32 =
                                if p.inventory.count(crate::inventory::ItemKind::StarGuardianAmulet) > 0 { 15 }
                                else if p.inventory.count(crate::inventory::ItemKind::StarAmulet) > 0 { 10 }
                                else { 0 };
                            let prosperity_pct = crate::town_prosperity::level_from_u8(
                                app.residents.read().unwrap().prosperity_level()
                            ).exp_bonus_pct();
                            let gather_exp_base = 5u32
                                + 5 * village_gather_pct / 100
                                + 5 * gathering_pct / 100
                                + 5 * forecast_exp_pct / 100;
                            // 乙太花盆（ROADMAP 155）：住家放置後採集 EXP +8%。
                            let plant_pct = if app.home_furnishings.read().unwrap()
                                .get(&id).map(|h| h.has_plant()).unwrap_or(false) {
                                crate::home_furniture::PLANT_GATHER_EXP_PCT
                            } else { 0 };
                            // 農耕盛典（ROADMAP 156）：公民投票通過後採集 EXP +50%。
                            let farming_festival_pct = if app.civic_vote.read().unwrap().farming_festival_active() {
                                crate::civic_vote::FARMING_FESTIVAL_EXP_BONUS_PCT
                            } else { 0 };
                            // 合併所有百分比加成（避免 5*N/100=0 截斷），一次整數乘法。
                            let gather_exp = (gather_exp_base * (100 + prosperity_pct + star_amulet_pct + plant_pct + farming_festival_pct) + 50) / 100;
                            let old_level = p.level();
                            p.exp = p.exp.saturating_add(gather_exp);
                            if p.level() > old_level {
                                // 升等給屬性點（ROADMAP 152）：先加點再計算 max HP，因為屬性點本輪剛到不影響加成。
                                p.stats.unspent = p.stats.unspent.saturating_add(crate::stat_points::POINTS_PER_LEVEL);
                                let full_max = crate::vitals::level_max_hp(p.level())
                                    + crate::class::hp_bonus(&p.masteries)
                                    + p.stats.hp * crate::stat_points::HP_PER_POINT;
                                p.vitals.on_level_up(full_max);
                                gather_level_up = Some((p.name.clone(), p.level()));
                            }
                            p.masteries.gain_artisan(1); // 工匠熟練度：採集節點（ROADMAP 38）
                            tracing::info!(player = %p.name, ?item, added, mult, bounty_bonus, level = p.level(), "採集入背包+exp");
                        }
                        // NPC 升等賀詞（ROADMAP 84）：採集升等時凱爾長老私信賀詞 / 全服廣播。
                        if let Some((pname, new_lv)) = gather_level_up {
                            let action = app.npc_level_greet.write().unwrap().on_level_up(&pname, new_lv);
                            match action {
                                crate::npc_level_greet::LevelGreetAction::WorldBroadcast { message } => {
                                    let _ = app.tx_chat.send(format!(
                                        "🌟 [{}] 全服宣告：「{}」",
                                        crate::npc_level_greet::CHIEF_DISPLAY_NAME, message
                                    ));
                                }
                                crate::npc_level_greet::LevelGreetAction::DirectMessage { message } => {
                                    let _ = tx_direct.try_send(format!(
                                        "💬 [{}] 悄聲道：「{}」",
                                        crate::npc_level_greet::CHIEF_DISPLAY_NAME, message
                                    ));
                                }
                            }
                        }
                        // 通知社群任務（ROADMAP 27）：採集事件推進進度並廣播完成公告。
                        let item: crate::inventory::ItemKind = kind.into();
                        let completed = app.quests.write().unwrap().on_gather(item);
                        notify_quest_complete(&app, completed);
                        // 每日任務：採集事件（ROADMAP 32）。
                        if let Some(uid) = authed_uid {
                            advance_daily_gather(&app, uid, item, amount, &tx_direct);
                        }
                        // 旅行商人限時委託：採集事件（ROADMAP 136）。
                        if let Some(uid) = authed_uid {
                            let quest_result = app.wandering_merchant.write().unwrap().on_gather(item, amount as u32);
                            if let Some((qid, qname, ether_reward, reward_item, reward_qty)) = quest_result {
                                let pname = {
                                    let mut players = app.players.write().unwrap();
                                    if let Some(p) = players.get_mut(&uid) {
                                        p.ether = p.ether.saturating_add(ether_reward);
                                        p.add_item_overflow(reward_item, reward_qty);
                                        tracing::info!(
                                            player = %p.name, quest_id = qid, qname, ether_reward,
                                            ?reward_item, reward_qty, "完成旅行商人採集委託"
                                        );
                                        p.name.clone()
                                    } else { String::new() }
                                };
                                if !pname.is_empty() {
                                    let item_name = crate::npc_deal::item_display_zh(reward_item);
                                    // tokio mpsc 的 send() 回傳 Future,過去沒 .await 直接丟棄=從未送出;
                                    // 且單播通道載的是 JSON 字串,要包成 ServerMsg::Chat 客戶端才解析得到。
                                    let note = ServerMsg::Chat { from: "系統".into(), text: format!(
                                        "📋 委託「{}」完成！獲得 {} 乙太 + {}×{}！",
                                        qname, ether_reward, item_name, reward_qty
                                    ) };
                                    if let Ok(j) = serde_json::to_string(&note) { let _ = tx_direct.try_send(j); }
                                }
                            }
                        }
                    }
                }
                Ok(ClientMsg::Craft { recipe_id }) => {
                    // 合成(1-C):用配方自己的穩定 `id` 欄位(crafting 的權威 wire key)查 recipe_id,
                    // 在玩家自己背包上全有全無地合成(夠料才扣料+產出)。產物隨下一張快照回前端。
                    // 走既有 `recipe_by_id`(已測)而非每訊息 serde 重組產物名:免每筆配料一次 Value 配置,
                    // 也不把查找耦死在「id 必等於產物序列化名」上(同產物不同配料就會抓錯)。
                    if let Some(recipe) = crate::crafting::recipe_by_id(&recipe_id) {
                        // 城鎮慶典配方（ROADMAP 130）：合成前先確認城鎮繁榮等級達門檻。
                        let min_pros = crate::crafting::recipe_min_prosperity(&recipe_id);
                        let prosperity_ok = min_pros == 0
                            || app.residents.read().unwrap().prosperity_level() >= min_pros;
                        // 等級門檻（ROADMAP 145）：部分高階配方需達到最低等級。
                        let min_lv = crate::crafting::recipe_min_level(&recipe_id);
                        let level_ok = min_lv == 0 || {
                            app.players.read().unwrap()
                                .get(&id)
                                .map(|p| p.level() >= min_lv)
                                .unwrap_or(false)
                        };
                        if prosperity_ok && level_ok {
                            // 精密合成自動施放（ROADMAP 151）：設定自動且冷卻到期就自動觸發。
                            {
                                use crate::active_skill::ActiveSkillKind;
                                let should_auto = app.players.read().unwrap().get(&id).map(|p| {
                                    p.auto_skills.contains("precision")
                                        && !p.pending_precision
                                        && p.skill_cooldowns.get(ActiveSkillKind::Precision) == 0.0
                                        && ActiveSkillKind::Precision.is_unlocked(&p.masteries)
                                }).unwrap_or(false);
                                if should_auto {
                                    if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                                        p.pending_precision = true;
                                        // 自動施放：熟練度縮短冷卻（ROADMAP 153）。
                                        let cd = p.skill_masteries.effective_cooldown(ActiveSkillKind::Precision, ActiveSkillKind::Precision.cooldown_secs());
                                        p.skill_cooldowns.set(ActiveSkillKind::Precision, cd);
                                        p.skill_masteries.increment(ActiveSkillKind::Precision);
                                    }
                                }
                            }
                            if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                                let discount = crate::class::crafting_reduction(&p.masteries);
                                if recipe.craft_with_discount(&mut p.inventory, discount) {
                                    // 精密合成（ROADMAP 45）：下次合成額外 +1 個成品；熟練加成再加（ROADMAP 153）。
                                    let used_precision = p.pending_precision;
                                    if used_precision {
                                        p.pending_precision = false;
                                        let bonus_out = p.skill_masteries.precision_bonus_output();
                                        p.add_item_overflow(recipe.output, 1 + bonus_out);
                                    }
                                    p.masteries.gain_artisan(2); // 工匠熟練度（ROADMAP 38）
                                    tracing::info!(player = %p.name, recipe = %recipe_id, discount, precision = used_precision, "合成成功");
                                }
                            }
                        }
                    }
                }
                Ok(ClientMsg::ClaimPlot) => {
                    // 領地購買(③ Slice D)：已登入玩家可用乙太購買第一塊地。
                    if let Some(uid) = authed_uid {
                        let has_plot = app.plots.index_of(uid).is_some();
                        if !has_plot {
                            // 先在 players 鎖內只扣乙太/判斷，**放掉 players 鎖後**再碰 plots/fields。
                            // 絕不持 players 鎖跨去拿 fields/plots——會和遊戲迴圈的 nodes/enemies→players
                            // 鎖序顛倒，整個遊戲迴圈死鎖凍住、全服收不到快照（玩家進去只有場景沒角色）。
                            // 比照下方 BuyExpansion 已採用的「先 drop(players) 再碰 fields」做法。
                            let buyer = {
                                let mut players = app.players.write().unwrap();
                                match players.get_mut(&uid) {
                                    Some(p) if p.ether >= crate::economy::PLOT_COST => {
                                        p.ether -= crate::economy::PLOT_COST;
                                        Some(p.name.clone())
                                    }
                                    _ => None,
                                }
                            }; // players 鎖到此放掉
                            if let Some(name) = buyer {
                                let index = app.plots.claim(uid);
                                app.fields
                                    .write()
                                    .unwrap()
                                    .insert(uid, Field::for_plot(index));
                                tracing::info!(player = %name, index, "成功購買第一塊領地");
                                // 即時通知客戶端購買結果，不用等下一次快照廣播。
                                let _ = app.tx.send(Arc::new(ServerMsg::ClaimPlotOk {
                                    owner: uid,
                                    plot_index: index,
                                }));
                            }
                        }
                    }
                }
                Ok(ClientMsg::BuyExpansion) => {
                    // 農地擴張：已登入 + 已有地塊 + 乙太夠，才扣款並讓農地多開一列。
                    if let Some(uid) = authed_uid {
                        if app.plots.index_of(uid).is_some() {
                            let mut players = app.players.write().unwrap();
                            if let Some(p) = players.get_mut(&uid) {
                                if let Some(new_ether) = p.wallet.buy_expansion(p.ether) {
                                    p.ether = new_ether;
                                    let expansions = p.wallet.expansions();
                                    tracing::info!(player = %p.name, expansions, "擴地成功");
                                    // 農地 grow（在 fields 鎖內，不持 players 鎖跨鎖）。
                                    drop(players);
                                    app.fields.write().unwrap()
                                        .entry(uid)
                                        .and_modify(|f| f.grow());
                                }
                            }
                        }
                    }
                }
                Ok(ClientMsg::PostListing { item, qty, price_per }) => {
                    // 掛單：已登入 + 背包夠量才執行。扣背包→建掛單，原子操作（同一把 players 鎖）。
                    // 防外掛：price_per/qty 須 >0，且單價封頂（防超大數溢出與洗錢式天價掛單）。
                    const MAX_PRICE_PER: u32 = 1_000_000;
                    if let Some(uid) = authed_uid {
                        let pos = app.players.read().unwrap().get(&uid).map(|p| (p.x, p.y, p.name.clone()));
                        if let Some((px, py, name)) = pos {
                            let valid = qty > 0 && price_per > 0 && price_per <= MAX_PRICE_PER;
                            let ok = valid && {
                                let mut players = app.players.write().unwrap();
                                if let Some(p) = players.get_mut(&uid) {
                                    // 量不足拒絕
                                    p.inventory.take(item, qty)
                                } else { false }
                            };
                            if ok {
                                let listing = MarketListing {
                                    id: Uuid::new_v4(),
                                    seller_id: uid,
                                    seller_name: name,
                                    item,
                                    qty,
                                    price_per,
                                    x: px,
                                    y: py,
                                };
                                tracing::info!(player = %listing.seller_name, ?item, qty, price_per, "市場掛單");
                                app.market.write().unwrap().post(listing);
                            }
                        }
                    }
                }
                Ok(ClientMsg::BuyListing { listing_id }) => {
                    // 購買掛單：已登入 + 乙太足夠 + 不買自己掛單。
                    if let Some(uid) = authed_uid {
                        // 先讀掛單資訊（不持 market 鎖跨持 players 鎖）
                        let listing_info = {
                            let market = app.market.read().unwrap();
                            let found = market.all()
                                .find(|l| l.id == listing_id)
                                .map(|l| (l.seller_id, l.item, l.qty, l.price_per, l.seller_name.clone()));
                            found
                        };
                        if let Some((seller_id, item, qty, price_per, seller_name)) = listing_info {
                            if seller_id == uid {
                                // 不能買自己的掛單，靜默忽略
                            } else {
                                let total = price_per.saturating_mul(qty);
                                // 從買家扣乙太
                                let buyer_ok = {
                                    let mut players = app.players.write().unwrap();
                                    if let Some(p) = players.get_mut(&uid) {
                                        if p.ether >= total {
                                            p.ether -= total;
                                            true
                                        } else { false }
                                    } else { false }
                                };
                                if buyer_ok {
                                    // 從 market 移除掛單（確認掛單還存在才算成功）
                                    let bought = app.market.write().unwrap().buy(listing_id);
                                    if let Some(l) = bought {
                                        // 物品給買家背包
                                        {
                                            let mut players = app.players.write().unwrap();
                                            if let Some(p) = players.get_mut(&uid) {
                                                p.add_item_overflow(l.item, l.qty);
                                                tracing::info!(buyer = %p.name, ?item, qty, "市場購買成功");
                                            }
                                        }
                                        // 乙太給賣家（在線或離線都要補）
                                        let seller_online = {
                                            let mut players = app.players.write().unwrap();
                                            if let Some(sp) = players.get_mut(&seller_id) {
                                                sp.ether = sp.ether.saturating_add(total);
                                                tracing::info!(seller = %sp.name, ether = sp.ether, "市場售出獲得乙太");
                                                true
                                            } else { false }
                                        };
                                        if !seller_online {
                                            // 賣家離線：直接更新持久化 store 裡的乙太
                                            if let Some(saved) = app.positions.recall(seller_id) {
                                                let new_ether = saved.ether.saturating_add(total);
                                                app.positions.remember(
                                                    seller_id,
                                                    saved.x, saved.y,
                                                    new_ether,
                                                    saved.wallet_expansions,
                                                    saved.exp,
                                                    saved.masteries,
                                                    saved.stats,
                                                    saved.skill_masteries,
                                                    saved.codex,
                                                    saved.atlas,
                                                    saved.skylog,
                                                    saved.cheers,
                                                );
                                                tracing::info!(%seller_name, total, "市場售出（賣家離線）：乙太已寫入持久化");
                                            }
                                        }
                                    } else {
                                        // 掛單已消失（競態），把乙太退回買家
                                        let mut players = app.players.write().unwrap();
                                        if let Some(p) = players.get_mut(&uid) {
                                            p.ether = p.ether.saturating_add(total);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Ok(ClientMsg::CancelListing { listing_id }) => {
                    // 取消掛單（只有賣家本人有效）：退回物品至背包。
                    if let Some(uid) = authed_uid {
                        let returned = app.market.write().unwrap().cancel(listing_id, uid);
                        if let Some((item, qty)) = returned {
                            let mut players = app.players.write().unwrap();
                            if let Some(p) = players.get_mut(&uid) {
                                p.add_item_overflow(item, qty);
                                tracing::info!(player = %p.name, ?item, qty, "市場取消掛單，物品歸還");
                            }
                        }
                    }
                }
                Ok(ClientMsg::ShopSell { item, qty }) => {
                    // 向 NPC 商人賣出物品（浮動收購價，ROADMAP 40）。
                    // 支援故鄉、翠幽星、赤焰星、虛空星、霧醚星、星源星商人六處。
                    // 農夫/商人職業加成在浮動有效收購價上再疊。
                    let player_pos = app.players.read().unwrap().get(&id).map(|p| (p.x, p.y, p.vitals.is_downed()));
                    if let Some((px, py, downed)) = player_pos {
                        if !downed && qty > 0 {
                            // 決定最近的商人收購清單
                            let maybe_buy_list: Option<(&[npc::ShopEntry], &str)> =
                                if app.is_near_npc(px, py, "merchant") {
                                    Some((npc::NPC_BUY_LIST, "故鄉"))
                                } else if npc::is_within_verdant_shop_reach(px, py) {
                                    Some((npc::VERDANT_BUY_LIST, "翠幽星"))
                                } else if npc::is_within_crimson_shop_reach(px, py) {
                                    Some((npc::CRIMSON_BUY_LIST, "赤焰星"))
                                } else if npc::is_within_void_shop_reach(px, py) {
                                    Some((npc::VOID_BUY_LIST, "虛空星"))
                                } else if npc::is_within_aether_shop_reach(px, py) {
                                    Some((npc::AETHER_BUY_LIST, "霧醚星"))
                                } else if npc::is_within_origin_shop_reach(px, py) {
                                    Some((npc::ORIGIN_BUY_LIST, "星源星"))
                                } else {
                                    None
                                };

                            if let Some((buy_list, merchant_name)) = maybe_buy_list {
                                // 查基準收購價（確認物品在清單內）
                                if let Some(base_price) = buy_list.iter().find(|e| e.item == item).map(|e| e.price_per) {
                                    // ROADMAP 102：單筆內批量漸降價。
                                    // 防 TOCTOU（公測前外掛硬化）：金庫「讀餘額→算可成交量→扣帳」三步驟
                                    // 必須在同一把 npc_treasury 寫鎖的臨界區內原子完成，否則併發賣貨
                                    // 會各自以同一份過時餘額算出成本、雙雙扣帳超抽金庫。
                                    // 鎖序：此處只持 npc_treasury 寫鎖（不與 players 重疊），扣完即放，再鎖 players。
                                    let now_secs = unix_secs();
                                    let (actual_qty, bulk_cost, treasury_notice) = {
                                        let mut treasury = app.npc_treasury.write().unwrap();
                                        let treasury_balance = treasury.balance(merchant_name);
                                        let (aq, cost, notice) = app.dynamic_prices.read().unwrap()
                                            .find_bulk_affordable(item, base_price, qty, treasury_balance, now_secs);
                                        // 同一臨界區內立即扣帳（reserve），避免 check 後放鎖再扣的競態。
                                        if cost > 0 {
                                            treasury.deduct(merchant_name, cost);
                                        }
                                        (aq, cost, notice)
                                    };

                                    if actual_qty == 0 {
                                        // 金庫清空，婉拒收購，私訊玩家
                                        let _ = tx_direct.try_send(format!(
                                            "💰 [{}商人] 婉拒道：「今天現金已見底，明天商隊回來後再來吧！」",
                                            merchant_name
                                        ));
                                    } else {
                                        // 急收令加成（ROADMAP 85）：以 actual_qty 重算，非原始 qty。
                                        let (commission_bonus_per_unit, commission_item_name) = if merchant_name == "故鄉" {
                                            let c = app.npc_commission.read().unwrap();
                                            match &c.active {
                                                Some(ac) if ac.item == item => (ac.bonus_per_unit, Some(ac.item_name)),
                                                _ => (0, None),
                                            }
                                        } else {
                                            (0, None)
                                        };
                                        let commission_bonus = commission_bonus_per_unit.saturating_mul(actual_qty);

                                        // 議價術自動施放（ROADMAP 151）：設定自動且冷卻到期就自動觸發。
                                        {
                                            use crate::active_skill::ActiveSkillKind;
                                            let should_auto = app.players.read().unwrap().get(&id).map(|p| {
                                                p.auto_skills.contains("haggle")
                                                    && !p.pending_haggle
                                                    && p.skill_cooldowns.get(ActiveSkillKind::Haggle) == 0.0
                                                    && ActiveSkillKind::Haggle.is_unlocked(&p.masteries)
                                            }).unwrap_or(false);
                                            if should_auto {
                                                if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                                                    p.pending_haggle = true;
                                                    // 自動施放：熟練度縮短冷卻（ROADMAP 153）。
                                                    let cd = p.skill_masteries.effective_cooldown(ActiveSkillKind::Haggle, ActiveSkillKind::Haggle.cooldown_secs());
                                                    p.skill_cooldowns.set(ActiveSkillKind::Haggle, cd);
                                                    p.skill_masteries.increment(ActiveSkillKind::Haggle);
                                                }
                                            }
                                        }
                                        // 扣除背包物品、結算乙太（write lock）
                                        let did_sell = {
                                            let mut players = app.players.write().unwrap();
                                            if let Some(p) = players.get_mut(&id) {
                                                if p.inventory.take(item, actual_qty) {
                                                    let earned = bulk_cost; // 批量漸降價總收益（ROADMAP 102）
                                                    let class_bonus = crate::class::apply_npc_bonus(&p.masteries, earned) - earned;
                                                    // 議價術（ROADMAP 45）：下次 NPC 賣出額外多得等額乙太（總收入 ×2）；熟練加成再加%（ROADMAP 153）。
                                                    let haggle_bonus = if p.pending_haggle {
                                                        p.pending_haggle = false;
                                                        let base_bonus = earned.saturating_add(class_bonus);
                                                        let mastery_extra_pct = p.skill_masteries.haggle_bonus_pct();
                                                        let mastery_extra = earned.saturating_mul(mastery_extra_pct) / 100;
                                                        base_bonus.saturating_add(mastery_extra)
                                                    } else { 0 };
                                                    // 星象預報金星入市（ROADMAP 132）：NPC 收購 +15%。
                                                    let forecast_npc_bonus = {
                                                        let obs = app.observatory.read().unwrap();
                                                        if obs.is_active() && obs.current_bonus == crate::observatory::StarForecastBonus::NpcBonus {
                                                            earned * crate::observatory::StarForecastBonus::npc_bonus_pct() / 100
                                                        } else { 0 }
                                                    };
                                                    // 古代擺件（ROADMAP 155）：住家放置後 NPC 收購 +10%。
                                                    let deco_npc_bonus = if app.home_furnishings.read().unwrap()
                                                        .get(&id).map(|h| h.has_deco()).unwrap_or(false) {
                                                        earned * crate::home_furniture::DECO_NPC_BONUS_PCT / 100
                                                    } else { 0 };
                                                    // 夜市開張（ROADMAP 156）：公民投票通過後 NPC 收購 +15%。
                                                    let night_market_bonus = if app.civic_vote.read().unwrap().night_market_active() {
                                                        earned * crate::civic_vote::NIGHT_MARKET_BUY_BONUS_PCT / 100
                                                    } else { 0 };
                                                    tracing::info!(player = %p.name, ?item, actual_qty, earned, class_bonus, haggle_bonus, commission_bonus, forecast_npc_bonus, deco_npc_bonus, night_market_bonus, bulk_cost, merchant_name, "NPC 收購（批量漸降價）");
                                                    p.ether = p.ether.saturating_add(earned).saturating_add(class_bonus).saturating_add(haggle_bonus).saturating_add(commission_bonus).saturating_add(forecast_npc_bonus).saturating_add(deco_npc_bonus).saturating_add(night_market_bonus);
                                                    p.masteries.gain_merchant(1); // 商人熟練度（ROADMAP 38）
                                                    true
                                                } else {
                                                    false
                                                }
                                            } else {
                                                false
                                            }
                                        }; // players write lock 在此釋放

                                        if !did_sell {
                                            // 背包扣除失敗（理論上 qty 已驗證，極端競態才會發生）：
                                            // 把先前原子扣下的金庫餘額退回，避免金庫白白蒸發。
                                            if bulk_cost > 0 {
                                                app.npc_treasury.write().unwrap()
                                                    .refund_amount(merchant_name, bulk_cost);
                                            }
                                        }
                                        if did_sell {
                                            // 金庫已於上方臨界區原子扣帳，此處不再重複扣。

                                            // 通知玩家部分收購（ROADMAP 100）。
                                            if treasury_notice {
                                                let _ = tx_direct.try_send(format!(
                                                    "💰 [{}商人] 說：「現金快見底了，只收了 {} 個，改天再來吧！」",
                                                    merchant_name, actual_qty
                                                ));
                                            }

                                            // 記錄賣出量，更新浮動收購價
                                            app.dynamic_prices.write().unwrap()
                                                .record_sale(item, actual_qty, now_secs);
                                            // 急收令進度追蹤（ROADMAP 85）
                                            if commission_bonus > 0 {
                                                let sell_result = app.npc_commission.write().unwrap()
                                                    .on_sold(item, actual_qty);
                                                if sell_result.fulfilled {
                                                    if let Some(item_name) = commission_item_name {
                                                        let merchant = crate::npc_commission::MERCHANT_DISPLAY_NAME;
                                                        let _ = app.tx_chat.send(format!(
                                                            "✅ [{merchant}] 宣告：「{}」",
                                                            crate::npc_commission::fulfilled_text(item_name)
                                                        ));
                                                    }
                                                }
                                            }
                                            // 關係綁真實交易（ROADMAP 61）
                                            if let Some(uid) = authed_uid {
                                                if app.is_near_npc(px, py, "merchant") {
                                                    let updated_rel = {
                                                        let mut mem = app.npc_memory.write().unwrap();
                                                        let r = mem.entry((uid, "merchant".to_string())).or_default();
                                                        r.sell_count = r.sell_count.saturating_add(1);
                                                        r.clone()
                                                    };
                                                    app.npc_needs.write().unwrap().apply_trade(true);
                                                    if updated_rel.sell_count % crate::npc_chat::TRADE_STOCK_EARN_INTERVAL == 0 {
                                                        let new_stock = {
                                                            let mut stk = app.npc_gift_stock.write().unwrap();
                                                            let s = stk.entry("merchant".to_string()).or_insert(0);
                                                            *s = crate::npc_chat::restock_npc_stock(*s);
                                                            *s
                                                        };
                                                        app.npc_memory_store.save_gift_stock("merchant".to_string(), new_stock);
                                                        tracing::debug!(sell_count = updated_rel.sell_count, new_stock, "貿易補貨：商人餘裕 +1");
                                                    }
                                                    app.npc_memory_store.save_rel(uid, "merchant".to_string(), updated_rel);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Ok(ClientMsg::ShopBuy { item, qty }) => {
                    // 向 NPC 商人購買物品：目前只有故鄉商人有販售清單。
                    let player_pos = app.players.read().unwrap().get(&id).map(|p| (p.x, p.y, p.vitals.is_downed()));
                    if let Some((px, py, downed)) = player_pos {
                        if !downed && app.is_near_npc(px, py, "merchant") {
                            // ROADMAP 104：庫存檢查——先確認物品在販售清單且有庫存。
                            let base_price = npc::NPC_SELL_LIST.iter()
                                .find(|e| e.item == item)
                                .map(|e| e.price_per);
                            let Some(base_price) = base_price else { continue; };
                            // 取有效價（含稀缺溢價）。
                            let effective_price = app.npc_stock.read().unwrap()
                                .effective_sell_price(
                                    crate::npc_treasury::MERCHANT_HOME,
                                    item,
                                    base_price,
                                );
                            // 嘗試從庫存扣除（若庫存不足則傳送提示後中止）。
                            let stock_result = app.npc_stock.write().unwrap()
                                .try_purchase(crate::npc_treasury::MERCHANT_HOME, item, qty);
                            if stock_result.actual_qty == 0 {
                                // 完全缺貨。
                                if let Some(notice) = stock_result.notice {
                                    let chat = crate::protocol::ServerMsg::Chat {
                                        from: "商人薇拉".to_string(),
                                        text: format!("💤 {notice}"),
                                    };
                                    if let Ok(json) = serde_json::to_string(&chat) {
                                        let _ = tx_direct.send(json).await;
                                    }
                                }
                                continue;
                            }
                            // 若只能部分成交：通知玩家，以實際可買量繼續。
                            let actual_qty = stock_result.actual_qty;
                            if let Some(notice) = stock_result.notice {
                                let chat = crate::protocol::ServerMsg::Chat {
                                    from: "商人薇拉".to_string(),
                                    text: format!("📦 {notice}"),
                                };
                                if let Ok(json) = serde_json::to_string(&chat) {
                                    let _ = tx_direct.send(json).await;
                                }
                            }
                            // 熟客折扣（ROADMAP 63）：取出待用折扣票（若有效期未過）。
                            // 票只在成功購買後才消耗；失敗不扣票（讓玩家有機會補足乙太再試）。
                            let discount_pct = {
                                let pending = app.npc_pending_discount.read().unwrap();
                                pending.get(&id).and_then(|(pct, expiry)| {
                                    if std::time::Instant::now() < *expiry { Some(*pct) } else { None }
                                }).unwrap_or(0)
                            };
                            let did_buy = {
                                if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                                    let old_ether = p.ether;
                                    // 使用有效價（含稀缺溢價）完成購買。
                                    if let Some(new_ether) = npc::buy_from_npc_at_price(
                                        &mut p.inventory, p.ether, item, actual_qty, effective_price, discount_pct
                                    ) {
                                        tracing::info!(
                                            player = %p.name, ?item, actual_qty,
                                            spent = old_ether - new_ether,
                                            effective_price, discount_pct,
                                            "NPC 販售（庫存扣除）"
                                        );
                                        p.ether = new_ether;
                                        true
                                    } else {
                                        // 乙太不足：退還庫存（補回扣除的數量）。
                                        drop(p);
                                        app.npc_stock.write().unwrap()
                                            .refund(crate::npc_treasury::MERCHANT_HOME, item, actual_qty);
                                        false
                                    }
                                } else {
                                    // 玩家不在線：退還庫存。
                                    app.npc_stock.write().unwrap()
                                        .refund(crate::npc_treasury::MERCHANT_HOME, item, actual_qty);
                                    false
                                }
                            };
                            // 購買成功後：清除折扣票（已使用）。
                            if did_buy && discount_pct > 0 {
                                app.npc_pending_discount.write().unwrap().remove(&id);
                                tracing::info!(player_id = %id, "熟客折扣已套用並清除");
                            }
                            // 關係綁真實交易（ROADMAP 61）：向故鄉商人購買時累積 buy_count。
                            // 需求驅力（ROADMAP 69）：玩家向商人購買，商人繁榮感微升。
                            if did_buy {
                                if let Some(uid) = authed_uid {
                                    let updated_rel = {
                                        let mut mem = app.npc_memory.write().unwrap();
                                        let r = mem.entry((uid, "merchant".to_string())).or_default();
                                        r.buy_count = r.buy_count.saturating_add(1);
                                        r.clone()
                                    };
                                    app.npc_memory_store.save_rel(uid, "merchant".to_string(), updated_rel);
                                }
                                app.npc_needs.write().unwrap().apply_trade(false);
                            }
                        }
                    }
                }
                // ── ROADMAP 101：玩家確認 / 拒絕 AI 議價──────────────────────────────
                Ok(ClientMsg::ConfirmDeal { accept }) => {
                    // 必須已登入。訪客不參與議價（無購買紀錄，商人也不會提議）。
                    let uid = match authed_uid {
                        Some(u) => u,
                        None => continue,
                    };
                    // 取出待確認議價（同時移除——每人只能用一次，不論接受或拒絕）。
                    let pending = {
                        let mut map = app.npc_pending_deal.write().unwrap();
                        map.remove(&uid)
                    };
                    let Some(deal) = pending else { continue };
                    if deal.is_expired() {
                        // 到期通知（讓玩家知道為何無效）。
                        let msg = crate::protocol::ServerMsg::Chat {
                            from: "系統".to_string(),
                            text: "⏰ 議價已過期，請再找商人重新洽談。".to_string(),
                        };
                        if let Ok(json) = serde_json::to_string(&msg) {
                            let _ = tx_direct.send(json).await;
                        }
                        continue;
                    }
                    if !accept { continue; } // 拒絕：靜默清除，已在上面 remove
                    // ── 接受：重新驗證（防時間差）→ 執行交易 ──────────────
                    let total = deal.total();
                    let treasury_ok = app.npc_treasury.read().unwrap()
                        .balance(crate::npc_treasury::MERCHANT_HOME) >= total;
                    if !treasury_ok {
                        let msg = crate::protocol::ServerMsg::Chat {
                            from: "系統".to_string(),
                            text: "💸 商人現金不足，無法成交。".to_string(),
                        };
                        if let Ok(json) = serde_json::to_string(&msg) {
                            let _ = tx_direct.send(json).await;
                        }
                        continue;
                    }
                    let (traded, player_name) = {
                        let mut players = app.players.write().unwrap();
                        if let Some(p) = players.get_mut(&uid) {
                            if p.inventory.take(deal.item, deal.qty) {
                                p.ether = p.ether.saturating_add(total);
                                (true, p.name.clone())
                            } else {
                                (false, p.name.clone())
                            }
                        } else {
                            (false, String::new())
                        }
                    };
                    if !traded {
                        let msg = crate::protocol::ServerMsg::Chat {
                            from: "系統".to_string(),
                            text: "📦 背包物品不足，無法完成議價。".to_string(),
                        };
                        if let Ok(json) = serde_json::to_string(&msg) {
                            let _ = tx_direct.send(json).await;
                        }
                        continue;
                    }
                    // 金庫扣款。
                    app.npc_treasury.write().unwrap()
                        .deduct(crate::npc_treasury::MERCHANT_HOME, total);
                    // 更新商人對玩家的 sell_count（引擎事實統計）。
                    let updated_rel = {
                        let mut mem = app.npc_memory.write().unwrap();
                        let r = mem.entry((uid, "merchant".to_string())).or_default();
                        r.sell_count = r.sell_count.saturating_add(1);
                        r.clone()
                    };
                    app.npc_memory_store.save_rel(uid, "merchant".to_string(), updated_rel);
                    // 需求驅力（ROADMAP 69）：成交 → 商人繁榮感升。
                    app.npc_needs.write().unwrap().apply_trade(true);
                    // 成交通知（私訊玩家）。
                    let item_name = crate::npc_deal::item_display_zh(deal.item);
                    let success_msg = crate::protocol::ServerMsg::Chat {
                        from: "商人薇拉".to_string(),
                        text: format!("🤝 議價成交！收了你 {} 個{}，付你 {} 乙太。感謝這筆生意！",
                            deal.qty, item_name, total),
                    };
                    if let Ok(json) = serde_json::to_string(&success_msg) {
                        let _ = tx_direct.send(json).await;
                    }
                    tracing::info!(player = %player_name, item = ?deal.item, qty = deal.qty, total, "議價成交");
                }

                Ok(ClientMsg::Dig { wx, wy }) => {
                    // C-2 挖掘地形格：倒地中不可挖（與採集/耕種同規則）。
                    if app.players.read().unwrap().get(&id).map(|p| p.vitals.is_downed()).unwrap_or(false) {
                        continue;
                    }
                    // 換算格座標，計算格中心世界像素座標，驗可及距離。
                    let (cx, cy, tx, ty) = crate::tiles::world_to_cell(wx, wy);
                    let (ccx, ccy) = crate::tiles::cell_center(cx, cy, tx, ty);
                    let player_pos = app.players.read().unwrap().get(&id).map(|p| (p.x, p.y));
                    let Some((px, py)) = player_pos else { continue; };
                    let dist_sq = (ccx - px) * (ccx - px) + (ccy - py) * (ccy - py);
                    let reach = crate::tiles::DIG_REACH;
                    if dist_sq > reach * reach { continue; }
                    // 查當前格種類；只能挖實心格（Empty 靜默忽略）。
                    let kind = app.tile_world.read().unwrap().tile_kind(cx, cy, tx, ty);
                    if kind == world_core::TileKind::Empty { continue; }
                    // 城牆是不可挖結構（玩家安全區的硬邊界），拒挖。
                    if kind == world_core::TileKind::TownWall { continue; }
                    // 產權保護（ROADMAP 34）：若此格在他人購買的城外地塊內，拒絕。
                    if let Some(uid) = authed_uid {
                        if app.land_plots.read().unwrap().is_protected_from(ccx, ccy, uid) {
                            continue;
                        }
                    } else {
                        // 訪客：只要格子在任何已購地塊內就拒絕（無身份無法判地主）。
                        if app.land_plots.read().unwrap().is_protected_from(ccx, ccy, uuid::Uuid::nil()) {
                            continue;
                        }
                    }
                    // 挖掘：更新記憶體 delta（記為 Empty），非同步落地到 DB。
                    app.tile_world.write().unwrap().apply_delta(cx, cy, tx, ty, world_core::TileKind::Empty);
                    let store = app.tile_store.clone();
                    tokio::spawn(async move {
                        store.upsert_delta(cx, cy, tx, ty, world_core::TileKind::Empty).await;
                    });
                    // 掉落材料入背包（工具加速倍率與採集一致）。
                    if let Some((item, qty)) = crate::tiles::drop_for_tile(kind) {
                        if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                            let mult = crate::tools::gather_speed_multiplier(&p.inventory);
                            let (added, _wh, _drop) = p.add_item_overflow(item, qty * mult);
                            p.masteries.gain_artisan(1); // 工匠熟練度：挖礦（ROADMAP 38）
                            tracing::info!(player = %p.name, ?item, added, "挖掘掉落");
                        }
                    }
                }
                Ok(ClientMsg::Place { wx, wy, material }) => {
                    // C-4 建造：倒地中不可放置（與挖掘同規則）。
                    if app.players.read().unwrap().get(&id).map(|p| p.vitals.is_downed()).unwrap_or(false) {
                        continue;
                    }
                    // 換算格座標，計算格中心世界像素座標，驗可及距離。
                    let (cx, cy, tx, ty) = crate::tiles::world_to_cell(wx, wy);
                    let (ccx, ccy) = crate::tiles::cell_center(cx, cy, tx, ty);
                    let player_pos = app.players.read().unwrap().get(&id).map(|p| (p.x, p.y));
                    let Some((px, py)) = player_pos else { continue; };
                    let dist_sq = (ccx - px) * (ccx - px) + (ccy - py) * (ccy - py);
                    if dist_sq > crate::tiles::DIG_REACH * crate::tiles::DIG_REACH { continue; }
                    // 城內禁止放置方塊：保護城鎮動線（不准把出生點/城門/NPC 圍死）。
                    let (pcx, pcy) = crate::tiles::cell_center(cx, cy, tx, ty);
                    if world_core::town_interior_at(pcx as f64, pcy as f64) { continue; }
                    // 產權保護（ROADMAP 34）：若此格在他人已購城外地塊內，拒絕放置。
                    if let Some(uid) = authed_uid {
                        if app.land_plots.read().unwrap().is_protected_from(pcx, pcy, uid) {
                            continue;
                        }
                    } else {
                        if app.land_plots.read().unwrap().is_protected_from(pcx, pcy, uuid::Uuid::nil()) {
                            continue;
                        }
                    }
                    // 只能放在 Empty 格（不可疊建）。
                    let current_kind = app.tile_world.read().unwrap().tile_kind(cx, cy, tx, ty);
                    if current_kind != world_core::TileKind::Empty { continue; }
                    // 驗材料字串是否合法且可放置。
                    let Some(tile_kind) = crate::tiles::tile_for_item(&material) else { continue; };
                    let Some(item_kind) = crate::tiles::item_for_placeable_tile(tile_kind) else { continue; };
                    // 扣背包（背包不足則靜默忽略）；同時取得玩家名供日誌。
                    let player_name = {
                        let mut players = app.players.write().unwrap();
                        let Some(p) = players.get_mut(&id) else { continue; };
                        if !p.inventory.take(item_kind, 1) { continue; }
                        p.name.clone()
                    };
                    // 更新記憶體 delta（設為實心格），非同步落地到 DB。
                    app.tile_world.write().unwrap().apply_delta(cx, cy, tx, ty, tile_kind);
                    let store = app.tile_store.clone();
                    tokio::spawn(async move {
                        store.upsert_delta(cx, cy, tx, ty, tile_kind).await;
                    });
                    tracing::info!(player = %player_name, ?tile_kind, "建造放置");
                }
                Ok(ClientMsg::PlaceSprinkler { wx, wy }) => {
                    // 放置灑水器（ROADMAP 112）：背包有 Sprinkler、未倒地、放置點在自己農地 FARM_REACH 內。
                    let uid = match authed_uid {
                        Some(u) => u,
                        None => continue, // 訪客不能放置
                    };
                    // 驗未倒地。
                    if app.players.read().unwrap().get(&id).map(|p| p.vitals.is_downed()).unwrap_or(true) {
                        continue;
                    }
                    // 驗放置點在自己農地的 FARM_REACH 內（保護：不能隔空放到別人地塊旁）。
                    let in_reach = {
                        let fields = app.fields.read().unwrap();
                        fields.get(&uid).map(|f| f.within_reach(wx, wy)).unwrap_or(false)
                    };
                    if !in_reach {
                        continue;
                    }
                    // 扣背包 1 個 Sprinkler。
                    let ok = {
                        let mut players = app.players.write().unwrap();
                        players.get_mut(&id).map(|p| p.inventory.take(crate::inventory::ItemKind::Sprinkler, 1)).unwrap_or(false)
                    };
                    if !ok {
                        continue;
                    }
                    // 先加進記憶體（db_id=0），再非同步落地拿到真實 id。
                    let data = crate::sprinkler::SprinklerData::new(wx, wy);
                    app.sprinklers.write().unwrap().add(uid, data);
                    let persist = app.sprinkler_persist.clone();
                    let store = app.sprinklers.clone();
                    tokio::spawn(async move {
                        let db_id = persist.insert(uid, wx, wy).await;
                        if db_id != 0 {
                            // 補上真實 db_id（找到 db_id==0 且位置匹配的最後一個灑水器）。
                            store.write().unwrap().update_db_id(uid, wx, wy, db_id);
                        }
                    });
                    tracing::info!(user_id = %uid, wx, wy, "放置灑水器");
                }
                Ok(ClientMsg::Attack) => {
                    // 主動攻擊：驗未倒地、冷卻已到期，再打射程內最近的存活敵人。
                    // 遠程武器（ROADMAP 146）：射程 3 倍於近戰；在安全區內遠程攻擊不給獎勵（防龜城）。
                    // 鎖序：讀 players（取位置+冷卻） → 寫 enemies（attack_nearest） → 寫 players（設冷卻+掉落）。
                    const ATTACK_COOLDOWN_SECS: f32 = 0.6;
                    let is_night = app.daynight.read().unwrap().phase() == crate::daynight::Phase::Night;
                    let has_lantern = app.home_furnishings.read().unwrap()
                        .get(&id).map(|h| h.has_lantern()).unwrap_or(false);
                    let info = app.players.read().unwrap().get(&id).map(|p| {
                        use crate::refinement::{enchant_extra_damage, is_crit_tick};
                        let enchant = p.equipment.weapon_meta.enchant;
                        let attempt = p.kill_count as u64;
                        let lantern_bonus = if is_night && has_lantern {
                            crate::home_furniture::LANTERN_NIGHT_ATK_BONUS as u32
                        } else { 0 };
                        let base_power = crate::equipment::equipped_weapon_power(&p.equipment)
                            + crate::combat::level_attack_bonus(p.level())
                            + crate::class::combat_bonus(&p.masteries)
                            + enchant_extra_damage(enchant)
                            + p.pet.map(|pk| pk.bonus_attack()).unwrap_or(0)
                            + lantern_bonus; // 星燈夜間攻擊加成（ROADMAP 155）
                        // 暴擊：每 5 次攻擊有一次雙倍傷害。
                        let power = if enchant == Some(crate::refinement::EnchantKind::CritStrike)
                            && is_crit_tick(attempt) {
                            base_power * 2
                        } else {
                            base_power
                        };
                        // 讀取裝備武器種類（遠程判斷用）。
                        let weapon_kind = p.equipment.weapon
                            .and_then(crate::combat::weapon_from_item)
                            .unwrap_or(crate::combat::WeaponKind::Unarmed);
                        (p.x, p.y, p.vitals.is_downed(), p.attack_cooldown, power, enchant, weapon_kind)
                    });
                    let Some((px, py, downed, cooldown, power, enchant, weapon_kind)) = info else { continue; };
                    if downed || cooldown > 0.0 { continue; }

                    // 遠程武器：使用較大射程；在安全區內時禁止給獎勵（防龜城刷怪）。
                    let is_ranged = weapon_kind.is_ranged();
                    let attack_reach = if is_ranged {
                        crate::combat::RANGED_ATTACK_REACH
                    } else {
                        crate::enemy_field::ATTACK_REACH
                    };
                    let in_safe_zone = crate::positions::is_in_safe_zone(px, py);
                    // 安全區防呆：玩家在城鎮範圍內用遠程打外面的怪不給獎勵/exp
                    let suppress_rewards = is_ranged && in_safe_zone;

                    // 戰吼（ROADMAP 45 / 151 自動施放）：讀取旗標、決定單攻或群攻，然後清旗。
                    // 若設定自動且冷卻到期、技能已解鎖、尚未 pending，先自動觸發。
                    {
                        use crate::active_skill::ActiveSkillKind;
                        let should_auto = app.players.read().unwrap().get(&id).map(|p| {
                            p.auto_skills.contains("warcry")
                                && !p.pending_warcry
                                && p.skill_cooldowns.get(ActiveSkillKind::Warcry) == 0.0
                                && ActiveSkillKind::Warcry.is_unlocked(&p.masteries)
                                && !p.vitals.is_downed()
                        }).unwrap_or(false);
                        if should_auto {
                            if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                                p.pending_warcry = true;
                                // 自動施放：熟練度縮短冷卻（ROADMAP 153）。
                                let cd = p.skill_masteries.effective_cooldown(ActiveSkillKind::Warcry, ActiveSkillKind::Warcry.cooldown_secs());
                                p.skill_cooldowns.set(ActiveSkillKind::Warcry, cd);
                                p.skill_masteries.increment(ActiveSkillKind::Warcry);
                            }
                        }
                    }
                    let (use_warcry, warcry_bonus_reach) = app.players.read().unwrap()
                        .get(&id).map(|p| (p.pending_warcry, p.skill_masteries.warcry_bonus_reach_px())).unwrap_or((false, 0.0));
                    let results: Vec<_> = if use_warcry {
                        // 戰吼：熟練度加成群攻範圍（ROADMAP 153）。
                        let effective_reach = attack_reach + warcry_bonus_reach;
                        app.enemies.write().unwrap().attack_all_in_reach(px, py, power, effective_reach)
                    } else {
                        app.enemies.write().unwrap().attack_nearest(px, py, power, attack_reach)
                            .into_iter().collect()
                    };
                    // 遠程攻擊廣播：只要玩家出手就播（命中或未命中皆可），前端負責動畫。
                    if is_ranged {
                        let hit = !results.is_empty();
                        let _ = app.tx.send(std::sync::Arc::new(
                            crate::protocol::ServerMsg::RangedHit { from_x: px, from_y: py, hit }
                        ));
                    }
                    // 取第一筆的兇名狀態（單攻時只有最多一筆，群攻取第一隻兇名）
                    let was_notorious = results.iter().any(|(_, _, n, _)| *n);
                    let result: Option<(crate::combat::EnemyKind, u32, bool, Option<(crate::inventory::ItemKind, u32)>)> =
                        results.iter().find(|(_, _, _, loot)| loot.is_some()).cloned();
                    let mut combat_level_up: Option<(String, u32)> = None;
                    if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                        // 攻擊速度加點縮短攻擊冷卻（ROADMAP 152）。
                        p.attack_cooldown = p.stats.effective_attack_cooldown(ATTACK_COOLDOWN_SECS);
                        if use_warcry { p.pending_warcry = false; }
                        // 彙整所有戰利品（單攻時 results 最多一筆；戰吼時可能多筆）。
                        // 安全區防呆：遠程在城內打城外怪不給獎勵。
                        let mut had_kill = false;
                        for (kind, enemy_level, notorious, loot) in &results {
                            let Some((item, qty)) = loot else { continue; };
                            if suppress_rewards { continue; }  // 安全區遠程無獎勵
                            had_kill = true;
                            p.add_item_overflow(*item, *qty);
                            let base_reward = crate::combat::scaled_exp(kind.exp_reward(), *enemy_level);
                            let notorious_mult = if *notorious { 2.0_f32 } else { 1.0_f32 };
                            // 寵物經驗加成（ROADMAP 46）：珊瑚蟹 +20% 擊殺經驗。
                            let pet_exp_pct = p.pet.map(|pk| pk.bonus_exp_pct()).unwrap_or(0);
                            let pet_mult = 1.0_f32 + pet_exp_pct as f32 / 100.0;
                            // 村落節慶加成（ROADMAP 64）：里長辦活動期間全服 EXP +30%。
                            let village_buff_pct = {
                                let lock = app.village_buff_until.read().unwrap();
                                lock.as_ref()
                                    .map(|&expiry| if std::time::Instant::now() < expiry { crate::village_chief::EVENT_EXP_BONUS_PCT } else { 0 })
                                    .unwrap_or(0)
                            };
                            let village_mult = 1.0_f32 + village_buff_pct as f32 / 100.0;
                            // 廣場聚會加成（ROADMAP 124）：聚會期間全服 EXP +20%。
                            let gathering_mult = if app.community_gathering.read().unwrap().is_active() {
                                1.0_f32 + crate::community_gathering::GATHERING_EXP_BONUS_PCT as f32 / 100.0
                            } else { 1.0_f32 };
                            // 繁榮紅利（ROADMAP 129）：生機 +15%/繁盛 +30% EXP。
                            let prosperity_mult = 1.0_f32 + crate::town_prosperity::level_from_u8(
                                app.residents.read().unwrap().prosperity_level()
                            ).exp_bonus_pct() as f32 / 100.0;
                            // 星象預報吉星高照（ROADMAP 132）：EXP +25%。
                            let forecast_mult = {
                                let obs = app.observatory.read().unwrap();
                                if obs.is_active() && obs.current_bonus == crate::observatory::StarForecastBonus::ExpBoost {
                                    1.0_f32 + crate::observatory::StarForecastBonus::exp_bonus_pct() as f32 / 100.0
                                } else { 1.0_f32 }
                            };
                            // 護符被動 EXP 加成：星際守護符 +15% > 星光護符 +10%（ROADMAP 133/134）。
                            let star_amulet_mult =
                                if p.inventory.count(crate::inventory::ItemKind::StarGuardianAmulet) > 0 { 1.15_f32 }
                                else if p.inventory.count(crate::inventory::ItemKind::StarAmulet) > 0 { 1.1_f32 }
                                else { 1.0_f32 };
                            let reward = (base_reward as f32
                                * crate::refinement::enchant_exp_multiplier(enchant)
                                * notorious_mult
                                * pet_mult
                                * village_mult
                                * gathering_mult
                                * prosperity_mult
                                * forecast_mult
                                * star_amulet_mult) as u32;
                            let old_level = p.level();
                            p.exp = p.exp.saturating_add(reward);
                            if p.level() > old_level {
                                // 升等給屬性點（ROADMAP 152）：先加點再計算 max HP。
                                p.stats.unspent = p.stats.unspent.saturating_add(crate::stat_points::POINTS_PER_LEVEL);
                                let full_max = crate::vitals::level_max_hp(p.level())
                                    + crate::class::hp_bonus(&p.masteries)
                                    + p.stats.hp * crate::stat_points::HP_PER_POINT;
                                p.vitals.on_level_up(full_max);
                                combat_level_up = Some((p.name.clone(), p.level()));
                            }
                            // 吸血：擊殺後回復 2 HP。
                            let ls = crate::refinement::enchant_lifesteal_hp(enchant);
                            if ls > 0 { p.vitals.heal(ls); }
                            tracing::info!(player = %p.name, ?item, qty, reward, level = p.level(), notorious, "主動攻擊戰利品+exp");
                        }
                        // 戰士熟練度（ROADMAP 38）：有擊殺才得 1 XP（每次攻擊一次，非每隻）。
                        if had_kill && p.masteries.gain_warrior(1) && p.masteries.warrior_level() == 1 {
                            let bonus = crate::class::hp_bonus(&p.masteries);
                            if bonus > 0 {
                                p.vitals.set_max_hp_full(p.vitals.max_hp() + bonus);
                            }
                        }
                    }
                    // NPC 升等賀詞（ROADMAP 84）：戰鬥升等時凱爾長老私信賀詞 / 全服廣播。
                    if let Some((pname, new_lv)) = combat_level_up {
                        let action = app.npc_level_greet.write().unwrap().on_level_up(&pname, new_lv);
                        match action {
                            crate::npc_level_greet::LevelGreetAction::WorldBroadcast { message } => {
                                let _ = app.tx_chat.send(format!(
                                    "🌟 [{}] 全服宣告：「{}」",
                                    crate::npc_level_greet::CHIEF_DISPLAY_NAME, message
                                ));
                            }
                            crate::npc_level_greet::LevelGreetAction::DirectMessage { message } => {
                                let _ = tx_direct.try_send(format!(
                                    "💬 [{}] 悄聲道：「{}」",
                                    crate::npc_level_greet::CHIEF_DISPLAY_NAME, message
                                ));
                            }
                        }
                    }
                    // 討伐兇名精英全服廣播（ROADMAP 42）；安全區遠程不觸發（防呆）。
                    if was_notorious && !suppress_rewards {
                        if let Some((kind, _, _, Some(_))) = result {
                            let pname = app.players.read().unwrap()
                                .get(&id).map(|p| p.name.clone()).unwrap_or_default();
                            if !pname.is_empty() {
                                let _ = app.tx_chat.send(format!(
                                    "⚔️ {} 討伐了兇名 {}！全服向英雄致敬！",
                                    pname, kind.display_name()
                                ));
                                // 世界事件記憶（ROADMAP 65）：兇名精英討伐是值得 NPC 提及的大事。
                                app.world_log.write().unwrap().push(format!(
                                    "勇者 {} 討伐了兇名精英 {}，全服英雄讚頌",
                                    pname, kind.display_name()
                                ));
                                // NPC 需求驅力（ROADMAP 69）：精英被討伐 → 安全感回升，獵手/里長歸屬感大升。
                                app.npc_needs.write().unwrap().apply_world_event(crate::npc_needs::NeedsEvent::EliteSlain);
                                // NPC 人際關係網（ROADMAP 70）：獵手聲望上升，各 NPC 好感提升。
                                app.npc_relations.write().unwrap().apply_world_event(crate::npc_relations::RelationsEvent::EliteSlain);
                                // NPC 主動評論（ROADMAP 68）：精英討伐，NPC 表達讚嘆。
                                {
                                    let event_kind = crate::npc_proactive::WorldEventKind::EliteSlain {
                                        name: kind.display_name().to_string(),
                                        slayer: pname.clone(),
                                    };
                                    let app2 = app.clone();
                                    tokio::spawn(async move {
                                        let now = std::time::Instant::now();
                                        let maybe_npc = {
                                            let mut cd = app2.npc_proactive.write().unwrap();
                                            crate::npc_proactive::pick_reacting_npc(&event_kind, &mut cd, now)
                                        };
                                        if let Some(npc_id) = maybe_npc {
                                            let reaction = crate::npc_proactive::generate_proactive_reaction(npc_id, event_kind).await;
                                            let _ = app2.tx_chat.send(reaction);
                                        }
                                    });
                                }
                            }
                        }
                    }
                    // NPC 自主懸賞令：兇名精英討伐 → 檢查是否符合蘭卡通緝目標（ROADMAP 82）。
                    if was_notorious && !suppress_rewards {
                        // 取第一筆兇名擊殺的種類名稱。
                        if let Some((nk, _, _, _)) = results.iter().find(|(_, _, n, _)| *n) {
                            let kind_name_str = nk.display_name();
                            if let Some(reward) = app.npc_bounty.write().unwrap()
                                .on_notorious_killed(kind_name_str, true)
                            {
                                let pname = {
                                    let mut players = app.players.write().unwrap();
                                    if let Some(p) = players.get_mut(&id) {
                                        p.ether = p.ether.saturating_add(reward);
                                        p.name.clone()
                                    } else {
                                        String::new()
                                    }
                                };
                                if !pname.is_empty() {
                                    // send() Future 沒 await=從未送出;包 ServerMsg::Chat JSON 才會被客戶端解析。
                                    let note = ServerMsg::Chat { from: "系統".into(), text: format!(
                                        "🎯 你討伐了蘭卡的通緝目標，獲得懸賞 {reward} 乙太！"
                                    ) };
                                    if let Ok(j) = serde_json::to_string(&note) { let _ = tx_direct.try_send(j); }
                                    let _ = app.tx_chat.send(format!(
                                        "🎯 [獵手蘭卡] 廣播：「通緝目標已被 {} 討伐！感謝這位勇者！」",
                                        pname
                                    ));
                                }
                            }
                        }
                    }
                    // 通知社群任務（ROADMAP 27）：擊殺事件推進進度並廣播完成公告。
                    // 安全區遠程擊殺不計數，防止城內架砲刷任務（suppress_rewards 同步守衛）。
                    if !suppress_rewards {
                        if let Some((kind, _, _, Some(_))) = result {
                            let completed = app.quests.write().unwrap().on_kill(kind);
                            notify_quest_complete(&app, completed);
                        }
                    }
                    // 成就：擊殺計數里程碑（ROADMAP 31）。
                    // 安全區遠程擊殺不計里程碑，防城內刷牆。
                    if !suppress_rewards {
                        if let Some((_, _, _, Some(_))) = result {
                            let (kill_count, new_level, pname, newly_unlocked) = {
                                let mut players = app.players.write().unwrap();
                                if let Some(p) = players.get_mut(&id) {
                                    p.kill_count = p.kill_count.saturating_add(1);
                                    let kc = p.kill_count;
                                    let lv = p.level();
                                    let pn = p.name.clone();
                                    // 擊殺里程碑成就
                                    let mut newly: Vec<crate::achievement::Achievement> = Vec::new();
                                    if let Some(ach) = crate::achievement::achievement_for_kill_count(kc) {
                                        if p.achievements.unlock(ach) { newly.push(ach); }
                                    }
                                    // 升級里程碑成就（跟隨 exp 升級一起檢查）
                                    for ach in crate::achievement::achievements_for_level(lv) {
                                        if p.achievements.unlock(ach) { newly.push(ach); }
                                    }
                                    (kc, lv, pn, newly)
                                } else {
                                    (0, 0, String::new(), Vec::new())
                                }
                            };
                            let _ = new_level; // 等級升等由 combat_level_up 廣播處理
                            // ROADMAP 147：擊殺通知——單播給玩家，讓他知道討伐了什麼、得到什麼。
                            if let Some((kill_kind, _, _, Some((item, qty)))) = result {
                                let msg = crate::protocol::ServerMsg::KillNotify {
                                    enemy_name: kill_kind.display_name().to_string(),
                                    item_display: format!(
                                        "{}×{}",
                                        crate::npc_deal::item_display_zh(item),
                                        qty
                                    ),
                                    kill_total: kill_count,
                                };
                                if let Ok(j) = serde_json::to_string(&msg) {
                                    let _ = tx_direct.try_send(j);
                                }
                            }
                            for ach in newly_unlocked {
                                let _ = app.tx_chat.send(format!(
                                    "🏆 {} 解鎖成就「{}」！", pname, ach.display_name()
                                ));
                            }
                        }
                    }
                    // 每日任務：擊殺事件（ROADMAP 32）。
                    // 安全區遠程擊殺不算每日任務進度。
                    if !suppress_rewards {
                        if let (Some(uid), Some((kill_kind, _, _, Some(_)))) = (authed_uid, result) {
                            advance_daily_kill(&app, uid, kill_kind, &tx_direct);
                        }
                    }
                    // 懸賞告示板：擊殺事件（ROADMAP 53）。
                    // 安全區遠程擊殺不結算懸賞，防止城牆龜縮刷賞。
                    if !suppress_rewards {
                        if let (Some(uid), Some((kill_kind, _, _, Some(_)))) = (authed_uid, result) {
                            let bounty_result = {
                                let mut players = app.players.write().unwrap();
                                if let Some(p) = players.get_mut(&uid) {
                                    crate::bounty_board::on_kill(&mut p.bounty_active, kill_kind)
                                } else {
                                    None
                                }
                            };
                            if let Some((reward, xp)) = bounty_result {
                                let pname = {
                                    let mut players = app.players.write().unwrap();
                                    if let Some(p) = players.get_mut(&uid) {
                                        p.ether = p.ether.saturating_add(reward);
                                        p.masteries.gain_warrior(xp);
                                        p.bounty_active = None;
                                        p.bounty_cooldown = crate::bounty_board::BOUNTY_COOLDOWN_SECS;
                                        tracing::info!(player = %p.name, reward, xp, "完成懸賞任務");
                                        p.name.clone()
                                    } else {
                                        String::new()
                                    }
                                };
                                if !pname.is_empty() {
                                    // send() Future 沒 await=從未送出;包 ServerMsg::Chat JSON 才會被客戶端解析。
                                    let note = ServerMsg::Chat { from: "系統".into(), text: format!(
                                        "🎯 懸賞完成！獲得 {} 乙太 + {} 戰士 XP！", reward, xp
                                    ) };
                                    if let Ok(j) = serde_json::to_string(&note) { let _ = tx_direct.try_send(j); }
                                    // 記入玩家事跡日誌（ROADMAP 67）：引擎事實，NPC 可自然提及。
                                    app.player_logs.write().unwrap()
                                        .entry(uid)
                                        .or_default()
                                        .push(format!("完成懸賞討伐任務，獲得 {} 乙太", reward));
                                }
                            }
                        }
                    }
                    // 旅行商人限時委託：擊殺事件（ROADMAP 136）。
                    // 安全區遠程擊殺不結算委託，防止城牆龜縮刷委託（與懸賞/社群/每日一致）。
                    if !suppress_rewards {
                        if let (Some(uid), Some((kill_kind, _, _, Some(_)))) = (authed_uid, result) {
                            let quest_result = app.wandering_merchant.write().unwrap().on_kill(kill_kind);
                            if let Some((qid, qname, ether_reward, reward_item, reward_qty)) = quest_result {
                                let pname = {
                                    let mut players = app.players.write().unwrap();
                                    if let Some(p) = players.get_mut(&uid) {
                                        p.ether = p.ether.saturating_add(ether_reward);
                                        p.add_item_overflow(reward_item, reward_qty);
                                        tracing::info!(
                                            player = %p.name, quest_id = qid, qname, ether_reward,
                                            ?reward_item, reward_qty, "完成旅行商人委託"
                                        );
                                        p.name.clone()
                                    } else { String::new() }
                                };
                                if !pname.is_empty() {
                                    let item_name = crate::npc_deal::item_display_zh(reward_item);
                                    // send() Future 沒 await=從未送出;包 ServerMsg::Chat JSON 才會被客戶端解析。
                                    let note = ServerMsg::Chat { from: "系統".into(), text: format!(
                                        "📋 委託「{}」完成！獲得 {} 乙太 + {}×{}！",
                                        qname, ether_reward, item_name, reward_qty
                                    ) };
                                    if let Ok(j) = serde_json::to_string(&note) { let _ = tx_direct.try_send(j); }
                                }
                            }
                        }
                    }
                    // 獸潮攻城（ROADMAP 44）：通知導演統計攻城點附近的擊殺數，達標即全服勝利。
                    if let Some((_, _, _, Some(_))) = result {
                        if let Some(cmd) = app.director.write().unwrap().register_kill_near_site(px, py) {
                            if let crate::director::DirectorCmd::HordeVictory { site_label, kills } = cmd {
                                let _ = app.tx_chat.send(format!(
                                    "🎉 玩家們成功打退{}的獸潮！（共斬殺 {} 隻）\
                                     全服每位登入玩家獲得 {} 乙太！",
                                    site_label, kills, crate::director::HORDE_VICTORY_ETHER
                                ));
                                for p in app.players.write().unwrap().values_mut() {
                                    p.ether = p.ether.saturating_add(crate::director::HORDE_VICTORY_ETHER);
                                }
                            }
                        }
                    }
                    // ROADMAP 163：玩家擊殺怪物 → 對應物種態度上升（怪物學會敬畏）。
                    // 事件由 game.rs tick() 統一廣播，這裡只做態度調整。
                    // 安全區遠程擊殺不計入（和其他 suppress_rewards 行為一致）。
                    if !suppress_rewards {
                        let killed_kinds: Vec<crate::combat::EnemyKind> = results.iter()
                            .filter(|(_, _, _, loot)| loot.is_some())
                            .map(|(kind, _, _, _)| *kind)
                            .collect();
                        if !killed_kinds.is_empty() {
                            // 鎖序鐵律：monster_species 寫鎖只在這個小區塊持有、用完立刻釋放，絕不與下方
                            // 會鎖 players 的巢穴/委託事件處理重疊——快照鎖序是 players→monster_species，
                            // 若這裡持 monster_species 期間再鎖 players 即反轉 → 與快照+排隊 write 成三方
                            // 死鎖環、整站永久卡死（同 AttackWildlife/EnterHome 修過的同族雷）。
                            {
                                let mut ms = app.monster_species.write().unwrap();
                                for k in &killed_kinds {
                                    ms.on_player_kills_monster(*k);
                                }
                            }
                            // ROADMAP 164：玩家擊殺怪物同時通知巢穴管理器扣族群數。
                            {
                                let colony_events = {
                                    let mut cols = app.monster_colonies.write().unwrap();
                                    let mut evts = Vec::new();
                                    for k in &killed_kinds {
                                        evts.extend(cols.on_monster_killed_near(px, py, *k));
                                    }
                                    evts
                                };
                                for ev in colony_events {
                                    use crate::monster_colony::MonsterColonyEvent;
                                    match ev {
                                        MonsterColonyEvent::ColonyCleared { name, cx, cy } => {
                                            let _ = app.tx_chat.send(format!(
                                                "🏕️ [{name}] ({cx:.0},{cy:.0}) 的怪物巢穴被清剿一空！"
                                            ));
                                        }
                                        // ROADMAP 172：通知生態清剿委託；完成事件由 eco_bounty 管理器回傳，
                                        // 但完成廣播+發獎在 game.rs tick 處理（避免 ws.rs 做玩家資料遍歷）。
                                        // 這裡只需推進 kills_so_far，完成時等下一個 tick 廣播即可。
                                        MonsterColonyEvent::MonsterKilledInColony { colony_id } => {
                                            let ev = app.eco_bounty.write().unwrap().on_colony_kill(colony_id);
                                            if let Some(crate::eco_bounty::EcoBountyEvent::Completed { colony_name, reward_per_player }) = ev {
                                                // 立即給在線玩家獎勵並廣播（ws.rs 有 players 鎖存取能力）。
                                                {
                                                    let mut players = app.players.write().unwrap();
                                                    for p in players.values_mut() {
                                                        p.ether = p.ether.saturating_add(reward_per_player);
                                                    }
                                                }
                                                let _ = app.tx_chat.send(
                                                    crate::eco_bounty::completed_text(&colony_name, reward_per_player)
                                                );
                                            }
                                        }
                                        MonsterColonyEvent::ColonyRevived { .. } => {}
                                        MonsterColonyEvent::SpawnAt { .. } => {}
                                        MonsterColonyEvent::AlphaAppeared { .. } => {}
                                        // AlphaCommandReady/ClashStart/ClashVictory 只由 game.rs 的主 tick 迴圈處理
                                        MonsterColonyEvent::AlphaCommandReady { .. } => {}
                                        MonsterColonyEvent::AlphaClashStart { .. } => {}
                                        MonsterColonyEvent::AlphaClashVictory { .. } => {}
                                        // ROADMAP 173：傳說古 Alpha 事件——只由 game.rs 主 tick 處理
                                        MonsterColonyEvent::AncientAlphaEmerged { .. } => {}
                                        MonsterColonyEvent::AncientAlphaSlain => {}
                                        // ROADMAP 174：跨族結盟——只由 game.rs 主 tick 處理
                                        MonsterColonyEvent::AllianceFormed { .. } => {}
                                        MonsterColonyEvent::AllianceBroken { .. } => {}
                                        // ROADMAP 175：Alpha 覺醒危機——只由 game.rs 主 tick 處理
                                        MonsterColonyEvent::AlphaAwakened { .. } => {}
                                        // ROADMAP 176：霸主巢穴普通怪擊殺——給擊殺者 +1 乙太
                                        MonsterColonyEvent::MonsterKilledInDominantColony => {
                                            if let Some(uid) = authed_uid {
                                                let mut players = app.players.write().unwrap();
                                                if let Some(p) = players.get_mut(&uid) {
                                                    p.ether = p.ether.saturating_add(
                                                        crate::monster_colony::DOMINANT_KILL_BONUS_ETHER
                                                    );
                                                }
                                            }
                                        }
                                        // ROADMAP 176：霸主宣告/落幕——只由 game.rs 主 tick 處理
                                        MonsterColonyEvent::DominanceDeclaration { .. } => {}
                                        MonsterColonyEvent::DominanceBroken { .. } => {}
                                        // ROADMAP 179：怪物王號令援軍——只由 game.rs 主 tick 處理
                                        MonsterColonyEvent::AlphaSummonedReinforcements { .. } => {}
                                        // ROADMAP 183：族群被打殘 → 殘兵潰逃。對巢穴範圍內同種殘部設 retreat_timer
                                        // 逃回巢穴，並廣播全服捷報。鎖序：此處未持其他鎖，只暫借 enemies 寫鎖。
                                        MonsterColonyEvent::ColonyRouted { name, kind, cx, cy, radius } => {
                                            let fled = app.enemies.write().unwrap().rout_region(
                                                cx, cy, kind, radius,
                                                crate::monster_colony::ROUT_DURATION_SECS,
                                            );
                                            if fled > 0 {
                                                let _ = app.tx_chat.send(format!(
                                                    "💨 [{name}] 的族群被打得潰不成軍，殘兵四散奔逃！"
                                                ));
                                            }
                                        }
                                        // ROADMAP 184：菁英 Alpha 背水死戰——只由 game.rs 主 tick 偵測並廣播，
                                        // 擊殺路徑不會發出此事件，這裡僅補 exhaustive 分支。
                                        MonsterColonyEvent::AlphaLastStand { .. } => {}
                                    }
                                }
                            }
                        }
                    }
                    // 入侵首領擊殺（ROADMAP 159）：乙太霸主被玩家擊倒，通知入侵狀態並廣播。
                    // 安全區遠程擊殺不觸發（防城牆龜縮刷首領）。
                    if !suppress_rewards {
                        let boss_just_killed = results.iter().any(|(kind, _, _, loot)| {
                            *kind == crate::combat::EnemyKind::EtherOverlord && loot.is_some()
                        });
                        if boss_just_killed {
                            let newly_killed = app.invasion.write().unwrap().mark_boss_killed();
                            if newly_killed {
                                let pname = app.players.read().unwrap()
                                    .get(&id).map(|p| p.name.clone()).unwrap_or_default();
                                // 依入侵等級決定晶核分配數量（ROADMAP 161）。
                                let (wave, cores_per_player, wave_level, ether_reward) = {
                                    let iv = app.invasion.read().unwrap();
                                    (iv.wave_count, iv.cores_reward(), iv.wave_level(), iv.ether_boss_reward())
                                };
                                let online_count = {
                                    let mut players = app.players.write().unwrap();
                                    let cnt = players.len();
                                    for p in players.values_mut() {
                                        p.inventory.add(
                                            crate::inventory::ItemKind::EtherOverlordCore,
                                            cores_per_player,
                                        );
                                    }
                                    cnt
                                };
                                let level_tag = if wave_level >= 2 {
                                    format!(" [Lv.{}]", wave_level)
                                } else {
                                    String::new()
                                };
                                let _ = app.tx_chat.send(format!(
                                    "💥 [首領擊倒{}！] 「{}」等英雄擊敗乙太霸主！全服 {} 位在線玩家各獲得 💠 霸主晶核×{}！(2 顆可合成守城戰刃⚔️)",
                                    level_tag, pname, online_count, cores_per_player
                                ));
                                let _ = app.tx_chat.send(format!(
                                    "🏆 第 {} 波入侵結束後將再獲 +{} 乙太特別獎勵！",
                                    wave + 1, ether_reward
                                ));
                                app.town_memory.write().unwrap().push_event(
                                    "💥",
                                    format!("入侵首領「乙太霸主」被{}等英雄擊倒（Lv.{}）——全服在線玩家各獲霸主晶核×{}", pname, wave_level, cores_per_player),
                                );
                            }
                        }
                    }
                }
                Ok(ClientMsg::ReturnHome) => {
                    // 回城：傳回新手村（出生點 / 安全區中心）。便利功能，無代價、無冷卻。
                    if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                        let (sx, sy) = crate::positions::default_spawn();
                        p.x = sx;
                        p.y = sy;
                        tracing::info!(player = %p.name, "回城（傳回新手村）");
                    }
                }
                Ok(ClientMsg::UseItem { item }) => {
                    // 使用道具：消耗一個指定道具，觸發對應效果。倒地 / 背包不足靜默忽略。
                    use crate::inventory::ItemKind;
                    if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                        match item {
                            ItemKind::HealingPotion => {
                                // 活力藥水：回復 6 HP。
                                if !p.vitals.is_downed() && p.inventory.take(item, 1) {
                                    let gained = p.vitals.heal(6);
                                    tracing::info!(player = %p.name, ?item, gained, "使用道具回血");
                                }
                            }
                            ItemKind::CrystalPotion => {
                                // 晶石強化液：回復 12 HP（Premium 晶洞探索回報）。
                                if !p.vitals.is_downed() && p.inventory.take(item, 1) {
                                    let gained = p.vitals.heal(12);
                                    tracing::info!(player = %p.name, ?item, gained, "使用道具回血");
                                }
                            }
                            ItemKind::MushroomElixir => {
                                // 蕈菇活化液：回復 8 HP 並重置回血冷卻，讓回血立刻開始。
                                if !p.vitals.is_downed() && p.inventory.take(item, 1) {
                                    let gained = p.vitals.heal(8);
                                    p.vitals.reset_regen_cooldown();
                                    tracing::info!(player = %p.name, ?item, gained, "使用道具回血+重置回血冷卻");
                                }
                            }
                            ItemKind::EtherPill => {
                                // 古代乙太丸：直接獲得 10 乙太（沙漠探索野外兌換遺跡能量）。
                                if !p.vitals.is_downed() && p.inventory.take(item, 1) {
                                    p.add_item_overflow(ItemKind::Ether, 10);
                                    tracing::info!(player = %p.name, ?item, "使用道具獲得乙太");
                                }
                            }
                            ItemKind::PearlPotion => {
                                // 珍珠復原藥：回復至等級對應的滿血（最稀有材料換最強效果）。
                                if !p.vitals.is_downed() && p.inventory.take(item, 1) {
                                    let gained = p.vitals.heal(p.vitals.max_hp());
                                    tracing::info!(player = %p.name, ?item, gained, "使用道具滿血復原");
                                }
                            }
                            ItemKind::JadeElixir => {
                                // 翠幽精露：回復至滿血並重置回血冷卻——翠幽星頂級精華，雙效加成。
                                if !p.vitals.is_downed() && p.inventory.take(item, 1) {
                                    let gained = p.vitals.heal(p.vitals.max_hp());
                                    p.vitals.reset_regen_cooldown();
                                    tracing::info!(player = %p.name, ?item, gained, "使用翠幽精露滿血+重置回血");
                                }
                            }
                            ItemKind::SteamElixir => {
                                // 蒸汽精粹：回復至滿血 + 獲得 8 乙太——赤焰星蒸汽燃料轉換器，雙效加成。
                                if !p.vitals.is_downed() && p.inventory.take(item, 1) {
                                    let gained = p.vitals.heal(p.vitals.max_hp());
                                    p.ether = p.ether.saturating_add(8);
                                    tracing::info!(player = %p.name, ?item, gained, "使用蒸汽精粹滿血+獲得8乙太");
                                }
                            }
                            ItemKind::VoidElixir => {
                                // 虛空精粹：回復至滿血 + 獲得 10 乙太——虛空星宇宙深淵能量轉換，比蒸汽精粹更強。
                                if !p.vitals.is_downed() && p.inventory.take(item, 1) {
                                    let gained = p.vitals.heal(p.vitals.max_hp());
                                    p.ether = p.ether.saturating_add(10);
                                    tracing::info!(player = %p.name, ?item, gained, "使用虛空精粹滿血+獲得10乙太");
                                }
                            }
                            ItemKind::AetherEssence => {
                                // 霧醚精粹：回復至滿血 + 獲得 15 乙太——霧醚星乙太迷霧高密度能量轉換，四星最強補給。
                                if !p.vitals.is_downed() && p.inventory.take(item, 1) {
                                    let gained = p.vitals.heal(p.vitals.max_hp());
                                    p.ether = p.ether.saturating_add(15);
                                    tracing::info!(player = %p.name, ?item, gained, "使用霧醚精粹滿血+獲得15乙太");
                                }
                            }
                            ItemKind::OriginEssence => {
                                // 源晶精粹：回復至滿血 + 獲得 20 乙太——星源星宇宙源頭能量轉換，五星最強補給。
                                if !p.vitals.is_downed() && p.inventory.take(item, 1) {
                                    let gained = p.vitals.heal(p.vitals.max_hp());
                                    p.ether = p.ether.saturating_add(20);
                                    tracing::info!(player = %p.name, ?item, gained, "使用源晶精粹滿血+獲得20乙太");
                                }
                            }
                            ItemKind::StarChart => {
                                // 星圖：展開遠方星球快照——道具本身不消耗（是導航工具而非消耗品）。
                                // 前端收到背包快照後本地彈出星圖彈窗；伺服器只記日誌。
                                if !p.vitals.is_downed() && p.inventory.count(item) > 0 {
                                    tracing::info!(player = %p.name, "展開星圖");
                                }
                            }
                            // ── 料理（ROADMAP 47 釣魚與烹飪）────────────────────────
                            ItemKind::GrilledFish => {
                                // 烤魚：回復 8 HP（小魚×2 烹飪而成，基礎療癒食物）。
                                if !p.vitals.is_downed() && p.inventory.take(item, 1) {
                                    let gained = p.vitals.heal(8);
                                    tracing::info!(player = %p.name, gained, "食用烤魚回血");
                                }
                            }
                            ItemKind::StarSashimi => {
                                // 星燦刺身：回復 15 HP（星星魚烹飪，稀有漁獲料理）。
                                if !p.vitals.is_downed() && p.inventory.take(item, 1) {
                                    let gained = p.vitals.heal(15);
                                    tracing::info!(player = %p.name, gained, "食用星燦刺身回血");
                                }
                            }
                            ItemKind::DeepBroth => {
                                // 深海濃湯：回復至等級滿血（最稀有漁獲換最強效果）。
                                if !p.vitals.is_downed() && p.inventory.take(item, 1) {
                                    let gained = p.vitals.heal(p.vitals.max_hp());
                                    tracing::info!(player = %p.name, gained, "飲用深海濃湯滿血復原");
                                }
                            }
                            // ── 牧場料理（ROADMAP 48）────────────────────────────────
                            ItemKind::FriedEgg => {
                                // 煎蛋：回復 10 HP（雞蛋×2 烹飪，農田地塊自產療癒食物）。
                                if !p.vitals.is_downed() && p.inventory.take(item, 1) {
                                    let gained = p.vitals.heal(10);
                                    tracing::info!(player = %p.name, gained, "食用煎蛋回血");
                                }
                            }
                            // ── 農地料理（ROADMAP 49）────────────────────────────
                            ItemKind::Bread => {
                                // 麵包：回復 12 HP（小麥×3 烹飪）。
                                if !p.vitals.is_downed() && p.inventory.take(item, 1) {
                                    let gained = p.vitals.heal(12);
                                    tracing::info!(player = %p.name, gained, "食用麵包回血");
                                }
                            }
                            ItemKind::CarrotSoup => {
                                // 蔬菜湯：回復 10 HP 並重置自然回血冷卻（胡蘿蔔×2 烹飪）。
                                if !p.vitals.is_downed() && p.inventory.take(item, 1) {
                                    let gained = p.vitals.heal(10);
                                    p.vitals.reset_regen_cooldown();
                                    tracing::info!(player = %p.name, gained, "食用蔬菜湯回血+重置回血冷卻");
                                }
                            }
                            ItemKind::PotatoGratin => {
                                // 焗烤馬鈴薯：回復 15 HP（馬鈴薯×2 烹飪，農地料理最豐盛）。
                                if !p.vitals.is_downed() && p.inventory.take(item, 1) {
                                    let gained = p.vitals.heal(15);
                                    tracing::info!(player = %p.name, gained, "食用焗烤馬鈴薯回血");
                                }
                            }
                            ItemKind::NightPotion => {
                                // 夜幻藥水：回復 20 HP（星晶碎片×3 合成；夜採路線最強效補給）。
                                if !p.vitals.is_downed() && p.inventory.take(item, 1) {
                                    let gained = p.vitals.heal(20);
                                    tracing::info!(player = %p.name, gained, "飲用夜幻藥水回血");
                                }
                            }
                            // ── 城鎮慶典配方（ROADMAP 130）───────────────────────────
                            ItemKind::TownBrew => {
                                // 城鎮特釀：回復 22 HP + 農夫熟練度 +10 XP。需城鎮達到【生機】才可合成。
                                if !p.vitals.is_downed() && p.inventory.take(item, 1) {
                                    let gained = p.vitals.heal(22);
                                    p.masteries.gain_farmer(10);
                                    tracing::info!(player = %p.name, gained, "飲用城鎮特釀回血+農夫熟練度");
                                }
                            }
                            ItemKind::VibrantElixir => {
                                // 繁盛精露：回復至等級滿血 + 獲得 20 乙太。需城鎮達到【繁盛】才可合成。
                                if !p.vitals.is_downed() && p.inventory.take(item, 1) {
                                    let gained = p.vitals.heal(p.vitals.max_hp());
                                    p.ether = p.ether.saturating_add(20);
                                    tracing::info!(player = %p.name, gained, "飲用繁盛精露滿血+獲得20乙太");
                                }
                            }
                            // ── 季節性限定合成品（ROADMAP 154）──────────────────────
                            ItemKind::SpringSachet => {
                                // 春日香囊：回血 25hp + 重置回血冷卻。
                                if !p.vitals.is_downed() && p.inventory.take(item, 1) {
                                    let gained = p.vitals.heal(25);
                                    p.vitals.reset_regen_cooldown();
                                    tracing::info!(player = %p.name, gained, "使用春日香囊回血+重置回血冷卻");
                                }
                            }
                            ItemKind::SummerElixir => {
                                // 夏日精粹：回血 15hp + 獲得 15 乙太。
                                if !p.vitals.is_downed() && p.inventory.take(item, 1) {
                                    let gained = p.vitals.heal(15);
                                    p.ether = p.ether.saturating_add(15);
                                    tracing::info!(player = %p.name, gained, "使用夏日精粹回血+獲得15乙太");
                                }
                            }
                            ItemKind::AutumnTonic => {
                                // 秋日補藥：回血 20hp + 農夫熟練度 +20 XP。
                                if !p.vitals.is_downed() && p.inventory.take(item, 1) {
                                    let gained = p.vitals.heal(20);
                                    p.masteries.gain_farmer(20);
                                    tracing::info!(player = %p.name, gained, "使用秋日補藥回血+農夫熟練度");
                                }
                            }
                            ItemKind::WinterMedicine => {
                                // 冬日神藥：回復至等級滿血——凜冬採集最難、效果最強。
                                if !p.vitals.is_downed() && p.inventory.take(item, 1) {
                                    let gained = p.vitals.heal(p.vitals.max_hp());
                                    tracing::info!(player = %p.name, gained, "使用冬日神藥滿血復原");
                                }
                            }
                            // ── 巢穴 Alpha 戰利品合成品（ROADMAP 168）──────────────
                            ItemKind::AlphaForce => {
                                // Alpha 之力：回滿血 + 獲得 +25 乙太——Alpha 原始生命力傾注。
                                if !p.vitals.is_downed() && p.inventory.take(item, 1) {
                                    let gained = p.vitals.heal(p.vitals.max_hp());
                                    p.ether = p.ether.saturating_add(25);
                                    tracing::info!(player = %p.name, gained, "使用 Alpha 之力滿血+獲得25乙太");
                                }
                            }
                            _ => {} // 非消耗品，忽略
                        }
                    }
                }
                Ok(ClientMsg::EquipItem { item }) => {
                    // 裝備道具（ROADMAP 36）：把背包裡的武器/護甲裝進對應槽。
                    // 背包無此物品 / 不可裝備 → 靜默忽略。換裝時舊裝備退回背包。
                    let mut old_item: Option<crate::inventory::ItemKind> = None;
                    if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                        if crate::equipment::slot_for_item(item).is_some()
                            && p.inventory.count(item) > 0
                        {
                            old_item = crate::equipment::equip(&mut p.equipment, item);
                            // 從背包扣除剛裝上的道具，維持「slot 裡的 ≠ 背包裡的」不變式
                            p.inventory.take(item, 1);
                            if let Some(old) = old_item {
                                // 換裝：舊裝備退回背包（允許溢出至倉庫，不丟失裝備）
                                p.add_item_overflow(old, 1);
                            }
                            tracing::info!(player = %p.name, ?item, "裝備道具");
                        }
                    }
                    let _ = old_item;
                    if let Some(uid) = authed_uid {
                        if let Some(p) = app.players.read().unwrap().get(&id) {
                            app.inventories.remember_equipment(uid, &p.equipment);
                        }
                    }
                }
                Ok(ClientMsg::UnequipItem { slot }) => {
                    // 卸下裝備（ROADMAP 36）：把指定槽的裝備退回背包。
                    if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                        if let Some(removed) = crate::equipment::unequip(&mut p.equipment, &slot) {
                            p.add_item_overflow(removed, 1);
                            tracing::info!(player = %p.name, ?removed, slot = %slot, "卸下裝備");
                        }
                    }
                    if let Some(uid) = authed_uid {
                        if let Some(p) = app.players.read().unwrap().get(&id) {
                            app.inventories.remember_equipment(uid, &p.equipment);
                        }
                    }
                }
                Ok(ClientMsg::RefineEquip { slot }) => {
                    // 精煉裝備（ROADMAP 37）：消耗同系材料，提升裝備精煉等級。
                    // +4 起有失敗率：失敗降一級（材料仍消耗、不碎裝）。
                    let slot_str = slot.as_str();
                    if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                        // 取得槽內裝備（weapon 或 armor）和其元資料。
                        let (item_opt, meta) = match slot_str {
                            "weapon" => (p.equipment.weapon, &mut p.equipment.weapon_meta),
                            "armor" => (p.equipment.armor, &mut p.equipment.armor_meta),
                            _ => (None, &mut p.equipment.weapon_meta), // 無效槽，直接忽略
                        };
                        if slot_str != "weapon" && slot_str != "armor" {
                            // pass
                        } else if let Some(item) = item_opt {
                            use crate::refinement::{refine_material, refine_cost_qty, refine_fails, MAX_REFINE};
                            if meta.refine >= MAX_REFINE {
                                // 已滿級，靜默忽略。
                            } else if let Some(mat) = refine_material(item) {
                                let cost = refine_cost_qty(meta.refine);
                                if p.inventory.has(mat, cost) {
                                    p.inventory.take(mat, cost);
                                    // 用 refine_attempt_count（每次嘗試遞增）確保連續精煉得到不同偽隨機結果。
                                    let attempt = p.refine_attempt_count;
                                    p.refine_attempt_count = p.refine_attempt_count.wrapping_add(1);
                                    if refine_fails(meta.refine, attempt) {
                                        meta.refine = meta.refine.saturating_sub(1);
                                        tracing::info!(player = %p.name, ?item, slot, refine = meta.refine, "精煉失敗");
                                    } else {
                                        meta.refine += 1;
                                        tracing::info!(player = %p.name, ?item, slot, refine = meta.refine, "精煉成功");
                                    }
                                }
                            }
                        }
                    }
                    if let Some(uid) = authed_uid {
                        if let Some(p) = app.players.read().unwrap().get(&id) {
                            app.inventories.remember_equipment(uid, &p.equipment);
                        }
                    }
                }
                Ok(ClientMsg::EnchantEquip { shard }) => {
                    // 附魔（ROADMAP 37）：消耗 1 個星球碎片，賦予武器槽特效。
                    if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                        if p.equipment.weapon.is_some() {
                            use crate::refinement::enchant_from_shard;
                            if let Some(enchant) = enchant_from_shard(shard) {
                                if p.inventory.has(shard, 1) {
                                    p.inventory.take(shard, 1);
                                    p.equipment.weapon_meta.enchant = Some(enchant);
                                    tracing::info!(
                                        player = %p.name, ?shard,
                                        enchant = enchant.display_name(), "武器附魔"
                                    );
                                }
                            }
                        }
                    }
                    if let Some(uid) = authed_uid {
                        if let Some(p) = app.players.read().unwrap().get(&id) {
                            app.inventories.remember_equipment(uid, &p.equipment);
                        }
                    }
                }
                Ok(ClientMsg::TravelToPlanet { planet }) => {
                    // 星際旅行（ROADMAP 20/22/24/25）：傳送玩家到指定星球。
                    use crate::state::{
                        PLANET_HOME, PLANET_VERDANT, PLANET_CRIMSON, PLANET_VOID, PLANET_AETHER, PLANET_ORIGIN,
                        VERDANT_SPAWN_X, VERDANT_SPAWN_Y,
                        CRIMSON_SPAWN_X, CRIMSON_SPAWN_Y,
                        VOID_SPAWN_X, VOID_SPAWN_Y,
                        AETHER_SPAWN_X, AETHER_SPAWN_Y,
                        ORIGIN_SPAWN_X, ORIGIN_SPAWN_Y,
                        TRAVEL_ETHER_COST, TRAVEL_ETHER_COST_CRIMSON, TRAVEL_ETHER_COST_VOID,
                        TRAVEL_ETHER_COST_AETHER, TRAVEL_ETHER_COST_ORIGIN,
                        TRAVEL_ETHER_COST_VERDANT_DIRECT,
                    };
                    use crate::protocol::ServerMsg;
                    // 星象預報星際順風（ROADMAP 132）：旅行費額外 -10 乙太。
                    let forecast_travel_discount: u32 = {
                        let obs = app.observatory.read().unwrap();
                        if obs.is_active() && obs.current_bonus == crate::observatory::StarForecastBonus::TravelDiscount {
                            crate::observatory::StarForecastBonus::travel_discount_ether()
                        } else { 0 }
                    };
                    let result = if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                        let travel_discount = crate::class::travel_cost_reduction(&p.masteries);
                        match p.can_travel_to(&planet, travel_discount) {
                            Err(msg) => Some(ServerMsg::TravelResult {
                                ok: false,
                                planet: p.planet.clone(),
                                message: msg,
                            }),
                            Ok(()) if planet == PLANET_VERDANT => {
                                // 依武裝狀態決定費用（ROADMAP 39 雙路徑）。
                                use crate::inventory::ItemKind;
                                let biome_weapons = [
                                    ItemKind::MeadowAmulet, ItemKind::MushroomStaff,
                                    ItemKind::CrystalBlade, ItemKind::RuneBlade, ItemKind::CoralLance,
                                ];
                                let has_all_weapons = biome_weapons.iter().all(|w| p.inventory.count(*w) > 0);
                                let base_cost = if has_all_weapons { TRAVEL_ETHER_COST } else { TRAVEL_ETHER_COST_VERDANT_DIRECT };
                                let cost = crate::class::apply_travel_discount(&p.masteries, base_cost).saturating_sub(forecast_travel_discount);
                                p.ether -= cost;
                                p.planet = PLANET_VERDANT.to_string();
                                p.x = VERDANT_SPAWN_X;
                                p.y = VERDANT_SPAWN_Y;
                                p.masteries.gain_explorer(10); // 探索者熟練度（ROADMAP 38）
                                tracing::info!(player = %p.name, cost, has_all_weapons, "星際旅行：抵達翠幽星");
                                Some(ServerMsg::TravelResult {
                                    ok: true,
                                    planet: PLANET_VERDANT.to_string(),
                                    message: "歡迎來到翠幽星！茂密叢林的古老氣息撲面而來⋯⋯".to_string(),
                                })
                            }
                            Ok(()) if planet == PLANET_CRIMSON => {
                                let cost = crate::class::apply_travel_discount(&p.masteries, TRAVEL_ETHER_COST_CRIMSON).saturating_sub(forecast_travel_discount);
                                p.ether -= cost;
                                p.planet = PLANET_CRIMSON.to_string();
                                p.x = CRIMSON_SPAWN_X;
                                p.y = CRIMSON_SPAWN_Y;
                                p.masteries.gain_explorer(10);
                                tracing::info!(player = %p.name, cost, "星際旅行：抵達赤焰星");
                                Some(ServerMsg::TravelResult {
                                    ok: true,
                                    planet: PLANET_CRIMSON.to_string(),
                                    message: "歡迎來到赤焰星！熔岩與蒸汽的氣息撲面——古代機械的低鳴迴盪遠方⋯⋯".to_string(),
                                })
                            }
                            Ok(()) if planet == PLANET_VOID => {
                                let cost = crate::class::apply_travel_discount(&p.masteries, TRAVEL_ETHER_COST_VOID).saturating_sub(forecast_travel_discount);
                                p.ether -= cost;
                                p.planet = PLANET_VOID.to_string();
                                p.x = VOID_SPAWN_X;
                                p.y = VOID_SPAWN_Y;
                                p.masteries.gain_explorer(10);
                                tracing::info!(player = %p.name, cost, "星際旅行：抵達虛空星");
                                Some(ServerMsg::TravelResult {
                                    ok: true,
                                    planet: PLANET_VOID.to_string(),
                                    message: "歡迎來到虛空星⋯⋯宇宙深淵的黑暗靜默將你環繞，虛空晶體在暗中低語。".to_string(),
                                })
                            }
                            Ok(()) if planet == PLANET_AETHER => {
                                let cost = crate::class::apply_travel_discount(&p.masteries, TRAVEL_ETHER_COST_AETHER).saturating_sub(forecast_travel_discount);
                                p.ether -= cost;
                                p.planet = PLANET_AETHER.to_string();
                                p.x = AETHER_SPAWN_X;
                                p.y = AETHER_SPAWN_Y;
                                p.masteries.gain_explorer(10);
                                tracing::info!(player = %p.name, cost, "星際旅行：抵達霧醚星");
                                Some(ServerMsg::TravelResult {
                                    ok: true,
                                    planet: PLANET_AETHER.to_string(),
                                    message: "歡迎來到霧醚星⋯⋯乙太迷霧輕柔地將你環繞，霧醚晶霧在薄霧中閃爍著青白色的光芒。".to_string(),
                                })
                            }
                            Ok(()) if planet == PLANET_ORIGIN => {
                                let cost = crate::class::apply_travel_discount(&p.masteries, TRAVEL_ETHER_COST_ORIGIN).saturating_sub(forecast_travel_discount);
                                p.ether -= cost;
                                p.planet = PLANET_ORIGIN.to_string();
                                p.x = ORIGIN_SPAWN_X;
                                p.y = ORIGIN_SPAWN_Y;
                                p.masteries.gain_explorer(10);
                                tracing::info!(player = %p.name, cost, "星際旅行：抵達星源星");
                                Some(ServerMsg::TravelResult {
                                    ok: true,
                                    planet: PLANET_ORIGIN.to_string(),
                                    message: "歡迎來到星源星⋯⋯乙太文明的源頭在此沉默等候，源晶的金白光芒照亮了宇宙的起源之地。".to_string(),
                                })
                            }
                            Ok(()) => {
                                let cost = crate::class::apply_travel_discount(&p.masteries, TRAVEL_ETHER_COST).saturating_sub(forecast_travel_discount);
                                p.ether -= cost;
                                p.planet = PLANET_HOME.to_string();
                                let (hx, hy) = crate::positions::default_spawn();
                                p.x = hx;
                                p.y = hy;
                                tracing::info!(player = %p.name, cost, "星際旅行：返回故鄉");
                                Some(ServerMsg::TravelResult {
                                    ok: true,
                                    planet: PLANET_HOME.to_string(),
                                    message: "安全返回故鄉星球！新手村的燈塔在遠方閃爍⋯⋯".to_string(),
                                })
                            }
                        }
                    } else {
                        None
                    };
                    if let Some(msg) = result {
                        // 通知社群任務（ROADMAP 27）：成功旅行到非故鄉星球時推進任務進度。
                        if let crate::protocol::ServerMsg::TravelResult { ok: true, planet: ref p, .. } = msg {
                            if p != "home" {
                                let completed = app.quests.write().unwrap().on_travel(p);
                                notify_quest_complete(&app, completed);
                                // 成就：首次踏上該星球（ROADMAP 31）。
                                if let Some(uid) = authed_uid {
                                    if let Some(ach) = crate::achievement::achievement_for_planet(p) {
                                        let is_new = app.players.write().unwrap()
                                            .get_mut(&uid)
                                            .map(|pl| pl.achievements.unlock(ach))
                                            .unwrap_or(false);
                                        if is_new {
                                            let pname = app.players.read().unwrap()
                                                .get(&uid).map(|pl| pl.name.clone()).unwrap_or_default();
                                            let _ = app.tx_chat.send(format!(
                                                "🏆 {} 解鎖成就「{}」！",
                                                pname, ach.display_name()
                                            ));
                                        }
                                    }
                                    // 每日任務：旅行事件（ROADMAP 32）。
                                    advance_daily_travel(&app, uid, p, &tx_direct);
                                }
                            }
                        }
                        let _ = tx_direct.send(
                            serde_json::to_string(&msg).unwrap_or_default(),
                        ).await;
                    }
                }
                // ROADMAP 38：職業改兼修熟練度，SetClass 已不再使用；舊客戶端訊息靜默忽略。
                Ok(ClientMsg::SetClass { .. }) => {}

                // ── 主動技能（ROADMAP 45）─────────────────────────────────────────
                Ok(ClientMsg::UseSkill { kind }) => {
                    use crate::active_skill::{ActiveSkillKind, GALE_DASH_PX};
                    use crate::state::{WORLD_WIDTH, WORLD_HEIGHT};

                    let Some(skill_kind) = ActiveSkillKind::from_str(&kind) else { continue; };

                    // 讀取玩家狀態（未登入 / 倒地 / 冷卻中 / 熟練度不足均靜默忽略）。
                    let info = app.players.read().unwrap().get(&id).map(|p| {
                        let cd = p.skill_cooldowns.get(skill_kind);
                        let unlocked = skill_kind.is_unlocked(&p.masteries);
                        let downed = p.vitals.is_downed();
                        (cd, unlocked, downed, p.x, p.y, p.input.up, p.input.down, p.input.left, p.input.right)
                    });
                    let Some((cd, unlocked, downed, px, py, inp_up, inp_down, inp_left, inp_right)) = info else { continue; };
                    if downed || !unlocked || cd > 0.0 { continue; }

                    // 風之步：立即瞬移（不設 pending 旗）。
                    if skill_kind == ActiveSkillKind::Gale {
                        let mut dx = 0.0_f32;
                        let mut dy = 0.0_f32;
                        if inp_up    { dy -= 1.0; }
                        if inp_down  { dy += 1.0; }
                        if inp_left  { dx -= 1.0; }
                        if inp_right { dx += 1.0; }
                        // 若無輸入方向，預設向上（不讓技能無效）。
                        if dx == 0.0 && dy == 0.0 { dy = -1.0; }
                        let len = (dx * dx + dy * dy).sqrt();
                        dx /= len;
                        dy /= len;
                        if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                            // 風之步：熟練度加成瞬移距離（ROADMAP 153）。
                            let bonus_px = p.skill_masteries.gale_bonus_dash_px();
                            let total_dash = GALE_DASH_PX + bonus_px;
                            p.x = (p.x + dx * total_dash).clamp(0.0, WORLD_WIDTH);
                            p.y = (p.y + dy * total_dash).clamp(0.0, WORLD_HEIGHT);
                            // 熟練度加成縮短冷卻（ROADMAP 153）。
                            let cd = p.skill_masteries.effective_cooldown(skill_kind, skill_kind.cooldown_secs());
                            p.skill_cooldowns.set(skill_kind, cd);
                            p.skill_masteries.increment(skill_kind);
                            tracing::info!(player = %p.name, dx, dy, bonus_px, "風之步瞬移");
                        }
                    } else {
                        // 其餘技能：設 pending 旗 + 進冷卻 + 計數（ROADMAP 153）。
                        if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                            match skill_kind {
                                ActiveSkillKind::Warcry    => p.pending_warcry    = true,
                                ActiveSkillKind::Bounty    => p.pending_bounty    = true,
                                ActiveSkillKind::Precision => p.pending_precision = true,
                                ActiveSkillKind::Haggle    => p.pending_haggle    = true,
                                ActiveSkillKind::Gale      => unreachable!(),
                            }
                            // 熟練度加成縮短冷卻（ROADMAP 153）。
                            let cd = p.skill_masteries.effective_cooldown(skill_kind, skill_kind.cooldown_secs());
                            p.skill_cooldowns.set(skill_kind, cd);
                            p.skill_masteries.increment(skill_kind);
                            tracing::info!(player = %p.name, ?skill_kind, "主動技能準備就緒");
                        }
                    }

                    // 廣播技能動畫（SkillActivated）給所有連線客戶端。
                    let _ = app.tx.send(Arc::new(ServerMsg::SkillActivated {
                        player_id: id,
                        kind: kind.clone(),
                    }));
                }

                // ── 技能自動施放設定（ROADMAP 151）────────────────────────────────
                Ok(ClientMsg::SetAutoSkill { kind, enabled }) => {
                    use crate::active_skill::ActiveSkillKind;
                    // 風之步（gale）不支援自動施放（需要方向輸入）。
                    let valid = ActiveSkillKind::from_str(&kind)
                        .map(|k| k != ActiveSkillKind::Gale)
                        .unwrap_or(false);
                    if !valid { continue; }
                    if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                        if enabled {
                            p.auto_skills.insert(kind.clone());
                        } else {
                            p.auto_skills.remove(&kind);
                        }
                        tracing::debug!(player = %p.name, kind = %kind, enabled, "技能自動施放設定");
                    }
                }

                // ── 屬性加點分配（ROADMAP 152）────────────────────────────────────
                Ok(ClientMsg::AllocateStat { stat, points }) => {
                    if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                        if let Ok(()) = p.stats.allocate(&stat, points) {
                            // HP 加點立即更新最大血量（不補滿，只調整上限）。
                            if stat == crate::stat_points::STAT_HP {
                                let new_max = crate::vitals::level_max_hp(p.level())
                                    + crate::class::hp_bonus(&p.masteries)
                                    + p.stats.hp * crate::stat_points::HP_PER_POINT;
                                p.vitals.update_max_hp(new_max);
                            }
                            tracing::debug!(player = %p.name, stat = %stat, points, unspent = p.stats.unspent, "屬性加點分配");
                        }
                    }
                }

                // ── 寵物系統（ROADMAP 46）──────────────────────────────────────────
                Ok(ClientMsg::TamePet) => {
                    // 倒地時無法馴化。
                    let downed = app.players.read().unwrap()
                        .get(&id).map(|p| p.vitals.is_downed()).unwrap_or(true);
                    if downed { continue; }

                    // 讀玩家位置與現有乙太值（先讀出來，才能在 predicate 裡判斷是否足夠）。
                    let player_info = app.players.read().unwrap()
                        .get(&id).map(|p| (p.x, p.y, p.ether));
                    let Some((px, py, cur_ether)) = player_info else { continue; };

                    // 嘗試馴化：只有「可馴化種類 + 乙太足夠」才移除敵人，避免不符條件時敵人無聲消失。
                    let tamed = {
                        let mut enemies = app.enemies.write().unwrap();
                        enemies.try_tame_nearest(px, py, crate::enemy_field::ATTACK_REACH, |kind| {
                            let Some(pk) = crate::pet::pet_from_enemy_kind(kind) else { return false; };
                            cur_ether >= pk.tame_cost()
                        })
                        .and_then(|enemy_kind| {
                            crate::pet::pet_from_enemy_kind(enemy_kind).map(|pk| (enemy_kind, pk))
                        })
                    };
                    let Some((_enemy_kind, pet_kind)) = tamed else { continue; };

                    // 扣乙太 + 設寵物（再次確認乙太以防極罕見的 race，雖然同 session 不太可能）。
                    if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                        let cost = pet_kind.tame_cost();
                        if p.ether < cost {
                            tracing::debug!(player = %p.name, ?pet_kind, cost, "馴化失敗：乙太不足（race）");
                            continue;
                        }
                        p.ether -= cost;
                        let old_pet = p.pet.replace(pet_kind);
                        // ROADMAP 343：新寵物瞬間出現在主人腳邊（不從世界原點慢慢走過來）。
                        p.pet_x = p.x;
                        p.pet_y = p.y;
                        tracing::info!(
                            player = %p.name,
                            new_pet = pet_kind.display_name(),
                            old_pet = old_pet.map(|k| k.display_name()).unwrap_or("無"),
                            "馴化寵物成功"
                        );
                        // 廣播聊天，讓其他玩家知道有新寵物加入。
                        let _ = app.tx_chat.send(format!(
                            "🐾 {} 馴化了 {} {} 成為寵物！",
                            p.name, pet_kind.emoji(), pet_kind.display_name()
                        ));
                    }
                }

                Ok(ClientMsg::ReleasePet) => {
                    if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                        if let Some(old_pet) = p.pet.take() {
                            tracing::info!(player = %p.name, pet = old_pet.display_name(), "放生寵物");
                        }
                    }
                }

                // ── 拋竿（ROADMAP 47 釣魚 / ROADMAP 346 上鉤小遊戲）──────────────────
                Ok(ClientMsg::Fish) => {
                    // 拋竿：驗未倒地、冷卻到期、站水邊、目前沒在釣 → 開一趟「等咬鉤」。
                    // 不再立即得魚；魚會在 1.5~4.5 秒後咬鉤（game.rs 每 tick 推進），
                    // 玩家須在咬鉤反應窗口內送 Reel 收竿。同一把 players 寫鎖、純記憶體。
                    use crate::fishing::{is_near_water, FISH_COOLDOWN_SECS};
                    use crate::fishing_bite::FishingCast;
                    if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                        if !p.vitals.is_downed()
                            && p.fish_cooldown <= 0.0
                            && p.fishing.is_none()
                            && is_near_water(p.x, p.y)
                        {
                            // 種子：player id 低 64 位 XOR fish_attempt_count（每趟咬鉤時機不同）。
                            let seed = {
                                let id_bytes = p.id.as_u128();
                                ((id_bytes & 0xFFFF_FFFF_FFFF_FFFF) as u64) ^ p.fish_attempt_count
                            };
                            p.fish_attempt_count = p.fish_attempt_count.wrapping_add(1);
                            // 拋竿即起冷卻（防連拋刷竿）；收竿成敗都不重置冷卻。
                            p.fish_cooldown = FISH_COOLDOWN_SECS;
                            p.fishing = Some(FishingCast::cast(seed));
                            tracing::debug!(player = %p.name, "拋竿");
                        }
                    }
                }

                // ── 收竿（ROADMAP 346 釣魚上鉤小遊戲）────────────────────────────────
                Ok(ClientMsg::Reel) => {
                    // 在魚咬鉤的反應窗口內收竿＝釣到魚（反應越快魚越好）；
                    // 魚還沒咬就收會嚇跑魚、空手而回。全程同一把 players 寫鎖、純記憶體，
                    // 廣播在出鎖後才送（守 prod-deadlock 鐵律：鎖內不送廣播）。
                    use crate::fishing::FISH_FARMER_XP;
                    use crate::fishing_bite::{roll_fish_quality, ReelOutcome};
                    // 1. 鎖內判定結果、給魚、清狀態；把要廣播的資料帶出鎖外。
                    let outcome_msg = {
                        let mut players = app.players.write().unwrap();
                        if let Some(p) = players.get_mut(&id) {
                            match p.fishing.take() {
                                // 沒在釣：靜默忽略，不廣播。
                                None => None,
                                Some(cast) => match cast.reel() {
                                    ReelOutcome::Caught(quality) => {
                                        // 反應品質決定魚種加權；種子沿用 attempt_count 推進。
                                        let seed = {
                                            let id_bytes = p.id.as_u128();
                                            ((id_bytes & 0xFFFF_FFFF_FFFF_FFFF) as u64)
                                                ^ p.fish_attempt_count
                                        };
                                        p.fish_attempt_count =
                                            p.fish_attempt_count.wrapping_add(1);
                                        let fish = roll_fish_quality(seed, quality);
                                        p.add_item_overflow(fish, 1);
                                        p.masteries.gain_farmer(FISH_FARMER_XP);
                                        tracing::info!(
                                            player = %p.name, fish = ?fish, quality = ?quality,
                                            "收竿釣到魚"
                                        );
                                        // 魚物品 → snake_case 線格式（serde 約定，鏡像 state.rs decay key）。
                                        let fish_key = serde_json::to_value(fish)
                                            .ok()
                                            .and_then(|v| v.as_str().map(|s| s.to_string()))
                                            .unwrap_or_default();
                                        Some(ServerMsg::FishResult {
                                            player_id: id,
                                            outcome: "caught".into(),
                                            fish: Some(fish_key),
                                            quality: Some(quality.as_str().to_string()),
                                            x: p.x,
                                            y: p.y,
                                        })
                                    }
                                    ReelOutcome::TooEarly => Some(ServerMsg::FishResult {
                                        player_id: id,
                                        outcome: "too_early".into(),
                                        fish: None,
                                        quality: None,
                                        x: p.x,
                                        y: p.y,
                                    }),
                                },
                            }
                        } else {
                            None
                        }
                    };
                    // 2. 出鎖後才廣播（前端只對自己 id 演出飄字）。
                    if let Some(msg) = outcome_msg {
                        let _ = app.tx.send(Arc::new(msg));
                    }
                }

                // ── 敲礦／往更深一層挖（ROADMAP 348 礦脈深掘）────────────────────────
                Ok(ClientMsg::Mine) => {
                    // 沒在挖→驗未倒地、站岩地旁、冷卻到期→開一條新礦脈並挖第一層；
                    // 已在挖→直接再往下敲一層（冷卻不擋續敲，只擋開新礦脈）。
                    // 全程同一把 players 寫鎖、純記憶體；崩塌只清狀態不發礦。廣播在出鎖後送。
                    use crate::mining_vein::{is_near_rock, MiningVein, StrikeOutcome};
                    let outcome_msg = {
                        let mut players = app.players.write().unwrap();
                        if let Some(p) = players.get_mut(&id) {
                            // 倒地不能挖。
                            if p.vitals.is_downed() {
                                None
                            } else {
                                // 沒在挖→嘗試開新礦脈（須冷卻到期＋站岩地旁）。
                                if p.mining.is_none() {
                                    if p.mine_cooldown <= 0.0 && is_near_rock(p.x, p.y) {
                                        let seed = {
                                            let id_bytes = p.id.as_u128();
                                            ((id_bytes & 0xFFFF_FFFF_FFFF_FFFF) as u64)
                                                ^ p.mine_attempt_count
                                        };
                                        p.mine_attempt_count = p.mine_attempt_count.wrapping_add(1);
                                        p.mining = Some(MiningVein::open(seed));
                                    } else {
                                        // 沒站岩地旁或冷卻中：開不了礦脈，靜默忽略。
                                    }
                                }
                                // 有礦脈（含剛開的）→ 敲一層。
                                if let Some(vein) = p.mining.as_mut() {
                                    match vein.strike() {
                                        StrikeOutcome::Struck { ore, haul, depth, tremor } => {
                                            tracing::debug!(
                                                player = %p.name, depth, haul, "敲礦"
                                            );
                                            Some(ServerMsg::MineResult {
                                                player_id: id,
                                                outcome: "struck".into(),
                                                ore: Some(ore),
                                                haul: Some(haul),
                                                depth: Some(depth),
                                                tremor: Some(tremor.as_str().to_string()),
                                                x: p.x,
                                                y: p.y,
                                            })
                                        }
                                        StrikeOutcome::Collapsed => {
                                            // 崩塌：清礦脈、不給任何礦、起冷卻。
                                            p.mining = None;
                                            p.mine_cooldown =
                                                crate::mining_vein::MINE_COOLDOWN_SECS;
                                            tracing::info!(player = %p.name, "礦脈崩塌、整袋礦全埋");
                                            Some(ServerMsg::MineResult {
                                                player_id: id,
                                                outcome: "collapsed".into(),
                                                ore: None,
                                                haul: None,
                                                depth: None,
                                                tremor: None,
                                                x: p.x,
                                                y: p.y,
                                            })
                                        }
                                    }
                                } else {
                                    None
                                }
                            }
                        } else {
                            None
                        }
                    };
                    if let Some(msg) = outcome_msg {
                        let _ = app.tx.send(Arc::new(msg));
                    }
                }

                // ── 收礦撤出（ROADMAP 348 礦脈深掘）──────────────────────────────────
                Ok(ClientMsg::MineHaul) => {
                    // 把目前礦脈累積袋量落袋（礦石進背包＋探索熟練度），結束礦脈、起冷卻。
                    // 沒在挖則靜默忽略。同一把寫鎖，廣播出鎖後送（守 prod-deadlock 鐵律）。
                    use crate::mining_vein::{MiningVein, MINE_COOLDOWN_SECS};
                    let outcome_msg = {
                        let mut players = app.players.write().unwrap();
                        if let Some(p) = players.get_mut(&id) {
                            match p.mining.take() {
                                None => None,
                                Some(vein) => {
                                    let (ore, xp) = vein.haul_out();
                                    let depth = vein.depth();
                                    if ore > 0 {
                                        p.add_item_overflow(MiningVein::ore_kind(), ore);
                                    }
                                    if xp > 0 {
                                        p.masteries.gain_explorer(xp);
                                    }
                                    p.mine_cooldown = MINE_COOLDOWN_SECS;
                                    tracing::info!(
                                        player = %p.name, ore, depth, "收礦撤出"
                                    );
                                    Some(ServerMsg::MineResult {
                                        player_id: id,
                                        outcome: "hauled".into(),
                                        ore: None,
                                        haul: Some(ore),
                                        depth: Some(depth),
                                        tremor: None,
                                        x: p.x,
                                        y: p.y,
                                    })
                                }
                            }
                        } else {
                            None
                        }
                    };
                    if let Some(msg) = outcome_msg {
                        let _ = app.tx.send(Arc::new(msg));
                    }
                }

                // ── 觀星連星座：索取今夜星圖（ROADMAP 347）────────────────────────────
                Ok(ClientMsg::RequestStarMap) => {
                    // 夜裡才看得見星空：非夜間回 available=false，前端據此提示。
                    // 今夜星座由共享夜數決定（伺服器權威），逐夜輪替；只單播給請求者本人。
                    use crate::daynight::Phase;
                    use std::sync::atomic::Ordering;
                    let is_night = {
                        let dn = app.daynight.read().unwrap();
                        dn.phase() == Phase::Night
                    };
                    let traced_mask = {
                        let players = app.players.read().unwrap();
                        players.get(&id).map(|p| p.traced_constellations).unwrap_or(0)
                    };
                    let msg = if is_night {
                        let night = app.night_index.load(Ordering::Relaxed);
                        let c = crate::constellation::tonight(night);
                        let bit = crate::constellation::index_of(c.key).unwrap_or(0);
                        let traced = traced_mask & (1u64 << bit) != 0;
                        ServerMsg::StarMap {
                            available: true,
                            key: c.key.to_string(),
                            name: c.name.to_string(),
                            emoji: c.emoji.to_string(),
                            stars: c.stars.iter().map(|s| (s.x, s.y)).collect(),
                            traced,
                            total: traced_mask.count_ones(),
                            catalog_total: crate::constellation::TOTAL as u32,
                        }
                    } else {
                        ServerMsg::StarMap {
                            available: false,
                            key: String::new(),
                            name: String::new(),
                            emoji: String::new(),
                            stars: Vec::new(),
                            traced: false,
                            total: traced_mask.count_ones(),
                            catalog_total: crate::constellation::TOTAL as u32,
                        }
                    };
                    if let Ok(json) = serde_json::to_string(&msg) {
                        let _ = tx_direct.try_send(json);
                    }
                }

                // ── 觀星連星座：玩家送出連好的邊、由伺服器驗證（ROADMAP 347）──────────
                Ok(ClientMsg::TraceConstellation { edges }) => {
                    // 以**伺服器重算的今夜星座**為準驗證（前端送的星座不算數，防作弊）；
                    // 連對且首次即記入星座錄＋給乙太與探索熟練度。全程同一把 players 寫鎖、純記憶體，
                    // 廣播在出鎖後才送（守 prod-deadlock 鐵律：鎖內不送廣播）。
                    use crate::daynight::Phase;
                    use std::sync::atomic::Ordering;
                    // 非夜間一律不受理（看不見星空就連不了）。
                    let is_night = {
                        let dn = app.daynight.read().unwrap();
                        dn.phase() == Phase::Night
                    };
                    if !is_night {
                        continue;
                    }
                    let night = app.night_index.load(Ordering::Relaxed);
                    let c = crate::constellation::tonight(night);
                    let bit = crate::constellation::index_of(c.key).unwrap_or(0);
                    let correct = crate::constellation::check_trace(c, &edges);
                    // 鎖內判定／給獎／set bit，把要回傳的資料帶出鎖外。
                    let result = {
                        let mut players = app.players.write().unwrap();
                        if let Some(p) = players.get_mut(&id) {
                            if !correct {
                                Some((false, 0u32, p.traced_constellations.count_ones()))
                            } else {
                                let already = p.traced_constellations & (1u64 << bit) != 0;
                                if already {
                                    // 先前已連過：仍算連對，但不重複給獎（冪等，鏡像 sky_codex witness）。
                                    Some((true, 0u32, p.traced_constellations.count_ones()))
                                } else {
                                    p.traced_constellations |= 1u64 << bit;
                                    p.ether = p.ether.saturating_add(crate::constellation::ETHER_REWARD);
                                    p.masteries.gain_explorer(crate::constellation::EXPLORER_XP);
                                    tracing::info!(
                                        player = %p.name, constellation = c.key,
                                        "連對今夜星座、記入星座錄"
                                    );
                                    Some((
                                        true,
                                        crate::constellation::ETHER_REWARD,
                                        p.traced_constellations.count_ones(),
                                    ))
                                }
                            }
                        } else {
                            None
                        }
                    };
                    // 出鎖後才回覆（僅單播給本人，前端演出星座入錄飄字）。
                    if let Some((ok, reward_ether, total)) = result {
                        let msg = ServerMsg::ConstellationResult {
                            ok,
                            name: c.name.to_string(),
                            emoji: c.emoji.to_string(),
                            reward_ether,
                            total,
                            catalog_total: crate::constellation::TOTAL as u32,
                        };
                        if let Ok(json) = serde_json::to_string(&msg) {
                            let _ = tx_direct.try_send(json);
                        }
                    }
                }

                // ── 席間舉杯：玩家加入午餐社交（ROADMAP 329）────────────────────────
                Ok(ClientMsg::JoinLunchToast) => {
                    // 午休聚食時段，玩家走到鎮中廣場餐桌旁舉杯入席，鄰近就座的 NPC 轉頭回敬一句。
                    // 零 LLM、純查表；只發就地 NpcSpeech 泡泡（不洗世界聊天頻道），與 327/328 同調。
                    use crate::npc_schedule::{is_lunch_time, NpcActivity, VILLAGE_NPCS};
                    use crate::lunch_chatter::{nearest_seated, toast_line, display_name, TOAST_COOLDOWN_SECS};
                    // 1. 驗：正值午休時段（非午休一律不回敬）。
                    let lunching_now = {
                        let dn = app.daynight.read().unwrap();
                        is_lunch_time(dn.phase(), dn.fraction())
                    };
                    if lunching_now {
                        // 2. 取玩家位置 + 是否可舉杯（未倒地、冷卻到期）+ 取用句子序號。
                        let player_info = {
                            let players = app.players.read().unwrap();
                            players.get(&id).and_then(|p| {
                                if p.vitals.is_downed() || p.toast_cooldown > 0.0 {
                                    None
                                } else {
                                    Some((p.x, p.y, p.toast_count as usize))
                                }
                            })
                        };
                        if let Some((px, py, slot)) = player_info {
                            // 3. 收集目前真正就座（Lunching）的 NPC 座標，挑最近、在搆得著範圍內的那位。
                            let seats: Vec<(&'static str, f32, f32)> = {
                                let sched = app.npc_schedule.read().unwrap();
                                VILLAGE_NPCS
                                    .iter()
                                    .filter_map(|s| {
                                        if sched.get_activity(s.id) == Some(NpcActivity::Lunching) {
                                            sched.get_pos(s.id).map(|(x, y)| (s.id, x, y))
                                        } else {
                                            None
                                        }
                                    })
                                    .collect()
                            };
                            if let Some(npc_id) = nearest_seated(px, py, &seats) {
                                // 4. 確定有人回敬：扣冷卻、推進取句計數（讓回敬逐句不重複）。
                                if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                                    p.toast_cooldown = TOAST_COOLDOWN_SECS;
                                    p.toast_count = p.toast_count.wrapping_add(1);
                                }
                                // 4b. 記一筆相熟度（ROADMAP 330）：登入玩家用帳號 uid（跨連線延續本場），
                                //     訪客用連線 id（斷線即失，與訪客無存檔一致）。回傳累積次數與是否跨層。
                                let player_key = authed_uid.unwrap_or(id);
                                let toast_rec = app
                                    .lunch_regulars
                                    .write()
                                    .unwrap()
                                    .record(&player_key.to_string(), npc_id);
                                // 4c. 老友的餐贈（ROADMAP 332）：剛跨進更高一層交情的那一刻，
                                //     這位 NPC 順手把自家行當的一份心意塞進你背包，讓五片社交
                                //     的累積頭一回兌現成實打實的東西。只在「跨層」那刻送、每層
                                //     至多一份（複用 330 `crossed`，不需任何新帳本／冷卻），
                                //     份量刻意壓小、不含武器，近乎零經濟擾動。背包變多會由前端
                                //     快照差值自動噴「+N 物品」飄字＋報讀器播報，無需協議改動。
                                if let Some(tier) = toast_rec.crossed {
                                    if let Some(gift) = crate::lunch_gift::gift_for(npc_id, tier) {
                                        if let Some(p) =
                                            app.players.write().unwrap().get_mut(&id)
                                        {
                                            p.add_item_overflow(gift.item, gift.qty);
                                            tracing::info!(
                                                player = %p.name,
                                                npc = npc_id,
                                                ?tier,
                                                item = ?gift.item,
                                                qty = gift.qty,
                                                "老友餐贈"
                                            );
                                        }
                                    }
                                }
                                // 5. 廣播該 NPC 的回敬泡泡（就地定位在其座位上）；回敬語氣隨相熟度
                                //    升溫——生面孔客套、熟客熱絡，剛跨層還會冒一句專屬「混熟了」台詞。
                                if let Some(text) =
                                    crate::lunch_regular::toast_response(npc_id, toast_rec, slot)
                                        .or_else(|| toast_line(npc_id, slot))
                                {
                                    let (wx, wy) = seats
                                        .iter()
                                        .find(|(sid, _, _)| *sid == npc_id)
                                        .map(|&(_, x, y)| (x, y))
                                        .unwrap_or((px, py));
                                    let _ = app.tx.send(std::sync::Arc::new(
                                        crate::protocol::ServerMsg::NpcSpeech {
                                            npc_id: npc_id.to_string(),
                                            npc_name: display_name(npc_id).to_string(),
                                            text: text.to_string(),
                                            display_secs: 6,
                                            wx,
                                            wy,
                                        },
                                    ));
                                }
                            }
                        }
                    }
                }

                // ── 牧場系統（ROADMAP 48）──────────────────────────────────────────
                Ok(ClientMsg::BuyChicken { plot_id }) => {
                    // 購雞：需登入、玩家是農田地塊地主、乙太 ≥ BUY_CHICKEN_COST、未達 MAX_CHICKENS。
                    use crate::ranching::BUY_CHICKEN_COST;
                    use crate::land_plot::PlotPurpose;
                    if let Some(uid) = authed_uid {
                        let plot_owner = app.land_plots.read().unwrap().owner_of(plot_id);
                        let plot_purpose = app.land_plots.read().unwrap().purpose_of(plot_id);
                        if plot_owner == Some(uid) && plot_purpose == Some(PlotPurpose::Farm) {
                            let player_ether = app.players.read().unwrap().get(&uid).map(|p| p.ether).unwrap_or(0);
                            if player_ether >= BUY_CHICKEN_COST {
                                let ok = app.ranch.write().unwrap().buy_chicken(plot_id);
                                if ok {
                                    if let Some(p) = app.players.write().unwrap().get_mut(&uid) {
                                        p.ether = p.ether.saturating_sub(BUY_CHICKEN_COST);
                                        tracing::info!(player = %p.name, plot_id, "購雞");
                                    }
                                }
                            }
                        }
                    }
                }

                Ok(ClientMsg::CollectEggs { plot_id }) => {
                    // 收雞蛋：需登入、是地主、未倒地、有蛋。
                    if let Some(uid) = authed_uid {
                        let is_owner = app.land_plots.read().unwrap().owner_of(plot_id) == Some(uid);
                        if is_owner {
                            let downed = app.players.read().unwrap().get(&uid).map(|p| p.vitals.is_downed()).unwrap_or(true);
                            if !downed {
                                let (eggs, xp) = app.ranch.write().unwrap().collect_eggs(plot_id);
                                if eggs > 0 {
                                    if let Some(p) = app.players.write().unwrap().get_mut(&uid) {
                                        p.add_item_overflow(crate::inventory::ItemKind::Egg, eggs);
                                        p.masteries.gain_farmer(xp);
                                        tracing::info!(player = %p.name, eggs, "收雞蛋");
                                    }
                                }
                            }
                        }
                    }
                }

                // ── 農地作物系統（ROADMAP 49）──────────────────────────────────────
                Ok(ClientMsg::PlantCrop { plot_id, crop_type }) => {
                    // 種植作物：需登入、是農田地主、未倒地、乙太足夠、未達作物上限。
                    use crate::land_plot::PlotPurpose;
                    use crate::farm_crops::CropKind;
                    if let Some(uid) = authed_uid {
                        let plot_owner = app.land_plots.read().unwrap().owner_of(plot_id);
                        let plot_purpose = app.land_plots.read().unwrap().purpose_of(plot_id);
                        if plot_owner == Some(uid) && plot_purpose == Some(PlotPurpose::Farm) {
                            if let Some(kind) = CropKind::from_str(&crop_type) {
                                let cost = kind.plant_cost();
                                let downed = app.players.read().unwrap().get(&uid)
                                    .map(|p| p.vitals.is_downed()).unwrap_or(true);
                                let ether = app.players.read().unwrap().get(&uid)
                                    .map(|p| p.ether).unwrap_or(0);
                                if !downed && ether >= cost {
                                    let ok = app.farm_crops.write().unwrap().plant(plot_id, kind);
                                    if ok {
                                        if let Some(p) = app.players.write().unwrap().get_mut(&uid) {
                                            p.ether = p.ether.saturating_sub(cost);
                                            tracing::info!(player = %p.name, ?kind, plot_id, "種植作物");
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                Ok(ClientMsg::HarvestCrops { plot_id }) => {
                    // 收割作物：需登入、是地主、未倒地、有成熟作物。
                    if let Some(uid) = authed_uid {
                        let is_owner = app.land_plots.read().unwrap().owner_of(plot_id) == Some(uid);
                        if is_owner {
                            let downed = app.players.read().unwrap().get(&uid)
                                .map(|p| p.vitals.is_downed()).unwrap_or(true);
                            if !downed {
                                let (items, xp) = app.farm_crops.write().unwrap().harvest(plot_id);
                                if !items.is_empty() {
                                    if let Some(p) = app.players.write().unwrap().get_mut(&uid) {
                                        for (item, qty) in &items {
                                            p.add_item_overflow(*item, *qty);
                                        }
                                        p.masteries.gain_farmer(xp);
                                        tracing::info!(player = %p.name, plot_id, items = items.len(), "收割作物");
                                    }
                                }
                            }
                        }
                    }
                }

                // ── 夜採星晶（ROADMAP 50）────────────────────────────────────────────
                Ok(ClientMsg::GatherStarCrystal) => {
                    // 採集星晶礦脈：需夜間、在礦脈 80px 內、未倒地、已登入。
                    use crate::inventory::ItemKind;
                    if let Some(uid) = authed_uid {
                        let is_night = {
                            app.daynight.read().unwrap().phase() == crate::daynight::Phase::Night
                        };
                        if is_night {
                            let (px, py, is_downed) = {
                                app.players.read().unwrap()
                                    .get(&uid)
                                    .map(|p| (p.x, p.y, p.vitals.is_downed()))
                                    .unwrap_or((0.0, 0.0, true))
                            };
                            if !is_downed {
                                let gathered = app.star_crystals.write().unwrap().gather_near(px, py);
                                if gathered {
                                    if let Some(p) = app.players.write().unwrap().get_mut(&uid) {
                                        p.add_item_overflow(ItemKind::StarCrystalShard, 1);
                                        p.masteries.gain_explorer(crate::star_crystal::GATHER_EXPLORER_XP);
                                        tracing::info!(player = %p.name, "採集星晶碎片");
                                    }
                                }
                            }
                        }
                    }
                }

                // ── 乙太微粒採集（ROADMAP 142 死亡餵養生命）────────────────────────
                Ok(ClientMsg::CollectCarrionOrb { orb_id }) => {
                    // 採集乙太微粒：需未倒地、在微粒 CARION_COLLECT_RADIUS 內。
                    use crate::wildlife::CARION_ETHER;
                    let (px, py, is_downed) = app.players.read().unwrap()
                        .get(&id)
                        .map(|p| (p.x, p.y, p.vitals.is_downed()))
                        .unwrap_or((0.0, 0.0, true));
                    if !is_downed {
                        let result = app.wildlife_manager.write().unwrap()
                            .collect_carion_orb(orb_id, px, py);
                        if result.is_some() {
                            let name = app.players.read().unwrap()
                                .get(&id)
                                .map(|p| p.name.clone())
                                .unwrap_or_default();
                            if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                                p.ether = p.ether.saturating_add(CARION_ETHER);
                                tracing::info!(player = %name, ether = CARION_ETHER, "採集乙太微粒");
                            }
                            let msg = format!(
                                "🌿 {} 採集了乙太微粒，得到 {} 乙太。萬物皆有其歸宿，死亡是循環的一環。",
                                name, CARION_ETHER
                            );
                            let _ = app.tx_chat.send(msg);
                        }
                    }
                }

                // ── 季節性野外採集節點（ROADMAP 154）──────────────────────────────────
                Ok(ClientMsg::GatherSeasonalNode { node_id }) => {
                    // 採集季節性節點：未倒地、在節點 80px 內、節點有剩餘次數。
                    use crate::inventory::ItemKind;
                    let (px, py, is_downed) = app.players.read().unwrap()
                        .get(&id)
                        .map(|p| (p.x, p.y, p.vitals.is_downed()))
                        .unwrap_or((0.0, 0.0, true));
                    if !is_downed {
                        let result = app.seasonal_nodes.write().unwrap()
                            .try_gather(node_id, px, py);
                        if let Some(item_kind) = result {
                            if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                                p.add_item_overflow(item_kind, 1);
                                tracing::info!(player = %p.name, ?item_kind, "採集季節性節點");
                            }
                        }
                    }
                    let _ = (node_id, px, py);
                }

                // ── 攻擊野生動物（ROADMAP 144）──────────────────────────────────────
                Ok(ClientMsg::AttackWildlife { wildlife_id }) => {
                    use crate::species_relations::ATTACK_WILDLIFE_REACH;
                    let (px, py, is_downed) = app.players.read().unwrap()
                        .get(&id)
                        .map(|p| (p.x, p.y, p.vitals.is_downed()))
                        .unwrap_or((0.0, 0.0, true));
                    if !is_downed {
                        let killed_kind = app.wildlife_manager.write().unwrap()
                            .attack_wildlife(wildlife_id, px, py, ATTACK_WILDLIFE_REACH);
                        if let Some(kind) = killed_kind {
                            use crate::wildlife::TrophicLevel;
                            // 鎖序鐵律：絕不在 species_relations guard 內再鎖 players——遊戲迴圈快照
                            // 反向持鎖（players.read → species_relations.read），加上隨時排隊的
                            // players.write（移動輸入）會組成三方死鎖環（寫者優先讓快照的 read 排隊）。
                            // 先讀好名字，再短暫鎖 sr，兩把鎖永不重疊。
                            let name = app.players.read().unwrap()
                                .get(&id).map(|p| p.name.clone()).unwrap_or_default();
                            {
                                let mut sr = app.species_relations.write().unwrap();
                                if kind.trophic_level() == TrophicLevel::Predator {
                                    // 殺死掠食者 → 被獵物種好感+
                                    sr.on_kill_predator(kind);
                                } else {
                                    // 殺死獵物 → 該物種敵意+
                                    sr.on_kill_prey(kind);
                                }
                            }
                            let msg = format!("🗡️ {} 攻擊了一隻 {}。", name, kind.display_name());
                            let _ = app.tx_chat.send(msg);
                        }
                    }
                }

                // ── 挑戰巢穴 Alpha（ROADMAP 168）────────────────────────────────────
                Ok(ClientMsg::AttackAlpha { alpha_id }) => {
                    use crate::monster_colony::ALPHA_ATTACK_REACH;
                    use crate::inventory::ItemKind;
                    // 攻擊冷卻閘（防外掛洪水秒殺世界頭目）：比照一般 Attack handler。
                    const ATTACK_COOLDOWN_SECS: f32 = 0.6;
                    let (px, py, is_downed, cooldown, power) = {
                        let players = app.players.read().unwrap();
                        let p = players.get(&id);
                        let power = p.map(|p| {
                            let base = crate::combat::level_attack_bonus(p.level())
                                + crate::equipment::equipped_weapon_power(&p.equipment)
                                + 1; // 最小 1 傷害
                            base.max(1)
                        }).unwrap_or(1);
                        (
                            p.map(|p| p.x).unwrap_or(0.0),
                            p.map(|p| p.y).unwrap_or(0.0),
                            p.map(|p| p.vitals.is_downed()).unwrap_or(true),
                            p.map(|p| p.attack_cooldown).unwrap_or(0.0),
                            power,
                        )
                    };
                    if !is_downed && cooldown <= 0.0 {
                        // 命中與否都設冷卻，防止洪水攻擊。
                        if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                            p.attack_cooldown = p.stats.effective_attack_cooldown(ATTACK_COOLDOWN_SECS);
                        }
                        let kill_result = app.monster_colonies.write().unwrap()
                            .attack_alpha(alpha_id, px, py, power, ALPHA_ATTACK_REACH);
                        if let Some(result) = kill_result {
                            use crate::monster_colony::{ALPHA_KILLER_ETHER, ALPHA_GLOBAL_ETHER, ALPHA_CRYSTAL_DROP, ALLIANCE_BREAK_BONUS_ETHER, AWAKENED_BONUS_ETHER, DOMINANT_ALPHA_BONUS_ETHER};
                            // 殺手個人獎勵（盟約中額外獎勵 + 覺醒中額外獎勵 + 霸主中額外獎勵）
                            let alliance_bonus = if result.was_allied { ALLIANCE_BREAK_BONUS_ETHER } else { 0 };
                            let awakened_bonus = if result.was_awakened { AWAKENED_BONUS_ETHER } else { 0 };
                            let dominant_bonus = if result.was_dominant { DOMINANT_ALPHA_BONUS_ETHER } else { 0 };
                            let total_killer = ALPHA_KILLER_ETHER + alliance_bonus + awakened_bonus + dominant_bonus;
                            let killer_name = {
                                let mut players = app.players.write().unwrap();
                                if let Some(p) = players.get_mut(&id) {
                                    p.ether = p.ether.saturating_add(total_killer);
                                    p.inventory.add(ItemKind::AlphaCrystal, ALPHA_CRYSTAL_DROP);
                                    p.name.clone()
                                } else {
                                    "某玩家".to_string()
                                }
                            };
                            // 全服在線玩家各得乙太
                            {
                                let mut players = app.players.write().unwrap();
                                for p in players.values_mut() {
                                    p.ether = p.ether.saturating_add(ALPHA_GLOBAL_ETHER);
                                }
                            }
                            let kind_name = result.kind.display_name();
                            let colony_name = result.colony_name;
                            if result.was_awakened && result.was_allied {
                                let _ = app.tx_chat.send(format!(
                                    "🔥⚔️ [覺醒盟約破碎！] {killer_name} 在覺醒危機中瓦解了 {colony_name} 的盟約 Alpha「{kind_name}·霸主」！\
                                     全服在線玩家各得 +{ALPHA_GLOBAL_ETHER} 乙太，\
                                     {killer_name} 額外獲得 +{total_killer} 乙太（含覺醒獎勵 +{AWAKENED_BONUS_ETHER}）+ Alpha 晶核💎！"
                                ));
                            } else if result.was_awakened {
                                let _ = app.tx_chat.send(format!(
                                    "🔥 [覺醒 Alpha 制伏！] {killer_name} 在覺醒危機中擊倒了 {colony_name} 的 Alpha 首領「{kind_name}·霸主」！\
                                     全服在線玩家各得 +{ALPHA_GLOBAL_ETHER} 乙太，\
                                     {killer_name} 額外獲得 +{total_killer} 乙太（含覺醒獎勵 +{AWAKENED_BONUS_ETHER}）+ Alpha 晶核💎！"
                                ));
                            } else if result.was_allied {
                                let _ = app.tx_chat.send(format!(
                                    "💎⚔️ [盟約破碎！] {killer_name} 瓦解了 {colony_name} 的盟約 Alpha「{kind_name}·霸主」！\
                                     全服在線玩家各得 +{ALPHA_GLOBAL_ETHER} 乙太，\
                                     {killer_name} 額外獲得 +{total_killer} 乙太（含破盟獎勵 +{ALLIANCE_BREAK_BONUS_ETHER}）+ Alpha 晶核💎！"
                                ));
                            } else {
                                let _ = app.tx_chat.send(format!(
                                    "💎 [Alpha 擊倒！] {killer_name} 制伏了 {colony_name} 的 Alpha 首領「{kind_name}·霸主」！\
                                     全服在線玩家各得 +{ALPHA_GLOBAL_ETHER} 乙太，{killer_name} 額外獲得 +{ALPHA_KILLER_ETHER} 乙太 + Alpha 晶核💎！"
                                ));
                            }
                            // ROADMAP 176：霸主 Alpha 被擊殺，廣播霸主落幕（含額外獎勵說明）
                            if result.was_dominant {
                                let _ = app.tx_chat.send(format!(
                                    "👑【霸主 Alpha 倒下！】{killer_name} 終結了 {colony_name} 的霸主之勢！\
                                     額外獲得 +{DOMINANT_ALPHA_BONUS_ETHER} 乙太！"
                                ));
                            }
                            // ROADMAP 183：斬首路——指揮全族的 Alpha 倒下，群龍無首，殘部當場潰逃回巢。
                            // monster_colonies 寫鎖已於 attack_alpha 結束時釋放，此處只暫借 enemies 寫鎖。
                            let fled = app.enemies.write().unwrap().rout_region(
                                result.cx, result.cy, result.kind, result.rout_radius,
                                crate::monster_colony::ROUT_DURATION_SECS,
                            );
                            if fled > 0 {
                                let _ = app.tx_chat.send(format!(
                                    "💨 [{colony_name}] 首領倒下，群龍無首——殘部驚潰逃竄！"
                                ));
                            }
                            // ROADMAP 185：菁英 Alpha 殞落凱旋——覺醒/霸主菁英首領被討伐，城鎮居民歡慶（🎉）。
                            // 城鎮仍在生態避難警戒時不歡慶（危機未解、避難優先），notify_hero_triumph 回傳 0、連捷報都不發。
                            if result.was_awakened || result.was_dominant {
                                let cheering = app.residents.write().unwrap().notify_hero_triumph(killer_name.clone());
                                if cheering > 0 {
                                    let _ = app.tx_chat.send(format!(
                                        "🎉【全城歡慶】{killer_name} 討伐了 {colony_name} 的菁英首領「{kind_name}·霸主」\
                                         ——城鎮居民紛紛放下手邊事、雀躍歡呼慶賀英雄凱旋！"
                                    ));
                                }
                            }
                        }
                    }
                }

                // ── 挑戰傳說古 Alpha（ROADMAP 173）──────────────────────────────────
                Ok(ClientMsg::AttackAncientAlpha) => {
                    use crate::monster_colony::{ANCIENT_ALPHA_ATTACK_REACH, ANCIENT_ALPHA_KILLER_ETHER, ANCIENT_ALPHA_GLOBAL_ETHER};
                    use crate::inventory::ItemKind;
                    // 攻擊冷卻閘（防外掛洪水秒殺傳說古 Alpha）：比照一般 Attack handler。
                    const ATTACK_COOLDOWN_SECS: f32 = 0.6;
                    let (px, py, is_downed, cooldown, power) = {
                        let players = app.players.read().unwrap();
                        let p = players.get(&id);
                        let power = p.map(|p| {
                            let base = crate::combat::level_attack_bonus(p.level())
                                + crate::equipment::equipped_weapon_power(&p.equipment)
                                + 1;
                            base.max(1)
                        }).unwrap_or(1);
                        (
                            p.map(|p| p.x).unwrap_or(0.0),
                            p.map(|p| p.y).unwrap_or(0.0),
                            p.map(|p| p.vitals.is_downed()).unwrap_or(true),
                            p.map(|p| p.attack_cooldown).unwrap_or(0.0),
                            power,
                        )
                    };
                    if !is_downed && cooldown <= 0.0 {
                        // 命中與否都設冷卻，防止洪水攻擊。
                        if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                            p.attack_cooldown = p.stats.effective_attack_cooldown(ATTACK_COOLDOWN_SECS);
                        }
                        let _ = ANCIENT_ALPHA_ATTACK_REACH; // 距離驗證在 attack_ancient_alpha 內
                        let kill_result = app.monster_colonies.write().unwrap()
                            .attack_ancient_alpha(px, py, power);
                        if kill_result.is_some() {
                            // 殺手個人獎勵：乙太 + 傳說晶核
                            let killer_name = {
                                let mut players = app.players.write().unwrap();
                                if let Some(p) = players.get_mut(&id) {
                                    p.ether = p.ether.saturating_add(ANCIENT_ALPHA_KILLER_ETHER);
                                    p.inventory.add(ItemKind::LegendaryCore, 1);
                                    p.name.clone()
                                } else {
                                    "某玩家".to_string()
                                }
                            };
                            // 全服在線玩家各得乙太
                            {
                                let mut players = app.players.write().unwrap();
                                for p in players.values_mut() {
                                    p.ether = p.ether.saturating_add(ANCIENT_ALPHA_GLOBAL_ETHER);
                                }
                            }
                            let _ = app.tx_chat.send(format!(
                                "🌟【傳說古 Alpha 倒下！】{killer_name} 率眾擊倒了傳說古 Alpha！\
                                 全服在線玩家各得 +{ANCIENT_ALPHA_GLOBAL_ETHER} 乙太，\
                                 {killer_name} 額外獲得 +{ANCIENT_ALPHA_KILLER_ETHER} 乙太 + 傳說晶核💫！\
                                 傳說戰刃等你來合成！"
                            ));
                        }
                    }
                }

                // ── 餵食野生動物（ROADMAP 144）──────────────────────────────────────
                Ok(ClientMsg::FeedWildlife { wildlife_id }) => {
                    use crate::species_relations::FEED_REACH;
                    use crate::inventory::ItemKind;
                    let (px, py, is_downed, has_seed) = {
                        let players = app.players.read().unwrap();
                        let p = players.get(&id);
                        (
                            p.map(|p| p.x).unwrap_or(0.0),
                            p.map(|p| p.y).unwrap_or(0.0),
                            p.map(|p| p.vitals.is_downed()).unwrap_or(true),
                            p.map(|p| p.inventory.count(ItemKind::WildflowerSeed) > 0).unwrap_or(false),
                        )
                    };
                    if !is_downed && has_seed {
                        // 找在餵食距離內的指定野生動物。
                        let target_kind = {
                            let wm = app.wildlife_manager.read().unwrap();
                            let reach2 = FEED_REACH * FEED_REACH;
                            wm.animals.iter().find(|a| {
                                a.id == wildlife_id && a.alive
                                    && (a.x - px).powi(2) + (a.y - py).powi(2) <= reach2
                            }).map(|a| a.kind)
                        };
                        if let Some(kind) = target_kind {
                            // 消耗一個野花種子。
                            if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                                p.inventory.take(ItemKind::WildflowerSeed, 1);
                            }
                            app.species_relations.write().unwrap().on_feed(kind);
                            // ROADMAP 205：提升「這一隻」的個體親近度；回傳「是否剛跨過馴養門檻」。
                            let just_tamed = app.wildlife_manager.write().unwrap()
                                .on_feed_animal(wildlife_id)
                                .map(|(_, _, t)| t)
                                .unwrap_or(false);
                            let name = app.players.read().unwrap()
                                .get(&id).map(|p| p.name.clone()).unwrap_or_default();
                            let attitude = app.species_relations.read().unwrap().attitude(kind);
                            let msg = format!(
                                "🌿 {} 餵食了 {}（消耗野花種子×1）。{} 對人類的態度：{}",
                                name, kind.display_name(), kind.display_name(), attitude
                            );
                            let _ = app.tx_chat.send(msg);
                            // ROADMAP 205：這一隻第一次被餵到「馴養」——溫馨慶賀，從此牠不再怕這位玩家、會跟著你。
                            if just_tamed {
                                let _ = app.tx_chat.send(format!(
                                    "💛 {} 與一隻 {} 建立了信任——牠不再害怕，願意親近並跟隨左右了。",
                                    name, kind.display_name()
                                ));
                            }
                        }
                    }
                }

                // ── 星際貿易（ROADMAP 51）──────────────────────────────────────────
                Ok(ClientMsg::PickupTrade { route_id }) => {
                    // 接取貿易任務：需登入、未倒地、在本星球商人 SHOP_REACH 內、無包裹、路線不在冷卻。
                    use crate::trade_route::{try_pickup, TRADE_COOLDOWN_SECS};
                    use crate::npc::SHOP_REACH;
                    if let Some(uid) = authed_uid {
                        let result = {
                            let players = app.players.read().unwrap();
                            if let Some(p) = players.get(&uid) {
                                if p.vitals.is_downed() {
                                    None
                                } else {
                                    // 驗距離（靠近本星球商人）。
                                    let merchant_xy = match p.planet.as_str() {
                                        "verdant" => crate::npc::verdant_merchant_pos(),
                                        "crimson" => crate::npc::crimson_merchant_pos(),
                                        "void"    => crate::npc::void_merchant_pos(),
                                        "aether"  => crate::npc::aether_merchant_pos(),
                                        "origin"  => crate::npc::origin_merchant_pos(),
                                        _         => crate::npc::merchant_pos(),
                                    };
                                    let dx = p.x - merchant_xy.0;
                                    let dy = p.y - merchant_xy.1;
                                    if (dx * dx + dy * dy).sqrt() > SHOP_REACH {
                                        None // 離商人太遠
                                    } else {
                                        try_pickup(route_id, &p.planet, &p.trade_cargo, &p.trade_cooldowns)
                                    }
                                }
                            } else {
                                None
                            }
                        };
                        if let Some(cargo) = result {
                            if let Some(p) = app.players.write().unwrap().get_mut(&uid) {
                                let rid = cargo.route_id;
                                p.trade_cargo = Some(cargo);
                                p.trade_cooldowns.insert(rid, TRADE_COOLDOWN_SECS);
                                tracing::info!(player = %p.name, route_id, "接取貿易任務");
                            }
                        }
                    }
                }

                Ok(ClientMsg::DeliverTrade) => {
                    // 交付貿易包裹：需登入、在目標星球、靠近目標商人。
                    use crate::trade_route::{try_deliver, TRADE_MERCHANT_XP};
                    use crate::npc::SHOP_REACH;
                    if let Some(uid) = authed_uid {
                        let reward = {
                            let players = app.players.read().unwrap();
                            if let Some(p) = players.get(&uid) {
                                if p.vitals.is_downed() {
                                    0
                                } else {
                                    // 驗距離（靠近目標星球商人）。
                                    let merchant_xy = match p.planet.as_str() {
                                        "verdant" => crate::npc::verdant_merchant_pos(),
                                        "crimson" => crate::npc::crimson_merchant_pos(),
                                        "void"    => crate::npc::void_merchant_pos(),
                                        "aether"  => crate::npc::aether_merchant_pos(),
                                        "origin"  => crate::npc::origin_merchant_pos(),
                                        _         => crate::npc::merchant_pos(),
                                    };
                                    let dx = p.x - merchant_xy.0;
                                    let dy = p.y - merchant_xy.1;
                                    if (dx * dx + dy * dy).sqrt() > SHOP_REACH {
                                        0 // 離商人太遠
                                    } else {
                                        try_deliver(&p.planet, &p.trade_cargo)
                                    }
                                }
                            } else {
                                0
                            }
                        };
                        if reward > 0 {
                            if let Some(p) = app.players.write().unwrap().get_mut(&uid) {
                                p.ether = p.ether.saturating_add(reward);
                                p.trade_cargo = None;
                                p.masteries.gain_merchant(TRADE_MERCHANT_XP);
                                tracing::info!(player = %p.name, reward, "交付貿易包裹");
                            }
                            // 記入玩家事跡日誌（ROADMAP 67）。
                            app.player_logs.write().unwrap()
                                .entry(uid)
                                .or_default()
                                .push(format!("完成星際貿易路線，獲得 {} 乙太", reward));
                        }
                    }
                }

                Ok(ClientMsg::CancelTrade) => {
                    // 取消貿易任務：丟棄包裹，無懲罰。
                    if let Some(uid) = authed_uid {
                        if let Some(p) = app.players.write().unwrap().get_mut(&uid) {
                            if p.trade_cargo.take().is_some() {
                                tracing::info!(player = %p.name, "取消貿易任務");
                            }
                        }
                    }
                }

                // ── 工匠工坊訂單（ROADMAP 52）─────────────────────────────────────
                Ok(ClientMsg::TakeWorkshopOrder { order_id }) => {
                    // 接取工坊訂單：需登入、故鄉、未倒地、靠近工坊 NPC、無進行中訂單、無冷卻。
                    use crate::workshop::{try_take, WORKSHOP_COOLDOWN_SECS};
                    if let Some(uid) = authed_uid {
                        let result = {
                            let players = app.players.read().unwrap();
                            if let Some(p) = players.get(&uid) {
                                if p.vitals.is_downed()
                                    || p.planet != crate::state::PLANET_HOME
                                    || !app.is_near_npc(p.x, p.y, "workshop_npc")
                                {
                                    None
                                } else {
                                    try_take(order_id, &p.workshop_active, p.workshop_cooldown)
                                }
                            } else {
                                None
                            }
                        };
                        if let Some(active) = result {
                            if let Some(p) = app.players.write().unwrap().get_mut(&uid) {
                                tracing::info!(player = %p.name, order_id, "接取工坊訂單");
                                p.workshop_active = Some(active);
                            }
                        }
                    }
                }

                Ok(ClientMsg::FulfillWorkshopOrder) => {
                    // 交付工坊訂單：需登入、靠近工坊 NPC、有進行中訂單、背包有足夠物品。
                    use crate::workshop::{try_fulfill, WORKSHOP_COOLDOWN_SECS};
                    if let Some(uid) = authed_uid {
                        let result = {
                            let players = app.players.read().unwrap();
                            if let Some(p) = players.get(&uid) {
                                if p.vitals.is_downed()
                                    || p.planet != crate::state::PLANET_HOME
                                    || !app.is_near_npc(p.x, p.y, "workshop_npc")
                                {
                                    None
                                } else {
                                    try_fulfill(&p.workshop_active, &p.inventory)
                                }
                            } else {
                                None
                            }
                        };
                        if let Some((reward, xp)) = result {
                            let player_name;
                            let total_reward;
                            {
                                let mut players = app.players.write().unwrap();
                                if let Some(p) = players.get_mut(&uid) {
                                    // 從背包扣除所需物品。
                                    if let Some(ref active) = p.workshop_active.clone() {
                                        if let Some(order) = crate::workshop::find_order(active.order_id) {
                                            p.inventory.take(order.required_item, order.required_qty);
                                        }
                                    }
                                    p.ether = p.ether.saturating_add(reward);
                                    p.masteries.gain_artisan(xp);
                                    p.workshop_active = None;
                                    p.workshop_cooldown = WORKSHOP_COOLDOWN_SECS;
                                    player_name = p.name.clone();
                                    total_reward = reward;
                                    tracing::info!(player = %p.name, reward, xp, "交付工坊訂單");
                                } else {
                                    player_name = String::new();
                                    total_reward = reward;
                                }
                            }
                            // 急修加成令（ROADMAP 87）：工坊訂單完成時檢查是否有加成。
                            if !player_name.is_empty() {
                                let boost_result = app.npc_workshop_boost.write().unwrap().on_order_fulfilled();
                                if boost_result.bonus > 0 {
                                    if let Some(p) = app.players.write().unwrap().get_mut(&uid) {
                                        p.ether = p.ether.saturating_add(boost_result.bonus);
                                    }
                                    let npc = crate::npc_workshop_boost::WORKSHOP_NPC_NAME;
                                    if boost_result.fulfilled {
                                        // 達到配額，只廣播完成公告。
                                        let txt = crate::npc_workshop_boost::fulfilled_text();
                                        let _ = app.tx_chat.send(format!("✅ [{npc}] 宣告：「{txt}」"));
                                    } else {
                                        // 未達配額，顯示剩餘份數。
                                        let remaining = app.npc_workshop_boost.read().unwrap()
                                            .active.as_ref()
                                            .map(|b| b.quota.saturating_sub(b.filled))
                                            .unwrap_or(0);
                                        let _ = app.tx_chat.send(format!(
                                            "🔨 [{npc}] 補充：「好手藝！{player_name} 今天的工坊活兒做得漂亮，多給你 {} 乙太！（還剩 {} 份加成）」",
                                            boost_result.bonus, remaining
                                        ));
                                    }
                                }
                            }
                            // 記入玩家事跡日誌（ROADMAP 67）。
                            app.player_logs.write().unwrap()
                                .entry(uid)
                                .or_default()
                                .push(format!("在工坊完成了加急訂單，獲得 {} 乙太", total_reward));
                        }
                    }
                }

                Ok(ClientMsg::AbandonWorkshopOrder) => {
                    // 放棄工坊訂單：取消進行中訂單，無懲罰（不啟動冷卻）。
                    if let Some(uid) = authed_uid {
                        if let Some(p) = app.players.write().unwrap().get_mut(&uid) {
                            if p.workshop_active.take().is_some() {
                                tracing::info!(player = %p.name, "放棄工坊訂單");
                            }
                        }
                    }
                }

                // ── 懸賞告示板（ROADMAP 53）────────────────────────────────────────
                Ok(ClientMsg::AcceptBounty { card_id }) => {
                    // 接取懸賞任務：需登入、故鄉、未倒地、靠近告示板 NPC、無進行中任務、無冷卻。
                    use crate::bounty_board::{try_accept};
                    if let Some(uid) = authed_uid {
                        let result = {
                            let players = app.players.read().unwrap();
                            if let Some(p) = players.get(&uid) {
                                if p.vitals.is_downed()
                                    || p.planet != crate::state::PLANET_HOME
                                    || !app.is_near_npc(p.x, p.y, "bounty_npc")
                                {
                                    None
                                } else {
                                    try_accept(card_id, &p.bounty_active, p.bounty_cooldown)
                                }
                            } else {
                                None
                            }
                        };
                        if let Some(active) = result {
                            if let Some(p) = app.players.write().unwrap().get_mut(&uid) {
                                tracing::info!(player = %p.name, card_id, "接取懸賞任務");
                                p.bounty_active = Some(active);
                            }
                        }
                    }
                }

                Ok(ClientMsg::AbandonBounty) => {
                    // 放棄懸賞任務：取消進行中任務，無懲罰（不啟動冷卻）。
                    if let Some(uid) = authed_uid {
                        if let Some(p) = app.players.write().unwrap().get_mut(&uid) {
                            if p.bounty_active.take().is_some() {
                                tracing::info!(player = %p.name, "放棄懸賞任務");
                            }
                        }
                    }
                }

                // ── 古蹟探勘（ROADMAP 54）──────────────────────────────────────────
                Ok(ClientMsg::AcceptExpedition { order_id }) => {
                    use crate::expedition::{try_accept};
                    if let Some(uid) = authed_uid {
                        let mut players = app.players.write().unwrap();
                        if let Some(p) = players.get_mut(&uid) {
                            if !p.vitals.is_downed()
                                && p.planet == crate::state::PLANET_HOME
                                && app.is_near_npc(p.x, p.y, "expedition_npc")
                            {
                                if let Some(active) = try_accept(order_id, &p.expedition_active, p.expedition_cooldown) {
                                    p.expedition_active = Some(active);
                                    tracing::info!(player = %p.name, order_id, "接取探勘令");
                                }
                            }
                        }
                    }
                }

                Ok(ClientMsg::SurveyExpedition) => {
                    // 採樣：驗生態域 + 距主城距離，成功立即發獎並進入冷卻。
                    use crate::expedition::try_survey;
                    if let Some(uid) = authed_uid {
                        let result = {
                            let mut players = app.players.write().unwrap();
                            if let Some(p) = players.get_mut(&uid) {
                                if p.vitals.is_downed() { None }
                                else {
                                    try_survey(&p.expedition_active, p.x, p.y).map(|(reward, xp)| {
                                        p.expedition_active = None;
                                        p.expedition_cooldown = crate::expedition::EXPEDITION_COOLDOWN_SECS;
                                        p.ether = p.ether.saturating_add(reward);
                                        p.masteries.gain_explorer(xp);
                                        tracing::info!(player = %p.name, reward, xp, "探勘採樣完成");
                                        (p.name.clone(), reward, xp)
                                    })
                                }
                            } else { None }
                        };
                        if let Some((pname, reward, xp)) = result {
                            // 探勘加碼令（ROADMAP 86）：若有活躍加碼令，扣減配額並發額外獎勵。
                            let boost_result = app.npc_expedition_boost.write().unwrap().on_surveyed();
                            if boost_result.bonus > 0 {
                                if let Some(p) = app.players.write().unwrap().get_mut(&uid) {
                                    p.ether = p.ether.saturating_add(boost_result.bonus);
                                }
                                let npc = crate::npc_expedition_boost::EXPEDITION_NPC_NAME;
                                let _ = app.tx_chat.send(format!(
                                    "🗺️ {} 完成探勘採樣！獲得 {} 乙太 + {} 探索者 XP！（🎉 加碼 +{} 乙太！）",
                                    pname, reward, xp, boost_result.bonus
                                ));
                                if boost_result.fulfilled {
                                    let txt = crate::npc_expedition_boost::fulfilled_text();
                                    let _ = app.tx_chat.send(format!("✅ [{npc}] 宣告：「{txt}」"));
                                }
                            } else {
                                let _ = app.tx_chat.send(format!(
                                    "🗺️ {} 完成探勘採樣！獲得 {} 乙太 + {} 探索者 XP！",
                                    pname, reward, xp
                                ));
                            }
                            // 記入玩家事跡日誌（ROADMAP 67）。
                            app.player_logs.write().unwrap()
                                .entry(uid)
                                .or_default()
                                .push(format!("完成野外探勘採樣任務，獲得 {} 乙太", reward));
                            let _ = pname; // suppress unused warning
                            let _ = xp;
                        }
                    }
                }

                Ok(ClientMsg::AbandonExpedition) => {
                    // 放棄探勘任務：取消進行中任務，無懲罰（不啟動冷卻）。
                    if let Some(uid) = authed_uid {
                        if let Some(p) = app.players.write().unwrap().get_mut(&uid) {
                            if p.expedition_active.take().is_some() {
                                tracing::info!(player = %p.name, "放棄探勘任務");
                            }
                        }
                    }
                }

                // ── 星際採購令（ROADMAP 55）────────────────────────────────────────
                Ok(ClientMsg::AcceptProcurement { order_id }) => {
                    // 接取採購令：需故鄉、未倒地、靠近代理人、無進行中任務、不在冷卻。
                    use crate::procurement::{try_accept, is_near_procurement_agent};
                    if let Some(uid) = authed_uid {
                        let mut players = app.players.write().unwrap();
                        if let Some(p) = players.get_mut(&uid) {
                            if !p.vitals.is_downed()
                                && p.planet == crate::state::PLANET_HOME
                                && app.is_near_npc(p.x, p.y, "procurement_npc")
                            {
                                if let Some(active) = try_accept(order_id, &p.procurement_active, p.procurement_cooldown) {
                                    p.procurement_active = Some(active);
                                    tracing::info!(player = %p.name, order_id, "接取採購令");
                                }
                            }
                        }
                    }
                }

                Ok(ClientMsg::DeliverProcurement) => {
                    // 交付採購令：靠近代理人、背包碎片足夠時完成任務並發獎。
                    use crate::procurement::{try_deliver, is_near_procurement_agent};
                    if let Some(uid) = authed_uid {
                        let result = {
                            let mut players = app.players.write().unwrap();
                            if let Some(p) = players.get_mut(&uid) {
                                if p.vitals.is_downed() || p.planet != crate::state::PLANET_HOME {
                                    None
                                } else if !app.is_near_npc(p.x, p.y, "procurement_npc") {
                                    None
                                } else {
                                    let inv_qty = if let Some(a) = &p.procurement_active {
                                        if let Some(o) = crate::procurement::find_order(a.order_id) {
                                            p.inventory.count(o.required_item)
                                        } else { 0 }
                                    } else { 0 };
                                    try_deliver(&p.procurement_active, inv_qty).map(|(reward, xp, item, qty)| {
                                        p.procurement_active = None;
                                        p.procurement_cooldown = crate::procurement::PROCUREMENT_COOLDOWN_SECS;
                                        p.inventory.take(item, qty);
                                        p.ether = p.ether.saturating_add(reward);
                                        p.masteries.gain_merchant(xp);
                                        tracing::info!(player = %p.name, reward, xp, "星際採購令交付完成");
                                        (p.name.clone(), reward, xp)
                                    })
                                }
                            } else { None }
                        };
                        if let Some((pname, reward, xp)) = result {
                            let _ = app.tx_chat.send(format!(
                                "📦 {} 完成星際採購令！獲得 {} 乙太 + {} 商人 XP！",
                                pname, reward, xp
                            ));
                            // 記入玩家事跡日誌（ROADMAP 67）。
                            app.player_logs.write().unwrap()
                                .entry(uid)
                                .or_default()
                                .push(format!("交付了跨星採購令，獲得 {} 乙太", reward));
                            let _ = pname;
                            let _ = xp;
                        }
                    }
                }

                Ok(ClientMsg::AbandonProcurement) => {
                    // 放棄採購任務：取消進行中任務，無懲罰（不啟動冷卻）。
                    if let Some(uid) = authed_uid {
                        if let Some(p) = app.players.write().unwrap().get_mut(&uid) {
                            if p.procurement_active.take().is_some() {
                                tracing::info!(player = %p.name, "放棄採購任務");
                            }
                        }
                    }
                }

                // ── 農產品展覽會（ROADMAP 56）──────────────────────────────────────
                Ok(ClientMsg::AcceptFairOrder { order_id }) => {
                    // 接取展覽委託：需登入 + 故鄉星球 + 靠近評審 + 無進行中委託 + 無冷卻。
                    if let Some(uid) = authed_uid {
                        if let Some(p) = app.players.write().unwrap().get_mut(&uid) {
                            use crate::farm_fair::{try_accept, is_near_fair_judge};
                            if p.planet == crate::state::PLANET_HOME
                                && app.is_near_npc(p.x, p.y, "farm_fair_npc")
                            {
                                if let Some(active) = try_accept(order_id, &p.farm_fair_active, p.farm_fair_cooldown) {
                                    let order_name = crate::farm_fair::find_order(order_id)
                                        .map(|o| o.name).unwrap_or("?");
                                    tracing::info!(player = %p.name, order = order_name, "接取農展委託");
                                    p.farm_fair_active = Some(active);
                                }
                            }
                        }
                    }
                }

                Ok(ClientMsg::SubmitFairOrder) => {
                    // 提交展覽委託：需登入 + 故鄉 + 靠近評審 + 有進行中委託 + 背包物品足夠。
                    if let Some(uid) = authed_uid {
                        let result = {
                            let mut players = app.players.write().unwrap();
                            if let Some(p) = players.get_mut(&uid) {
                                use crate::farm_fair::{try_submit, is_near_fair_judge};
                                if p.planet != crate::state::PLANET_HOME {
                                    None
                                } else if !app.is_near_npc(p.x, p.y, "farm_fair_npc") {
                                    None
                                } else {
                                    let inv = p.inventory.clone();
                                    let sub = try_submit(&p.farm_fair_active, |item| inv.count(item));
                                    if let Some((reward, xp, deductions)) = sub {
                                        // 先記錄委託名稱（active 清除前）
                                        let order_name = p.farm_fair_active.as_ref()
                                            .and_then(|a| crate::farm_fair::find_order(a.order_id))
                                            .map(|o| o.name)
                                            .unwrap_or("農展委託");
                                        for (item, qty) in &deductions {
                                            p.inventory.take(*item, *qty);
                                        }
                                        p.ether = p.ether.saturating_add(reward);
                                        p.masteries.gain_farmer(xp);
                                        p.farm_fair_active = None;
                                        p.farm_fair_cooldown = crate::farm_fair::FAIR_COOLDOWN_SECS;
                                        Some((p.name.clone(), reward, order_name))
                                    } else {
                                        None
                                    }
                                }
                            } else {
                                None
                            }
                        };
                        if let Some((pname, reward, order_name)) = result {
                            let _ = app.tx_chat.send(format!("🏅 {} 完成了{}！獲得 {} 乙太", pname, order_name, reward));
                            // 記入玩家事跡日誌（ROADMAP 67）。
                            app.player_logs.write().unwrap()
                                .entry(uid)
                                .or_default()
                                .push(format!("向評審卡特提交農展委託，獲得 {} 乙太", reward));
                            let _ = pname;
                        }
                    }
                }

                Ok(ClientMsg::AbandonFairOrder) => {
                    // 放棄展覽委託：取消進行中委託，無懲罰（不啟動冷卻）。
                    if let Some(uid) = authed_uid {
                        if let Some(p) = app.players.write().unwrap().get_mut(&uid) {
                            if p.farm_fair_active.take().is_some() {
                                tracing::info!(player = %p.name, "放棄農展委託");
                            }
                        }
                    }
                }

                // ── 會動腦的 NPC 對話（第一塊：會聊天、會記得你、會自己判斷要不要善待你）──
                Ok(ClientMsg::TalkToNpc { npc, text }) => {
                    let text: String = text.chars().take(300).collect(); // 輸入上限
                    if !text.trim().is_empty() {
                        // —— 城外旅人分支（ROADMAP 74）—— 旅人不走 NpcPersona 路線。
                        if npc == "traveler" {
                            let (is_near, traveler_name, traveler_origin, talk_count) = {
                                let players = app.players.read().unwrap();
                                let tv = app.traveler.read().unwrap();
                                let near = players.get(&id).map(|p| {
                                    if !tv.is_visible() { return false; }
                                    let dx = p.x - tv.x;
                                    let dy = p.y - tv.y;
                                    dx * dx + dy * dy <= crate::traveler_npc::TRAVELER_REACH * crate::traveler_npc::TRAVELER_REACH
                                }).unwrap_or(false);
                                let name = tv.name();
                                let origin = tv.origin();
                                let tc = tv.talk_count_for(id);
                                (near, name, origin, tc)
                            };
                            if !is_near {
                                continue;
                            }
                            // 冷卻：8 秒（同其他 NPC）。
                            let chat_key = (id, "traveler".to_string());
                            {
                                let mut last = app.npc_last_chat.write().unwrap();
                                let now = std::time::Instant::now();
                                if let Some(t) = last.get(&chat_key) {
                                    if t.elapsed().as_secs() < crate::npc_chat::PER_PLAYER_NPC_COOLDOWN_SECS {
                                        continue;
                                    }
                                }
                                last.insert(chat_key, now);
                            }
                            // 記錄對話次數。
                            app.traveler.write().unwrap().record_talk(id);
                            let player_text = text.clone();
                            let tx = tx_direct.clone();
                            let sem = app.npc_llm_sem.clone();
                            let name_s = traveler_name.to_string();
                            let origin_s = traveler_origin.to_string();
                            tokio::spawn(async move {
                                let _permit = tokio::time::timeout(
                                    std::time::Duration::from_secs(2),
                                    sem.acquire_owned(),
                                ).await.ok().and_then(|r| r.ok());
                                let reply = crate::npc_chat::reply_traveler(
                                    &name_s, &origin_s, talk_count, &player_text
                                ).await;
                                if let Ok(json) = serde_json::to_string(&crate::protocol::ServerMsg::NpcReply {
                                    npc: "traveler".to_string(),
                                    display: format!("🧳 {name_s}"),
                                    text: reply,
                                }) {
                                    let _ = tx.send(json).await;
                                }
                            });
                            continue;
                        }

                        // 驗證距離（ROADMAP 73）：必須靠近 NPC 才能交談。
                        let is_near = {
                            let players = app.players.read().unwrap();
                            players.get(&id).map(|p| app.is_near_npc(p.x, p.y, &npc)).unwrap_or(false)
                        };
                        if !is_near {
                            continue;
                        }

                        if let Some(persona) = crate::npc_chat::find_npc(&npc) {
                            // 每人每 NPC 冷卻：防單人狂送吃掉所有許可。
                            let chat_key = (id, npc.clone());
                            {
                                let mut last = app.npc_last_chat.write().unwrap();
                                let now = std::time::Instant::now();
                                if let Some(t) = last.get(&chat_key) {
                                    if t.elapsed().as_secs() < crate::npc_chat::PER_PLAYER_NPC_COOLDOWN_SECS {
                                        continue; // 冷卻中，靜默丟棄
                                    }
                                }
                                last.insert(chat_key, now);
                            }
                            let player_name = app
                                .players
                                .read()
                                .unwrap()
                                .get(&id)
                                .map(|p| p.name.clone())
                                .unwrap_or_default();
                            let key = (id, npc.clone());
                            // 讀關係、累積往來統計（talks 是「資料」，不是觸發規則）。
                            let rel = {
                                let mut mem = app.npc_memory.write().unwrap();
                                let r = mem.entry(key.clone()).or_default();
                                r.talks = r.talks.saturating_add(1);
                                r.clone()
                            };
                            // 世界近況（ROADMAP 65）：引擎事實，只有引擎能寫；空字串 = 無近況。
                            let world_news = app.world_log.read().unwrap().to_prompt_section();
                            // 玩家個人事跡（ROADMAP 67）：讀取當前玩家完成任務的引擎紀錄。
                            let player_activity = {
                                let logs = app.player_logs.read().unwrap();
                                logs.get(&id).map(|l| l.to_prompt_section()).unwrap_or_default()
                            };
                            // NPC 需求驅力（ROADMAP 69）：讀取此 NPC 目前的心情狀態，注入 prompt 影響語氣。
                            let needs_context = app.npc_needs.read().unwrap().to_prompt_section(&npc);
                            // NPC 人際關係網（ROADMAP 70）：讀取此 NPC 對其他居民的好惡，注入 prompt 讓談到彼此時語氣自然。
                            let relations_context = app.npc_relations.read().unwrap().to_prompt_section(&npc);
                            // NPC 派系自主湧現（ROADMAP 71）：讀取此 NPC 已公開的結盟/競爭關係，注入 prompt 讓口吻自然反映派系立場。
                            let faction_context = app.npc_factions.read().unwrap().to_prompt_section(&npc);

                            // NPC 生命週期（ROADMAP 66）：老年語境 + 繼承人首次登場語境 + 動態顯示名。
                            let (elder_context, heir_context_opt, lifecycle_display) = {
                                let mut lc = app.npc_lifecycle.write().unwrap();
                                let elder = lc.elder_context(persona.id);
                                let heir = lc.take_heir_context(persona.id);
                                let disp = lc.current_display(persona.id).to_string();
                                (elder, heir, disp)
                            };
                            // 合成完整老年語境：繼承人首次登場時注入「前任記憶」框架。
                            let full_elder_context = if let Some(heir) = heir_context_opt {
                                format!("\n\n【繼承記憶】{heir}{elder_context}")
                            } else {
                                elder_context.clone()
                            };
                            // 顯示名：若 lifecycle 有值則用動態名，否則 fallback 到靜態 persona.display。
                            let display_name = if lifecycle_display.is_empty() {
                                persona.display.to_string()
                            } else {
                                lifecycle_display
                            };

                            // ── 里長：特殊路徑（村落金庫 + 活動暗號，ROADMAP 64）────
                            if persona.id == "village_chief" {
                                let treasury = *app.village_treasury.read().unwrap();
                                // 將生命週期老年語境、需求驅力、人際關係網注入到 chief_prompt 末尾。
                                let chief_prompt = {
                                    let base = crate::village_chief::system_prompt(&rel, treasury, &world_news, &player_activity);
                                    let with_elder = if full_elder_context.is_empty() { base } else { format!("{base}{full_elder_context}") };
                                    let with_needs = if needs_context.is_empty() { with_elder } else { format!("{with_elder}{needs_context}") };
                                    let with_rel = if relations_context.is_empty() { with_needs } else { format!("{with_needs}{relations_context}") };
                                    if faction_context.is_empty() { with_rel } else { format!("{with_rel}{faction_context}") }
                                };
                                let display_name_chief = display_name.clone();
                                let tx = tx_direct.clone();
                                let app2 = app.clone();
                                let sem = app.npc_llm_sem.clone();
                                tokio::spawn(async move {
                                    let _permit = match tokio::time::timeout(
                                        std::time::Duration::from_secs(2),
                                        sem.acquire_owned(),
                                    ).await {
                                        Ok(Ok(p)) => p,
                                        _ => {
                                            if let Ok(json) = serde_json::to_string(
                                                &crate::protocol::ServerMsg::NpcReply {
                                                    npc: persona.id.to_string(),
                                                    display: display_name_chief.clone(),
                                                    text: crate::village_chief::canned_reply(),
                                                },
                                            ) {
                                                let _ = tx.send(json).await;
                                            }
                                            return;
                                        }
                                    };
                                    let raw = crate::npc_chat::reply_with_custom_prompt(persona, &chief_prompt, &text).await;
                                    // 里長的「手」：偵測活動暗號，引擎原子扣減金庫。
                                    let (wants_event, clean) = crate::village_chief::extract_event_decision(&raw);
                                    let event_triggered = if wants_event {
                                        let new_treasury = {
                                            let mut t = app2.village_treasury.write().unwrap();
                                            if let Some(after) = crate::village_chief::spend_on_event(*t) {
                                                *t = after;
                                                Some(after)
                                            } else {
                                                None // 金庫在並發中被用完，本次作罷
                                            }
                                        };
                                        if let Some(new_t) = new_treasury {
                                            // 節慶加成開始計時。
                                            {
                                                let expiry = std::time::Instant::now()
                                                    + std::time::Duration::from_secs(crate::village_chief::EVENT_DURATION_SECS);
                                                *app2.village_buff_until.write().unwrap() = Some(expiry);
                                            }
                                            // 廣播全服公告。
                                            let msg = crate::protocol::ServerMsg::VillageEvent {
                                                message: "🎉 凱爾長老宣布舉辦村落節慶！未來 10 分鐘全服殺怪/採集 EXP +30%！".to_string(),
                                                duration_secs: crate::village_chief::EVENT_DURATION_SECS,
                                                new_treasury: new_t,
                                            };
                                            if let Ok(json) = serde_json::to_string(&msg) {
                                                let _ = app2.tx_chat.send(json);
                                            }
                                            // 世界事件記憶（ROADMAP 65）：節慶是全服大事，NPC 應知道。
                                            app2.world_log.write().unwrap().push(
                                                "凱爾長老動用村落金庫舉辦村落節慶，全服 EXP +30%（持續 10 分鐘）"
                                            );
                                            // NPC 需求驅力（ROADMAP 69）：節慶 → 歸屬感大升，商人繁榮感也大升。
                                            app2.npc_needs.write().unwrap().apply_world_event(crate::npc_needs::NeedsEvent::VillageFestival);
                                            // NPC 人際關係網（ROADMAP 70）：節慶帶動全村和睦。
                                            app2.npc_relations.write().unwrap().apply_world_event(crate::npc_relations::RelationsEvent::VillageFestival);
                                            // NPC 主動評論（ROADMAP 68）：節慶開始，NPC 熱鬧回應。
                                            {
                                                let app3 = app2.clone();
                                                tokio::spawn(async move {
                                                    let now = std::time::Instant::now();
                                                    let event_kind = crate::npc_proactive::WorldEventKind::VillageFestival;
                                                    let maybe_npc = {
                                                        let mut cd = app3.npc_proactive.write().unwrap();
                                                        crate::npc_proactive::pick_reacting_npc(&event_kind, &mut cd, now)
                                                    };
                                                    if let Some(npc_id) = maybe_npc {
                                                        let reaction = crate::npc_proactive::generate_proactive_reaction(npc_id, event_kind).await;
                                                        let _ = app3.tx_chat.send(reaction);
                                                    }
                                                });
                                            }
                                            tracing::info!(player = %player_name, new_treasury = new_t, "里長自主辦村落節慶，金庫扣減");
                                            true
                                        } else {
                                            false
                                        }
                                    } else {
                                        false
                                    };
                                    let _ = event_triggered; // 已處理
                                    if let Ok(json) = serde_json::to_string(
                                        &crate::protocol::ServerMsg::NpcReply {
                                            npc: persona.id.to_string(),
                                            display: display_name_chief.clone(),
                                            text: clean.clone(),
                                        },
                                    ) {
                                        let _ = tx.send(json).await;
                                    }
                                    // 更新印象。
                                    let new_imp = crate::npc_chat::update_impression(
                                        persona, &rel.impression, &text, &clean,
                                    ).await;
                                    let updated_rel = {
                                        let mut mem = app2.npc_memory.write().unwrap();
                                        let r = mem.entry(key.clone()).or_default();
                                        r.impression = new_imp;
                                        r.clone()
                                    };
                                    app2.npc_memory_store.save_rel(key.0, key.1, updated_rel);
                                    tracing::info!(player = %player_name, "里長對話");
                                });
                                continue; // 跳過一般 NPC 路徑
                            }

                            // NPC 自己有限的餘裕（送完就沒了＝真實稀缺）。
                            let stock = app
                                .npc_gift_stock
                                .read()
                                .unwrap()
                                .get(persona.id)
                                .copied()
                                .unwrap_or(0);
                            // 送禮選項：這位玩家還沒收過 且 NPC 手邊還有餘裕。
                            let gift_available = !rel.gifted && stock > 0;
                            // 非同步：呼叫地端 LLM 要數秒，絕不能卡住 15Hz 迴圈。
                            let tx = tx_direct.clone();
                            let app2 = app.clone();
                            let sem = app.npc_llm_sem.clone();
                            let display_name_npc = display_name.clone();
                            tokio::spawn(async move {
                                // 等全域並發許可（上限 MAX_CONCURRENT_LLM）。
                                // 等超 2 秒仍拿不到 → 回罐頭句，避免佇列無限堆積。
                                let _permit = match tokio::time::timeout(
                                    std::time::Duration::from_secs(2),
                                    sem.acquire_owned(),
                                ).await {
                                    Ok(Ok(p)) => p,
                                    _ => {
                                        // LLM 太忙，回罐頭讓玩家感知最小（不要噴錯誤）。
                                        if let Ok(json) = serde_json::to_string(
                                            &crate::protocol::ServerMsg::NpcReply {
                                                npc: persona.id.to_string(),
                                                display: display_name_npc.clone(),
                                                text: crate::npc_chat::canned_reply(persona),
                                            },
                                        ) {
                                            let _ = tx.send(json).await;
                                        }
                                        return;
                                    }
                                };
                                let raw = crate::npc_chat::reply(persona, &rel, gift_available, stock, &text, &world_news, &full_elder_context, &player_activity, &needs_context, &relations_context, &faction_context).await;
                                // NPC 自己決定的送禮（暗號）。引擎原子扣減餘裕：送完就真的沒了（手有界＋稀缺）。
                                let (wants_gift, after_gift) = crate::npc_chat::extract_gift_decision(&raw);
                                // 熟客折扣（ROADMAP 63）：商人自主決定是否給下次購買打折。
                                // 只有商人 NPC 才有折扣選項（其他工職 NPC 沒有售價可讓利）。
                                let (wants_discount, after_discount) = if persona.id == "merchant" {
                                    crate::npc_chat::extract_discount_decision(&after_gift)
                                } else {
                                    (false, after_gift)
                                };
                                // AI 議價（ROADMAP 101）：只有故鄉商人才有議價能力。
                                // 引擎解析 [DEAL item qty price]，驗合法後存入 PendingDeal 並送 DealOffer；
                                // 驗證失敗（天文數字/不明物品/金庫不足）靜默忽略，不打斷對話。
                                let (wants_deal, clean) = if persona.id == "merchant" {
                                    let (parsed, clean2) = crate::npc_deal::extract_deal(&after_discount);
                                    if let Some((item_str, qty, price_per)) = parsed {
                                        let treasury_balance = app2.npc_treasury.read().unwrap()
                                            .balance(crate::npc_treasury::MERCHANT_HOME);
                                        match crate::npc_deal::validate_deal(&item_str, qty, price_per, treasury_balance) {
                                            Ok(pending) => {
                                                let item_display = crate::npc_deal::item_display_zh(pending.item).to_string();
                                                let deal_total = pending.total();
                                                let deal_qty = pending.qty;
                                                let deal_price = pending.price_per;
                                                app2.npc_pending_deal.write().unwrap().insert(id, pending);
                                                let offer = crate::protocol::ServerMsg::DealOffer {
                                                    npc: persona.id.to_string(),
                                                    display: display_name_npc.clone(),
                                                    item_display,
                                                    qty: deal_qty,
                                                    price_per: deal_price,
                                                    total: deal_total,
                                                };
                                                if let Ok(json) = serde_json::to_string(&offer) {
                                                    let _ = tx.send(json).await;
                                                }
                                                tracing::debug!(item_str, qty, price_per, "商人提出議價");
                                                (true, clean2)
                                            }
                                            Err(reason) => {
                                                tracing::debug!(reason, "商人議價驗證失敗，靜默忽略");
                                                (false, clean2)
                                            }
                                        }
                                    } else {
                                        (false, clean2)
                                    }
                                } else {
                                    (false, after_discount)
                                };
                                let _ = wants_deal; // 已處理
                                let granted = if gift_available && wants_gift {
                                    let new_stock = {
                                        let mut stk = app2.npc_gift_stock.write().unwrap();
                                        let s = stk.entry(persona.id.to_string()).or_insert(0);
                                        if *s > 0 {
                                            *s -= 1;
                                            Some(*s)
                                        } else {
                                            None // 餘裕剛好被別人用完了
                                        }
                                    };
                                    if let Some(s) = new_stock {
                                        // 餘裕扣減後立刻持久化（fire-and-forget）。
                                        app2.npc_memory_store.save_gift_stock(persona.id.to_string(), s);
                                        true
                                    } else {
                                        false
                                    }
                                } else {
                                    false
                                };
                                if granted {
                                    if let Some(p) = app2.players.write().unwrap().get_mut(&id) {
                                        p.add_item_overflow(crate::npc_chat::GIFT_ITEM, crate::npc_chat::GIFT_QTY);
                                    }
                                    tracing::info!(player = %player_name, npc = persona.id, "NPC 自主送了熟客小禮（餘裕扣減）");
                                }
                                // 折扣存入：商人決定打折 → 存入待用票（限時 DISCOUNT_DURATION_SECS 秒）。
                                // 下次 ShopBuy 套用一次後清除。每人限一張，舊票被新票覆蓋（取最新惠）。
                                if wants_discount {
                                    let expiry = std::time::Instant::now()
                                        + std::time::Duration::from_secs(crate::npc_chat::DISCOUNT_DURATION_SECS);
                                    app2.npc_pending_discount.write().unwrap()
                                        .insert(id, (crate::npc_chat::DISCOUNT_PERCENT, expiry));
                                    tracing::info!(player = %player_name, discount = crate::npc_chat::DISCOUNT_PERCENT, "商人自主給出熟客折扣票");
                                }
                                if let Ok(json) = serde_json::to_string(
                                    &crate::protocol::ServerMsg::NpcReply {
                                        npc: persona.id.to_string(),
                                        display: display_name_npc.clone(),
                                        text: clean.clone(),
                                    },
                                ) {
                                    let _ = tx.send(json).await; // 單播回該玩家
                                }
                                // 對話後更新印象 + 落定送禮狀態（隔離：只影響 NPC 對這位玩家）。
                                let new_imp = crate::npc_chat::update_impression(
                                    persona, &rel.impression, &text, &clean,
                                )
                                .await;
                                let updated_rel = {
                                    let mut mem = app2.npc_memory.write().unwrap();
                                    let r = mem.entry(key.clone()).or_default();
                                    r.impression = new_imp;
                                    if granted {
                                        r.gifted = true;
                                    }
                                    r.clone()
                                };
                                // 對話後立刻持久化關係狀態（fire-and-forget）。
                                app2.npc_memory_store.save_rel(key.0, key.1, updated_rel);
                                tracing::info!(player = %player_name, npc = persona.id, "NPC 對話");
                            });
                        }
                    }
                }

                // ── 里長 NPC：村落金庫捐獻（ROADMAP 64）───────────────────────────
                Ok(ClientMsg::DonateToVillage) => {
                    if let Some(uid) = authed_uid {
                        let player_name = app.players.read().unwrap()
                            .get(&uid).map(|p| p.name.clone()).unwrap_or_default();
                        let (pos_x, pos_y) = app.players.read().unwrap()
                            .get(&uid).map(|p| (p.x, p.y)).unwrap_or((0.0, 0.0));
                        // 必須在里長互動範圍內。
                        if !crate::village_chief::is_within_reach(pos_x, pos_y) {
                            // 不在範圍內，靜默忽略（前端應先確認）。
                        } else {
                            let amount = crate::village_chief::DONATE_AMOUNT;
                            let new_treasury = {
                                let mut players = app.players.write().unwrap();
                                if let Some(p) = players.get_mut(&uid) {
                                    if p.ether >= amount {
                                        p.ether -= amount;
                                        let mut t = app.village_treasury.write().unwrap();
                                        *t = crate::village_chief::donate_to_treasury(*t, amount);
                                        Some(*t)
                                    } else {
                                        None // 乙太不足
                                    }
                                } else {
                                    None
                                }
                            };
                            if let Some(new_t) = new_treasury {
                                let _ = app.tx_chat.send(format!(
                                    "💛 {} 向村落金庫捐獻了 {} 乙太（金庫：{} 乙太）",
                                    player_name, amount, new_t
                                ));
                                tracing::info!(player = %player_name, amount, new_treasury = new_t, "玩家捐獻村落金庫");
                            }
                        }
                    }
                }

                // ── 城鎮大工程：捐獻（ROADMAP 131）───────────────────────────
                Ok(ClientMsg::DonateToProject { item, qty }) => {
                    if let Some(uid) = authed_uid {
                        let (player_name, pos_x, pos_y, downed) = {
                            let players = app.players.read().unwrap();
                            players.get(&uid).map(|p| (p.name.clone(), p.x, p.y, p.vitals.is_downed()))
                                .unwrap_or_else(|| ("".into(), 0.0, 0.0, true))
                        };

                        // 必須靠近里長（工程發起人）。
                        if !downed && crate::village_chief::is_within_reach(pos_x, pos_y) && qty > 0 {
                            let mut actual_qty = 0;

                            // 1. 扣除玩家資源
                            {
                                let mut players = app.players.write().unwrap();
                                if let Some(p) = players.get_mut(&uid) {
                                    match item {
                                        None => { // 捐乙太
                                            actual_qty = p.ether.min(qty);
                                            p.ether -= actual_qty;
                                        }
                                        Some(kind) => {
                                            if p.inventory.count(kind) >= qty {
                                                p.inventory.take(kind, qty);
                                                actual_qty = qty;
                                            }
                                        }
                                    }
                                }
                            }

                            if actual_qty > 0 {
                                // 2. 更新工程進度
                                let (score, taken_qty, is_completed, project_id, project_name) = {
                                    let mut project = app.town_project.write().unwrap();
                                    let (s, t) = project.donate(item, actual_qty);
                                    (s, t, project.status == crate::town_project::TownProjectStatus::Completed, project.project_id.clone(), project.name.clone())
                                };

                                if score > 0 {
                                    // 3. 紀錄並持久化
                                    let (ether, wood, stone, crystal) = match item {
                                        None => (taken_qty, 0, 0, 0),
                                        Some(crate::inventory::ItemKind::Wood) => (0, taken_qty, 0, 0),
                                        Some(crate::inventory::ItemKind::Stone) => (0, 0, taken_qty, 0),
                                        Some(crate::inventory::ItemKind::CrystalShard) | Some(crate::inventory::ItemKind::StarCrystalShard) => (0, 0, 0, taken_qty),
                                        _ => (0, 0, 0, 0),
                                    };
                                    app.town_project_store.save_donation(uid, project_id.clone(), ether, wood, stone, crystal, score);
                                    app.town_project_store.save_progress(app.town_project.read().unwrap().clone());

                                    // 退還溢出部分
                                    if taken_qty < actual_qty {
                                        let mut players = app.players.write().unwrap();
                                        if let Some(p) = players.get_mut(&uid) {
                                            match item {
                                                // saturating:防 taken>actual 反向 wrap 印鈔、防 u32 上限 wrap 歸零。
                                                None => p.ether = p.ether.saturating_add(actual_qty.saturating_sub(taken_qty)),
                                                Some(kind) => { p.inventory.add(kind, actual_qty - taken_qty); }
                                            }
                                        }
                                    }

                                    // 4. 廣播
                                    let item_name = match item {
                                        None => "乙太".to_string(),
                                        Some(k) => format!("{:?}", k),
                                    };
                                    let _ = app.tx_chat.send(format!("🏗️ {} 為【{}】工程捐獻了 {} {}！", player_name, project_name, taken_qty, item_name));
                                    
                                    if is_completed {
                                        let _ = app.tx_chat.send(format!("🎊 慶賀！【{}】工程已圓滿完工！城鎮的未來更加閃耀 ✨", project_name));
                                        // 記錄世界大事
                                        app.world_log.write().unwrap().push(format!("【{}】大工程順利完工！", project_name));
                                        // 城鎮記憶石（ROADMAP 157）：大工程完工是城鎮歷史的重要一頁。
                                        app.town_memory.write().unwrap().push_event(
                                            "🏗️",
                                            format!("城鎮大工程完工——【{}】正式落成！", project_name),
                                        );
                                    }
                                } else {
                                    // 資源不合或已滿，全部退回
                                    let mut players = app.players.write().unwrap();
                                    if let Some(p) = players.get_mut(&uid) {
                                        match item {
                                            None => p.ether += actual_qty,
                                            Some(kind) => { p.inventory.add(kind, actual_qty); }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // ── 公會系統（ROADMAP 29）──────────────────────────────────────────
                Ok(ClientMsg::CreateGuild { name, tag }) => {
                    // 建立公會：需登入 + 乙太 ≥ 50；成功後從玩家扣乙太、更新 guild_tag。
                    if let Some(uid) = authed_uid {
                        let result = {
                            let ether = app.players.read().unwrap().get(&uid).map(|p| p.ether).unwrap_or(0);
                            if ether < crate::guild::GUILD_CREATE_COST {
                                Err(format!("乙太不足（建立公會需要 {} 乙太）", crate::guild::GUILD_CREATE_COST))
                            } else {
                                app.guilds.create(uid, name, tag)
                            }
                        };
                        match result {
                            Ok(gid) => {
                                let guild_tag = app.guilds.tag_of(uid);
                                // 扣乙太，更新 guild_tag；成就：建立公會=加入公會（ROADMAP 31）。
                                let (is_new_ach, pname) = {
                                    let mut players = app.players.write().unwrap();
                                    if let Some(p) = players.get_mut(&uid) {
                                        p.ether = p.ether.saturating_sub(crate::guild::GUILD_CREATE_COST);
                                        p.guild_tag = guild_tag.clone();
                                        let new = p.achievements.unlock(crate::achievement::Achievement::GuildMember);
                                        (new, p.name.clone())
                                    } else {
                                        (false, String::new())
                                    }
                                };
                                if is_new_ach {
                                    let _ = app.tx_chat.send(format!(
                                        "🏆 {} 解鎖成就「{}」！",
                                        pname,
                                        crate::achievement::Achievement::GuildMember.display_name()
                                    ));
                                }
                                // 回傳公會詳情給本人。
                                let view = build_guild_view(&app, uid, gid);
                                let msg = ServerMsg::GuildUpdate { guild: view };
                                if let Ok(json) = serde_json::to_string(&msg) {
                                    let _ = tx_direct.try_send(json);
                                }
                                tracing::info!(player = %id, ?gid, "建立公會");
                            }
                            Err(e) => {
                                // 錯誤訊息以聊天方式通知（不增新訊息型別）。
                                let msg = ServerMsg::Chat { from: "系統".into(), text: format!("⚠️ {e}") };
                                if let Ok(json) = serde_json::to_string(&msg) {
                                    let _ = tx_direct.try_send(json);
                                }
                            }
                        }
                    }
                }
                Ok(ClientMsg::JoinGuild { guild_id }) => {
                    // 加入公會：需登入；公會不存在 / 已滿 / 已有公會時回錯誤訊息。
                    if let Some(uid) = authed_uid {
                        let result = app.guilds.join(guild_id, uid);
                        match result {
                            Ok(()) => {
                                let guild_tag = app.guilds.tag_of(uid);
                                // 成就：加入公會（ROADMAP 31）。
                                let (is_new_ach, pname) = {
                                    let mut players = app.players.write().unwrap();
                                    if let Some(p) = players.get_mut(&uid) {
                                        p.guild_tag = guild_tag;
                                        let new = p.achievements.unlock(crate::achievement::Achievement::GuildMember);
                                        (new, p.name.clone())
                                    } else {
                                        (false, String::new())
                                    }
                                };
                                if is_new_ach {
                                    let _ = app.tx_chat.send(format!(
                                        "🏆 {} 解鎖成就「{}」！",
                                        pname,
                                        crate::achievement::Achievement::GuildMember.display_name()
                                    ));
                                }
                                let view = build_guild_view(&app, uid, guild_id);
                                let msg = ServerMsg::GuildUpdate { guild: view };
                                if let Ok(json) = serde_json::to_string(&msg) {
                                    let _ = tx_direct.try_send(json);
                                }
                            }
                            Err(e) => {
                                let msg = ServerMsg::Chat { from: "系統".into(), text: format!("⚠️ {e}") };
                                if let Ok(json) = serde_json::to_string(&msg) {
                                    let _ = tx_direct.try_send(json);
                                }
                            }
                        }
                    }
                }
                Ok(ClientMsg::LeaveGuild) => {
                    // 離開公會：需登入；若是最後成員公會自動解散。
                    if let Some(uid) = authed_uid {
                        let result = app.guilds.leave(uid);
                        if result.is_ok() {
                            if let Some(p) = app.players.write().unwrap().get_mut(&uid) {
                                p.guild_tag = None;
                            }
                        }
                        let msg = ServerMsg::GuildUpdate { guild: None };
                        if let Ok(json) = serde_json::to_string(&msg) {
                            let _ = tx_direct.try_send(json);
                        }
                    }
                }
                Ok(ClientMsg::DonateToGuild { amount }) => {
                    // 向公會捐贈乙太：需登入 + 在公會 + 乙太足夠。
                    if let Some(uid) = authed_uid {
                        let ether = app.players.read().unwrap().get(&uid).map(|p| p.ether).unwrap_or(0);
                        if amount == 0 || ether < amount {
                            let text = if amount == 0 {
                                "捐贈金額需大於 0".into()
                            } else {
                                format!("乙太不足（捐贈 {} 乙太，但你只有 {} 乙太）", amount, ether)
                            };
                            let msg = ServerMsg::Chat { from: "系統".into(), text: format!("⚠️ {}", text) };
                            if let Ok(json) = serde_json::to_string(&msg) {
                                let _ = tx_direct.try_send(json);
                            }
                        } else {
                            let result = app.guilds.donate(uid, amount);
                            match result {
                                Ok(_new_treasury) => {
                                    if let Some(p) = app.players.write().unwrap().get_mut(&uid) {
                                        p.ether = p.ether.saturating_sub(amount);
                                    }
                                    let gid = app.guilds.guild_of(uid);
                                    let view = gid.and_then(|gid| build_guild_view(&app, uid, gid));
                                    let msg = ServerMsg::GuildUpdate { guild: view };
                                    if let Ok(json) = serde_json::to_string(&msg) {
                                        let _ = tx_direct.try_send(json);
                                    }
                                }
                                Err(e) => {
                                    let msg = ServerMsg::Chat { from: "系統".into(), text: format!("⚠️ {e}") };
                                    if let Ok(json) = serde_json::to_string(&msg) {
                                        let _ = tx_direct.try_send(json);
                                    }
                                }
                            }
                        }
                    }
                }
                Ok(ClientMsg::RequestGuildList) => {
                    // 傳回全部公會簡介給請求者。
                    let briefs: Vec<crate::protocol::GuildBrief> = app.guilds.brief_list()
                        .into_iter()
                        .map(|b| crate::protocol::GuildBrief {
                            id: b.id,
                            name: b.name,
                            tag: b.tag,
                            member_count: b.member_count,
                            treasury: b.treasury,
                        })
                        .collect();
                    let msg = ServerMsg::GuildList { guilds: briefs };
                    if let Ok(json) = serde_json::to_string(&msg) {
                        let _ = tx_direct.try_send(json);
                    }
                }
                // ── 公會系統 end ───────────────────────────────────────────────

                // ── 每日任務系統（ROADMAP 32）────────────────────────────────────
                Ok(ClientMsg::RequestDailyQuests) => {
                    if let Some(uid) = authed_uid {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs();
                        let seed = uid.as_u128() as u64;
                        let mut dq = app.daily_quests.write().unwrap();
                        let state = dq.entry(uid).or_insert_with(|| {
                            crate::daily_quest::PlayerDailyState::new(seed, now)
                        });
                        state.check_reset(now, seed);
                        let views: Vec<_> = state.tasks.iter().map(|t| t.to_view()).collect();
                        let done = state.done_count() as u32;
                        drop(dq);
                        let msg = ServerMsg::DailyQuestsUpdate { tasks: views, done_count: done };
                        if let Ok(json) = serde_json::to_string(&msg) {
                            let _ = tx_direct.try_send(json);
                        }
                    }
                }
                // ── 每日任務系統 end ─────────────────────────────────────────────

                // ── 排行榜系統（ROADMAP 33）──────────────────────────────────────
                Ok(ClientMsg::RequestLeaderboard) => {
                    let level_top = app.positions.leaderboard_top_level(20).await;
                    let ether_top = app.positions.leaderboard_top_ether(20).await;

                    // 等級/乙太：Postgres 模式已含離線玩家；記憶體模式以線上玩家補底。
                    let level_top = if level_top.is_empty() {
                        let players = app.players.read().unwrap();
                        let mut v: Vec<(String, u32)> = players.values()
                            .map(|p| (p.name.clone(), p.level()))
                            .collect();
                        v.sort_by(|a, b| b.1.cmp(&a.1));
                        v.truncate(20);
                        v
                    } else { level_top };
                    let ether_top = if ether_top.is_empty() {
                        let players = app.players.read().unwrap();
                        let mut v: Vec<(String, u32)> = players.values()
                            .map(|p| (p.name.clone(), p.ether))
                            .collect();
                        v.sort_by(|a, b| b.1.cmp(&a.1));
                        v.truncate(20);
                        v
                    } else { ether_top };

                    // 殺怪榜：線上玩家即時數（kill_count 不持久化）。
                    let kills_top: Vec<(String, u32)> = {
                        let players = app.players.read().unwrap();
                        let mut v: Vec<(String, u32)> = players.values()
                            .map(|p| (p.name.clone(), p.kill_count))
                            .collect();
                        v.sort_by(|a, b| b.1.cmp(&a.1));
                        v.truncate(20);
                        v
                    };

                    let to_entries = |v: Vec<(String, u32)>| -> Vec<crate::protocol::LeaderboardEntry> {
                        v.into_iter().enumerate().map(|(i, (name, value))| {
                            crate::protocol::LeaderboardEntry { rank: (i + 1) as u32, name, value }
                        }).collect()
                    };

                    let msg = ServerMsg::Leaderboard {
                        level_top: to_entries(level_top),
                        ether_top: to_entries(ether_top),
                        kills_top: to_entries(kills_top),
                    };
                    if let Ok(json) = serde_json::to_string(&msg) {
                        let _ = tx_direct.try_send(json);
                    }
                }
                // ── 排行榜系統 end ───────────────────────────────────────────────

                Ok(ClientMsg::BuyLandPlot { plot_id, purpose }) => {
                    // ROADMAP 35：購買城外地塊（含用途）。需：已登入、乙太足夠、地塊可購、自己尚無地塊。
                    let Some(uid) = authed_uid else { continue; };
                    // 解析用途（未帶預設 FreeBuild）
                    let plot_purpose = purpose.as_deref()
                        .map(crate::land_plot::PlotPurpose::from_str)
                        .unwrap_or(crate::land_plot::PlotPurpose::FreeBuild);
                    // 一次讀鎖取乙太
                    let ether = app.players.read().unwrap().get(&uid).map(|p| p.ether);
                    let Some(ether) = ether else { continue; };
                    if ether < crate::land_plot::LAND_PLOT_COST { continue; }
                    // 嘗試登記產權（LandPlotRegistry 內部驗地塊合法、未售、玩家限一塊）。
                    let ok = app.land_plots.write().unwrap().buy(plot_id, uid, plot_purpose);
                    if !ok { continue; }
                    // 扣乙太
                    if let Some(p) = app.players.write().unwrap().get_mut(&uid) {
                        p.ether = p.ether.saturating_sub(crate::land_plot::LAND_PLOT_COST);
                    }
                    // 持久化（fire-and-forget）
                    app.land_plot_store.save_purchase(plot_id, uid, plot_purpose);
                    tracing::info!(%uid, plot_id, ?plot_purpose, "玩家購買城外地塊");
                }
                // ── 城外地塊購買 end ─────────────────────────────────────────────

                // ── 好友系統（ROADMAP 96）───────────────────────────────────────
                Ok(ClientMsg::AddFriend { name }) => {
                    let Some(uid) = authed_uid else { continue; };
                    // 找目標帳號（線上或離線都可以，依名字查 UserStore）。
                    let target = app.users.find_by_name(&name);
                    let Some(target) = target else {
                        let err = ServerMsg::Chat {
                            from: "系統".into(),
                            text: format!("找不到玩家「{name}」"),
                        };
                        if let Ok(json) = serde_json::to_string(&err) {
                            let _ = tx_direct.try_send(json);
                        }
                        continue;
                    };
                    if target.id == uid {
                        let err = ServerMsg::Chat {
                            from: "系統".into(),
                            text: "不能加自己為好友哦".into(),
                        };
                        if let Ok(json) = serde_json::to_string(&err) {
                            let _ = tx_direct.try_send(json);
                        }
                        continue;
                    }
                    let added = app.friends.add(uid, target.id);
                    if added {
                        tracing::info!(%uid, friend_id=%target.id, name, "加好友");
                    }
                    // 不管是否已存在，回傳最新清單。
                    let msg = build_friend_list_msg(&app, uid);
                    if let Ok(json) = serde_json::to_string(&msg) {
                        let _ = tx_direct.try_send(json);
                    }
                }
                Ok(ClientMsg::RemoveFriend { name }) => {
                    let Some(uid) = authed_uid else { continue; };
                    let target = app.users.find_by_name(&name);
                    let Some(target) = target else { continue; };
                    app.friends.remove(uid, target.id);
                    tracing::info!(%uid, friend_id=%target.id, name, "刪好友");
                    let msg = build_friend_list_msg(&app, uid);
                    if let Ok(json) = serde_json::to_string(&msg) {
                        let _ = tx_direct.try_send(json);
                    }
                }
                Ok(ClientMsg::RequestFriendList) => {
                    let Some(uid) = authed_uid else { continue; };
                    let msg = build_friend_list_msg(&app, uid);
                    if let Ok(json) = serde_json::to_string(&msg) {
                        let _ = tx_direct.try_send(json);
                    }
                }
                // ── 好友系統 end ─────────────────────────────────────────────────

                // ── 隊伍系統（ROADMAP 97）──────────────────────────────────────
                Ok(ClientMsg::InviteToParty { name }) => {
                    let Some(uid) = authed_uid else { continue; };
                    // 查目標玩家（線上即可，離線不接受邀請）。
                    let target_id = {
                        let ps = app.players.read().unwrap();
                        ps.values().find(|p| p.name == name).map(|p| p.id)
                    };
                    let Some(target_id) = target_id else {
                        let err = ServerMsg::Chat { from: "系統".into(), text: format!("找不到在線玩家「{name}」") };
                        if let Ok(j) = serde_json::to_string(&err) { let _ = tx_direct.try_send(j); }
                        continue;
                    };
                    if target_id == uid {
                        let err = ServerMsg::Chat { from: "系統".into(), text: "不能邀請自己哦".into() };
                        if let Ok(j) = serde_json::to_string(&err) { let _ = tx_direct.try_send(j); }
                        continue;
                    }
                    // 建立（或取得自己的現有）隊伍。
                    let my_party_id = {
                        app.parties.party_of(uid).unwrap_or_else(|| {
                            let pid = app.parties.create(uid);
                            // 同步到 Player 結構
                            if let Some(p) = app.players.write().unwrap().get_mut(&uid) {
                                p.party_id = Some(pid);
                            }
                            pid
                        })
                    };
                    // 邀請目標。
                    match app.parties.invite(my_party_id, target_id) {
                        None => {
                            let err = ServerMsg::Chat { from: "系統".into(), text: format!("「{name}」已在隊伍中，無法邀請") };
                            if let Ok(j) = serde_json::to_string(&err) { let _ = tx_direct.try_send(j); }
                        }
                        Some(_) => {
                            let my_name = app.players.read().unwrap()
                                .get(&uid).map(|p| p.name.clone()).unwrap_or_default();
                            let invite_msg = ServerMsg::PartyInvite { from_name: my_name };
                            if let Ok(j) = serde_json::to_string(&invite_msg) {
                                let senders = app.whisper_senders.read().unwrap();
                                if let Some(tx) = senders.get(&target_id) {
                                    let _ = tx.try_send(j);
                                }
                            }
                            let ok = ServerMsg::Chat { from: "系統".into(), text: format!("已傳送隊伍邀請給「{name}」") };
                            if let Ok(j) = serde_json::to_string(&ok) { let _ = tx_direct.try_send(j); }
                        }
                    }
                }
                Ok(ClientMsg::JoinParty) => {
                    let Some(uid) = authed_uid else { continue; };
                    if let Some((pid, leader_id, members)) = app.parties.accept_invite(uid) {
                        // 同步所有新成員的 party_id
                        {
                            let mut ps = app.players.write().unwrap();
                            for &m in &members {
                                if let Some(p) = ps.get_mut(&m) { p.party_id = Some(pid); }
                            }
                        }
                        broadcast_party_update(&app, pid, &members, leader_id);
                        tracing::info!(%uid, %pid, "加入隊伍");
                    } else {
                        let err = ServerMsg::Chat { from: "系統".into(), text: "目前沒有待處理的隊伍邀請".into() };
                        if let Ok(j) = serde_json::to_string(&err) { let _ = tx_direct.try_send(j); }
                    }
                }
                Ok(ClientMsg::DeclineParty) => {
                    let Some(uid) = authed_uid else { continue; };
                    app.parties.decline_invite(uid);
                }
                Ok(ClientMsg::LeaveParty) => {
                    let Some(uid) = authed_uid else { continue; };
                    if let Some((disbanded, remaining)) = app.parties.leave(uid) {
                        // 清除自己的 party_id
                        if let Some(p) = app.players.write().unwrap().get_mut(&uid) {
                            p.party_id = None;
                        }
                        if disbanded {
                            // 通知所有前成員解散
                            {
                                let mut ps = app.players.write().unwrap();
                                for &m in &remaining {
                                    if let Some(p) = ps.get_mut(&m) { p.party_id = None; }
                                }
                            }
                            let msg = ServerMsg::PartyDisbanded;
                            let senders = app.whisper_senders.read().unwrap();
                            for m in &remaining {
                                if let Some(tx) = senders.get(m) {
                                    if let Ok(j) = serde_json::to_string(&msg) { let _ = tx.try_send(j); }
                                }
                            }
                            // 自己也收到解散通知
                            if let Ok(j) = serde_json::to_string(&msg) { let _ = tx_direct.try_send(j); }
                        } else {
                            // 非解散：通知剩餘成員更新列表；通知自己已離隊。
                            if let Some(&first) = remaining.first() {
                                if let Some(pid) = app.parties.party_of(first) {
                                    let leader_id = app.parties.leader_of(pid).unwrap_or_default();
                                    broadcast_party_update(&app, pid, &remaining, leader_id);
                                }
                            }
                            // 告知自己已離隊（清除前端 party UI）
                            let left = ServerMsg::PartyDisbanded;
                            if let Ok(j) = serde_json::to_string(&left) { let _ = tx_direct.try_send(j); }
                        }
                        tracing::info!(%uid, disbanded, "離開隊伍");
                    }
                }
                // ── 隊伍系統 end ─────────────────────────────────────────────

                // ── 倉庫（ROADMAP 105）───────────────────────────────────────
                Ok(ClientMsg::BuyWarehouseExpansion) => {
                    use crate::warehouse::{WAREHOUSE_EXPANSION_COST};
                    let Some(_uid) = authed_uid else { continue; };
                    let mut chat_opt = None;
                    if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                        if p.ether < WAREHOUSE_EXPANSION_COST {
                            // 乙太不足：私訊告知
                            let msg = ServerMsg::Chat {
                                from: "系統".into(),
                                text: format!("倉庫擴充需要 {} 乙太，目前不足。", WAREHOUSE_EXPANSION_COST),
                            };
                            if let Ok(j) = serde_json::to_string(&msg) { let _ = tx_direct.try_send(j); }
                        } else if !p.warehouse.can_buy_expansion() {
                            let msg = ServerMsg::Chat {
                                from: "系統".into(),
                                text: "倉庫已達最大容量，無法再擴充。".into(),
                            };
                            if let Ok(j) = serde_json::to_string(&msg) { let _ = tx_direct.try_send(j); }
                        } else {
                            p.ether -= WAREHOUSE_EXPANSION_COST;
                            p.warehouse.buy_expansion();
                            let cap = p.warehouse.capacity();
                            tracing::info!(player = %p.name, cap, "購買倉庫擴充");
                            chat_opt = Some(format!("📦 倉庫擴充成功！現可存放最多 {} 種物品。", cap));
                        }
                    }
                    if let Some(text) = chat_opt {
                        let msg = ServerMsg::Chat { from: "系統".into(), text };
                        if let Ok(j) = serde_json::to_string(&msg) { let _ = tx_direct.try_send(j); }
                    }
                }
                Ok(ClientMsg::WithdrawFromWarehouse { item, qty }) => {
                    use crate::warehouse::MAX_INVENTORY_ITEM_KINDS;
                    let Some(_uid) = authed_uid else { continue; };
                    let mut chat_opt = None;
                    if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                        if p.warehouse.count(item) < qty {
                            // 倉庫不足
                            let msg = ServerMsg::Chat {
                                from: "系統".into(),
                                text: format!("倉庫中的 {:?} 不足 {} 個。", item, qty),
                            };
                            if let Ok(j) = serde_json::to_string(&msg) { let _ = tx_direct.try_send(j); }
                        } else if p.inventory.is_full_for_new_kind(item, MAX_INVENTORY_ITEM_KINDS) {
                            // 背包種類槽滿
                            let msg = ServerMsg::Chat {
                                from: "系統".into(),
                                text: format!("背包種類已達 {} 種上限，取出前請先整理背包。", MAX_INVENTORY_ITEM_KINDS),
                            };
                            if let Ok(j) = serde_json::to_string(&msg) { let _ = tx_direct.try_send(j); }
                        } else if p.warehouse.take(item, qty) {
                            let added = p.inventory.add(item, qty);
                            if added < qty {
                                // 背包同種堆到 MAX_STACK，多餘量退回倉庫，不丟失
                                p.warehouse.add(item, qty - added);
                            }
                            tracing::info!(player = %p.name, ?item, qty, added, "從倉庫取回物品");
                            chat_opt = Some(format!("📦 已從倉庫取回 {:?} ×{}。", item, added));
                        }
                    }
                    if let Some(text) = chat_opt {
                        let msg = ServerMsg::Chat { from: "系統".into(), text };
                        if let Ok(j) = serde_json::to_string(&msg) { let _ = tx_direct.try_send(j); }
                    }
                }
                // ── 倉庫 end ─────────────────────────────────────────────────

                // ── 住家內裝（ROADMAP 111）────────────────────────────────────
                Ok(ClientMsg::EnterHome) => {
                    let Some(uid) = authed_uid else { continue; };
                    let mut notice: Option<String> = None;
                    {
                        // 鎖序鐵律：先在 land_plots guard 內算好 plot_id 並「釋放」，才鎖 players——
                        // 遊戲迴圈快照反向持鎖（players.read → land_plots.read），在 land guard 內鎖
                        // players 會與排隊中的 land_plots.write（買地）組成三方死鎖環。
                        let plot_id_opt = {
                            let land = app.land_plots.read().unwrap();
                            land.plot_of(uid).and_then(|pid| {
                                use crate::land_plot::PlotPurpose;
                                if land.purpose_of(pid) == Some(PlotPurpose::FreeBuild) {
                                    Some(pid)
                                } else {
                                    None
                                }
                            })
                        };
                        if let Some(plot_id) = plot_id_opt {
                            let mut players = app.players.write().unwrap();
                            if let Some(p) = players.get_mut(&id) {
                                if p.indoor_plot_id.is_some() {
                                    // 已在室內，忽略
                                } else if crate::home_interior::near_home(plot_id, p.x, p.y) {
                                    let (ix, iy) = crate::home_interior::entry_position();
                                    p.indoor_plot_id = Some(plot_id);
                                    p.indoor_x = ix;
                                    p.indoor_y = iy;
                                    tracing::info!(player = %p.name, plot_id, "進入住家室內");
                                } else {
                                    notice = Some("🏠 需靠近自己的建地中心才能進入室內。".to_string());
                                }
                            }
                        } else {
                            notice = Some("🏠 你還沒有 FreeBuild 建地，無法進入室內。".to_string());
                        }
                    }
                    if let Some(text) = notice {
                        let msg = ServerMsg::Chat { from: "系統".into(), text };
                        if let Ok(j) = serde_json::to_string(&msg) { let _ = tx_direct.try_send(j); }
                    }
                }
                Ok(ClientMsg::ExitHome) => {
                    if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                        if p.indoor_plot_id.is_some() {
                            p.indoor_plot_id = None;
                            p.indoor_x = 0.0;
                            p.indoor_y = 0.0;
                            tracing::info!(player = %p.name, "離開住家室內");
                        }
                    }
                }
                // ── 住家家具（ROADMAP 155）───────────────────────────────────
                Ok(ClientMsg::PlaceFurniture { kind }) => {
                    use crate::home_furniture::FurnitureKind;
                    use crate::inventory::ItemKind;
                    if let Some(uid) = authed_uid {
                        let fkind = FurnitureKind::from_str(&kind);
                        let item_kind: Option<ItemKind> = match fkind {
                            Some(FurnitureKind::SteamBed)    => Some(ItemKind::SteamBed),
                            Some(FurnitureKind::AetherChest) => Some(ItemKind::AetherChest),
                            Some(FurnitureKind::EtherPlant)  => Some(ItemKind::EtherPlant),
                            Some(FurnitureKind::StarLantern) => Some(ItemKind::StarLantern),
                            Some(FurnitureKind::AncientDeco) => Some(ItemKind::AncientDeco),
                            None => None,
                        };
                        if let (Some(fkind), Some(iitem)) = (fkind, item_kind) {
                            let mut players = app.players.write().unwrap();
                            if let Some(p) = players.get_mut(&uid) {
                                // 玩家必須在室內且背包有對應家具物品。
                                if p.indoor_plot_id.is_some() && p.inventory.has(iitem, 1) {
                                    // ROADMAP 323：把家具擺在玩家當前所站的室內格——走到想擺的位置再按放置。
                                    let (col, row) = crate::home_interior::cell_of(p.indoor_x, p.indoor_y);
                                    let mut furnishings = app.home_furnishings.write().unwrap();
                                    let home = furnishings.entry(uid).or_default();
                                    if home.place(fkind, col, row) {
                                        // 成功放置，從背包扣除。
                                        let _ = p.inventory.take(iitem, 1);
                                        // 乙太箱背包容量加成即時生效。
                                        if fkind == FurnitureKind::AetherChest {
                                            p.inventory_extra_kinds = crate::home_furniture::CHEST_CAPACITY_BONUS as u32;
                                        }
                                        tracing::info!(player = %p.name, ?fkind, "放置住家家具");
                                    }
                                }
                            }
                        }
                    }
                }
                Ok(ClientMsg::RemoveFurniture { idx }) => {
                    use crate::inventory::ItemKind;
                    if let Some(uid) = authed_uid {
                        let removed = {
                            let mut furnishings = app.home_furnishings.write().unwrap();
                            furnishings.get_mut(&uid).and_then(|h| h.remove(idx))
                        };
                        if let Some(fkind) = removed {
                            let item_kind: ItemKind = match fkind {
                                crate::home_furniture::FurnitureKind::SteamBed    => ItemKind::SteamBed,
                                crate::home_furniture::FurnitureKind::AetherChest => ItemKind::AetherChest,
                                crate::home_furniture::FurnitureKind::EtherPlant  => ItemKind::EtherPlant,
                                crate::home_furniture::FurnitureKind::StarLantern => ItemKind::StarLantern,
                                crate::home_furniture::FurnitureKind::AncientDeco => ItemKind::AncientDeco,
                            };
                            let mut players = app.players.write().unwrap();
                            if let Some(p) = players.get_mut(&uid) {
                                p.add_item_overflow(item_kind, 1);
                                // 移除乙太箱：背包容量加成消失。
                                if fkind == crate::home_furniture::FurnitureKind::AetherChest {
                                    p.inventory_extra_kinds = 0;
                                }
                                tracing::info!(player = %p.name, ?fkind, idx, "移除住家家具（退還背包）");
                            }
                        }
                    }
                }
                Ok(ClientMsg::CycleHomeStyle) => {
                    // ROADMAP 325：玩家在自己室內循環切換居家風格主題。
                    if let Some(uid) = authed_uid {
                        let indoor = {
                            let players = app.players.read().unwrap();
                            players.get(&uid).map(|p| p.indoor_plot_id.is_some()).unwrap_or(false)
                        };
                        if indoor {
                            let mut furnishings = app.home_furnishings.write().unwrap();
                            let home = furnishings.entry(uid).or_default();
                            let new_style = home.cycle_style();
                            tracing::info!(?uid, style = new_style.code(), "切換居家風格");
                        }
                    }
                }
                // ── 住家內裝 + 家具 end ──────────────────────────────────────

                // ── 居民搭話（ROADMAP 118）────────────────────────────────────
                Ok(ClientMsg::TalkToResident { resident_id }) => {
                    // 驗證範圍 + 找居民
                    let found = {
                        let players = app.players.read().unwrap();
                        let residents = app.residents.read().unwrap();
                        players.get(&id).and_then(|p| {
                            residents.find_by_id(&resident_id).and_then(|(persona, name, rx, ry)| {
                                let dx = p.x - rx;
                                let dy = p.y - ry;
                                if dx * dx + dy * dy
                                    <= crate::resident_npc::RESIDENT_REACH
                                        * crate::resident_npc::RESIDENT_REACH
                                {
                                    // 種子 = 玩家 id bits XOR 居民名長度（可重現但夠隨機）
                                    let seed = id.as_u128() as usize ^ name.len();
                                    Some((persona, name.to_string(), rx, ry, seed))
                                } else {
                                    None
                                }
                            })
                        })
                    };
                    if let Some((persona, name, rx, ry, seed)) = found {
                        let reply_text = crate::resident_chat::get_chat(persona, seed).to_string();
                        // 私人回應（NpcReply，只有本人看到）
                        if let Ok(json) = serde_json::to_string(&crate::protocol::ServerMsg::NpcReply {
                            npc: resident_id.clone(),
                            display: format!("💬 居民 {name}"),
                            text: reply_text.clone(),
                        }) {
                            let _ = tx_direct.send(json).await;
                        }
                        // 世界可見泡泡（NpcSpeech，讓附近玩家也看到居民在說話）
                        let _ = app.tx.send(std::sync::Arc::new(crate::protocol::ServerMsg::NpcSpeech {
                            npc_id: resident_id,
                            npc_name: format!("居民 {name}"),
                            text: reply_text,
                            display_secs: 6,
                            wx: rx,
                            wy: ry,
                        }));
                    }
                }
                // ── 居民搭話 end ──────────────────────────────────────────────

                // ── 向主要 NPC 搭話（ROADMAP 255）────────────────────────────
                Ok(ClientMsg::TalkToMajorNpc { npc_id }) => {
                    use crate::npc::SHOP_REACH;
                    // 解析該主要 NPC 的即時座標與顯示名（固定六大走 npc_schedule＋npc_lifecycle、旅人走 traveler）
                    let npc_loc: Option<(String, f32, f32)> = if npc_id.starts_with("traveler") {
                        let tv = app.traveler.read().unwrap();
                        if tv.is_visible() {
                            Some((tv.name().to_string(), tv.x, tv.y))
                        } else {
                            None
                        }
                    } else {
                        let sch = app.npc_schedule.read().unwrap();
                        let lc = app.npc_lifecycle.read().unwrap();
                        sch.get_pos(&npc_id)
                            .map(|(mx, my)| (lc.current_display(&npc_id).to_string(), mx, my))
                    };
                    // 驗證玩家在互動範圍內，取玩家名與搭話種子
                    let found = npc_loc.and_then(|(npc_name, mx, my)| {
                        let players = app.players.read().unwrap();
                        players.get(&id).and_then(|p| {
                            let dx = p.x - mx;
                            let dy = p.y - my;
                            if dx * dx + dy * dy <= SHOP_REACH * SHOP_REACH {
                                // 種子 = 玩家 id bits XOR 玩家名長度（可重現又夠隨機，每次搭話輪替話題）
                                let seed = id.as_u128() as usize ^ p.name.len();
                                Some((npc_name, mx, my, p.name.clone(), seed))
                            } else {
                                None
                            }
                        })
                    });
                    if let Some((npc_name, mx, my, player_name, seed)) = found {
                        // 動態話題層（沿用 244）：世界大事 > NPC 八卦 > 日常寒暄，零 LLM
                        let world_events: Vec<String> =
                            app.world_log.read().unwrap().recent().iter().cloned().collect();
                        let relations = app.npc_relations.read().unwrap().significant_relations(&npc_id);
                        let reply_text = crate::resident_chat::get_dynamic_major_npc_greet(
                            &npc_id,
                            &player_name,
                            seed,
                            &world_events,
                            &relations,
                        );
                        // 私人回應（NpcReply，只有本人看到）
                        if let Ok(json) = serde_json::to_string(&crate::protocol::ServerMsg::NpcReply {
                            npc: npc_id.clone(),
                            display: format!("💬 {npc_name}"),
                            text: reply_text.clone(),
                        }) {
                            let _ = tx_direct.send(json).await;
                        }
                        // 世界可見泡泡（NpcSpeech，讓附近玩家也看到大人物在跟人說話）
                        let _ = app.tx.send(std::sync::Arc::new(crate::protocol::ServerMsg::NpcSpeech {
                            npc_id,
                            npc_name,
                            text: reply_text,
                            display_secs: 6,
                            wx: mx,
                            wy: my,
                        }));
                    }
                }
                // ── 向主要 NPC 搭話 end ────────────────────────────────────────

                // ── 居民互助請求（ROADMAP 125）────────────────────────────────
                Ok(ClientMsg::HelpResident { resident_id }) => {
                    use crate::resident_npc::RESIDENT_REACH;
                    use crate::resident_npc::HELP_REWARD_ETHER;

                    // 驗證範圍 + 確認居民正在求助
                    let found = {
                        let players = app.players.read().unwrap();
                        let residents = app.residents.read().unwrap();
                        players.get(&id).and_then(|p| {
                            residents.find_requesting_by_id(&resident_id).and_then(|(persona, name, rx, ry)| {
                                let dx = p.x - rx;
                                let dy = p.y - ry;
                                if dx * dx + dy * dy > RESIDENT_REACH * RESIDENT_REACH {
                                    return None;
                                }
                                Some((persona, name.to_string(), rx, ry))
                            })
                        })
                    };
                    if let Some((persona, resident_name, rx, ry)) = found {
                        // 完成請求（原子性：只有第一個點的玩家能成功）；ROADMAP 126 同時回傳快樂提升事件。
                        let (fulfilled, happiness_boost) = app.residents.write().unwrap().fulfill_help_request(&resident_id);
                        // 快樂值突破門檻時廣播世界聊天（ROADMAP 126）
                        if let Some(crate::resident_npc::ResidentLifecycleEvent::HappinessBoost { msg, .. }) = happiness_boost {
                            let _ = app.tx_chat.send(msg);
                        }
                        if fulfilled {
                            // 給玩家乙太獎勵
                            let player_name = {
                                let mut players = app.players.write().unwrap();
                                if let Some(p) = players.get_mut(&id) {
                                    p.ether = p.ether.saturating_add(HELP_REWARD_ETHER);
                                    p.name.clone()
                                } else {
                                    String::new()
                                }
                            };
                            if !player_name.is_empty() {
                                // 居民感謝語泡泡（廣播給周圍玩家）
                                let seed = player_name.len() ^ resident_name.len();
                                let thanks_text = crate::resident_chat::get_help_thanks(
                                    persona, &resident_name, &player_name, seed,
                                );
                                let _ = app.tx.send(std::sync::Arc::new(
                                    crate::protocol::ServerMsg::NpcSpeech {
                                        npc_id: resident_id.clone(),
                                        npc_name: format!("居民 {resident_name}"),
                                        text: thanks_text.clone(),
                                        display_secs: 7,
                                        wx: rx,
                                        wy: ry,
                                    }
                                ));
                                // 私信告知玩家獎勵明細
                                if let Ok(json) = serde_json::to_string(
                                    &crate::protocol::ServerMsg::NpcReply {
                                        npc: resident_id,
                                        display: format!("居民 {resident_name}"),
                                        text: format!("{thanks_text}（+{HELP_REWARD_ETHER} 乙太）"),
                                    }
                                ) {
                                    let _ = tx_direct.send(json).await;
                                }
                            }
                        }
                    }
                }
                // ── 居民互助請求 end ──────────────────────────────────────────

                // ── 流星雨星塵採集（ROADMAP 133/134）───────────────────────────
                Ok(ClientMsg::CollectDustNode { node_id }) => {
                    // try_collect 回 Some(is_rainbow)=成功，None=失敗。
                    let player_pos = app.players.read().unwrap().get(&id).map(|p| (p.x, p.y));
                    let collect_result = player_pos.and_then(|(px, py)| {
                        app.meteor_shower.write().unwrap().try_collect(node_id, px, py)
                    });
                    if let Some(is_rainbow) = collect_result {
                        let shower_active = app.meteor_shower.read().unwrap().is_active();
                        if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                            // 彩虹節點給彩虹星塵，普通節點給星塵。
                            if is_rainbow {
                                p.add_item_overflow(crate::inventory::ItemKind::RainbowStarDust, 1);
                                tracing::info!(player = %p.name, node_id, "採集彩虹星塵節點");
                            } else {
                                p.add_item_overflow(crate::inventory::ItemKind::StarDust, 1);
                                tracing::info!(player = %p.name, node_id, "採集星塵節點");
                            }
                            // 流星雨期間持有星際守護符額外 +1 星塵（ROADMAP 134）。
                            if shower_active && p.inventory.count(crate::inventory::ItemKind::StarGuardianAmulet) > 0 {
                                p.add_item_overflow(crate::inventory::ItemKind::StarDust, 1);
                            }
                        }
                    }
                }
                // ── 流星雨星塵採集 end ───────────────────────────────────────────

                // ── 夜間乙太泉採集（ROADMAP 162）─────────────────────────────────────
                Ok(ClientMsg::CollectSpringNode { node_id }) => {
                    let player_pos = app.players.read().unwrap().get(&id).map(|p| (p.x, p.y));
                    let collected = player_pos.map(|(px, py)| {
                        app.night_springs.write().unwrap().try_collect(node_id, px, py)
                    }).unwrap_or(false);
                    if collected {
                        let all_done = app.night_springs.read().unwrap().all_collected();
                        if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                            p.ether = p.ether.saturating_add(crate::night_aether_springs::ETHER_REWARD);
                            tracing::info!(player = %p.name, node_id, "採集夜間乙太泉");
                        }
                        // 全部採集完成時記入城鎮記憶石（一晚只記一次）。
                        let mut ns = app.night_springs.write().unwrap();
                        if all_done && !ns.all_collected_announced {
                            ns.all_collected_announced = true;
                            drop(ns);
                            let player_name = app.players.read().unwrap()
                                .get(&id).map(|p| p.name.clone()).unwrap_or_default();
                            if !player_name.is_empty() {
                                app.town_memory.write().unwrap().push_event(
                                    "🌙",
                                    format!("夜探者 {} 採集了今夜全部乙太泉！", player_name),
                                );
                            }
                        }
                    }
                }
                // ── 夜間乙太泉採集 end ────────────────────────────────────────────────

                // ── 旅行商人交易（ROADMAP 135）───────────────────────────────────────
                Ok(ClientMsg::BuyFromWanderer { item, qty }) => {
                    // 1. 需登入
                    if authed_uid.is_none() {
                        if let Ok(j) = serde_json::to_string(&crate::protocol::ServerMsg::Chat {
                            from: "系統".into(), text: "需要登入才能與旅行商人交易".into(),
                        }) { let _ = tx_direct.try_send(j); }
                        continue;
                    }
                    // 2. 玩家在範圍內
                    let player_pos = app.players.read().unwrap().get(&id).map(|p| (p.x, p.y));
                    let Some((px, py)) = player_pos else { continue; };
                    let dx = px - crate::wandering_merchant::WANDERER_X;
                    let dy = py - crate::wandering_merchant::WANDERER_Y;
                    let in_range = dx * dx + dy * dy
                        <= crate::wandering_merchant::TRADE_REACH * crate::wandering_merchant::TRADE_REACH;
                    if !in_range {
                        continue; // 靜默忽略（前端不應在玩家不在範圍內時送此訊息）
                    }
                    // 3. 嘗試購買（扣庫存、計算費用）
                    let buy_result = app.wandering_merchant.write().unwrap().buy(item, qty);
                    match buy_result {
                        Ok(cost) => {
                            let mut players = app.players.write().unwrap();
                            if let Some(p) = players.get_mut(&id) {
                                if p.ether < cost {
                                    drop(players);
                                    // 乙太不足：退回庫存
                                    if let Ok(mut wm) = app.wandering_merchant.write() {
                                        if let Some(e) = wm.catalog.iter_mut().find(|e| e.item == item) {
                                            e.sold = e.sold.saturating_sub(qty);
                                        }
                                    }
                                    if let Ok(j) = serde_json::to_string(&crate::protocol::ServerMsg::Chat {
                                        from: "旅行商人".into(), text: "😅 你的乙太不夠，我也沒辦法……".into(),
                                    }) { let _ = tx_direct.try_send(j); }
                                } else {
                                    p.ether -= cost;
                                    p.add_item_overflow(item, qty);
                                    let item_name = crate::npc_deal::item_display_zh(item);
                                    let name = p.name.clone();
                                    drop(players);
                                    tracing::info!(
                                        player = %name, ?item, qty, cost,
                                        "向旅行商人購買商品"
                                    );
                                    if let Ok(j) = serde_json::to_string(&crate::protocol::ServerMsg::Chat {
                                        from: "旅行商人".into(),
                                        text: format!("✅ 賣給你 {}×{}，共 {} 乙太。一路平安！", item_name, qty, cost),
                                    }) { let _ = tx_direct.try_send(j); }
                                }
                            }
                        }
                        Err(reason) => {
                            if let Ok(j) = serde_json::to_string(&crate::protocol::ServerMsg::Chat {
                                from: "旅行商人".into(),
                                text: format!("😅 {}", reason),
                            }) { let _ = tx_direct.try_send(j); }
                        }
                    }
                }
                // ── 旅行商人交易 end ─────────────────────────────────────────────────

                // ── 旅行商人限時委託接取（ROADMAP 136）──────────────────────────────────
                Ok(ClientMsg::AcceptMerchantQuest { quest_id }) => {
                    // 需登入
                    if authed_uid.is_none() {
                        if let Ok(j) = serde_json::to_string(&crate::protocol::ServerMsg::Chat {
                            from: "系統".into(), text: "需要登入才能接取委託".into(),
                        }) { let _ = tx_direct.try_send(j); }
                        continue;
                    }
                    // 玩家在範圍內
                    let player_pos = app.players.read().unwrap().get(&id).map(|p| (p.x, p.y));
                    let Some((px, py)) = player_pos else { continue; };
                    let dx = px - crate::wandering_merchant::WANDERER_X;
                    let dy = py - crate::wandering_merchant::WANDERER_Y;
                    let in_range = dx * dx + dy * dy
                        <= crate::wandering_merchant::TRADE_REACH * crate::wandering_merchant::TRADE_REACH;
                    if !in_range { continue; }
                    // 嘗試接取委託
                    let result = app.wandering_merchant.write().unwrap().accept_quest(quest_id);
                    match result {
                        Ok(quest_name) => {
                            if let Ok(j) = serde_json::to_string(&crate::protocol::ServerMsg::Chat {
                                from: "旅行商人".into(),
                                text: format!("📋 接取委託：{}。商人在場期間完成即可領賞！", quest_name),
                            }) { let _ = tx_direct.try_send(j); }
                        }
                        Err(reason) => {
                            if let Ok(j) = serde_json::to_string(&crate::protocol::ServerMsg::Chat {
                                from: "旅行商人".into(),
                                text: format!("😅 {}", reason),
                            }) { let _ = tx_direct.try_send(j); }
                        }
                    }
                }
                // ── 旅行商人限時委託接取 end ──────────────────────────────────────────

                // ── 公民投票（ROADMAP 156）────────────────────────────────────────────
                Ok(ClientMsg::CivicVote { yes }) => {
                    // 必須已登入才能投票。訪客 id 每次連線都換新 Uuid，重連即可重複灌票，
                    // 故僅登入玩家可投，並以 uid 為去重鍵（與 ConfirmDeal「訪客不參與」一致）。
                    let uid = match authed_uid {
                        Some(u) => u,
                        None => {
                            if let Ok(j) = serde_json::to_string(&crate::protocol::ServerMsg::Chat {
                                from: "城鎮".into(),
                                text: "🗳️ 登入後才能參與城鎮投票喔！".into(),
                            }) { let _ = tx_direct.try_send(j); }
                            continue;
                        }
                    };
                    let player_id = uid.to_string();
                    let accepted = app.civic_vote.write().unwrap().cast_vote(&player_id, yes);
                    if accepted {
                        let vote_label = if yes { "✅ 讚成" } else { "❌ 反對" };
                        if let Ok(j) = serde_json::to_string(&crate::protocol::ServerMsg::Chat {
                            from: "城鎮".into(),
                            text: format!("🗳️ 你投下了{}票！", vote_label),
                        }) { let _ = tx_direct.try_send(j); }
                    }
                    // 已投過或無活躍投票 → 靜默忽略。
                }
                // ── 公民投票 end ──────────────────────────────────────────────────────

                // ── 城鎮記憶石（ROADMAP 157）────────────────────────────────────────
                Ok(ClientMsg::ReadTownMemory) => {
                    // 無需登入、無需在保護圈；只需玩家靠近記憶石。
                    let in_range = {
                        let players = app.players.read().unwrap();
                        players.get(&id).map(|p| {
                            crate::town_memory::is_near_stone(p.x, p.y)
                        }).unwrap_or(false)
                    };
                    if in_range {
                        let entries: Vec<_> = {
                            let mem = app.town_memory.read().unwrap();
                            mem.recent_desc(crate::town_memory::MAX_ENTRIES)
                                .into_iter()
                                .cloned()
                                .collect()
                        };
                        if let Ok(j) = serde_json::to_string(&crate::protocol::ServerMsg::TownMemoryList { entries }) {
                            let _ = tx_direct.try_send(j);
                        }
                    }
                    // 不在範圍 → 靜默忽略。
                }
                // ── 城鎮記憶石 end ────────────────────────────────────────────────

                Ok(ClientMsg::Join { .. }) => {} // 已進場，忽略
                Err(e) => tracing::debug!("無法解析客戶端訊息：{e}"),
            },
            Message::Close(_) => break,
            _ => {}
        }
    }

    // ROADMAP 95 密語路由：離線時從 map 移除，後續密語嘗試會正確回報「不在線」。
    app.whisper_senders.write().unwrap().remove(&id);
    // ROADMAP 97 隊伍清理：玩家離線視同離隊，通知其他成員。
    if let Some(uid) = authed_uid {
        if let Some((disbanded, remaining)) = app.parties.leave(uid) {
            if let Some(p) = app.players.write().unwrap().get_mut(&uid) { p.party_id = None; }
            if disbanded {
                // 隊長離線或人數不足 → 解散：清除所有前成員的 party_id 並通知。
                {
                    let mut ps = app.players.write().unwrap();
                    for &m in &remaining { if let Some(p) = ps.get_mut(&m) { p.party_id = None; } }
                }
                let disbanded_msg = ServerMsg::PartyDisbanded;
                let senders = app.whisper_senders.read().unwrap();
                for m in &remaining {
                    if let Some(tx) = senders.get(m) {
                        if let Ok(j) = serde_json::to_string(&disbanded_msg) { let _ = tx.try_send(j); }
                    }
                }
            } else {
                // 普通成員離線 → 隊伍繼續，通知剩餘成員更新名單。
                if let Some(&first) = remaining.first() {
                    if let Some(pid) = app.parties.party_of(first) {
                        let leader_id = app.parties.leader_of(pid).unwrap_or_default();
                        broadcast_party_update(&app, pid, &remaining, leader_id);
                    }
                }
            }
        }
    }
    forward.abort();
    cleanup(&app, id, authed_uid.is_some()).await;
    tracing::info!(player = %player.name, %id, "玩家離線");
}

/// 組裝 `ServerMsg::FriendList`：查好友 UUID → 查名字（UserStore）→ 判斷在線（players map）。
/// 廣播 `PartyUpdate` 給隊伍所有在線成員（ROADMAP 97）。
fn broadcast_party_update(app: &AppState, party_id: Uuid, members: &[Uuid], leader_id: Uuid) {
    let ps = app.players.read().unwrap();
    let member_names: Vec<String> = members.iter()
        .filter_map(|m| ps.get(m).map(|p| p.name.clone()))
        .collect();
    drop(ps);
    let senders = app.whisper_senders.read().unwrap();
    for &m in members {
        let is_leader = m == leader_id;
        let msg = ServerMsg::PartyUpdate { members: member_names.clone(), is_leader };
        if let Ok(j) = serde_json::to_string(&msg) {
            if let Some(tx) = senders.get(&m) { let _ = tx.try_send(j); }
        }
    }
    let _ = party_id; // 目前僅用 members，保留 party_id 供未來擴充
}

fn build_friend_list_msg(app: &AppState, user_id: Uuid) -> ServerMsg {
    use crate::protocol::FriendEntry;
    let friend_ids = app.friends.get_friends(user_id);
    let online_ids: std::collections::HashSet<Uuid> = app.players.read().unwrap().keys().copied().collect();
    let friends: Vec<FriendEntry> = friend_ids
        .into_iter()
        .filter_map(|fid| {
            let user = app.users.get(fid)?;
            Some(FriendEntry {
                id: fid,
                name: user.name,
                online: online_ids.contains(&fid),
            })
        })
        .collect();
    ServerMsg::FriendList { friends }
}

/// 依 guild_id 與 player_id 建立 GuildView（ROADMAP 29）。
fn build_guild_view(app: &AppState, player_id: Uuid, guild_id: Uuid) -> Option<crate::protocol::GuildView> {
    let g = app.guilds.get(guild_id)?;
    Some(crate::protocol::GuildView {
        id: g.id,
        name: g.name.clone(),
        tag: g.tag.clone(),
        is_founder: g.founder_id == player_id,
        member_count: g.member_count(),
        treasury: g.treasury,
    })
}

/// 玩家離線清理。先放掉這條連線；只有當這是該玩家的**最後一條**連線（同帳號其餘分頁
/// 社群任務完成時：廣播公告 + 給全員在線玩家乙太獎勵（ROADMAP 27）。
fn notify_quest_complete(app: &AppState, completed_descs: Vec<String>) {
    if completed_descs.is_empty() { return; }
    for desc in &completed_descs {
        let msg = format!(
            "🎉 全服任務達成！「{}」完成！所有在線玩家各得 {} 乙太！",
            desc,
            crate::quest::QUEST_COMPLETE_REWARD,
        );
        let _ = app.tx_chat.send(msg);
        // 世界事件記憶（ROADMAP 65）：全服任務完成是重大里程碑，NPC 值得提及。
        app.world_log.write().unwrap().push(format!(
            "全服社群任務「{}」完成，全體拓荒者共同達成壯舉", desc
        ));
        // NPC 需求驅力（ROADMAP 69）：任務完成 → 社群歸屬感升高，里長/老農特別高興。
        app.npc_needs.write().unwrap().apply_world_event(crate::npc_needs::NeedsEvent::QuestCompleted);
        // NPC 人際關係網（ROADMAP 70）：任務完成加深里長對執行者的好感。
        app.npc_relations.write().unwrap().apply_world_event(crate::npc_relations::RelationsEvent::QuestCompleted);
        // NPC 主動評論（ROADMAP 68）：任務完成，NPC 在聊天頻道慶賀。
        {
            let event_kind = crate::npc_proactive::WorldEventKind::QuestComplete {
                name: desc.clone(),
            };
            let app2 = app.clone();
            tokio::spawn(async move {
                let now = std::time::Instant::now();
                let maybe_npc = {
                    let mut cd = app2.npc_proactive.write().unwrap();
                    crate::npc_proactive::pick_reacting_npc(&event_kind, &mut cd, now)
                };
                if let Some(npc_id) = maybe_npc {
                    let reaction = crate::npc_proactive::generate_proactive_reaction(npc_id, event_kind).await;
                    let _ = app2.tx_chat.send(reaction);
                }
            });
        }
    }
    // 全員分潤乙太 + 成就：任務英雄（ROADMAP 31）。
    let mut newly_heroes: Vec<(String, bool)> = Vec::new();
    let mut players = app.players.write().unwrap();
    for p in players.values_mut() {
        p.ether = p.ether.saturating_add(
            crate::quest::QUEST_COMPLETE_REWARD * completed_descs.len() as u32
        );
        let is_new = p.achievements.unlock(crate::achievement::Achievement::QuestHero);
        newly_heroes.push((p.name.clone(), is_new));
    }
    drop(players);
    for (pname, is_new) in newly_heroes {
        if is_new {
            let _ = app.tx_chat.send(format!(
                "🏆 {} 解鎖成就「{}」！",
                pname,
                crate::achievement::Achievement::QuestHero.display_name()
            ));
        }
    }
}

// ── 每日任務輔助函式（ROADMAP 32）────────────────────────────────────────────────

/// 取得或初始化玩家每日狀態後，執行閉包並回傳結果。
/// 閉包回傳 `(completed_task_idx, views, done_count, player_name)`。
fn with_daily_state<F, R>(app: &AppState, uid: uuid::Uuid, f: F) -> Option<R>
where
    F: FnOnce(&mut crate::daily_quest::PlayerDailyState) -> R,
{
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let seed = uid.as_u128() as u64;
    let mut dq = app.daily_quests.write().unwrap();
    let state = dq.entry(uid).or_insert_with(|| crate::daily_quest::PlayerDailyState::new(seed, now));
    state.check_reset(now, seed);
    Some(f(state))
}

/// 每日任務完成時，給玩家乙太 + EXP 並送出更新。
fn on_daily_task_completed(
    app: &AppState,
    uid: uuid::Uuid,
    views: Vec<crate::daily_quest::DailyTaskView>,
    done_count: u32,
    all_done: bool,
    tx: &tokio::sync::mpsc::Sender<String>,
) {
    // 乙太 + EXP 獎勵。
    let (pname, daily_level_up): (String, Option<u32>) = {
        let mut players = app.players.write().unwrap();
        if let Some(p) = players.get_mut(&uid) {
            p.ether = p.ether.saturating_add(crate::daily_quest::DAILY_TASK_ETHER_REWARD);
            let old_level = p.level();
            p.exp = p.exp.saturating_add(crate::daily_quest::DAILY_TASK_EXP_REWARD);
            let new_lv = if p.level() > old_level {
                // 升等給屬性點（ROADMAP 152）：先加點再計算 max HP。
                p.stats.unspent = p.stats.unspent.saturating_add(crate::stat_points::POINTS_PER_LEVEL);
                let full_max = crate::vitals::level_max_hp(p.level())
                    + crate::class::hp_bonus(&p.masteries)
                    + p.stats.hp * crate::stat_points::HP_PER_POINT;
                p.vitals.on_level_up(full_max);
                Some(p.level())
            } else {
                None
            };
            (p.name.clone(), new_lv)
        } else {
            (String::new(), None)
        }
    };
    // NPC 升等賀詞（ROADMAP 84）：每日任務升等時凱爾長老私信賀詞 / 全服廣播。
    if let Some(new_lv) = daily_level_up {
        if !pname.is_empty() {
            let action = app.npc_level_greet.write().unwrap().on_level_up(&pname, new_lv);
            match action {
                crate::npc_level_greet::LevelGreetAction::WorldBroadcast { message } => {
                    let _ = app.tx_chat.send(format!(
                        "🌟 [{}] 全服宣告：「{}」",
                        crate::npc_level_greet::CHIEF_DISPLAY_NAME, message
                    ));
                }
                crate::npc_level_greet::LevelGreetAction::DirectMessage { message } => {
                    let _ = tx.try_send(format!(
                        "💬 [{}] 悄聲道：「{}」",
                        crate::npc_level_greet::CHIEF_DISPLAY_NAME, message
                    ));
                }
            }
        }
    }
    // 送出更新給本人。
    let msg = ServerMsg::DailyQuestsUpdate { tasks: views, done_count };
    if let Ok(json) = serde_json::to_string(&msg) {
        let _ = tx.try_send(json);
    }
    // 三條全完：全服廣播。
    if all_done && !pname.is_empty() {
        let _ = app.tx_chat.send(format!("🌟 {} 完成今日全部每日任務！", pname));
    }
}

/// 擊殺事件推進每日任務。
fn advance_daily_kill(
    app: &AppState,
    uid: uuid::Uuid,
    kind: crate::combat::EnemyKind,
    tx: &tokio::sync::mpsc::Sender<String>,
) {
    let result = with_daily_state(app, uid, |state| {
        let completed = state.on_kill(kind);
        if completed.is_some() {
            let views: Vec<_> = state.tasks.iter().map(|t| t.to_view()).collect();
            let done = state.done_count() as u32;
            let all = state.all_complete() && !state.all_done_announced;
            if all { state.all_done_announced = true; }
            Some((views, done, all))
        } else {
            None
        }
    });
    if let Some(Some((views, done, all))) = result {
        on_daily_task_completed(app, uid, views, done, all, tx);
    }
}

/// 採集事件推進每日任務。
fn advance_daily_gather(
    app: &AppState,
    uid: uuid::Uuid,
    item: crate::inventory::ItemKind,
    amount: u32,
    tx: &tokio::sync::mpsc::Sender<String>,
) {
    let result = with_daily_state(app, uid, |state| {
        let completed = state.on_gather(item, amount);
        if completed.is_some() {
            let views: Vec<_> = state.tasks.iter().map(|t| t.to_view()).collect();
            let done = state.done_count() as u32;
            let all = state.all_complete() && !state.all_done_announced;
            if all { state.all_done_announced = true; }
            Some((views, done, all))
        } else {
            None
        }
    });
    if let Some(Some((views, done, all))) = result {
        on_daily_task_completed(app, uid, views, done, all, tx);
    }
}

/// 旅行事件推進每日任務。
fn advance_daily_travel(
    app: &AppState,
    uid: uuid::Uuid,
    planet: &str,
    tx: &tokio::sync::mpsc::Sender<String>,
) {
    let result = with_daily_state(app, uid, |state| {
        let completed = state.on_travel(planet);
        if completed.is_some() {
            let views: Vec<_> = state.tasks.iter().map(|t| t.to_view()).collect();
            let done = state.done_count() as u32;
            let all = state.all_complete() && !state.all_done_announced;
            if all { state.all_done_announced = true; }
            Some((views, done, all))
        } else {
            None
        }
    });
    if let Some(Some((views, done, all))) = result {
        on_daily_task_completed(app, uid, views, done, all, tx);
    }
}

/// 都離線）時，才真正把玩家移出世界——避免關掉一個分頁順手把另一個還連著的同帳號
/// session 一起踢掉。`persist_pos` 為真（已登入玩家）時，移除前先把最後位置與乙太記
/// 下來，讓同帳號下次重連回到原位、保有收成。鎖序固定「先 players 再 conns」。
async fn cleanup(app: &AppState, id: Uuid, persist_pos: bool) {
    let removed = {
        let mut players = app.players.write().unwrap();
        if app.connections.release(id) {
            let p = players.remove(&id);
            // remember **在鎖內**做,跟新連線的 recall(也在這把鎖內)用同一把鎖排序,
            // 消除 refresh race(舊 cleanup 釋放鎖後才 remember,新連線取得鎖時 recall
            // 還是 None,被瞬移回中央)。鎖內呼叫 PositionStore.remember 用的是它自己的
            // 內部 Mutex,與 players 鎖無交集,不會死鎖。
            if let Some(ref player) = p {
                if persist_pos {
                    app.positions.remember(id, player.x, player.y, player.ether, player.wallet.expansions(), player.exp, player.masteries, player.stats, player.skill_masteries, player.codex, player.atlas, player.skylog, player.cheers);
                    // 背包與裝備槽同樣在鎖內更新 cache。
                    app.inventories.remember(id, &player.inventory);
                    app.inventories.remember_equipment(id, &player.equipment);
                }
            }
            p
        } else {
            None // 同帳號還有其他連線在線，保留玩家
        }
    };
    // Postgres 模式：離線時把最後狀態 upsert 到 DB,補上「最後一次 10s flush 後到離線之間」
    // 的移動（離線後就不再進線上快照了）。在鎖外 await（不可持 std 鎖跨 await）;cache 已在
    // 鎖內由 remember 更新,recall 不受此 await 時序影響。非 Postgres 模式此呼叫無動作。
    if persist_pos {
        if let Some(ref player) = removed {
            app.positions
                .flush_one(id, &player.name, &player.species, player.x, player.y, player.ether, player.wallet.expansions(), player.exp, player.masteries, player.stats, player.skill_masteries, player.codex, player.atlas, player.skylog, player.cheers)
                .await;
            app.inventories.flush_one(id, &player.inventory).await;
            app.inventories.flush_equipment_one(id, &player.equipment).await;
            // 農地離線落地（Phase 0-E）。玩家移出世界後,他的地仍留在 `app.fields` 繼續長,所以
            // 從那裡取當下狀態（不是已移除的 player）。序號由 PlotRegistry 查,一起存好讓重啟能
            // reseat 回正確 origin。補上「最後一次 10s flush 到離線之間」種/澆/收的進度。
            if let Some(index) = app.plots.index_of(id) {
                let field = app.fields.read().unwrap().get(&id).cloned();
                if let Some(field) = field {
                    app.field_store.remember(id, index, &field);
                    app.field_store.flush_one(id, index, &field).await;
                }
            }
        }
    }
    // 只有真的移除了玩家（最後一條連線離線）才廣播離線；否則世界裡那名玩家還在，
    // 不該送 PlayerLeft（會讓其他客戶端先移除、下一張快照又加回造成閃爍）。
    if removed.is_some() {
        let _ = app.tx.send(Arc::new(ServerMsg::PlayerLeft { id }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_normal_chat_trimmed() {
        assert_eq!(sanitize_chat("  哈囉大家  "), Some("哈囉大家".to_string()));
    }

    #[test]
    fn strips_control_chars() {
        // 換行 / 歸位 / NUL / tab 都該被濾掉，不讓客戶端廣播多行或破壞顯示的內容。
        assert_eq!(
            sanitize_chat("一\n二\r三\0四\t五"),
            Some("一二三四五".to_string())
        );
    }

    #[test]
    fn whitespace_or_control_only_is_none() {
        // 全空白或清乾淨後變空 → 不廣播。
        assert_eq!(sanitize_chat("   "), None);
        assert_eq!(sanitize_chat("\n\r\0\t"), None);
        assert_eq!(sanitize_chat(""), None);
    }

    #[test]
    fn caps_by_chars_not_bytes() {
        // 全中文（每字多位元組）：以字元數截到上限，不被切壞。
        let long = "乙".repeat(MAX_CHAT_CHARS + 50);
        let out = sanitize_chat(&long).unwrap();
        assert_eq!(out.chars().count(), MAX_CHAT_CHARS);
    }

    #[test]
    fn keeps_chat_at_exactly_the_cap() {
        let exact = "a".repeat(MAX_CHAT_CHARS);
        assert_eq!(sanitize_chat(&exact).unwrap().chars().count(), MAX_CHAT_CHARS);
    }

    #[test]
    fn lagged_client_is_skipped_not_disconnected() {
        // 跟不上廣播（手機網路抖／分頁背景）只跳過丟掉的快照、繼續轉發，不踢人下線。
        assert_eq!(forward_action(&RecvError::Lagged(7)), ForwardAction::Skip);
    }

    #[test]
    fn closed_channel_stops_forwarding() {
        // 伺服器端關了廣播頻道才結束轉發。
        assert_eq!(forward_action(&RecvError::Closed), ForwardAction::Stop);
    }
}
