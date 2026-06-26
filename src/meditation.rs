//! 安靜打坐（ROADMAP 391）——在黃昏或夜晚的安全地帶靜止不動，緩緩恢復 HP ＋ 乙太；
//! 打坐者身旁漾出柔和光圈，讓附近玩家也看得見「有人在靜心」。
//!
//! ROADMAP 552 三重定境（玩家建議驅動·完善 2D）：在此之前打坐是「坐滿 30 秒→拿固定一份
//! 獎勵→結束」的單一節拍，夜裡那一坐沒有任何「坐得越久越值得」的取捨。建議箱多位居民
//! 一再反映「夜間只能等待／缺乏主動可進行的目標與即時達成感」。本切片把打坐從「定時完成」
//! 升級成「**越坐越深的定境**」：撐過三道時間門檻（淺定 30s／入定 75s／深定 120s）入更深的
//! 定境，**結束時（移動／按結束／坐到最深）依「入了第幾重定境」發獎**——入定回得多、深定回最多。
//! 夜裡那一坐第一次有了「再撐一會兒換更深的回復，還是起身去做別的事」的真實取捨與看得見的目標。
//!
//! 設計鐵律：
//! - **純邏輯可測**：`can_meditate`／`is_calm_phase`／`Meditation::is_interrupted`／
//!   `reached_tier`／`is_capped`／`progress`／`hp_heal`／`tier_hp_pct`／`tier_ether` 皆純函式，無副作用。
//! - **純正向（療癒向）**：入了哪重定境就**穩拿哪重**，移動結束不倒扣；只有「還沒入第一重
//!   就起身」才空手而回（與改版前「沒坐滿 30 秒就移動＝中斷」一致）。淺定（30s）的獎勵
//!   ＝改版前的固定獎勵，向後相容。
//! - **零持久化、零 migration**：`meditation` 與 `last_meditate` 是 Player 上的記憶體前置欄位，
//!   斷線/重啟清空，無需儲存。
//! - **玩家一眼有感**：打坐中的定境深度（`meditate_tier`，0/1/2/3）放進 `PlayerView` 快照廣播，
//!   前端對周圍打坐者畫**隨定境加深而更盛**的金色呼吸光圈，旁觀者一眼看得見「誰入了更深的定」。
//! - **不碰任何遊戲邏輯**：只讀安全區與時段，不改戰鬥 / 採集 / 合成規則；獎勵受 10 分鐘冷卻＋
//!   夜間限定＋安全區限定三重節流，對乙太經濟近乎零擾動。

use std::time::Instant;
use crate::daynight::Phase;

/// 入第一重「淺定」所需的靜止秒數（＝改版前的完成秒數；前端倒數與 `progress` 仍以此為一拍）。
pub const MEDITATE_DURATION_SECS: f32 = 30.0;
/// 淺定的乙太獎勵（＝改版前固定獎勵，向後相容）。
pub const MEDITATE_ETHER: u32 = 15;
/// 淺定的 HP 回復比例（百分比，30 = 回復 max HP 的 30%；＝改版前固定獎勵，向後相容）。
pub const MEDITATE_HP_PCT: u32 = 30;
/// 每次打坐之間的最短冷卻時間（秒）。
pub const MEDITATE_COOLDOWN_SECS: u64 = 600;
/// 移動超過此距離（像素）視為打坐被打斷，取得比腳步漂移誤差略大的值。
pub const ABORT_MOVE_PX: f32 = 8.0;

// ── ROADMAP 552 三重定境 ──────────────────────────────────────────────────────
/// 第一重「淺定」門檻（秒）＝既有 `MEDITATE_DURATION_SECS`：坐滿這麼久入淺定（向後相容）。
pub const TIER1_SECS: f32 = MEDITATE_DURATION_SECS;
/// 第二重「入定」門檻（秒）：再撐久一點入更深的定。
pub const TIER2_SECS: f32 = 75.0;
/// 第三重「深定」門檻（秒）＝最深，亦即自動結束的封頂（坐到這即穩拿深定，不必再無止境坐下去）。
pub const TIER3_SECS: f32 = 120.0;
/// 定境總重數（前端列指示／窮舉測試共用）。
pub const TIER_COUNT: u8 = 3;

/// 某一重定境的 HP 回復比例（百分比）。淺定＝既有 `MEDITATE_HP_PCT`，逐重加深；0 或越界＝0。
/// 純查表、可測——伺服器結算與前端預覽（如有）用同一份數值。
pub fn tier_hp_pct(tier: u8) -> u32 {
    match tier {
        1 => MEDITATE_HP_PCT, // 30：淺定＝改版前
        2 => 55,              // 入定
        3 => 90,              // 深定
        _ => 0,
    }
}

/// 某一重定境的乙太獎勵。淺定＝既有 `MEDITATE_ETHER`，逐重加深；0 或越界＝0。純查表、可測。
pub fn tier_ether(tier: u8) -> u32 {
    match tier {
        1 => MEDITATE_ETHER, // 15：淺定＝改版前
        2 => 28,             // 入定
        3 => 45,             // 深定
        _ => 0,
    }
}

