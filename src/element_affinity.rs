//! 元素剋制系統（ROADMAP 380）——武器附魔對應敵人弱點，戰鬥第一次有策略選擇。
//!
//! **設計**：
//! - 6 種元素：火焰、乙太、自然、晶石、虛空、機械。
//! - 各附魔僅兩種具有元素屬性（BurnBurst=火焰、AetherResonance=乙太）；
//!   其餘（暴擊/吸血/增幅）為純物理效果、無元素屬性。
//! - 弱點倍率 1.5×；無「抵抗」（只有弱點或一般），保持正面遊戲感。
//! - 晶石系無弱點（設計意圖：晶石類高 HP 怪需以純輸出克服，不靠元素捷徑）。
//!
//! **元素剋制表**：
//!   火焰  → 自然（火燒草木）
//!   火焰  → 機械（熔爐電路）
//!   乙太  → 虛空（乙太之光驅散虛空黑暗）
//!   乙太  → 火焰（乙太霧氣澆熄蒸汽爐焰）
//!
//! 純邏輯層，無 IO，便於測試。

use crate::combat::EnemyKind;
use crate::refinement::EnchantKind;

// ───────────────────────── 元素種類 ─────────────────────────

/// 戰鬥中使用的元素類型。每種敵人各屬一種元素；只有部分附魔具有元素屬性。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Element {
    /// 火焰：赤焰碎片召喚的灼燒之力。
    Fire,
    /// 乙太：霧醚碎片凝聚的共鳴能量。
    Ether,
    /// 自然：草木生靈、珊瑚海洋的原初之力。
    Nature,
    /// 晶石：晶體傀儡與符文守衛的礦物能量。
    Crystal,
    /// 虛空：虛空幽靈與裂縫守護者的次元黑暗。
    Void,
    /// 機械：蒸汽龐克廢鐵構裝的齒輪驅動。
    Mechanical,
}

impl Element {
    /// 前端 wire 字串（穩定協議，不可輕易重排）。
    pub fn wire_str(self) -> &'static str {
        match self {
            Element::Fire       => "fire",
            Element::Ether      => "ether",
            Element::Nature     => "nature",
            Element::Crystal    => "crystal",
            Element::Void       => "void",
            Element::Mechanical => "mechanical",
        }
    }
}

// ───────────────────────── 敵人元素 ─────────────────────────

/// 回傳敵人種類的元素屬性。純函式、確定性。
pub fn enemy_element(kind: EnemyKind) -> Element {
    match kind {
        EnemyKind::ScrapDrone      => Element::Mechanical, // 廢鐵無人機 = 機械
        EnemyKind::EtherWisp       => Element::Ether,      // 乙太鬼火 = 乙太
        EnemyKind::FlutterSprite   => Element::Nature,     // 飄舞精靈 = 自然
        EnemyKind::MushroomStalker => Element::Nature,     // 蕈菇潛行者 = 自然
        EnemyKind::CrystalGolem    => Element::Crystal,    // 晶石傀儡 = 晶石
        EnemyKind::RuneGuardian    => Element::Crystal,    // 符文守衛 = 晶石
        EnemyKind::CoralCrab       => Element::Nature,     // 珊瑚蟹 = 自然（海洋生靈）
        EnemyKind::JadeWraith      => Element::Void,       // 翠幽魅影 = 虛空（異星幽魂）
        EnemyKind::SteamConstruct  => Element::Fire,       // 蒸汽構裝 = 火焰（蒸汽鍋爐）
        EnemyKind::VoidPhantom     => Element::Void,       // 虛空幽靈 = 虛空
        EnemyKind::AetherSpecter   => Element::Ether,      // 霧醚幻靈 = 乙太
        EnemyKind::OriginGuardian  => Element::Crystal,    // 源晶守護者 = 晶石
        EnemyKind::RiftGuardian    => Element::Void,       // 裂縫守護者 = 虛空
        EnemyKind::EtherOverlord   => Element::Ether,      // 乙太霸主 = 乙太
    }
}

// ───────────────────────── 附魔元素 ─────────────────────────

/// 回傳附魔的元素屬性；沒有元素性的附魔回 `None`。純函式。
pub fn enchant_to_element(enchant: EnchantKind) -> Option<Element> {
    match enchant {
        EnchantKind::BurnBurst      => Some(Element::Fire),  // 赤焰碎片 = 火焰
        EnchantKind::AetherResonance => Some(Element::Ether), // 霧醚碎片 = 乙太
        // 以下三種為物理/經驗效果，無元素屬性
        EnchantKind::CritStrike     => None,
        EnchantKind::Lifesteal      => None,
        EnchantKind::ExpBonus       => None,
    }
}

// ───────────────────────── 弱點判定 ─────────────────────────

/// 判定攻擊者元素是否剋制目標元素。純函式。
///
/// 剋制關係（單向，無「抵抗」，只有弱點或一般）：
/// - 火焰 → 自然（火燒草木）
/// - 火焰 → 機械（熔爐電路）
/// - 乙太 → 虛空（乙太之光驅散虛空黑暗）
/// - 乙太 → 火焰（乙太霧氣澆熄蒸汽爐焰）
pub fn is_weakness(attacker: Element, target: Element) -> bool {
    matches!((attacker, target),
        (Element::Fire,  Element::Nature)     |  // 火燒草木
        (Element::Fire,  Element::Mechanical) |  // 熔爐電路
        (Element::Ether, Element::Void)       |  // 乙太之光驅散虛空
        (Element::Ether, Element::Fire)          // 乙太霧氣澆熄蒸汽
    )
}

/// 弱點傷害倍率。純函式。
pub const WEAKNESS_MULTIPLIER: f32 = 1.5;

