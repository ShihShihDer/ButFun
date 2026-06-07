//! 敵人的世界佈置與自動鎖定（Phase 1-F 戰鬥 MVP「自動打怪」的純邏輯地基之二）。
//!
//! `combat.rs` 解了「一隻敵人怎麼被打、被打倒掉什麼、之後怎麼重生」；接線還缺另一半——
//! 「**敵人擺在世界哪裡、玩家走近時自動鎖定哪一隻**」。本層就是那塊純幾何 + 純互動，
//! 嚴格比照 `gather_field.rs` 之於 `gather.rs`：一組散佈在世界裡的 `PlacedEnemy`
//! （座標 + `Enemy`），加上
//!   - `tick(dt)`：一次推進全部敵人的重生倒數（遊戲迴圈每 tick 呼叫）。
//!   - `attack_nearest(x, y, power)`：玩家在 `(x,y)`，自動鎖定攻擊範圍內**最近**、
//!     仍存活的那隻敵人打一下（這就是「自動打怪」——靠近即出手，客戶端不選目標）。
//!
//! 延續 `gather_field.rs` / `plots.rs` / `field.rs` 的前置慣例：純函式、無 IO、不碰
//! WebSocket / 遊戲迴圈 / 廣播 shape，標 `allow(dead_code)`，接線輪（AppState 持有
//! `EnemyField`、遊戲迴圈 tick、ws 自動攻擊掉落進背包、前端畫敵人 / 戰鬥回饋）才有呼叫端。
//!
//! 佈置刻意做成「比採集節點更外圈的曠野」：世界中央留給家園農莊（`plots.rs`）、
//! 內圈一帶散採集節點（`gather_field.rs`），敵人則散在更外一圈——愈往外走愈有危險，
//! 給「居家種田 → 出門採集 → 深入曠野打怪」漸進的探索節奏。佈置由敵人序號決定
//! （確定性雜湊，不靠亂數 / 時鐘，且雜湊種子與節點不同以免疊在同一點），同一份存檔
//! 重啟後敵人落在同一處。

use crate::combat::{Enemy, EnemyKind};
use crate::inventory::ItemKind;
use crate::state::{WORLD_HEIGHT, WORLD_WIDTH};

/// 散佈在世界裡的敵人總數（兩種輪流分配，故偶數較均衡）。
const ENEMY_COUNT: usize = 12;

/// 世界中央留給家園農莊 + 採集圈的淨空半徑：敵人不會生在這個圈內，
/// 比 `gather_field` 的淨空半徑更大，讓敵人散在採集節點更外一圈。
const CLEARING_RADIUS: f32 = 480.0;

/// 敵人散佈的外圈半徑上限（距世界中心）。再往外會貼到世界邊界，留點邊距。
const SCATTER_OUTER_RADIUS: f32 = 940.0;

/// 敵人距世界邊界至少留這麼多，免得卡在邊上打不到。
const EDGE_MARGIN: f32 = 60.0;

/// 自動攻擊的伸手範圍：玩家走進敵人這個距離內就會自動出手（比採集略大，
/// 戰鬥是「靠近自動打」而非「站到正上方採」）。
pub const ATTACK_REACH: f32 = 64.0;

/// 敵人察覺玩家、開始追擊的半徑：玩家進到這麼近，敵人就盯上並追過來（遠大於 `ATTACK_REACH`，
/// 先發現再追近開打——給「怪會動、會撲上來」的感覺，治「敵人站著不動」）。
pub const AGGRO_RADIUS: f32 = 260.0;

/// 追擊速度（像素/秒）。**刻意低於玩家速度**（`state::PLAYER_SPEED` 320），玩家想跑就拉得開——
/// 這就是 RO 式「拉怪 / 風箏」的空間：站著打會被圍、邊跑邊打能放風箏。
const CHASE_SPEED: f32 = 105.0;

/// 沒有玩家在附近時，敵人緩緩漂回自己的出生點（序號決定的家）。慢於追擊，像悠閒巡邏；
/// 也避免敵人追了一段後散落各地、把中央家園圈擠爆。
const RETURN_SPEED: f32 = 48.0;

/// 世界裡一隻有座標的敵人。
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedEnemy {
    /// 世界座標 X。
    pub x: f32,
    /// 世界座標 Y。
    pub y: f32,
    /// 敵人本身（生命 / 重生狀態）。
    pub enemy: Enemy,
}

