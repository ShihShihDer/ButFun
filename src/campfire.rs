//! 野營篝火（ROADMAP 474）——玩家在荒野升起一堆篝火，火光與暖意把附近的野獸逼退，
//! 在危機四伏的野外（尤其入夜後敵人加速追擊時）替自己與同伴圍出一塊喘息的安全角落。
//!
//! 設計取捨（刻意換骨架、不複製既有套路）：
//! - 既有的療癒聚集（圍爐分食 462／林蔭小憩 467／街頭合奏 472）都是「靠近→緩回血」的增益骨架；
//!   篝火**不回血**，而是第一個「玩家主動改變野外危險度」的動詞——把敵人的**追擊意圖**按下去，
//!   是「玩家 × 生態／戰鬥」的新交會（reviewer 多輪點名要的真機制，非裝飾、非顯影既有狀態）。
//! - 民間自古「野獸畏火」：原創實作，不抄任何外部遊戲碼。
//!
//! 眾人拾柴（ROADMAP 545）——把篝火從「個人戰術工具」推進到「協作湧現」：圍在同一堆火旁的
//! 玩家越多，火就燒得越旺，暖意逼退半徑階梯式擴大。一個人能護住自己一小圈，一群人湊到一起
//! 卻能撐出一片更大的安全營地——「聚在一起更暖、更安全」這份療癒多人世界該有的體感，第一次
//! 從個體行為長成群體湧現（個體圍爐人數 → 集體火勢 → 共享安全範圍），而非各烤各的火。
//!
//! 成本／安全紀律：
//! - 純記憶體模式，重啟清零。零 migration、零 LLM、零持久化、不碰玩家存檔與經濟。
//! - 純邏輯可獨立測試（升火冷卻、全服上限、燃燒倒數、暖意中心查詢），不依賴 WebSocket／遊戲迴圈。
//! - 平衡分寸：升火有每人冷卻＋全服同時上限，暖意只是一圈「不被追」的局部緩衝、非無敵罩；
//!   火一燒完（CAMPFIRE_BURN_SECS）敵人就回神，玩家踏出暖意圈仍要面對野外，是戰術工具非神器。

use std::collections::HashMap;
use uuid::Uuid;

/// 一堆篝火的燃燒時長（秒）——升起後燒 90 秒，期間暖意生效，燒完自動熄滅。
pub const CAMPFIRE_BURN_SECS: f32 = 90.0;
/// 同一玩家兩次升火的最短間隔（秒）——防止一個人洗版鋪滿篝火。
pub const RELIGHT_COOLDOWN_SECS: f32 = 30.0;
/// 全服同時存在的篝火上限——超過則升火靜默失敗，避免畫面與安撫範圍被洗爆。
pub const MAX_CAMPFIRES: usize = 24;
/// 暖意半徑（像素）——一堆篝火**單獨**燃燒（沒人圍爐或只有一人）時的基礎逼退半徑。
/// 落在此半徑內的敵人會被「火光逼退」、暫時放棄追擊。眾人圍爐時實際半徑由
/// `effective_warmth_radius` 依圍爐人數放大（見下）。
pub const WARMTH_RADIUS: f32 = 210.0;

/// 圍爐半徑（像素）——玩家站在篝火這個距離內才算「圍著這堆火」、替它添旺。
/// 刻意小於 `WARMTH_RADIUS`：要真的湊到火堆邊才算數，路過暖意圈外緣不算圍爐。
pub const GATHER_RADIUS: f32 = 130.0;

/// 眾人拾柴：每多一位圍爐者（第 2 人起）讓暖意半徑增加的比例。
const GATHER_STEP: f32 = 0.15;
/// 眾人拾柴：暖意半徑相對基礎的最大加成上限（防止人海戰術把安全圈撐到無限大）。
/// 0.60＝最多放大到基礎的 1.6 倍；對齊 `GATHER_STEP` 即 5 人（4 位額外圍爐者）封頂。
const GATHER_MAX_BONUS: f32 = 0.60;

/// 純函式：依圍爐人數算一堆篝火的有效暖意半徑（眾人拾柴 ROADMAP 545）。
/// 「眾人拾柴火焰高」——0 或 1 人＝基礎半徑（單獨燃燒不放大）；每多一位圍爐者
/// 半徑階梯式變大，加成封頂在 `GATHER_MAX_BONUS`（5 人封頂）。確定性、無副作用、好測。
pub fn effective_warmth_radius(gather_count: u8) -> f32 {
    // 第 2 人起才放大（首位圍爐者＝基礎安全圈，不加成）。
    let extra = gather_count.saturating_sub(1) as f32;
    let bonus = (extra * GATHER_STEP).min(GATHER_MAX_BONUS);
    WARMTH_RADIUS * (1.0 + bonus)
}

