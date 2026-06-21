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
            let (daynight_view, growth_rate, is_night, is_dawn, is_dusk, is_hot, is_cold) = {
                let mut daynight = app.daynight.write().unwrap();
                daynight.advance(dt);
                let phase = daynight.phase();
                let is_night = phase == crate::daynight::Phase::Night;
                // ROADMAP 305：本幀是否為破曉時段（供野鳥「拂曉滿林齊鳴」判定，與 is_night 同把時鐘鎖一次取得）。
                let is_dawn = phase == crate::daynight::Phase::Dawn;
                // ROADMAP 306：本幀是否為黃昏時段（供野鳥「黃昏暮鳴」判定，同把時鐘鎖一次取得）。
                let is_dusk = phase == crate::daynight::Phase::Dusk;
                let view = if want_broadcast {
                    Some(daynight.view())
                } else {
                    None
                };
                // ROADMAP 307：酷暑判定——光照強度超過 0.85 即為酷暑（盛夏正午）。
                let is_hot = daynight.light_level() > 0.85;
                // ROADMAP 308：寒涼判定——光照強度低於 0.5 即為寒涼（寒冬清晨／向晚，寒意最深的非夜時段）。
                let is_cold = daynight.light_level() < 0.5;
                (view, daynight.growth_rate(), is_night, is_dawn, is_dusk, is_hot, is_cold)
            };

            // 季節循環（ROADMAP 137）：推進季節計時器，切換時廣播公告。
            // 季節成長倍率疊乘在日夜倍率之上，獨立正交不互相侵犯。
            let (season_growth, season_now, is_summer, is_winter, is_autumn, is_spring) = {
                let mut s = app.season.write().unwrap();
                let is_summer = s.current == crate::season::Season::Summer;
                // ROADMAP 308：本幀是否為冬季（供哺乳獸「寒冬哆嗦取暖」判定，與 is_summer 同把季節鎖一次取得）。
                let is_winter = s.current == crate::season::Season::Winter;
                // ROADMAP 311：本幀是否為秋季（供野鳥「秋日群鳥集結」判定，同把季節鎖一次取得）。
                let is_autumn = s.current == crate::season::Season::Autumn;
                // ROADMAP 312：本幀是否為春季（供草食獸「春日嗅花」判定，同把季節鎖一次取得）。
                let is_spring = s.current == crate::season::Season::Spring;
                if let Some(new_season) = s.tick(dt) {
                    let _ = app.tx_chat.send(new_season.announce_text().to_string());
                    tracing::info!(season = new_season.as_str(), "季節切換");
                    // 季節性野外採集節點（ROADMAP 154）：季節切換時重置節點。
                    app.seasonal_nodes.write().unwrap().on_season_change(new_season);
                    // 城鎮記憶石（ROADMAP 157）：季節更替是值得留存的世界大事。
                    app.town_memory.write().unwrap().push_event(
                        "🍂",
                        format!("季節更替——進入{}了", new_season.display_name()),
                    );
                }
                (s.growth_rate_modifier(), s.current, is_summer, is_winter, is_autumn, is_spring)
            };

            // 夜採星晶（ROADMAP 50）：偵測日夜轉換事件，生成或清除星晶礦脈。
            if is_night && !prev_is_night {
                // 剛進入夜間：生成本夜礦脈。
                app.star_crystals.write().unwrap().spawn_for_night();
                // 觀星連星座（ROADMAP 347）：夜數 +1，讓「今夜星座」逐夜輪替（lock-free 原子）。
                app.night_index
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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
            // 養蜂釀蜜（ROADMAP 412）：在農地寫鎖內順手收集每位巢主的「蜜源」（田裡生長中作物數），
            // 出鎖後餵給蜂巢 tick——蜜源越豐、產蜜越快（不巢狀上鎖、守 prod-deadlock）。
            let mut blooms_by_owner: std::collections::HashMap<uuid::Uuid, u32> =
                std::collections::HashMap::new();
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
                for (owner, field) in fields.iter_mut() {
                    // 草原細雨時先替所有缺水作物補水，再正常 tick 成長。
                    if is_raining {
                        field.water_all_planted();
                    }
                    // ROADMAP 453：全域季節倍率已 bake 進 dt（對所有作物一致）；品種 × 當季的
                    // 偏好（耐寒／戀夏）由 field.tick 內逐株疊上，故把當前季節一併傳入。
                    field.tick(dt * effective_growth, season_now);
                    // 記下這塊地的蜜源（生長中作物數），供蜂巢產蜜放大。
                    let blooms = field.blooming_count();
                    if blooms > 0 {
                        blooms_by_owner.insert(*owner, blooms);
                    }
                }
                // 公共農地與個人地塊同步成長，廣播時以 owner=nil 加入列表讓前端辨識。
                let pub_view = {
                    let mut pf = app.pub_field.write().unwrap();
                    if is_raining {
                        pf.water_all_planted();
                    }
                    pf.tick(dt * effective_growth, season_now);
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

            // 養蜂釀蜜（ROADMAP 412）：依各巢主的蜜源推進蜂巢產蜜（fields 鎖已釋放，獨立取 apiary 寫鎖）。
            app.apiary.write().unwrap().tick(dt, &blooms_by_owner);

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
                            regrow: p.node.regrowth_progress(),
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

            // ROADMAP 165：取野生動物快照（讀鎖即放），供怪物追獵目標更新用。
            let wildlife_snap: Vec<(u32, crate::wildlife::WildlifeKind, f32, f32)> = {
                app.wildlife_manager.read().unwrap().alive_snapshot()
            };

            // 推進敵人:重生倒數(被打倒的復活)+ 移動(巡邏 / 追擊走近的玩家)。兩者無條件跑;
            // view 只在廣播時建。怪會動起來——撲向玩家、沒人時漂回家,世界因此活起來。
            // ③ 無限世界: 先確保玩家周圍區塊已載入。
            let (enemy_views, monster_wildlife_kills, enemy_disc): (Vec<EnemyView>, Vec<(crate::combat::EnemyKind, u32, crate::wildlife::WildlifeKind, f32, f32)>, Vec<(crate::combat::EnemyKind, f32, f32)>) = {
                let mut enemies = app.enemies.write().unwrap();
                {
                    let players = app.players.read().unwrap();
                    for p in players.values() {
                        enemies.ensure_chunks_around(p.x, p.y, 1000.0);
                    }
                }
                enemies.tick(dt);
                // ROADMAP 163：依怪物物種態度層級更新每種怪的 aggro 倍率。
                {
                    let mults = app.monster_species.read().unwrap().aggro_multipliers_snapshot();
                    enemies.update_aggro_multipliers(mults);
                }
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
                // ROADMAP 165：更新怪物追獵目標，收集本幀擊殺事件。
                enemies.update_wildlife_targets(&wildlife_snap, &chase_targets);
                let kills = enemies.collect_wildlife_kills();
                // ROADMAP 333 生態圖鑑：永遠收一份「活著的守護者怪物」座標＋種類快照（不限廣播幀），
                // 供下方圖鑑發現比對。鎖內取、owned Vec 帶出，鎖外才碰玩家表（不巢狀上鎖）。
                let enemy_disc: Vec<(crate::combat::EnemyKind, f32, f32)> = enemies
                    .enemies()
                    .iter()
                    .filter(|p| p.enemy.is_alive())
                    .map(|p| (p.enemy.kind(), p.x, p.y))
                    .collect();
                // ROADMAP 424：怪物王預警重擊的蓄力進度由「伺服器時鐘 + 怪物 id」決定性推導，
                // 不存任何新狀態。now_secs 與下方傷害結算用的時鐘同源（皆由 tick 換算）。
                let now_secs = tick as f64 / TICK_HZ as f64;
                let views = if want_broadcast {
                    enemies
                        .enemies()
                        .iter()
                        .map(|p| {
                            let notorious = p.level >= p.base_level.saturating_add(3);
                            EnemyView {
                                kind: p.enemy.kind(),
                                x: p.x,
                                y: p.y,
                                level: p.level,
                                hp: p.enemy.remaining_hp(),
                                max_hp: p.enemy.max_hp(),
                                alive: p.enemy.is_alive(),
                                notorious,
                                resting: is_night && crate::enemy_field::is_night_rester(p.id),
                                // ROADMAP 183：retreat_timer>0 → 潰逃中，前端畫 💨。
                                routing: p.retreat_timer > 0.0,
                                // ROADMAP 424：只有存活的怪物王在蓄力窗內才帶蓄力進度（地面預警圈）。
                                slam_windup: if notorious && p.enemy.is_alive() {
                                    crate::boss_slam::windup_progress(p.id, now_secs)
                                } else {
                                    None
                                },
                            }
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                (views, kills, enemy_disc)
            };

            // ROADMAP 165：套用怪物獵殺野生動物事件（標記死亡 + 生成乙太微粒 + 廣播）。
            {
                let mut wm = app.wildlife_manager.write().unwrap();
                for &(monster_kind, wildlife_id, _, _, _) in &monster_wildlife_kills {
                    if let Some(ev) = wm.on_monster_kills_wildlife(wildlife_id, monster_kind) {
                        use crate::wildlife::WildlifeEvent;
                        if let WildlifeEvent::MonsterHunted { monster_kind, wildlife_kind, x, y } = ev {
                            let msg = format!(
                                "🌿 城外 ({:.0},{:.0})：{} 獵殺了 {}，弱肉強食是生態的法則。",
                                x, y,
                                monster_kind.display_name(),
                                wildlife_kind.display_name(),
                            );
                            let _ = app.tx_chat.send(msg);
                        }
                    }
                }
            }

            // ── ROADMAP 333 生態圖鑑：玩家走近野生動物／守護者怪物即「發現」、點亮圖鑑、首見給乙太 ──
            // 鎖序乾淨：wildlife（wildlife_snap）與 enemies（enemy_disc）都已在上面各自 scope 取完、
            // 以 owned 快照帶到此處，這裡只新開一把 players 寫鎖（無巢狀上鎖，守 prod-deadlock 鐵律）。
            // 發現天然冪等（discover 對已點亮位元回 false、不重複領獎），故逐幀跑安全、不需冷卻帳本。
            // codex 與新增的乙太都隨既有玩家快照廣播，前端比對 codex 位元差噴「新發現」、乙太差噴「+N」。
            if !wildlife_snap.is_empty() || !enemy_disc.is_empty() {
                use crate::field_guide::{
                    bit_for_enemy, bit_for_wildlife, celebrate_line, discover, newly_completed,
                    reward_for_bit, title_earned_line, title_for, DISCOVER_RADIUS,
                };
                let r2 = DISCOVER_RADIUS * DISCOVER_RADIUS;
                // ROADMAP 334：本幀新達成的圖鑑里程碑要廣播全世界同慶，收集起來、出鎖後再送。
                let mut milestone_msgs: Vec<String> = Vec::new();
                {
                    let mut players = app.players.write().unwrap();
                    for p in players.values_mut() {
                        // 倒下玩家休息中、不在世界裡探查（比照戰鬥／追擊略過倒下者）。
                        if p.vitals.is_downed() {
                            continue;
                        }
                        let codex_before = p.codex; // ROADMAP 334：本幀發現前的圖鑑，判定是否湊滿一類
                        // 野生動物（wildlife_snap：(id, kind, x, y)）。
                        for &(_, kind, wx, wy) in &wildlife_snap {
                            let dx = p.x - wx;
                            let dy = p.y - wy;
                            if dx * dx + dy * dy <= r2 {
                                let bit = bit_for_wildlife(kind);
                                let (mask, first) = discover(p.codex, bit);
                                if first {
                                    p.codex = mask;
                                    p.ether = p.ether.saturating_add(reward_for_bit(bit));
                                }
                            }
                        }
                        // 守護者怪物（enemy_disc：(kind, x, y)，已濾活著的）。
                        for &(kind, ex, ey) in &enemy_disc {
                            let dx = p.x - ex;
                            let dy = p.y - ey;
                            if dx * dx + dy * dy <= r2 {
                                let bit = bit_for_enemy(kind);
                                let (mask, first) = discover(p.codex, bit);
                                if first {
                                    p.codex = mask;
                                    p.ether = p.ether.saturating_add(reward_for_bit(bit));
                                }
                            }
                        }
                        // ROADMAP 334：本幀的發現若湊滿了某一整類 → 一次性大獎 + 世界同慶。
                        // newly_completed 只在「由不滿→滿」那一刻回傳該里程碑，天然每位玩家每類只發一次。
                        if p.codex != codex_before {
                            let crossed = newly_completed(codex_before, p.codex);
                            if !crossed.is_empty() {
                                for m in &crossed {
                                    p.ether = p.ether.saturating_add(m.reward_ether);
                                    milestone_msgs.push(celebrate_line(m, &p.name));
                                }
                                // ROADMAP 335：集滿里程碑即「配戴」一枚蒐集稱號，全世界都看得到。
                                // 達成的當下世界頻道也報一聲新配戴的最高階稱號（同 codex 推導、零新狀態）。
                                if let Some(t) = title_for(p.codex) {
                                    milestone_msgs.push(title_earned_line(t, &p.name));
                                }
                            }
                        }
                    }
                }
                // 出鎖後廣播同慶（極稀有：每位玩家每類一生一次，不會洗頻）。
                for msg in milestone_msgs {
                    let _ = app.tx_chat.send(msg);
                }
            }

            // ── ROADMAP 351 階梯榮銜：偵測熟練度跨階，前端慶賀＋跨師匠以上世界同慶 ──
            // 熟練度 XP 在 ws.rs 各活動處累加，但「跨階」需逐幀比對：以每位玩家的
            // seen_mastery_tiers（上次已見階級 tier 快照）與當前 masteries.tier_snapshot() 比對；
            // 某條 tier 升高即晉階——更新快照（前端據既有 masteries 廣播自行噴本地慶賀），跨到師匠
            // 以上（is_high）才另蒐集世界頻道同慶台詞（避免每個學徒洗頻）。連線／重連時快照已以當前
            // 熟練度種下，故只有「在線苦練跨階」才觸發、回鍋高階玩家不會被回放歷史晉階。
            // 只新開一把 players 寫鎖、純記憶體＋純函式、廣播一律出鎖後送（守 prod-deadlock 鐵律）。
            {
                let mut rank_up_msgs: Vec<String> = Vec::new();
                {
                    let mut players = app.players.write().unwrap();
                    for p in players.values_mut() {
                        let now = p.masteries.tier_snapshot();
                        if now == p.seen_mastery_tiers {
                            continue;
                        }
                        for (i, class) in crate::class::JobClass::ALL.iter().enumerate() {
                            if now[i] > p.seen_mastery_tiers[i] {
                                let rank = p.masteries.rank(*class);
                                if rank.is_high() {
                                    let line = crate::class::rank_up_line(*class, rank, &p.name);
                                    let chat = crate::protocol::ServerMsg::Chat {
                                        from: "系統".into(),
                                        text: line,
                                    };
                                    if let Ok(json) = serde_json::to_string(&chat) {
                                        rank_up_msgs.push(json);
                                    }
                                }
                            }
                        }
                        p.seen_mastery_tiers = now;
                    }
                }
                for msg in rank_up_msgs {
                    let _ = app.tx_chat.send(msg);
                }
            }

            // ── ROADMAP 336 探索圖鑑：玩家走近各種奇景地形即「探索」、點亮圖鑑、首見給乙太 ──
            // 奇景地形多為實心格、玩家被碰撞擋在外緣，故在玩家四周取樣（中心＋八方向，
            // 半徑 EXPLORE_REACH），任一取樣點落在收錄地形上就記下。地形來源走 tile_deltas_snap
            // （玩家挖掘差異，已在上方快照）＋ world_core::tile_kind_at 後援，與碰撞判定同一份。
            // 探索天然冪等（explore 對已點亮位元回 false、不重複領獎），逐幀跑安全、不需冷卻帳本。
            // atlas 隨既有玩家快照廣播，前端比對位元差噴「新發現地形」、乙太差噴「+N」（鏡像 333）。
            // 只新開一把 players 寫鎖、無巢狀上鎖（守 prod-deadlock 鐵律）。
            {
                use crate::terrain_atlas::{bit_for_tile, explore, reward_for_bit, EXPLORE_REACH};
                // 取樣偏移（單位向量）：中心 + 八方向，乘上 EXPLORE_REACH 後即取樣點相對位移。
                const OFFS: [(f32, f32); 9] = [
                    (0.0, 0.0),
                    (1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0),
                    (1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0),
                ];
                let mut players = app.players.write().unwrap();
                for p in players.values_mut() {
                    // 倒下玩家休息中、不在世界裡探查（比照圖鑑發現略過倒下者）。
                    if p.vitals.is_downed() {
                        continue;
                    }
                    for (ox, oy) in OFFS {
                        let sx = p.x + ox * EXPLORE_REACH;
                        let sy = p.y + oy * EXPLORE_REACH;
                        let (cx, cy, tx, ty) = crate::tiles::world_to_cell(sx, sy);
                        let kind = tile_deltas_snap
                            .get(&(cx, cy, tx, ty))
                            .copied()
                            .unwrap_or_else(|| world_core::tile_kind_at(sx as f64, sy as f64));
                        if let Some(bit) = bit_for_tile(kind) {
                            let (mask, first) = explore(p.atlas, bit);
                            if first {
                                p.atlas = mask;
                                p.ether = p.ether.saturating_add(reward_for_bit(bit));
                            }
                        }
                    }
                }
            }

            // ── ROADMAP 337 天象圖鑑：玩家身處某種天象之下即「目睹」、點亮圖鑑、首見給乙太 ──
            // 與 333／336 的「空間蒐集」不同，天象是「時間蒐集」：天氣、流星雨、滿月都是全域訊號，
            // 凡此刻在線的玩家便共同身處同一片天空之下，無需逐人取樣座標——先算出「當下天空正在發生
            // 的天象位元」一次，若非空才開一把 players 寫鎖（晴朗無事的多數幀直接略過、零鎖零成本）。
            // 目睹天然冪等（witness 對已點亮位元回 false、不重複領獎）。skylog 隨既有玩家快照廣播，
            // 前端比對位元差噴「新目睹天象」、乙太差噴「+N」（鏡像 333／336）。只新開一把寫鎖、
            // 無巢狀上鎖（守 prod-deadlock 鐵律）。
            {
                use crate::sky_codex::{active_bits, reward_for_bit, witness};
                // 全域訊號（皆為短暫持鎖、不跨 await 的便宜讀取）：當下天氣 / 流星雨 / 滿月夜。
                let weather_key = app.weather.read().unwrap().view().weather_type;
                let meteor_active = app.meteor_shower.read().unwrap().is_active();
                let is_night = matches!(
                    app.daynight.read().unwrap().phase(),
                    crate::daynight::Phase::Night | crate::daynight::Phase::Dusk
                );
                // 滿月要掛在夜空才看得見圓滿（白天即使月相為滿也不算「滿月夜」）。
                let full_moon_night = is_night
                    && std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| crate::moon::is_full_moon(d.as_millis() as f64))
                        .unwrap_or(false);
                let active = active_bits(&weather_key, meteor_active, full_moon_night);
                // 多數時刻天空無可蒐集的天象（晴朗白天）→ active 為 0，整段略過、連寫鎖都不開。
                if active != 0 {
                    let mut players = app.players.write().unwrap();
                    for p in players.values_mut() {
                        // 逐一檢查當下發生的每種天象，凡玩家尚未目睹過的就點亮＋首見加乙太。
                        for bit in 0..crate::sky_codex::TOTAL as u8 {
                            if active & (1u64 << bit) == 0 {
                                continue;
                            }
                            let (mask, first) = witness(p.skylog, bit);
                            if first {
                                p.skylog = mask;
                                p.ether = p.ether.saturating_add(reward_for_bit(bit));
                            }
                        }
                    }
                }
            }

            // ── ROADMAP 339 玩家擊掌：兩名同區、靠得夠近、也都在比擊掌的玩家配成一對、迸特效 ──
            // 338 表情是單向廣播；擊掌是第一條「兩個真人各自出手、又站得夠近才成立」的雙向同步線。
            // 玩家比擊掌時 ws.rs 在他身上點亮一個短暫的意願倒數（high_five_offer），這裡每幀：
            // 把當下「還在比」（且在室外）的玩家依距離兩兩配對 → 配上的迸特效＋清意願、沒配上的遞減。
            // 配對是純函式（high_five::match_pairs，吃同區鍵＋座標、吐確定的配對），接線只新開一把
            // players 寫鎖、特效廣播在出鎖後才送（守 prod-deadlock 鐵律：無巢狀上鎖）。
            {
                // 先在寫鎖內配對＋更新意願，把要廣播的擊掌事件帶出鎖外再送。
                let matches: Vec<(uuid::Uuid, String, uuid::Uuid, String, f32, f32)> = {
                    let mut players = app.players.write().unwrap();
                    // 蒐集當下還在比擊掌、且在室外的玩家意願（室內外空間不同、不互配）。
                    let offers: Vec<crate::high_five::Offer> = players
                        .values()
                        .filter(|p| p.high_five_offer > 0 && p.indoor_plot_id.is_none())
                        .map(|p| crate::high_five::Offer {
                            id: p.id,
                            zone: p.planet.clone(),
                            x: p.x,
                            y: p.y,
                        })
                        .collect();
                    if offers.is_empty() {
                        Vec::new()
                    } else {
                        let pairs = crate::high_five::match_pairs(&offers);
                        // 先讀出每對的名字＋中點（不可變借用），再做意願清零（可變借用），避免交疊借用。
                        let mut evs = Vec::with_capacity(pairs.len());
                        let mut matched: std::collections::HashSet<uuid::Uuid> = std::collections::HashSet::new();
                        for (a, b) in &pairs {
                            if let (Some(pa), Some(pb)) = (players.get(a), players.get(b)) {
                                let mx = (pa.x + pb.x) * 0.5;
                                let my = (pa.y + pb.y) * 0.5;
                                evs.push((*a, pa.name.clone(), *b, pb.name.clone(), mx, my));
                                matched.insert(*a);
                                matched.insert(*b);
                            }
                        }
                        // 配上的清零意願（避免下幀重複迸特效）；沒配上的遞減、留待下幀再試。
                        for p in players.values_mut() {
                            if p.high_five_offer > 0 {
                                if matched.contains(&p.id) {
                                    p.high_five_offer = 0;
                                } else {
                                    p.high_five_offer -= 1;
                                }
                            }
                        }
                        evs
                    }
                };
                for (a_id, a_name, b_id, b_name, mx, my) in matches {
                    let _ = app.tx.send(std::sync::Arc::new(
                        crate::protocol::ServerMsg::HighFiveMatch {
                            a_id,
                            a_name,
                            b_id,
                            b_name,
                            mx,
                            my,
                            display_secs: crate::high_five::HIGH_FIVE_DISPLAY_SECS,
                        },
                    ));
                }
            }

            // ── ROADMAP 340 表情共鳴：一群靠近的玩家同時比同個表情 → 在重心迸放大發光特效 ──
            // 338 是各比各的、339 是兩人擊掌；共鳴是「群體同步」——完全長在 338 之上，沒有新指令，
            // 只把大家本就在比的表情湊在一起放大成眾人共享的大場面。玩家比表情時 ws.rs 點亮
            // 「最近表情」倒數（recent_emote），這裡每幀：把當下「最近還在比同個表情、且在室外」
            // 的玩家聚團偵測共鳴（emote_resonance::detect 純函式、確定可重現），共鳴成員清掉倒數
            // ＋在重心廣播 EmoteResonance，其餘遞減留待下幀。只新開一把 players 寫鎖、特效廣播在
            // 出鎖後才送（守 prod-deadlock 鐵律：無巢狀上鎖）。多數時刻沒人比表情→回響清單空、近乎零成本。
            {
                let bursts: Vec<(String, f32, f32, u32)> = {
                    let mut players = app.players.write().unwrap();
                    // 蒐集當下「最近還在比表情、且在室外」的玩家回響（室內外空間不同、不互湊）。
                    let echoes: Vec<crate::emote_resonance::Echo> = players
                        .values()
                        .filter_map(|p| {
                            let (kind, ttl) = p.recent_emote?;
                            if ttl > 0 && p.indoor_plot_id.is_none() {
                                Some(crate::emote_resonance::Echo {
                                    id: p.id,
                                    kind,
                                    zone: p.planet.clone(),
                                    x: p.x,
                                    y: p.y,
                                })
                            } else {
                                None
                            }
                        })
                        .collect();
                    if echoes.is_empty() {
                        Vec::new()
                    } else {
                        let resonances = crate::emote_resonance::detect(&echoes);
                        // 共鳴成員集合：他們的倒數清零（避免下幀重複迸），其餘遞減。
                        let mut in_resonance: std::collections::HashSet<uuid::Uuid> =
                            std::collections::HashSet::new();
                        let mut evs = Vec::with_capacity(resonances.len());
                        for r in &resonances {
                            // glyph 由索引查出；理論上必在白名單內（索引源自 ws.rs index_of）。
                            let glyph = crate::player_emote::glyph_at(r.kind)
                                .unwrap_or("✨")
                                .to_string();
                            evs.push((glyph, r.mx, r.my, r.size));
                            for m in &r.members {
                                in_resonance.insert(*m);
                            }
                        }
                        for p in players.values_mut() {
                            if let Some((_, ttl)) = p.recent_emote.as_mut() {
                                if in_resonance.contains(&p.id) {
                                    p.recent_emote = None;
                                } else if *ttl > 1 {
                                    *ttl -= 1;
                                } else {
                                    p.recent_emote = None;
                                }
                            }
                        }
                        evs
                    }
                };
                for (glyph, mx, my, size) in bursts {
                    let _ = app.tx.send(std::sync::Arc::new(
                        crate::protocol::ServerMsg::EmoteResonance {
                            glyph,
                            mx,
                            my,
                            size,
                            display_secs: crate::emote_resonance::RESONANCE_DISPLAY_SECS,
                        },
                    ));
                }
            }

            // ── ROADMAP 341 喝采人氣：對附近玩家「👏 喝采」→ 替對方人氣 +1、到階亮名牌徽記 ──
            // 338/339/340 都是迸完就散的一次性特效；喝采是第一筆「會留下印記」的互動——把社交
            // 從「即生即滅」推進到「沉澱成看得見的人氣身份」。玩家按喝采時 ws.rs 點亮 cheer_offer，
            // 這裡每幀：替「還在喝采、且在室外」的玩家挑最近的同區對象（pick_target 純函式）、過了
            // 每對象冷卻（can_cheer）就替**對方** cheers +1、寫冷卻、清意願、把事件帶出鎖外廣播；
            // 對方人氣跨階時世界頻道報一聲。只新開一把 players 寫鎖、廣播在出鎖後才送（守
            // prod-deadlock 鐵律：無巢狀上鎖）；多數時刻沒人喝采→意圖清單空、近乎零成本。
            {
                // (giver_name, target_id, target_name, target_cheers, mx, my)
                let cheered: Vec<(String, uuid::Uuid, String, u64, f32, f32)>;
                let mut tier_msgs: Vec<String> = Vec::new();
                {
                    let mut players = app.players.write().unwrap();
                    // 候選對象快照：所有室外玩家（室內外空間不同、不互喝）。
                    let candidates: Vec<crate::player_cheer::Candidate> = players
                        .values()
                        .filter(|p| p.indoor_plot_id.is_none())
                        .map(|p| crate::player_cheer::Candidate {
                            id: p.id,
                            zone: p.planet.clone(),
                            x: p.x,
                            y: p.y,
                        })
                        .collect();
                    // 當下還在喝采、且在室外的喝采者（依 id 排序求確定）。
                    let mut givers: Vec<uuid::Uuid> = players
                        .values()
                        .filter(|p| p.cheer_offer > 0 && p.indoor_plot_id.is_none())
                        .map(|p| p.id)
                        .collect();
                    givers.sort();

                    // 第一階段（只讀）：替每位喝采者挑對象＋驗冷卻，算出本幀成立的喝采。
                    // (giver_id, giver_name, target_id, gx, gy, old_cheers)
                    let mut decisions: Vec<(uuid::Uuid, String, uuid::Uuid, f32, f32, u64)> = Vec::new();
                    let mut matched: std::collections::HashSet<uuid::Uuid> = std::collections::HashSet::new();
                    for gid in &givers {
                        let Some(g) = players.get(gid) else { continue };
                        let Some(tid) = crate::player_cheer::pick_target(
                            g.id, &g.planet, g.x, g.y, &candidates,
                        ) else { continue };
                        if !crate::player_cheer::can_cheer(&g.cheer_cooldowns, tid) {
                            continue; // 對象還在冷卻 → 這幀不計、意願留待下幀（或自然淡掉）。
                        }
                        let old = players.get(&tid).map(|t| t.cheers).unwrap_or(0);
                        decisions.push((g.id, g.name.clone(), tid, g.x, g.y, old));
                        matched.insert(g.id);
                    }

                    // 第二階段（套用）：逐 id 各別 get_mut（不同時持兩個可變借用）。
                    let mut evs = Vec::with_capacity(decisions.len());
                    for (gid, gname, tid, gx, gy, old) in decisions {
                        // 替對方人氣 +1，取回最新值與座標、名字。
                        let (new_cheers, tname, tx_, ty_) = match players.get_mut(&tid) {
                            Some(t) => {
                                t.cheers = t.cheers.saturating_add(1);
                                (t.cheers, t.name.clone(), t.x, t.y)
                            }
                            None => continue,
                        };
                        // 喝采者：寫該對象冷卻、清掉意願。
                        if let Some(g) = players.get_mut(&gid) {
                            g.cheer_cooldowns.insert(tid, crate::player_cheer::CHEER_COOLDOWN);
                            g.cheer_offer = 0;
                        }
                        // 人氣跨階（由無→有或更高）時世界頻道報一聲（出鎖後送）。
                        let before = crate::player_cheer::popularity_for(old).map(|t| t.threshold);
                        let after = crate::player_cheer::popularity_for(new_cheers);
                        if let Some(t) = after {
                            if before != Some(t.threshold) {
                                tier_msgs.push(format!(
                                    "🎉 {} 在眾人的喝采中成為了「{} {}」！",
                                    tname, t.badge, t.title
                                ));
                            }
                        }
                        let mx = (gx + tx_) * 0.5;
                        let my = (gy + ty_) * 0.5;
                        evs.push((gname, tid, tname, new_cheers, mx, my));
                    }

                    // 沒挑到對象的喝采者意願遞減（留待下幀再試或淡掉）；同時每幀推進所有冷卻表。
                    for p in players.values_mut() {
                        if p.cheer_offer > 0 && !matched.contains(&p.id) {
                            p.cheer_offer -= 1;
                        }
                        crate::player_cheer::tick_cooldowns(&mut p.cheer_cooldowns);
                    }
                    cheered = evs;
                }
                for (giver_name, target_id, target_name, target_cheers, mx, my) in cheered {
                    let _ = app.tx.send(std::sync::Arc::new(
                        crate::protocol::ServerMsg::Cheered {
                            giver_name,
                            target_id,
                            target_name,
                            target_cheers,
                            mx,
                            my,
                            display_secs: crate::player_cheer::CHEER_DISPLAY_SECS,
                        },
                    ));
                }
                // 人氣跨階同慶（稀有：每位玩家每階一生一次，不會洗頻）。
                for msg in tier_msgs {
                    let _ = app.tx_chat.send(msg);
                }
            }

            // ── ROADMAP 342 人氣聚會：高人氣玩家身邊聚起人潮 → 腳下湧現發光聚會圈＋世界宣告 ──
            // 341 把人氣沉澱成名牌身份；342 讓那份人氣長出後果——「受歡迎」（≥10 人氣）的玩家會真的
            // 吸引人潮、在他周圍聚成一個看得見的社交節點。每幀：把室外玩家（同星球才湊）餵進
            // popularity_gathering::detect 找出當下成局的聚會（純函式、確定可重現），再經 reconcile
            // 純狀態機（含散場緩衝防閃爍）對映成 Started/Ended 事件，**出鎖後**才取主人名字＋廣播給前端
            // 畫聚會圈／淡出。讀鎖只開來蒐快照、隨即釋放後再動 gathering 狀態與 tx（守 prod-deadlock
            // 鐵律：無巢狀上鎖）；多數時刻沒人攢到人氣門檻→detect 第一輪就跳光、近乎零成本。
            {
                let attendees: Vec<crate::popularity_gathering::Attendee> = {
                    let players = app.players.read().unwrap();
                    players
                        .values()
                        .filter(|p| p.indoor_plot_id.is_none())
                        .map(|p| crate::popularity_gathering::Attendee {
                            id: p.id,
                            zone: p.planet.clone(),
                            x: p.x,
                            y: p.y,
                            cheers: p.cheers,
                        })
                        .collect()
                };
                let parties = crate::popularity_gathering::detect(&attendees);
                let events = app.popularity_gathering.write().unwrap().reconcile(&parties);
                if !events.is_empty() {
                    use crate::popularity_gathering::GatheringEvent;
                    // 只在有事件時才回查一次主人名字（讀鎖快照、隨即釋放）。
                    let names: std::collections::HashMap<uuid::Uuid, String> = {
                        let players = app.players.read().unwrap();
                        players.values().map(|p| (p.id, p.name.clone())).collect()
                    };
                    for ev in events {
                        match ev {
                            GatheringEvent::Started { host, guests } => {
                                let host_name = names.get(&host).cloned().unwrap_or_default();
                                let _ = app.tx.send(std::sync::Arc::new(
                                    crate::protocol::ServerMsg::PopGatheringStarted {
                                        host_id: host,
                                        host_name,
                                        guests,
                                    },
                                ));
                            }
                            GatheringEvent::Ended { host } => {
                                let _ = app.tx.send(std::sync::Arc::new(
                                    crate::protocol::ServerMsg::PopGatheringEnded { host_id: host },
                                ));
                            }
                        }
                    }
                }
            }

            // ── ROADMAP 177: 野外採集隊觸發與受擊判定 ──
            if tick % (TICK_HZ as u64) == 0 {
                // 1. 觸發判定
                let start_ev = {
                    let mut res = app.residents.write().unwrap();
                    let colonies = app.monster_colonies.read().unwrap();
                    let invasion = app.invasion.read().unwrap();
                    if res.expedition_cooldown <= 0.0 && !invasion.active && res.prosperity_level() >= 3 {
                        if let Some(target) = colonies.colonies.iter().max_by_key(|c| c.population) {
                            res.expedition_cooldown = 2400.0; // 40 分鐘冷卻
                            res.start_expedition(target.name.to_string(), target.cx, target.cy)
                        } else { None }
                    } else { None }
                };
                if let Some(ev) = start_ev {
                    use crate::resident_npc::ResidentLifecycleEvent;
                    if let ResidentLifecycleEvent::ExpeditionStarted { ref msg, .. } = ev {
                        let _ = app.tx_chat.send(msg.clone());
                    }
                }

                // 2. 受擊判定（比照玩家反擊邏輯，每秒一次）
                let mut npc_dmgs: Vec<(String, u32)> = Vec::new();
                {
                    let res = app.residents.read().unwrap();
                    let enemies = app.enemies.read().unwrap();
                    for r in &res.residents {
                        if r.expedition.is_some() {
                            let threat = enemies.threat_at(r.x, r.y);
                            if threat > 0 {
                                npc_dmgs.push((r.id.clone(), threat));
                            }
                        }
                    }
                }
                if !npc_dmgs.is_empty() {
                    let mut res = app.residents.write().unwrap();
                    for (rid, dmg) in npc_dmgs {
                        if let Some(r) = res.residents.iter_mut().find(|r| r.id == rid) {
                            r.hp -= dmg as f32;
                        }
                    }
                }

                // 3. 護送津貼判定（ROADMAP 177，每 10 秒一次）
                if tick % (10 * TICK_HZ as u64) == 0 {
                    let mut rewarded_players = Vec::new();
                    {
                        let res = app.residents.read().unwrap();
                        let players = app.players.read().unwrap();
                        let exp_npcs: Vec<_> = res.residents.iter()
                            .filter(|r| r.expedition.is_some())
                            .map(|r| (r.x, r.y))
                            .collect();
                        
                        if !exp_npcs.is_empty() {
                            for p in players.values() {
                                if p.vitals.is_downed() { continue; }
                                for (nx, ny) in &exp_npcs {
                                    let dx = p.x - nx;
                                    let dy = p.y - ny;
                                    if (dx * dx + dy * dy).sqrt() < 100.0 {
                                        rewarded_players.push(p.id);
                                        break; // 同一玩家一次只得一份
                                    }
                                }
                            }
                        }
                    }
                    if !rewarded_players.is_empty() {
                        let mut players = app.players.write().unwrap();
                        for pid in rewarded_players {
                            if let Some(p) = players.get_mut(&pid) {
                                p.ether += 5;
                            }
                        }
                    }
                }
            }

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
                // ROADMAP 469：本拍受擊、且身旁有乙太迷霧／孢子系敵人的玩家，待注入中毒 (pid, 秒數)。
                let mut poison_apply: Vec<(uuid::Uuid, f32)> = Vec::new();
                // ROADMAP 424：本拍剛砸下重擊的怪物王落點（出鎖後廣播衝擊波）。
                let mut slam_events: Vec<(f32, f32, f32)> = Vec::new();
                {
                    let enemies = app.enemies.read().unwrap();
                    for (pid, px, py, downed) in &positions {
                        if *downed { continue; }
                        let threat = enemies.threat_at(*px, *py);
                        if threat > 0 {
                            dmgs.push((*pid, threat));
                            // ROADMAP 469：受到反擊傷害的同時，若身旁有帶毒敵人就注入中毒。
                            let psecs = enemies.poison_secs_at(*px, *py);
                            if psecs > 0.0 {
                                poison_apply.push((*pid, psecs));
                            }
                        }
                    }
                    // ROADMAP 424：怪物王預警重擊——本傷害結算每秒一次，故以 (now-1s, now) 偵測
                    // 「上一秒到這一秒之間蓄滿砸下」的怪物王；命中圈內未倒地玩家加一發爆發傷害，
                    // 該傷害與一般威脅共用下方減傷鏈（護甲→格擋→翻滾），格擋／翻滾可完全化解。
                    let now_secs = tick as f64 / TICK_HZ as f64;
                    let prev_secs = now_secs - 1.0;
                    for (id, bx, by, level) in enemies.notorious_slammers() {
                        if !crate::boss_slam::just_struck(id, prev_secs, now_secs) {
                            continue;
                        }
                        slam_events.push((bx, by, crate::boss_slam::SLAM_RADIUS));
                        let dmg = crate::boss_slam::slam_damage(level);
                        for (pid, px, py, downed) in &positions {
                            if *downed { continue; }
                            if crate::boss_slam::is_in_blast(bx, by, *px, *py) {
                                dmgs.push((*pid, dmg));
                            }
                        }
                    }
                }
                // 位置→玩家的對映，供後續步驟查找倒地玩家座標
                let mut pos_map: std::collections::HashMap<uuid::Uuid, (f32, f32)> = positions
                    .iter().map(|(id, x, y, _)| (*id, (*x, *y))).collect();
                let mut downed_positions: Vec<(f32, f32)> = Vec::new();
                // ROADMAP 83：同步收集倒地玩家名稱，供 NPC 落敗反應使用。
                let mut newly_downed_names: Vec<String> = Vec::new();
                // ROADMAP 410：本拍翻滾閃掉反擊的玩家（出鎖後廣播「閃避！」飄字）。
                let mut evaded_dodges: Vec<(uuid::Uuid, f32, f32)> = Vec::new();
                {
                    let mut players = app.players.write().unwrap();
                    // ROADMAP 469：先注入本拍的中毒（受擊且身旁有帶毒敵人）。
                    for (pid, secs) in poison_apply {
                        if let Some(p) = players.get_mut(&pid) {
                            if !p.vitals.is_downed() {
                                p.poison.apply(secs);
                            }
                        }
                    }
                    for (pid, dmg) in dmgs {
                        if let Some(p) = players.get_mut(&pid) {
                            // 護甲減傷：讀裝備槽（ROADMAP 36）+ 寵物加成（ROADMAP 46）。
                            let defense = crate::equipment::equipped_armor_defense(&p.equipment)
                                + p.pet.map(|pk| pk.bonus_defense()).unwrap_or(0);
                            let mut actual_dmg = dmg.saturating_sub(defense);
                            // 臨陣格擋（ROADMAP 408）：若此刻有乙太護盾，卸掉一部分反擊傷害
                            //（封頂 85%、永不完全免傷）。護盾在主迴圈每 tick 遞減、消散即清空。
                            if let Some(shield) = p.guard_shield {
                                actual_dmg = shield.reduce(actual_dmg);
                            }
                            // 翻滾閃避（ROADMAP 410）：翻滾的恩典窗內完全閃掉這次反擊（零傷）。
                            // 與護盾正交——先卸傷再判閃避，閃中即歸零；只在真有傷可閃時記事件。
                            if actual_dmg > 0 {
                                if let Some(d) = p.dodging {
                                    if d.in_grace() {
                                        actual_dmg = 0;
                                        evaded_dodges.push((p.id, p.x, p.y));
                                    }
                                }
                            }
                            if actual_dmg > 0 && p.vitals.take_damage(actual_dmg) {
                                tracing::info!(player = %p.name, defense, actual_dmg, "被敵人打趴，休息復原中");
                                if let Some(&(px, py)) = pos_map.get(&pid) {
                                    downed_positions.push((px, py));
                                }
                                newly_downed_names.push(p.name.clone());
                            }
                        }
                    }
                    // ROADMAP 469：中毒結算——對所有中毒玩家推進 1 秒（本分支每秒一次），毒傷穿透
                    // 護甲／格擋／翻滾直接流失（體內毒、減傷鏈擋不住），城鎮安全圈內加速代謝解毒。
                    // 走出敵人範圍仍會繼續掉血，逼玩家撤離 / 回鎮——這是中毒帶來的新空間決策。
                    // 毒傷打趴也算被怪擊倒（最近敵人升級、NPC 落敗反應走同一條路徑）。
                    for p in players.values_mut() {
                        if p.vitals.is_downed() || !p.poison.is_active() {
                            continue;
                        }
                        let in_town = crate::affliction::near_town_cleanse(p.x, p.y);
                        let pdmg = p.poison.tick(1.0, in_town);
                        if pdmg > 0 && p.vitals.take_damage(pdmg) {
                            tracing::info!(player = %p.name, pdmg, "中毒流盡，休息復原中");
                            downed_positions.push((p.x, p.y));
                            newly_downed_names.push(p.name.clone());
                        }
                    }
                }
                // 翻滾閃避成功廣播（ROADMAP 410）：出鎖後送，守 prod-deadlock 鐵律；
                // 前端只對 player_id == 自己 演出「閃避！」飄字。
                for (pid, ex, ey) in evaded_dodges {
                    let _ = app.tx.send(std::sync::Arc::new(
                        crate::protocol::ServerMsg::DodgeEvaded { player_id: pid, x: ex, y: ey }
                    ));
                }
                // 怪物王重擊衝擊波廣播（ROADMAP 424）：出鎖後送，守 prod-deadlock 鐵律；
                // 全服都看得到落點的衝擊波環（無論自己是否在圈內），傷害已於上方結算。
                for (bx, by, radius) in slam_events {
                    let _ = app.tx.send(std::sync::Arc::new(
                        crate::protocol::ServerMsg::BossSlam { x: bx, y: by, radius }
                    ));
                }
                // 玩家倒地 → 最近敵人升一級（ROADMAP 42）。
                // 分開持 enemies 寫鎖，避免與上方 players 寫鎖同時持有。
                if !downed_positions.is_empty() {
                    let mut newly_notorious: Vec<crate::enemy_field::EnemyLevelUpResult> = Vec::new();
                    let mut killer_kinds: Vec<crate::combat::EnemyKind> = Vec::new();
                    {
                        let mut enemies = app.enemies.write().unwrap();
                        for (px, py) in downed_positions {
                            if let Some(r) = enemies.level_up_nearest_killer(px, py) {
                                killer_kinds.push(r.kind);
                                if r.newly_notorious {
                                    newly_notorious.push(r);
                                }
                            }
                        }
                    }
                    // ROADMAP 163：怪物擊倒玩家 → 對應物種態度下降（更囂張）。
                    // 事件由 tick() 統一廣播，這裡只做態度調整。
                    if !killer_kinds.is_empty() {
                        let mut ms = app.monster_species.write().unwrap();
                        for k in killer_kinds {
                            ms.on_monster_kills_player(k);
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
            // 釣魚脫鉤（ROADMAP 346）：等太久沒收竿的玩家，出鎖後廣播「魚跑了」。
            let mut fish_escapes: Vec<(uuid::Uuid, f32, f32)> = Vec::new();
            // 打坐事件（ROADMAP 391）：完成或中斷，出鎖後廣播並給獎勵。
            // (player_id, x, y, kind) — kind: true=完成, false=移動中斷
            let mut meditation_events: Vec<(uuid::Uuid, f32, f32, bool, u32, u32)> = Vec::new();
            // 廣場獻奏事件（ROADMAP 399）：完成或中斷，出鎖後廣播並給打賞。
            // 完成：(player_id, ether_gained, listeners, busk_count)；中斷另記。
            let mut busk_completes: Vec<(uuid::Uuid, u32, u32, u32)> = Vec::new();
            let mut busk_aborts: Vec<uuid::Uuid> = Vec::new();
            // 在地地名變更（ROADMAP 398 天地有名）：踏入新 locale，出鎖後廣播地名卡。
            // (player_id, name, subtitle, initial) — initial=true 為進場首次定位（前端不彈大卡）。
            // (pid, name, subtitle, initial, first_footfall, tally, xp_reward) — ROADMAP 398＋411。
            let mut locale_changes: Vec<(
                uuid::Uuid,
                &'static str,
                &'static str,
                bool,
                bool,
                u32,
                u32,
            )> = Vec::new();
            // 林蔭小憩（ROADMAP 467）：先取一份「成樹座標」快照——grove 讀鎖即取即放（回傳擁有的
            // Vec、不綁變數，敘述句結束即釋放讀鎖），故下方 players 寫鎖期間不與 grove 鎖巢狀（守
            // prod-deadlock 鐵律）。供玩家迴圈判定誰正站在社群種大的樹蔭下、加速其脫戰回血。
            let shade_trees: Vec<(f32, f32)> =
                app.world_grove.read().unwrap().mature_positions();
            {
                let mut players = app.players.write().unwrap();
                // 寵物玩伴嬉戲（ROADMAP 344）：先讀一遍「有寵物、在室外」的玩家位置，偵測寵物玩伴
                // 配對——兩名各有寵物、同星球、站得夠近的玩家，他們的寵物會自己湊到中間玩耍。
                // 純函式偵測（pet_play::detect），這裡只建「主人 id → 玩耍點」索引，供下方 tick
                // 迴圈據此決定該隻寵物是「跑去玩耍」還是「跟著主人」。（不可變借用先結束、再進可變迴圈。）
                let pet_play_targets: std::collections::HashMap<uuid::Uuid, (f32, f32)> = {
                    let actors: Vec<crate::pet_play::PetActor> = players
                        .values()
                        .filter(|p| p.pet.is_some() && p.indoor_plot_id.is_none())
                        .map(|p| crate::pet_play::PetActor {
                            owner_id: p.id,
                            zone: p.planet.clone(),
                            owner_x: p.x,
                            owner_y: p.y,
                        })
                        .collect();
                    let mut m = std::collections::HashMap::new();
                    for pair in crate::pet_play::detect(&actors) {
                        m.insert(pair.a, pair.spot_a);
                        m.insert(pair.b, pair.spot_b);
                    }
                    m
                };
                // 廣場獻奏聽眾快照（ROADMAP 399）：在進入可變迴圈前，先記下所有在線、未倒地玩家的
                // (id, 星球, 座標)，供下方獻奏「完成」時數算身旁聆賞的鄰近玩家人數（不在可變借用中
                // 重新走訪 map、不巢狀上鎖）。位置一 tick 內幾乎不動，用幀初快照足夠。
                let busk_listener_snap: Vec<(uuid::Uuid, String, f32, f32)> = players
                    .values()
                    .filter(|p| !p.vitals.is_downed())
                    .map(|p| (p.id, p.planet.clone(), p.x, p.y))
                    .collect();
                for p in players.values_mut() {
                    p.step(dt, |x: f32, y: f32| {
                        let (cx, cy, tx, ty) = crate::tiles::world_to_cell(x, y);
                        let kind = tile_deltas_snap
                            .get(&(cx, cy, tx, ty))
                            .copied()
                            .unwrap_or_else(|| world_core::tile_kind_at(x as f64, y as f64));
                        kind != world_core::TileKind::Empty
                    });
                    // 寵物現身相伴（ROADMAP 343）＋寵物玩伴嬉戲（ROADMAP 344）：有寵物時，每 tick
                    // 推進寵物座標（純函式、零鎖、無 IO）——附近有寵物玩伴（在 pet_play_targets 裡）
                    // 就跑去兩人中間的玩耍點蹦跳玩耍，否則回復跟隨主人。
                    if p.pet.is_some() {
                        // 寵物逗玩接物（ROADMAP 345）優先：玩家丟出玩具後，寵物先衝去叼、再叼回主人，
                        // 期間不跟隨也不玩耍。`PetFetch` 是 Copy，先複製出來再改 p 各欄位，避開借用衝突。
                        // 接物途中若主人瞬移／換星球（寵物離主人超過 ABORT_DIST）就放棄這趟，讓跟隨接手。
                        if let Some(f) = p.pet_fetch {
                            let far = (p.pet_x - p.x).hypot(p.pet_y - p.y) > crate::pet_fetch::ABORT_DIST;
                            if far {
                                p.pet_fetch = None;
                            } else {
                                match f.phase {
                                    crate::pet_fetch::FetchPhase::Chasing => {
                                        // 衝向玩具落點；叼到（進 GRAB_REACH）就轉入叼回階段。
                                        let (nx, ny, got) = crate::pet_fetch::chase_step(
                                            p.pet_x, p.pet_y, f.toy_x, f.toy_y, dt,
                                            crate::pet_fetch::FETCH_SPEED,
                                            crate::pet_fetch::GRAB_REACH,
                                        );
                                        p.pet_x = nx;
                                        p.pet_y = ny;
                                        p.pet_fetch = Some(crate::pet_fetch::PetFetch {
                                            phase: if got {
                                                crate::pet_fetch::FetchPhase::Returning
                                            } else {
                                                crate::pet_fetch::FetchPhase::Chasing
                                            },
                                            ..f
                                        });
                                    }
                                    crate::pet_fetch::FetchPhase::Returning => {
                                        // 叼著玩具跑回主人；回到腳邊（進 RETURN_REACH）即交差、接物結束。
                                        let (nx, ny, back) = crate::pet_fetch::chase_step(
                                            p.pet_x, p.pet_y, p.x, p.y, dt,
                                            crate::pet_fetch::FETCH_SPEED,
                                            crate::pet_fetch::RETURN_REACH,
                                        );
                                        p.pet_x = nx;
                                        p.pet_y = ny;
                                        p.pet_fetch = if back {
                                            None
                                        } else {
                                            // 玩具被叼著走，每 tick 跟到寵物身上（前端據此畫被叼的玩具）。
                                            Some(crate::pet_fetch::PetFetch { toy_x: nx, toy_y: ny, ..f })
                                        };
                                    }
                                }
                            }
                            p.pet_playing = false;
                            p.pet_fetching = p.pet_fetch.is_some();
                        } else if let Some(&spot) = pet_play_targets.get(&p.id) {
                            let (nx, ny, _moving) =
                                crate::pet_play::play_step((p.pet_x, p.pet_y), spot, dt);
                            p.pet_x = nx;
                            p.pet_y = ny;
                            p.pet_playing = true;
                            p.pet_fetching = false;
                        } else {
                            // 寵物性格（ROADMAP 358）：歇腳距離隨性格而異（黏人貼最近、慵懶／好奇
                            // 愛在後頭）。性格由「主人帳號 ＋ 寵物種類」確定性算出（純函式、零鎖、無 IO）。
                            let stop = p
                                .pet
                                .map(|k| {
                                    crate::pet_personality::personality_for(p.id.as_bytes(), k)
                                        .follow_stop()
                                })
                                .unwrap_or(crate::pet_follow::FOLLOW_STOP);
                            let (nx, ny, _moving) = crate::pet_follow::follow_step_with_stop(
                                (p.pet_x, p.pet_y),
                                (p.x, p.y),
                                dt,
                                stop,
                            );
                            p.pet_x = nx;
                            p.pet_y = ny;
                            p.pet_playing = false;
                            p.pet_fetching = false;
                        }
                    }
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
                    // 採礦冷卻倒數（ROADMAP 348）：一輪礦脈結束後起算，只擋開新礦脈。
                    if p.mine_cooldown > 0.0 {
                        p.mine_cooldown = (p.mine_cooldown - dt).max(0.0);
                    }
                    // 開灶冷卻倒數（ROADMAP 349）：開灶起算，只擋開新一趟掌勺。
                    if p.cook_cooldown > 0.0 {
                        p.cook_cooldown = (p.cook_cooldown - dt).max(0.0);
                    }
                    // 伐木冷卻倒數（ROADMAP 403）：放倒樹後起算，只擋開新一趟連揮。
                    if p.chop_cooldown > 0.0 {
                        p.chop_cooldown = (p.chop_cooldown - dt).max(0.0);
                    }
                    // 格擋冷卻倒數（ROADMAP 408）：格擋結算後起算，只擋開新一趟格擋。
                    if p.guard_cooldown > 0.0 {
                        p.guard_cooldown = (p.guard_cooldown - dt).max(0.0);
                    }
                    // 天地有名（ROADMAP 398）：戶外玩家踏入新「在地地名」locale 即記錄，出鎖後廣播地名卡。
                    // 室內（自家屋內）不算進世界地名。locale_at 純函式、零鎖無 IO，確定性。
                    if p.indoor_plot_id.is_none() {
                        let loc = crate::region_name::locale_at(p.x as f64, p.y as f64);
                        if p.current_locale != Some(loc.id) {
                            // 首次定位（None）只靜默更新標、不彈大卡；真的換地方才彈卡＋報讀。
                            let initial = p.current_locale.is_none();
                            p.current_locale = Some(loc.id);
                            // 遠遊見聞（ROADMAP 411）：進場起點靜默記為已踏足、不慶賀；真的換到本趟
                            // 沒去過的新地方才是「初次踏足」（攢探索者 XP、增足跡計數）。
                            let (first_footfall, tally, xp_reward) = if initial {
                                p.wayfaring.mark_seen(loc.id);
                                (false, 0, 0)
                            } else {
                                match p.wayfaring.discover(loc.id) {
                                    crate::wayfaring::WayfareOutcome::FirstFootfall {
                                        tally,
                                        xp_reward,
                                    } => {
                                        if xp_reward > 0 {
                                            p.masteries.gain_explorer(xp_reward);
                                        }
                                        (true, tally, xp_reward)
                                    }
                                    crate::wayfaring::WayfareOutcome::Known => (false, 0, 0),
                                }
                            };
                            locale_changes.push((
                                p.id,
                                loc.name,
                                loc.subtitle,
                                initial,
                                first_footfall,
                                tally,
                                xp_reward,
                            ));
                        }
                    }
                    // 釣魚上鉤小遊戲推進（ROADMAP 346）：等咬鉤→咬鉤→脫鉤。
                    // advance 純函式、零鎖無 IO；JustBit 只讓 phase 轉 Biting（隨快照廣播、
                    // 前端抖浮標），Escaped 才清狀態、出鎖後廣播「魚跑了」（守 prod-deadlock）。
                    if let Some(cast) = p.fishing.as_mut() {
                        if cast.advance(dt) == crate::fishing_bite::BiteEvent::Escaped {
                            p.fishing = None;
                            fish_escapes.push((p.id, p.x, p.y));
                        }
                    }
                    // 夜泉汲取推進（ROADMAP 350 汲泉聚精）：advance 累時間、逾時即中斷這趟
                    // （泉眼留著、可重試）。純函式、零鎖無 IO；準星位置由前端用同一公式渲染。
                    if let Some(d) = p.aether_draw.as_mut() {
                        if d.advance(dt) {
                            p.aether_draw = None;
                        }
                    }
                    // 伐木連揮推進（ROADMAP 403 林間揮斧）：advance 累時間、逾時即中斷這趟
                    // （樹留著、可重來）。純函式、零鎖無 IO；節拍由前端用同一公式渲染。
                    if let Some(c) = p.chopping.as_mut() {
                        if c.advance(dt) {
                            p.chopping = None;
                        }
                    }
                    // 格擋備防推進（ROADMAP 408 臨陣格擋）：advance 累時間、逾時即解除這趟
                    // （不罰冷卻、可重來）。純函式、零鎖無 IO；格擋環由前端用同一公式渲染。
                    if let Some(g) = p.guarding.as_mut() {
                        if g.advance(dt) {
                            p.guarding = None;
                        }
                    }
                    // 乙太護盾消散（ROADMAP 408）：advance 遞減剩餘秒數、歸零即清空。
                    // 反擊迴圈（每秒一次）讀它卸掉反擊傷害。
                    if let Some(s) = p.guard_shield.as_mut() {
                        if s.advance(dt) {
                            p.guard_shield = None;
                        }
                    }
                    // 翻滾冷卻倒數（ROADMAP 410）：翻滾結算後起算，只擋開新一趟翻滾。
                    if p.dodge_cooldown > 0.0 {
                        p.dodge_cooldown = (p.dodge_cooldown - dt).max(0.0);
                    }
                    // 翻滾推進（ROADMAP 410 翻滾閃避）：advance 累時間、恩典窗過即落幕這趟。
                    // 純函式、零鎖無 IO；翻身位移由前端用 elapsed 演出；反擊迴圈讀 in_grace 判免傷。
                    if let Some(d) = p.dodging.as_mut() {
                        if d.advance(dt) {
                            p.dodging = None;
                        }
                    }
                    // 蓄力冷卻倒數（ROADMAP 423 蓄力重擊）：放開後起算，只擋開新一趟蓄力。
                    if p.charge_cooldown > 0.0 {
                        p.charge_cooldown = (p.charge_cooldown - dt).max(0.0);
                    }
                    // 蓄力推進（ROADMAP 423）：advance 累時間並夾在滿蓄上限，不自行結束——
                    // 蓄力靠玩家放開（ReleaseCharge）結算。純函式、零鎖無 IO；蓄力環由前端用 progress 渲染。
                    if let Some(c) = p.charging.as_mut() {
                        c.advance(dt);
                    }
                    // 待擊重擊存活窗倒數（ROADMAP 423）：放開後限時存活，逾時未攻擊即消散。
                    if let Some(r) = p.charge_ready.as_mut() {
                        if r.advance(dt) {
                            p.charge_ready = None;
                        }
                    }
                    // 放風箏（ROADMAP 470）：倒地（休息復原中）時自動收線——躺平的人放不了風箏。
                    // 純暫態旗標、無 IO、無廣播（收線狀態隨既有快照自然傳出）。
                    if p.flying_kite && p.vitals.is_downed() {
                        p.flying_kite = false;
                    }
                    // 安靜打坐推進（ROADMAP 391）：每 tick 檢查移動中斷或完成；出鎖後給獎勵並廣播。
                    if let Some(m) = p.meditation {
                        let now = std::time::Instant::now();
                        if m.is_interrupted(p.x, p.y) {
                            p.meditation = None;
                            meditation_events.push((p.id, p.x, p.y, false, 0, 0));
                        } else if m.is_complete(now) {
                            p.meditation = None;
                            p.last_meditate = Some(now);
                            let hp_healed = crate::meditation::hp_heal(
                                p.vitals.max_hp(),
                                crate::meditation::MEDITATE_HP_PCT,
                            );
                            let actual_hp = p.vitals.heal(hp_healed);
                            let ether = crate::meditation::MEDITATE_ETHER;
                            p.ether = p.ether.saturating_add(ether);
                            meditation_events.push((p.id, p.x, p.y, true, ether, actual_hp));
                        }
                    }
                    // 廣場獻奏推進（ROADMAP 399）：每 tick 檢查移動中斷或完成；完成時依身旁聆賞的
                    // 鄰近玩家人數計打賞乙太、累積資歷，出鎖後廣播。鎖序與打坐相同（鎖內結算、鎖外廣播）。
                    if let Some(b) = p.busking {
                        let now = std::time::Instant::now();
                        if b.is_interrupted(p.x, p.y) {
                            p.busking = None;
                            busk_aborts.push(p.id);
                        } else if b.is_complete(now) {
                            p.busking = None;
                            p.last_busk = Some(now);
                            // 數算聆賞者：同星球、未倒地、在半徑內的其他在線玩家（排除自己）。
                            let listeners = busk_listener_snap
                                .iter()
                                .filter(|(oid, oplanet, ox, oy)| {
                                    *oid != p.id
                                        && *oplanet == p.planet
                                        && crate::busking::within_listen_range(p.x, p.y, *ox, *oy)
                                })
                                .count() as u32;
                            let ether = crate::busking::tip_ether(listeners);
                            p.ether = p.ether.saturating_add(ether);
                            p.busk_count = p.busk_count.saturating_add(1);
                            busk_completes.push((p.id, ether, listeners, p.busk_count));
                        }
                    }
                    // 暖食飽足回復（ROADMAP 395）：吃料理後一段時間 HP 緩慢回復、過期自動清。
                    // 先在 meal_buff 借用內取出本幀回血量與是否續存，借用結束後再動 vitals／清欄位，
                    // 避免同時可變借用 p 的兩個欄位。
                    let meal_step = p.meal_buff.as_mut().map(|b| (b.tick(dt), b.is_active()));
                    if let Some((healed, active)) = meal_step {
                        if healed > 0 {
                            p.vitals.heal(healed); // 倒地時 heal 自動 no-op，飽足不會把趴著的人拉起
                        }
                        if !active {
                            p.meal_buff = None;
                        }
                    }
                    // 席間舉杯冷卻倒數（ROADMAP 329）。
                    if p.toast_cooldown > 0.0 {
                        p.toast_cooldown = (p.toast_cooldown - dt).max(0.0);
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
                    // 水族缸靜心回血（ROADMAP 437）：每 25 秒，在室內且擁有水族缸的玩家，
                    // 依背包養著的不同魚種數回血（每種魚 +1，最多 +3）；空缸只是裝飾、不回血。
                    // 這給「把釣到的魚留著養」一個療癒回報，與 436「煮好的菜賣乙太」形成取捨。
                    let aqua_interval =
                        crate::home_furniture::AQUARIUM_REGEN_INTERVAL_SECS as u64 * TICK_HZ as u64;
                    if tick % aqua_interval == 0 && p.indoor_plot_id.is_some() && !p.vitals.is_downed() {
                        let has_aqua = app.home_furnishings.read().unwrap()
                            .get(&p.id).map(|h| h.has_aquarium()).unwrap_or(false);
                        if has_aqua {
                            let species = [
                                crate::inventory::ItemKind::FishSmall,
                                crate::inventory::ItemKind::FishStar,
                                crate::inventory::ItemKind::FishDeep,
                            ]
                            .iter()
                            .filter(|f| p.inventory.count(**f) > 0)
                            .count() as u32;
                            let hp = crate::home_furniture::aquarium_regen_hp(species);
                            if hp > 0 {
                                p.vitals.heal(hp);
                            }
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
                    // 林蔭小憩（ROADMAP 467）：站在社群親手種大的成樹樹蔭下（室外）、脫離戰鬥時
                    // 回血更快。`shade_regen` 內部已自守倒地／剛挨打／滿血一律 no-op，這裡只先濾掉
                    // 室內與「世界上根本沒有成樹」兩種無謂呼叫。
                    if !shade_trees.is_empty()
                        && p.indoor_plot_id.is_none()
                        && crate::world_grove::in_shade(p.x, p.y, &shade_trees)
                    {
                        p.vitals.shade_regen(dt);
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

            // 釣魚脫鉤廣播（ROADMAP 346）：鎖已釋放，安全廣播「魚跑了」（前端只對自己 id 飄字）。
            for (pid, x, y) in fish_escapes {
                let _ = app.tx.send(std::sync::Arc::new(ServerMsg::FishResult {
                    player_id: pid,
                    outcome: "escaped".into(),
                    fish: None,
                    quality: None,
                    in_season: None,
                    size_cm: None,
                    personal_best: None,
                    prev_best_cm: None,
                    x,
                    y,
                }));
            }

            // 安靜打坐廣播（ROADMAP 391）：鎖已釋放，安全廣播完成或中斷事件。
            for (pid, px, py, completed, ether, hp_healed) in meditation_events {
                if completed {
                    let _ = app.tx.send(std::sync::Arc::new(
                        crate::protocol::ServerMsg::MeditationComplete {
                            player_id: pid,
                            ether_gained: ether,
                            hp_healed,
                        }
                    ));
                } else {
                    let _ = app.tx.send(std::sync::Arc::new(
                        crate::protocol::ServerMsg::MeditationAborted { player_id: pid }
                    ));
                }
                let _ = (px, py); // 座標備而不用，前端從快照讀取
            }

            // 廣場獻奏廣播（ROADMAP 399）：鎖已釋放，安全廣播完成或中斷事件。
            for (pid, ether, listeners, busk_count) in busk_completes {
                let _ = app.tx.send(std::sync::Arc::new(
                    crate::protocol::ServerMsg::BuskComplete {
                        player_id: pid,
                        ether_gained: ether,
                        listeners,
                        busk_count,
                    }
                ));
            }
            for pid in busk_aborts {
                let _ = app.tx.send(std::sync::Arc::new(
                    crate::protocol::ServerMsg::BuskAborted { player_id: pid }
                ));
            }

            // 天地有名廣播（ROADMAP 398）：鎖已釋放，安全廣播地名卡（前端只對自己 id 演出）。
            for (pid, name, subtitle, initial, first_footfall, tally, xp_reward) in locale_changes {
                let _ = app.tx.send(std::sync::Arc::new(
                    crate::protocol::ServerMsg::LocaleEntered {
                        player_id: pid,
                        name: name.to_string(),
                        subtitle: subtitle.to_string(),
                        initial,
                        first_footfall,
                        tally,
                        xp_reward,
                    },
                ));
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

            // AI 導演層＋獸潮攻城（ROADMAP 44 / 139 / 166 湧現化）：
            // 低頻導演 tick，獸潮從生態壓力長出（不再純計時器）。
            // 先把居民數＋生態壓力傳給導演，讓它依狀態決定是否觸發及波次規模。
            {
                let resident_count = app.residents.read().unwrap().population();
                let defense_drill = app.civic_vote.read().unwrap().defense_drill_active();

                // 計算生態壓力（ROADMAP 166）——純讀取，不加鎖競爭。
                let eco_pressure = {
                    use crate::eco_pressure::{compute_eco_pressure, ColonyPressureInput};
                    use crate::species_relations::{HOSTILE_THRESHOLD, WARY_THRESHOLD, ALL_MONSTER_KINDS};
                    use crate::wildlife::WildlifeKind;

                    // 巢穴族群飽和度
                    let colony_inputs: Vec<ColonyPressureInput> = app.monster_colonies.read().unwrap()
                        .colonies.iter()
                        .map(|c| ColonyPressureInput { population: c.population, max_population: c.max_population })
                        .collect();

                    // 怪物敵視/警覺計數
                    let ms = app.monster_species.read().unwrap();
                    let (hostile_count, wary_count) = ALL_MONSTER_KINDS.iter().fold((0u32, 0u32), |(h, w), &kind| {
                        let att = ms.attitude(kind);
                        if att < HOSTILE_THRESHOLD { (h + 1, w) }
                        else if att < WARY_THRESHOLD { (h, w + 1) }
                        else { (h, w) }
                    });
                    let monster_total = ALL_MONSTER_KINDS.len() as u32;
                    drop(ms);

                    // 野生獵物平均態度（WildBird/WildDeer/SmallCritter）
                    let prey_kinds = [WildlifeKind::WildBird, WildlifeKind::WildDeer, WildlifeKind::SmallCritter];
                    let sr = app.species_relations.read().unwrap();
                    let prey_avg = prey_kinds.iter().map(|&k| sr.attitude(k)).sum::<i32>() / prey_kinds.len() as i32;
                    drop(sr);

                    let raw = compute_eco_pressure(&colony_inputs, hostile_count, wary_count, monster_total, prey_avg);
                    // ROADMAP 174：跨族結盟期間額外生態壓力加成 +15
                    let alliance_bonus = if app.monster_colonies.read().unwrap().alliance_active() { 15.0 } else { 0.0 };
                    // ROADMAP 176：霸主巢穴存續期間額外生態壓力加成 +8
                    let dominant_bonus = app.monster_colonies.read().unwrap().dominant_pressure_bonus();
                    (raw + alliance_bonus + dominant_bonus).min(100.0)
                };

                // ROADMAP 172：生態清剿委託 tick——壓力超標時自動發布全服清剿任務。
                {
                    // 找族群最多的巢穴（busiest colony）傳給委託管理器。
                    let busiest = app.monster_colonies.read().unwrap()
                        .colonies.iter()
                        .max_by_key(|c| c.population)
                        .map(|c| (c.id, c.name.to_string(), c.population));

                    let event = app.eco_bounty.write().unwrap().tick(dt, eco_pressure, busiest);
                    if let Some(ev) = event {
                        use crate::eco_bounty::EcoBountyEvent;
                        match ev {
                            EcoBountyEvent::Started { colony_name, kill_target } => {
                                let _ = app.tx_chat.send(
                                    crate::eco_bounty::started_text(&colony_name, kill_target, crate::eco_bounty::REWARD_PER_PLAYER)
                                );
                            }
                            EcoBountyEvent::Completed { colony_name, reward_per_player } => {
                                // 給所有在線玩家乙太獎勵。
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
                            EcoBountyEvent::Expired { colony_name } => {
                                let _ = app.tx_chat.send(
                                    crate::eco_bounty::expired_text(&colony_name)
                                );
                            }
                        }
                    }
                }

                // ROADMAP 178：生態豐收節 tick——壓力曾衝上危機後被壓回安寧時，全城自動慶祝。
                {
                    let invasion_active = app.invasion.read().unwrap().active;
                    let event = app.eco_festival.write().unwrap().tick(dt, eco_pressure, invasion_active);
                    if let Some(ev) = event {
                        use crate::eco_festival::EcoFestivalEvent;
                        match ev {
                            EcoFestivalEvent::Started { reward_per_player } => {
                                // 開節即時發給所有在線玩家乙太。
                                {
                                    let mut players = app.players.write().unwrap();
                                    for p in players.values_mut() {
                                        p.ether = p.ether.saturating_add(reward_per_player);
                                    }
                                }
                                let _ = app.tx_chat.send(
                                    crate::eco_festival::started_text(reward_per_player)
                                );
                            }
                            EcoFestivalEvent::Ended => {
                                let _ = app.tx_chat.send(crate::eco_festival::ended_text());
                            }
                        }
                    }
                }

                let cmds = {
                    let mut director = app.director.write().unwrap();
                    director.update_population(resident_count);
                    director.update_eco_pressure(eco_pressure);
                    director.tick(dt)
                };
                for cmd in cmds {
                    match cmd {
                        crate::director::DirectorCmd::AnnounceHorde { site_x, site_y, site_label, wave, eco_pressure } => {
                            // 城防演練（ROADMAP 156）：城防演練進行中，跳過怪物注入。
                            if defense_drill {
                                let _ = app.tx_chat.send(
                                    "🛡️ 城防演練進行中，獸潮被警戒陣形阻擋，暫時撤退！".to_string()
                                );
                                continue;
                            }
                            // 全域軟上限：世界已夠擠時不再往裡塞獸潮怪（人多才容許更多）。
                            // 先取在線存活人數快照，再鎖 enemies（鎖序：players 不在 enemies 寫鎖內二次上鎖）。
                            let alive_players = {
                                let players = app.players.read().unwrap();
                                players.values().filter(|p| !p.vitals.is_downed()).count()
                            };
                            let softcap = crate::enemy_field::global_enemy_softcap(alive_players);
                            // 注入第一波怪物（全部在保護圈外確認）。
                            let mut enemies = app.enemies.write().unwrap();
                            let mut injected = 0u32;
                            for (wx, wy, kind) in wave {
                                if enemies.total_count() >= softcap {
                                    tracing::debug!(softcap, alive_players, "全世界敵數達軟上限，略過獸潮注入");
                                    break;
                                }
                                if world_core::town_protected_at(wx as f64, wy as f64) {
                                    tracing::warn!(x = wx, y = wy, "獸潮波次位置在保護圈內，跳過");
                                    continue;
                                }
                                enemies.inject_event_enemy(wx, wy, kind);
                                injected += 1;
                            }
                            drop(enemies);
                            tracing::info!(site = site_label, injected, eco_pressure, "獸潮廣播＋注入第一波怪物");
                            // 依生態壓力等級加上說明標籤（讓玩家感知生態失衡的嚴重程度）
                            let eco_label = if eco_pressure >= crate::director::ECO_PRESSURE_HIGH {
                                "⚠️ 生態危機！"
                            } else if eco_pressure >= crate::director::ECO_PRESSURE_MID {
                                "⚡ 生態緊張"
                            } else {
                                ""
                            };
                            let eco_hint = if eco_pressure >= crate::director::ECO_PRESSURE_HIGH {
                                "（巢穴過剩＋怪物氣焰正盛，波次更大！）"
                            } else if eco_pressure >= crate::director::ECO_PRESSURE_MID {
                                "（生態壓力上升，比平時更猛！）"
                            } else {
                                ""
                            };
                            let _ = app.tx_chat.send(format!(
                                "⚔️ 獸潮來襲！{eco_label}大批怪物正聚集在{}！{eco_hint}\
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
                            // 城鎮記憶石（ROADMAP 157）：守城勝利永久留存。
                            app.town_memory.write().unwrap().push_event(
                                "⚔️",
                                format!("守城勝利——{}獸潮被拓荒者打退（斬殺 {} 隻）", site_label, kills),
                            );
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
                            // 城鎮記憶石（ROADMAP 157）：守城失守也值得記錄，提醒玩家。
                            app.town_memory.write().unwrap().push_event(
                                "😔",
                                format!("守城失守——{}的獸潮未能在時限內打退", site_label),
                            );
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
            // 街坊相認（ROADMAP 331）：推進「玩家×NPC」崗位招呼冷卻倒數（歸零者自動剔除）。
            app.npc_recognition.write().unwrap().tick(dt);
            // 鎮民認得你的夥伴（ROADMAP 359）：推進「玩家×NPC」寵物評論冷卻倒數（歸零者自動剔除）。
            app.pet_greeting.write().unwrap().tick(dt);
            // 探索者路標（ROADMAP 353）：推進過期；有路標消失時全服重播一次路標列表（出鎖後送）。
            {
                let changed = app.wayposts.write().unwrap().tick(dt);
                if changed {
                    let posts: Vec<crate::protocol::WaypostView> = app
                        .wayposts
                        .read()
                        .unwrap()
                        .posts()
                        .iter()
                        .map(|p| crate::protocol::WaypostView {
                            id: p.id,
                            x: p.x,
                            y: p.y,
                            owner_name: p.owner_name.clone(),
                            message_key: p.message_key.clone(),
                            remaining_secs: p.remaining,
                        })
                        .collect();
                    let _ = app
                        .tx
                        .send(std::sync::Arc::new(crate::protocol::ServerMsg::Wayposts { posts }));
                }
            }
            // 星海寄語 / 漂流瓶（ROADMAP 354）：推進漂流瓶／回贈／待回贈過期；有瓶子沉沒導致
            // 海上數量變動時全服重播一次最新數量（出鎖後送，守 prod-deadlock）。
            {
                let changed = app.bottles.write().unwrap().tick(dt);
                if changed {
                    let count = app.bottles.read().unwrap().drifting_count() as u32;
                    let _ = app.tx.send(std::sync::Arc::new(
                        crate::protocol::ServerMsg::BottleSeaCount { count },
                    ));
                }
            }
            // 牧場系統（ROADMAP 48）：推進所有有雞地塊的下蛋計時器。
            app.ranch.write().unwrap().tick(dt);
            // 農地作物系統（ROADMAP 49）：推進所有農田地塊的作物生長計時器；下雨時給 1.5x 加成。
            app.farm_crops.write().unwrap().tick(dt, is_raining);
            // NPC 作息與移動（ROADMAP 73）：推進 NPC 位置。
            {
                let daynight = app.daynight.read().unwrap();
                // ROADMAP 356：黃昏串門子——黃昏時讀一次 355 湧現的結盟配對，算出「誰去拜訪誰」，
                // 讓結盟 NPC 離崗走到盟友攤前寒暄。只在黃昏算（其餘時段省下這把讀鎖與計算）；
                // 先取完結盟、放掉 npc_relations 鎖，再取 npc_schedule 寫鎖 tick（不巢狀上鎖，守死鎖鐵律）。
                let visits = if daynight.phase() == crate::daynight::Phase::Dusk {
                    let alliances: Vec<(&str, &str)> = {
                        let rel = app.npc_relations.read().unwrap();
                        crate::npc_factions::current_standings(&rel)
                            .into_iter()
                            .filter(|s| s.bond == crate::npc_factions::FactionBond::Alliance)
                            .map(|s| (s.npc_a, s.npc_b))
                            .collect()
                    };
                    crate::npc_schedule::dusk_visit_plan(&alliances)
                } else {
                    std::collections::HashMap::new()
                };
                app.npc_schedule.write().unwrap().tick(dt, &daynight, &visits);
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
                            // ROADMAP 171：旅人野外見聞——根據生態狀況廣播野外快報。
                            {
                                let pressure = app.director.read().unwrap().eco_pressure();
                                let (alpha_count, top_colony) = {
                                    let cols = app.monster_colonies.read().unwrap();
                                    let ac = cols.alphas.len();
                                    let tc = cols.colonies.iter()
                                        .max_by_key(|c| c.population)
                                        .filter(|c| c.population > 0)
                                        .map(|c| c.name.to_string());
                                    (ac, tc)
                                };
                                let ctx = crate::eco_report::EcoReportContext {
                                    pressure,
                                    active_alpha_count: alpha_count,
                                    top_colony_name: top_colony,
                                };
                                if let Some(report) = crate::eco_report::pick_eco_report(name, &ctx) {
                                    let _ = app.tx_chat.send(report);
                                }
                            }
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
                // 街坊相認（ROADMAP 331）：白天各自在崗位的七大 NPC，認出走近的熟客玩家、點名招呼一句，
                // 把 329/330 在午休桌上攢起的相熟度，第一次兌現到午休之外的整日城鎮裡。
                // 只在白天工作時段、非午休（午休另走席間舉杯）、相熟度 ≥ 點頭之交、且過了招呼冷卻時觸發。
                {
                    use crate::npc_schedule::{NpcActivity, VILLAGE_NPCS};
                    // 1. 只在白天且非午休時段相認（夜歇／趕路／午休皆不觸發；午休自有席間舉杯）。
                    let day_not_lunch = {
                        let dn = app.daynight.read().unwrap();
                        dn.phase() == crate::daynight::Phase::Day
                            && !crate::npc_schedule::is_lunch_time(dn.phase(), dn.fraction())
                    };
                    if day_not_lunch {
                        // 2. 收集目前真正在崗位工作（非趕路／夜歇／午休）的村落 NPC 及其座標。
                        let working: Vec<(&'static str, f32, f32)> = {
                            let sched = app.npc_schedule.read().unwrap();
                            VILLAGE_NPCS
                                .iter()
                                .filter_map(|s| {
                                    let act = sched.get_activity(s.id)?;
                                    let on_duty = !matches!(
                                        act,
                                        NpcActivity::Commuting
                                            | NpcActivity::Resting
                                            | NpcActivity::Lunching
                                    );
                                    if on_duty {
                                        sched.get_pos(s.id).map(|(x, y)| (s.id, x, y))
                                    } else {
                                        None
                                    }
                                })
                                .collect()
                        };
                        // 3. 快照在線、未倒地的玩家（鍵＝玩家 id，與相熟度帳本一致）。
                        let players_snap: Vec<(uuid::Uuid, String, f32, f32)> = {
                            let players = app.players.read().unwrap();
                            players
                                .values()
                                .filter(|p| !p.vitals.is_downed())
                                .map(|p| (p.id, p.name.clone(), p.x, p.y))
                                .collect()
                        };
                        if !working.is_empty() && !players_snap.is_empty() {
                            // 4. 在讀鎖／冷卻寫鎖內判定要招呼誰、招呼哪一句（持鎖期間不送訊息）。
                            let mut greetings: Vec<(String, String, String, f32, f32)> = Vec::new();
                            {
                                let regulars = app.lunch_regulars.read().unwrap();
                                let mut recog = app.npc_recognition.write().unwrap();
                                for &(npc_id, nx, ny) in &working {
                                    for (pid, pname, px, py) in &players_snap {
                                        if !crate::npc_recognition::within_reach(nx, ny, *px, *py) {
                                            continue;
                                        }
                                        let player_key = pid.to_string();
                                        if !recog.ready(&player_key, npc_id) {
                                            continue;
                                        }
                                        // 相熟度由午餐桌累積的舉杯次數推定；生面孔（Stranger）取不到招呼語、自然跳過。
                                        let count = regulars.count(&player_key, npc_id);
                                        let tier = crate::lunch_regular::tier_of(count);
                                        if let Some(tpl) = crate::npc_recognition::recognize_line(
                                            npc_id, tier, count as usize,
                                        ) {
                                            let text =
                                                crate::npc_recognition::fill_name(tpl, pname);
                                            recog.mark(&player_key, npc_id);
                                            greetings.push((
                                                npc_id.to_string(),
                                                crate::lunch_chatter::display_name(npc_id)
                                                    .to_string(),
                                                text,
                                                nx,
                                                ny,
                                            ));
                                        }
                                    }
                                }
                            }
                            // 5. 鎖外廣播 NpcSpeech 招呼泡泡（就地定位在 NPC 崗位上，不洗世界聊天）。
                            for (npc_id, npc_name, text, wx, wy) in greetings {
                                let _ = app.tx.send(std::sync::Arc::new(
                                    crate::protocol::ServerMsg::NpcSpeech {
                                        npc_id,
                                        npc_name,
                                        text,
                                        display_secs: 6,
                                        wx,
                                        wy,
                                    },
                                ));
                            }
                        }

                        // 鎮民認得你的夥伴（ROADMAP 359）：複用上面收好的「在崗 NPC（working）」，
                        // 讓帶著寵物走近的玩家被搭一句評論寵物個性（358）的就地泡泡。與 331 街坊相認
                        // 刻意分工——那條認的是「玩家是誰」（依午餐相熟度點名），這條認的是「你帶的是
                        // 什麼脾氣的夥伴」（依寵物個性，不需任何相熟度，人人帶寵物路過都可能被搭話）。
                        if !working.is_empty() {
                            // 快照在線、室外、帶著寵物的玩家及其寵物個性（鍵＝玩家 id，與冷卻帳本一致）。
                            // 個性由「主人帳號＋寵物種類」確定性算出（純函式、零鎖、無 IO，與 358 跟隨一致）。
                            let pet_walkers: Vec<(uuid::Uuid, f32, f32, crate::pet_personality::PetPersonality)> = {
                                let players = app.players.read().unwrap();
                                players
                                    .values()
                                    .filter(|p| !p.vitals.is_downed() && p.indoor_plot_id.is_none())
                                    .filter_map(|p| {
                                        let kind = p.pet?;
                                        let personality = crate::pet_personality::personality_for(
                                            p.id.as_bytes(),
                                            kind,
                                        );
                                        Some((p.id, p.x, p.y, personality))
                                    })
                                    .collect()
                            };
                            if !pet_walkers.is_empty() {
                                // 在冷卻寫鎖內判定要評論誰、說哪一句（持鎖期間不送訊息）。
                                let mut pet_greets: Vec<(String, String, String, f32, f32)> = Vec::new();
                                {
                                    let mut book = app.pet_greeting.write().unwrap();
                                    for &(npc_id, nx, ny) in &working {
                                        for &(pid, px, py, personality) in &pet_walkers {
                                            if !crate::pet_greeting::within_reach(nx, ny, px, py) {
                                                continue;
                                            }
                                            let player_key = pid.to_string();
                                            if !book.ready(&player_key, npc_id) {
                                                continue;
                                            }
                                            if let Some(text) = book.greet(npc_id, personality) {
                                                book.mark(&player_key, npc_id);
                                                pet_greets.push((
                                                    npc_id.to_string(),
                                                    crate::lunch_chatter::display_name(npc_id)
                                                        .to_string(),
                                                    text.to_string(),
                                                    nx,
                                                    ny,
                                                ));
                                            }
                                        }
                                    }
                                }
                                // 鎖外廣播 NpcSpeech 評論泡泡（就地定位在 NPC 崗位上，不洗世界聊天）。
                                for (npc_id, npc_name, text, wx, wy) in pet_greets {
                                    let _ = app.tx.send(std::sync::Arc::new(
                                        crate::protocol::ServerMsg::NpcSpeech {
                                            npc_id,
                                            npc_name,
                                            text,
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
                // 野生動物 tick（ROADMAP 141 食物鏈）。
                {
                    let positions: Vec<(f32, f32)> = player_positions.iter()
                        .map(|(_, x, y)| (*x, *y))
                        .collect();
                    // ROADMAP 144：取得物種態度 Map，傳入 wildlife tick。
                    let attitudes = app.species_relations.read().unwrap().attitudes.clone();
                    // ROADMAP 165：收集獵食型怪物位置，供野生動物逃跑用（讀鎖即放）。
                    let monster_threats: Vec<(crate::combat::EnemyKind, f32, f32)> = {
                        let enemies = app.enemies.read().unwrap();
                        enemies.enemies().iter()
                            .filter(|e| e.enemy.is_alive())
                            .filter_map(|e| {
                                crate::wildlife::monster_hunts_wildlife(e.enemy.kind())
                                    .map(|_| (e.enemy.kind(), e.x, e.y))
                            })
                            .collect()
                    };
                    // ROADMAP 296：餵入本幀權威天氣（是否下雨），供草食獸雨中避雨判定（走欄位、不動 tick 簽名）。
                    // ROADMAP 301：餵入本幀權威天象（是否正逢流星雨），供夜裡草食獸抬頭仰望流星判定（同走欄位）。
                    let meteor_active = app.meteor_shower.read().unwrap().is_active();
                    // ROADMAP 302：餵入本幀權威月相（是否滿月夜），供滿月夜掠食者「對月特別愛嚎」判定。
                    // 以系統時間（Unix epoch 毫秒）經 moon 模組（與前端 moonPhase 同公式）判定，前後端月相一致。
                    let moon_full = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| crate::moon::is_full_moon(d.as_millis() as f64))
                        .unwrap_or(false);
                    // ROADMAP 305：餵入本幀權威時辰（是否破曉，頂部已與 is_night 同把鎖取得），供破曉野鳥
                    // 「拂曉滿林齊鳴」判定（同走欄位、不動 tick 簽名）。
                    let wildlife_events = {
                        let mut wm = app.wildlife_manager.write().unwrap();
                        wm.set_raining(is_raining);
                        wm.set_meteor_active(meteor_active);
                        wm.set_moon_full(moon_full);
                        wm.set_dawn(is_dawn);
                        wm.set_dusk(is_dusk);
                        wm.set_summer(is_summer);
                        wm.set_hot(is_hot);
                        wm.set_winter(is_winter);
                        wm.set_cold(is_cold);
                        wm.set_autumn(is_autumn);
                        wm.set_spring(is_spring);
                        wm.tick(dt, &positions, &attitudes, &monster_threats, is_night)
                    };
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
                            // ROADMAP 165：MonsterHunted 由怪物擊殺路徑（on_monster_kills_wildlife）
                            // 產生並已在上方處理；wildlife tick 不應再發出此事件，此 arm 僅防止
                            // 非窮盡匹配的編譯錯誤。
                            WildlifeEvent::MonsterHunted { .. } => {}
                            // ROADMAP 207：安穩成群的獵物孕育出新生命——低頻、療癒向的世界訊息。
                            WildlifeEvent::Born { kind, x, y } => {
                                let msg = format!(
                                    "🌱 城外 ({:.0},{:.0})：一隻 {} 的幼獸誕生了，獸群又添了新成員。",
                                    x, y, kind.display_name()
                                );
                                let _ = app.tx_chat.send(msg);
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
                // ROADMAP 163：怪物物種關係 tick（態度自然衰減 + 層級改變廣播）。
                {
                    let ms_events = app.monster_species.write().unwrap().tick(dt);
                    for ev in ms_events {
                        use crate::species_relations::MonsterRelationEvent;
                        match ev {
                            MonsterRelationEvent::TierChanged { kind, new_tier } => {
                                let _ = app.tx_chat.send(format!(
                                    "🐾 [怪物生態] {} 對人類的態度變為{}。",
                                    kind.display_name(), new_tier.display_zh()
                                ));
                            }
                        }
                    }
                }
                // ROADMAP 164：怪物巢穴 tick——族群補充計時器推進，發出生成指令。
                // 傳入當前生態壓力（ROADMAP 175：覺醒危機判斷用）。
                {
                    let eco_now = app.director.read().unwrap().eco_pressure();
                    let colony_events = app.monster_colonies.write().unwrap().tick(dt, eco_now);
                    for ev in colony_events {
                        use crate::monster_colony::MonsterColonyEvent;
                        match ev {
                            MonsterColonyEvent::SpawnAt { kind, x, y, .. } => {
                                app.enemies.write().unwrap().inject_event_enemy(x, y, kind);
                            }
                            MonsterColonyEvent::ColonyCleared { name, cx, cy } => {
                                let _ = app.tx_chat.send(format!(
                                    "🏕️ [{name}] ({cx:.0},{cy:.0}) 的怪物巢穴被清剿一空！暫時解放此區域。"
                                ));
                            }
                            MonsterColonyEvent::ColonyRevived { name } => {
                                let _ = app.tx_chat.send(format!(
                                    "⚠️ [{name}] 怪物巢穴族群開始恢復，請注意！"
                                ));
                            }
                            MonsterColonyEvent::AlphaAppeared { colony_name, kind } => {
                                let kind_name = kind.display_name();
                                let _ = app.tx_chat.send(format!(
                                    "👑 [{colony_name}] 族群達到巔峰！Alpha 首領「{kind_name}·霸主」降臨領地，尋求挑戰者！"
                                ));
                                // ROADMAP 171：Alpha 湧現觸發 NPC 主動評論。
                                {
                                    let event_kind = crate::npc_proactive::WorldEventKind::AlphaEmergent {
                                        colony_name: colony_name.to_string(),
                                        kind_name: kind_name.to_string(),
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
                            // ROADMAP 170：Alpha 領地爭奪——衝突開始廣播。
                            MonsterColonyEvent::AlphaClashStart { colony_a_name, colony_b_name } => {
                                let _ = app.tx_chat.send(format!(
                                    "⚔️ 領地爭奪！[{colony_a_name}] Alpha vs [{colony_b_name}] Alpha——兩霸互搏！"
                                ));
                            }
                            // ROADMAP 170：Alpha 領地爭奪——衝突結果廣播。
                            MonsterColonyEvent::AlphaClashVictory { winner_colony_name, loser_colony_name, .. } => {
                                let _ = app.tx_chat.send(format!(
                                    "🏆 [{winner_colony_name}] Alpha 擊潰了 [{loser_colony_name}] Alpha！[{winner_colony_name}] 稱霸此區域，[{loser_colony_name}] 元氣大傷！趁勢攻擊殘血霸主！"
                                ));
                                // ROADMAP 171：領地爭奪結果觸發 NPC 主動評論。
                                {
                                    let event_kind = crate::npc_proactive::WorldEventKind::AlphaClashResult {
                                        winner_colony: winner_colony_name.to_string(),
                                        loser_colony: loser_colony_name.to_string(),
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
                            // ROADMAP 169：Alpha 咆哮指揮——決定戰術並非同步生成台詞廣播。
                            MonsterColonyEvent::AlphaCommandReady { alpha_id, colony_name, kind, hp_pct, alpha_x, alpha_y } => {
                                let kind_name = kind.display_name();
                                // ROADMAP 371：收集 Alpha 感知半徑內的玩家座標，算出陣形（人數＋離散度）。
                                // 先讀 players 取座標即放鎖，後續純函式計算、不巢狀上鎖（守 prod-deadlock）。
                                let player_positions: Vec<(f32, f32)> = {
                                    let players = app.players.read().unwrap();
                                    players.values().map(|p| (p.x, p.y)).collect()
                                };
                                let (nearby_players, spread_px) = crate::boss_ai::formation_of(
                                    (alpha_x, alpha_y),
                                    &player_positions,
                                    crate::boss_ai::PERCEPTION_RADIUS,
                                );
                                // 同步決定戰術（零延遲、讀陣反制），寫回 Alpha 狀態（前端快照可見）。
                                let tactic = crate::boss_ai::adaptive_tactic(hp_pct, nearby_players, spread_px);
                                let tactic_name = tactic.display_name().to_string();
                                app.monster_colonies.write().unwrap()
                                    .set_alpha_tactic(alpha_id, tactic_name.clone());
                                // 非同步生成廣播台詞（Groq → 罐頭降級）。
                                let tx_chat = app.tx_chat.clone();
                                let sem = app.boss_ai_sem.clone();
                                let kind_name_s = kind_name.to_string();
                                let colony_s = colony_name.to_string();
                                let tactic_c = tactic.clone();
                                tokio::spawn(async move {
                                    let Ok(_permit) = sem.try_acquire_owned() else { return };
                                    let msg = crate::boss_ai::generate_tactic_message(&kind_name_s, 1, &tactic_c).await;
                                    let tactic_disp = tactic_c.display_name();
                                    let _ = tx_chat.send(format!(
                                        "📣 〔{colony_s} Alpha・{kind_name_s}〕下令「{tactic_disp}」：{msg}"
                                    ));
                                });
                            }
                            // ROADMAP 172：MonsterKilledInColony 由 ws.rs 的 on_monster_killed_near 路徑處理，
                            // game.rs tick() 不會產生此事件，但需加入 match 分支。
                            MonsterColonyEvent::MonsterKilledInColony { .. } => {}
                            // ROADMAP 173：傳說古 Alpha 降臨——廣播警報 + Groq 生成宣言台詞。
                            MonsterColonyEvent::AncientAlphaEmerged { x, y } => {
                                let _ = app.tx_chat.send(format!(
                                    "⚡【傳說古 Alpha 降臨！】生態壓力達到頂點，傳說古 Alpha 現身荒野 ({x:.0},{y:.0})！\
                                     全服合力挑戰，擊倒後可得傳說晶核，合成傳說戰刃！"
                                ));
                                // 非同步呼叫 Groq 生成古 Alpha 宣言（利用 boss_ai_sem 限流）。
                                let tx_chat = app.tx_chat.clone();
                                let sem = app.boss_ai_sem.clone();
                                tokio::spawn(async move {
                                    let Ok(_permit) = sem.try_acquire_owned() else { return };
                                    let system =
                                        "你是蒸汽龐克太空歌劇世界中的「傳說古 Alpha」，宇宙最古老的生態守護者，\
                                         被人類打破生態平衡所激怒而降臨。\
                                         你俯視所有生命，視一切入侵者為微塵。\
                                         請用 30 字以內的繁體中文，以古老王者的口吻，\
                                         發出一聲震懾四方的降臨宣言。\
                                         只輸出那一句宣言，不要引號或前綴。";
                                    let user = "傳說古 Alpha 降臨，發出你的宣言！";
                                    let canned = "「爾等渺小的侵略者，今日便是汝等滅頂之時！」";
                                    let text = match crate::npc_chat::raw_llm_call(system, user).await {
                                        Some(t) if !t.trim().is_empty() => t.trim().to_string(),
                                        _ => canned.to_string(),
                                    };
                                    let _ = tx_chat.send(format!("⚡ 古 Alpha 宣言：{text}"));
                                });
                            }
                            // ROADMAP 173：傳說古 Alpha 被擊倒——廣播由 ws.rs 負責（含殺手名稱）。
                            MonsterColonyEvent::AncientAlphaSlain => {}
                            // ROADMAP 174：跨族結盟達成——廣播警報。
                            MonsterColonyEvent::AllianceFormed { alpha_a_name, alpha_b_name } => {
                                let _ = app.tx_chat.send(format!(
                                    "🤝【跨族結盟！】[{alpha_a_name}] Alpha 與 [{alpha_b_name}] Alpha \
                                     締結盟約！雙方生命激增、生態壓力飆升！趁結盟未穩——分頭瓦解！"
                                ));
                            }
                            // ROADMAP 174：跨族結盟瓦解——廣播捷報。
                            MonsterColonyEvent::AllianceBroken { survivor_name } => {
                                let _ = app.tx_chat.send(format!(
                                    "💥【結盟瓦解！】盟約 Alpha 被擊倒，跨族結盟宣告瓦解！\
                                     [{survivor_name}] Alpha 失去盟友，是趁機出擊的好時機！"
                                ));
                            }
                            // ROADMAP 175：Alpha 覺醒危機——廣播全服緊急警報。
                            MonsterColonyEvent::AlphaAwakened { count } => {
                                let _ = app.tx_chat.send(format!(
                                    "🔥【Alpha 覺醒危機！】生態壓力衝頂，荒野 {count} 隻 Alpha 首領全數覺醒！\
                                     生命激增、攻擊加倍——速速清剿或壓低生態壓力！"
                                ));
                            }
                            // ROADMAP 176：物種霸主稱霸——廣播全服警示，玩家知道有額外乙太可拿
                            MonsterColonyEvent::DominanceDeclaration { colony_name, .. } => {
                                let _ = app.tx_chat.send(format!(
                                    "👑【霸主湧現！】{colony_name} 巢穴稱霸！族群鼎盛、Alpha 長踞——\
                                     速去制伏，擊殺有額外乙太！生態壓力持續上升！"
                                ));
                            }
                            // ROADMAP 176：霸主落幕——廣播全服
                            MonsterColonyEvent::DominanceBroken { colony_name, .. } => {
                                let _ = app.tx_chat.send(format!(
                                    "👑【霸主落幕！】{colony_name} 的霸主之勢瓦解！"
                                ));
                            }
                            // ROADMAP 176：霸主巢穴普通怪擊殺——由 ws.rs 發獎，game.rs 忽略
                            MonsterColonyEvent::MonsterKilledInDominantColony => {}
                            // ROADMAP 179：怪物王號令援軍——注入援軍小怪並廣播全服警示。
                            MonsterColonyEvent::AlphaSummonedReinforcements { colony_name, kind, count, positions } => {
                                // 全域軟上限：世界已夠擠時 Alpha 援軍也不再注入（先取人數快照再鎖 enemies）。
                                let alive_players = {
                                    let players = app.players.read().unwrap();
                                    players.values().filter(|p| !p.vitals.is_downed()).count()
                                };
                                let softcap = crate::enemy_field::global_enemy_softcap(alive_players);
                                {
                                    let mut enemies = app.enemies.write().unwrap();
                                    for (x, y) in &positions {
                                        if enemies.total_count() >= softcap {
                                            tracing::debug!(softcap, alive_players, "全世界敵數達軟上限，略過 Alpha 援軍注入");
                                            break;
                                        }
                                        enemies.inject_event_enemy(*x, *y, kind);
                                    }
                                }
                                let kind_name = kind.display_name();
                                let _ = app.tx_chat.send(format!(
                                    "🩸 [{colony_name}] Alpha 重傷！號令「{kind_name}」援軍 {count} 隻馳援、圍護首領——把握時機速戰速決！"
                                ));
                            }
                            // ROADMAP 183：潰逃事件只由 ws.rs 擊殺路徑（on_monster_killed_near）產生並處理，
                            // 主 tick 的巢穴推進不會發出此事件，這裡僅補 exhaustive 分支。
                            MonsterColonyEvent::ColonyRouted { .. } => {}
                            // ROADMAP 184：菁英 Alpha 背水死戰——無援可召的瀕死絕境，發垂死怒吼廣播。
                            MonsterColonyEvent::AlphaLastStand { colony_name, kind } => {
                                let kind_name = kind.display_name();
                                let _ = app.tx_chat.send(format!(
                                    "🩸 [{colony_name}] 的「{kind_name}」首領走投無路、援軍已斷，發出垂死怒吼背水死戰——趁勝追擊，斬草除根！"
                                ));
                            }
                        }
                    }
                }
                // ROADMAP 244：收集主要 NPC 與旅人的即時位置，傳入居民 tick 觸發鄰里招呼。
                let mut major_npcs = Vec::new();
                {
                    let sch = app.npc_schedule.read().unwrap();
                    let lc = app.npc_lifecycle.read().unwrap();
                    for s in crate::npc_schedule::VILLAGE_NPCS {
                        let name = lc.current_display(s.id).to_string();
                        if let Some((mx, my)) = sch.get_pos(s.id) {
                            major_npcs.push((s.id.to_string(), name, mx, my));
                        }
                    }
                    let tv = app.traveler.read().unwrap();
                    if tv.is_visible() {
                        major_npcs.push(("traveler".to_string(), tv.name().to_string(), tv.x, tv.y));
                    }
                }

                // ROADMAP 180：把當前生態壓力傳入居民 tick，驅動「生態危機避難」整體反應。
                let eco_for_residents = app.director.read().unwrap().eco_pressure();
                let world_log_snap: Vec<String> = app.world_log.read().unwrap().recent().iter().cloned().collect();
                let relations_ref = app.npc_relations.read().unwrap();
                let (resident_events, thought_events) = app.residents.write().unwrap()
                    .tick(dt, avg_prosperity, current_phase, &player_positions, eco_for_residents, &major_npcs, &world_log_snap, &relations_ref);
                drop(relations_ref);
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
                        // ROADMAP 180：生態危機避難開始／解除——全服廣播，讓玩家感受到城鎮對野外危機的反應。
                        ResidentLifecycleEvent::EcoAlarm { msg }
                        | ResidentLifecycleEvent::EcoCalm { msg } => {
                            let _ = app.tx_chat.send(msg);
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
                        // ROADMAP 188：凱旋英雄禮讚——餘韻期間英雄本人走近，居民停步致謝。
                        // 只發頭頂 🙏 NpcSpeech 泡泡（就近專屬反應，不洗世界聊天；全城歡慶捷報已在 185 廣播過）。
                        ResidentLifecycleEvent::HeroGratitude {
                            resident_id, resident_name, x, y, hero_name: _, text,
                        } => {
                            let _ = app.tx.send(std::sync::Arc::new(crate::protocol::ServerMsg::NpcSpeech {
                                npc_id: resident_id,
                                npc_name: format!("居民 {}", resident_name),
                                text,
                                display_secs: 7,
                                wx: x,
                                wy: y,
                            }));
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
                        // ROADMAP 244：主要 NPC 與居民相遇打招呼——廣播雙方 NpcSpeech 泡泡。
                        ResidentLifecycleEvent::MajorNpcChat {
                            major_id, major_name, major_x, major_y,
                            resident_id, resident_name, resident_x, resident_y,
                            major_text, resident_text,
                        } => {
                            let _ = app.tx.send(std::sync::Arc::new(crate::protocol::ServerMsg::NpcSpeech {
                                npc_id: major_id,
                                npc_name: major_name,
                                text: major_text,
                                display_secs: 6,
                                wx: major_x,
                                wy: major_y,
                            }));
                            let _ = app.tx.send(std::sync::Arc::new(crate::protocol::ServerMsg::NpcSpeech {
                                npc_id: resident_id,
                                npc_name: format!("居民 {}", resident_name),
                                text: resident_text,
                                display_secs: 6,
                                wx: resident_x,
                                wy: resident_y,
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
                        ResidentLifecycleEvent::ProsperityChanged { msg, old_level, new_level } => {
                            let _ = app.tx_chat.send(msg.clone());
                            // 城鎮記憶石（ROADMAP 157）：繁榮等級升降是城鎮的重要里程碑。
                            let icon = if new_level > old_level { "📈" } else { "📉" };
                            app.town_memory.write().unwrap().push_event(
                                icon,
                                format!("城鎮繁榮{}至等級 {}", if new_level > old_level { "提升" } else { "下滑" }, new_level),
                            );
                        }
                        ResidentLifecycleEvent::ExpeditionStarted { msg, .. } => {
                            let _ = app.tx_chat.send(msg);
                        }
                        ResidentLifecycleEvent::ExpeditionSuccess { names, msg } => {
                            let _ = app.tx_chat.send(msg);
                            // 全服在線玩家各得 10 乙太
                            {
                                let mut players = app.players.write().unwrap();
                                for p in players.values_mut() {
                                    p.ether += 10;
                                }
                            }
                            // 記錄到記憶石
                            app.town_memory.write().unwrap().push_event(
                                "🏹",
                                format!("居民採集隊 {} 平安歸來，帶回珍貴樣本！全服在線玩家獲 10 乙太。", names.join("、"))
                            );
                        }
                        ResidentLifecycleEvent::ExpeditionFailed { msg, .. } => {
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
                // ROADMAP 245：入夜聚會閒談——入夜後圍聚在聚會點的兩位主要 NPC 彼此攀談，
                // 語氣隨 npc_relations 好感冷暖流動。與 244（白天、NPC↔居民）對成晝夜一對。
                // 頻率溫和（每隔 GATHER_INTERVAL 才一組）、純啟發式零 LLM。
                {
                    const GATHER_INTERVAL_SECS: u64 = 20;
                    let gather_ticks = GATHER_INTERVAL_SECS * TICK_HZ as u64;
                    if current_phase == crate::daynight::Phase::Night
                        && tick % gather_ticks == 0
                        && tick > 0
                    {
                        let relations_ref = app.npc_relations.read().unwrap();
                        // major_npcs 已含村裡主要 NPC 的即時（夜間聚會）位置（旅人入夜多半不在場）。
                        if let Some(chat) =
                            crate::npc_gather::pick_gather_pair(&major_npcs, &relations_ref, tick as usize)
                        {
                            drop(relations_ref);
                            let _ = app.tx.send(std::sync::Arc::new(crate::protocol::ServerMsg::NpcSpeech {
                                npc_id: chat.speaker_id,
                                npc_name: chat.speaker_name,
                                text: chat.speaker_text,
                                display_secs: 6,
                                wx: chat.speaker_x,
                                wy: chat.speaker_y,
                            }));
                            let _ = app.tx.send(std::sync::Arc::new(crate::protocol::ServerMsg::NpcSpeech {
                                npc_id: chat.listener_id,
                                npc_name: chat.listener_name,
                                text: chat.listener_text,
                                display_secs: 6,
                                wx: chat.listener_x,
                                wy: chat.listener_y,
                            }));
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
                        // ROADMAP 186：凱旋餘韻期內改冒「勝利談資」（居民聊剛斬下的菁英首領）；否則照常態思想模板。
                        let text = if ev.triumph {
                            crate::resident_chat::get_triumph_thought(ev.seed)
                        } else {
                            crate::resident_chat::get_thought(ev.persona, &ctx, ev.seed)
                        };
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
                            let _ = app.tx_chat.send(text.clone());
                            // 城鎮記憶石（ROADMAP 157）：提案通過是城鎮歷史的一頁。
                            app.town_memory.write().unwrap().push_event("📜", text);
                        }
                        CivicVoteEvent::ProposalRejected { text } => {
                            let _ = app.tx_chat.send(text.clone());
                            // 城鎮記憶石（ROADMAP 157）：提案未通過也記下來。
                            app.town_memory.write().unwrap().push_event("🗳️", text);
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

            // 城鎮入侵警報 tick（ROADMAP 158/159）：每 90 分鐘一波野怪+首領衝向城鎮外圍，玩家攜手抵禦。
            // 城鎮入侵警報 tick（ROADMAP 158/159/161）。
            {
                use crate::invasion::InvasionEvent;
                let invasion_event = app.invasion.write().unwrap().tick(dt);
                if let Some(ev) = invasion_event {
                    match ev {
                        InvasionEvent::Started { spawns, wave_level, consecutive_successes } => {
                            // 注入入侵怪物到敵人場中（含首領乙太霸主）。
                            {
                                let mut enemies = app.enemies.write().unwrap();
                                for (kind, x, y) in &spawns {
                                    enemies.inject_event_enemy(*x, *y, *kind);
                                }
                            }
                            let wave = app.invasion.read().unwrap().wave_count;
                            let mob_count = spawns.len() - 1; // 不含首領
                            // 連勝指示文字（Lv.2+ 才顯示）。
                            let streak_hint = if consecutive_successes > 0 {
                                format!("（連勝 {} 波）", consecutive_successes)
                            } else {
                                String::new()
                            };
                            let level_tag = if wave_level >= 2 {
                                format!(" [Lv.{} 入侵]", wave_level)
                            } else {
                                String::new()
                            };
                            let text = format!(
                                "⚔️ [入侵警報{}] 第 {} 波野獸大軍從城鎮外圍逼近{}！乙太霸主率 {} 隻怪物包圍城鎮！(5 分鐘後波次消退)",
                                level_tag, wave + 1, streak_hint, mob_count
                            );
                            let _ = app.tx_chat.send(text);
                            app.town_memory.write().unwrap().push_event("⚔️", format!(
                                "城鎮入侵警報——第 {} 波野獸大軍突襲（Lv.{}），乙太霸主率軍登場",
                                wave + 1, wave_level
                            ));
                        }
                        InvasionEvent::Ended { boss_killed, wave_level, consecutive_successes } => {
                            let wave = app.invasion.read().unwrap().wave_count;
                            // 首領逃脫固定 +5 乙太；首領擊殺依等級遞增。
                            let ether_reward = if boss_killed {
                                // 從 invasion state 讀取本波等級對應的獎勵。
                                match wave_level { 3 => 20, 2 => 15, _ => 10 }
                            } else { 5 };
                            {
                                let mut players = app.players.write().unwrap();
                                for p in players.values_mut() {
                                    p.ether = p.ether.saturating_add(ether_reward);
                                }
                            }
                            let (text, memory_emoji, memory_text) = if boss_killed {
                                let streak_msg = if consecutive_successes >= crate::invasion::WAVE_LEVEL_3_THRESHOLD {
                                    format!(" 🔥 連勝 {} 波！已達 Lv.3 入侵等級！", consecutive_successes)
                                } else if consecutive_successes >= crate::invasion::WAVE_LEVEL_2_THRESHOLD {
                                    format!(" ✨ 連勝 {} 波！已達 Lv.2 入侵等級！", consecutive_successes)
                                } else {
                                    String::new()
                                };
                                (
                                    format!(
                                        "🏆 [入侵勝利] 第 {} 波入侵擊退！乙太霸主已被玩家消滅！全服在線玩家獲得 +{} 乙太特別獎勵！{}",
                                        wave, ether_reward, streak_msg
                                    ),
                                    "🏆",
                                    format!("首領勝利——第 {} 波乙太霸主被英雄擊敗（Lv.{}），全服 +{} 乙太獎勵", wave, wave_level, ether_reward),
                                )
                            } else {
                                (
                                    format!(
                                        "🛡️ [入侵結束] 第 {} 波入侵退去，乙太霸主逃脫... 全服在線玩家獲得 +5 乙太獎勵",
                                        wave
                                    ),
                                    "🛡️",
                                    format!("守城退去——第 {} 波入侵撤退（Lv.{}），乙太霸主趁亂逃脫", wave, wave_level),
                                )
                            };
                            let _ = app.tx_chat.send(text);
                            app.town_memory.write().unwrap().push_event(memory_emoji, memory_text);
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
                    // 城鎮記憶石（ROADMAP 157）：星象預報在非同步生成前先同步記下加成種類。
                    app.town_memory.write().unwrap().push_event(
                        "🔭",
                        format!("天文台星象預報——今日全服加成：{}", bonus.name()),
                    );
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
                    // 城鎮記憶石（ROADMAP 157）：流星雨是稀有天象，值得留存。
                    app.town_memory.write().unwrap().push_event(
                        "🌠",
                        format!("流星雨降臨！城鎮周圍出現 {} 個星塵採集點", crate::meteor_shower::DUST_NODE_COUNT),
                    );
                }
            }

            // 夜間乙太泉 tick（ROADMAP 162；ROADMAP 362 滿月乙太潮）：黃昏轉夜晚時生成城外乙太泉採集點，
            // 滿月夜（同 1914 行掠食者嗥月的權威月相）額外多生 3 口月華泉。
            {
                use crate::night_aether_springs::SpringsEvent;
                let current_phase = app.daynight.read().unwrap().phase();
                // 本幀權威月相（鏡像本檔掠食者嗥月的算法；前後端 moon 公式一致）。
                let moon_full = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| crate::moon::is_full_moon(d.as_millis() as f64))
                    .unwrap_or(false);
                // 守 prod-deadlock：鎖內 tick 並讀出本夜資訊即釋放鎖，出鎖後才廣播。
                let (ev, moonlit_tonight, live_count) = {
                    let mut ns = app.night_springs.write().unwrap();
                    let ev = ns.tick(current_phase, moon_full);
                    (ev, ns.moonlit_tonight, ns.nodes.len())
                };
                match ev {
                    Some(SpringsEvent::Activated) => {
                        let msg = if moonlit_tonight {
                            format!(
                                "🌕 滿月乙太潮——月華泉格外豐沛！城外共 {} 口乙太泉湧現（含 {} 口滿月限定月華泉），夜探者可在天亮前採集，各得 {} 乙太！",
                                live_count,
                                crate::night_aether_springs::MOONLIT_SPRING_COUNT,
                                crate::night_aether_springs::ETHER_REWARD,
                            )
                        } else {
                            format!(
                                "🌙 夜幕降臨，{} 個乙太泉在城外湧現——夜探者可在天亮前採集，各得 {} 乙太！",
                                crate::night_aether_springs::SPRING_COUNT,
                                crate::night_aether_springs::ETHER_REWARD,
                            )
                        };
                        let _ = app.tx_chat.send(msg);
                    }
                    Some(SpringsEvent::Deactivated) => {
                        // 天亮靜默清除，不廣播（避免擾民）。
                    }
                    None => {}
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
            // NPC 社交平衡漣漪（ROADMAP 365）：每 SOCIAL_TICK_SECS 秒，讓關係網依社會平衡理論
            // 自我演化一步（朋友的朋友更親、朋友的敵人漸疏）——玩家在 364 替兩位鎮民和解的善意
            // 會自己漾開到他們共同的朋友身上。比衰減/派系（5 分鐘）更頻繁，讓人情變化在一場遊玩中就感受得到。
            // 鎖序（守 prod-deadlock）：先讀鎖算漂移→放掉；再寫鎖逐筆套用→放掉；最後 social_dynamics
            // 寫鎖挑廣播（warmth_after 已純算進 SocialDrift，挑廣播不需再讀 relations，不巢狀上鎖）；廣播一律出鎖後送。
            {
                const SOCIAL_TICK_SECS: u64 = 90;
                let social_ticks = SOCIAL_TICK_SECS * TICK_HZ as u64;
                if tick % social_ticks == 0 && tick > 0 {
                    let drifts = {
                        let rel = app.npc_relations.read().unwrap();
                        crate::social_dynamics::compute_drift(&rel)
                    };
                    if !drifts.is_empty() {
                        let mut rel = app.npc_relations.write().unwrap();
                        for d in &drifts {
                            rel.nudge_pair(d.a, d.b, d.delta);
                        }
                        drop(rel);
                        let line = app
                            .social_dynamics
                            .write()
                            .unwrap()
                            .pick_announcement(&drifts);
                        if let Some(text) = line {
                            let _ = app.tx_chat.send(text);
                        }
                    }
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
                    // 鎮民陣營成形（ROADMAP 366）：同週期、同一把關係讀鎖內純算當前陣營，
                    // 出鎖後再偵測新成形／成員增長的陣營並廣播（不巢狀上鎖，守 prod-deadlock）。
                    let blocs = crate::town_blocs::compute_blocs(&relations_snapshot);
                    drop(relations_snapshot);
                    for ev in faction_events {
                        let text = ev.announce_text();
                        if !text.is_empty() {
                            let _ = app.tx_chat.send(text);
                        }
                    }
                    let bloc_events = app.town_blocs.write().unwrap().detect_new(&blocs);
                    for ev in bloc_events {
                        let _ = app.tx_chat.send(ev.announce_text());
                    }
                }
            }
            // 鎮民互助分享（ROADMAP 369）：每幀推進進行中的送禮手勢（光禮飄越廣場）。
            app.town_share.write().unwrap().tick(dt);
            // 每 SHARE_TICK_SECS 秒，依七大 NPC 的「繁榮感」需求（ROADMAP 69）挑一樁
            // 「寬裕者勻給拮据者」的分享：受禮者繁榮感回升、送禮者勻出一點，世界頻道飄來暖訊、
            // 啟動一段看得見的送禮手勢。鎖序（守 prod-deadlock）：先讀 needs 取繁榮感快照→放掉；
            // 純函式挑事件；再寫 needs 套用→放掉；最後 town_share 寫鎖記錄＋啟動手勢；廣播出鎖後送。
            {
                const SHARE_TICK_SECS: u64 = 120;
                let share_ticks = SHARE_TICK_SECS * TICK_HZ as u64;
                if tick % share_ticks == 0 && tick > 0 {
                    // 七大 NPC（穩定 id 次序，與 npc_factions 一致）的當前繁榮感快照。
                    const VILLAGE_IDS: &[&str] = &[
                        "merchant", "workshop_npc", "bounty_npc", "expedition_npc",
                        "procurement_npc", "farm_fair_npc", "village_chief",
                    ];
                    let candidates: Vec<crate::town_share::ShareCandidate> = {
                        let needs = app.npc_needs.read().unwrap();
                        VILLAGE_IDS
                            .iter()
                            .map(|&id| crate::town_share::ShareCandidate {
                                id,
                                prosperity: needs.get(id).prosperity,
                            })
                            .collect()
                    };
                    let last_pair = app.town_share.read().unwrap().last_pair().map(|(g, r)| {
                        // 把借出的 &str 轉成擁有，避免跨鎖借用。
                        (g.to_string(), r.to_string())
                    });
                    let last_ref = last_pair.as_ref().map(|(g, r)| (g.as_str(), r.as_str()));
                    if let Some(ev) = crate::town_share::pick_share(&candidates, last_ref) {
                        {
                            let mut needs = app.npc_needs.write().unwrap();
                            needs.adjust_prosperity(ev.receiver, ev.give);
                            needs.adjust_prosperity(ev.giver, -ev.cost);
                        }
                        app.town_share.write().unwrap().begin(ev.giver, ev.receiver);
                        let line = crate::town_share::announce_text(
                            crate::npc_factions::npc_display_name(ev.giver),
                            crate::npc_factions::npc_display_name(ev.receiver),
                        );
                        let _ = app.tx_chat.send(line);
                    }
                }
            }
            // 親手植樹成蔭（ROADMAP 370）：每幀推進所有玩家種下的樹的成長（隨真實時間長大）；
            // 若本幀有樹剛長成大樹（過了冷卻），飄一句世界頻道暖訊。鎖序（守 prod-deadlock）：
            // world_grove 寫鎖內純算（不碰其他鎖、無 IO），廣播一律出鎖後送。
            {
                let matured = app.world_grove.write().unwrap().tick(dt);
                if let Some(line) = matured {
                    let _ = app.tx_chat.send(line);
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
                // ROADMAP 371：先取一次全玩家座標，供每隻怪物王各自讀陣＋套用戰術共用
                // （先讀即放鎖，純函式算陣形，不巢狀上鎖、守 prod-deadlock）。
                let players_pos: Vec<(f32, f32)> = app.players.read().unwrap()
                    .values()
                    .map(|p| (p.x, p.y))
                    .collect();
                let tactic_inputs: Vec<_> = app
                    .enemies
                    .read()
                    .unwrap()
                    .enemies()
                    .into_iter()
                    .filter(|e| e.enemy.is_alive() && e.level >= e.base_level.saturating_add(3))
                    .map(|e| {
                        // 每隻怪物王讀自己感知半徑內的玩家陣形。
                        let (nearby_players, spread_px) = crate::boss_ai::formation_of(
                            (e.x, e.y),
                            &players_pos,
                            crate::boss_ai::PERCEPTION_RADIUS,
                        );
                        crate::boss_ai::TacticInput {
                            id: e.id,
                            kind_name: e.enemy.kind().display_name(),
                            level: e.level,
                            x: e.x,
                            y: e.y,
                            hp_pct: e.enemy.remaining_hp() as f32 / e.enemy.max_hp().max(1) as f32,
                            nearby_players,
                            spread_px,
                        }
                    })
                    .collect();
                let candidate = app.boss_ai.write().unwrap().tick(dt, &tactic_inputs);
                if let Some(c) = candidate {
                    // 立即套用戰術（機制效果，同步），不等 AI 台詞。
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
                // 雨後彩虹（ROADMAP 361）只在「天還亮著、有日光」時架起（夜裡不出，符合天象真實感）。
                // 先以獨立語句讀一次日夜相位（鎖即放、不與後面 weather/rainbow 寫鎖巢狀，守 prod-deadlock）。
                let is_daytime = !matches!(
                    app.daynight.read().unwrap().phase(),
                    crate::daynight::Phase::Night
                );
                let switched = app.weather.write().unwrap().advance(dt);
                if let Some(new_type) = switched {
                    let _ = app.tx_chat.send(new_type.announce_text().to_string());
                    // 從草原細雨轉換至其他天氣時，補一則「雨停了」提示。
                    if is_raining {
                        let _ = app.tx_chat.send(
                            "🌤️ 雨停了！農地恢復乾燥，記得幫作物澆水喔！".to_string()
                        );
                        // ROADMAP 361：雨過天青且日光仍在 → 架起全服彩虹、開啟「彩虹祝福」療癒光環。
                        if is_daytime {
                            app.rainbow.write().unwrap().appear();
                            let _ = app.tx_chat.send(crate::rainbow::APPEAR_TEXT.to_string());
                        }
                    }
                }
            }

            // 雨後彩虹療癒光環（ROADMAP 361）：天象級全服共享療癒——彩虹高掛期間，每隔
            // HEAL_PULSE_SECS 讓全服存活玩家獲得一次溫和回血（療癒向，非走近採集；刻意不複製
            // 流星雨／夜間乙太泉／季節節點那套「節點」骨架）。多數時刻無彩虹 → tick 立即早退、
            // 連 players 寫鎖都不開、零成本。回血在鎖內、公告在出鎖後送（守 prod-deadlock 鐵律：
            // rainbow 寫鎖於上一語句即釋放，與 players 寫鎖不巢狀）。
            {
                let rt = app.rainbow.write().unwrap().tick(dt);
                if rt.heal_pulse {
                    let mut players = app.players.write().unwrap();
                    for p in players.values_mut() {
                        // 倒地休息中的玩家須先自然復原，不吃彩虹祝福（heal 對 hp==0 本就 no-op，這裡明示）。
                        if p.vitals.is_downed() {
                            continue;
                        }
                        p.vitals.heal(crate::rainbow::HEAL_AMOUNT);
                    }
                }
                if rt.vanished {
                    let _ = app.tx_chat.send(crate::rainbow::VANISH_TEXT.to_string());
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

            // 午休席間閒話（ROADMAP 328）：正午圍桌共食時，NPC 輪流冒出一句家常閒話泡泡。
            // 純模板、零 LLM、零網路；只在午休時段、只發就地 NpcSpeech（不洗世界聊天頻道），
            // 與 76 夜談 / 81 白日對話（皆 LLM＋聊天頻道）區隔。ROADMAP 114：0 玩家時仍持續。
            {
                let (phase, fraction) = {
                    let dn = app.daynight.read().unwrap();
                    (dn.phase(), dn.fraction())
                };
                let lunching = crate::npc_schedule::is_lunch_time(phase, fraction);
                let utterance = app.lunch_chatter.write().unwrap().tick(dt, lunching);
                if let Some(u) = utterance {
                    let sched = app.npc_schedule.read().unwrap();
                    // 只有真的已坐定（活動為 Lunching）才開口；還在趕路就略過這句，
                    // 避免泡泡浮在去廣場的半路上。
                    if sched.get_activity(u.speaker_id) == Some(crate::npc_schedule::NpcActivity::Lunching) {
                        let (wx, wy) = sched
                            .get_pos(u.speaker_id)
                            .unwrap_or_else(|| crate::npc_schedule::fallback_pos(u.speaker_id));
                        drop(sched);
                        let _ = app.tx.send(std::sync::Arc::new(crate::protocol::ServerMsg::NpcSpeech {
                            npc_id: u.speaker_id.to_string(),
                            npc_name: crate::lunch_chatter::display_name(u.speaker_id).to_string(),
                            text: u.text.to_string(),
                            display_secs: 6,
                            wx,
                            wy,
                        }));
                    }
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

            // 世界冒險日報（ROADMAP 385）：黎明轉換時廣播今日精彩回顧。
            // 在晨喚（凱爾長老）之後、午鐘（老胡）之前，讓日報夾在兩則 NPC 播報之間。
            {
                let current_phase = app.daynight.read().unwrap().phase();
                if let Some(lines) = app.daily_recap.write().unwrap().tick(dt, current_phase) {
                    let tx_chat = app.tx_chat.clone();
                    tokio::spawn(async move {
                        for line in lines {
                            let _ = tx_chat.send(line);
                        }
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
                            is_expedition: false,
                            hp_pct: None,
                            alarmed: false,
                            celebrating: false,
                            // ROADMAP 324：故鄉 NPC 的工作 / 活動狀態，前端據此畫頭頂活動符號。
                            activity: sch.get_activity(s.id).map(|a| a.code().to_string()),
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
                        is_expedition: false,
                        hp_pct: None,
                        alarmed: false,
                        celebrating: false,
                        activity: None, // 他星商人不安排故鄉作息工作狀態
                    });
                    let (cmx, cmy) = crimson_merchant_pos();
                    npc_views.push(NpcView {
                        id: "crimson_merchant".to_string(),
                        name: "🔴 赤焰商人".to_string(),
                        x: cmx,
                        y: cmy,
                        buy_list: build_dynamic_buy_list(CRIMSON_BUY_LIST, &dm, now_secs),
                        sell_list: build_static_sell_list(CRIMSON_SELL_LIST),
                        is_expedition: false,
                        hp_pct: None,
                        alarmed: false,
                        celebrating: false,
                        activity: None, // 他星商人不安排故鄉作息工作狀態
                    });
                    let (vmx2, vmy2) = void_merchant_pos();
                    npc_views.push(NpcView {
                        id: "void_merchant".to_string(),
                        name: "🌑 虛空商人".to_string(),
                        x: vmx2,
                        y: vmy2,
                        buy_list: build_dynamic_buy_list(VOID_BUY_LIST, &dm, now_secs),
                        sell_list: build_static_sell_list(VOID_SELL_LIST),
                        is_expedition: false,
                        hp_pct: None,
                        alarmed: false,
                        celebrating: false,
                        activity: None, // 他星商人不安排故鄉作息工作狀態
                    });
                    let (amx, amy) = aether_merchant_pos();
                    npc_views.push(NpcView {
                        id: "aether_merchant".to_string(),
                        name: "🌫️ 霧醚商人".to_string(),
                        x: amx,
                        y: amy,
                        buy_list: build_dynamic_buy_list(AETHER_BUY_LIST, &dm, now_secs),
                        sell_list: build_static_sell_list(AETHER_SELL_LIST),
                        is_expedition: false,
                        hp_pct: None,
                        alarmed: false,
                        celebrating: false,
                        activity: None, // 他星商人不安排故鄉作息工作狀態
                    });
                    let (omx, omy) = origin_merchant_pos();
                    npc_views.push(NpcView {
                        id: "origin_merchant".to_string(),
                        name: "🌟 星源商人".to_string(),
                        x: omx,
                        y: omy,
                        buy_list: build_dynamic_buy_list(ORIGIN_BUY_LIST, &dm, now_secs),
                        sell_list: build_static_sell_list(ORIGIN_SELL_LIST),
                        is_expedition: false,
                        hp_pct: None,
                        alarmed: false,
                        celebrating: false,
                        activity: None, // 他星商人不安排故鄉作息工作狀態
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
                                is_expedition: false,
                                hp_pct: None,
                                alarmed: false,
                                celebrating: false,
                                activity: None, // 旅人不走故鄉作息工作狀態
                            });
                            Some((tv.x, tv.y))
                        } else {
                            None
                        }
                    };

                    // —— 路人居民（ROADMAP 115）——：純模板 NPC，無商店功能。
                    let mut expedition_target = None;
                    {
                        let res = app.residents.read().unwrap();
                        expedition_target = res.expedition_target();
                        for (id, name, x, y, is_exp, hp_pct, alarmed, celebrating) in res.views() {
                            npc_views.push(NpcView {
                                id: id.to_string(),
                                name: name.to_string(),
                                x,
                                y,
                                buy_list: Vec::new(),
                                sell_list: Vec::new(),
                                is_expedition: is_exp,
                                hp_pct,
                                alarmed,
                                celebrating,
                                activity: None, // 路人居民另有作息調度，不走故鄉七大 NPC 工作狀態
                            });
                        }
                    }

                    ServerMsg::Snapshot {
                        tick,
                        players: players.values().map(|p| {
                            let mut pv = p.view(&sch, traveler_xy, app.wandering_merchant.read().unwrap().is_active());
                            // ROADMAP 418 歸家羅盤：對「已買地」的玩家補上回家方位／距離／八方位。
                            // app.plots 為內部 Mutex（index_of 只短暫上鎖、不碰 players），於此持 players
                            // 讀鎖時呼叫安全——claim 路徑一律先放掉 players 再碰 plots，鎖序一致不死鎖。
                            let has_plot = app.plots.index_of(p.id);
                            if let Some(idx) = has_plot {
                                let g = crate::wayfinding::guide_home(p.x, p.y, idx);
                                pv.home_bearing = Some(g.bearing);
                                pv.home_dist = Some(g.distance);
                                pv.home_dir = Some(g.cardinal as u8);
                            }
                            // ROADMAP 426：情境下一步提示——讀此刻情境，對已畢業玩家挑一句溫柔提示。
                            // 全部訊號便宜可得（無新增鎖、無新增持久化）；夜晚是這份提示的主場。
                            let nudge_ctx = crate::idle_nudge::NudgeCtx {
                                onboarding_active: p.onboarding.is_active(),
                                busy: p.fishing.is_some()
                                    || p.mining.is_some()
                                    || p.charging.is_some()
                                    || p.aether_draw.is_some()
                                    || p.chopping.is_some()
                                    || p.cooking.is_some(),
                                downed: p.vitals.is_downed(),
                                is_visitor: has_plot.is_none(),
                                low_hp: (p.vitals.hp() as f32)
                                    < (p.vitals.max_hp() as f32) * crate::idle_nudge::LOW_HP_FRAC,
                                is_nightish: is_night || is_dusk,
                                near_water: crate::fishing::is_near_water(p.x, p.y),
                            };
                            pv.idle_nudge =
                                crate::idle_nudge::suggest(&nudge_ctx).map(|n| n.wire_key().to_string());
                            pv
                        }).collect(),
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
                        // 蜂巢狀態（養蜂釀蜜 ROADMAP 412）：每個有蜂箱的玩家一筆。
                        hives: app.apiary.read().unwrap().all_views(),
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
                        // 雨後彩虹（ROADMAP 361）：伺服器權威全服天象，前端據此畫彩虹弧＋祝福 pill。
                        rainbow: app.rainbow.read().unwrap().view(),
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
                                    // ROADMAP 205：餵食馴養——個體親近度與馴養旗標。
                                    familiarity: a.familiarity(),
                                    tamed: a.is_tamed(),
                                    // ROADMAP 207：繁衍——幼獸體型與旗標。
                                    scale: a.scale(),
                                    juvenile: a.is_juvenile(),
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
                        expedition_target,
                        // 住家家具（ROADMAP 155）：廣播時以空陣列佔位，ws.rs 過濾層依玩家 id 填入本人家具。
                        home_furniture: vec![],
                        // 居家風格（ROADMAP 325）：同家具，廣播以 None 佔位，ws.rs 過濾層依玩家 id 填入本人風格。
                        home_style: None,
                        // 公民投票（ROADMAP 156）：當前活躍投票視圖 + 效果狀態。
                        civic_vote: app.civic_vote.read().unwrap().vote_view(),
                        civic_effect_secs: app.civic_vote.read().unwrap().effect_remaining_secs(),
                        civic_effect_kind: app.civic_vote.read().unwrap().active_effect_kind(),
                        // 城鎮入侵警報（ROADMAP 158/161）：入侵狀態供前端 HUD 顯示倒數。
                        invasion: {
                            let iv = app.invasion.read().unwrap();
                            crate::protocol::InvasionView {
                                active: iv.active,
                                remaining_secs: iv.remaining_secs(),
                                wave_count: iv.wave_count,
                                boss_alive: iv.boss_alive,
                                wave_level: iv.wave_level(),
                                consecutive_successes: iv.consecutive_successes,
                            }
                        },
                        // 夜間乙太泉（ROADMAP 162）。
                        night_spring_nodes: app.night_springs.read().unwrap().active_nodes()
                            .map(|n| crate::protocol::SpringNodeView { id: n.id, wx: n.wx, wy: n.wy, moonlit: n.moonlit })
                            .collect(),
                        // 怪物物種態度（ROADMAP 163）：各怪物種類對人類的態度值與層級。
                        monster_species_attitudes: app.monster_species.read().unwrap().views(),
                        // 怪物巢穴（ROADMAP 164）：各巢穴位置、種類、密度。
                        monster_colony_views: app.monster_colonies.read().unwrap().colony_views(),
                        // 生態壓力值（ROADMAP 167）：director 儲存的最新生態壓力，直接讀出廣播。
                        eco_pressure_value: app.director.read().unwrap().eco_pressure(),
                        // 巢穴 Alpha（ROADMAP 168）：目前活躍的 Alpha 首領。
                        alpha_monsters: app.monster_colonies.read().unwrap().alpha_views(),
                        // 生態清剿委託（ROADMAP 172）：目前活躍的清剿委託（無則為 None）。
                        eco_bounty: app.eco_bounty.read().unwrap().view(),
                        // 傳說古 Alpha（ROADMAP 173）：目前存活的世界頭目（無則為 None）。
                        ancient_alpha: app.monster_colonies.read().unwrap().ancient_alpha_view(),
                        // 生態豐收節（ROADMAP 178）：進行中的全城慶典（無則為 None）。
                        eco_festival: app.eco_festival.read().unwrap().view(),
                        // 鎮民派系一覽（ROADMAP 355）：讀當前 NPC 關係，純算出此刻所有明顯的
                        // 結盟／敵對配對送前端「鎮民派系」面板。純讀取、確定性、量小（七大 NPC ≤21 對，
                        // 通常寥寥數筆）；和平相處時為空陣列。
                        town_factions: {
                            let rel = app.npc_relations.read().unwrap();
                            crate::npc_factions::current_standings(&rel)
                                .into_iter()
                                .map(|s| crate::protocol::FactionStandingView {
                                    npc_a: s.npc_a.to_string(),
                                    npc_b: s.npc_b.to_string(),
                                    npc_a_name: crate::npc_factions::npc_display_name(s.npc_a).to_string(),
                                    npc_b_name: crate::npc_factions::npc_display_name(s.npc_b).to_string(),
                                    bond: s.bond.wire_key().to_string(),
                                    affinity: s.affinity,
                                })
                                .collect()
                        },
                        // 鎮民陣營（ROADMAP 366）：讀當前關係，純算出此刻連通成盟的三人以上群體
                        // 與各自核心人物，送前端「鎮民派系」面板的「陣營」段。純讀取、確定性、量小
                        // （七大 NPC 至多兩三群）；無人成群時為空陣列。
                        town_blocs: {
                            let rel = app.npc_relations.read().unwrap();
                            crate::town_blocs::compute_blocs(&rel)
                                .into_iter()
                                .map(|b| crate::protocol::TownBlocView {
                                    member_names: b
                                        .members
                                        .iter()
                                        .map(|id| crate::npc_factions::npc_display_name(id).to_string())
                                        .collect(),
                                    members: b.members.iter().map(|id| id.to_string()).collect(),
                                    figurehead_name: crate::npc_factions::npc_display_name(b.figurehead).to_string(),
                                    figurehead: b.figurehead.to_string(),
                                    cohesion: b.cohesion,
                                })
                                .collect()
                        },
                        // 鎮民互助分享（ROADMAP 369）：進行中的送禮手勢（至多一樁），無則 None。
                        town_share: app.town_share.read().unwrap().view(),
                        // 親手植樹成蔭（ROADMAP 370）：全服共享的世界樹群快照（隨真實時間長大）。
                        world_groves: app
                            .world_grove
                            .read()
                            .unwrap()
                            .view()
                            .into_iter()
                            .map(|t| crate::protocol::TreeView {
                                x: t.x,
                                y: t.y,
                                stage: t.stage,
                            })
                            .collect(),
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
                .map(|p| (p.id, p.name.clone(), p.species.clone(), p.x, p.y, p.ether, p.wallet.expansions(), p.exp, p.masteries, p.stats, p.skill_masteries, p.codex, p.atlas, p.skylog, p.cheers))
                .collect(),
            authed.iter().map(|p| (p.id, p.inventory.clone())).collect(),
            authed.iter().map(|p| (p.id, p.equipment.clone())).collect(),
        )
    };
    if !online.is_empty() {
        // 先更新行程內 cache（同步,供重連 recall）,再非同步 upsert 到 Postgres。
        app.positions
            .remember_all(online.iter().map(|(id, _, _, x, y, e, we, exp, m, s, sk, cx, ax, sl, cl)| (*id, *x, *y, *e, *we, *exp, *m, *s, *sk, *cx, *ax, *sl, *cl)));
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
