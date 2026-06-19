//! ROADMAP 396 新手引導 —— 全新玩家「最初幾步」的核心循環指引。
//!
//! 這個世界對全新玩家其實很沉默：一進場只看得到一片地圖，卻不知道該往哪走、要做什麼。
//! 本模組給全新玩家一張「最初幾步」的引導清單——把這個療癒世界的核心循環
//! （採集 → 種植 → 收成 → 打招呼 → 合成）拆成五個小步，每完成一步就在 HUD 點亮一格，
//! 全部走完獲得一份迎新乙太、引導從此功成身退。
//!
//! 設計上刻意與 ROADMAP 390「每日活動鏈」分工乾淨：
//!   - 活動鏈是**每天**重來、獎勵「今天去了世界多少角落」、含戰鬥維度的**留存**機制；
//!   - 本模組是**一次性**、只給**全新**玩家、聚焦溫和農作核心循環的**新手引導**。
//! 兩者生命週期、受眾、骨架都不同：活動鏈是循環累積的進度條，引導是一次走完即畢業的教學。
//!
//! 全程**記憶體前置、零持久化、零 migration**：老玩家如何不被打擾？——連線時依玩家
//! 還原後的累積進度（經驗／乙太／背包／熟練度）判斷是否「看起來全新」，有任何進度的
//! 回鍋玩家直接種成「已畢業」，永不顯示；只有全零的全新帳號才啟用引導。與 ROADMAP 351
//! `seen_mastery_tiers`「連線即以當前狀態種下」是同一個成熟模式。

/// 新手引導的五個核心步驟（療癒向核心循環）。值即 bitmask 的位元位置（0~4）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnboardStep {
    /// 採集一份資源（撿起世界裡的素材）。
    Gather = 0,
    /// 種下第一棵作物。
    Plant = 1,
    /// 收成你的作物。
    Harvest = 2,
    /// 向鎮民打聲招呼（在 NPC 身旁送出表情）。
    Greet = 3,
    /// 親手合成一樣東西。
    Craft = 4,
}

/// 步驟總數。
pub const STEP_COUNT: u8 = 5;
/// 五步全完成的 bitmask。
const ALL_MASK: u8 = 0b1_1111;

/// 走完全部步驟的迎新獎勵（小額乙太）。療癒向、與既有立即回血／採集所得同量級，不影響平衡。
pub const WELCOME_ETHER: u32 = 15;

impl OnboardStep {
    #[inline]
    fn bit(self) -> u8 {
        1u8 << (self as u8)
    }
}

/// 引導步驟的「教學順序」——前端據此高亮「當前該做的下一步」並畫世界動線指引（ROADMAP 413）。
/// 順序＝採集→種植→收成→打招呼→合成（療癒農作核心循環，由易到難、先農後社交）。
/// 與 bitmask 位元值一致，但分開列出以明確「順序」這個獨立契約。
const STEPS_IN_ORDER: [OnboardStep; STEP_COUNT as usize] = [
    OnboardStep::Gather,
    OnboardStep::Plant,
    OnboardStep::Harvest,
    OnboardStep::Greet,
    OnboardStep::Craft,
];

/// 新手引導狀態。記憶體前置、零持久化、零 migration。
///
/// `Default` = 引導關閉（`active=false`）——對訪客／尚未種下的占位玩家而言「不顯示」是安全預設。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Onboarding {
    /// 已完成步驟 bitmask（位元對齊 `OnboardStep`）。
    done: u8,
    /// 引導是否啟用中：false = 老玩家／訪客／已畢業，永不顯示。
    active: bool,
    /// 迎新獎勵是否已發（防重複發放）。
    rewarded: bool,
}

/// 標記一步完成後的結果，供呼叫端決定要不要發獎／通知。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnboardOutcome {
    /// 沒有變化（引導未啟用、或該步早已完成、或已畢業）。
    NoChange,
    /// 完成一步、尚未走完全程。
    Advanced { done_count: u8 },
    /// 完成最後一步、發放迎新獎勵（一次性）。
    Finished { reward: u32 },
}

