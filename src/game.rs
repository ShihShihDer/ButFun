//! 權威遊戲迴圈：固定 tick 整合所有玩家位置，廣播世界快照。

use std::time::Duration;

use crate::dynamic_price::{DynamicPriceMarket, unix_secs};
use crate::npc::{NPC_BUY_LIST, NPC_SELL_LIST, VERDANT_BUY_LIST, VERDANT_SELL_LIST, CRIMSON_BUY_LIST, CRIMSON_SELL_LIST, VOID_BUY_LIST, VOID_SELL_LIST, AETHER_BUY_LIST, AETHER_SELL_LIST, ORIGIN_BUY_LIST, ORIGIN_SELL_LIST, merchant_pos, verdant_merchant_pos, crimson_merchant_pos, void_merchant_pos, aether_merchant_pos, origin_merchant_pos};
use crate::protocol::{EnemyView, FieldView, ListingView, NodeView, NpcView, ServerMsg, ShopCatalogEntry, TileDeltaView};
use crate::combat::EnemyKind;
use crate::state::AppState;

/// 把一個商人的收購清單轉成帶浮動收購價的 ShopCatalogEntry 列表。
fn build_dynamic_buy_list(
    buy_list: &[crate::npc::ShopEntry],
    market: &DynamicPriceMarket,
    now_secs: u64,
) -> Vec<ShopCatalogEntry> {
    buy_list.iter().map(|e| {
        let price_per = market.current_price(e.item, e.price_per, now_secs);
        let trend = market.current_trend(e.item, now_secs).to_string();
        ShopCatalogEntry { item: e.item, price_per, trend }
    }).collect()
}

/// 把販售清單轉成 ShopCatalogEntry（販售價固定不浮動，趨勢固定 stable）。
fn build_static_sell_list(sell_list: &[crate::npc::ShopEntry]) -> Vec<ShopCatalogEntry> {
    sell_list.iter().map(|e| ShopCatalogEntry {
        item: e.item,
        price_per: e.price_per,
        trend: "stable".to_string(),
    }).collect()
}

/// 每秒 tick 數（伺服器模擬頻率）。
const TICK_HZ: f32 = 15.0;

/// flush 時從玩家快照收下的「線上已登入玩家狀態列」。與 `PositionStore::OnlinePlayerRow` 完全對齊。
type OnlinePlayerRow = crate::positions::OnlinePlayerRow;

/// 這個 tick 要不要建構並廣播世界快照。
/// 沒有任何訂閱者（連線的客戶端）時回 false——自走營運的離峰時段沒人連線,
/// 每 tick 把整個世界轉成 JSON 純屬浪費。判斷抽成純函式以便測試(同 `ws::forward_action` 慣例)。
/// 注意:世界本身的推進(日夜/農地/節點/敵人/玩家位置與生命)與此無關、每 tick 必跑,
/// 這裡只決定「要不要序列化送出」。
fn should_broadcast(receiver_count: usize) -> bool {
    receiver_count > 0
}

/// 落地(flush)節律,以 tick 數表示。有客戶端連線時每 10 秒一次,如常保住線上玩家進度。
/// 沒人連線的離峰時段拉長到每 60 秒一次——此時只有背景世界(離線玩家農地成長、日夜時鐘)
/// 在變,把 checkpoint 拉疏 6× 省離峰 CPU+DB 寫(`flush_all` 每次都 clone 全部歷來農地再
/// upsert、外加寫日夜)。代價僅是離峰重啟最多丟約 60 秒的離線成長/時鐘 granularity,沒人在看、
/// 返場玩家也察覺不到。**線上玩家進度不受影響**:只要有人連線(`want_broadcast` 為真)就是
/// 10 秒節律。延續「沒人看就別白做」的離峰優化路線(同 `should_broadcast`),抽成純函式以便測試。
fn flush_interval_ticks(has_subscribers: bool) -> u64 {
    let base = TICK_HZ as u64;
    if has_subscribers {
        base * 10
    } else {
        base * 60
    }
}

