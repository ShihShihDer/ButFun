//! 精煉＋附魔系統（ROADMAP 37）。
//! 純邏輯、無 IO；裝備的強化與特效由此層決定，接線層（ws.rs）呼叫並套用。
//!
//! **精煉**：裝備 +1～+9；消耗同系材料；+4 起有失敗率（失敗降一級，不碎裝）。
//! **附魔**：用星球碎片賦予武器特效；一武器一附魔，可覆蓋。

use serde::{Deserialize, Serialize};

use crate::inventory::ItemKind;

// ───────────────────────── 附魔種類 ─────────────────────────

/// 附魔效果種類（武器槽專用，來自各星球碎片）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnchantKind {
    /// 翠幽碎片→吸血：擊殺敵人時回復 2 HP。
    Lifesteal,
    /// 赤焰碎片→灼燒：命中時額外造成 3 點火焰傷害。
    BurnBurst,
    /// 虛空碎片→暴擊：每 5 次攻擊第 5 次造成雙倍傷害。
    CritStrike,
    /// 霧醚碎片→共鳴：命中時額外造成 2 點霧醚傷害。
    AetherResonance,
    /// 源晶碎片→增幅：擊殺獲得的經驗值增加 30%。
    ExpBonus,
}

impl EnchantKind {
    /// 前端顯示名稱。
    pub fn display_name(self) -> &'static str {
        match self {
            EnchantKind::Lifesteal => "吸血",
            EnchantKind::BurnBurst => "灼燒",
            EnchantKind::CritStrike => "暴擊",
            EnchantKind::AetherResonance => "共鳴",
            EnchantKind::ExpBonus => "增幅",
        }
    }

    /// 前端 wire key（snake_case，與 serde 一致）。
    pub fn wire_key(self) -> &'static str {
        match self {
            EnchantKind::Lifesteal => "lifesteal",
            EnchantKind::BurnBurst => "burn_burst",
            EnchantKind::CritStrike => "crit_strike",
            EnchantKind::AetherResonance => "aether_resonance",
            EnchantKind::ExpBonus => "exp_bonus",
        }
    }
}

// ───────────────────────── 元資料結構 ─────────────────────────

/// 單一裝備槽的精煉與附魔元資料。序列化內嵌於 `EquipmentSlots` JSON（向後相容，舊資料 default 為 0 / None）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EquipmentMeta {
    /// 精煉等級（0 = 未精煉，上限 `MAX_REFINE`）。
    #[serde(default)]
    pub refine: u8,
    /// 附魔效果（`None` = 無附魔；只對武器槽有意義）。
    #[serde(default)]
    pub enchant: Option<EnchantKind>,
}

/// 精煉等級上限。
pub const MAX_REFINE: u8 = 9;

// ───────────────────────── 精煉邏輯 ─────────────────────────

/// 精煉等級帶來的攻擊力加成（每級 +1 ATK）。
pub fn refine_bonus_atk(level: u8) -> u32 {
    level as u32
}

/// 精煉等級帶來的防禦加成（每 2 級 +1 DEF）。
pub fn refine_bonus_def(level: u8) -> u32 {
    level as u32 / 2
}

/// 某裝備精煉時消耗的素材種類（`None` = 此物品不可精煉）。
pub fn refine_material(item: ItemKind) -> Option<ItemKind> {
    match item {
        ItemKind::Weapon => Some(ItemKind::Stone),
        ItemKind::CrystalBlade => Some(ItemKind::CrystalShard),
        ItemKind::CoralLance => Some(ItemKind::DeepSeaPearl),
        ItemKind::MushroomStaff => Some(ItemKind::MushroomSpore),
        ItemKind::RuneBlade => Some(ItemKind::AncientFragment),
        ItemKind::JadeBlade => Some(ItemKind::JadeShard),
        ItemKind::CrimsonBlade => Some(ItemKind::LavaCrystal),
        ItemKind::VoidBlade => Some(ItemKind::VoidShard),
        ItemKind::AetherBlade => Some(ItemKind::AetherShard),
        ItemKind::OriginBlade => Some(ItemKind::OriginShard),
        ItemKind::MeadowAmulet => Some(ItemKind::WildflowerSeed),
        ItemKind::CrystalShield => Some(ItemKind::CrystalShard),
        ItemKind::CosmicShield => Some(ItemKind::RiftShard),
        _ => None,
    }
}