/// 一堆篝火（純記憶體）。
#[derive(Debug, Clone)]
pub struct Campfire {
    pub id: u32,
    pub wx: f32,
    pub wy: f32,
    /// 剩餘燃燒秒數；歸零即熄滅、自列表移除。
    pub remaining: f32,
    /// 本拍圍爐人數（眾人拾柴 545）——由 `sync_warmth` 每拍依玩家座標重算；前端據此把火畫得更旺。
    pub gather_count: u8,
    /// 本拍有效暖意半徑（眾人拾柴 545）——`effective_warmth_radius(gather_count)`，
    /// 由 `sync_warmth` 每拍同步；敵人安撫與前端暖意圈都讀它。預設＝基礎半徑（沒人圍爐時）。
    pub warmth_radius: f32,
}

/// 全服篝火狀態（純記憶體，重啟清零）。
pub struct CampfireField {
    /// 目前所有燃燒中的篝火。
    fires: Vec<Campfire>,
    /// 篝火 id 計數器（遞增，確保 id 不重複）。
    counter: u32,
    /// 每位玩家的升火冷卻剩餘秒數；>0 表示還在冷卻、暫不能再升火。
    cooldowns: HashMap<Uuid, f32>,
}

impl CampfireField {
    pub fn new() -> Self {
        Self {
            fires: Vec::new(),
            counter: 0,
            cooldowns: HashMap::new(),
        }
    }

    /// 目前燃燒中的篝火數量。
    pub fn len(&self) -> usize {
        self.fires.len()
    }

    /// 是否一堆篝火都沒有。
    pub fn is_empty(&self) -> bool {
        self.fires.is_empty()
    }

    /// 目前所有燃燒中的篝火（供快照廣播給前端顯示）。
    pub fn active(&self) -> &[Campfire] {
        &self.fires
    }

    /// 所有篝火的暖意中心座標（供敵人安撫判定使用）。
    pub fn warmth_centers(&self) -> Vec<(f32, f32)> {
        self.fires.iter().map(|c| (c.wx, c.wy)).collect()
    }

    /// 所有篝火的暖意區（中心 X／Y／**有效半徑**）——供敵人安撫判定使用（眾人拾柴 545）。
    /// 有效半徑已含圍爐人數加成（由 `sync_warmth` 每拍同步進 `warmth_radius`）。
    pub fn warmth_zones(&self) -> Vec<(f32, f32, f32)> {
        self.fires.iter().map(|c| (c.wx, c.wy, c.warmth_radius)).collect()
    }

    /// 眾人拾柴（545）：依本拍玩家座標，重算每堆篝火的圍爐人數與有效暖意半徑。
    /// 對每堆火數出 `GATHER_RADIUS` 內的玩家數（壞座標略過），存進 `gather_count`，
    /// 並把 `warmth_radius` 設成 `effective_warmth_radius(人數)`。純邏輯、確定性、好測；
    /// 呼叫端先讀玩家權威座標（讀鎖即放），出鎖後再以篝火寫鎖呼叫此函式（守鎖序不巢狀）。
    pub fn sync_warmth(&mut self, player_positions: &[(f32, f32)]) {
        let r2 = GATHER_RADIUS * GATHER_RADIUS;
        for c in self.fires.iter_mut() {
            let mut count: u8 = 0;
            for &(px, py) in player_positions {
                if !px.is_finite() || !py.is_finite() {
                    continue;
                }
                let dx = px - c.wx;
                let dy = py - c.wy;
                if dx * dx + dy * dy <= r2 {
                    count = count.saturating_add(1);
                }
            }
            c.gather_count = count;
            c.warmth_radius = effective_warmth_radius(count);
        }
    }

    /// 推進時間（`dt` 秒）：每堆篝火燃燒倒數、燒完移除；每位玩家的升火冷卻一併遞減、歸零清除。
    pub fn tick(&mut self, dt: f32) {
        if dt <= 0.0 {
            return;
        }
        for c in self.fires.iter_mut() {
            c.remaining -= dt;
        }
        self.fires.retain(|c| c.remaining > 0.0);
        // 冷卻遞減，歸零者移除（避免 map 無限長大）。
        self.cooldowns.retain(|_, cd| {
            *cd -= dt;
            *cd > 0.0
        });
    }

