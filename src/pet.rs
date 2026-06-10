//! 寵物系統（ROADMAP 46）。
//!
//! 玩家可以馴化瀕死（HP < 25%）的怪物成為跟班寵物，獲得對應的被動加成。
//! 一次只能帶一隻寵物；馴化消耗乙太，新馴化替換舊寵物（舊寵物自然放生）。
//!
//! 馴化方式：在 ATTACK_REACH 內，對 HP < 25% 的可馴化怪物發出 `TamePet` 指令。
//! 每種怪物有不同馴化費用與加成內容。

use crate::combat::EnemyKind;

/// 可馴化的寵物種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PetKind {
    /// 飄舞精靈（草原）：採集額外 +1 個物品。馴化費 10 乙太。
    FlutterSprite,
    /// 晶石傀儡（岩地）：防禦 +3。馴化費 20 乙太。
    CrystalGolem,
    /// 珊瑚蟹（水域）：擊殺經驗 +20%。馴化費 25 乙太。
    CoralCrab,
    /// 翠幽魅影（翠幽星）：攻擊力 +4。馴化費 35 乙太。
    JadeWraith,
    /// 源晶守護者（星源星）：攻擊力 +6、防禦 +3。馴化費 60 乙太（最強）。
    OriginGuardian,
}

impl PetKind {
    /// Wire key（前端 + 序列化用，snake_case）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FlutterSprite  => "flutter_sprite",
            Self::CrystalGolem   => "crystal_golem",
            Self::CoralCrab      => "coral_crab",
            Self::JadeWraith     => "jade_wraith",
            Self::OriginGuardian => "origin_guardian",
        }
    }

    /// 從 wire key 解析（前端送來的字串）。
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "flutter_sprite"  => Some(Self::FlutterSprite),
            "crystal_golem"   => Some(Self::CrystalGolem),
            "coral_crab"      => Some(Self::CoralCrab),
            "jade_wraith"     => Some(Self::JadeWraith),
            "origin_guardian" => Some(Self::OriginGuardian),
            _                  => None,
        }
    }

    /// 中文顯示名稱（前端寵物面板用）。
    pub fn display_name(self) -> &'static str {
        match self {
            Self::FlutterSprite  => "飄舞精靈",
            Self::CrystalGolem   => "晶石傀儡",
            Self::CoralCrab      => "珊瑚蟹",
            Self::JadeWraith     => "翠幽魅影",
            Self::OriginGuardian => "源晶守護者",
        }
    }

    /// 對應的生物 emoji（前端顯示在玩家旁邊）。
    pub fn emoji(self) -> &'static str {
        match self {
            Self::FlutterSprite  => "🧚",
            Self::CrystalGolem   => "💠",
            Self::CoralCrab      => "🦀",
            Self::JadeWraith     => "👻",
            Self::OriginGuardian => "🌟",
        }
    }

    /// 馴化消耗乙太。
    pub fn tame_cost(self) -> u32 {
        match self {
            Self::FlutterSprite  => 10,
            Self::CrystalGolem   => 20,
            Self::CoralCrab      => 25,
            Self::JadeWraith     => 35,
            Self::OriginGuardian => 60,
        }
    }

    /// 攻擊加成（直接加入有效攻擊力）。
    pub fn bonus_attack(self) -> u32 {
        match self {
            Self::JadeWraith     => 4,
            Self::OriginGuardian => 6,
            _                    => 0,
        }
    }

    /// 防禦加成（直接加入護甲減傷值）。
    pub fn bonus_defense(self) -> u32 {
        match self {
            Self::CrystalGolem   => 3,
            Self::OriginGuardian => 3,
            _                    => 0,
        }
    }

    /// 採集每次額外獲得的物品數量（0 = 無加成）。
    pub fn bonus_gather_qty(self) -> u32 {
        match self {
            Self::FlutterSprite => 1,
            _                   => 0,
        }
    }

    /// 擊殺經驗加成百分比整數（20 = +20%，0 = 無加成）。
    pub fn bonus_exp_pct(self) -> u32 {
        match self {
            Self::CoralCrab => 20,
            _               => 0,
        }
    }

    /// 加成一行描述文字（前端寵物面板用）。
    pub fn bonus_description(self) -> &'static str {
        match self {
            Self::FlutterSprite  => "每次採集額外 +1 物品",
            Self::CrystalGolem   => "防禦 +3",
            Self::CoralCrab      => "擊殺經驗 +20%",
            Self::JadeWraith     => "攻擊力 +4",
            Self::OriginGuardian => "攻擊力 +6、防禦 +3",
        }
    }
}