/// 精煉到下一級的材料消耗量（`current_level + 1` 個）。
pub fn refine_cost_qty(current_level: u8) -> u32 {
    current_level as u32 + 1
}

/// 本次精煉是否失敗（+4 起有失敗率；使用 `attempt_index` 做確定性偽隨機）。
/// `attempt_index`：單調遞增的計數（如玩家總精煉次數），伺服器權威，防客戶端操控。
pub fn refine_fails(current_level: u8, attempt_index: u64) -> bool {
    // +1~+3 永遠成功；已達上限也不可再精煉（呼叫端應先擋住）。
    let fail_rate_pct: u64 = match current_level {
        0..=3 => return false,
        4 => 30,
        5 => 40,
        6 => 50,
        7 => 60,
        8 => 70,
        _ => return true, // current_level >= MAX_REFINE，不允許
    };
    // 確定性偽隨機：Knuth 乘法雜湊，結果 % 100。
    let h = attempt_index.wrapping_mul(2654435761) % 100;
    h < fail_rate_pct
}

// ───────────────────────── 附魔邏輯 ─────────────────────────

/// 星球碎片對應的附魔種類（`None` = 此物品不是附魔碎片）。
pub fn enchant_from_shard(shard: ItemKind) -> Option<EnchantKind> {
    match shard {
        ItemKind::JadeShard => Some(EnchantKind::Lifesteal),
        ItemKind::LavaCrystal => Some(EnchantKind::BurnBurst),
        ItemKind::VoidShard => Some(EnchantKind::CritStrike),
        ItemKind::AetherShard => Some(EnchantKind::AetherResonance),
        ItemKind::OriginShard => Some(EnchantKind::ExpBonus),
        _ => None,
    }
}

/// 附魔帶來的命中額外傷害（暴擊在呼叫端另算，此處不含）。
pub fn enchant_extra_damage(enchant: Option<EnchantKind>) -> u32 {
    match enchant {
        Some(EnchantKind::BurnBurst) => 3,
        Some(EnchantKind::AetherResonance) => 2,
        _ => 0,
    }
}

/// 本次攻擊是否觸發暴擊（每 5 次第 5 次，即 attempt_index % 5 == 4）。
pub fn is_crit_tick(attempt_index: u64) -> bool {
    attempt_index % 5 == 4
}

/// 附魔對經驗值的倍率（ExpBonus 給 1.3，其他給 1.0）。
pub fn enchant_exp_multiplier(enchant: Option<EnchantKind>) -> f32 {
    match enchant {
        Some(EnchantKind::ExpBonus) => 1.3,
        _ => 1.0,
    }
}

/// 附魔吸血效果：擊殺時回復 HP（Lifesteal 給 2，其他給 0）。
pub fn enchant_lifesteal_hp(enchant: Option<EnchantKind>) -> u32 {
    match enchant {
        Some(EnchantKind::Lifesteal) => 2,
        _ => 0,
    }
}