    /// 嘗試替玩家 `pid` 在其權威座標 `(px, py)` 升起一堆篝火。
    /// 回傳 `Some(id)` 表示成功（`id` = 新篝火編號）；`None` 表示失敗：
    /// 座標非有限值、該玩家仍在升火冷卻中、或全服篝火已達上限。
    /// 純邏輯、確定性；呼叫端負責先讀玩家權威座標（防隔空升火）、出鎖後才廣播。
    pub fn light(&mut self, pid: Uuid, px: f32, py: f32) -> Option<u32> {
        if !px.is_finite() || !py.is_finite() {
            return None;
        }
        // 升火冷卻中。
        if self.cooldowns.get(&pid).is_some_and(|&cd| cd > 0.0) {
            return None;
        }
        // 全服上限。
        if self.fires.len() >= MAX_CAMPFIRES {
            return None;
        }
        let id = self.counter;
        self.counter = self.counter.wrapping_add(1);
        self.fires.push(Campfire {
            id,
            wx: px,
            wy: py,
            remaining: CAMPFIRE_BURN_SECS,
            gather_count: 0,
            warmth_radius: WARMTH_RADIUS, // 剛升起、沒人圍爐＝基礎半徑；下一拍 sync_warmth 重算。
        });
        self.cooldowns.insert(pid, RELIGHT_COOLDOWN_SECS);
        Some(id)
    }
}

