//! 角色屬性加點系統（ROADMAP 152）。
//!
//! 每升等獲得 POINTS_PER_LEVEL 個屬性點，玩家可自由分配到四條屬性：
//!   HP / 攻擊 / 移動速度 / 攻擊速度（冷卻縮短）。
//! 設計哲學：直觀、即時生效、個性化 build——與「使用型熟練（ROADMAP 153）」並存，
//! 熟練給職能成長，加點給個人化。

use serde::{Deserialize, Serialize};

/// 玩家的屬性加點分配。
/// `#[serde(default)]` 讓舊存檔（不含這些欄位）安全讀為全 0。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct StatPoints {
    /// 未分配的屬性點。
    #[serde(default)]
    pub unspent: u32,
    /// 已分配到 HP 的點數（每點 +HP_PER_POINT max HP）。
    #[serde(default)]
    pub hp: u32,
    /// 已分配到攻擊的點數（每點 +ATTACK_PER_POINT 攻擊）。
    #[serde(default)]
    pub attack: u32,
    /// 已分配到移動速度的點數（每點 +SPEED_PCT_PER_POINT% 速度）。
    #[serde(default)]
    pub speed: u32,
    /// 已分配到攻擊速度的點數（每點減少 ATK_SPEED_PCT_PER_POINT% 攻擊冷卻）。
    #[serde(default)]
    pub atk_speed: u32,
}

/// 每升等獲得的屬性點數。
pub const POINTS_PER_LEVEL: u32 = 2;

/// HP 加點：每點加多少 max HP。
pub const HP_PER_POINT: u32 = 5;

/// 攻擊加點：每點加多少攻擊力。
pub const ATTACK_PER_POINT: u32 = 2;

/// 速度加點：每點增加速度百分比（整數，如 8 = +8%）。
pub const SPEED_PCT_PER_POINT: u32 = 8;

/// 攻擊速度加點：每點減少攻擊冷卻百分比（整數，如 5 = -5%）。
pub const ATK_SPEED_PCT_PER_POINT: u32 = 5;

/// 攻擊冷卻下限（秒），攻擊速度加點不可突破此值。
pub const ATTACK_COOLDOWN_MIN: f32 = 0.25;

/// 可分配的屬性種類標識。
pub const STAT_HP: &str = "hp";
pub const STAT_ATTACK: &str = "attack";
pub const STAT_SPEED: &str = "speed";
pub const STAT_ATK_SPEED: &str = "atk_speed";

impl StatPoints {
    /// 計算移動速度乘數（1.0 = 原速，> 1.0 = 加速）。
    pub fn speed_mult(&self) -> f32 {
        1.0 + (self.speed * SPEED_PCT_PER_POINT) as f32 / 100.0
    }

    /// 計算有效攻擊冷卻（秒），以基礎冷卻為輸入。
    pub fn effective_attack_cooldown(&self, base_cooldown: f32) -> f32 {
        let reduction = (self.atk_speed * ATK_SPEED_PCT_PER_POINT) as f32 / 100.0;
        let reduced = base_cooldown * (1.0 - reduction);
        reduced.max(ATTACK_COOLDOWN_MIN)
    }

    /// 嘗試將 `points` 個點分配到指定屬性。
    /// 若 `stat` 名稱無效或可用點數不足，回傳 Err。
    pub fn allocate(&mut self, stat: &str, points: u32) -> Result<(), &'static str> {
        if points == 0 {
            return Err("points 不可為 0");
        }
        if self.unspent < points {
            return Err("屬性點不足");
        }
        match stat {
            STAT_HP        => self.hp        += points,
            STAT_ATTACK    => self.attack    += points,
            STAT_SPEED     => self.speed     += points,
            STAT_ATK_SPEED => self.atk_speed += points,
            _ => return Err("未知屬性種類"),
        }
        self.unspent -= points;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocate_deducts_unspent() {
        let mut s = StatPoints { unspent: 4, ..Default::default() };
        s.allocate("hp", 2).unwrap();
        assert_eq!(s.unspent, 2);
        assert_eq!(s.hp, 2);
    }

    #[test]
    fn allocate_fails_when_insufficient() {
        let mut s = StatPoints { unspent: 1, ..Default::default() };
        assert!(s.allocate("hp", 2).is_err());
        assert_eq!(s.unspent, 1); // 未變動
    }

    #[test]
    fn allocate_fails_on_unknown_stat() {
        let mut s = StatPoints { unspent: 5, ..Default::default() };
        assert!(s.allocate("unknown", 1).is_err());
    }

    #[test]
    fn speed_mult_scales_correctly() {
        let s = StatPoints { speed: 2, ..Default::default() };
        // 2 點 × 8% = +16% → mult = 1.16
        let mult = s.speed_mult();
        assert!((mult - 1.16).abs() < 0.001, "mult={mult}");
    }

    #[test]
    fn effective_cooldown_respects_minimum() {
        // 20 點 atk_speed × 5% = 100% 減少，應被下限 0.25s 截住
        let s = StatPoints { atk_speed: 20, ..Default::default() };
        assert_eq!(s.effective_attack_cooldown(0.6), ATTACK_COOLDOWN_MIN);
    }

    #[test]
    fn effective_cooldown_partial_reduction() {
        // 4 點 × 5% = 20% 減少：0.6 × 0.8 = 0.48s
        let s = StatPoints { atk_speed: 4, ..Default::default() };
        let cd = s.effective_attack_cooldown(0.6);
        assert!((cd - 0.48).abs() < 0.001, "cd={cd}");
    }
}
