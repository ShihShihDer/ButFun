//! ROADMAP 410 翻滾閃避——戰鬥防禦從「凝盾硬擋」長出第二條路：「翻身閃開」。
//!
//! 408 臨陣格擋給了戰鬥第一個防禦技巧——看準甜蜜點按下、凝一面乙太護盾「卸掉」反擊傷害；
//! 但那是**定點硬擋**：站著不動、靠時機把傷害減到 ≤85%（永不完全免傷）。本切片補上防禦的
//! 另一極——**翻身閃開**：被敵人威脅時按「🤸 翻滾」往移動方向一個翻滾，閃身的這短短一瞬
//! （閃避恩典窗）**完全閃掉接下來那一次反擊**（零傷，而非卸一部分）；你照常邊翻邊往移動方向走，
//! 自然就閃離了原地（位移走玩家自身移動，伺服器不另推座標）。
//!
//! 全純記憶體、純函式、零持久化、零 migration、零 LLM。
//!
//! **換骨架（戰鬥防禦的第二招，且明確別於既有兩招）**：
//!   - 不同於 408 格擋——格擋是「定點、看甜蜜點時機、凝盾**減**傷（≤85%）、撐 1~2 秒」；
//!     翻滾是「位移、隨手即用、恩典窗**完全閃掉**一次反擊（零傷）、瞬發」。一個是硬擋的工匠手藝、
//!     一個是身法的閃身，互補成完整的防禦套路。
//!   - 不同於風之步（ROADMAP 45 Gale）——風之步是**熟練度解鎖、長冷卻**的瞬移逃離（拉開距離），
//!     翻滾是**人人即有、短冷卻**的近身閃避（閃掉一次反擊）；用途與手感都不同。
//!
//! **平衡（誠實交代，純療癒向、零經濟擾動）**：翻滾**只讓玩家「躲掉」一次敵人反擊傷害**，
//! 完全不碰攻擊力、不改掉落／經驗、不縮短擊倒（攻擊仍走既有手動 `Attack`、行為一字不改）。
//! 閃避恩典窗只可靠地涵蓋「接下來那一次」每秒反擊（窗略長於 1 秒、必含一個反擊節拍），
//! 且翻滾有冷卻——`DODGE_COOLDOWN_SECS`（3 秒）> 恩典窗，無法靠連續翻滾達成永久無敵：
//! 反擊每秒一次、翻滾每 3 秒才一次，最多每三次反擊閃掉一次，戰鬥仍「有來有回」。
//! 翻滾期間的位移走玩家自身移動（前端權威座標，已含地形碰撞），伺服器只管「這一瞬免不免傷」，
//! 與進攻管線（暴擊／連殺／元素／戰吼）完全正交、零經濟面影響。

/// 翻滾的視覺時長（秒）：前端據此演出一個翻身的位移動畫；伺服器用同一常數渲染翻滾環。
/// 比恩典窗短——翻身在前半瞬完成，後半是落地的餘韻。
pub const DODGE_ROLL_SECS: f32 = 0.5;

/// 閃避恩典窗（秒）：自翻滾起算這麼久內，敵人的「每秒一次」反擊被完全閃掉（零傷）。
/// 刻意略長於 1 秒——反擊在整秒節拍觸發，長度 > 1 秒的窗必定涵蓋「接下來那一次」反擊節拍，
/// 讓「翻滾就能躲掉一下」公平可預期（不會剛好落在兩拍之間白翻一場）。
pub const DODGE_GRACE_SECS: f32 = 1.05;

/// 翻滾冷卻（秒）：翻滾結算後起算，只擋「開新一趟翻滾」。
/// > 恩典窗，確保無法靠連續翻滾永久免傷（反擊每秒一次、翻滾每 3 秒一次）。
pub const DODGE_COOLDOWN_SECS: f32 = 3.0;

/// 一趟進行中的翻滾（記憶體前置、`Copy`、重啟清空）。
///
/// 只記一個累計時間：前 `DODGE_ROLL_SECS` 是翻身動畫、整段 `DODGE_GRACE_SECS` 內享閃避恩典。
/// 與 `guard::GuardBrace`／`woodcutting::ChopSwing` 同脈絡——純函式、確定可重現、`Copy`。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DodgeRoll {
    /// 距翻滾起算經過的時間（秒）。
    elapsed: f32,
}

