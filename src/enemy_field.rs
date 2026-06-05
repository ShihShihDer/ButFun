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

    /// 載入入口（接 0-E 從存檔還原敵人狀態用）：佈置（座標）一律由序號重新推導，
    /// 只有「生命 / 重生倒數」這組會變的狀態取自存檔。延續 `gather_field::from_saved`
    /// 的載入時驗證——存檔敵人數必須與目前佈置一致、種類對齊序號、且每隻都 `is_loadable`，
    /// 否則整組拒收回 `None`，呼叫端退回 `EnemyField::new()`（全新一組）。
    ///
    /// 敵人不像載具會被移動（位置固定可由序號重算），故比照 `gather_field` 只信存檔的
    /// 生命 / 重生狀態、座標一律重推，不像 `vehicle_field` 要信存檔位置。
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
}
