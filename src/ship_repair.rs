//! 廢棄蒸汽星艦共修（ROADMAP 492）。
//!
//! 世界東北方矗立著一艘墜落的蒸汽龐克星艦——鏽蝕的齒輪、破損的蒸汽管道、斜插進地面的
//! 流線形艦身。多位旅人可以走近貢獻木材（×2）協力修繕；總貢獻量達到 `REPAIR_GOAL` 時
//! 星艦短暫復甦、發出金色齒輪光，全服公告「星艦再度飛翔」。
//!
//! 成本紀律：純記憶體、重啟清零、零 migration、零 LLM、零持久化、不動任何玩家存檔。

use std::collections::HashMap;
use uuid::Uuid;

/// 星艦的世界座標 X（主城東北方空曠地帶）。
pub const SHIP_WX: f32 = 3200.0;
/// 星艦的世界座標 Y。
pub const SHIP_WY: f32 = 1800.0;
/// 修繕互動半徑（像素）——玩家需在此範圍內才能貢獻。
pub const REPAIR_REACH: f32 = 150.0;
/// 完成修繕所需的總貢獻次數（每次消耗 2 Wood）。
pub const REPAIR_GOAL: u32 = 20;
/// 修繕完成後「閃耀飛翔」持續秒數（10 分鐘）。
pub const REPAIRED_SECS: f32 = 600.0;
/// 修繕完成後重新接受貢獻前的冷卻秒數（1 分鐘讓玩家欣賞）。
pub const COOLDOWN_AFTER_REPAIR_SECS: f32 = 60.0;
/// 同一玩家連續兩次貢獻的最短間隔（秒）——防止快速洗材。
pub const CONTRIB_COOLDOWN_SECS: f32 = 5.0;
/// 每次貢獻消耗的木材數量。
pub const COST_WOOD: u32 = 2;

/// 每次 `contribute()` 的回傳結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContributeOutcome {
    /// 貢獻成功，更新後的進度（< `REPAIR_GOAL`）。
    Progress(u32),
    /// 此次貢獻恰好達到 `REPAIR_GOAL`——修繕完成，呼叫端廣播全服公告。
    Repaired,
}

/// 星艦 tick 後的週期事件（供 game.rs 決定是否廣播）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepairTick {
    /// 本 tick 無事。
    None,
    /// 冷卻結束，星艦再次損毀、準備好下輪修繕。
    BrokeDown,
}

/// 星艦目前的狀態。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShipStatus {
    /// 損毀中，等待旅人共修。
    Broken,
    /// 修繕完成，發光飛翔中。
    Repaired,
    /// 修繕後冷卻期（光芒消退、等待計時結束後再回 Broken）。
    Cooldown,
}

/// 快照視圖（送給前端）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShipView {
    /// 目前修繕進度（Broken 狀態時有效；其他狀態為 0）。
    pub progress: u32,
    /// 修繕目標（與 `REPAIR_GOAL` 相同，供前端直接計算百分比）。
    pub goal: u32,
    /// 閃耀剩餘秒數（Repaired 時 > 0；其他狀態為 0）。
    pub repaired_secs: u32,
}

/// 全服星艦修繕狀態（純記憶體，重啟清零）。
pub struct ShipRepairState {
    pub status: ShipStatus,
    /// 目前修繕進度（0..=REPAIR_GOAL）；Broken 以外的狀態皆為 0。
    pub progress: u32,
    /// 計時器（秒）：Repaired→剩餘閃耀秒數；Cooldown→剩餘冷卻秒數；Broken→0。
    pub timer: f32,
    /// 每位玩家的貢獻冷卻剩餘秒數（> 0 表示還在冷卻）。
    contrib_cooldowns: HashMap<Uuid, f32>,
}

impl ShipRepairState {
    pub fn new() -> Self {
        Self {
            status: ShipStatus::Broken,
            progress: 0,
            timer: 0.0,
            contrib_cooldowns: HashMap::new(),
        }
    }

    /// 星艦是否處於損毀（可接受貢獻）狀態。
    pub fn is_broken(&self) -> bool {
        self.status == ShipStatus::Broken
    }

    /// 星艦是否正在閃耀（修繕完成後的飛翔期）。
    pub fn is_repaired(&self) -> bool {
        self.status == ShipStatus::Repaired
    }

    /// 建構快照視圖（每幀廣播用）。
    pub fn view(&self) -> ShipView {
        ShipView {
            progress: if self.is_broken() { self.progress } else { 0 },
            goal: REPAIR_GOAL,
            repaired_secs: if self.is_repaired() {
                self.timer.ceil().max(0.0) as u32
            } else {
                0
            },
        }
    }

