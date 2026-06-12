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
        ShopCatalogEntry { item: e.item, price_per, trend, stock: None, max_stock: None }
    }).collect()
}

/// 把販售清單轉成 ShopCatalogEntry（販售價固定不浮動，趨勢固定 stable）。
/// 不含庫存資訊——對沒有庫存設定的商人（翠幽星等）使用。
fn build_static_sell_list(sell_list: &[crate::npc::ShopEntry]) -> Vec<ShopCatalogEntry> {
    sell_list.iter().map(|e| ShopCatalogEntry {
        item: e.item,
        price_per: e.price_per,
        trend: "stable".to_string(),
        stock: None,
        max_stock: None,
    }).collect()
}

/// 把故鄉商人販售清單轉成含庫存資訊的 ShopCatalogEntry（ROADMAP 104）。
/// effective_price 已含稀缺溢價，stock / max_stock 供前端顯示缺貨狀況。
fn build_home_sell_list_with_stock(
    sell_list: &[crate::npc::ShopEntry],
    stock_state: &crate::npc_stock::NpcStockState,
) -> Vec<ShopCatalogEntry> {
    sell_list.iter().map(|e| {
        let price_per = stock_state.effective_sell_price(
            crate::npc_treasury::MERCHANT_HOME, e.item, e.price_per
        );
        let stock = Some(stock_state.available(crate::npc_treasury::MERCHANT_HOME, e.item));
        let max_stock = Some(stock_state.max_stock(crate::npc_treasury::MERCHANT_HOME, e.item));
        ShopCatalogEntry { item: e.item, price_per, trend: "stable".to_string(), stock, max_stock }
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
        // 追蹤上一 tick 是否為夜間，用來偵測「剛進夜」和「剛出夜」事件。
        let mut prev_is_night = false;

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

            // 季節循環（ROADMAP 137）：推進季節計時器，切換時廣播公告。
            // 季節成長倍率疊乘在日夜倍率之上，獨立正交不互相侵犯。
            let season_growth = {
                let mut s = app.season.write().unwrap();
                if let Some(new_season) = s.tick(dt) {
                    let _ = app.tx_chat.send(new_season.announce_text().to_string());
                    tracing::info!(season = new_season.as_str(), "季節切換");
                    // 季節性野外採集節點（ROADMAP 154）：季節切換時重置節點。
                    app.seasonal_nodes.write().unwrap().on_season_change(new_season);
                }
                s.growth_rate_modifier()
            };

            // 夜採星晶（ROADMAP 50）：偵測日夜轉換事件，生成或清除星晶礦脈。
            if is_night && !prev_is_night {
                // 剛進入夜間：生成本夜礦脈。
                app.star_crystals.write().unwrap().spawn_for_night();
            } else if !is_night && prev_is_night {
                // 剛退出夜間：清除所有礦脈。
                app.star_crystals.write().unwrap().clear();
            }
            prev_is_night = is_night;

            // 下雨澆田（ROADMAP 109）：在農田 tick 前讀取當前天氣狀態，草原細雨時自動澆灌。
            let is_raining = app.weather.read().unwrap().is_raining();

            // 推進所有玩家農地的成長：依日夜成長倍率 × 季節倍率縮放 dt——
            // 白天亮、長得快，夜裡暗、放慢；春天加速、冬天幾乎停滯（ROADMAP 137）。
            // 濕度也一併縮放，故每次澆水的總成長量不變、只有牆鐘速度隨日夜/季節變化。
            // 同時把每塊地轉成快照、並戳上擁有者 id（`Field` 自己不知道屬於誰）。短暫持鎖，不跨 await。
            // 成長無條件推進(每塊地 tick);view 只在要廣播時才在同一把鎖內多走一趟建。
            let effective_growth = growth_rate * season_growth;
            let field_views: Vec<FieldView> = {
                let mut fields = app.fields.write().unwrap();
                // 灑水器自動澆灌：每個灑水器對主人的農地 tick，倒數到 0 時澆周圍格。
                // 鎖序：sprinklers 先讀，再用 fields 寫鎖（不與 ws 農地操作衝突：ws 是 Farm 鎖序）。
                {
                    let mut sprinklers = app.sprinklers.write().unwrap();
                    for (owner, spr_list) in sprinklers.all_mut() {
                        if let Some(field) = fields.get_mut(owner) {
                            for spr in spr_list.iter_mut() {
                                spr.tick(dt, field);
                            }
                        }
                    }
                }
                for (_owner, field) in fields.iter_mut() {
                    // 草原細雨時先替所有缺水作物補水，再正常 tick 成長。
                    if is_raining {
                        field.water_all_planted();
                    }
                    field.tick(dt * effective_growth);
                }
                // 公共農地與個人地塊同步成長，廣播時以 owner=nil 加入列表讓前端辨識。
                let pub_view = {
                    let mut pf = app.pub_field.write().unwrap();
                    if is_raining {
                        pf.water_all_planted();
                    }
                    pf.tick(dt * effective_growth);
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
                            resting: is_night && crate::enemy_field::is_night_rester(p.id),
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
                // ROADMAP 83：同步收集倒地玩家名稱，供 NPC 落敗反應使用。
                let mut newly_downed_names: Vec<String> = Vec::new();
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
                                newly_downed_names.push(p.name.clone());
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
                // NPC 落敗反應（ROADMAP 83）：玩家倒地時 NPC 廣播慰問 / 警示。
                // 此段在每秒一次的戰鬥判定區塊內，tick 傳 1.0（一秒）。
                {
                    let npc_opt = {
                        let mut s = app.npc_defeat_reaction.write().unwrap();
                        s.tick(1.0);
                        if !newly_downed_names.is_empty() {
                            s.on_player_downed()
                        } else {
                            None
                        }
                    };
                    if let Some(npc) = npc_opt {
                        let tx_chat = app.tx_chat.clone();
                        let sem = app.npc_defeat_reaction_sem.clone();
                        let player_name = newly_downed_names[0].clone();
                        tokio::spawn(async move {
                            let Ok(_permit) = sem.try_acquire_owned() else { return };
                            let (npc, text) =
                                crate::npc_defeat_reaction::generate_reaction(npc, player_name).await;
                            let (emoji, npc_name) = match npc {
                                crate::npc_defeat_reaction::ReactionNpc::Chief => {
                                    ("💔", crate::npc_defeat_reaction::CHIEF_DISPLAY_NAME)
                                }
                                crate::npc_defeat_reaction::ReactionNpc::Lanka => {
                                    ("⚔️", crate::npc_defeat_reaction::RANKA_DISPLAY_NAME)
                                }
                            };
                            let _ = tx_chat.send(format!("{emoji} [{npc_name}] 道：「{text}」"));
                        });
                    }
                }
                // 清理暫時用的 pos_map 避免 unused 警告
                let _ = pos_map.drain();
            }

            // 整合位置 + 推進生命回復（權威模擬,每 tick 必跑,與有無觀眾無關;短暫持鎖,不跨 await）。
            // （tile_deltas_snap 已在敵人段前快照，玩家碰撞沿用同一份。）
            // 易腐品腐壞通知（ROADMAP 106）：在鎖外送訊息，避免死鎖。
            let mut decay_notifications: Vec<(uuid::Uuid, Vec<crate::perishable::DecayEvent>)> = Vec::new();
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
                    // 星際貿易路線冷卻倒數（ROADMAP 51）。
                    crate::trade_route::tick_cooldowns(&mut p.trade_cooldowns, dt);
                    // 工匠工坊訂單計時（ROADMAP 52）。
                    crate::workshop::tick(&mut p.workshop_active, &mut p.workshop_cooldown, dt);
                    // 懸賞告示板計時（ROADMAP 53）。
                    crate::bounty_board::tick(&mut p.bounty_active, &mut p.bounty_cooldown, dt);
                    // 古蹟探勘計時（ROADMAP 54）。
                    crate::expedition::tick(&mut p.expedition_active, &mut p.expedition_cooldown, dt);
                    // 星際採購令計時（ROADMAP 55）。
                    crate::procurement::tick(&mut p.procurement_active, &mut p.procurement_cooldown, dt);
                    // 農產品展覽會計時（ROADMAP 56）。
                    crate::farm_fair::tick(&mut p.farm_fair_active, &mut p.farm_fair_cooldown, dt);
                    // 易腐品腐壞計時（ROADMAP 106）：只在玩家連線（在 players map）時遞減。
                    {
                        // decay_timers 與 inventory/warehouse 是不同欄位，可同時借用。
                        let events = p.decay_timers.tick(dt, &p.inventory, &p.warehouse);
                        if !events.is_empty() {
                            // 立即移除腐壞的物品
                            for event in &events {
                                if let crate::perishable::DecayEvent::Spoiled(item) = event {
                                    let qty = p.inventory.count(*item);
                                    if qty > 0 { let _ = p.inventory.take(*item, qty); }
                                    let wqty = p.warehouse.count(*item);
                                    if wqty > 0 { let _ = p.warehouse.take(*item, wqty); }
                                }
                            }
                            decay_notifications.push((p.id, events));
                        }
                    }
                    // 蒸汽床 HP 回復（ROADMAP 155）：每 30 秒，在室內且擁有蒸汽床的玩家回復 2 HP。
                    let bed_interval = crate::home_furniture::BED_REGEN_INTERVAL_SECS as u64 * TICK_HZ as u64;
                    if tick % bed_interval == 0 && p.indoor_plot_id.is_some() && !p.vitals.is_downed() {
                        let has_bed = app.home_furnishings.read().unwrap()
                            .get(&p.id).map(|h| h.has_bed()).unwrap_or(false);
                        if has_bed {
                            p.vitals.heal(crate::home_furniture::BED_REGEN_HP);
                        }
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

            // 易腐品腐壞通知（ROADMAP 106）：鎖已釋放，可安全送直接訊息。
            if !decay_notifications.is_empty() {
                let senders = app.whisper_senders.read().unwrap();
                for (pid, events) in decay_notifications {
                    if let Some(tx) = senders.get(&pid) {
                        for event in events {
                            let msg = match &event {
                                crate::perishable::DecayEvent::Spoiled(item) => {
                                    let name = crate::perishable::item_display_zh(*item);
                                    format!("🍂 你的{name}因存放過久已腐壞消失！下次請盡快使用或賣給 NPC。")
                                }
                                crate::perishable::DecayEvent::Warning { item, remaining_secs } => {
                                    let name = crate::perishable::item_display_zh(*item);
                                    let mins = remaining_secs / 60;
                                    format!("⚠️ 你的{name}再過 {mins} 分鐘就會腐壞！請盡快使用或賣出。")
                                }
                            };
                            let _ = tx.try_send(msg);
                        }
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
                    // 世界事件記憶（ROADMAP 65）：裂縫是引擎事實，NPC 可自然提及。
                    app.world_log.write().unwrap().push(format!(
                        "宇宙裂縫在座標 ({:.0}, {:.0}) 附近開啟，裂縫守護者現身",
                        rx, ry
                    ));
                    // NPC 需求驅力（ROADMAP 69）：裂縫開啟 → 安全感下降。
                    app.npc_needs.write().unwrap().apply_world_event(crate::npc_needs::NeedsEvent::RiftOpened);
                    // NPC 人際關係網（ROADMAP 70）：裂縫開啟影響 NPC 對之間的關係。
                    app.npc_relations.write().unwrap().apply_world_event(crate::npc_relations::RelationsEvent::RiftOpened);
                    // NPC 主動評論（ROADMAP 68）：裂縫開啟觸發相關 NPC 在聊天頻道表態。
                    {
                        let event_kind = crate::npc_proactive::WorldEventKind::RiftOpened {
                            desc: format!("({:.0}, {:.0}) 附近", rx, ry),
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
                    tracing::info!(x = rx, y = ry, "宇宙裂縫觸發，裂縫守護者注入");
                }
            }

            // AI 導演層＋獸潮攻城（ROADMAP 44 / 139）：低頻導演 tick，觸發時注入怪波＋廣播公告。
            // 先把居民數傳給導演，讓它依人口縮放波次（ROADMAP 139 平衡）。
            {
                let resident_count = app.residents.read().unwrap().population();
                let defense_drill = app.civic_vote.read().unwrap().defense_drill_active();
                let cmds = {
                    let mut director = app.director.write().unwrap();
                    director.update_population(resident_count);
                    director.tick(dt)
                };
                for cmd in cmds {
                    match cmd {
                        crate::director::DirectorCmd::AnnounceHorde { site_x, site_y, site_label, wave } => {
                            // 城防演練（ROADMAP 156）：城防演練進行中，跳過怪物注入。
                            if defense_drill {
                                let _ = app.tx_chat.send(
                                    "🛡️ 城防演練進行中，獸潮被警戒陣形阻擋，暫時撤退！".to_string()
                                );
                                continue;
                            }
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
                            drop(enemies);
                            tracing::info!(site = site_label, injected, "獸潮廣播＋注入第一波怪物");
                            let _ = app.tx_chat.send(format!(
                                "⚔️ 獸潮來襲！大批怪物正聚集在{}！\
                                 30 秒後衝擊城門——出城迎戰或守在城牆輸出！",
                                site_label
                            ));
                            // 世界事件記憶（ROADMAP 65）：獸潮集結是重要世界事件。
                            app.world_log.write().unwrap().push(format!(
                                "獸潮集結在{}城門外，怪物大軍蓄勢衝擊——拓荒者們嚴陣以待",
                                site_label
                            ));
                            // NPC 需求驅力（ROADMAP 69）：獸潮集結 → 全員安全感大跌。
                            app.npc_needs.write().unwrap().apply_world_event(crate::npc_needs::NeedsEvent::HordeArriving);
                            // NPC 人際關係網（ROADMAP 70）：獸潮壓力帶出小摩擦。
                            app.npc_relations.write().unwrap().apply_world_event(crate::npc_relations::RelationsEvent::HordeArriving);
                            // NPC 主動評論（ROADMAP 68）：獸潮警報觸發 NPC 聊天頻道表態。
                            {
                                let event_kind = crate::npc_proactive::WorldEventKind::HordeArriving {
                                    site: site_label.to_string(),
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
                            let _ = (site_x, site_y); // 座標已廣播至聊天，此處無需記憶
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
                            // 世界事件記憶（ROADMAP 65）：獸潮退守是值得 NPC 提及的大事。
                            app.world_log.write().unwrap().push(format!(
                                "拓荒者們在{}成功打退獸潮（斬殺 {} 隻），英勇守護了村落",
                                site_label, kills
                            ));
                            // NPC 需求驅力（ROADMAP 69）：獸潮打退 → 安全感回升，社群歸屬感大升。
                            app.npc_needs.write().unwrap().apply_world_event(crate::npc_needs::NeedsEvent::HordeRepelled);
                            // NPC 人際關係網（ROADMAP 70）：共患難加深信任。
                            app.npc_relations.write().unwrap().apply_world_event(crate::npc_relations::RelationsEvent::HordeRepelled);
                            // NPC 主動評論（ROADMAP 68）：獸潮打退，NPC 慶祝。
                            {
                                let event_kind = crate::npc_proactive::WorldEventKind::HordeRepelled {
                                    site: site_label.to_string(),
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

            // NPC 升等賀詞（ROADMAP 84）：推進全服廣播冷卻倒數。
            app.npc_level_greet.write().unwrap().tick(dt);
            // 牧場系統（ROADMAP 48）：推進所有有雞地塊的下蛋計時器。
            app.ranch.write().unwrap().tick(dt);
            // 農地作物系統（ROADMAP 49）：推進所有農田地塊的作物生長計時器；下雨時給 1.5x 加成。
            app.farm_crops.write().unwrap().tick(dt, is_raining);
            // NPC 作息與移動（ROADMAP 73）：推進 NPC 位置。
            {
                let daynight = app.daynight.read().unwrap();
                app.npc_schedule.write().unwrap().tick(dt, &daynight);
            }
            // 城外旅人（ROADMAP 74）：推進旅人狀態並廣播到訪/離開事件。
            {
                let event = app.traveler.write().unwrap().tick(dt);
                if let Some(ev) = event {
                    use crate::traveler_npc::TravelerEvent;
                    match ev {
                        TravelerEvent::Arrived { name, origin } => {
                            let msg = format!(
                                "🧳 【旅人到訪】{name} 走進了主城廣場！（身份：{origin}）靠近可以聊天。"
                            );
                            let _ = app.tx_chat.send(msg);
                            app.world_log.write().unwrap().push(format!(
                                "旅行者 {name} 從外地走入主城，身份：{origin}"
                            ));
                        }
                        TravelerEvent::Departed { name } => {
                            let _ = app.tx_chat.send(format!(
                                "🧳 【旅人離去】{name} 拾起行囊，繼續上路了，感謝陪伴！"
                            ));
                        }
                    }
                }
            }

            // 路人居民推進（ROADMAP 115+116）：移動 + 生命週期 + 人口增減；廣播生命事件。
            {
                let avg_prosperity = {
                    let needs = app.npc_needs.read().unwrap();
                    use crate::npc_schedule::VILLAGE_NPCS;
                    let total: i32 = VILLAGE_NPCS.iter()
                        .map(|s| needs.get(s.id).prosperity)
                        .sum();
                    total / VILLAGE_NPCS.len().max(1) as i32
                };
                let current_phase = app.daynight.read().unwrap().phase();
                // 收集玩家座標快照（ROADMAP 123）：先取讀鎖收集、再取居民寫鎖，避免死鎖。
                let player_positions: Vec<(String, f32, f32)> = {
                    let players = app.players.read().unwrap();
                    players.values()
                        .filter(|p| !p.vitals.is_downed())
                        .map(|p| (p.name.clone(), p.x, p.y))
                        .collect()
                };
                // 野生動物 tick（ROADMAP 141 食物鏈）。
                {
                    let positions: Vec<(f32, f32)> = player_positions.iter()
                        .map(|(_, x, y)| (*x, *y))
                        .collect();
                    // ROADMAP 144：取得物種態度 Map，傳入 wildlife tick。
                    let attitudes = app.species_relations.read().unwrap().attitudes.clone();
                    let wildlife_events = app.wildlife_manager.write().unwrap()
                        .tick(dt, &positions, &attitudes);
                    for ev in wildlife_events {
                        use crate::wildlife::WildlifeEvent;
                        match ev {
                            WildlifeEvent::Kill { predator_kind, prey_kind, x, y } => {
                                let msg = format!(
                                    "🌿 城外 ({:.0},{:.0})：{} 捕獲了 {}，弱肉強食是生態的法則。",
                                    x, y,
                                    predator_kind.display_name(),
                                    prey_kind.display_name()
                                );
                                let _ = app.tx_chat.send(msg);
                            }
                            // ROADMAP 143：物種聚落被入侵，廣播世界聊天警示。
                            WildlifeEvent::ColonyThreatened { colony_name, cx, cy } => {
                                let msg = format!(
                                    "🛡️ 城外 ({:.0},{:.0})：{} 察覺到入侵者，正在驅離！",
                                    cx, cy, colony_name
                                );
                                let _ = app.tx_chat.send(msg);
                            }
                            // ROADMAP 144：敵視物種近身攻擊玩家——找出 near_x/near_y 附近的玩家並扣血。
                            WildlifeEvent::WildlifeAttack { attacker_kind, near_x, near_y, damage } => {
                                let reach2 = (crate::species_relations::HOSTILE_WILDLIFE_DAMAGE as f32 * 20.0_f32).powi(2);
                                let victim_ids: Vec<uuid::Uuid> = {
                                    let pl = app.players.read().unwrap();
                                    pl.values()
                                        .filter(|p| {
                                            let dx = p.x - near_x;
                                            let dy = p.y - near_y;
                                            dx * dx + dy * dy <= reach2 && !p.vitals.is_downed()
                                        })
                                        .map(|p| p.id)
                                        .collect()
                                };
                                for vid in victim_ids {
                                    let downed = {
                                        let mut players = app.players.write().unwrap();
                                        if let Some(p) = players.get_mut(&vid) {
                                            p.vitals.take_damage(damage);
                                            p.vitals.is_downed()
                                        } else { false }
                                    };
                                    if downed {
                                        let name = app.players.read().unwrap()
                                            .get(&vid).map(|p| p.name.clone()).unwrap_or_default();
                                        let msg = format!(
                                            "⚠️ {} 被 {} 擊倒！需要保護這片野地的信任，試著餵食牠們改善關係。",
                                            name, attacker_kind.display_name()
                                        );
                                        let _ = app.tx_chat.send(msg);
                                    }
                                }
                            }
                        }
                    }
                }
                // ROADMAP 144：物種關係 tick（態度自然衰減 + 層級改變廣播）。
                {
                    let sr_events = app.species_relations.write().unwrap().tick(dt);
                    for ev in sr_events {
                        use crate::species_relations::SpeciesRelationEvent;
                        match ev {
                            SpeciesRelationEvent::TierChanged { kind, new_tier } => {
                                let msg = format!(
                                    "🌿 [生態] {} 對人類的態度改變為{}！",
                                    kind.display_name(),
                                    new_tier.display_zh()
                                );
                                let _ = app.tx_chat.send(msg);
                            }
                        }
                    }
                }
                let (resident_events, thought_events) = app.residents.write().unwrap()
                    .tick(dt, avg_prosperity, current_phase, &player_positions);
                for ev in resident_events {
                    use crate::resident_npc::ResidentLifecycleEvent;
                    match ev {
                        ResidentLifecycleEvent::PhaseTransition { msg, .. } => {
                            let _ = app.tx_chat.send(msg.to_string());
                        }
                        ResidentLifecycleEvent::RetirementSoon { msg, .. } => {
                            let _ = app.tx_chat.send(msg);
                        }
                        ResidentLifecycleEvent::RetiredToEther { old_name, new_name, farewell_msg, arrival_msg } => {
                            let _ = app.tx_chat.send(farewell_msg);
                            let _ = app.tx_chat.send(arrival_msg);
                            app.world_log.write().unwrap().push(
                                format!("居民 {} 回歸乙太，{} 遷入接替。", old_name, new_name)
                            );
                        }
                        ResidentLifecycleEvent::NewArrival { msg, .. } => {
                            let _ = app.tx_chat.send(msg);
                        }
                        ResidentLifecycleEvent::Departed { msg, .. } => {
                            let _ = app.tx_chat.send(msg);
                        }
                        // ROADMAP 120：居民工作動態廣播——0 玩家也持續廣播，玩家回來可見城鎮活動紀錄。
                        ResidentLifecycleEvent::WorkActivity { text } => {
                            let _ = app.tx_chat.send(text);
                        }
                        // ROADMAP 122：居民隨機小事件——廣播至世界聊天，0 玩家也持續累積。
                        ResidentLifecycleEvent::MiniEvent { text } => {
                            let _ = app.tx_chat.send(text);
                        }
                        // ROADMAP 123：居民主動向玩家打招呼——廣播 NpcSpeech 泡泡 + 世界聊天通知。
                        ResidentLifecycleEvent::PlayerGreeting {
                            resident_id, resident_name, x, y, player_name: _, text,
                        } => {
                            let _ = app.tx.send(std::sync::Arc::new(crate::protocol::ServerMsg::NpcSpeech {
                                npc_id: resident_id,
                                npc_name: format!("居民 {}", resident_name),
                                text: text.clone(),
                                display_secs: 7,
                                wx: x,
                                wy: y,
                            }));
                            let _ = app.tx_chat.send(text);
                        }
                        // ROADMAP 121：兩位居民相遇打招呼——廣播雙方 NpcSpeech 泡泡。
                        ResidentLifecycleEvent::NeighborChat {
                            id_a, name_a, text_a, x_a, y_a,
                            id_b, name_b, text_b, x_b, y_b,
                        } => {
                            let _ = app.tx.send(std::sync::Arc::new(crate::protocol::ServerMsg::NpcSpeech {
                                npc_id: id_a,
                                npc_name: format!("居民 {}", name_a),
                                text: text_a,
                                display_secs: 6,
                                wx: x_a,
                                wy: y_a,
                            }));
                            let _ = app.tx.send(std::sync::Arc::new(crate::protocol::ServerMsg::NpcSpeech {
                                npc_id: id_b,
                                npc_name: format!("居民 {}", name_b),
                                text: text_b,
                                display_secs: 6,
                                wx: x_b,
                                wy: y_b,
                            }));
                        }
                        // ROADMAP 125：居民互助請求——廣播世界聊天 + NpcSpeech 頭頂泡泡。
                        ResidentLifecycleEvent::HelpRequested { resident_id, resident_name, x, y, text } => {
                            let _ = app.tx_chat.send(text.clone());
                            let _ = app.tx.send(std::sync::Arc::new(crate::protocol::ServerMsg::NpcSpeech {
                                npc_id: resident_id,
                                npc_name: format!("居民 {}", resident_name),
                                text,
                                display_secs: 10,
                                wx: x,
                                wy: y,
                            }));
                        }
                        // 居民快樂值首次突破門檻（ROADMAP 126）：廣播暖心世界聊天。
                        ResidentLifecycleEvent::HappinessBoost { msg, .. } => {
                            let _ = app.tx_chat.send(msg);
                        }
                        // 城鎮繁榮等級改變（ROADMAP 128）：廣播世界聊天讓全服玩家都知道。
                        ResidentLifecycleEvent::ProsperityChanged { msg, .. } => {
                            let _ = app.tx_chat.send(msg);
                        }
                        // 快樂居民招待附近玩家（ROADMAP 127）：給乙太小禮 + 泡泡 + 世界聊天。
                        ResidentLifecycleEvent::PlayerGift {
                            resident_id, resident_name, x, y, player_name, text, ..
                        } => {
                            use crate::resident_npc::GIFT_ETHER;
                            // 給指定玩家乙太（依名字查找；此時 residents 寫鎖已釋放，安全取 players 寫鎖）
                            {
                                let mut players = app.players.write().unwrap();
                                if let Some(p) = players.values_mut().find(|p| p.name == player_name) {
                                    p.ether = p.ether.saturating_add(GIFT_ETHER);
                                }
                            }
                            // NpcSpeech 頭頂泡泡
                            let _ = app.tx.send(std::sync::Arc::new(crate::protocol::ServerMsg::NpcSpeech {
                                npc_id: resident_id,
                                npc_name: format!("居民 {}", resident_name),
                                text: text.clone(),
                                display_secs: 8,
                                wx: x,
                                wy: y,
                            }));
                            // 世界聊天帶 +5 乙太提示
                            let _ = app.tx_chat.send(format!("{}（+{} 乙太）", text, GIFT_ETHER));
                        }
                    }
                }
                // ROADMAP 118：居民思想泡泡——廣播 NpcSpeech，前端在居民頭頂繪製泡泡。
                if !thought_events.is_empty() {
                    let (phase, weather) = {
                        let dn = app.daynight.read().unwrap();
                        let wx = app.weather.read().unwrap();
                        (dn.phase(), wx.weather_type)
                    };
                    let ctx = crate::resident_chat::ResidentContext { phase, weather };
                    for ev in thought_events {
                        let text = crate::resident_chat::get_thought(ev.persona, &ctx, ev.seed);
                        let _ = app.tx.send(std::sync::Arc::new(crate::protocol::ServerMsg::NpcSpeech {
                            npc_id: ev.id,
                            npc_name: format!("居民 {}", ev.name),
                            text: text.to_string(),
                            display_secs: 5,
                            wx: ev.x,
                            wy: ev.y,
                        }));
                    }
                }
            }

            // 廣場聚會 tick（ROADMAP 124）：白天時段定期觸發居民聚會，全服 EXP +20%。
            {
                use crate::community_gathering::GatheringEvent;
                let phase_now = app.daynight.read().unwrap().phase();
                let gathering_events = app.community_gathering.write().unwrap()
                    .tick(dt, phase_now);
                for ev in gathering_events {
                    match ev {
                        GatheringEvent::Started { text } => {
                            let _ = app.tx_chat.send(text);
                        }
                        GatheringEvent::Ended { text } => {
                            let _ = app.tx_chat.send(text);
                        }
                    }
                }
            }

            // 公民投票 tick（ROADMAP 156）：居民代言人定期提案，玩家投票決定城鎮短期效果。
            {
                use crate::civic_vote::CivicVoteEvent;
                let spokesman = crate::civic_vote::CivicVoteState::elect_spokesman(
                    &app.residents.read().unwrap().residents
                );
                let civic_events = app.civic_vote.write().unwrap().tick(dt, spokesman);
                for ev in civic_events {
                    match ev {
                        CivicVoteEvent::ProposalStarted { text } => {
                            let _ = app.tx_chat.send(text);
                        }
                        CivicVoteEvent::ProposalPassed { text, .. } => {
                            let _ = app.tx_chat.send(text);
                        }
                        CivicVoteEvent::ProposalRejected { text } => {
                            let _ = app.tx_chat.send(text);
                        }
                        CivicVoteEvent::AetherReward => {
                            // 乙太集資：給所有在線玩家 +AETHER_REWARD_AMOUNT 乙太。
                            let reward = crate::civic_vote::AETHER_REWARD_AMOUNT;
                            {
                                let mut players = app.players.write().unwrap();
                                for p in players.values_mut() {
                                    p.ether = p.ether.saturating_add(reward);
                                }
                            }
                            let _ = app.tx_chat.send(format!(
                                "⚡ 乙太集資成功！每位在線玩家獲得 +{} 乙太！",
                                reward
                            ));
                        }
                    }
                }
            }

            // 天文台星象預報 tick（ROADMAP 132）：天文台竣工後每個黎明廣播星象、啟用全服加成。
            {
                let project_completed = app.town_project.read().unwrap().status
                    == crate::town_project::TownProjectStatus::Completed;
                let phase_now = app.daynight.read().unwrap().phase();
                let forecast = app.observatory.write().unwrap()
                    .tick(dt, phase_now, project_completed);
                if let Some(bonus) = forecast {
                    let app2 = app.clone();
                    let sem = app.observatory_sem.clone();
                    tokio::spawn(async move {
                        let _permit = sem.try_acquire();
                        let text = crate::observatory::generate_forecast(bonus).await;
                        let bonus_name = bonus.name();
                        let msg = format!(
                            "🔭 [蒸汽天文台] 今日星象：「{}」→ {} 持續 {} 分鐘！",
                            text,
                            bonus_name,
                            (crate::observatory::FORECAST_DURATION_SECS / 60.0) as u32,
                        );
                        let _ = app2.tx_chat.send(msg);
                    });
                }
            }

            // 流星雨 tick（ROADMAP 133）：天文台竣工後每 30 分鐘觸發流星雨，地面出現星塵採集點。
            {
                let project_completed = app.town_project.read().unwrap().status
                    == crate::town_project::TownProjectStatus::Completed;
                let triggered = app.meteor_shower.write().unwrap().tick(dt, project_completed);
                if triggered {
                    let dur_min = (crate::meteor_shower::SHOWER_DURATION_SECS / 60.0) as u32;
                    let msg = format!(
                        "☄️ 流星雨降臨！城鎮周圍出現 {} 個星塵採集點，限時 {} 分鐘——快去採集吧！",
                        crate::meteor_shower::DUST_NODE_COUNT,
                        dur_min,
                    );
                    let _ = app.tx_chat.send(msg);
                }
            }

            // 旅行商人（ROADMAP 135）：每 2 小時來訪，停留 10 分鐘。
            {
                let (arrived, departed) = app.wandering_merchant.write().unwrap().tick(dt);
                if arrived {
                    let stay_min = (crate::wandering_merchant::STAY_SECS / 60.0) as u32;
                    let _ = app.tx_chat.send(format!(
                        "🧳 旅行商人來了！帶著其他星球的稀有貨物，在廣場北緣等候你 {} 分鐘——快去交易！",
                        stay_min,
                    ));
                }
                if departed {
                    let _ = app.tx_chat.send(
                        "👋 旅行商人的貨物已賣完，他揮手道別，踏上下一段旅程……".to_string(),
                    );
                }
            }

            // NPC 需求驅力衰減（ROADMAP 69）：每 DECAY_INTERVAL_SECS 秒，所有 NPC 的需求值向基線緩慢靠近。
            // 讓情緒狀態有明顯持續性（事件影響維持數分鐘）但不永久停在極端值。
            {
                let decay_ticks = crate::npc_needs::DECAY_INTERVAL_SECS * TICK_HZ as u64;
                if tick % decay_ticks == 0 && tick > 0 {
                    app.npc_needs.write().unwrap().tick_decay_all();
                }
            }
            // NPC 人際關係網衰減（ROADMAP 70）：每 DECAY_INTERVAL_SECS 秒，所有 NPC 對的好惡值向中性緩慢靠近。
            // 關係比情緒更持久（5 分鐘一次），確保共患難的信任不會瞬間消散。
            {
                let rel_decay_ticks = crate::npc_relations::DECAY_INTERVAL_SECS * TICK_HZ as u64;
                if tick % rel_decay_ticks == 0 && tick > 0 {
                    app.npc_relations.write().unwrap().tick_decay_all();
                }
            }
            // NPC 派系自主湧現（ROADMAP 71）：在關係網衰減後，偵測是否有 NPC 對的好惡值越過
            // 結盟（≥80）或競爭（≤22）門檻，廣播派系事件到全服聊天頻道。
            // 同週期（5 分鐘）：關係剛衰減完，是最佳時機偵測狀態變化。
            {
                let faction_ticks = crate::npc_relations::DECAY_INTERVAL_SECS * TICK_HZ as u64;
                if tick % faction_ticks == 0 && tick > 0 {
                    let relations_snapshot = app.npc_relations.read().unwrap();
                    let faction_events = app.npc_factions.write().unwrap().detect_changes(&relations_snapshot);
                    drop(relations_snapshot);
                    for ev in faction_events {
                        let text = ev.announce_text();
                        if !text.is_empty() {
                            let _ = app.tx_chat.send(text);
                        }
                    }
                }
            }

            // 怪物王咆哮（ROADMAP 75）：每 tick 推進各菁英精英的咆哮冷卻計時；
            // 冷卻歸零時非同步呼叫 LLM（Groq→ollama→罐頭），結果廣播至全服聊天頻道。
            // 成本紀律：Semaphore(1) 限制同時只有一個 AI 咆哮呼叫。
            // ROADMAP 114：0 玩家時仍持續運轉，讓世界有生命感。
            {
                let player_count = app.players.read().unwrap().len();
                // 收集所有菁英精英（notorious = level >= base_level + 3）。
                let notorious: Vec<_> = app
                    .enemies
                    .read()
                    .unwrap()
                    .enemies()
                    .into_iter()
                    .filter(|e| e.level >= e.base_level.saturating_add(3))
                    .map(|e| (e.id, e.enemy.kind().display_name(), e.level))
                    .collect();
                let candidate = app.boss_roar.write().unwrap().tick(dt, &notorious);
                if let Some(c) = candidate {
                    let tx_chat = app.tx_chat.clone();
                    let sem = app.boss_roar_sem.clone();
                    let kind_name = c.kind_name.to_string();
                    let level = c.level;
                    tokio::spawn(async move {
                        // 非阻塞嘗試取得 Semaphore；若已有咆哮進行中則直接略過。
                        let Ok(_permit) = sem.try_acquire_owned() else { return };
                        let text = crate::boss_roar::generate_roar(&kind_name, level, player_count).await;
                        let _ = tx_chat.send(format!(
                            "👹 〔怪物王・{kind_name} Lv.{level}〕{text}"
                        ));
                    });
                }
            }

            // 怪物王戰術指揮（ROADMAP 117）：菁英精英每 90 秒決策一次戰術；
            // 戰術由罐頭邏輯即時決定（零延遲），AI 非同步生成廣播台詞。
            // ROADMAP 114：0 玩家時仍持續運轉，讓世界不因無人而沉默。
            {
                let player_count = app.players.read().unwrap().len();
                let tactic_inputs: Vec<_> = app
                    .enemies
                    .read()
                    .unwrap()
                    .enemies()
                    .into_iter()
                    .filter(|e| e.enemy.is_alive() && e.level >= e.base_level.saturating_add(3))
                    .map(|e| crate::boss_ai::TacticInput {
                        id: e.id,
                        kind_name: e.enemy.kind().display_name(),
                        level: e.level,
                        x: e.x,
                        y: e.y,
                        hp_pct: e.enemy.remaining_hp() as f32 / e.enemy.max_hp().max(1) as f32,
                    })
                    .collect();
                let candidate = app.boss_ai.write().unwrap().tick(dt, &tactic_inputs, player_count);
                if let Some(c) = candidate {
                    // 立即套用戰術（機制效果，同步），不等 AI 台詞。
                    let players_pos: Vec<(f32, f32)> = app.players.read().unwrap()
                        .values()
                        .map(|p| (p.x, p.y))
                        .collect();
                    app.enemies.write().unwrap()
                        .broadcast_boss_command(c.id, c.x, c.y, &c.tactic, &players_pos);
                    // 非同步生成廣播台詞（AI 台詞或罐頭降級）。
                    let tx_chat = app.tx_chat.clone();
                    let sem = app.boss_ai_sem.clone();
                    let kind_name = c.kind_name.clone();
                    let level = c.level;
                    let tactic = c.tactic.clone();
                    tokio::spawn(async move {
                        let Ok(_permit) = sem.try_acquire_owned() else { return };
                        let msg = crate::boss_ai::generate_tactic_message(&kind_name, level, &tactic).await;
                        let tactic_name = tactic.display_name();
                        let _ = tx_chat.send(format!(
                            "⚔️ 〔怪物王・{kind_name} Lv.{level}〕下令「{tactic_name}」：{msg}"
                        ));
                    });
                }
            }

            // NPC 自主懸賞令（ROADMAP 82）：蘭卡安全感低且兇名精英存在時自主發布通緝令。
            // 成本紀律：15 分鐘冷卻、純罐頭降級可運作、無 Semaphore（state 機保證最多一筆）。
            // ROADMAP 114：0 玩家時仍持續運轉。
            {
                // 收集所有兇名精英（同 boss_roar 邏輯）。
                let notorious: Vec<_> = app
                    .enemies
                    .read()
                    .unwrap()
                    .enemies()
                    .into_iter()
                    .filter(|e| e.level >= e.base_level.saturating_add(3))
                    .map(|e| (e.enemy.kind().display_name(), e.level))
                    .collect();
                let lanca_safety = app.npc_needs.read().unwrap().get("bounty_npc").safety;
                let candidate = app.npc_bounty.write().unwrap().tick(dt, &notorious, lanca_safety);
                if let Some((kind_name, level)) = candidate {
                    let tx_chat = app.tx_chat.clone();
                    tokio::spawn(async move {
                        let text = crate::npc_bounty::generate_announcement(&kind_name, level).await;
                        let _ = tx_chat.send(format!(
                            "🎯 [獵手蘭卡] 貼出通緝令：「{text}」"
                        ));
                    });
                }
            }

            // NPC 主動資材委託（ROADMAP 85）：繁榮感低時商人薇拉自動發急收令。
            // ROADMAP 114：0 玩家時仍持續運轉。
            {
                let merchant_prosperity = app.npc_needs.read().unwrap().get("merchant").prosperity;
                let commission_event = app.npc_commission.write().unwrap()
                    .tick(dt, merchant_prosperity);
                if let Some(event) = commission_event {
                    let tx_chat = app.tx_chat.clone();
                    let merchant = crate::npc_commission::MERCHANT_DISPLAY_NAME;
                    match event {
                        crate::npc_commission::CommissionEvent::NewCommission { item_name, bonus, quota } => {
                            let text = crate::npc_commission::announce_text(item_name, bonus, quota);
                            let _ = tx_chat.send(format!("📦 [{merchant}] 發布急收令：「{text}」"));
                        }
                        crate::npc_commission::CommissionEvent::Expired => {
                            // 逾時靜默消失，不廣播，保持頻道乾淨。
                        }
                    }
                }
            }

            // NPC 探勘加碼令（ROADMAP 86）：安全感高時芙利亞自動發加碼令。
            // ROADMAP 114：0 玩家時仍持續運轉。
            {
                let expedition_safety = app.npc_needs.read().unwrap().get("expedition_npc").safety;
                let boost_event = app.npc_expedition_boost.write().unwrap()
                    .tick(dt, expedition_safety);
                if let Some(event) = boost_event {
                    let tx_chat = app.tx_chat.clone();
                    let npc = crate::npc_expedition_boost::EXPEDITION_NPC_NAME;
                    match event {
                        crate::npc_expedition_boost::BoostEvent::NewBoost { bonus, quota } => {
                            let text = crate::npc_expedition_boost::announce_text(bonus, quota);
                            let _ = tx_chat.send(format!("🗺️ [{npc}] 宣告：「{text}」"));
                        }
                        crate::npc_expedition_boost::BoostEvent::Expired => {
                            // 逾時靜默消失，不廣播，保持頻道乾淨。
                        }
                    }
                }
            }

            // NPC 工坊加成令（ROADMAP 87）：歸屬感高時老胡自動發急修加成令。
            // ROADMAP 114：0 玩家時仍持續運轉。
            {
                let workshop_belonging = app.npc_needs.read().unwrap().get("workshop_npc").belonging;
                let boost_event = app.npc_workshop_boost.write().unwrap()
                    .tick(dt, workshop_belonging);
                if let Some(event) = boost_event {
                    let tx_chat = app.tx_chat.clone();
                    let npc = crate::npc_workshop_boost::WORKSHOP_NPC_NAME;
                    match event {
                        crate::npc_workshop_boost::BoostEvent::NewBoost { bonus, quota } => {
                            let text = crate::npc_workshop_boost::announce_text(bonus, quota);
                            let _ = tx_chat.send(format!("🔧 [{npc}] 喊道：「{text}」"));
                        }
                        crate::npc_workshop_boost::BoostEvent::Expired => {
                            // 逾時靜默消失，不廣播，保持頻道乾淨。
                        }
                    }
                }
            }

            // 天氣系統（ROADMAP 93）：推進天氣計時器，切換時廣播聊天公告。
            // 下雨澆田（ROADMAP 109）：雨停時提示玩家農地需手動澆水。
            {
                let switched = app.weather.write().unwrap().advance(dt);
                if let Some(new_type) = switched {
                    let _ = app.tx_chat.send(new_type.announce_text().to_string());
                    // 從草原細雨轉換至其他天氣時，補一則「雨停了」提示。
                    if is_raining {
                        let _ = app.tx_chat.send(
                            "🌤️ 雨停了！農地恢復乾燥，記得幫作物澆水喔！".to_string()
                        );
                    }
                }
            }

            // 廣場夜談（ROADMAP 76）：夜間 NPC 在廣場閒聊（ROADMAP 114：0 玩家也持續）。
            {
                let talk_pair = app.plaza_talk.write().unwrap().tick(dt, is_night);
                if let Some(pair) = talk_pair {
                    let tx_chat = app.tx_chat.clone();
                    let tx = app.tx.clone();
                    let sem = app.plaza_talk_sem.clone();
                    let speaker_id = pair.speaker_id;
                    let listener_id = pair.listener_id;
                    // 取說話者目前世界座標（夜間位置），供 NpcSpeech 泡泡定位。
                    let (wx, wy) = app.npc_schedule.read().unwrap()
                        .get_pos(speaker_id)
                        .unwrap_or_else(|| crate::npc_schedule::fallback_pos(speaker_id));
                    tokio::spawn(async move {
                        // 非阻塞嘗試取得 Semaphore；若已有夜談進行中則直接略過。
                        let Ok(_permit) = sem.try_acquire_owned() else { return };
                        let text = crate::plaza_talk::generate_talk(speaker_id, listener_id).await;
                        let s_name = crate::plaza_talk::display_name(speaker_id);
                        let l_name = crate::plaza_talk::display_name(listener_id);
                        let _ = tx_chat.send(format!(
                            "🌙 [{s_name}] 對 [{l_name}] 說：「{text}」"
                        ));
                        // ROADMAP 92：同時廣播 NpcSpeech，前端在 NPC 頭頂畫對話泡泡。
                        let _ = tx.send(std::sync::Arc::new(crate::protocol::ServerMsg::NpcSpeech {
                            npc_id: speaker_id.to_string(),
                            npc_name: s_name.to_string(),
                            text,
                            display_secs: 8,
                            wx,
                            wy,
                        }));
                    });
                }
            }

            // 白日工位對話（ROADMAP 81）：白天 NPC 在工位互相閒聊（ROADMAP 114：0 玩家也持續）。
            {
                let is_day = app.daynight.read().unwrap().phase() == crate::daynight::Phase::Day;
                let talk_pair = app.daytime_talk.write().unwrap().tick(dt, is_day);
                if let Some(pair) = talk_pair {
                    let tx_chat = app.tx_chat.clone();
                    let tx = app.tx.clone();
                    let sem = app.daytime_talk_sem.clone();
                    let speaker_id = pair.speaker_id;
                    let listener_id = pair.listener_id;
                    // 取說話者目前世界座標（白天崗位），供 NpcSpeech 泡泡定位。
                    let (wx, wy) = app.npc_schedule.read().unwrap()
                        .get_pos(speaker_id)
                        .unwrap_or_else(|| crate::npc_schedule::fallback_pos(speaker_id));
                    tokio::spawn(async move {
                        // 非阻塞嘗試取得 Semaphore；若已有白日對話進行中則直接略過。
                        let Ok(_permit) = sem.try_acquire_owned() else { return };
                        let text = crate::daytime_talk::generate_talk(speaker_id, listener_id).await;
                        let s_name = crate::daytime_talk::display_name(speaker_id);
                        let l_name = crate::daytime_talk::display_name(listener_id);
                        let _ = tx_chat.send(format!(
                            "☀️ [{s_name}] 對 [{l_name}] 說：「{text}」"
                        ));
                        // ROADMAP 92：同時廣播 NpcSpeech，前端在 NPC 頭頂畫對話泡泡。
                        let _ = tx.send(std::sync::Arc::new(crate::protocol::ServerMsg::NpcSpeech {
                            npc_id: speaker_id.to_string(),
                            npc_name: s_name.to_string(),
                            text,
                            display_secs: 8,
                            wx,
                            wy,
                        }));
                    });
                }
            }

            // 晨喚（ROADMAP 77）：日夜循環進入黎明時，凱爾長老廣播晨間致辭。
            // ROADMAP 114：0 玩家時仍持續，讓世界保持日夜節律。
            {
                let online_count = app.players.read().unwrap().len();
                let current_phase = app.daynight.read().unwrap().phase();
                let should_call = app.dawn_call.write().unwrap().tick(dt, current_phase);
                if should_call {
                    let tx_chat = app.tx_chat.clone();
                    let sem = app.dawn_call_sem.clone();
                    tokio::spawn(async move {
                        // 非阻塞嘗試取得 Semaphore；若已有晨喚進行中則直接略過。
                        let Ok(_permit) = sem.try_acquire_owned() else { return };
                        let text = crate::npc_dawn_call::generate_dawn_call(online_count).await;
                        let _ = tx_chat.send(format!(
                            "🌅 [{chief}] 朗聲道：「{text}」",
                            chief = crate::npc_dawn_call::CHIEF_DISPLAY_NAME,
                        ));
                    });
                }
            }

            // NPC 午鐘廣播（ROADMAP 79）：黎明→白天轉換時，工匠老胡廣播開工令。
            // 與晨喚（凱爾長老）和暮告（薇拉）形成三時段節律：黎明/日出/黃昏。
            // ROADMAP 114：0 玩家時仍持續，讓世界保持日夜節律。
            {
                let online_count = app.players.read().unwrap().len();
                let current_phase = app.daynight.read().unwrap().phase();
                let should_call = app.noon_bell.write().unwrap().tick(dt, current_phase);
                if should_call {
                    let tx_chat = app.tx_chat.clone();
                    let sem = app.noon_bell_sem.clone();
                    tokio::spawn(async move {
                        // 非阻塞嘗試取得 Semaphore；若已有午鐘進行中則直接略過。
                        let Ok(_permit) = sem.try_acquire_owned() else { return };
                        let text = crate::npc_noon_bell::generate_noon_bell(online_count).await;
                        let _ = tx_chat.send(format!(
                            "☀️ [{hu}] 喊道：「{text}」",
                            hu = crate::npc_noon_bell::HU_DISPLAY_NAME,
                        ));
                    });
                }
            }

            // NPC 暮告（ROADMAP 78）：白天→黃昏轉換時，商人薇拉廣播傍晚感言。
            // 與晨喚形成完整日夜節律——黎明有凱爾長老，黃昏有商人薇拉。
            // ROADMAP 114：0 玩家時仍持續，讓世界保持日夜節律。
            {
                let online_count = app.players.read().unwrap().len();
                let current_phase = app.daynight.read().unwrap().phase();
                let should_call = app.dusk_call.write().unwrap().tick(dt, current_phase);
                if should_call {
                    let tx_chat = app.tx_chat.clone();
                    let sem = app.dusk_call_sem.clone();
                    tokio::spawn(async move {
                        // 非阻塞嘗試取得 Semaphore；若已有暮告進行中則直接略過。
                        let Ok(_permit) = sem.try_acquire_owned() else { return };
                        let text = crate::npc_dusk_call::generate_dusk_call(online_count).await;
                        let _ = tx_chat.send(format!(
                            "🌇 [{vela}] 說：「{text}」",
                            vela = crate::npc_dusk_call::VELA_DISPLAY_NAME,
                        ));
                    });
                }
            }

            // NPC 入夜守衛令（ROADMAP 80）：黃昏→夜晚轉換時，獵手蘭卡廣播守衛令。
            // 與晨喚（凱爾長老）、午鐘（老胡）及暮告（薇拉）形成四時段節律。
            // ROADMAP 114：0 玩家時仍持續，讓世界保持日夜節律。
            {
                let online_count = app.players.read().unwrap().len();
                let current_phase = app.daynight.read().unwrap().phase();
                let should_call = app.night_watch.write().unwrap().tick(dt, current_phase);
                if should_call {
                    let tx_chat = app.tx_chat.clone();
                    let sem = app.night_watch_sem.clone();
                    tokio::spawn(async move {
                        // 非阻塞嘗試取得 Semaphore；若已有守衛令進行中則直接略過。
                        let Ok(_permit) = sem.try_acquire_owned() else { return };
                        let text = crate::npc_night_watch::generate_night_watch(online_count).await;
                        let _ = tx_chat.send(format!(
                            "🌙 [{ranka}] 低沉道：「{text}」",
                            ranka = crate::npc_night_watch::RANKA_DISPLAY_NAME,
                        ));
                    });
                }
            }

            // NPC 餘裕回補（ROADMAP 62）：每 RESTOCK_INTERVAL_SECS 秒對所有 NPC 補 +1 庫存（至上限）。
            // 讓送完餘裕的 NPC 隨時間恢復，維持「稀缺但不永久缺貨」的體感。
            {
                let restock_ticks = crate::npc_chat::RESTOCK_INTERVAL_SECS * TICK_HZ as u64;
                if tick % restock_ticks == 0 {
                    let mut restocked: Vec<(String, u32)> = Vec::new();
                    {
                        let mut stock = app.npc_gift_stock.write().unwrap();
                        for npc in crate::npc_chat::NPCS {
                            let e = stock.entry(npc.id.to_string()).or_insert(0);
                            let new_val = crate::npc_chat::restock_npc_stock(*e);
                            if new_val != *e {
                                *e = new_val;
                                restocked.push((npc.id.to_string(), new_val));
                            }
                        }
                    }
                    for (npc_id, s) in restocked {
                        app.npc_memory_store.save_gift_stock(npc_id, s);
                    }
                }
            }

            // 商人金庫回補（ROADMAP 100）：每 RESTOCK_INTERVAL_SECS 秒補充商隊收入，讓金庫慢慢恢復。
            {
                let restock_ticks = crate::npc_treasury::RESTOCK_INTERVAL_SECS * TICK_HZ as u64;
                if tick % restock_ticks == 0 && tick > 0 {
                    app.npc_treasury.write().unwrap().tick_restock();
                }
            }

            // 商人販售庫存補貨（ROADMAP 104）：每 STOCK_RESTOCK_INTERVAL_SECS 秒補充各品項庫存。
            // 供應鏈進貨成本（ROADMAP 107）：補貨時向上游付進貨成本，乙太從商人金庫流出。
            {
                let restock_ticks = crate::npc_stock::STOCK_RESTOCK_INTERVAL_SECS * TICK_HZ as u64;
                if tick % restock_ticks == 0 && tick > 0 {
                    let delta = app.npc_stock.write().unwrap().tick_restock();
                    let supply_cost = crate::supply_chain::total_supply_cost(&delta);
                    if supply_cost > 0 {
                        app.npc_treasury.write().unwrap()
                            .deduct(crate::npc_treasury::MERCHANT_HOME, supply_cost);
                        tracing::debug!(supply_cost, "供應鏈進貨成本：故鄉商人金庫 -{}", supply_cost);
                    }
                }
            }

            // NPC 生命週期 tick（ROADMAP 66）：推進壽命計時器，廣播老年 / 退休事件。
            {
                let events = app.npc_lifecycle.write().unwrap().tick(dt as f64);
                for event in events {
                    use crate::npc_lifecycle::LifecycleEvent;
                    match event {
                        LifecycleEvent::ElderPhase { npc_id, display } => {
                            // 老年期：從居民中選最年長者為徒弟，廣播收徒公告（ROADMAP 116）。
                            let already_has = app.npc_lifecycle.read().unwrap().has_apprentice(&npc_id);
                            if !already_has {
                                let apprentice = app.residents.read().unwrap().oldest_resident_name()
                                    .map(|s| s.to_string());
                                if let Some(name) = apprentice {
                                    app.npc_lifecycle.write().unwrap().set_apprentice(&npc_id, name.clone());
                                    let msg = format!(
                                        "🔮 {} 感到乙太的呼喚，決定收 {} 為徒，開始傾囊傳授畢生所學。",
                                        display, name
                                    );
                                    let _ = app.tx_chat.send(msg);
                                }
                            }
                        }
                        LifecycleEvent::RetirementSoon { display: _, msg, .. } => {
                            let _ = app.tx_chat.send(msg);
                        }
                        LifecycleEvent::RetiredToEther { npc_id, old_display, new_display, farewell_msg, arrival_msg } => {
                            let _ = app.tx_chat.send(farewell_msg.clone());
                            let _ = app.tx_chat.send(arrival_msg);
                            app.world_log.write().unwrap().push(format!(
                                "{}回歸乙太，{}接任。",
                                old_display, new_display
                            ));
                            tracing::info!(npc = %npc_id, old = %old_display, new = %new_display, "NPC 回歸乙太，繼承人登場");
                        }
                    }
                }
            }

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

            // 每 60 tick (約 4 秒) 更新一次大工程貢獻者名單（若有工程）。
            if tick % 60 == 0 {
                let project_id = app.town_project.read().unwrap().project_id.clone();
                let store = app.town_project_store.clone();
                let project_lock = app.town_project.clone();
                tokio::spawn(async move {
                    let list = store.load_top_contributors(&project_id).await;
                    project_lock.write().unwrap().update_contributors(list);
                });
            }

            // 廣播快照——只在有訂閱者時(tick 開頭已判定的 want_broadcast)才建構。
            // ③ 無限世界（切片 C）：傳出 Arc<ServerMsg> 原始結構，不在此序列化。
            if want_broadcast {
                let snapshot = {
                    let players = app.players.read().unwrap();
                    // 每次快照帶上 NPC 目錄（六大商人，收購價套用浮動市場價格）。
                    let now_secs = unix_secs();
                    let dm = app.dynamic_prices.read().unwrap();
                    let sch = app.npc_schedule.read().unwrap();

                    let mut npc_views = Vec::new();

                    // —— 故鄉村落 NPC（會移動）——
                    let lc = app.npc_lifecycle.read().unwrap();
                    for s in crate::npc_schedule::VILLAGE_NPCS {
                        let pos = sch.get_pos(s.id).unwrap_or((s.station_pos.x, s.station_pos.y));
                        let (buy_list, sell_list) = match s.id {
                            "merchant" => (
                                build_dynamic_buy_list(NPC_BUY_LIST, &dm, now_secs),
                                // ROADMAP 104：故鄉商人販售清單含庫存資訊（稀缺溢價 + 剩餘數量）。
                                build_home_sell_list_with_stock(NPC_SELL_LIST, &app.npc_stock.read().unwrap()),
                            ),
                            _ => (Vec::new(), Vec::new()), // 其他 NPC 暫無商店功能
                        };
                        npc_views.push(NpcView {
                            id: s.id.to_string(),
                            name: lc.current_display(s.id).to_string(),
                            x: pos.0,
                            y: pos.1,
                            buy_list,
                            sell_list,
                        });
                    }
                    drop(lc);

                    // —— 其他星球商人（固定位置）——
                    let (vmx, vmy) = verdant_merchant_pos();
                    npc_views.push(NpcView {
                        id: "verdant_merchant".to_string(),
                        name: "🌿 翠幽商人".to_string(),
                        x: vmx,
                        y: vmy,
                        buy_list: build_dynamic_buy_list(VERDANT_BUY_LIST, &dm, now_secs),
                        sell_list: build_static_sell_list(VERDANT_SELL_LIST),
                    });
                    let (cmx, cmy) = crimson_merchant_pos();
                    npc_views.push(NpcView {
                        id: "crimson_merchant".to_string(),
                        name: "🔴 赤焰商人".to_string(),
                        x: cmx,
                        y: cmy,
                        buy_list: build_dynamic_buy_list(CRIMSON_BUY_LIST, &dm, now_secs),
                        sell_list: build_static_sell_list(CRIMSON_SELL_LIST),
                    });
                    let (vmx2, vmy2) = void_merchant_pos();
                    npc_views.push(NpcView {
                        id: "void_merchant".to_string(),
                        name: "🌑 虛空商人".to_string(),
                        x: vmx2,
                        y: vmy2,
                        buy_list: build_dynamic_buy_list(VOID_BUY_LIST, &dm, now_secs),
                        sell_list: build_static_sell_list(VOID_SELL_LIST),
                    });
                    let (amx, amy) = aether_merchant_pos();
                    npc_views.push(NpcView {
                        id: "aether_merchant".to_string(),
                        name: "🌫️ 霧醚商人".to_string(),
                        x: amx,
                        y: amy,
                        buy_list: build_dynamic_buy_list(AETHER_BUY_LIST, &dm, now_secs),
                        sell_list: build_static_sell_list(AETHER_SELL_LIST),
                    });
                    let (omx, omy) = origin_merchant_pos();
                    npc_views.push(NpcView {
                        id: "origin_merchant".to_string(),
                        name: "🌟 星源商人".to_string(),
                        x: omx,
                        y: omy,
                        buy_list: build_dynamic_buy_list(ORIGIN_BUY_LIST, &dm, now_secs),
                        sell_list: build_static_sell_list(ORIGIN_SELL_LIST),
                    });

                    // —— 城外旅人（ROADMAP 74）——：可見時加入快照。
                    let traveler_xy: Option<(f32, f32)> = {
                        let tv = app.traveler.read().unwrap();
                        if tv.is_visible() {
                            npc_views.push(NpcView {
                                id: "traveler".to_string(),
                                name: format!("🧳 {}", tv.name()),
                                x: tv.x,
                                y: tv.y,
                                buy_list: Vec::new(),
                                sell_list: Vec::new(),
                            });
                            Some((tv.x, tv.y))
                        } else {
                            None
                        }
                    };

                    // —— 路人居民（ROADMAP 115）——：純模板 NPC，無商店功能。
                    {
                        let res = app.residents.read().unwrap();
                        for (id, name, x, y) in res.views() {
                            npc_views.push(NpcView {
                                id: id.to_string(),
                                name: name.to_string(),
                                x,
                                y,
                                buy_list: Vec::new(),
                                sell_list: Vec::new(),
                            });
                        }
                    }

                    ServerMsg::Snapshot {
                        tick,
                        players: players.values().map(|p| p.view(&sch, traveler_xy, app.wandering_merchant.read().unwrap().is_active())).collect(),
                        fields: field_views,
                        nodes: node_views,
                        enemies: enemy_views,
                        daynight: daynight_view.expect("want_broadcast 時必有 daynight_view"),
                        listings: listing_views,
                        npcs: npc_views,
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
                            // ⚠️ 死鎖修正：這裡**沿用** Snapshot 起頭(本函式上方)已持有的 `players` read guard，
                            // **絕不可**對同一把 `app.players`(std RwLock)同執行緒二次上鎖——glibc 寫者優先下，
                            // 若此刻有玩家動作的 writer 正在等鎖，第二次 read 會被擋住、外層 guard 永遠放不掉 → 永久死鎖。
                            // 查名字：先從線上玩家找，再從 UserStore 找（含離線玩家）
                            registry.all_plots_view(|uid| {
                                players.get(&uid).map(|p| p.name.clone())
                                    .or_else(|| app.users.get(uid).map(|u| u.name))
                            })
                        },
                        // 牧場狀態（ROADMAP 48）：只送有雞或有蛋的地塊。
                        ranch_plots: app.ranch.read().unwrap().all_active_views(),
                        // 農地作物狀態（ROADMAP 49）：只送有種植作物的地塊。
                        farm_crop_plots: app.farm_crops.read().unwrap().all_active_views(),
                        // 夜採星晶礦脈（ROADMAP 50）：夜間有節點，白天空陣列。
                        star_crystals: app.star_crystals.read().unwrap().views(),
                        // 村落節慶加成剩餘秒數（ROADMAP 64）：0 = 無加成；>0 = EXP +30%。
                        village_buff_remaining_secs: {
                            let lock = app.village_buff_until.read().unwrap();
                            lock.as_ref()
                                .map(|&expiry| {
                                    let now = std::time::Instant::now();
                                    if now < expiry {
                                        expiry.duration_since(now).as_secs() as u32
                                    } else {
                                        0
                                    }
                                })
                                .unwrap_or(0)
                        },
                        village_treasury: *app.village_treasury.read().unwrap(),
                        weather: app.weather.read().unwrap().view(),
                        sprinklers: app.sprinklers.read().unwrap().views(),
                        // 廣場聚會剩餘秒數（ROADMAP 124）：0 = 無聚會；>0 = 全服 EXP +20%。
                        gathering_secs: app.community_gathering.read().unwrap().remaining_secs(),
                        // 目前求助中的居民 id 清單（ROADMAP 125）：前端顯示「🤝 幫忙」按鈕用。
                        active_help_requests: app.residents.read().unwrap().requesting_ids(),
                        // 居民心情（ROADMAP 126）：前端在快樂居民頭上顯示 💛。
                        resident_moods: app.residents.read().unwrap().moods(),
                        // 城鎮繁榮等級（ROADMAP 128）：0=凋零 1=平靜 2=生機 3=繁盛。
                        town_prosperity_level: app.residents.read().unwrap().prosperity_level(),
                        // 城鎮大工程狀態（ROADMAP 131）。
                        town_project: app.town_project.read().unwrap().view(),
                        // 天文台星象預報（ROADMAP 132）。
                        star_forecast_secs: app.observatory.read().unwrap().remaining_secs(),
                        star_forecast_bonus: app.observatory.read().unwrap().bonus_kind_str().to_string(),
                        // 流星雨（ROADMAP 133）。
                        meteor_shower_secs: app.meteor_shower.read().unwrap().remaining_secs(),
                        dust_nodes: app.meteor_shower.read().unwrap().active_nodes()
                            .map(|n| crate::protocol::DustNodeView { id: n.id, wx: n.wx, wy: n.wy, is_rainbow: n.is_rainbow })
                            .collect(),
                        // 旅行商人（ROADMAP 135）。
                        wandering_merchant_secs: app.wandering_merchant.read().unwrap().remaining_secs(),
                        wandering_catalog: {
                            let wm = app.wandering_merchant.read().unwrap();
                            wm.catalog.iter().map(|e| crate::protocol::WanderingCatalogEntry {
                                item: e.item,
                                price_ether: e.price_ether,
                                remaining: e.remaining(),
                            }).collect()
                        },
                        // 旅行商人限時委託（ROADMAP 136）。
                        merchant_quests: app.wandering_merchant.read().unwrap().quest_views(),
                        // 季節循環（ROADMAP 137）。
                        current_season: app.season.read().unwrap().current.as_str().to_string(),
                        season_remaining_secs: app.season.read().unwrap().remaining_secs(),
                        // 季節性野外採集節點（ROADMAP 154）：只廣播有剩餘次數的節點。
                        seasonal_nodes: app.seasonal_nodes.read().unwrap().active_nodes()
                            .map(|n| crate::protocol::SeasonalNodeView {
                                id: n.id,
                                wx: n.wx,
                                wy: n.wy,
                                season: n.season.as_str().to_string(),
                                charges: n.charges,
                            })
                            .collect(),
                        // 野生動物（ROADMAP 141 食物鏈）：只廣播存活個體。
                        wildlife: {
                            let wm = app.wildlife_manager.read().unwrap();
                            wm.animals.iter()
                                .filter(|a| a.alive)
                                .map(|a| crate::protocol::WildlifeView {
                                    id: a.id,
                                    kind: a.kind.as_str().to_string(),
                                    name: a.kind.display_name().to_string(),
                                    x: a.x,
                                    y: a.y,
                                    state: a.state_str().to_string(),
                                }).collect()
                        },
                        // 乙太微粒（ROADMAP 142 死亡餵養生命）：死亡獵物留下的乙太節點。
                        carion_orbs: {
                            let wm = app.wildlife_manager.read().unwrap();
                            wm.carion_orbs.iter()
                                .map(|o| crate::protocol::CarrionOrbView { id: o.id, x: o.x, y: o.y })
                                .collect()
                        },
                        // 物種聚落（ROADMAP 143）：靜態領地資料，前端渲染聚落圓圈 + 小地圖標記。
                        colonies: {
                            let wm = app.wildlife_manager.read().unwrap();
                            wm.colony_views()
                        },
                        // 物種態度（ROADMAP 144）：各物種對人類的態度值與層級。
                        species_attitudes: app.species_relations.read().unwrap().views(),
                        // 住家家具（ROADMAP 155）：廣播時以空陣列佔位，ws.rs 過濾層依玩家 id 填入本人家具。
                        home_furniture: vec![],
                        // 公民投票（ROADMAP 156）：當前活躍投票視圖 + 效果狀態。
                        civic_vote: app.civic_vote.read().unwrap().vote_view(),
                        civic_effect_secs: app.civic_vote.read().unwrap().effect_remaining_secs(),
                        civic_effect_kind: app.civic_vote.read().unwrap().active_effect_kind(),
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
                .map(|p| (p.id, p.name.clone(), p.species.clone(), p.x, p.y, p.ether, p.wallet.expansions(), p.exp, p.masteries, p.stats, p.skill_masteries))
                .collect(),
            authed.iter().map(|p| (p.id, p.inventory.clone())).collect(),
            authed.iter().map(|p| (p.id, p.equipment.clone())).collect(),
        )
    };
    if !online.is_empty() {
        // 先更新行程內 cache（同步,供重連 recall）,再非同步 upsert 到 Postgres。
        app.positions
            .remember_all(online.iter().map(|(id, _, _, x, y, e, we, exp, m, s, sk)| (*id, *x, *y, *e, *we, *exp, *m, *s, *sk)));
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
