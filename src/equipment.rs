//! 裝備槽（ROADMAP 36）：三槽顯式裝備——🗡️ 武器 / 🛡️ 防具 / 📿 飾品。
//! 純邏輯、無 IO，取代「背包有就自動取最強」的隱式戰鬥加成。
//! 飾品槽 MVP 保留空結構，尚無飾品種類。

use serde::{Deserialize, Serialize};

use crate::inventory::{Inventory, ItemKind};

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
}

/// 一個物品對應的裝備槽名稱（"weapon" / "armor"）；不可裝備的物品回 `None`。
pub fn slot_for_item(item: ItemKind) -> Option<&'static str> {
    if crate::combat::weapon_from_item(item).is_some() {
        Some("weapon")
    } else if crate::combat::armor_from_item(item).is_some() {
        Some("armor")
    } else {
        None
    }
}

/// 裝備一個物品到對應槽（呼叫端已確認背包有貨）。
/// 回 `Some(old_item)` = 同槽換裝，舊裝備退回背包；首次裝備回 `None`。
/// 物品不可裝備（`slot_for_item` 回 `None`）時不改 slots 亦回 `None`。
pub fn equip(slots: &mut EquipmentSlots, item: ItemKind) -> Option<ItemKind> {
    match slot_for_item(item)? {
        "weapon" => slots.weapon.replace(item),
        "armor" => slots.armor.replace(item),
        _ => None,
    }
}

/// 卸下指定槽的裝備並回傳（退回背包用）；`None` = 空槽或槽名無效。
pub fn unequip(slots: &mut EquipmentSlots, slot: &str) -> Option<ItemKind> {
    match slot {
        "weapon" => slots.weapon.take(),
        "armor" => slots.armor.take(),
        "accessory" => slots.accessory.take(),
        _ => None,
    }
}

/// 已裝備武器的攻擊力（無裝備 = 徒手 `UNARMED_ATTACK_POWER`）。
pub fn equipped_weapon_power(slots: &EquipmentSlots) -> u32 {
    slots
        .weapon
        .and_then(crate::combat::weapon_from_item)
        .map(|w| w.attack_power())
        .unwrap_or(crate::combat::UNARMED_ATTACK_POWER)
}

/// 已裝備護甲的減傷值（無護甲 = 0）。
pub fn equipped_armor_defense(slots: &EquipmentSlots) -> u32 {
    slots
        .armor
        .and_then(crate::combat::armor_from_item)
        .map(|a| a.defense())
        .unwrap_or(0)
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

    EquipmentSlots { weapon, armor, accessory: None }
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