    /// 推進時間（`dt` 秒）。回傳 `RepairTick::BrokeDown` 表示冷卻結束、星艦再次損毀。
    pub fn tick(&mut self, dt: f32) -> RepairTick {
        if dt <= 0.0 {
            return RepairTick::None;
        }
        // 每人貢獻冷卻遞減，歸零後移除。
        self.contrib_cooldowns.retain(|_, cd| {
            *cd -= dt;
            *cd > 0.0
        });
        match self.status {
            ShipStatus::Broken => RepairTick::None,
            ShipStatus::Repaired => {
                self.timer -= dt;
                if self.timer <= 0.0 {
                    self.status = ShipStatus::Cooldown;
                    self.timer = COOLDOWN_AFTER_REPAIR_SECS;
                }
                RepairTick::None
            }
            ShipStatus::Cooldown => {
                self.timer -= dt;
                if self.timer <= 0.0 {
                    self.status = ShipStatus::Broken;
                    self.timer = 0.0;
                    return RepairTick::BrokeDown;
                }
                RepairTick::None
            }
        }
    }

    /// 嘗試為玩家 `pid`（位於 `(px, py)`）貢獻木材修繕。
    /// 回傳 `Some(ContributeOutcome)` 表示貢獻成功；
    /// `None` 表示被拒：星艦未損毀 / 玩家在冷卻中 / 座標無效 / 超出修繕半徑。
    /// **呼叫端** 已從背包扣除木材；若此函式回傳 `None` 請退還。
    pub fn contribute(&mut self, pid: Uuid, px: f32, py: f32) -> Option<ContributeOutcome> {
        if !px.is_finite() || !py.is_finite() {
            return None;
        }
        if self.status != ShipStatus::Broken {
            return None;
        }
        if self.contrib_cooldowns.get(&pid).is_some_and(|&cd| cd > 0.0) {
            return None;
        }
        let dx = px - SHIP_WX;
        let dy = py - SHIP_WY;
        if dx * dx + dy * dy > REPAIR_REACH * REPAIR_REACH {
            return None;
        }
        // 貢獻成功——設冷卻、累進進度。
        self.contrib_cooldowns.insert(pid, CONTRIB_COOLDOWN_SECS);
        self.progress += 1;
        if self.progress >= REPAIR_GOAL {
            self.status = ShipStatus::Repaired;
            self.timer = REPAIRED_SECS;
            self.progress = 0;
            Some(ContributeOutcome::Repaired)
        } else {
            Some(ContributeOutcome::Progress(self.progress))
        }
    }
}

impl Default for ShipRepairState {
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

    /// 星艦中心座標（方便呼叫）。
    fn at_ship() -> (f32, f32) {
        (SHIP_WX, SHIP_WY)
    }

    /// 距星艦剛好超出半徑外。
    fn outside_ship() -> (f32, f32) {
        (SHIP_WX + REPAIR_REACH + 1.0, SHIP_WY)
    }

    #[test]
    fn new_state_is_broken_zero_progress() {
        let s = ShipRepairState::new();
        assert!(s.is_broken());
        assert!(!s.is_repaired());
        assert_eq!(s.progress, 0);
        let v = s.view();
        assert_eq!(v.progress, 0);
        assert_eq!(v.goal, REPAIR_GOAL);
        assert_eq!(v.repaired_secs, 0);
    }

    #[test]
    fn contribute_advances_progress() {
        let mut s = ShipRepairState::new();
        let (px, py) = at_ship();
        let r = s.contribute(pid(1), px, py);
        assert_eq!(r, Some(ContributeOutcome::Progress(1)));
        assert_eq!(s.progress, 1);
        assert!(s.is_broken());
    }

    #[test]
    fn contribute_completes_repair_at_goal() {
        let mut s = ShipRepairState::new();
        let (px, py) = at_ship();
        // 先推到 REPAIR_GOAL - 1 次（不同玩家繞過個人冷卻）
        for i in 0..(REPAIR_GOAL - 1) {
            let r = s.contribute(pid(i as u128), px, py);
            assert!(matches!(r, Some(ContributeOutcome::Progress(_))));
        }
        // 第 REPAIR_GOAL 次觸發修繕完成
        let r = s.contribute(pid(99), px, py);
        assert_eq!(r, Some(ContributeOutcome::Repaired));
        assert!(s.is_repaired());
        assert_eq!(s.progress, 0, "修繕完成後進度應歸零");
        let v = s.view();
        assert_eq!(v.repaired_secs, REPAIRED_SECS.ceil() as u32);
    }

