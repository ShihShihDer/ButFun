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
use crate::protocol::{ClientMsg, ServerMsg};
use crate::state::{AppState, Input, Player, WORLD_HEIGHT, WORLD_WIDTH};

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
                                ServerMsg::Snapshot { tick, players, fields, nodes, enemies, daynight } => {
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
                            tracing::info!(player = %p.name, ?item, added, mult, "採集入背包");
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
                    app.positions.remember(id, player.x, player.y, player.ether, player.wallet.expansions());
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
                .flush_one(id, &player.name, &player.species, player.x, player.y, player.ether, player.wallet.expansions())
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
