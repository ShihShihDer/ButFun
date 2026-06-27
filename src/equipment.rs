//! 裝備槽（ROADMAP 36）：三槽顯式裝備——🗡️ 武器 / 🛡️ 防具 / 📿 飾品。
//! 純邏輯、無 IO，取代「背包有就自動取最強」的隱式戰鬥加成。
//! 飾品槽 MVP 保留空結構，尚無飾品種類。
//! ROADMAP 37 擴充：精煉等級與附魔效果嵌入同一結構（向後相容，舊資料 default 為 0/None）。

use serde::{Deserialize, Serialize};

use crate::inventory::{Inventory, ItemKind};
use crate::refinement::{refine_bonus_atk, refine_bonus_def, EquipmentMeta};

/// 三槽裝備狀態。序列化儲存於 inventories 表的 equipment 欄（TEXT JSON）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EquipmentSlots {
    /// 🗡️ 武器槽：`weapon_from_item` 可識別的物品。
    #[serde(default)]
    pub weapon: Option<ItemKind>,
    /// 🛡️ 防具槽：`armor_from_item` 可識別的物品。
    #[serde(default)]
    pub armor: Option<ItemKind>,
    /// 📿 飾品槽：MVP 保留結構，尚無飾品種類。
    #[serde(default)]
    pub accessory: Option<ItemKind>,
    /// 武器槽精煉／附魔元資料（ROADMAP 37；舊資料 default 為 0/None）。
    #[serde(default)]
    pub weapon_meta: EquipmentMeta,
    /// 防具槽精煉元資料（ROADMAP 37；舊資料 default 為 0/None）。
    #[serde(default)]
    pub armor_meta: EquipmentMeta,
}

/// 一個物品對應的裝備槽名稱（"weapon" / "armor" / "accessory"）；不可裝備的物品回 `None`。
pub fn slot_for_item(item: ItemKind) -> Option<&'static str> {
    if crate::combat::weapon_from_item(item).is_some() {
        Some("weapon")
    } else if crate::combat::armor_from_item(item).is_some() {
        Some("armor")
    } else if matches!(item, ItemKind::StarAmulet | ItemKind::StarGuardianAmulet) {
        // 護符類飾品——提供被動加成，無戰鬥數值（ROADMAP 133/134）。
        Some("accessory")
    } else {
        None
    }
}

/// 裝備一個物品到對應槽（呼叫端已確認背包有貨）。
/// 回 `Some(old_item)` = 同槽換裝，舊裝備退回背包；首次裝備回 `None`。
/// 物品不可裝備（`slot_for_item` 回 `None`）時不改 slots 亦回 `None`。
pub fn equip(slots: &mut EquipmentSlots, item: ItemKind) -> Option<ItemKind> {
    match slot_for_item(item)? {
        // ⚠️ ROADMAP 37 安全修正：精煉／附魔 meta 綁在「槽」而非具體物品。換上**不同**物品時
        // 必須清空舊 meta，否則新裝白嫖前一把的精煉加成＋附魔——一個可由正常訊息觸發、且會
        // 透過 remember_equipment 持久化的免費戰力 exploit（卸裝同理見 unequip）。
        // 換上同一物品（冪等）不清，避免無謂損失。
        "weapon" => {
            let old = slots.weapon.replace(item);
            if old != Some(item) {
                slots.weapon_meta = EquipmentMeta::default();
            }
            old
        }
        "armor" => {
            let old = slots.armor.replace(item);
            if old != Some(item) {
                slots.armor_meta = EquipmentMeta::default();
            }
            old
        }
        "accessory" => slots.accessory.replace(item),
        _ => None,
    }
}

/// 卸下指定槽的裝備並回傳（退回背包用）；`None` = 空槽或槽名無效。
pub fn unequip(slots: &mut EquipmentSlots, slot: &str) -> Option<ItemKind> {
    match slot {
        // 卸裝清空對應 meta：避免精煉/附魔殘留在空槽，徒手仍享加成（同 equip 的 exploit 防線）。
        "weapon" => {
            let removed = slots.weapon.take();
            if removed.is_some() {
                slots.weapon_meta = EquipmentMeta::default();
            }
            removed
        }
        "armor" => {
            let removed = slots.armor.take();
            if removed.is_some() {
                slots.armor_meta = EquipmentMeta::default();
            }
            removed
        }
        "accessory" => slots.accessory.take(),
        _ => None,
    }
}