/// 某一重定境的名稱（面向玩家字串；前端可逕用或自行在地化）。
pub fn tier_name(tier: u8) -> &'static str {
    match tier {
        1 => "淺定",
        2 => "入定",
        3 => "深定",
        _ => "靜心",
    }
}

/// 判斷是否可開始打坐（冷卻已過）。
pub fn can_meditate(last: Option<Instant>, now: Instant) -> bool {
    match last {
        None => true,
        Some(t) => now.duration_since(t).as_secs() >= MEDITATE_COOLDOWN_SECS,
    }
}

/// 此時段是否適合打坐（黃昏、夜晚、黎明——光線偏暗的「靜心時刻」）。
pub fn is_calm_phase(phase: Phase) -> bool {
    matches!(phase, Phase::Dusk | Phase::Night | Phase::Dawn)
}

/// 計算打坐完成後的 HP 回復量（至少 1）。
pub fn hp_heal(max_hp: u32, pct: u32) -> u32 {
    (max_hp * pct / 100).max(1)
}

/// 進行中的打坐狀態。
#[derive(Debug, Clone, Copy)]
pub struct Meditation {
    /// 打坐開始的時間點。
    pub start: Instant,
    /// 打坐開始時的玩家 X 座標（世界像素）。
    pub start_x: f32,
    /// 打坐開始時的玩家 Y 座標（世界像素）。
    pub start_y: f32,
}

impl Meditation {
    /// 建立一次新的打坐（以當下座標為基準點）。
    pub fn new(now: Instant, x: f32, y: f32) -> Self {
        Self { start: now, start_x: x, start_y: y }
    }

    /// 計算打坐進度 [0.0, 1.0]。
    pub fn progress(&self, now: Instant) -> f32 {
        (now.duration_since(self.start).as_secs_f32() / MEDITATE_DURATION_SECS).clamp(0.0, 1.0)
    }

    /// 玩家是否因移動而被打斷（位移 > `ABORT_MOVE_PX`）。
    pub fn is_interrupted(&self, x: f32, y: f32) -> bool {
        let dx = x - self.start_x;
        let dy = y - self.start_y;
        (dx * dx + dy * dy).sqrt() > ABORT_MOVE_PX
    }

    /// 打坐是否已坐滿第一重定境（靜止時間達淺定門檻）。保留供既有呼叫端／前端倒數判斷用。
    pub fn is_complete(&self, now: Instant) -> bool {
        now.duration_since(self.start).as_secs_f32() >= MEDITATE_DURATION_SECS
    }

    /// ROADMAP 552：已入第幾重定境——0＝還沒坐滿第一重；1＝淺定、2＝入定、3＝深定。
    /// 純由靜坐時長決定、確定可重現；伺服器結算、快照廣播、前端光圈共用同一套門檻。
    pub fn reached_tier(&self, now: Instant) -> u8 {
        let secs = now.duration_since(self.start).as_secs_f32();
        if secs >= TIER3_SECS {
            3
        } else if secs >= TIER2_SECS {
            2
        } else if secs >= TIER1_SECS {
            1
        } else {
            0
        }
    }