/// 散佈在世界裡的一整組敵人。
#[derive(Debug, Clone, PartialEq)]
pub struct EnemyField {
    enemies: Vec<PlacedEnemy>,
}

// 整個模組是前置地基，接線輪才有呼叫端，比照 `gather_field.rs` / `combat.rs` 標 `allow(dead_code)`。
#[allow(dead_code)]
impl EnemyField {
    /// 用確定性佈置生出一組全新（滿血）的敵人。
    pub fn new() -> Self {
        let enemies = (0..ENEMY_COUNT)
            .map(|i| {
                let kind = kind_for(i);
                let (x, y) = scatter_position(i);
                PlacedEnemy {
                    x,
                    y,
                    enemy: Enemy::new(kind),
                }
            })
            .collect();
        Self { enemies }
    }

    /// 目前的敵人（供前端畫、供測試檢視）。
    pub fn enemies(&self) -> &[PlacedEnemy] {
        &self.enemies
    }

    /// 推進 `dt` 秒：對全部敵人呼叫 `tick`（被打倒的倒數重生，其餘 no-op）。
    pub fn tick(&mut self, dt: f32) {
        for placed in &mut self.enemies {
            placed.enemy.tick(dt);
        }
    }

    /// 推進 `dt` 秒的**移動**（與 `tick` 的「生死重生」分工：這層只管位置）。活著的敵人：
    ///   - 有任一玩家進入 `AGGRO_RADIUS` → 追**最近**那位（`CHASE_SPEED`，低於玩家可被拉開）。
    ///   - 附近沒玩家 → 緩緩漂回自己的出生點（`RETURN_SPEED`，序號推導的家），不會越追越散。
    ///
    /// 被打倒（重生中）的敵人不移動。`players` 由接線層先讀好傳入（玩家權威座標的快照），
    /// 避免在持 `EnemyField` 寫鎖時再去鎖玩家表造成巢狀鎖。非有限玩家座標一律略過（延續
    /// `attack_nearest` 的載入防線脈絡）；非正 `dt` 為 no-op（比照 `Enemy::tick`）。
    ///
    /// 接線層每 tick 緊接 `tick(dt)` 之後呼叫，世界因此「活起來」：怪會巡邏、會撲上來，
    /// 戰鬥不再是站樁收割，而有了走位、拉怪、被圍的張力。
    pub fn advance(&mut self, dt: f32, players: &[(f32, f32)]) {
        if dt <= 0.0 {
            return;
        }
        let aggro_sq = AGGRO_RADIUS * AGGRO_RADIUS;
        for (i, placed) in self.enemies.iter_mut().enumerate() {
            // 重生中的敵人不動（牠此刻不在場）。
            if !placed.enemy.is_alive() {
                continue;
            }
            // 找最近、座標有限的玩家（接線層只會傳沒被打趴的玩家）。
            let mut nearest: Option<(f32, f32, f32)> = None; // (x, y, dist_sq)
            for &(tx, ty) in players {
                if !tx.is_finite() || !ty.is_finite() {
                    continue;
                }
                let dx = tx - placed.x;
                let dy = ty - placed.y;
                let d2 = dx * dx + dy * dy;
                if d2 <= aggro_sq && nearest.is_none_or(|(_, _, b)| d2 < b) {
                    nearest = Some((tx, ty, d2));
                }
            }
            // 有目標就追玩家、沒有就漂回家。
            let (target_x, target_y, speed) = match nearest {
                Some((tx, ty, _)) => (tx, ty, CHASE_SPEED),
                None => {
                    let (hx, hy) = scatter_position(i);
                    (hx, hy, RETURN_SPEED)
                }
            };
            let dx = target_x - placed.x;
            let dy = target_y - placed.y;
            let dist = (dx * dx + dy * dy).sqrt();
            // 已經貼著目標就別抖動；移動量夾在剩餘距離內（不衝過頭、不繞著目標打轉）。
            if dist > 2.0 {
                let step = (speed * dt).min(dist);
                placed.x += dx / dist * step;
                placed.y += dy / dist * step;
            }
            // 夾進世界邊界內（同佈置的邊距），永遠不會被推出世界。
            placed.x = placed.x.clamp(EDGE_MARGIN, WORLD_WIDTH - EDGE_MARGIN);
            placed.y = placed.y.clamp(EDGE_MARGIN, WORLD_HEIGHT - EDGE_MARGIN);
        }
    }