/// 啟動遊戲迴圈，常駐執行。
pub fn spawn(app: AppState) {
    tokio::spawn(async move {
        let dt = 1.0 / TICK_HZ;
        let mut interval = tokio::time::interval(Duration::from_secs_f32(dt));
        let mut tick: u64 = 0;

        loop {
            interval.tick().await;
            tick += 1;

            // 這個 tick 到底要不要建構快照?在 tick 開頭一次決定,讓底下農地/節點/敵人/日夜的
            // view 建構全都據此跳過——沒人連線的離峰時段,世界照常推進,但不再每 tick 白白配置
            // 那幾個 view Vec + clone(上一輪的離峰優化只省了最後的 JSON 序列化,view 卻照建)。
            // 新訂閱者本就等下一個 tick 才收第一筆快照,故此處一次判定不改變既有延遲語意。
            let want_broadcast = should_broadcast(app.tx.receiver_count());

            // 先推進日夜時鐘，取得當下亮度決定作物成長速度（短暫持鎖，不跨 await）。
            // 時鐘無條件前進;view 只在要廣播時才取。
            let (daynight_view, growth_rate, is_night) = {
                let mut daynight = app.daynight.write().unwrap();
                daynight.advance(dt);
                let is_night = daynight.phase() == crate::daynight::Phase::Night;
                let view = if want_broadcast {
                    Some(daynight.view())
                } else {
                    None
                };
                (view, daynight.growth_rate(), is_night)
            };

            // 推進所有玩家農地的成長：依日夜成長倍率縮放 dt——白天亮、長得快，夜裡暗、
            // 放慢（0-G「隨日夜成長」）。濕度也一併縮放，故每次澆水的總成長量不變、
            // 只有牆鐘速度隨日夜變化。同時把每塊地轉成快照、並戳上擁有者 id（`Field`
            // 自己不知道屬於誰，由這層持有的 `user_id → Field` 對映補上）。短暫持鎖，不跨 await。
            // 成長無條件推進(每塊地 tick);view 只在要廣播時才在同一把鎖內多走一趟建。
            let field_views: Vec<FieldView> = {
                let mut fields = app.fields.write().unwrap();
                for (_owner, field) in fields.iter_mut() {
                    field.tick(dt * growth_rate);
                }
                // 公共農地與個人地塊同步成長，廣播時以 owner=nil 加入列表讓前端辨識。
                let pub_view = {
                    let mut pf = app.pub_field.write().unwrap();
                    pf.tick(dt * growth_rate);
                    if want_broadcast {
                        let mut v = pf.view();
                        v.owner = uuid::Uuid::nil();
                        Some(v)
                    } else {
                        None
                    }
                };
                if want_broadcast {
                    let mut views: Vec<FieldView> = fields
                        .iter()
                        .map(|(owner, field)| {
                            let mut v = field.view();
                            v.owner = *owner;
                            v
                        })
                        .collect();
                    if let Some(pv) = pub_view {
                        views.push(pv);
                    }
                    views
                } else {
                    Vec::new()
                }
            };

            // 推進採集節點重生（採空的倒數補耐久,其餘 no-op）。重生無條件跑;view 只在廣播時建。
            // ③ 無限世界: 先確保玩家周圍區塊已載入。
            let node_views: Vec<NodeView> = {
                let mut nodes = app.nodes.write().unwrap();
                {
                    let players = app.players.read().unwrap();
                    for p in players.values() {
                        nodes.ensure_chunks_around(p.x, p.y, 1000.0);
                    }
                }
                nodes.tick(dt);
                if want_broadcast {
                    nodes
                        .nodes()
                        .iter()
                        .map(|p| NodeView {
                            kind: p.node.kind(),
                            x: p.x,
                            y: p.y,
                            remaining: p.node.remaining(),
                            harvestable: p.node.is_harvestable(),
                        })
                        .collect()
                } else {
                    Vec::new()
                }
            };

            // C-3 碰撞:先快照 tile deltas（取讀鎖即放），供敵人與玩家移動共用，且不與
            // Dig handler（tile.write→players.write）的鎖序衝突（這裡 tile 讀鎖先放，再各自取寫鎖）。
            let tile_deltas_snap: std::collections::HashMap<(i32, i32, u8, u8), world_core::TileKind> = {
                let tw = app.tile_world.read().unwrap();
                tw.deltas().clone()
            };

            // 敵人移動需要玩家座標:先讀 players(短暫讀鎖)收集**沒被打趴**的玩家位置快照,
            // 放開後再持 enemies 寫鎖推進——避免在敵人寫鎖內再去鎖玩家表造成巢狀鎖。
            // 只餵非倒下玩家(倒下玩家休息中、不被追擊,比照下方戰鬥結算略過倒下者)。
            let chase_targets: Vec<(f32, f32)> = {
                let players = app.players.read().unwrap();
                players
                    .values()
                    .filter(|p| !p.vitals.is_downed())
                    .map(|p| (p.x, p.y))
                    .collect()
            };

            // 推進敵人:重生倒數(被打倒的復活)+ 移動(巡邏 / 追擊走近的玩家)。兩者無條件跑;
            // view 只在廣播時建。怪會動起來——撲向玩家、沒人時漂回家,世界因此活起來。
            // ③ 無限世界: 先確保玩家周圍區塊已載入。
            let enemy_views: Vec<EnemyView> = {
                let mut enemies = app.enemies.write().unwrap();
                {
                    let players = app.players.read().unwrap();
                    for p in players.values() {
                        enemies.ensure_chunks_around(p.x, p.y, 1000.0);
                    }
                }
                enemies.tick(dt);
                // C-3:敵人也吃地形碰撞（用同一份 tile deltas 快照），不再穿牆。
                // 夜間危機：夜裡敵人追擊速度加成（is_night）。
                enemies.advance(dt, &chase_targets, is_night, |x: f32, y: f32| {
                    let (cx, cy, tx, ty) = crate::tiles::world_to_cell(x, y);
                    tile_deltas_snap
                        .get(&(cx, cy, tx, ty))
                        .copied()
                        .unwrap_or_else(|| world_core::tile_kind_at(x as f64, y as f64))
                        != world_core::TileKind::Empty
                });
                if want_broadcast {
                    enemies
                        .enemies()
                        .iter()
                        .map(|p| EnemyView {
                            kind: p.enemy.kind(),
                            x: p.x,
                            y: p.y,
                            level: p.level,
                            hp: p.enemy.remaining_hp(),
                            max_hp: p.enemy.max_hp(),
                            alive: p.enemy.is_alive(),
                            notorious: p.level >= p.base_level.saturating_add(3),
                        })
                        .collect()
                } else {
                    Vec::new()
                }
            };

            // 敵人反擊（每秒一次）：玩家在攻擊範圍內時，敵人自動造成傷害——
            // 站著不動不打怪也會被打，逼玩家主動出擊或趕緊走開。
            // 避免巢狀鎖：先讀玩家位置 → 查敵人威脅 → 把傷害套回玩家，三步各持一把鎖。
            if tick % (TICK_HZ as u64) == 0 {
                let positions: Vec<(uuid::Uuid, f32, f32, bool)> = {
                    let players = app.players.read().unwrap();
                    players
                        .values()
                        .map(|p| (p.id, p.x, p.y, p.vitals.is_downed()))
                        .collect()
                };
                let mut dmgs: Vec<(uuid::Uuid, u32)> = Vec::new();
                {
                    let enemies = app.enemies.read().unwrap();
                    for (pid, px, py, downed) in &positions {
                        if *downed { continue; }
                        let threat = enemies.threat_at(*px, *py);
                        if threat > 0 {
                            dmgs.push((*pid, threat));
                        }
                    }
                }
                // 位置→玩家的對映，供後續步驟查找倒地玩家座標
                let mut pos_map: std::collections::HashMap<uuid::Uuid, (f32, f32)> = positions
                    .iter().map(|(id, x, y, _)| (*id, (*x, *y))).collect();
                let mut downed_positions: Vec<(f32, f32)> = Vec::new();
                if !dmgs.is_empty() {
                    let mut players = app.players.write().unwrap();
                    for (pid, dmg) in dmgs {
                        if let Some(p) = players.get_mut(&pid) {
                            // 護甲減傷：讀裝備槽（ROADMAP 36）+ 寵物加成（ROADMAP 46）。
                            let defense = crate::equipment::equipped_armor_defense(&p.equipment)
                                + p.pet.map(|pk| pk.bonus_defense()).unwrap_or(0);
                            let actual_dmg = dmg.saturating_sub(defense);
                            if actual_dmg > 0 && p.vitals.take_damage(actual_dmg) {
                                tracing::info!(player = %p.name, defense, actual_dmg, "被敵人打趴，休息復原中");
                                if let Some(&(px, py)) = pos_map.get(&pid) {
                                    downed_positions.push((px, py));
                                }
                            }
                        }
                    }
                }
                // 玩家倒地 → 最近敵人升一級（ROADMAP 42）。
                // 分開持 enemies 寫鎖，避免與上方 players 寫鎖同時持有。
                if !downed_positions.is_empty() {
                    let mut newly_notorious: Vec<crate::enemy_field::EnemyLevelUpResult> = Vec::new();
                    {
                        let mut enemies = app.enemies.write().unwrap();
                        for (px, py) in downed_positions {
                            if let Some(r) = enemies.level_up_nearest_killer(px, py) {
                                if r.newly_notorious {
                                    newly_notorious.push(r);
                                }
                            }
                        }
                    }
                    for r in newly_notorious {
                        let name = r.kind.display_name();
                        let _ = app.tx_chat.send(format!(
                            "⚠️ 一隻兇名 Lv.{} {} 正在肆虐！勇者可前往討伐，擊倒有豐厚獎勵！",
                            r.new_level, name
                        ));
                    }
                }
                // 清理暫時用的 pos_map 避免 unused 警告
                let _ = pos_map.drain();
            }

            // 整合位置 + 推進生命回復（權威模擬,每 tick 必跑,與有無觀眾無關;短暫持鎖,不跨 await）。
            // （tile_deltas_snap 已在敵人段前快照，玩家碰撞沿用同一份。）
            {
                let mut players = app.players.write().unwrap();
                for p in players.values_mut() {
                    p.step(dt, |x: f32, y: f32| {
                        let (cx, cy, tx, ty) = crate::tiles::world_to_cell(x, y);
                        let kind = tile_deltas_snap
                            .get(&(cx, cy, tx, ty))
                            .copied()
                            .unwrap_or_else(|| world_core::tile_kind_at(x as f64, y as f64));
                        kind != world_core::TileKind::Empty
                    });
                    // 主動攻擊冷卻倒數：每 tick 遞減，讓下次攻擊請求能被接受。
                    if p.attack_cooldown > 0.0 {
                        p.attack_cooldown = (p.attack_cooldown - dt).max(0.0);
                    }
                    // 主動技能冷卻倒數（ROADMAP 45）。
                    p.skill_cooldowns.tick(dt);
                    // 釣魚冷卻倒數（ROADMAP 47）。
                    if p.fish_cooldown > 0.0 {
                        p.fish_cooldown = (p.fish_cooldown - dt).max(0.0);
                    }
                    let was_downed = p.vitals.is_downed();
                    p.vitals.tick(dt); // 離戰一陣子自動回血 / 被打趴的休息倒數
                    // 從倒地復原的那一 tick：傳回新手村（公共農地中央）。
                    if was_downed && p.vitals.is_alive() {
                        let (sx, sy) = crate::positions::default_spawn();
                        p.x = sx;
                        p.y = sy;
                        tracing::info!(player = %p.name, "從倒地復原，傳回新手村");
                    }
                }
            }

            // 宇宙裂縫事件（ROADMAP 26）：推進事件計時器；觸發時注入守護者 + 廣播聊天公告。
            {
                let triggered = {
                    let mut we = app.world_event.write().unwrap();
                    we.tick(dt)
                };
                if let Some((rx, ry)) = triggered {
                    // 防呆：事件座標若落在城鎮保護圈內就不注入怪（座標清單有測試釘住
                    // 在圈外，這裡是最後一道防線——城裡絕不能憑空冒出怪，線上踩過）。
                    if world_core::town_protected_at(rx as f64, ry as f64) {
                        tracing::warn!(x = rx, y = ry, "事件座標在城鎮保護圈內，跳過注入守護者");
                    } else {
                        app.enemies.write().unwrap()
                            .inject_event_enemy(rx, ry, EnemyKind::RiftGuardian);
                    }
                    // 全服廣播聊天公告。
                    let msg = format!(
                        "🌀 宇宙裂縫在 ({:.0}, {:.0}) 附近開啟！裂縫守護者現身！快去獵殺！",
                        rx, ry
                    );
                    let _ = app.tx_chat.send(msg);
                    tracing::info!(x = rx, y = ry, "宇宙裂縫觸發，裂縫守護者注入");
                }
            }

            // AI 導演層＋獸潮攻城（ROADMAP 44）：低頻導演 tick，觸發時注入怪波＋廣播公告。
            {
                let cmds = app.director.write().unwrap().tick(dt);
                for cmd in cmds {
                    match cmd {
                        crate::director::DirectorCmd::AnnounceHorde { site_x, site_y, site_label, wave } => {
                            // 注入第一波怪物（全部在保護圈外確認）。
                            let mut enemies = app.enemies.write().unwrap();
                            let mut injected = 0u32;
                            for (wx, wy, kind) in wave {
                                if world_core::town_protected_at(wx as f64, wy as f64) {
                                    tracing::warn!(x = wx, y = wy, "獸潮波次位置在保護圈內，跳過");
                                    continue;
                                }
                                enemies.inject_event_enemy(wx, wy, kind);
                                injected += 1;
                            }
                            tracing::info!(site = site_label, injected, "獸潮廣播＋注入第一波怪物");
                            let _ = app.tx_chat.send(format!(
                                "⚔️ 獸潮來襲！大批怪物正聚集在{}！\
                                 30 秒後衝擊城門——出城迎戰或守在城牆輸出！",
                                site_label
                            ));
                        }
                        crate::director::DirectorCmd::SiegeStart { site_label } => {
                            tracing::info!(site = site_label, "獸潮攻城開始");
                            let _ = app.tx_chat.send(format!(
                                "⚔️ 獸潮衝擊{}！打倒 {} 隻怪物可為全服贏得獎勵！",
                                site_label, crate::director::HORDE_VICTORY_KILLS
                            ));
                        }
                        crate::director::DirectorCmd::HordeVictory { site_label, kills } => {
                            tracing::info!(site = site_label, kills, "獸潮被玩家打退，全服獎勵");
                            let _ = app.tx_chat.send(format!(
                                "🎉 玩家們成功打退{}的獸潮！（共斬殺 {} 隻）\
                                 全服每位登入玩家獲得 {} 乙太！",
                                site_label, kills, crate::director::HORDE_VICTORY_ETHER
                            ));
                            // 全服所有線上玩家各得勝利獎勵乙太（與社群任務獎勵機制相同）。
                            for p in app.players.write().unwrap().values_mut() {
                                p.ether = p.ether.saturating_add(crate::director::HORDE_VICTORY_ETHER);
                            }
                        }
                        crate::director::DirectorCmd::HordeRetreat { site_label } => {
                            tracing::info!(site = site_label, "獸潮時間耗盡，自行退去");
                            let _ = app.tx_chat.send(format!(
                                "😔 {}的獸潮自行退去了…下次要更快打退！",
                                site_label
                            ));
                        }
                    }
                }
            }

            // 全服社群任務（ROADMAP 27）：推進計時器；換輪時廣播公告。
            {
                let reset = app.quests.write().unwrap().tick(dt);
                if reset {
                    let _ = app.tx_chat.send(
                        "📋 任務換輪！三條新的全服探索任務已開啟，快去完成吧！".to_string()
                    );
                }
            }

            // 牧場系統（ROADMAP 48）：推進所有有雞地塊的下蛋計時器。
            app.ranch.write().unwrap().tick(dt);

            // 收集市場掛單（AOI 剔除在 ws.rs 做，這裡只收全部）。
            let listing_views: Vec<ListingView> = if want_broadcast {
                app.market
                    .read()
                    .unwrap()
                    .all()
                    .map(|l| ListingView {
                        id: l.id,
                        seller_id: l.seller_id,
                        seller_name: l.seller_name.clone(),
                        item: l.item,
                        qty: l.qty,
                        price_per: l.price_per,
                        x: l.x,
                        y: l.y,
                    })
                    .collect()
            } else {
                Vec::new()
            };

            // 廣播快照——只在有訂閱者時(tick 開頭已判定的 want_broadcast)才建構。
            // ③ 無限世界（切片 C）：傳出 Arc<ServerMsg> 原始結構，不在此序列化。
            if want_broadcast {
                let snapshot = {
                    let players = app.players.read().unwrap();
                    // 每次快照帶上 NPC 目錄（六大商人，收購價套用浮動市場價格）。
                    let now_secs = unix_secs();
                    let dm = app.dynamic_prices.read().unwrap();
                    let (mx, my) = merchant_pos();
                    let home_npc = NpcView {
                        x: mx,
                        y: my,
                        buy_list: build_dynamic_buy_list(NPC_BUY_LIST, &dm, now_secs),
                        sell_list: build_static_sell_list(NPC_SELL_LIST),
                    };
                    let (vmx, vmy) = verdant_merchant_pos();
                    let verdant_npc = NpcView {
                        x: vmx,
                        y: vmy,
                        buy_list: build_dynamic_buy_list(VERDANT_BUY_LIST, &dm, now_secs),
                        sell_list: build_static_sell_list(VERDANT_SELL_LIST),
                    };
                    let (cmx, cmy) = crimson_merchant_pos();
                    let crimson_npc = NpcView {
                        x: cmx,
                        y: cmy,
                        buy_list: build_dynamic_buy_list(CRIMSON_BUY_LIST, &dm, now_secs),
                        sell_list: build_static_sell_list(CRIMSON_SELL_LIST),
                    };
                    let (vmx2, vmy2) = void_merchant_pos();
                    let void_npc = NpcView {
                        x: vmx2,
                        y: vmy2,
                        buy_list: build_dynamic_buy_list(VOID_BUY_LIST, &dm, now_secs),
                        sell_list: build_static_sell_list(VOID_SELL_LIST),
                    };
                    let (amx, amy) = aether_merchant_pos();
                    let aether_npc = NpcView {
                        x: amx,
                        y: amy,
                        buy_list: build_dynamic_buy_list(AETHER_BUY_LIST, &dm, now_secs),
                        sell_list: build_static_sell_list(AETHER_SELL_LIST),
                    };
                    let (omx, omy) = origin_merchant_pos();
                    let origin_npc = NpcView {
                        x: omx,
                        y: omy,
                        buy_list: build_dynamic_buy_list(ORIGIN_BUY_LIST, &dm, now_secs),
                        sell_list: build_static_sell_list(ORIGIN_SELL_LIST),
                    };
                    drop(dm);
                    ServerMsg::Snapshot {
                        tick,
                        players: players.values().map(|p| p.view()).collect(),
                        fields: field_views,
                        nodes: node_views,
                        enemies: enemy_views,
                        daynight: daynight_view.expect("want_broadcast 時必有 daynight_view"),
                        listings: listing_views,
                        npcs: vec![home_npc, verdant_npc, crimson_npc, void_npc, aether_npc, origin_npc],
                        // C-2 起：把 TileWorld 中所有玩家挖掘後的差異帶入快照。
                        // delta 稀疏（只存偏離確定性生成的格），ws.rs 轉發時再依 AOI 剔除。
                        terrain: {
                            let tw = app.tile_world.read().unwrap();
                            tw.deltas().iter().map(|(&(cx, cy, tx, ty), &kind)| {
                                TileDeltaView { cx, cy, tx, ty, kind: kind.into() }
                            }).collect()
                        },
                        world_event: app.world_event.read().unwrap().view(),
                        horde_event: app.director.read().unwrap().view(),
                        quests: crate::protocol::quests_view(&app.quests.read().unwrap()),
                        land_plots: {
                            let registry = app.land_plots.read().unwrap();
                            let players = app.players.read().unwrap();
                            // 查名字：先從線上玩家找，再從 UserStore 找（含離線玩家）
                            registry.all_plots_view(|uid| {
                                players.get(&uid).map(|p| p.name.clone())
                                    .or_else(|| app.users.get(uid).map(|u| u.name))
                            })
                        },
                        // 牧場狀態（ROADMAP 48）：只送有雞或有蛋的地塊。
                        ranch_plots: app.ranch.read().unwrap().all_active_views(),
                    }
                };
                let _ = app.tx.send(std::sync::Arc::new(snapshot));
            }

            // 定期把「線上已登入玩家」的位置 + 乙太快照落地。
            // 先前只有玩家離線時才記,線上玩家撐不過 server 重啟（換版）——乙太會歸零。
            // 這裡讓線上玩家的狀態也持續落地,重啟後重連即帶回。
            // 只記已登入玩家（id 在 users 裡）；訪客 id 隨機、不記,避免 cache 無界成長。
            // 節律隨有無連線變化(`flush_interval_ticks`):有人連線維持 10 秒,離峰無人連線
            // 拉長到 60 秒,省離峰白做的 clone+upsert(同上面只在有觀眾才建 view 的思路)。
            if tick % flush_interval_ticks(want_broadcast) == 0 {
                flush_all(&app).await;
            }
        }
    });
}

