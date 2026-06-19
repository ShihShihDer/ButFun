//! ROADMAP 408 臨陣格擋——戰鬥從「走近自動互毆」升級成「看準時機擋下反擊」。
//!
//! 戰鬥（`combat.rs` / `enemy_field.rs`）是世上最古老的核心動作線：自開服就是「走近一隻敵人、
//! 一鍵互毆」，每秒承受一次敵人的反擊威脅（`game.rs` 反擊迴圈）——除了「站位／走開」之外，
//! 玩家對「挨打」這件事從沒有任何技巧介入，跟 403 之前的伐木一樣，是核心線裡唯一沒有玩法的環。
//! 本切片給戰鬥第一個**技巧機制**：被敵人威脅時按「🛡️ 格擋」進入短暫的備防，盤面浮一圈脈動的
//! 格擋環，**在環收束到甜蜜點的瞬間按下＝完美格擋**，凝起一面乙太護盾、把接下來幾秒的反擊傷害
//! 大幅卸掉；時機偏了只能擋下一部分，全沒抓到就白費這一防。
//!
//! 全純記憶體、純函式、零持久化、零 migration、零 LLM。
//!
//! **換骨架（戰鬥線的第一個技巧、且是第一個「防禦」技巧）**：不同於 403 伐木的「節奏連擊」
//! （連續累積乾淨擊），格擋是**單次決斷的反應格擋**（一次按下、依時機分三檔、凝出一面限時護盾），
//! 也不同於釣魚反應計時／觀星連線／採礦賭運／料理記譜／汲泉準星——這些全是**採集／生產**的玩法，
//! 格擋是世界第一個**戰鬥防禦**玩法。
//!
//! **平衡（誠實交代，純療癒向、零經濟擾動）**：格擋**只減少玩家「受到」的反擊傷害**，
//! 完全不碰攻擊力、不改掉落、不送物品／乙太、不縮短擊倒（攻擊仍走既有手動 `Attack`、行為一字不改）。
//! 護盾再強也**封頂卸 85%、永不完全免傷**（強敵照樣咬得到你），且護盾僅持續幾秒、格擋有冷卻，
//! 無法靠連續格擋達成永久無敵——戰鬥仍是「有來有回」，只是看準時機的玩家挨打更少。整數向下取整的
//! 卸傷對小傷害近乎無感，格擋的價值集中在擋下真正的重擊，療癒向、零失控。

/// 格擋環脈動週期（秒）：環每這麼久收束到甜蜜點一次，玩家瞄著甜蜜點按下格擋。
/// 偏慢，讓玩家來得及瞄準、也壓低網路延遲造成的判定漂移。
pub const GUARD_BEAT_SECS: f32 = 1.2;

/// 完美格擋窗口（秒）：按下時刻與最近甜蜜點的距離在此內＝「完美格擋」。
pub const PERFECT_WINDOW: f32 = 0.16;

/// 普通格擋窗口（秒）：距甜蜜點在此內（但超出完美窗）＝「擋下一部分」；再外＝沒抓到。
/// 療癒向、刻意比完美窗寬鬆好上手。
pub const PARTIAL_WINDOW: f32 = 0.38;

/// 備防逾時（秒）：開了格擋後這麼久沒按下就自動解除這趟（不罰冷卻、可重來）。
pub const GUARD_TIMEOUT_SECS: f32 = 5.0;

/// 格擋結算後的冷卻（秒）：只擋「開新一趟格擋」，避免靠連續格擋達成永久無敵。
pub const GUARD_COOLDOWN_SECS: f32 = 3.5;

/// 一次格擋按下的判定檔位，依按下時機距甜蜜點的遠近分三級。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardTier {
    /// 沒抓到時機：白費這一防，不凝護盾。
    Whiff,
    /// 抓到一部分：凝出較弱、較短的護盾。
    Partial,
    /// 完美格擋：凝出最強、最久的護盾。
    Perfect,
}

impl GuardTier {
    /// 與前端事件字串對齊的 snake_case 線格式（穩定契約）。
    pub fn wire(self) -> &'static str {
        match self {
            GuardTier::Whiff => "whiff",
            GuardTier::Partial => "partial",
            GuardTier::Perfect => "perfect",
        }
    }

    /// 這一檔卸掉的反擊傷害百分比。封頂 85%——再完美也永不完全免傷，戰鬥仍有來有回。
    pub fn block_pct(self) -> u32 {
        match self {
            GuardTier::Whiff => 0,
            GuardTier::Partial => 45,
            GuardTier::Perfect => 85,
        }
    }

    /// 這一檔凝出的護盾持續秒數（反擊每秒一次，故僅覆蓋一兩跳，無法永久無敵）。
    pub fn shield_secs(self) -> f32 {
        match self {
            GuardTier::Whiff => 0.0,
            GuardTier::Partial => 1.2,
            GuardTier::Perfect => 2.0,
        }
    }

    /// 成功格擋得的戰士熟練度（沒抓到不給；越精準越精進防身手藝）。
    pub fn mastery_xp(self) -> u32 {
        match self {
            GuardTier::Whiff => 0,
            GuardTier::Partial => 1,
            GuardTier::Perfect => 2,
        }
    }
}

