//! 權威遊戲迴圈：固定 tick 整合所有玩家位置，廣播世界快照。

use std::time::Duration;

use crate::protocol::{EnemyView, FieldView, NodeView, ServerMsg};
use crate::state::AppState;

/// 每秒 tick 數（伺服器模擬頻率）。
const TICK_HZ: f32 = 15.0;

/// flush 時從玩家快照收下的「線上已登入玩家狀態列」:id、名字、物種、座標、乙太。
/// 與 `PositionStore::flush_online` 收的列型別逐欄對齊(同一瞬間的快照),集中這串否則
/// 會在 `flush_all` 觸發 clippy `type_complexity` 警告的長 tuple,讓該處標註更易讀。
type OnlinePlayerRow = (uuid::Uuid, String, String, f32, f32, u32);

/// 玩家每次自動攻擊的傷害(戰鬥 1-F)。固定值,將來武器/技能可加倍(1-D 工具倍率同款)。
/// 配合「每秒結算一次」:銹蝕機(6hp)約 3 秒、乙太靈(4hp)約 2 秒打倒。
const PLAYER_ATTACK_POWER: u32 = 2;

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
            let (daynight_view, growth_rate) = {
                let mut daynight = app.daynight.write().unwrap();
                daynight.advance(dt);
                let view = if want_broadcast {
                    Some(daynight.view())
                } else {
                    None
                };
                (view, daynight.growth_rate())
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
                if want_broadcast {
                    fields
                        .iter()
                        .map(|(owner, field)| {
                            let mut v = field.view();
                            v.owner = *owner;
                            v
                        })
                        .collect()
                } else {
                    Vec::new()
                }
            };

            // 推進採集節點重生（採空的倒數補耐久,其餘 no-op）。重生無條件跑;view 只在廣播時建。
            let node_views: Vec<NodeView> = {
                let mut nodes = app.nodes.write().unwrap();
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

            // 推進敵人重生(被打倒的倒數復活)。重生無條件跑;view 只在廣播時建。
            let enemy_views: Vec<EnemyView> = {
                let mut enemies = app.enemies.write().unwrap();
                enemies.tick(dt);
                if want_broadcast {
                    enemies
                        .enemies()
                        .iter()
                        .map(|p| EnemyView {
                            kind: p.enemy.kind(),
                            x: p.x,
                            y: p.y,
                            hp: p.enemy.remaining_hp(),
                            max_hp: p.enemy.kind().max_hp(),
                            alive: p.enemy.is_alive(),
                        })
                        .collect()
                } else {
                    Vec::new()
                }
            };

            // 戰鬥結算(每秒一次):玩家自動打最近的敵人、敵人反擊。**自動打怪**——不需客戶端輸入。
            // 避免巢狀鎖:先讀玩家位置 → 對敵人結算 → 把戰果(掉落/傷害)套回玩家,三步各持一把鎖。
            if tick % (TICK_HZ as u64) == 0 {
                let positions: Vec<(uuid::Uuid, f32, f32, bool)> = {
                    let players = app.players.read().unwrap();
                    players
                        .values()
                        .map(|p| (p.id, p.x, p.y, p.vitals.is_downed()))
                        .collect()
                };
                let mut loots: Vec<(uuid::Uuid, crate::inventory::ItemKind, u32)> = Vec::new();
                let mut dmgs: Vec<(uuid::Uuid, u32)> = Vec::new();
                {
                    let mut enemies = app.enemies.write().unwrap();
                    for (pid, px, py, downed) in &positions {
                        if *downed {
                            continue; // 被打趴的玩家不攻擊、也不再挨打(休息中)
                        }
                        if let Some((_kind, Some((item, qty)))) =
                            enemies.attack_nearest(*px, *py, PLAYER_ATTACK_POWER)
                        {
                            loots.push((*pid, item, qty)); // 打倒 → 掉落進背包
                        }
                        let threat = enemies.threat_at(*px, *py);
                        if threat > 0 {
                            dmgs.push((*pid, threat)); // 範圍內敵人反擊的威脅總和
                        }
                    }
                }
                if !loots.is_empty() || !dmgs.is_empty() {
                    let mut players = app.players.write().unwrap();
                    for (pid, item, qty) in loots {
                        if let Some(p) = players.get_mut(&pid) {
                            p.inventory.add(item, qty);
                        }
                    }
                    for (pid, dmg) in dmgs {
                        if let Some(p) = players.get_mut(&pid) {
                            if p.vitals.take_damage(dmg) {
                                tracing::info!(player = %p.name, "被敵人打趴,休息復原中");
                            }
                        }
                    }
                }
            }

            // 整合位置 + 推進生命回復（權威模擬,每 tick 必跑,與有無觀眾無關;短暫持鎖,不跨 await）。
            {
                let mut players = app.players.write().unwrap();
                for p in players.values_mut() {
                    p.step(dt);
                    p.vitals.tick(dt); // 離戰一陣子自動回血 / 被打趴的休息倒數
                }
            }

            // 廣播快照——只在有訂閱者時(tick 開頭已判定的 want_broadcast)才建構並序列化。
            // 沒人連線的離峰時段,上面的世界推進照常跑、狀態不停,但連 view 都不建、更不序列化。
            // 新訂閱者本來就是等下一個 tick 才收到第一筆快照,故下一個有人連上的 tick 會立刻送出
            // 完整快照,延遲不變。`daynight_view` 在不廣播時為 None,故此處能直接 unwrap。
            if want_broadcast {
                let snapshot = {
                    let players = app.players.read().unwrap();
                    ServerMsg::Snapshot {
                        tick,
                        players: players.values().map(|p| p.view()).collect(),
                        fields: field_views,
                        nodes: node_views,
                        enemies: enemy_views,
                        daynight: daynight_view.expect("want_broadcast 時必有 daynight_view"),
                    }
                };
                // 沒有訂閱者時 send 會回 Err（這裡已 receiver_count>0,正常路徑），忽略即可。
                if let Ok(json) = serde_json::to_string(&snapshot) {
                    let _ = app.tx.send(json);
                }
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
    let (online, inventories): (
        Vec<OnlinePlayerRow>,
        Vec<(uuid::Uuid, crate::inventory::Inventory)>,
    ) = {
        let players = app.players.read().unwrap();
        let authed: Vec<_> = players
            .values()
            .filter(|p| app.users.get(p.id).is_some())
            .collect();
        (
            authed
                .iter()
                .map(|p| (p.id, p.name.clone(), p.species.clone(), p.x, p.y, p.ether))
                .collect(),
            authed.iter().map(|p| (p.id, p.inventory.clone())).collect(),
        )
    };
    if !online.is_empty() {
        // 先更新行程內 cache（同步,供重連 recall）,再非同步 upsert 到 Postgres。
        app.positions
            .remember_all(online.iter().map(|(id, _, _, x, y, e)| (*id, *x, *y, *e)));
        app.positions.flush_online(&online).await;
        app.inventories.remember_all(inventories.iter().cloned());
        app.inventories.flush_online(&inventories).await;
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