// ───────────────────────── 單元測試 ─────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refine_bonus_atk_scales_linearly() {
        assert_eq!(refine_bonus_atk(0), 0);
        assert_eq!(refine_bonus_atk(1), 1);
        assert_eq!(refine_bonus_atk(9), 9);
    }

    #[test]
    fn refine_bonus_def_scales_every_two_levels() {
        assert_eq!(refine_bonus_def(0), 0);
        assert_eq!(refine_bonus_def(1), 0);
        assert_eq!(refine_bonus_def(2), 1);
        assert_eq!(refine_bonus_def(4), 2);
        assert_eq!(refine_bonus_def(9), 4);
    }

    #[test]
    fn refine_cost_qty_is_level_plus_one() {
        assert_eq!(refine_cost_qty(0), 1);
        assert_eq!(refine_cost_qty(4), 5);
        assert_eq!(refine_cost_qty(8), 9);
    }

    #[test]
    fn refine_fails_never_at_level_three_or_below() {
        for attempt in 0..100 {
            assert!(!refine_fails(0, attempt));
            assert!(!refine_fails(3, attempt));
        }
    }

    #[test]
    fn refine_fails_always_at_max_level() {
        for attempt in 0..10 {
            assert!(refine_fails(MAX_REFINE, attempt));
        }
    }

    #[test]
    fn refine_fails_rate_at_level_four_is_roughly_thirty_pct() {
        let fails = (0u64..100).filter(|&i| refine_fails(4, i)).count();
        // 確定性偽隨機：30 次失敗（±0）。
        assert_eq!(fails, 30, "Level 4 應有 30/100 失敗率，實際 {fails}");
    }

    #[test]
    fn refine_material_known_weapons() {
        assert_eq!(refine_material(ItemKind::CrystalBlade), Some(ItemKind::CrystalShard));
        assert_eq!(refine_material(ItemKind::JadeBlade), Some(ItemKind::JadeShard));
        assert_eq!(refine_material(ItemKind::OriginBlade), Some(ItemKind::OriginShard));
        assert_eq!(refine_material(ItemKind::Weapon), Some(ItemKind::Stone));
    }

    #[test]
    fn refine_material_non_refinable_items_return_none() {
        assert_eq!(refine_material(ItemKind::Wood), None);
        assert_eq!(refine_material(ItemKind::HealingPotion), None);
        assert_eq!(refine_material(ItemKind::Ether), None);
    }

    #[test]
    fn enchant_from_shard_maps_all_planets() {
        assert_eq!(enchant_from_shard(ItemKind::JadeShard), Some(EnchantKind::Lifesteal));
        assert_eq!(enchant_from_shard(ItemKind::LavaCrystal), Some(EnchantKind::BurnBurst));
        assert_eq!(enchant_from_shard(ItemKind::VoidShard), Some(EnchantKind::CritStrike));
        assert_eq!(enchant_from_shard(ItemKind::AetherShard), Some(EnchantKind::AetherResonance));
        assert_eq!(enchant_from_shard(ItemKind::OriginShard), Some(EnchantKind::ExpBonus));
    }

    #[test]
    fn enchant_from_shard_ignores_non_shards() {
        assert_eq!(enchant_from_shard(ItemKind::Wood), None);
        assert_eq!(enchant_from_shard(ItemKind::CrystalShard), None);
    }

    #[test]
    fn enchant_extra_damage_burn_and_resonance() {
        assert_eq!(enchant_extra_damage(Some(EnchantKind::BurnBurst)), 3);
        assert_eq!(enchant_extra_damage(Some(EnchantKind::AetherResonance)), 2);
        assert_eq!(enchant_extra_damage(Some(EnchantKind::Lifesteal)), 0);
        assert_eq!(enchant_extra_damage(None), 0);
    }

    #[test]
    fn is_crit_tick_every_fifth() {
        assert!(!is_crit_tick(0));
        assert!(!is_crit_tick(1));
        assert!(is_crit_tick(4));
        assert!(!is_crit_tick(5));
        assert!(is_crit_tick(9));
    }

    #[test]
    fn enchant_exp_multiplier_exp_bonus_is_1_3() {
        assert!((enchant_exp_multiplier(Some(EnchantKind::ExpBonus)) - 1.3).abs() < 0.001);
        assert!((enchant_exp_multiplier(None) - 1.0).abs() < 0.001);
    }

    #[test]
    fn enchant_lifesteal_hp_returns_two_only_for_lifesteal() {
        assert_eq!(enchant_lifesteal_hp(Some(EnchantKind::Lifesteal)), 2);
        assert_eq!(enchant_lifesteal_hp(Some(EnchantKind::BurnBurst)), 0);
        assert_eq!(enchant_lifesteal_hp(None), 0);
    }

    #[test]
    fn equipment_meta_serde_round_trips() {
        let meta = EquipmentMeta { refine: 5, enchant: Some(EnchantKind::Lifesteal) };
        let json = serde_json::to_string(&meta).unwrap();
        let back: EquipmentMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(back, meta);
    }

    #[test]
    fn equipment_meta_default_is_zero_no_enchant() {
        let meta = EquipmentMeta::default();
        assert_eq!(meta.refine, 0);
        assert_eq!(meta.enchant, None);
    }

    #[test]
    fn old_json_without_meta_fields_deserializes_to_default() {
        let old_json = r#"{}"#;
        let meta: EquipmentMeta = serde_json::from_str(old_json).unwrap();
        assert_eq!(meta, EquipmentMeta::default());
    }
}