    /// 玩家在 `(px, py)` 自動攻擊：在 `ATTACK_REACH` 內、仍存活的敵人中鎖定**最近**的打一下。
    ///
    /// 回傳語意（沿用 `Enemy::attack` 的「掉落只在致命那下」）：
    ///   - `None`：範圍內沒有可打的存活敵人（不出手）。
    ///   - `Some((kind, None))`：打中了（種類供前端做受擊回饋）、但這下不致命、沒掉落。
    ///   - `Some((kind, Some(loot)))`：這下打倒了，回傳一次性掉落 `(物品, 數量)`。
    ///
    /// `power` 由接線層決定（將來武器 / 戰鬥技能讓每下傷害更高，比照採集的工具倍率）。
    /// 由伺服器權威判定鎖定哪隻、是否打到，客戶端只送「我在攻擊」的意圖。
    pub fn attack_nearest(
        &mut self,
        px: f32,
        py: f32,
        power: u32,
    ) -> Option<(EnemyKind, Option<(ItemKind, u32)>)> {
        // 非有限座標一律視為打不到（延續 `gather_near` / `cell_at` 的載入防線脈絡）。
        if !px.is_finite() || !py.is_finite() {
            return None;
        }
        let reach_sq = ATTACK_REACH * ATTACK_REACH;
        let mut best: Option<(usize, f32)> = None;
        for (i, placed) in self.enemies.iter().enumerate() {
            // 只鎖定還活著的敵人；重生中的略過。
            if !placed.enemy.is_alive() {
                continue;
            }
            let dx = placed.x - px;
            let dy = placed.y - py;
            let dist_sq = dx * dx + dy * dy;
            if dist_sq <= reach_sq && best.is_none_or(|(_, b)| dist_sq < b) {
                best = Some((i, dist_sq));
            }
        }
        let (idx, _) = best?;
        let kind = self.enemies[idx].enemy.kind();
        let loot = self.enemies[idx].enemy.attack(power);
        Some((kind, loot))
    }

    /// 玩家在 `(px, py)` 這一刻**承受的反擊總威脅**：所有在 `ATTACK_REACH` 內、仍存活的
    /// 敵人，各自的 `threat`（每次反擊傷害）相加。`attack_nearest` 是玩家→敵人這一向，
    /// 本函式是對等的敵人→玩家那一向——湊成完整的「自動打怪」迴圈：靠近自動出手的同時，
    /// 圍上來的敵人也在還手。被多隻敵人同時包圍時威脅疊加（愈深入曠野、敵人愈密就愈危險），
    /// 給戰鬥真正的風險面，避免淪為「無傷收割」的空殼（正是 `vitals.rs` 承接 `threat` 要的對象）。
    ///
    /// 威脅範圍刻意**重用 `ATTACK_REACH`**（單一真實來源）：你近到能自動打牠，牠就近到能還手，
    /// 不另立一套距離常數。純查詢——**不改任何敵人狀態**（反擊不消耗敵人，敵人照樣被
    /// `attack_nearest` 打），故吃 `&self`。反擊的**節奏**（接線層多久把這份威脅套進玩家
    /// `Vitals::take_damage` 一次）由伺服器迴圈決定——比照 `attack_nearest` 的 `power`、
    /// `tick` 的 `dt`，cadence 是接線層的權責，這層只回「此刻圍著你的敵人有多危險」。
    /// 非有限座標一律回 `0`（延續 `attack_nearest` 的載入防線脈絡）。
    pub fn threat_at(&self, px: f32, py: f32) -> u32 {
        if !px.is_finite() || !py.is_finite() {
            return 0;
        }
        let reach_sq = ATTACK_REACH * ATTACK_REACH;
        self.enemies
            .iter()
            // 只有活著的敵人會還手；重生中的不構成威脅。
            .filter(|placed| placed.enemy.is_alive())
            .filter(|placed| {
                let dx = placed.x - px;
                let dy = placed.y - py;
                dx * dx + dy * dy <= reach_sq
            })
            .map(|placed| placed.enemy.kind().threat())
            .sum()
    }