/// 從怪物種類查對應的可馴化寵物（不可馴化的回 None）。
/// 只有 5 種特別怪物可以馴化；普通怪物（ScrapDrone / EtherWisp 等）不行。
pub fn pet_from_enemy_kind(kind: EnemyKind) -> Option<PetKind> {
    match kind {
        EnemyKind::FlutterSprite  => Some(PetKind::FlutterSprite),
        EnemyKind::CrystalGolem   => Some(PetKind::CrystalGolem),
        EnemyKind::CoralCrab      => Some(PetKind::CoralCrab),
        EnemyKind::JadeWraith     => Some(PetKind::JadeWraith),
        EnemyKind::OriginGuardian => Some(PetKind::OriginGuardian),
        _                          => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_str_and_from_str_round_trips() {
        let kinds = [
            PetKind::FlutterSprite,
            PetKind::CrystalGolem,
            PetKind::CoralCrab,
            PetKind::JadeWraith,
            PetKind::OriginGuardian,
        ];
        for k in kinds {
            let s = k.as_str();
            let back = PetKind::from_str(s).expect("應能從 wire key 解析回來");
            assert_eq!(k, back, "wire key 往返：{s}");
        }
    }

    #[test]
    fn unknown_key_returns_none() {
        assert!(PetKind::from_str("dragon").is_none());
    }

    #[test]
    fn tame_cost_increases_with_rarity() {
        // 飄舞精靈最便宜，源晶守護者最貴。
        assert!(PetKind::FlutterSprite.tame_cost() < PetKind::CrystalGolem.tame_cost());
        assert!(PetKind::CrystalGolem.tame_cost() < PetKind::CoralCrab.tame_cost());
        assert!(PetKind::CoralCrab.tame_cost() < PetKind::JadeWraith.tame_cost());
        assert!(PetKind::JadeWraith.tame_cost() < PetKind::OriginGuardian.tame_cost());
    }

    #[test]
    fn pet_from_enemy_kind_maps_correctly() {
        use crate::combat::EnemyKind;
        assert_eq!(pet_from_enemy_kind(EnemyKind::FlutterSprite),  Some(PetKind::FlutterSprite));
        assert_eq!(pet_from_enemy_kind(EnemyKind::CrystalGolem),   Some(PetKind::CrystalGolem));
        assert_eq!(pet_from_enemy_kind(EnemyKind::CoralCrab),      Some(PetKind::CoralCrab));
        assert_eq!(pet_from_enemy_kind(EnemyKind::JadeWraith),     Some(PetKind::JadeWraith));
        assert_eq!(pet_from_enemy_kind(EnemyKind::OriginGuardian), Some(PetKind::OriginGuardian));
        // 不可馴化的怪物應回 None。
        assert!(pet_from_enemy_kind(EnemyKind::ScrapDrone).is_none());
        assert!(pet_from_enemy_kind(EnemyKind::EtherWisp).is_none());
        assert!(pet_from_enemy_kind(EnemyKind::RiftGuardian).is_none());
    }

    #[test]
    fn origin_guardian_has_highest_bonuses() {
        // 源晶守護者是最強的寵物，攻擊+防禦皆有加成。
        let og = PetKind::OriginGuardian;
        assert!(og.bonus_attack() > 0);
        assert!(og.bonus_defense() > 0);
    }

    #[test]
    fn flutter_sprite_gives_gather_bonus() {
        assert_eq!(PetKind::FlutterSprite.bonus_gather_qty(), 1);
        assert_eq!(PetKind::CrystalGolem.bonus_gather_qty(), 0);
    }

    #[test]
    fn coral_crab_gives_exp_bonus() {
        assert_eq!(PetKind::CoralCrab.bonus_exp_pct(), 20);
        assert_eq!(PetKind::JadeWraith.bonus_exp_pct(), 0);
    }

    #[test]
    fn all_pets_have_non_empty_descriptions() {
        for k in [
            PetKind::FlutterSprite, PetKind::CrystalGolem,
            PetKind::CoralCrab, PetKind::JadeWraith, PetKind::OriginGuardian,
        ] {
            assert!(!k.bonus_description().is_empty(), "{:?} 應有描述文字", k);
            assert!(!k.display_name().is_empty(), "{:?} 應有顯示名稱", k);
            assert!(!k.emoji().is_empty(), "{:?} 應有 emoji", k);
        }
    }
}