    /// ROADMAP 552：是否已坐到最深定境的封頂（撐滿 `TIER3_SECS`）——game.rs 據此自動結束，
    /// 讓玩家坐到深定即穩拿、不必無止境坐下去（也避免狀態永遠掛著）。
    pub fn is_capped(&self, now: Instant) -> bool {
        now.duration_since(self.start).as_secs_f32() >= TIER3_SECS
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn inst(secs_ago: u64) -> Instant {
        Instant::now() - Duration::from_secs(secs_ago)
    }

    #[test]
    fn no_cooldown_can_meditate() {
        assert!(can_meditate(None, Instant::now()));
    }

    #[test]
    fn within_cooldown_cannot() {
        assert!(!can_meditate(Some(inst(60)), Instant::now()));
    }

    #[test]
    fn expired_cooldown_can() {
        assert!(can_meditate(Some(inst(MEDITATE_COOLDOWN_SECS + 1)), Instant::now()));
    }

    #[test]
    fn boundary_cooldown_exact() {
        // 恰好等於冷卻時間算通過
        assert!(can_meditate(Some(inst(MEDITATE_COOLDOWN_SECS)), Instant::now()));
    }

    #[test]
    fn calm_phases_correct() {
        assert!(is_calm_phase(Phase::Dusk));
        assert!(is_calm_phase(Phase::Night));
        assert!(is_calm_phase(Phase::Dawn));
        assert!(!is_calm_phase(Phase::Day));
    }

    #[test]
    fn no_move_not_interrupted() {
        let m = Meditation::new(Instant::now(), 100.0, 100.0);
        assert!(!m.is_interrupted(100.0, 100.0));
        assert!(!m.is_interrupted(104.0, 100.0)); // 4px < 8px
    }

    #[test]
    fn big_move_interrupted() {
        let m = Meditation::new(Instant::now(), 100.0, 100.0);
        assert!(m.is_interrupted(200.0, 100.0)); // 100px > 8px
    }

    #[test]
    fn diagonal_move_interrupted() {
        let m = Meditation::new(Instant::now(), 0.0, 0.0);
        // 對角 6px*√2 ≈ 8.49px > 8px
        assert!(m.is_interrupted(6.0, 6.0));
    }

    #[test]
    fn not_complete_fresh() {
        let m = Meditation::new(Instant::now(), 0.0, 0.0);
        assert!(!m.is_complete(Instant::now()));
    }

    #[test]
    fn complete_after_duration() {
        let m = Meditation {
            start: inst(MEDITATE_DURATION_SECS as u64 + 1),
            start_x: 0.0,
            start_y: 0.0,
        };
        assert!(m.is_complete(Instant::now()));
    }

    #[test]
    fn progress_clamps_to_one() {
        let m = Meditation {
            start: inst(MEDITATE_DURATION_SECS as u64 + 10),
            start_x: 0.0,
            start_y: 0.0,
        };
        assert_eq!(m.progress(Instant::now()), 1.0);
    }

    #[test]
    fn hp_heal_calculation() {
        assert_eq!(hp_heal(100, 30), 30);
        assert_eq!(hp_heal(1, 30), 1); // 至少 1
        assert_eq!(hp_heal(200, 30), 60);
        assert_eq!(hp_heal(0, 30), 1); // 0 max_hp 也回 1（防呆）
    }

    // ── ROADMAP 552 三重定境 ──────────────────────────────────────────────────

    /// 入定境的時長門檻：未滿淺定＝0，三道門檻各自進階一重。
    #[test]
    fn reached_tier_steps_through_thresholds() {
        let m = Meditation::new(Instant::now(), 0.0, 0.0);
        let at = |secs: f32| m.reached_tier(m.start + Duration::from_secs_f32(secs));
        assert_eq!(at(0.0), 0, "剛開始還沒入定");
        assert_eq!(at(TIER1_SECS - 0.1), 0, "差一點淺定仍是 0");
        assert_eq!(at(TIER1_SECS), 1, "坐滿淺定門檻＝1");
        assert_eq!(at(TIER2_SECS - 0.1), 1, "未滿入定仍是淺定");
        assert_eq!(at(TIER2_SECS), 2, "坐滿入定門檻＝2");
        assert_eq!(at(TIER3_SECS - 0.1), 2, "未滿深定仍是入定");
        assert_eq!(at(TIER3_SECS), 3, "坐滿深定門檻＝3");
        assert_eq!(at(TIER3_SECS + 999.0), 3, "坐再久仍封頂在深定 3");
    }

    /// 封頂判定：坐到 TIER3_SECS 才算撐到最深、自動結束。
    #[test]
    fn is_capped_at_deepest_tier() {
        let m = Meditation::new(Instant::now(), 0.0, 0.0);
        assert!(!m.is_capped(m.start + Duration::from_secs_f32(TIER3_SECS - 0.5)));
        assert!(m.is_capped(m.start + Duration::from_secs_f32(TIER3_SECS)));
        assert!(m.is_capped(m.start + Duration::from_secs_f32(TIER3_SECS + 10.0)));
    }

    /// 門檻單調遞增，且淺定＝改版前秒數（向後相容）。
    #[test]
    fn tier_thresholds_monotone_and_legacy_first() {
        assert!(TIER1_SECS < TIER2_SECS);
        assert!(TIER2_SECS < TIER3_SECS);
        assert_eq!(TIER1_SECS, MEDITATE_DURATION_SECS, "淺定門檻＝改版前的完成秒數");
    }

    /// 獎勵：淺定＝改版前固定值（向後相容）、逐重嚴格遞增、越界回 0。
    #[test]
    fn tier_rewards_backcompat_and_increasing() {
        // 淺定＝改版前固定獎勵。
        assert_eq!(tier_hp_pct(1), MEDITATE_HP_PCT);
        assert_eq!(tier_ether(1), MEDITATE_ETHER);
        // 逐重嚴格遞增（坐得越深回得越多）。
        assert!(tier_hp_pct(2) > tier_hp_pct(1));
        assert!(tier_hp_pct(3) > tier_hp_pct(2));
        assert!(tier_ether(2) > tier_ether(1));
        assert!(tier_ether(3) > tier_ether(2));
        // 第 0 重與越界皆 0（沒入定就沒獎勵）。
        assert_eq!(tier_hp_pct(0), 0);
        assert_eq!(tier_ether(0), 0);
        assert_eq!(tier_hp_pct(TIER_COUNT + 1), 0);
        assert_eq!(tier_ether(99), 0);
        // HP 比例不超過 100%（不會回超過滿血）。
        for t in 1..=TIER_COUNT {
            assert!(tier_hp_pct(t) <= 100, "第 {t} 重 HP 比例不該超過 100%");
        }
    }

    /// 每一重都有名稱、第 0 重退回通稱（不 panic、不空字串）。
    #[test]
    fn tier_names_present() {
        for t in 1..=TIER_COUNT {
            assert!(!tier_name(t).is_empty());
        }
        assert_eq!(tier_name(0), "靜心");
        assert_eq!(tier_name(255), "靜心");
    }
}
