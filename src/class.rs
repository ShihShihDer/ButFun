//! 職業選擇系統（ROADMAP 28）。
//!
//! 純邏輯層：職業枚舉 + 加成函式，無 IO / 無 WebSocket。
//! 玩家可在任意時刻選擇一個職業；更換職業即時生效（除了戰士 HP 需在 ws.rs 同步）。
//!
//! 五大職業與核心加成：
//!   - 戰士 (Warrior)   ⚔️：攻擊力 +5、最大 HP +10
//!   - 農夫 (Farmer)    🌾：農地收割 +3 乙太/次、NPC 收購 +25%
//!   - 工匠 (Artisan)   🔧：合成每項素材 -1（最少 1）
//!   - 探索者 (Explorer) 🧭：星際旅行費 -10 乙太（最低不低於 10）
//!   - 商人 (Merchant)  💰：NPC 所有收購 +50%

use serde::{Deserialize, Serialize};

/// 職業種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobClass {
    Warrior,
    Farmer,
    Artisan,
    Explorer,
    Merchant,
}

impl JobClass {
    /// 從字串解析職業（對應 wire key）。
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "warrior"  => Some(Self::Warrior),
            "farmer"   => Some(Self::Farmer),
            "artisan"  => Some(Self::Artisan),
            "explorer" => Some(Self::Explorer),
            "merchant" => Some(Self::Merchant),
            _ => None,
        }
    }

    /// 職業的 wire key（小寫 snake_case）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Warrior  => "warrior",
            Self::Farmer   => "farmer",
            Self::Artisan  => "artisan",
            Self::Explorer => "explorer",
            Self::Merchant => "merchant",
        }
    }
}

/// 戰士：攻擊力加成（+5）。
pub fn combat_bonus(class: Option<JobClass>) -> u32 {
    if class == Some(JobClass::Warrior) { 5 } else { 0 }
}

/// 戰士：最大 HP 加成（+10）。
pub fn hp_bonus(class: Option<JobClass>) -> u32 {
    if class == Some(JobClass::Warrior) { 10 } else { 0 }
}

/// 農夫：農地收割額外乙太（+3）。
pub fn harvest_ether_bonus(class: Option<JobClass>) -> u32 {
    if class == Some(JobClass::Farmer) { 3 } else { 0 }
}

/// 農夫 / 商人：NPC 收購乙太加成百分比。
/// 農夫 +25%，商人 +50%，兩者不疊加（取較大值）。
pub fn npc_sell_bonus_pct(class: Option<JobClass>) -> u32 {
    match class {
        Some(JobClass::Merchant) => 50,
        Some(JobClass::Farmer)   => 25,
        _ => 0,
    }
}

/// 工匠：合成時每項素材的減量（-1，最少 1）。
pub fn crafting_reduction(class: Option<JobClass>) -> u32 {
    if class == Some(JobClass::Artisan) { 1 } else { 0 }
}

/// 探索者：星際旅行費減少量（-10，最終費用不低於 10）。
pub fn travel_cost_reduction(class: Option<JobClass>) -> u32 {
    if class == Some(JobClass::Explorer) { 10 } else { 0 }
}

/// 套用 NPC 收購加成後的乙太收益。
/// `base_earned` 為原始收益；回傳加成後的總收益。
pub fn apply_npc_bonus(class: Option<JobClass>, base_earned: u32) -> u32 {
    let pct = npc_sell_bonus_pct(class);
    if pct == 0 {
        return base_earned;
    }
    base_earned.saturating_add(base_earned.saturating_mul(pct) / 100)
}

/// 套用旅行費折扣後的實際費用。
/// `base_cost` 為原始費用；回傳折扣後費用（最低 10）。
pub fn apply_travel_discount(class: Option<JobClass>, base_cost: u32) -> u32 {
    let reduction = travel_cost_reduction(class);
    base_cost.saturating_sub(reduction).max(10)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warrior_combat_bonus() {
        assert_eq!(combat_bonus(Some(JobClass::Warrior)), 5);
        assert_eq!(combat_bonus(Some(JobClass::Farmer)), 0);
        assert_eq!(combat_bonus(None), 0);
    }

    #[test]
    fn warrior_hp_bonus() {
        assert_eq!(hp_bonus(Some(JobClass::Warrior)), 10);
        assert_eq!(hp_bonus(None), 0);
    }

    #[test]
    fn farmer_harvest_bonus() {
        assert_eq!(harvest_ether_bonus(Some(JobClass::Farmer)), 3);
        assert_eq!(harvest_ether_bonus(Some(JobClass::Warrior)), 0);
        assert_eq!(harvest_ether_bonus(None), 0);
    }

    #[test]
    fn npc_bonus_merchant_beats_farmer() {
        assert!(npc_sell_bonus_pct(Some(JobClass::Merchant)) > npc_sell_bonus_pct(Some(JobClass::Farmer)));
    }

    #[test]
    fn npc_bonus_applies_correctly() {
        // 商人：+50%，10 乙太 → 15。
        assert_eq!(apply_npc_bonus(Some(JobClass::Merchant), 10), 15);
        // 農夫：+25%，8 乙太 → 10（整數截斷）。
        assert_eq!(apply_npc_bonus(Some(JobClass::Farmer), 8), 10);
        // 其他：無加成。
        assert_eq!(apply_npc_bonus(Some(JobClass::Warrior), 10), 10);
        assert_eq!(apply_npc_bonus(None, 10), 10);
    }

    #[test]
    fn artisan_crafting_reduction() {
        assert_eq!(crafting_reduction(Some(JobClass::Artisan)), 1);
        assert_eq!(crafting_reduction(Some(JobClass::Warrior)), 0);
        assert_eq!(crafting_reduction(None), 0);
    }

    #[test]
    fn explorer_travel_discount() {
        // 30 乙太旅費 → 20（折扣 10）。
        assert_eq!(apply_travel_discount(Some(JobClass::Explorer), 30), 20);
        // 非探索者無折扣。
        assert_eq!(apply_travel_discount(Some(JobClass::Merchant), 30), 30);
        // 費用最低 10，即使減去超過剩餘也不低於 10。
        assert_eq!(apply_travel_discount(Some(JobClass::Explorer), 5), 10);
        assert_eq!(apply_travel_discount(None, 30), 30);
    }

    #[test]
    fn from_str_round_trips() {
        for class in [
            JobClass::Warrior, JobClass::Farmer, JobClass::Artisan,
            JobClass::Explorer, JobClass::Merchant,
        ] {
            assert_eq!(JobClass::from_str(class.as_str()), Some(class));
        }
        assert_eq!(JobClass::from_str("unknown"), None);
    }
}