/// 給定按下時刻（距開格擋經過的秒數），回傳該時刻在脈動環上的相位 [0,1)。
/// 0＝正落在甜蜜點上。純函式——前端用同一條公式渲染收束的格擋環。
pub fn beat_fraction(elapsed_secs: f32) -> f32 {
    let mut frac = (elapsed_secs.max(0.0) / GUARD_BEAT_SECS).fract();
    if frac < 0.0 {
        frac += 1.0; // fract 對負數回負；上方已 max(0)，此為雙保險
    }
    frac
}

/// 按下時刻距最近甜蜜點（相位 0 或 1）的距離，換算回秒。純函式、確定可重現。
fn beat_dist_secs(elapsed_secs: f32) -> f32 {
    let frac = beat_fraction(elapsed_secs);
    frac.min(1.0 - frac) * GUARD_BEAT_SECS
}

/// 判定某時刻按下格擋落在哪一檔：距甜蜜點越近檔位越高。
/// 伺服器判定與前端渲染窗口用同一套標準。
pub fn tier_at(elapsed_secs: f32) -> GuardTier {
    let dist = beat_dist_secs(elapsed_secs);
    if dist <= PERFECT_WINDOW {
        GuardTier::Perfect
    } else if dist <= PARTIAL_WINDOW {
        GuardTier::Partial
    } else {
        GuardTier::Whiff
    }
}

/// 一趟進行中的格擋備防（記憶體前置、`Copy`、重啟清空）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GuardBrace {
    /// 距開格擋經過的時間（秒）。
    elapsed: f32,
}

impl GuardBrace {
    /// 開一趟新的格擋備防。
    pub fn start() -> Self {
        GuardBrace { elapsed: 0.0 }
    }

    /// 距開格擋經過的時間（秒）——隨快照廣播給前端，渲染格擋環的收束相位。
    pub fn elapsed(self) -> f32 {
        self.elapsed
    }

    /// 推進一個 tick：累加時間，回傳是否已逾時（呼叫端據此解除這趟、不罰冷卻）。
    /// 負或零 dt 不前進（守時間單調）。
    pub fn advance(&mut self, dt: f32) -> bool {
        if dt > 0.0 {
            self.elapsed += dt;
        }
        self.elapsed >= GUARD_TIMEOUT_SECS
    }

    /// 按下格擋：以當下時刻判定落在哪一檔。呼叫端據此凝護盾＋給熟練度＋起冷卻。
    pub fn resolve(self) -> GuardTier {
        tier_at(self.elapsed)
    }
}

/// 一面凝起的乙太護盾（記憶體前置、`Copy`、重啟清空）：限時卸掉反擊傷害的一部分。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GuardShield {
    /// 卸掉的傷害百分比（0~85）。
    pct: u32,
    /// 剩餘秒數，隨 tick 遞減；歸零即消散。
    remaining: f32,
}

impl GuardShield {
    /// 依格擋檔位凝出護盾；沒抓到（無卸傷／無持續）回 None，呼叫端據此不上盾。
    pub fn from_tier(tier: GuardTier) -> Option<Self> {
        let pct = tier.block_pct();
        let secs = tier.shield_secs();
        if pct > 0 && secs > 0.0 {
            Some(GuardShield { pct, remaining: secs })
        } else {
            None
        }
    }

    /// 卸傷百分比——供前端畫護盾微光的強度。
    pub fn pct(self) -> u32 {
        self.pct
    }

    /// 剩餘秒數。
    pub fn remaining(self) -> f32 {
        self.remaining
    }

    /// 推進一個 tick：遞減剩餘秒數，回傳護盾是否已消散（呼叫端據此清掉）。
    /// 負或零 dt 不前進。
    pub fn advance(&mut self, dt: f32) -> bool {
        if dt > 0.0 {
            self.remaining -= dt;
        }
        self.remaining <= 0.0
    }

    /// 把一筆反擊傷害依卸傷百分比減少（整數向下取整，故小傷害近乎無感、價值集中在重擊）。
    /// 永不卸到負數，也因 85% 封頂而永不完全免傷。
    pub fn reduce(self, dmg: u32) -> u32 {
        dmg.saturating_sub(dmg.saturating_mul(self.pct) / 100)
    }
}

