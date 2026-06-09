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
                        // 根據存檔等級校正最大血量（Vitals 不持久化，重連給滿血）。
                        p.vitals.set_max_hp_full(crate::vitals::level_max_hp(p.level()));
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
                                ServerMsg::Snapshot { tick, players, fields, nodes, enemies, daynight, listings, npcs, terrain } => {
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
                        let from = app
                            .players
                            .read()
                            .unwrap()
                            .get(&id)
                            .map(|p| p.name.clone())
                            .unwrap_or_else(|| player.name.clone());
                        let chat = ServerMsg::Chat { from, text };
                        if let Ok(json) = serde_json::to_string(&chat) {
                            // 走聊天專用頻道，不與高頻快照爭緩衝、不被 Lagged 一起丟。
                            let _ = app.tx_chat.send(json);
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
                            p.ether = p.ether.saturating_add(ether);
                            tracing::info!(player = %p.name, ether = p.ether, "農地收成乙太");
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
                            tracing::info!(player = %p.name, ?item, added, mult, level = p.level(), "採集入背包+exp");
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
                            if recipe.craft(&mut p.inventory) {
                                tracing::info!(player = %p.name, recipe = %recipe_id, "合成成功");
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
                    // 向 NPC 商人賣出物品：驗距離 + 未倒地 + 在收購清單 + 背包有貨 → 換乙太。
                    let player_pos = app.players.read().unwrap().get(&id).map(|p| (p.x, p.y, p.vitals.is_downed()));
                    if let Some((px, py, downed)) = player_pos {
                        if !downed && npc::is_within_shop_reach(px, py) {
                            if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                                if let Some(new_ether) = npc::sell_to_npc(&mut p.inventory, p.ether, item, qty) {
                                    tracing::info!(player = %p.name, ?item, qty, earned = new_ether - p.ether, "NPC 收購");
                                    p.ether = new_ether;
                                }
                            }
                        }
                    }
                }
                Ok(ClientMsg::ShopBuy { item, qty }) => {
                    // 向 NPC 商人購買物品：驗距離 + 未倒地 + 在販售清單 + 乙太足夠 → 取得物品。
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
                        (p.x, p.y, p.vitals.is_downed(), p.attack_cooldown,
                         crate::combat::weapon_power(&p.inventory)
                             + crate::combat::level_attack_bonus(p.level()))
                    });
                    let Some((px, py, downed, cooldown, power)) = info else { continue; };
                    if downed || cooldown > 0.0 { continue; }
                    let result = app.enemies.write().unwrap().attack_nearest(px, py, power);
                    if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                        p.attack_cooldown = ATTACK_COOLDOWN_SECS;
                        if let Some((kind, Some((item, qty)))) = result {
                            p.inventory.add(item, qty);
                            // 殺怪得 exp（依敵人難度決定獎勵量）；偵測升級並更新血量上限。
                            let reward = kind.exp_reward();
                            let old_level = p.level();
                            p.exp = p.exp.saturating_add(reward);
                            if p.level() > old_level {
                                p.vitals.on_level_up(p.level());
                            }
                            tracing::info!(player = %p.name, ?item, qty, reward, level = p.level(), "主動攻擊戰利品+exp");
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
                Ok(ClientMsg::TravelToPlanet { planet }) => {
                    // 星際旅行（ROADMAP 20）：傳送玩家到指定星球。
                    use crate::state::{
                        PLANET_HOME, PLANET_VERDANT,
                        VERDANT_SPAWN_X, VERDANT_SPAWN_Y,
                        TRAVEL_ETHER_COST,
                    };
                    use crate::protocol::ServerMsg;
                    let result = if let Some(p) = app.players.write().unwrap().get_mut(&id) {
                        match p.can_travel_to(&planet) {
                            Err(msg) => Some(ServerMsg::TravelResult {
                                ok: false,
                                planet: p.planet.clone(),
                                message: msg,
                            }),
                            Ok(()) if planet == PLANET_VERDANT => {
                                p.ether -= TRAVEL_ETHER_COST;
                                p.planet = PLANET_VERDANT.to_string();
                                p.x = VERDANT_SPAWN_X;
                                p.y = VERDANT_SPAWN_Y;
                                tracing::info!(player = %p.name, "星際旅行：抵達翠幽星");
                                Some(ServerMsg::TravelResult {
                                    ok: true,
                                    planet: PLANET_VERDANT.to_string(),
                                    message: "歡迎來到翠幽星！茂密叢林的古老氣息撲面而來⋯⋯".to_string(),
                                })
                            }
                            Ok(()) => {
                                p.ether -= TRAVEL_ETHER_COST;
                                p.planet = PLANET_HOME.to_string();
                                let (hx, hy) = crate::positions::default_spawn();
                                p.x = hx;
                                p.y = hy;
                                tracing::info!(player = %p.name, "星際旅行：返回故鄉");
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
                        let _ = tx_direct.send(
                            serde_json::to_string(&msg).unwrap_or_default(),
                        ).await;
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

/// 玩家離線清理。先放掉這條連線；只有當這是該玩家的**最後一條**連線（同帳號其餘分頁
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
                    app.positions.remember(id, player.x, player.y, player.ether, player.wallet.expansions(), player.exp);
                    // 背包同樣在鎖內更新 cache。
                    app.inventories.remember(id, &player.inventory);
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
                .flush_one(id, &player.name, &player.species, player.x, player.y, player.ether, player.wallet.expansions(), player.exp)
                .await;
            app.inventories.flush_one(id, &player.inventory).await;
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
