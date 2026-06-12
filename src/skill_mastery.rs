//! 技能使用型熟練度系統（ROADMAP 153）。
//!
//! 五條主動技能各有獨立使用次數計數；次數越高，技能效果越強（自然成長，無固定門檻）。
//! 成長方向：
//!   - 所有技能：冷卻時間自然縮短（最多 -30%）。
//!   - 戰吼 (Warcry)    ：群攻範圍半徑擴大。
//!   - 豐饒術 (Bounty)  ：額外採集物品數增加。
//!   - 精密合成(Precision)：額外產出件數增加。
//!   - 風之步 (Gale)    ：瞬移距離加長。
//!   - 議價術 (Haggle)  ：額外乙太收益百分比增加。

use serde::{Deserialize, Serialize};

// ── 成長常數 ─────────────────────────────────────────────────────────────────

/// 每次使用縮減的冷卻比例（0.003 = 0.3%/次）；上限 MAX_CD_REDUCTION。
pub const CD_REDUCTION_PER_USE: f32 = 0.003;
/// 冷卻縮短最高比例（30%）。
pub const MAX_CD_REDUCTION: f32 = 0.30;

/// 戰吼：每 10 次增加的群攻半徑（像素）；上限 MAX_WARCRY_BONUS_PX。
pub const WARCRY_REACH_PER_10_USES: f32 = 16.0;
pub const MAX_WARCRY_BONUS_PX: f32 = 160.0;

/// 豐饒術：每 15 次增加的額外物品數；上限 MAX_BOUNTY_BONUS_QTY。
pub const BOUNTY_QTY_PER_15_USES: u32 = 1;
pub const MAX_BOUNTY_BONUS_QTY: u32 = 5;

/// 精密合成：每 20 次增加的額外產出件數；上限 MAX_PRECISION_BONUS_OUT。
pub const PRECISION_OUT_PER_20_USES: u32 = 1;
pub const MAX_PRECISION_BONUS_OUT: u32 = 3;

/// 風之步：每 10 次增加的瞬移距離（像素）；上限 MAX_GALE_BONUS_PX。
pub const GALE_DASH_PER_10_USES: f32 = 24.0;
pub const MAX_GALE_BONUS_PX: f32 = 240.0;

/// 議價術：每 10 次增加的額外乙太收益百分比（整數）；上限 MAX_HAGGLE_BONUS_PCT。
pub const HAGGLE_PCT_PER_10_USES: u32 = 3;
pub const MAX_HAGGLE_BONUS_PCT: u32 = 30;

// ── 資料結構 ──────────────────────────────────────────────────────────────────

/// 五條主動技能的使用次數（熟練度來源）。
/// `#[serde(default)]` 讓舊存檔安全讀回全 0。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SkillMasteries {
    #[serde(default)]
    pub warcry:    u32,
    #[serde(default)]
    pub bounty:    u32,
    #[serde(default)]
    pub precision: u32,
    #[serde(default)]
    pub gale:      u32,
    #[serde(default)]
    pub haggle:    u32,
}

impl SkillMasteries {
    /// 增加指定技能使用次數（溢位安全）。
    pub fn increment(&mut self, kind: crate::active_skill::ActiveSkillKind) {
        use crate::active_skill::ActiveSkillKind::*;
        match kind {
            Warcry    => self.warcry    = self.warcry   .saturating_add(1),
            Bounty    => self.bounty    = self.bounty   .saturating_add(1),
            Precision => self.precision = self.precision.saturating_add(1),
            Gale      => self.gale      = self.gale     .saturating_add(1),
            Haggle    => self.haggle    = self.haggle   .saturating_add(1),
        }
    }

    /// 指定技能的冷卻縮減乘數（0.7 ~ 1.0）。
    /// 傳入「原始冷卻秒數」，回傳「實際冷卻秒數」。
    pub fn effective_cooldown(&self, kind: crate::active_skill::ActiveSkillKind, base_cd: f32) -> f32 {
        let uses = self.uses(kind);
        let reduction = (uses as f32 * CD_REDUCTION_PER_USE).min(MAX_CD_REDUCTION);
        base_cd * (1.0 - reduction)
    }

    /// 戰吼額外群攻半徑（px）。
    pub fn warcry_bonus_reach_px(&self) -> f32 {
        let extra = (self.warcry / 10) as f32 * WARCRY_REACH_PER_10_USES;
        extra.min(MAX_WARCRY_BONUS_PX)
    }

    /// 豐饒術額外物品數（基礎值之外再加）。
    pub fn bounty_bonus_qty(&self) -> u32 {
        let extra = (self.bounty / 15) * BOUNTY_QTY_PER_15_USES;
        extra.min(MAX_BOUNTY_BONUS_QTY)
    }