impl Onboarding {
    /// 全新玩家：引導啟用、五步皆待完成。
    pub fn fresh() -> Self {
        Self { done: 0, active: true, rewarded: false }
    }

    /// 老玩家／訪客：引導關閉、視為已畢業，永不顯示。
    pub fn graduated() -> Self {
        Self { done: ALL_MASK, active: false, rewarded: true }
    }

    /// 依玩家是否「看起來是全新玩家」種下初始狀態（連線、還原進度後呼叫）。
    pub fn seed(looks_new: bool) -> Self {
        if looks_new { Self::fresh() } else { Self::graduated() }
    }

    /// 引導是否仍在顯示中（啟用且尚未走完）。
    pub fn is_active(&self) -> bool {
        self.active && self.done != ALL_MASK
    }

    /// 已完成步驟數（0~5）。
    pub fn done_count(&self) -> u8 {
        self.done.count_ones() as u8
    }

    /// 某步是否已完成。
    pub fn is_done(&self, step: OnboardStep) -> bool {
        self.done & step.bit() != 0
    }

    /// 已完成步驟 bitmask（供前端逐格點亮）。
    pub fn mask(&self) -> u8 {
        self.done
    }

    /// 當前「該做的下一步」——教學順序中第一個尚未完成的步驟（ROADMAP 413）。
    /// 引導未啟用或已全數完成回 `None`（前端據此隱藏動線指引）。純函式、確定可重現。
    pub fn next_step(&self) -> Option<OnboardStep> {
        if !self.is_active() {
            return None;
        }
        STEPS_IN_ORDER.into_iter().find(|&step| !self.is_done(step))
    }

    /// 標記某步驟完成。冪等：引導未啟用、或該步已完成皆回 `NoChange`。
    /// 完成最後一步時把獎勵標記為已發並畢業（`active=false`），回 `Finished`。
    pub fn complete(&mut self, step: OnboardStep) -> OnboardOutcome {
        if !self.active || self.done & step.bit() != 0 {
            return OnboardOutcome::NoChange;
        }
        self.done |= step.bit();
        if self.done == ALL_MASK {
            self.active = false; // 走完即畢業，不再顯示
            if !self.rewarded {
                self.rewarded = true;
                return OnboardOutcome::Finished { reward: WELCOME_ETHER };
            }
            return OnboardOutcome::NoChange;
        }
        OnboardOutcome::Advanced { done_count: self.done_count() }
    }
}

