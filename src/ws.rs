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
            planet: crate::state::PLANET_HOME.to_string(),
            masteries: crate::class::Masteries::new(),
            guild_tag: None,
            achievements: crate::achievement::AchievementSet::new(),
            kill_count: 0,
            refine_attempt_count: 0,
            equipment: crate::equipment::EquipmentSlots::default(),
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
            planet: crate::state::PLANET_HOME.to_string(),
            masteries: crate::class::Masteries::new(),
            guild_tag: None,
            achievements: crate::achievement::AchievementSet::new(),
            kill_count: 0,
            refine_attempt_count: 0,
            equipment: crate::equipment::EquipmentSlots::default(),
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
                        // 根據存檔等級 + 戰士熟練度校正最大血量（Vitals 不持久化，重連給滿血）。
                        let base_hp = crate::vitals::level_max_hp(p.level());
                        let warrior_bonus = crate::class::hp_bonus(&p.masteries);
                        p.vitals.set_max_hp_full(base_hp + warrior_bonus);
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

    // 轉發任務：把兩條廣播推給這個客戶端。
    // 快照（高頻、會淹）走 tx；聊天（低頻、一次性、漏了就永久看不到）走獨立的 tx_chat，
    // 這樣追快照造成的 Lagged 不會把同段時間捲過的聊天一起丟掉。兩條各自用 forward_action
    // 判斷 Lagged（跳過、不踢人）/ Closed（結束）。
    // ③ 無限世界（切片 C）：從 tx 收到的是 Arc<ServerMsg>，依玩家當下位置做 AOI 剔除後才序列化。
    // tx_direct：單播通道——讓讀取迴圈把僅給本玩家看的訊息（如 TravelResult）推給 forward task。
    let (tx_direct, mut rx_direct) = tokio::sync::mpsc::channel::<String>(16);
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
                                ServerMsg::Snapshot { tick, players, fields, nodes, enemies, daynight, listings, npcs, terrain, world_event, quests, land_plots } => {
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
                                        // 社群任務全服廣播（所有玩家看同一套任務進度）。
                                        quests: quests.clone(),
                                        // 城外地塊全部送出（20 塊量小；地塊都在主城附近）。
                                        land_plots: land_plots.clone(),
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
                        } else {
                            let chat = ServerMsg::Chat { from, text };
                            if let Ok(json) = serde_json::to_string(&chat) {
                                // 走聊天專用頻道，不與高頻快照爭緩衝、不被 Lagged 一起丟。
                                let _ = app.tx_chat.send(json);
                            }
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
                        if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                            let item: crate::inventory::ItemKind = kind.into();
                            // 工具效用(1-D):背包有鎬子/強化鎬就採更多(乘工具倍率)——
                            // 給合成出的工具一個用處,接上「採集→合成工具→採更快」迴圈。
                            let mult = crate::tools::gather_speed_multiplier(&p.inventory);
                            let added = p.inventory.add(item, amount * mult);
                            // 採集得 exp（鼓勵探索）；偵測升級並更新血量上限。
                            let old_level = p.level();
                            p.exp = p.exp.saturating_add(5);
                            if p.level() > old_level {
                                p.vitals.on_level_up(p.level());
                            }
                            p.masteries.gain_artisan(1); // 工匠熟練度：採集節點（ROADMAP 38）
                            tracing::info!(player = %p.name, ?item, added, mult, level = p.level(), "採集入背包+exp");
                        }
                        // 通知社群任務（ROADMAP 27）：採集事件推進進度並廣播完成公告。
                        let item: crate::inventory::ItemKind = kind.into();
                        let completed = app.quests.write().unwrap().on_gather(item);
                        notify_quest_complete(&app, completed);
                        // 每日任務：採集事件（ROADMAP 32）。
                        if let Some(uid) = authed_uid {
                            advance_daily_gather(&app, uid, item, amount, &tx_direct);
                        }
                    }
                }
                Ok(ClientMsg::Craft { recipe_id }) => {
                    // 合成(1-C):用配方自己的穩定 `id` 欄位(crafting 的權威 wire key)查 recipe_id,
                    // 在玩家自己背包上全有全無地合成(夠料才扣料+產出)。產物隨下一張快照回前端。
                    // 走既有 `recipe_by_id`(已測)而非每訊息 serde 重組產物名:免每筆配料一次 Value 配置,
                    // 也不把查找耦死在「id 必等於產物序列化名」上(同產物不同配料就會抓錯)。
                    if let Some(recipe) = crate::crafting::recipe_by_id(&recipe_id) {
                        if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                            let discount = crate::class::crafting_reduction(&p.masteries);
                            if recipe.craft_with_discount(&mut p.inventory, discount) {
                                p.masteries.gain_artisan(2); // 工匠熟練度（ROADMAP 38）
                                tracing::info!(player = %p.name, recipe = %recipe_id, discount, "合成成功");
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
                    if let Some(uid) = authed_uid {
                        let pos = app.players.read().unwrap().get(&uid).map(|p| (p.x, p.y, p.name.clone()));
                        if let Some((px, py, name)) = pos {
                            let ok = {
                                let mut players = app.players.write().unwrap();
                                if let Some(p) = players.get_mut(&uid) {
                                    // qty=0 或量不足都拒絕
                                    qty > 0 && p.inventory.take(item, qty)
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
                                                p.inventory.add(l.item, l.qty);
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
                                p.inventory.add(item, qty);
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
                                if npc::is_within_shop_reach(px, py) {
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
                                    // 查當前浮動收購價（read lock，用完立即釋放）
                                    let now_secs = unix_secs();
                                    let dynamic_price = app.dynamic_prices.read().unwrap()
                                        .current_price(item, base_price, now_secs);

                                    // 扣除背包物品、結算乙太（write lock）
                                    let did_sell = {
                                        let mut players = app.players.write().unwrap();
                                        if let Some(p) = players.get_mut(&id) {
                                            if p.inventory.take(item, qty) {
                                                let earned = dynamic_price.saturating_mul(qty);
                                                let bonus = crate::class::apply_npc_bonus(&p.masteries, earned) - earned;
                                                tracing::info!(player = %p.name, ?item, qty, earned, bonus, dynamic_price, merchant_name, "NPC 收購（浮動價）");
                                                p.ether = p.ether.saturating_add(earned).saturating_add(bonus);
                                                p.masteries.gain_merchant(1); // 商人熟練度（ROADMAP 38）
                                                true
                                            } else {
                                                false
                                            }
                                        } else {
                                            false
                                        }
                                    }; // players write lock 在此釋放

                                    // 記錄賣出量，更新浮動收購價（write lock，與 players 鎖無交疊）
                                    if did_sell {
                                        app.dynamic_prices.write().unwrap()
                                            .record_sale(item, qty, now_secs);
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
                        if !downed && npc::is_within_shop_reach(px, py) {
                            if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                                let old_ether = p.ether;
                                if let Some(new_ether) = npc::buy_from_npc(&mut p.inventory, p.ether, item, qty) {
                                    tracing::info!(player = %p.name, ?item, qty, spent = old_ether - new_ether, "NPC 販售");
                                    p.ether = new_ether;
                                }
                            }
                        }
                    }
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
                            let added = p.inventory.add(item, qty * mult);
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
                Ok(ClientMsg::Attack) => {
                    // 主動攻擊：驗未倒地、冷卻已到期，再打 ATTACK_REACH 內最近的存活敵人。
                    // 鎖序：讀 players（取位置+冷卻） → 寫 enemies（attack_nearest） → 寫 players（設冷卻+掉落）。
                    const ATTACK_COOLDOWN_SECS: f32 = 0.6;
                    let info = app.players.read().unwrap().get(&id).map(|p| {
                        use crate::refinement::{enchant_extra_damage, is_crit_tick};
                        let enchant = p.equipment.weapon_meta.enchant;
                        let attempt = p.kill_count as u64;
                        let base_power = crate::equipment::equipped_weapon_power(&p.equipment)
                            + crate::combat::level_attack_bonus(p.level())
                            + crate::class::combat_bonus(&p.masteries)
                            + enchant_extra_damage(enchant);
                        // 暴擊：每 5 次攻擊有一次雙倍傷害。
                        let power = if enchant == Some(crate::refinement::EnchantKind::CritStrike)
                            && is_crit_tick(attempt) {
                            base_power * 2
                        } else {
                            base_power
                        };
                        (p.x, p.y, p.vitals.is_downed(), p.attack_cooldown, power, enchant)
                    });
                    let Some((px, py, downed, cooldown, power, enchant)) = info else { continue; };
                    if downed || cooldown > 0.0 { continue; }
                    let result = app.enemies.write().unwrap().attack_nearest(px, py, power);
                    // 追蹤討伐的兇名資訊（ROADMAP 42），供後續廣播用
                    let was_notorious = result.as_ref().map(|(_, _, n, _)| *n).unwrap_or(false);
                    if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                        p.attack_cooldown = ATTACK_COOLDOWN_SECS;
                        if let Some((kind, enemy_level, _, Some((item, qty)))) = result {
                            p.inventory.add(item, qty);
                            // 殺怪得 exp（依敵人等級縮放後的難度決定獎勵量；附魔增幅加成）。
                            let base_reward = crate::combat::scaled_exp(kind.exp_reward(), enemy_level);
                            // 討伐兇名精英：exp 翻倍（ROADMAP 42）。
                            let notorious_mult = if was_notorious { 2.0_f32 } else { 1.0_f32 };
                            let reward = (base_reward as f32
                                * crate::refinement::enchant_exp_multiplier(enchant)
                                * notorious_mult) as u32;
                            let old_level = p.level();
                            p.exp = p.exp.saturating_add(reward);
                            if p.level() > old_level {
                                p.vitals.on_level_up(p.level());
                            }
                            // 吸血：擊殺後回復 2 HP。
                            let ls = crate::refinement::enchant_lifesteal_hp(enchant);
                            if ls > 0 { p.vitals.heal(ls); }
                            // 戰士熟練度（ROADMAP 38）：殺怪得 1 XP；首次升到 1 級時補 HP 加成。
                            if p.masteries.gain_warrior(1) && p.masteries.warrior_level() == 1 {
                                let bonus = crate::class::hp_bonus(&p.masteries);
                                if bonus > 0 {
                                    p.vitals.set_max_hp_full(p.vitals.max_hp() + bonus);
                                }
                            }
                            tracing::info!(player = %p.name, ?item, qty, reward, level = p.level(), notorious = was_notorious, "主動攻擊戰利品+exp");
                        }
                    }
                    // 討伐兇名精英全服廣播（ROADMAP 42）
                    if was_notorious {
                        if let Some((kind, _, _, Some(_))) = result {
                            let pname = app.players.read().unwrap()
                                .get(&id).map(|p| p.name.clone()).unwrap_or_default();
                            if !pname.is_empty() {
                                let _ = app.tx_chat.send(format!(
                                    "⚔️ {} 討伐了兇名 {}！全服向英雄致敬！",
                                    pname, kind.display_name()
                                ));
                            }
                        }
                    }
                    // 通知社群任務（ROADMAP 27）：擊殺事件推進進度並廣播完成公告。
                    if let Some((kind, _, _, Some(_))) = result {
                        let completed = app.quests.write().unwrap().on_kill(kind);
                        notify_quest_complete(&app, completed);
                    }
                    // 成就：擊殺計數里程碑（ROADMAP 31）。
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
                        let _ = (kill_count, new_level); // 避免 unused 警告
                        for ach in newly_unlocked {
                            let _ = app.tx_chat.send(format!(
                                "🏆 {} 解鎖成就「{}」！", pname, ach.display_name()
                            ));
                        }
                    }
                    // 每日任務：擊殺事件（ROADMAP 32）。
                    if let (Some(uid), Some((kill_kind, _, _, Some(_)))) = (authed_uid, result) {
                        advance_daily_kill(&app, uid, kill_kind, &tx_direct);
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
                                    p.inventory.add(ItemKind::Ether, 10);
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
                                // 換裝：舊裝備退回背包
                                p.inventory.add(old, 1);
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
                            p.inventory.add(removed, 1);
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
                                let cost = crate::class::apply_travel_discount(&p.masteries, base_cost);
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
                                let cost = crate::class::apply_travel_discount(&p.masteries, TRAVEL_ETHER_COST_CRIMSON);
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
                                let cost = crate::class::apply_travel_discount(&p.masteries, TRAVEL_ETHER_COST_VOID);
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
                                let cost = crate::class::apply_travel_discount(&p.masteries, TRAVEL_ETHER_COST_AETHER);
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
                                let cost = crate::class::apply_travel_discount(&p.masteries, TRAVEL_ETHER_COST_ORIGIN);
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
                                let cost = crate::class::apply_travel_discount(&p.masteries, TRAVEL_ETHER_COST);
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

                // ── 公會系統（ROADMAP 29）──────────────────────────────────────────
                Ok(ClientMsg::CreateGuild { name, tag }) => {
                    // 建立公會：需登入 + 乙太 ≥ 50；成功後從玩家扣乙太、更新 guild_tag。
                    if let Some(uid) = authed_uid {
                        let result = {
                            let ether = app.players.read().unwrap().get(&uid).map(|p| p.ether).unwrap_or(0);
                            if ether < crate::guild::GUILD_CREATE_COST {
                                Err(format!("乙太不足（建立公會需要 {} 乙太）", crate::guild::GUILD_CREATE_COST))
                            } else {
                                app.guilds.write().unwrap().create(uid, name, tag)
                            }
                        };
                        match result {
                            Ok(gid) => {
                                let guild_tag = app.guilds.read().unwrap().tag_of(uid);
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
                        let result = app.guilds.write().unwrap().join(guild_id, uid);
                        match result {
                            Ok(()) => {
                                let guild_tag = app.guilds.read().unwrap().tag_of(uid);
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
                        let result = app.guilds.write().unwrap().leave(uid);
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
                            let result = app.guilds.write().unwrap().donate(uid, amount);
                            match result {
                                Ok(_new_treasury) => {
                                    if let Some(p) = app.players.write().unwrap().get_mut(&uid) {
                                        p.ether = p.ether.saturating_sub(amount);
                                    }
                                    let gid = app.guilds.read().unwrap().guild_of(uid);
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
                    let store = app.guilds.read().unwrap();
                    let briefs: Vec<crate::protocol::GuildBrief> = store.brief_list()
                        .into_iter()
                        .map(|b| crate::protocol::GuildBrief {
                            id: b.id,
                            name: b.name,
                            tag: b.tag,
                            member_count: b.member_count,
                            treasury: b.treasury,
                        })
                        .collect();
                    drop(store);
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

/// 依 guild_id 與 player_id 建立 GuildView（ROADMAP 29）。
fn build_guild_view(app: &AppState, player_id: Uuid, guild_id: Uuid) -> Option<crate::protocol::GuildView> {
    let store = app.guilds.read().unwrap();
    let g = store.get(guild_id)?;
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
    let pname = {
        let mut players = app.players.write().unwrap();
        if let Some(p) = players.get_mut(&uid) {
            p.ether = p.ether.saturating_add(crate::daily_quest::DAILY_TASK_ETHER_REWARD);
            let old_level = p.level();
            p.exp = p.exp.saturating_add(crate::daily_quest::DAILY_TASK_EXP_REWARD);
            if p.level() > old_level {
                p.vitals.on_level_up(p.level());
            }
            p.name.clone()
        } else {
            String::new()
        }
    };
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
                    app.positions.remember(id, player.x, player.y, player.ether, player.wallet.expansions(), player.exp, player.masteries);
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
                .flush_one(id, &player.name, &player.species, player.x, player.y, player.ether, player.wallet.expansions(), player.exp, player.masteries)
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