/// 把全部需跨重啟保留的狀態落地一次:已登入玩家的位置/背包/乙太、全部農地、日夜時刻。
/// 由遊戲迴圈每 10 秒呼叫一次,也由優雅關機(收到 SIGTERM/Ctrl-C)在退出前最後呼叫一次——
/// 否則換版重啟(deploy 送 SIGTERM)會丟掉上次週期 flush 之後、線上玩家最多約 10 秒的進度
/// (新賺的乙太、移動、剛採/合成的道具、農地成長)。多 flush 永遠安全:寫的是當下快照、冪等 upsert。
pub async fn flush_all(app: &AppState) {
    // 同一把 read 鎖內一併收位置與背包,兩者快照來自同一瞬間、不會錯位。
    let (online, inventories, equipment_rows): (
        Vec<OnlinePlayerRow>,
        Vec<(uuid::Uuid, crate::inventory::Inventory)>,
        Vec<(uuid::Uuid, crate::equipment::EquipmentSlots)>,
    ) = {
        let players = app.players.read().unwrap();
        let authed: Vec<_> = players
            .values()
            .filter(|p| app.users.get(p.id).is_some())
            .collect();
        (
            authed
                .iter()
                .map(|p| (p.id, p.name.clone(), p.species.clone(), p.x, p.y, p.ether, p.wallet.expansions(), p.exp, p.masteries))
                .collect(),
            authed.iter().map(|p| (p.id, p.inventory.clone())).collect(),
            authed.iter().map(|p| (p.id, p.equipment.clone())).collect(),
        )
    };
    if !online.is_empty() {
        // 先更新行程內 cache（同步,供重連 recall）,再非同步 upsert 到 Postgres。
        app.positions
            .remember_all(online.iter().map(|(id, _, _, x, y, e, we, exp, m)| (*id, *x, *y, *e, *we, *exp, *m)));
        app.positions.flush_online(&online).await;
        app.inventories.remember_all(inventories.iter().cloned());
        app.inventories.flush_online(&inventories).await;
        // 裝備槽定期落地（ROADMAP 36）。
        app.inventories.remember_all_equipment(equipment_rows.iter().cloned());
        app.inventories.flush_equipment_online(&equipment_rows).await;
    }

    // 農地一併落地（Phase 0-E）。與位置/背包不同:離線玩家的地仍在世界裡繼續長
    // （上面 field tick 推進「全部」地），所以這裡快照**全部**農地、不限線上,讓離線
    // 期間的成長也撐得過重啟。量級＝歷來已登入玩家數（有界,同 positions）。每塊地的
    // plot 序號由 PlotRegistry 查、一起存好,重啟才能用 reseat 安置回正確 origin、
    // 並用 from_saved 重建序號歸屬。
    let field_rows: Vec<(uuid::Uuid, usize, crate::field::Field)> = {
        let fields = app.fields.read().unwrap();
        fields
            .iter()
            .filter_map(|(uid, f)| app.plots.index_of(*uid).map(|idx| (*uid, idx, f.clone())))
            .collect()
    };
    if !field_rows.is_empty() {
        app.field_store.remember_all(field_rows.iter().cloned());
        app.field_store.flush_online(&field_rows).await;
    }

    // 日夜時刻一併落地（Phase 0-E）。與玩家狀態不同:時鐘不分玩家、沒人在線也持續走,
    // 故**無條件** flush（不像位置/背包/農地只在有對象時才寫）。讀當下時刻（短暫持鎖、
    // 不跨 await）再非同步寫出,重啟後從同一個時刻接續、不跳回破曉。
    let daynight_now = *app.daynight.read().unwrap();
    app.daynight_store.flush(&daynight_now).await;
}

#[cfg(test)]
mod tests {
    use super::{flush_interval_ticks, should_broadcast, TICK_HZ};

    #[test]
    fn 沒有訂閱者時不廣播() {
        assert!(!should_broadcast(0));
    }

    #[test]
    fn 有任一訂閱者就廣播() {
        assert!(should_broadcast(1));
        assert!(should_broadcast(42));
    }

    #[test]
    fn 有連線時每十秒落地一次() {
        assert_eq!(flush_interval_ticks(true), (TICK_HZ as u64) * 10);
    }

    #[test]
    fn 離峰無連線時落地拉長到每六十秒() {
        assert_eq!(flush_interval_ticks(false), (TICK_HZ as u64) * 60);
    }

    #[test]
    fn 離峰節律是有連線節律的整數倍_轉場乾淨() {
        // 拉疏後的落地點必落在原 10 秒節律的邊界上,有人連上恢復 10 秒節律時不會錯位漏拍。
        assert_eq!(
            flush_interval_ticks(false) % flush_interval_ticks(true),
            0
        );
    }
}