/// 計算元素傷害倍率（1.0 = 無加成，`WEAKNESS_MULTIPLIER` = 弱點）。
/// `enchant` 為攻擊方武器的附魔；`target` 為被攻擊的敵人種類。純函式。
pub fn damage_multiplier(enchant: Option<EnchantKind>, target: EnemyKind) -> f32 {
    let Some(enc) = enchant else { return 1.0; };
    let Some(atk_elem) = enchant_to_element(enc) else { return 1.0; };
    let def_elem = enemy_element(target);
    if is_weakness(atk_elem, def_elem) { WEAKNESS_MULTIPLIER } else { 1.0 }
}

// ───────────────────────── 測試 ─────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_enemy_kinds_have_elements() {
        use crate::combat::EnemyKind::*;
        let kinds = [
            ScrapDrone, EtherWisp, FlutterSprite, MushroomStalker, CrystalGolem,
            RuneGuardian, CoralCrab, JadeWraith, SteamConstruct, VoidPhantom,
            AetherSpecter, OriginGuardian, RiftGuardian, EtherOverlord,
        ];
        for k in kinds {
            let _ = enemy_element(k); // 應窮舉不 panic
        }
    }

    #[test]
    fn fire_beats_nature() {
        assert_eq!(damage_multiplier(Some(EnchantKind::BurnBurst), EnemyKind::FlutterSprite), WEAKNESS_MULTIPLIER);
        assert_eq!(damage_multiplier(Some(EnchantKind::BurnBurst), EnemyKind::MushroomStalker), WEAKNESS_MULTIPLIER);
        assert_eq!(damage_multiplier(Some(EnchantKind::BurnBurst), EnemyKind::CoralCrab), WEAKNESS_MULTIPLIER);
    }

    #[test]
    fn fire_beats_mechanical() {
        assert_eq!(damage_multiplier(Some(EnchantKind::BurnBurst), EnemyKind::ScrapDrone), WEAKNESS_MULTIPLIER);
    }

    #[test]
    fn ether_beats_void() {
        assert_eq!(damage_multiplier(Some(EnchantKind::AetherResonance), EnemyKind::JadeWraith), WEAKNESS_MULTIPLIER);
        assert_eq!(damage_multiplier(Some(EnchantKind::AetherResonance), EnemyKind::VoidPhantom), WEAKNESS_MULTIPLIER);
        assert_eq!(damage_multiplier(Some(EnchantKind::AetherResonance), EnemyKind::RiftGuardian), WEAKNESS_MULTIPLIER);
    }

    #[test]
    fn ether_beats_fire() {
        // 乙太霧氣澆熄蒸汽構裝的爐焰
        assert_eq!(damage_multiplier(Some(EnchantKind::AetherResonance), EnemyKind::SteamConstruct), WEAKNESS_MULTIPLIER);
    }

    #[test]
    fn no_weakness_for_crystal() {
        // 晶石系（傀儡/守衛/源晶）無元素弱點，需純輸出攻克
        assert_eq!(damage_multiplier(Some(EnchantKind::BurnBurst), EnemyKind::CrystalGolem), 1.0);
        assert_eq!(damage_multiplier(Some(EnchantKind::BurnBurst), EnemyKind::RuneGuardian), 1.0);
        assert_eq!(damage_multiplier(Some(EnchantKind::AetherResonance), EnemyKind::OriginGuardian), 1.0);
    }

    #[test]
    fn non_elemental_enchants_no_bonus() {
        // CritStrike / Lifesteal / ExpBonus 無元素屬性，不觸發弱點加成
        assert_eq!(damage_multiplier(Some(EnchantKind::CritStrike), EnemyKind::FlutterSprite), 1.0);
        assert_eq!(damage_multiplier(Some(EnchantKind::Lifesteal), EnemyKind::ScrapDrone), 1.0);
        assert_eq!(damage_multiplier(Some(EnchantKind::ExpBonus), EnemyKind::VoidPhantom), 1.0);
    }

    #[test]
    fn no_enchant_no_bonus() {
        assert_eq!(damage_multiplier(None, EnemyKind::EtherWisp), 1.0);
    }

    #[test]
    fn fire_no_bonus_vs_ether_or_void() {
        // 火焰不剋乙太、不剋虛空
        assert_eq!(damage_multiplier(Some(EnchantKind::BurnBurst), EnemyKind::EtherWisp), 1.0);
        assert_eq!(damage_multiplier(Some(EnchantKind::BurnBurst), EnemyKind::VoidPhantom), 1.0);
    }

    #[test]
    fn ether_no_bonus_vs_nature_or_crystal() {
        // 乙太不剋自然、不剋晶石
        assert_eq!(damage_multiplier(Some(EnchantKind::AetherResonance), EnemyKind::FlutterSprite), 1.0);
        assert_eq!(damage_multiplier(Some(EnchantKind::AetherResonance), EnemyKind::CrystalGolem), 1.0);
    }

    #[test]
    fn wire_str_stable_no_empty() {
        use crate::combat::EnemyKind::*;
        let kinds = [ScrapDrone, EtherWisp, FlutterSprite, CrystalGolem, JadeWraith, SteamConstruct];
        for k in kinds {
            assert!(!enemy_element(k).wire_str().is_empty());
        }
    }

    #[test]
    fn multiplier_is_positive() {
        // 所有組合傷害倍率皆為正
        use crate::combat::EnemyKind::*;
        let kinds = [ScrapDrone, EtherWisp, FlutterSprite, MushroomStalker, CrystalGolem, CoralCrab];
        let enchants = [Some(EnchantKind::BurnBurst), Some(EnchantKind::AetherResonance), None];
        for k in kinds {
            for e in enchants {
                assert!(damage_multiplier(e, k) > 0.0);
            }
        }
    }
}