// ─── 單元測試 ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beat_fraction_within_bounds() {
        for i in 0..1000 {
            let t = i as f32 * 0.019;
            let f = beat_fraction(t);
            assert!((0.0..1.0).contains(&f), "beat_fraction({t}) = {f} 超出 [0,1)");
        }
    }

    #[test]
    fn beat_fraction_zero_at_beats() {
        assert!(beat_fraction(0.0).abs() < 1e-4);
        assert!(beat_fraction(GUARD_BEAT_SECS).abs() < 1e-4);
        assert!(beat_fraction(GUARD_BEAT_SECS * 4.0).abs() < 1e-4);
    }

    #[test]
    fn beat_fraction_handles_negative() {
        // 異常負時間不 panic、不出界。
        let f = beat_fraction(-5.0);
        assert!((0.0..1.0).contains(&f));
    }

    #[test]
    fn tier_grades_by_timing() {
        // 正落在甜蜜點＝完美。
        assert_eq!(tier_at(0.0), GuardTier::Perfect);
        assert_eq!(tier_at(GUARD_BEAT_SECS), GuardTier::Perfect);
        // 完美窗內仍完美。
        assert_eq!(tier_at(PERFECT_WINDOW - 0.01), GuardTier::Perfect);
        // 完美窗外、普通窗內＝擋一部分。
        assert_eq!(tier_at(PERFECT_WINDOW + 0.02), GuardTier::Partial);
        assert_eq!(tier_at(PARTIAL_WINDOW - 0.01), GuardTier::Partial);
        // 普通窗外＝沒抓到。
        assert_eq!(tier_at(PARTIAL_WINDOW + 0.02), GuardTier::Whiff);
        // 兩拍正中間（離甜蜜點最遠）＝沒抓到。
        assert_eq!(tier_at(GUARD_BEAT_SECS * 0.5), GuardTier::Whiff);
    }

    #[test]
    fn tier_perks_are_monotonic() {
        // 越高檔卸傷越多、護盾越久、熟練度越高；沒抓到一律 0。
        assert_eq!(GuardTier::Whiff.block_pct(), 0);
        assert!(GuardTier::Perfect.block_pct() > GuardTier::Partial.block_pct());
        assert!(GuardTier::Partial.block_pct() > GuardTier::Whiff.block_pct());
        assert!(GuardTier::Perfect.block_pct() <= 85, "永不完全免傷");
        assert!(GuardTier::Perfect.shield_secs() > GuardTier::Partial.shield_secs());
        assert!(GuardTier::Whiff.shield_secs() == 0.0);
        assert!(GuardTier::Perfect.mastery_xp() > GuardTier::Partial.mastery_xp());
        assert_eq!(GuardTier::Whiff.mastery_xp(), 0);
    }

    #[test]
    fn wire_strings_stable() {
        assert_eq!(GuardTier::Perfect.wire(), "perfect");
        assert_eq!(GuardTier::Partial.wire(), "partial");
        assert_eq!(GuardTier::Whiff.wire(), "whiff");
    }

    #[test]
    fn brace_starts_zeroed_and_times_out() {
        let mut g = GuardBrace::start();
        assert!((g.elapsed() - 0.0).abs() < 1e-6);
        assert!(!g.advance(GUARD_TIMEOUT_SECS - 0.5));
        assert!(g.advance(1.0), "累過逾時門檻回 true");
    }

    #[test]
    fn brace_ignores_non_positive_dt() {
        let mut g = GuardBrace::start();
        let before = g;
        assert!(!g.advance(0.0));
        assert!(!g.advance(-3.0));
        assert_eq!(g, before, "非正 dt 不應改變狀態");
    }

    #[test]
    fn brace_resolve_uses_timing() {
        // 開格擋時 elapsed=0 落在甜蜜點＝完美。
        let g = GuardBrace::start();
        assert_eq!(g.resolve(), GuardTier::Perfect);
        // 推進到兩拍正中間＝沒抓到。
        let mut g2 = GuardBrace::start();
        g2.advance(GUARD_BEAT_SECS * 0.5);
        assert_eq!(g2.resolve(), GuardTier::Whiff);
    }

    #[test]
    fn shield_only_for_blocking_tiers() {
        assert!(GuardShield::from_tier(GuardTier::Whiff).is_none(), "沒抓到不上盾");
        assert!(GuardShield::from_tier(GuardTier::Partial).is_some());
        assert!(GuardShield::from_tier(GuardTier::Perfect).is_some());
    }

    #[test]
    fn shield_reduces_damage_and_never_negates() {
        let perfect = GuardShield::from_tier(GuardTier::Perfect).unwrap();
        // 重擊被大幅卸掉，但永不歸零。
        assert_eq!(perfect.reduce(100), 15, "85% 卸傷：100→15");
        assert!(perfect.reduce(20) > 0, "永不完全免傷");
        // 小傷害整數向下取整近乎無感（價值集中在重擊）。
        assert_eq!(perfect.reduce(1), 1);
        // 普通檔卸得較少。
        let partial = GuardShield::from_tier(GuardTier::Partial).unwrap();
        assert!(partial.reduce(100) > perfect.reduce(100), "普通檔卸傷比完美少");
        assert_eq!(partial.reduce(100), 55, "45% 卸傷：100→55");
    }

    #[test]
    fn shield_ticks_down_and_expires() {
        let mut s = GuardShield::from_tier(GuardTier::Perfect).unwrap();
        assert!(!s.advance(0.5));
        assert!(s.remaining() > 0.0);
        assert!(s.advance(10.0), "耗盡剩餘秒數即消散");
    }

    #[test]
    fn shield_ignores_non_positive_dt() {
        let mut s = GuardShield::from_tier(GuardTier::Partial).unwrap();
        let before = s;
        assert!(!s.advance(0.0));
        assert!(!s.advance(-1.0));
        assert_eq!(s, before);
    }
}