/// 已裝備武器的攻擊力（無裝備 = 徒手 `UNARMED_ATTACK_POWER`；含精煉加成）。
pub fn equipped_weapon_power(slots: &EquipmentSlots) -> u32 {
    // 防線：精煉加成只在「槽內真有武器」時計入；空槽＝徒手，任何殘留 meta 一律不套用。
    match slots.weapon.and_then(crate::combat::weapon_from_item) {
        Some(w) => w.attack_power() + refine_bonus_atk(slots.weapon_meta.refine),
        None => crate::combat::UNARMED_ATTACK_POWER,
    }
}

/// 已裝備護甲的減傷值（無護甲 = 0；含精煉加成）。
pub fn equipped_armor_defense(slots: &EquipmentSlots) -> u32 {
    // 防線：同武器，空槽不套用任何殘留精煉。
    match slots.armor.and_then(crate::combat::armor_from_item) {
        Some(a) => a.defense() + refine_bonus_def(slots.armor_meta.refine),
        None => 0,
    }
}

/// 既有玩家遷移：從背包自動裝上最強武器 + 最強護甲，體感零變化。
pub fn auto_equip_best(inv: &Inventory) -> EquipmentSlots {
    let weapon = inv
        .entries()
        .filter_map(|(item, _)| {
            crate::combat::weapon_from_item(item).map(|wk| (item, wk.attack_power()))
        })
        .max_by_key(|&(_, pwr)| pwr)
        .map(|(item, _)| item);

    let armor = inv
        .entries()
        .filter_map(|(item, _)| {
            crate::combat::armor_from_item(item).map(|ak| (item, ak.defense()))
        })
        .max_by_key(|&(_, def)| def)
        .map(|(item, _)| item);

    EquipmentSlots { weapon, armor, accessory: None, weapon_meta: EquipmentMeta::default(), armor_meta: EquipmentMeta::default() }
}

