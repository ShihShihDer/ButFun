//! 野營篝火（ROADMAP 474）——玩家在荒野升起一堆篝火，火光與暖意把附近的野獸逼退，
//! 在危機四伏的野外（尤其入夜後敵人加速追擊時）替自己與同伴圍出一塊喘息的安全角落。
//!
//! 設計取捨（刻意換骨架、不複製既有套路）：
//! - 既有的療癒聚集（圍爐分食 462／林蔭小憩 467／街頭合奏 472）都是「靠近→緩回血」的增益骨架；
//!   篝火**不回血**，而是第一個「玩家主動改變野外危險度」的動詞——把敵人的**追擊意圖**按下去，
//!   是「玩家 × 生態／戰鬥」的新交會（reviewer 多輪點名要的真機制，非裝飾、非顯影既有狀態）。
//! - 民間自古「野獸畏火」：原創實作，不抄任何外部遊戲碼。
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
/// 暖意半徑（像素）——前端畫光圈、後端判定敵人安撫，兩端同口徑。
/// 落在此半徑內的敵人會被「火光逼退」、暫時放棄追擊。
pub const WARMTH_RADIUS: f32 = 210.0;

/// 一堆篝火（純記憶體）。
#[derive(Debug, Clone)]
pub struct Campfire {
    pub id: u32,
    pub wx: f32,
    pub wy: f32,
    /// 剩餘燃燒秒數；歸零即熄滅、自列表移除。
    pub remaining: f32,
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
}
