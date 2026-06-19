//! 活動鏈系統（ROADMAP 390）：每個現實日內完成不同類型的活動，每完成一種新類型即加一環。
//!
//! 五種活動類型：戰鬥（擊殺敵人）、採集（採集資源）、合成（合成物品）、
//! 社交（表情/擊掌）、探索（旅行到非故鄉星球）。
//! 3 環里程碑 +20 乙太；5 環全鏈完成 +50 乙太 + 全服廣播。
//!
//! 純邏輯層（無 IO / 無 WebSocket）：記憶體前置，重啟清空，零 migration。

/// 活動類型（五種，對應 bitmask 的第 0~4 位）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityKind {
    /// 擊殺任意敵人。
    Battle  = 0,
    /// 採集任意資源（木材、礦石、野花、蕈菇、晶石、古代碎片等）。
    Gather  = 1,
    /// 在工作台合成任意物品。
    Craft   = 2,
    /// 社交互動（對他人使用表情、擊掌）。
    Social  = 3,
    /// 探索（星際旅行到任何非故鄉星球）。
    Explore = 4,
}

/// 活動類型總數（5 種）。
pub const TOTAL_KINDS: u8 = 5;

/// 達到此環數時給予第一段里程碑乙太獎勵。
pub const MILESTONE3_LINKS: u8 = 3;
/// 達到全鏈（TOTAL_KINDS 環）時的乙太獎勵。
pub const MILESTONE5_LINKS: u8 = TOTAL_KINDS;

/// 3 環里程碑乙太獎勵。
pub const REWARD_3_ETHER: u32 = 20;
/// 全鏈完成乙太獎勵。
pub const REWARD_5_ETHER: u32 = 50;

/// 重置週期：24 小時（秒）。
pub const RESET_SECS: u64 = 86_400;

/// `record` 回傳：描述本次呼叫的結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChainUpdate {
    /// 記錄前的環數。
    pub links_before: u8,
    /// 記錄後的環數（若 == links_before 代表此活動今日已完成，無新環）。
    pub links_after: u8,
    /// 本次觸發的乙太獎勵（0 = 無獎勵）。
    pub ether_reward: u32,
}

impl ChainUpdate {
    /// 無新環（活動已完成）。
    pub fn already_done(links: u8) -> Self {
        Self { links_before: links, links_after: links, ether_reward: 0 }
    }

    /// 新增一環（鏈增長了）。
    fn new_link(before: u8, after: u8, ether: u32) -> Self {
        Self { links_before: before, links_after: after, ether_reward: ether }
    }

    /// 是否真的新增了環。
    pub fn is_new_link(&self) -> bool {
        self.links_after > self.links_before
    }

    /// 是否達成全鏈（5/5）——僅在本次呼叫真正新增了第 5 環時為 true。
    pub fn is_chain_complete(&self) -> bool {
        self.is_new_link() && self.links_after == TOTAL_KINDS
    }
}

/// 一位玩家的活動鏈狀態（記憶體前置，重啟清空，零 migration）。
#[derive(Debug, Clone)]
pub struct ActivityChain {
    /// 今日已完成活動的 bitmask（第 i 位對應 ActivityKind 的 discriminant）。
    done: u8,
    /// 此批活動在此 Unix timestamp（秒）之後需重置。
    pub reset_at: u64,
    /// 已發放的里程碑 bitmask（bit0=3環, bit1=5環），防止重複發獎。
    milestones_rewarded: u8,
}

impl ActivityChain {
    /// 以現在時刻建立今日活動鏈（清空、計算重置點）。
    pub fn new(now_secs: u64) -> Self {
        let day_index = now_secs / RESET_SECS;
        let reset_at = (day_index + 1) * RESET_SECS;
        Self { done: 0, reset_at, milestones_rewarded: 0 }
    }

    /// 若已過重置時刻，以今天的新狀態取代。
    pub fn check_reset(&mut self, now_secs: u64) {
        if now_secs >= self.reset_at {
            *self = Self::new(now_secs);
        }
    }