/// ItemKind → 前端 wire key（serde snake_case，去除引號）。供 `PlayerView` 填 equipped_* 欄位。
pub fn item_to_wire_key(item: ItemKind) -> String {
    serde_json::to_string(&item)
        .unwrap_or_default()
        .trim_matches('"')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combat::UNARMED_ATTACK_POWER;
    use crate::inventory::{Inventory, ItemKind};

    #[test]
    fn empty_slots_give_unarmed_power_and_zero_defense() {
        let slots = EquipmentSlots::default();
        assert_eq!(equipped_weapon_power(&slots), UNARMED_ATTACK_POWER);
        assert_eq!(equipped_armor_defense(&slots), 0);
    }

    #[test]
    fn equip_weapon_raises_attack() {
        let mut slots = EquipmentSlots::default();
        let old = equip(&mut slots, ItemKind::CrystalBlade);
        assert_eq!(old, None);
        assert_eq!(slots.weapon, Some(ItemKind::CrystalBlade));
        assert_eq!(equipped_weapon_power(&slots), 8);
    }

    #[test]
    fn equip_weapon_swap_returns_old_item() {
        let mut slots = EquipmentSlots::default();
        equip(&mut slots, ItemKind::Weapon);
        let old = equip(&mut slots, ItemKind::CrystalBlade);
        assert_eq!(old, Some(ItemKind::Weapon));
        assert_eq!(slots.weapon, Some(ItemKind::CrystalBlade));
    }

    #[test]
    fn equip_armor_raises_defense() {
        let mut slots = EquipmentSlots::default();
        equip(&mut slots, ItemKind::CrystalShield);
        assert_eq!(equipped_armor_defense(&slots), 2);
    }

    #[test]
    fn unequip_returns_item_and_clears_slot() {
        let mut slots = EquipmentSlots::default();
        equip(&mut slots, ItemKind::Weapon);
        let removed = unequip(&mut slots, "weapon");
        assert_eq!(removed, Some(ItemKind::Weapon));
        assert_eq!(slots.weapon, None);
        assert_eq!(equipped_weapon_power(&slots), UNARMED_ATTACK_POWER);
    }

    #[test]
    fn unequip_empty_slot_returns_none() {
        let mut slots = EquipmentSlots::default();
        assert_eq!(unequip(&mut slots, "weapon"), None);
        assert_eq!(unequip(&mut slots, "armor"), None);
    }

    #[test]
    fn swap_to_different_weapon_clears_refine_meta() {
        // exploit 防護：把便宜武器精煉滿，換上更強武器後不得白嫖舊精煉。
        let mut slots = EquipmentSlots::default();
        equip(&mut slots, ItemKind::Weapon);
        slots.weapon_meta.refine = crate::refinement::MAX_REFINE; // +9
        slots.weapon_meta.enchant = Some(crate::refinement::EnchantKind::CritStrike);
        equip(&mut slots, ItemKind::CrystalBlade); // 換不同武器
        assert_eq!(slots.weapon_meta.refine, 0, "換裝應清空舊精煉");
        assert_eq!(slots.weapon_meta.enchant, None, "換裝應清空舊附魔");
        assert_eq!(equipped_weapon_power(&slots), 8, "CrystalBlade 基礎 8，不含舊 +9");
    }

    #[test]
    fn unequip_clears_refine_meta_and_unarmed_has_no_bonus() {
        // exploit 防護：精煉後卸裝，徒手不得保留精煉加成。
        let mut slots = EquipmentSlots::default();
        equip(&mut slots, ItemKind::Weapon);
        slots.weapon_meta.refine = crate::refinement::MAX_REFINE;
        unequip(&mut slots, "weapon");
        assert_eq!(slots.weapon_meta.refine, 0);
        assert_eq!(equipped_weapon_power(&slots), UNARMED_ATTACK_POWER, "徒手 = 基礎徒手值，無殘留精煉");
    }

    #[test]
    fn dangling_meta_on_empty_slot_is_ignored() {
        // 防線：即使 meta 因任何路徑殘留在空槽，power 也不套用。
        let mut slots = EquipmentSlots::default();
        slots.weapon_meta.refine = crate::refinement::MAX_REFINE;
        slots.armor_meta.refine = crate::refinement::MAX_REFINE;
        assert_eq!(equipped_weapon_power(&slots), UNARMED_ATTACK_POWER);
        assert_eq!(equipped_armor_defense(&slots), 0);
    }

    #[test]
    fn re_equip_same_weapon_keeps_refine_meta() {
        // 冪等：重複裝同一把武器不清精煉（避免無謂損失，也不算 exploit）。
        let mut slots = EquipmentSlots::default();
        equip(&mut slots, ItemKind::Weapon);
        slots.weapon_meta.refine = 5;
        equip(&mut slots, ItemKind::Weapon); // 同一把
        assert_eq!(slots.weapon_meta.refine, 5);
        assert_eq!(equipped_weapon_power(&slots), 5 + 5, "Weapon 基礎 5 + 精煉 5");
    }

    #[test]
    fn non_equippable_item_has_no_slot() {
        assert_eq!(slot_for_item(ItemKind::Wood), None);
        assert_eq!(slot_for_item(ItemKind::HealingPotion), None);
        assert_eq!(slot_for_item(ItemKind::Ether), None);
    }

    #[test]
    fn auto_equip_picks_strongest_weapon_and_armor() {
        let mut inv = Inventory::new();
        inv.add(ItemKind::Weapon, 1);
        inv.add(ItemKind::CrystalBlade, 1); // +8，強於 Weapon（+5）
        inv.add(ItemKind::MeadowAmulet, 1);
        inv.add(ItemKind::CrystalShield, 1); // def 2，強於 MeadowAmulet（def 1）
        let slots = auto_equip_best(&inv);
        assert_eq!(slots.weapon, Some(ItemKind::CrystalBlade));
        assert_eq!(slots.armor, Some(ItemKind::CrystalShield));
    }

    #[test]
    fn auto_equip_empty_inventory_gives_empty_slots() {
        let inv = Inventory::new();
        assert_eq!(auto_equip_best(&inv), EquipmentSlots::default());
    }

    #[test]
    fn slots_serde_round_trips() {
        let mut slots = EquipmentSlots::default();
        equip(&mut slots, ItemKind::CoralLance);
        equip(&mut slots, ItemKind::CosmicShield);
        let json = serde_json::to_string(&slots).unwrap();
        let back: EquipmentSlots = serde_json::from_str(&json).unwrap();
        assert_eq!(back, slots);
    }

    #[test]
    fn item_to_wire_key_produces_snake_case() {
        assert_eq!(item_to_wire_key(ItemKind::CrystalBlade), "crystal_blade");
        assert_eq!(item_to_wire_key(ItemKind::MeadowAmulet), "meadow_amulet");
        assert_eq!(item_to_wire_key(ItemKind::Weapon), "weapon");
    }
}