    /// 載入入口（接 0-E 從存檔還原敵人狀態用）：佈置（座標）一律由序號重新推導，
    /// 只有「生命 / 重生倒數」這組會變的狀態取自存檔。延續 `gather_field::from_saved`
    /// 的載入時驗證——存檔敵人數必須與目前佈置一致、種類對齊序號、且每隻都 `is_loadable`，
    /// 否則整組拒收回 `None`，呼叫端退回 `EnemyField::new()`（全新一組）。
    ///
    /// 敵人**執行期會移動**（`advance` 的巡邏 / 追擊），但移動後的位置**不入存檔**：座標一律
    /// 由序號重推（重啟後敵人回到出生點重新巡邏）。敵人不持久化位置（`EnemyField` 本就每次啟動
    /// 重新撒佈），故比照 `gather_field` 只信存檔的生命 / 重生狀態、座標一律重算。
    pub fn from_saved(saved: Vec<Enemy>) -> Option<Self> {
        if saved.len() != ENEMY_COUNT {
            return None;
        }
        let mut enemies = Vec::with_capacity(ENEMY_COUNT);
        for (i, enemy) in saved.into_iter().enumerate() {
            // 種類也得對齊佈置：存檔種類若和序號推導的不符，視為壞檔。
            if enemy.kind() != kind_for(i) || !enemy.is_loadable() {
                return None;
            }
            let (x, y) = scatter_position(i);
            enemies.push(PlacedEnemy { x, y, enemy });
        }
        Some(Self { enemies })
    }
}

impl Default for EnemyField {
    fn default() -> Self {
        Self::new()
    }
}

/// 第 `i` 隻敵人的種類：兩種輪流分配，數量大致均衡。
fn kind_for(i: usize) -> EnemyKind {
    match i % 2 {
        0 => EnemyKind::ScrapDrone,
        _ => EnemyKind::EtherWisp,
    }
}

/// 第 `i` 隻敵人的世界座標：以序號雜湊出一個極座標（半徑、角度），落在比採集節點更外
/// 一圈的曠野裡，再夾進世界邊界內。確定性（同序號永遠同位置）、不靠亂數 / 時鐘，故重啟
/// 後敵人落在同一處。雜湊種子刻意與 `gather_field` 不同，避免敵人剛好疊在採集節點上。
fn scatter_position(i: usize) -> (f32, f32) {
    let cx = WORLD_WIDTH / 2.0;
    let cy = WORLD_HEIGHT / 2.0;
    // 兩個獨立的雜湊流：一個決定半徑、一個決定角度（種子加偏移與節點區隔）。
    let r = CLEARING_RADIUS + hash01((i as u64).wrapping_add(0x5151)) * (SCATTER_OUTER_RADIUS - CLEARING_RADIUS);
    let theta =
        hash01((i as u64).wrapping_mul(2).wrapping_add(0x9090)) * std::f32::consts::TAU;
    let x = (cx + r * theta.cos()).clamp(EDGE_MARGIN, WORLD_WIDTH - EDGE_MARGIN);
    let y = (cy + r * theta.sin()).clamp(EDGE_MARGIN, WORLD_HEIGHT - EDGE_MARGIN);
    (x, y)
}