    /// 目前已完成的環數。
    pub fn link_count(&self) -> u8 {
        self.done.count_ones() as u8
    }

    /// 記錄一種活動。
    ///
    /// 若此活動今日已完成，回傳 `AlreadyDone`（無新環）；
    /// 否則加環，並視乎是否觸發里程碑決定 `ether_reward`。
    pub fn record(&mut self, kind: ActivityKind, now_secs: u64) -> ChainUpdate {
        self.check_reset(now_secs);
        let bit = 1u8 << (kind as u8);
        if self.done & bit != 0 {
            return ChainUpdate::already_done(self.link_count());
        }
        let before = self.link_count();
        self.done |= bit;
        let after = self.link_count();

        let ether = self.consume_milestone_reward(after);
        ChainUpdate::new_link(before, after, ether)
    }

    /// 是否已完成此活動類型。
    pub fn is_done(&self, kind: ActivityKind) -> bool {
        self.done & (1u8 << kind as u8) != 0
    }

    /// 取本次加環後應發放的里程碑乙太（並標記已發放，防重複）。
    fn consume_milestone_reward(&mut self, links: u8) -> u32 {
        let mut reward = 0u32;
        // 3 環里程碑（bit0）
        if links >= MILESTONE3_LINKS && self.milestones_rewarded & 0b01 == 0 {
            self.milestones_rewarded |= 0b01;
            reward += REWARD_3_ETHER;
        }
        // 5 環全鏈（bit1）—— 依序累加（若一次從 0 跳到 5，兩個里程碑都給）
        if links >= MILESTONE5_LINKS && self.milestones_rewarded & 0b10 == 0 {
            self.milestones_rewarded |= 0b10;
            reward += REWARD_5_ETHER;
        }
        reward
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// 測試
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn chain_at(secs: u64) -> ActivityChain {
        ActivityChain::new(secs)
    }

    #[test]
    fn new_chain_has_zero_links() {
        let c = chain_at(0);
        assert_eq!(c.link_count(), 0);
    }

    #[test]
    fn record_first_activity_adds_link() {
        let mut c = chain_at(0);
        let up = c.record(ActivityKind::Battle, 0);
        assert!(up.is_new_link());
        assert_eq!(up.links_after, 1);
        assert_eq!(c.link_count(), 1);
    }

    #[test]
    fn record_same_activity_twice_is_noop() {
        let mut c = chain_at(0);
        c.record(ActivityKind::Gather, 0);
        let up2 = c.record(ActivityKind::Gather, 0);
        assert!(!up2.is_new_link());
        assert_eq!(c.link_count(), 1);
        assert_eq!(up2.ether_reward, 0);
    }

    #[test]
    fn five_different_activities_reach_full_chain() {
        let mut c = chain_at(0);
        for kind in [
            ActivityKind::Battle,
            ActivityKind::Gather,
            ActivityKind::Craft,
            ActivityKind::Social,
            ActivityKind::Explore,
        ] {
            c.record(kind, 0);
        }
        assert_eq!(c.link_count(), 5);
    }

    #[test]
    fn milestone3_reward_at_third_link() {
        let mut c = chain_at(0);
        c.record(ActivityKind::Battle, 0);
        c.record(ActivityKind::Gather, 0);
        let up3 = c.record(ActivityKind::Craft, 0);
        assert_eq!(up3.links_after, 3);
        assert_eq!(up3.ether_reward, REWARD_3_ETHER);
    }

    #[test]
    fn milestone3_not_repeated() {
        let mut c = chain_at(0);
        c.record(ActivityKind::Battle, 0);
        c.record(ActivityKind::Gather, 0);
        let up3 = c.record(ActivityKind::Craft, 0);
        // 第三環已給獎
        let up4 = c.record(ActivityKind::Social, 0);
        assert_eq!(up3.ether_reward, REWARD_3_ETHER);
        assert_eq!(up4.ether_reward, 0); // 第四環無獎
    }

    #[test]
    fn chain_complete_reward_at_fifth_link() {
        let mut c = chain_at(0);
        c.record(ActivityKind::Battle, 0);
        c.record(ActivityKind::Gather, 0);
        c.record(ActivityKind::Craft, 0); // 3 環里程碑
        c.record(ActivityKind::Social, 0);
        let up5 = c.record(ActivityKind::Explore, 0);
        assert!(up5.is_chain_complete());
        assert_eq!(up5.ether_reward, REWARD_5_ETHER);
    }

    #[test]
    fn chain_complete_flag() {
        let mut c = chain_at(0);
        c.record(ActivityKind::Battle, 0);
        c.record(ActivityKind::Gather, 0);
        c.record(ActivityKind::Craft, 0);
        c.record(ActivityKind::Social, 0);
        let up = c.record(ActivityKind::Explore, 0);
        assert!(up.is_chain_complete());
    }

    #[test]
    fn already_done_is_not_chain_complete() {
        let mut c = chain_at(0);
        for kind in [
            ActivityKind::Battle,
            ActivityKind::Gather,
            ActivityKind::Craft,
            ActivityKind::Social,
            ActivityKind::Explore,
        ] {
            c.record(kind, 0);
        }
        // 再做一次，應為 already_done
        let up = c.record(ActivityKind::Battle, 0);
        assert!(!up.is_new_link());
        assert!(!up.is_chain_complete());
        assert_eq!(up.ether_reward, 0);
    }

    #[test]
    fn reset_clears_chain() {
        let mut c = chain_at(0);
        c.record(ActivityKind::Battle, 0);
        c.record(ActivityKind::Gather, 0);
        assert_eq!(c.link_count(), 2);
        // 模擬隔天
        c.check_reset(RESET_SECS);
        assert_eq!(c.link_count(), 0);
        // 隔天的活動可以重新累積
        let up = c.record(ActivityKind::Battle, RESET_SECS);
        assert!(up.is_new_link());
    }

    #[test]
    fn reset_at_calculated_correctly() {
        let c = chain_at(0);
        assert_eq!(c.reset_at, RESET_SECS); // 第 0 天到 86400
        let c2 = chain_at(RESET_SECS + 1);
        assert_eq!(c2.reset_at, 2 * RESET_SECS);
    }

    #[test]
    fn is_done_returns_correct_flags() {
        let mut c = chain_at(0);
        assert!(!c.is_done(ActivityKind::Battle));
        c.record(ActivityKind::Battle, 0);
        assert!(c.is_done(ActivityKind::Battle));
        assert!(!c.is_done(ActivityKind::Craft));
    }

    #[test]
    fn total_reward_for_full_chain() {
        let mut c = chain_at(0);
        let kinds = [
            ActivityKind::Battle,
            ActivityKind::Gather,
            ActivityKind::Craft,
            ActivityKind::Social,
            ActivityKind::Explore,
        ];
        let total_ether: u32 = kinds.iter().map(|&k| c.record(k, 0).ether_reward).sum();
        // 3 環 +20 乙太 + 5 環 +50 乙太 = 70
        assert_eq!(total_ether, REWARD_3_ETHER + REWARD_5_ETHER);
    }

    #[test]
    fn milestones_not_given_after_reset() {
        let mut c = chain_at(0);
        c.record(ActivityKind::Battle, 0);
        c.record(ActivityKind::Gather, 0);
        c.record(ActivityKind::Craft, 0); // 3 環里程碑已給
        // 重置
        c.check_reset(RESET_SECS);
        // 隔天再做三件事，里程碑應重新給
        c.record(ActivityKind::Battle, RESET_SECS);
        c.record(ActivityKind::Gather, RESET_SECS);
        let up3_again = c.record(ActivityKind::Craft, RESET_SECS);
        assert_eq!(up3_again.ether_reward, REWARD_3_ETHER);
    }
}