impl DodgeRoll {
    /// 開一趟新的翻滾。
    pub fn start() -> Self {
        DodgeRoll { elapsed: 0.0 }
    }

    /// 距翻滾起算經過的時間（秒）——隨快照廣播給前端，渲染翻身的位移與翻滾環。
    pub fn elapsed(self) -> f32 {
        self.elapsed
    }

    /// 此刻是否仍在閃避恩典窗內：窗內的敵人反擊被完全閃掉（零傷）。
    /// 反擊迴圈每秒讀它一次——窗長 > 1 秒，必涵蓋接下來那一個反擊節拍。
    pub fn in_grace(self) -> bool {
        self.elapsed < DODGE_GRACE_SECS
    }

    /// 推進一個 tick：累加時間，回傳這趟翻滾是否已落幕（恩典窗已過，呼叫端據此清空）。
    /// 負或零 dt 不前進（守時間單調）。
    pub fn advance(&mut self, dt: f32) -> bool {
        if dt > 0.0 {
            self.elapsed += dt;
        }
        self.elapsed >= DODGE_GRACE_SECS
    }
}

// ─── 單元測試 ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // 編譯期不變式：冷卻必須 > 恩典窗，否則可連續翻滾達成永久免傷（戰鬥失去「有來有回」）。
    const _: () = assert!(DODGE_COOLDOWN_SECS > DODGE_GRACE_SECS);
    // 編譯期不變式：恩典窗 > 1 秒，才必定涵蓋「每秒一次」反擊的接下來那一拍。
    const _: () = assert!(DODGE_GRACE_SECS > 1.0);
    // 視覺翻身應在恩典窗內完成（翻身先於恩典結束）。
    const _: () = assert!(DODGE_ROLL_SECS < DODGE_GRACE_SECS);

    #[test]
    fn roll_starts_zeroed_and_in_grace() {
        let d = DodgeRoll::start();
        assert!((d.elapsed() - 0.0).abs() < 1e-6);
        // 剛翻滾即享閃避恩典。
        assert!(d.in_grace());
    }

    #[test]
    fn grace_covers_first_then_lapses() {
        let mut d = DodgeRoll::start();
        // 恩典窗內仍可閃避。
        assert!(!d.advance(DODGE_GRACE_SECS - 0.1));
        assert!(d.in_grace(), "恩典窗內應仍免傷");
        // 推過恩典窗即落幕、不再免傷。
        assert!(d.advance(0.2), "累過恩典窗回 true（這趟落幕）");
        assert!(!d.in_grace(), "恩典窗已過不該再免傷");
    }

    #[test]
    fn grace_window_spans_a_full_second_tick() {
        // 反擊每整秒一次。恩典窗 > 1 秒：不論翻滾落在哪個相位，窗內必含一個反擊節拍。
        // 以最壞相位（剛錯過一拍）驗證：起算後 1.0 秒處的那一拍仍在窗內。
        let d = DodgeRoll::start();
        // 模擬「翻滾後第 1.0 秒的反擊節拍」尚在恩典窗內。
        let mut at_next_tick = d;
        at_next_tick.advance(1.0);
        assert!(at_next_tick.in_grace(), "接下來那一拍（+1.0s）必須落在恩典窗內");
    }

    #[test]
    fn advance_ignores_non_positive_dt() {
        let mut d = DodgeRoll::start();
        let before = d;
        assert!(!d.advance(0.0));
        assert!(!d.advance(-2.0));
        assert_eq!(d, before, "非正 dt 不應改變狀態");
    }

    #[test]
    fn advance_accumulates_until_lapse() {
        let mut d = DodgeRoll::start();
        let mut lapsed = false;
        // 逐小步推進，恰在累過恩典窗那步回 true。
        for _ in 0..100 {
            if d.advance(0.02) {
                lapsed = true;
                break;
            }
        }
        assert!(lapsed, "持續推進終會落幕");
        assert!(!d.in_grace());
    }
}