/// 確定性雜湊：把序號攪成 `[0, 1)` 的浮點（splitmix64 風格），佈置用。
/// 不引入亂數相依、不碰時鐘，純函式，可重現（比照 `gather_field::hash01`）。
fn hash01(n: u64) -> f32 {
    let mut z = n.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    // 取高 53 位映到 [0,1)，與 f64 尾數對齊避免取捨偏差。
    (z >> 11) as f32 / (1u64 << 53) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_field_has_full_count_all_alive() {
        let f = EnemyField::new();
        assert_eq!(f.enemies().len(), ENEMY_COUNT);
        assert!(f.enemies().iter().all(|p| p.enemy.is_alive()));
    }

    #[test]
    fn placement_is_deterministic() {
        // 兩次建構座標完全一致（確定性，不靠亂數 / 時鐘）。
        let a = EnemyField::new();
        let b = EnemyField::new();
        assert_eq!(a, b);
    }

    #[test]
    fn enemies_avoid_central_clearing_and_stay_in_world() {
        let cx = WORLD_WIDTH / 2.0;
        let cy = WORLD_HEIGHT / 2.0;
        for p in EnemyField::new().enemies() {
            // 在世界內、留邊距。
            assert!((EDGE_MARGIN..=WORLD_WIDTH - EDGE_MARGIN).contains(&p.x));
            assert!((EDGE_MARGIN..=WORLD_HEIGHT - EDGE_MARGIN).contains(&p.y));
            // 在家園 + 採集淨空圈外（夾邊只會往內拉，仍應遠離正中心）。
            let dx = p.x - cx;
            let dy = p.y - cy;
            let dist = (dx * dx + dy * dy).sqrt();
            assert!(dist > CLEARING_RADIUS / 2.0, "敵人離中心太近: {dist}");
        }
    }

    #[test]
    fn both_kinds_present() {
        let f = EnemyField::new();
        let kinds: Vec<EnemyKind> = f.enemies().iter().map(|p| p.enemy.kind()).collect();
        assert!(kinds.contains(&EnemyKind::ScrapDrone));
        assert!(kinds.contains(&EnemyKind::EtherWisp));
    }

    #[test]
    fn attack_nearest_hits_enemy_in_reach() {
        let mut f = EnemyField::new();
        let target = f.enemies()[0].clone();
        let before = target.enemy.remaining_hp();
        // 站在第一隻敵人正上方自動攻擊。
        let got = f.attack_nearest(target.x, target.y, 1);
        assert!(got.is_some());
        let (kind, loot) = got.unwrap();
        assert_eq!(kind, target.enemy.kind());
        // 一下打不死（max_hp >= 4 > 1），不掉落但扣了血。
        assert_eq!(loot, None);
        assert_eq!(f.enemies()[0].enemy.remaining_hp(), before - 1);
    }

    #[test]
    fn attack_nearest_returns_none_when_out_of_reach() {
        let mut f = EnemyField::new();
        // 一定在世界外的座標確保 None（鎖不到任何敵人）。
        assert!(f
            .attack_nearest(WORLD_WIDTH + 5000.0, WORLD_HEIGHT + 5000.0, 1)
            .is_none());
    }

    #[test]
    fn attack_nearest_rejects_non_finite_coords() {
        let mut f = EnemyField::new();
        assert!(f.attack_nearest(f32::NAN, 100.0, 1).is_none());
        assert!(f.attack_nearest(100.0, f32::INFINITY, 1).is_none());
    }

    #[test]
    fn killing_blow_returns_loot_then_respawns_via_tick() {
        let mut f = EnemyField::new();
        let target = f.enemies()[0].clone();
        let kind = target.enemy.kind();
        // 一口氣打掉滿血：致命那下回傳掉落。
        let got = f.attack_nearest(target.x, target.y, kind.max_hp());
        assert_eq!(got, Some((kind, Some(kind.drop_loot()))));
        assert!(f.enemies()[0].enemy.is_defeated());
        // 重生中再站原地攻擊：鎖不到（該隻被略過、附近無其他存活敵人時回 None）。
        // 至少這隻已被打倒。
        assert!(!f.enemies()[0].enemy.is_alive());
        // 推進到重生時間，滿血復活、再次可打。
        f.tick(kind.respawn_secs());
        assert!(f.enemies()[0].enemy.is_alive());
        assert_eq!(f.enemies()[0].enemy.remaining_hp(), kind.max_hp());
    }

    #[test]
    fn defeated_enemy_is_not_retargeted() {
        let mut f = EnemyField::new();
        let target = f.enemies()[0].clone();
        let kind = target.enemy.kind();
        // 把第一隻打倒。
        assert!(f.attack_nearest(target.x, target.y, kind.max_hp()).is_some());
        // 站在牠原位再攻擊：牠已被打倒，不會被重複鎖定掉落。
        // 若附近剛好有別的存活敵人會打到別隻，但無論如何不會再從這隻拿到掉落。
        if let Some((hit_kind, loot)) = f.attack_nearest(target.x, target.y, kind.max_hp()) {
            // 打到的一定是別隻（仍存活），不是被打倒的那隻 idx 0。
            assert!(f.enemies()[0].enemy.is_defeated());
            let _ = (hit_kind, loot);
        }
        // 被打倒的那隻仍維持被打倒（沒有被重複攻擊復活或多掉）。
        assert!(f.enemies()[0].enemy.is_defeated());
    }

    #[test]
    fn threat_at_is_zero_when_no_enemy_in_range() {
        // 真實佈置、站在世界外：附近沒有任何敵人，反擊威脅為 0。
        let f = EnemyField::new();
        assert_eq!(f.threat_at(WORLD_WIDTH + 5000.0, WORLD_HEIGHT + 5000.0), 0);
    }

    #[test]
    fn threat_at_sums_alive_enemies_in_reach() {
        // 兩隻不同種類的存活敵人都落在攻擊範圍內：威脅相加（被包圍更危險）。
        let f = EnemyField {
            enemies: vec![
                PlacedEnemy {
                    x: 100.0,
                    y: 100.0,
                    enemy: Enemy::new(EnemyKind::ScrapDrone),
                },
                PlacedEnemy {
                    x: 140.0,
                    y: 100.0,
                    enemy: Enemy::new(EnemyKind::EtherWisp),
                },
            ],
        };
        // 站在兩隻中間（各距 20 < ATTACK_REACH 64）：兩隻都還手。
        let expected = EnemyKind::ScrapDrone.threat() + EnemyKind::EtherWisp.threat();
        assert_eq!(f.threat_at(120.0, 100.0), expected);
    }

    #[test]
    fn threat_at_excludes_enemies_out_of_reach() {
        let f = EnemyField {
            enemies: vec![PlacedEnemy {
                x: 100.0,
                y: 100.0,
                enemy: Enemy::new(EnemyKind::ScrapDrone),
            }],
        };
        // 玩家在攻擊範圍外（距 65 > 64）：威脅為 0。
        assert_eq!(f.threat_at(100.0, 100.0 + ATTACK_REACH + 1.0), 0);
    }

    #[test]
    fn threat_at_ignores_defeated_enemies() {
        // 一隻存活、一隻被打倒，都在範圍內：只算存活那隻的威脅（倒下的不還手）。
        let mut defeated = Enemy::new(EnemyKind::EtherWisp);
        defeated.attack(EnemyKind::EtherWisp.max_hp());
        assert!(defeated.is_defeated());
        let f = EnemyField {
            enemies: vec![
                PlacedEnemy {
                    x: 100.0,
                    y: 100.0,
                    enemy: Enemy::new(EnemyKind::ScrapDrone),
                },
                PlacedEnemy {
                    x: 105.0,
                    y: 100.0,
                    enemy: defeated,
                },
            ],
        };
        assert_eq!(f.threat_at(102.0, 100.0), EnemyKind::ScrapDrone.threat());
    }

    #[test]
    fn threat_at_rejects_non_finite_coords() {
        let f = EnemyField::new();
        assert_eq!(f.threat_at(f32::NAN, 100.0), 0);
        assert_eq!(f.threat_at(100.0, f32::INFINITY), 0);
    }

    #[test]
    fn from_saved_round_trips_and_validates() {
        let f = EnemyField::new();
        let saved: Vec<Enemy> = f.enemies().iter().map(|p| p.enemy.clone()).collect();
        let restored = EnemyField::from_saved(saved).expect("正常存檔該還原");
        assert_eq!(restored, f);
    }

    #[test]
    fn from_saved_rejects_wrong_count() {
        assert!(EnemyField::from_saved(vec![]).is_none());
        let too_few: Vec<Enemy> =
            (0..ENEMY_COUNT - 1).map(|i| Enemy::new(kind_for(i))).collect();
        assert!(EnemyField::from_saved(too_few).is_none());
    }

    #[test]
    fn from_saved_rejects_corrupt_or_mismatched_kind() {
        // 種類和序號推導不符 → 壞檔。
        let mut wrong_kind: Vec<Enemy> =
            (0..ENEMY_COUNT).map(|i| Enemy::new(kind_for(i))).collect();
        // 把序號 0（應為 ScrapDrone）換成別種。
        wrong_kind[0] = Enemy::new(EnemyKind::EtherWisp);
        assert!(EnemyField::from_saved(wrong_kind).is_none());

        // 含壞值（生命超上限）的存檔 → 拒收。
        let mut corrupt: Vec<Enemy> =
            (0..ENEMY_COUNT).map(|i| Enemy::new(kind_for(i))).collect();
        corrupt[1] = Enemy::from_raw(kind_for(1), 999, 0.0);
        assert!(EnemyField::from_saved(corrupt).is_none());
    }

    // ── Phase 1 完整戰鬥迴圈（field 級）的跨模組組合測試 ─────────────────────
    // `combat`/`enemy_field`/`vitals` 三塊純邏輯各自單元測試都很扎實，`vitals.rs` 也有
    // 「單一 `EnemyKind::threat` 餵進 `take_damage`」的組合測試——但**整個戰鬥迴圈在
    // field 層接起來**的這道接縫此前零測試保證：玩家站在一群敵人中，每 tick 承受
    // `threat_at` 聚合反擊扣血→被打趴，同時靠 `attack_nearest` 反過來打倒敵人、減少自己
    // 承受的威脅。接線層（backend ws / 遊戲迴圈）正是要把這兩向串起來，bug 就藏在接縫。
    // 不疊第 N 個沒人呼叫的死碼，改補上證明這幾塊地基真的組合成完整迴圈的組合測試——
    // 任一邊契約日後漂移（threat_at 聚合語意 / take_damage 致命判定 / attack_nearest
    // 鎖定致命掉落）都會在此整條斷掉，而非等上線才在 ws 裡爆。

    use crate::vitals::Vitals;

    #[test]
    fn aggregated_field_threat_downs_player_and_beats_a_single_enemy() {
        // 玩家被兩隻不同種類的敵人包圍（都在 ATTACK_REACH 內）。
        let field = EnemyField {
            enemies: vec![
                PlacedEnemy {
                    x: 100.0,
                    y: 100.0,
                    enemy: Enemy::new(EnemyKind::ScrapDrone),
                },
                PlacedEnemy {
                    x: 140.0,
                    y: 100.0,
                    enemy: Enemy::new(EnemyKind::EtherWisp),
                },
            ],
        };
        // 站在兩隻中間（各距 20 < ATTACK_REACH 64）：兩隻都還手。
        let (px, py) = (120.0, 100.0);
        let per_tick = field.threat_at(px, py);
        assert!(per_tick > 0, "被包圍時每次聚合反擊應有正威脅");

        // 反覆把當下聚合威脅餵進玩家生命值：有限次內被打趴——戰鬥確實有風險、
        // 不是無傷收割（正是 `threat_at`／`vitals` 一起要證明的設計）。
        let mut v = Vitals::new();
        let mut blows = 0;
        while v.is_alive() {
            v.take_damage(field.threat_at(px, py));
            blows += 1;
            assert!(blows < 1000, "正聚合威脅應在有限次內把玩家打趴");
        }
        assert!(v.is_downed());

        // 聚合威脅（兩隻相加）嚴格大於任一單隻：被一群包圍確實比遇到一隻更危險。
        let strongest_single = EnemyKind::ScrapDrone
            .threat()
            .max(EnemyKind::EtherWisp.threat());
        assert!(
            per_tick > strongest_single,
            "兩隻包圍的聚合威脅應大於任一單隻"
        );
    }

    #[test]
    fn fighting_back_reduces_incoming_threat() {
        // 雙向迴圈的另一半：玩家不只挨打，反過來打倒敵人能減少自己承受的威脅。
        // A（ScrapDrone，威脅較高）離玩家更近、B（EtherWisp）較遠但仍在範圍內。
        let mut field = EnemyField {
            enemies: vec![
                PlacedEnemy {
                    x: 105.0,
                    y: 100.0,
                    enemy: Enemy::new(EnemyKind::ScrapDrone),
                },
                PlacedEnemy {
                    x: 130.0,
                    y: 100.0,
                    enemy: Enemy::new(EnemyKind::EtherWisp),
                },
            ],
        };
        let (px, py) = (100.0, 100.0);
        // 一開始兩隻都還手：威脅是兩者相加。
        let before = field.threat_at(px, py);
        assert_eq!(
            before,
            EnemyKind::ScrapDrone.threat() + EnemyKind::EtherWisp.threat()
        );

        // 自動鎖定最近那隻（A）一口氣打倒：致命那下回傳掉落。
        let got = field.attack_nearest(px, py, EnemyKind::ScrapDrone.max_hp());
        assert_eq!(
            got,
            Some((EnemyKind::ScrapDrone, Some(EnemyKind::ScrapDrone.drop_loot())))
        );

        // 打倒一隻後，承受的威脅隨之下降——只剩仍存活的 B 在還手。
        let after = field.threat_at(px, py);
        assert!(after < before, "打倒敵人後承受的威脅應下降");
        assert_eq!(after, EnemyKind::EtherWisp.threat());
    }

    // ── Phase 1 戰鬥「敵人會動」（巡邏 / 追擊）的移動測試 ──────────────────────
    // `advance` 是讓世界活起來的那一步：怪不再站樁，會撲向走近的玩家、沒人時漂回家。
    // 下面鎖住它的核心契約——追擊朝玩家、閒時回家、倒下不動、永不出界、追速可被拉開。

    #[test]
    fn aggro_enemy_moves_toward_nearby_player() {
        // 一隻活敵人 + 一位玩家在 AGGRO_RADIUS 內：推進後敵人離玩家更近（追上去了）。
        let mut field = EnemyField {
            enemies: vec![PlacedEnemy {
                x: 1000.0,
                y: 1000.0,
                enemy: Enemy::new(EnemyKind::ScrapDrone),
            }],
        };
        let player = (1000.0, 1000.0 + AGGRO_RADIUS - 20.0); // 在察覺範圍內
        let dist_to_player = |e: &PlacedEnemy| {
            let dx = player.0 - e.x;
            let dy = player.1 - e.y;
            (dx * dx + dy * dy).sqrt()
        };
        let before = dist_to_player(&field.enemies()[0]);
        field.advance(0.5, &[player]);
        let after = dist_to_player(&field.enemies()[0]);
        assert!(after < before, "敵人應朝玩家移動、距離縮短：{before} → {after}");
    }

    #[test]
    fn enemy_outside_aggro_drifts_back_home() {
        // 沒有玩家在附近：敵人朝出生點（序號 0 的家）漂回去。
        let (hx, hy) = scatter_position(0);
        let start_x = (hx + 200.0).min(WORLD_WIDTH - EDGE_MARGIN);
        let mut field = EnemyField {
            enemies: vec![PlacedEnemy {
                x: start_x,
                y: hy,
                enemy: Enemy::new(kind_for(0)),
            }],
        };
        let home_dist = |e: &PlacedEnemy| {
            let dx = hx - e.x;
            let dy = hy - e.y;
            (dx * dx + dy * dy).sqrt()
        };
        let before = home_dist(&field.enemies()[0]);
        field.advance(1.0, &[]); // 沒有任何玩家
        let after = home_dist(&field.enemies()[0]);
        assert!(after < before, "閒置敵人應漂回家、離家更近：{before} → {after}");
    }

    #[test]
    fn defeated_enemy_does_not_move() {
        // 重生中的敵人不在場，不該被移動（就算玩家貼著牠）。
        let mut enemy = Enemy::new(EnemyKind::EtherWisp);
        enemy.attack(EnemyKind::EtherWisp.max_hp()); // 打倒
        assert!(!enemy.is_alive());
        let mut field = EnemyField {
            enemies: vec![PlacedEnemy {
                x: 500.0,
                y: 500.0,
                enemy,
            }],
        };
        field.advance(1.0, &[(505.0, 500.0)]); // 玩家就在旁邊
        assert_eq!(field.enemies()[0].x, 500.0);
        assert_eq!(field.enemies()[0].y, 500.0);
    }

    #[test]
    fn advance_keeps_enemies_in_world() {
        // 真實佈置、玩家站在世界角落狂拉，多步推進後所有敵人仍在世界邊界內（clamp 生效）。
        let mut field = EnemyField::new();
        for _ in 0..2000 {
            field.advance(1.0 / 15.0, &[(EDGE_MARGIN, EDGE_MARGIN)]);
        }
        for p in field.enemies() {
            assert!((EDGE_MARGIN..=WORLD_WIDTH - EDGE_MARGIN).contains(&p.x));
            assert!((EDGE_MARGIN..=WORLD_HEIGHT - EDGE_MARGIN).contains(&p.y));
        }
    }

    #[test]
    fn non_positive_dt_does_not_move() {
        let mut field = EnemyField {
            enemies: vec![PlacedEnemy {
                x: 300.0,
                y: 300.0,
                enemy: Enemy::new(EnemyKind::ScrapDrone),
            }],
        };
        field.advance(0.0, &[(310.0, 300.0)]);
        field.advance(-1.0, &[(310.0, 300.0)]);
        assert_eq!(field.enemies()[0].x, 300.0);
        assert_eq!(field.enemies()[0].y, 300.0);
    }

    #[test]
    #[allow(clippy::assertions_on_constants)] // 刻意斷言兩個調校常數的關係不變式
    fn chase_speed_is_slower_than_player_so_kiting_works() {
        // 設計不變式：追擊速度必須低於玩家速度，否則玩家永遠拉不開、無法風箏——
        // 戰鬥就退化成「被黏死」而非「走位拉怪」。日後誰把追速調得 >= 玩家速度會在此紅燈。
        assert!(
            CHASE_SPEED < crate::state::PLAYER_SPEED,
            "追擊速度（{CHASE_SPEED}）應低於玩家速度（{}），否則拉不開怪",
            crate::state::PLAYER_SPEED
        );
    }
}