impl Default for CampfireField {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn pid(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn new_field_is_empty() {
        let f = CampfireField::new();
        assert!(f.is_empty());
        assert_eq!(f.len(), 0);
        assert!(f.warmth_centers().is_empty());
    }

    #[test]
    fn light_adds_a_campfire() {
        let mut f = CampfireField::new();
        let id = f.light(pid(1), 100.0, 200.0);
        assert!(id.is_some(), "首次升火應成功");
        assert_eq!(f.len(), 1);
        assert_eq!(f.warmth_centers(), vec![(100.0, 200.0)]);
        assert_eq!(f.active()[0].remaining, CAMPFIRE_BURN_SECS);
    }

    #[test]
    fn light_rejects_non_finite_position() {
        let mut f = CampfireField::new();
        assert_eq!(f.light(pid(1), f32::NAN, 0.0), None, "NaN 座標應拒絕");
        assert_eq!(f.light(pid(1), 0.0, f32::INFINITY), None, "Inf 座標應拒絕");
        assert!(f.is_empty(), "非有限座標不應產生篝火");
    }

    #[test]
    fn light_is_rate_limited_per_player() {
        let mut f = CampfireField::new();
        assert!(f.light(pid(1), 0.0, 0.0).is_some(), "首次升火成功");
        assert_eq!(f.light(pid(1), 50.0, 50.0), None, "冷卻中同一人不能再升火");
        assert_eq!(f.len(), 1);
        // 另一位玩家不受別人冷卻影響。
        assert!(f.light(pid(2), 0.0, 0.0).is_some(), "別的玩家可各自升火");
        assert_eq!(f.len(), 2);
    }

    #[test]
    fn cooldown_clears_after_waiting() {
        let mut f = CampfireField::new();
        f.light(pid(1), 0.0, 0.0);
        // 還在冷卻內。
        f.tick(RELIGHT_COOLDOWN_SECS - 1.0);
        assert_eq!(f.light(pid(1), 0.0, 0.0), None, "冷卻未滿不能再升火");
        // 冷卻走完。
        f.tick(2.0);
        assert!(f.light(pid(1), 0.0, 0.0).is_some(), "冷卻走完後可再升火");
    }

    #[test]
    fn campfire_burns_down_and_extinguishes() {
        let mut f = CampfireField::new();
        f.light(pid(1), 0.0, 0.0);
        f.tick(CAMPFIRE_BURN_SECS - 1.0);
        assert_eq!(f.len(), 1, "未燒完前篝火仍在");
        f.tick(2.0);
        assert!(f.is_empty(), "燒完後篝火應自動熄滅移除");
        assert!(f.warmth_centers().is_empty());
    }

    #[test]
    fn global_cap_blocks_extra_campfires() {
        let mut f = CampfireField::new();
        // 用不同玩家避開個人冷卻，填滿到上限。
        for i in 0..MAX_CAMPFIRES {
            assert!(f.light(pid(i as u128), i as f32, 0.0).is_some());
        }
        assert_eq!(f.len(), MAX_CAMPFIRES);
        assert_eq!(
            f.light(pid(9999), 0.0, 0.0),
            None,
            "達全服上限後再升火應失敗"
        );
        assert_eq!(f.len(), MAX_CAMPFIRES);
    }

    #[test]
    fn cap_frees_up_after_a_fire_burns_out() {
        let mut f = CampfireField::new();
        for i in 0..MAX_CAMPFIRES {
            f.light(pid(i as u128), i as f32, 0.0);
        }
        // 全部燒完。
        f.tick(CAMPFIRE_BURN_SECS + 1.0);
        assert!(f.is_empty());
        assert!(
            f.light(pid(1234), 0.0, 0.0).is_some(),
            "篝火燒完空出名額後可再升火"
        );
    }

    #[test]
    fn ids_are_unique() {
        let mut f = CampfireField::new();
        let a = f.light(pid(1), 0.0, 0.0).unwrap();
        let b = f.light(pid(2), 0.0, 0.0).unwrap();
        assert_ne!(a, b, "不同篝火 id 不應重複");
    }

    #[test]
    fn tick_zero_dt_is_noop() {
        let mut f = CampfireField::new();
        f.light(pid(1), 0.0, 0.0);
        let before = f.active()[0].remaining;
        f.tick(0.0);
        f.tick(-5.0);
        assert_eq!(f.active()[0].remaining, before, "壞 dt 不應改變燃燒進度");
    }

    // ── 眾人拾柴（ROADMAP 545）──────────────────────────────────────────────

    #[test]
    fn effective_radius_base_for_zero_or_one() {
        // 0 人（無人圍爐的孤火）與 1 人（首位圍爐者）都＝基礎半徑，不放大。
        assert_eq!(effective_warmth_radius(0), WARMTH_RADIUS);
        assert_eq!(effective_warmth_radius(1), WARMTH_RADIUS);
    }

    #[test]
    fn effective_radius_grows_then_caps() {
        // 第 2 人起階梯式變大、單調遞增。
        let r1 = effective_warmth_radius(1);
        let r2 = effective_warmth_radius(2);
        let r3 = effective_warmth_radius(3);
        assert!(r2 > r1, "2 人應比 1 人暖意圈大");
        assert!(r3 > r2, "3 人應比 2 人更大");
        // 5 人封頂（4 位額外圍爐者 × 0.15 = 0.60 = 上限）。
        let cap = WARMTH_RADIUS * (1.0 + GATHER_MAX_BONUS);
        assert!((effective_warmth_radius(5) - cap).abs() < 1e-3, "5 人應達加成上限");
        // 超過 5 人不再變大（封頂），且不溢位（u8 上限 255 也保守）。
        assert_eq!(effective_warmth_radius(6), cap);
        assert_eq!(effective_warmth_radius(20), cap);
        assert_eq!(effective_warmth_radius(255), cap);
    }

    #[test]
    fn sync_warmth_counts_nearby_players_and_scales() {
        let mut f = CampfireField::new();
        f.light(pid(1), 0.0, 0.0);
        // 三位玩家都在圍爐半徑內 → 火更旺、半徑＝3 人的有效值。
        let near = [(10.0, 0.0), (0.0, 20.0), (-30.0, 30.0)];
        f.sync_warmth(&near);
        assert_eq!(f.active()[0].gather_count, 3, "三位近身玩家都算圍爐");
        assert_eq!(f.active()[0].warmth_radius, effective_warmth_radius(3));
        // warmth_zones 帶出有效半徑供敵人安撫用。
        let z = f.warmth_zones();
        assert_eq!(z.len(), 1);
        assert_eq!(z[0].2, effective_warmth_radius(3));
    }

    #[test]
    fn sync_warmth_ignores_far_and_bad_positions() {
        let mut f = CampfireField::new();
        f.light(pid(1), 0.0, 0.0);
        // 一位在圈內、一位遠在圈外、一位壞座標 → 只算 1 人＝基礎半徑。
        let mixed = [
            (50.0, 0.0),                 // 圈內
            (GATHER_RADIUS + 50.0, 0.0), // 圈外
            (f32::NAN, 0.0),             // 壞座標
        ];
        f.sync_warmth(&mixed);
        assert_eq!(f.active()[0].gather_count, 1, "只有圈內有限座標的玩家算圍爐");
        assert_eq!(f.active()[0].warmth_radius, WARMTH_RADIUS, "1 人＝基礎半徑");
    }

    #[test]
    fn sync_warmth_empty_players_resets_to_base() {
        let mut f = CampfireField::new();
        f.light(pid(1), 0.0, 0.0);
        f.sync_warmth(&[(10.0, 0.0), (20.0, 0.0)]); // 先 2 人撐旺
        assert!(f.active()[0].warmth_radius > WARMTH_RADIUS);
        f.sync_warmth(&[]); // 大家散去 → 回基礎半徑
        assert_eq!(f.active()[0].gather_count, 0);
        assert_eq!(f.active()[0].warmth_radius, WARMTH_RADIUS, "無人圍爐回基礎半徑");
    }
}