/// 純啟發式：玩家是否「看起來剛開始」（零累積進度）。
///
/// 連線、還原進度後呼叫；有任何累積（經驗／乙太／背包／熟練度）的回鍋玩家視為已畢業、
/// 不被引導打擾，只有全零的全新帳號才啟用引導。純函式、確定可測。
pub fn looks_like_new_player(exp: u32, ether: u32, inventory_empty: bool, mastery_total: u32) -> bool {
    exp == 0 && ether == 0 && inventory_empty && mastery_total == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_starts_active_and_empty() {
        let ob = Onboarding::fresh();
        assert!(ob.is_active());
        assert_eq!(ob.done_count(), 0);
        assert_eq!(ob.mask(), 0);
        assert!(!ob.is_done(OnboardStep::Gather));
    }

    #[test]
    fn graduated_never_shows() {
        let ob = Onboarding::graduated();
        assert!(!ob.is_active());
        // 已畢業者再標記任何步驟都不會重新顯示或再發獎。
        let mut ob = ob;
        assert_eq!(ob.complete(OnboardStep::Gather), OnboardOutcome::NoChange);
        assert!(!ob.is_active());
    }

    #[test]
    fn default_is_inactive() {
        // 預設（訪客／占位）= 不顯示。
        let ob = Onboarding::default();
        assert!(!ob.is_active());
    }

    #[test]
    fn seed_picks_fresh_or_graduated() {
        assert!(Onboarding::seed(true).is_active());
        assert!(!Onboarding::seed(false).is_active());
    }

    #[test]
    fn complete_advances_one_step() {
        let mut ob = Onboarding::fresh();
        let out = ob.complete(OnboardStep::Gather);
        assert_eq!(out, OnboardOutcome::Advanced { done_count: 1 });
        assert!(ob.is_done(OnboardStep::Gather));
        assert_eq!(ob.done_count(), 1);
        assert!(ob.is_active());
    }

    #[test]
    fn complete_is_idempotent_per_step() {
        let mut ob = Onboarding::fresh();
        assert_eq!(ob.complete(OnboardStep::Plant), OnboardOutcome::Advanced { done_count: 1 });
        // 同一步再完成不重複加環。
        assert_eq!(ob.complete(OnboardStep::Plant), OnboardOutcome::NoChange);
        assert_eq!(ob.done_count(), 1);
    }

    #[test]
    fn finishing_all_steps_rewards_once_and_graduates() {
        let mut ob = Onboarding::fresh();
        let steps = [
            OnboardStep::Gather,
            OnboardStep::Plant,
            OnboardStep::Harvest,
            OnboardStep::Greet,
        ];
        for s in steps {
            assert!(matches!(ob.complete(s), OnboardOutcome::Advanced { .. }));
        }
        assert!(ob.is_active());
        // 最後一步：發迎新獎勵、畢業。
        let out = ob.complete(OnboardStep::Craft);
        assert_eq!(out, OnboardOutcome::Finished { reward: WELCOME_ETHER });
        assert!(!ob.is_active());
        assert_eq!(ob.done_count(), STEP_COUNT);
        // 再戳已畢業者不重複發獎。
        assert_eq!(ob.complete(OnboardStep::Craft), OnboardOutcome::NoChange);
    }

    #[test]
    fn finishing_in_any_order_still_rewards_exactly_once() {
        let mut ob = Onboarding::fresh();
        let order = [
            OnboardStep::Craft,
            OnboardStep::Greet,
            OnboardStep::Harvest,
            OnboardStep::Plant,
            OnboardStep::Gather,
        ];
        let mut finishes = 0;
        for s in order {
            if let OnboardOutcome::Finished { reward } = ob.complete(s) {
                assert_eq!(reward, WELCOME_ETHER);
                finishes += 1;
            }
        }
        assert_eq!(finishes, 1);
    }

    #[test]
    fn next_step_is_first_uncompleted_in_order() {
        let mut ob = Onboarding::fresh();
        // 全新玩家：第一步＝採集。
        assert_eq!(ob.next_step(), Some(OnboardStep::Gather));
        ob.complete(OnboardStep::Gather);
        assert_eq!(ob.next_step(), Some(OnboardStep::Plant));
        ob.complete(OnboardStep::Plant);
        assert_eq!(ob.next_step(), Some(OnboardStep::Harvest));
    }

    #[test]
    fn next_step_skips_out_of_order_completions() {
        let mut ob = Onboarding::fresh();
        // 先完成靠後的「合成」：下一步仍指向順序中最前的未完成步（採集），不被打亂。
        ob.complete(OnboardStep::Craft);
        assert_eq!(ob.next_step(), Some(OnboardStep::Gather));
        ob.complete(OnboardStep::Gather);
        ob.complete(OnboardStep::Plant);
        ob.complete(OnboardStep::Harvest);
        // 只剩「打招呼」未完成（合成早已完成）。
        assert_eq!(ob.next_step(), Some(OnboardStep::Greet));
    }

    #[test]
    fn next_step_none_when_inactive_or_done() {
        // 訪客／已畢業：無下一步。
        assert_eq!(Onboarding::default().next_step(), None);
        assert_eq!(Onboarding::graduated().next_step(), None);
        // 走完全程後（畢業）也無下一步。
        let mut ob = Onboarding::fresh();
        for s in STEPS_IN_ORDER {
            ob.complete(s);
        }
        assert_eq!(ob.next_step(), None);
    }

    #[test]
    fn looks_new_only_when_all_zero() {
        assert!(looks_like_new_player(0, 0, true, 0));
        assert!(!looks_like_new_player(1, 0, true, 0));
        assert!(!looks_like_new_player(0, 5, true, 0));
        assert!(!looks_like_new_player(0, 0, false, 0));
        assert!(!looks_like_new_player(0, 0, true, 30));
    }
}