    /// 精密合成額外產出件數（基礎值之外再加）。
    pub fn precision_bonus_output(&self) -> u32 {
        let extra = (self.precision / 20) * PRECISION_OUT_PER_20_USES;
        extra.min(MAX_PRECISION_BONUS_OUT)
    }

    /// 風之步額外瞬移距離（px）。
    pub fn gale_bonus_dash_px(&self) -> f32 {
        let extra = (self.gale / 10) as f32 * GALE_DASH_PER_10_USES;
        extra.min(MAX_GALE_BONUS_PX)
    }

    /// 議價術額外乙太收益百分比（整數，如 15 = +15%）。
    pub fn haggle_bonus_pct(&self) -> u32 {
        let extra = (self.haggle / 10) * HAGGLE_PCT_PER_10_USES;
        extra.min(MAX_HAGGLE_BONUS_PCT)
    }

    fn uses(&self, kind: crate::active_skill::ActiveSkillKind) -> u32 {
        use crate::active_skill::ActiveSkillKind::*;
        match kind {
            Warcry    => self.warcry,
            Bounty    => self.bounty,
            Precision => self.precision,
            Gale      => self.gale,
            Haggle    => self.haggle,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::active_skill::ActiveSkillKind;

    #[test]
    fn increment_increases_uses() {
        let mut m = SkillMasteries::default();
        m.increment(ActiveSkillKind::Warcry);
        m.increment(ActiveSkillKind::Warcry);
        assert_eq!(m.warcry, 2);
    }

    #[test]
    fn cooldown_zero_uses_no_reduction() {
        let m = SkillMasteries::default();
        let base = 60.0_f32;
        let cd = m.effective_cooldown(ActiveSkillKind::Warcry, base);
        assert!((cd - base).abs() < 0.001, "零使用不應縮短冷卻");
    }

    #[test]
    fn cooldown_caps_at_30pct() {
        let mut m = SkillMasteries::default();
        // 200 次使用 × 0.3% = 60%，但上限為 30%
        for _ in 0..200 {
            m.increment(ActiveSkillKind::Bounty);
        }
        let base = 120.0_f32;
        let cd = m.effective_cooldown(ActiveSkillKind::Bounty, base);
        // 不應低於 base × 0.70 = 84.0
        assert!(cd >= base * 0.70 - 0.01, "冷卻縮短不應超過 30%，cd={cd}");
        assert!(cd < base, "100 次後應有縮短");
    }

    #[test]
    fn warcry_bonus_reach_caps() {
        let mut m = SkillMasteries::default();
        for _ in 0..1000 {
            m.increment(ActiveSkillKind::Warcry);
        }
        assert_eq!(m.warcry_bonus_reach_px(), MAX_WARCRY_BONUS_PX);
    }

    #[test]
    fn bounty_bonus_qty_grows_per_15_uses() {
        let mut m = SkillMasteries::default();
        for _ in 0..15 {
            m.increment(ActiveSkillKind::Bounty);
        }
        assert_eq!(m.bounty_bonus_qty(), 1);
        for _ in 0..15 {
            m.increment(ActiveSkillKind::Bounty);
        }
        assert_eq!(m.bounty_bonus_qty(), 2);
    }

    #[test]
    fn precision_bonus_grows_per_20_uses() {
        let mut m = SkillMasteries::default();
        for _ in 0..20 {
            m.increment(ActiveSkillKind::Precision);
        }
        assert_eq!(m.precision_bonus_output(), 1);
    }

    #[test]
    fn gale_bonus_px_grows() {
        let mut m = SkillMasteries::default();
        for _ in 0..10 {
            m.increment(ActiveSkillKind::Gale);
        }
        assert!((m.gale_bonus_dash_px() - GALE_DASH_PER_10_USES).abs() < 0.01);
    }

    #[test]
    fn haggle_bonus_pct_grows() {
        let mut m = SkillMasteries::default();
        for _ in 0..10 {
            m.increment(ActiveSkillKind::Haggle);
        }
        assert_eq!(m.haggle_bonus_pct(), HAGGLE_PCT_PER_10_USES);
    }

    #[test]
    fn haggle_bonus_pct_caps() {
        let mut m = SkillMasteries::default();
        for _ in 0..1000 {
            m.increment(ActiveSkillKind::Haggle);
        }
        assert_eq!(m.haggle_bonus_pct(), MAX_HAGGLE_BONUS_PCT);
    }

    #[test]
    fn default_all_zero() {
        let m = SkillMasteries::default();
        assert_eq!(m.warcry, 0);
        assert_eq!(m.bounty_bonus_qty(), 0);
        assert_eq!(m.precision_bonus_output(), 0);
        assert_eq!(m.gale_bonus_dash_px(), 0.0);
        assert_eq!(m.haggle_bonus_pct(), 0);
    }
}