    #[test]
    fn contribute_rejects_outside_reach() {
        let mut s = ShipRepairState::new();
        let (px, py) = outside_ship();
        let r = s.contribute(pid(1), px, py);
        assert_eq!(r, None, "超出半徑不應貢獻");
        assert_eq!(s.progress, 0);
    }

    #[test]
    fn contribute_rejects_non_finite_position() {
        let mut s = ShipRepairState::new();
        assert_eq!(s.contribute(pid(1), f32::NAN, SHIP_WY), None, "NaN 座標應拒絕");
        assert_eq!(s.contribute(pid(1), SHIP_WX, f32::INFINITY), None, "Inf 座標應拒絕");
        assert_eq!(s.progress, 0);
    }

    #[test]
    fn contribute_has_per_player_cooldown() {
        let mut s = ShipRepairState::new();
        let (px, py) = at_ship();
        assert!(s.contribute(pid(1), px, py).is_some(), "首次貢獻應成功");
        assert_eq!(
            s.contribute(pid(1), px, py),
            None,
            "冷卻中不能再貢獻"
        );
        // 另一位玩家不受影響
        assert!(s.contribute(pid(2), px, py).is_some(), "別的玩家可各自貢獻");
    }

    #[test]
    fn cooldown_clears_after_tick() {
        let mut s = ShipRepairState::new();
        let (px, py) = at_ship();
        s.contribute(pid(1), px, py);
        // 冷卻期內仍不能貢獻
        s.tick(CONTRIB_COOLDOWN_SECS - 1.0);
        assert_eq!(s.contribute(pid(1), px, py), None, "冷卻未滿");
        // 冷卻走完
        s.tick(2.0);
        assert!(s.contribute(pid(1), px, py).is_some(), "冷卻走完後可再貢獻");
    }

    #[test]
    fn repaired_ship_rejects_contributions() {
        let mut s = ShipRepairState::new();
        let (px, py) = at_ship();
        for i in 0..REPAIR_GOAL {
            s.contribute(pid(i as u128), px, py);
        }
        assert!(s.is_repaired());
        // 修繕完成後不接受貢獻
        assert_eq!(
            s.contribute(pid(99), px, py),
            None,
            "已修繕的星艦不接受貢獻"
        );
    }

    #[test]
    fn repaired_timer_counts_down_and_enters_cooldown() {
        let mut s = ShipRepairState::new();
        let (px, py) = at_ship();
        for i in 0..REPAIR_GOAL {
            s.contribute(pid(i as u128), px, py);
        }
        assert!(s.is_repaired());
        // 閃耀倒計時
        s.tick(REPAIRED_SECS - 1.0);
        assert!(s.is_repaired(), "閃耀未結束應仍在 Repaired");
        s.tick(2.0);
        assert_eq!(s.status, ShipStatus::Cooldown, "閃耀結束後應進入 Cooldown");
    }

    #[test]
    fn cooldown_ends_with_broke_down_tick() {
        let mut s = ShipRepairState::new();
        let (px, py) = at_ship();
        for i in 0..REPAIR_GOAL {
            s.contribute(pid(i as u128), px, py);
        }
        // 跳到 Cooldown
        s.tick(REPAIRED_SECS + 1.0);
        assert_eq!(s.status, ShipStatus::Cooldown);
        // 冷卻結束 → BrokeDown
        s.tick(COOLDOWN_AFTER_REPAIR_SECS - 1.0);
        let r = s.tick(2.0);
        assert_eq!(r, RepairTick::BrokeDown, "冷卻結束應回傳 BrokeDown");
        assert!(s.is_broken(), "BrokeDown 後應回到 Broken");
        assert_eq!(s.progress, 0);
    }

    #[test]
    fn tick_zero_dt_is_noop() {
        let mut s = ShipRepairState::new();
        let (px, py) = at_ship();
        s.contribute(pid(1), px, py);
        let before = s.progress;
        assert_eq!(s.tick(0.0), RepairTick::None);
        assert_eq!(s.tick(-5.0), RepairTick::None);
        assert_eq!(s.progress, before, "壞 dt 不應改變進度");
    }

    #[test]
    fn view_shows_zero_progress_when_repaired() {
        let mut s = ShipRepairState::new();
        let (px, py) = at_ship();
        for i in 0..REPAIR_GOAL {
            s.contribute(pid(i as u128), px, py);
        }
        let v = s.view();
        assert_eq!(v.progress, 0, "修繕完成後快照進度應顯示 0（非 Broken 狀態）");
        assert!(v.repaired_secs > 0, "修繕完成後剩餘秒數應 > 0");
    }
}
