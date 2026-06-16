//! 生態圖鑑（ROADMAP 333）：把世界裡的野生動物與守護者怪物，第一次變成玩家
//! 「蒐集得到、看得到進度」的東西。
//!
//! 動機：240～322 連近百個切片，都是讓世界在玩家周圍「活起來」的環境氛圍——
//! 野鳥理羽、群狼碰鼻、母獸舐犢、四大守護者各據生態……玩家多半只是旁觀。
//! reviewer 自 #496/#498/#499 一再點出「想看到不同成長維度、且玩家有明顯獲得感」。
//! 本模組正面回應：**把那一大票既有物種，第一次轉成玩家親手蒐集的圖鑑**——走近
//! 某種生物即「發現」牠、永久記進圖鑑，每種第一次發現給一筆小乙太獎勵（明顯獲得感），
//! 並能在面板看到「已發現 N / 全部」的蒐集進度。
//!
//! 設計：
//! - **純查表 / 純位元運算**，零 LLM、零額度、零平衡風險（獎勵刻意壓小）。
//! - 已發現集合壓成單一 `u64` bitmask（每位玩家一個整數），照既有 `exp` 欄位的模式
//!   持久化、跨重啟保留（圖鑑若不存活就失去蒐集意義）。
//! - **物種→bit 的對應穩定不可重排**（持久化相容契約：bit 一旦指定就固定，
//!   日後新增物種一律往高位接，絕不插隊／重排，否則舊存檔的位元會錯位）。
//! - 面向玩家字串（名稱）集中在本檔 `CATALOG`，為 i18n 集中替換點。

use crate::combat::EnemyKind;
use crate::wildlife::WildlifeKind;

/// 圖鑑一筆條目：給前端面板渲染與計數用。
#[derive(Debug, Clone, Copy)]
pub struct CodexEntry {
    /// 在 bitmask 裡的位元索引（穩定、不可重排）。
    pub bit: u8,
    /// 穩定 wire key（snake_case）：前端據此對應圖示與在地化字串。
    pub key: &'static str,
    /// 顯示名（繁中；i18n 集中替換點）。
    pub name: &'static str,
    /// 面板用 emoji。
    pub emoji: &'static str,
    /// 分類：`"wildlife"` 野生動物 ／ `"guardian"` 守護者怪物。
    pub category: &'static str,
}

/// 全部圖鑑條目。bit 連續且穩定（0..N）。**順序與 bit 絕不可重排**（持久化相容）。
/// 前 5 筆是中立野生動物（WildlifeKind），其後是各生態／各星球的守護者怪物（EnemyKind）。
pub const CATALOG: &[CodexEntry] = &[
    // ── 野生動物（bit 0..4，對齊 WildlifeKind）──
    CodexEntry { bit: 0, key: "wild_bird",       name: "野鳥",     emoji: "🐦", category: "wildlife" },
    CodexEntry { bit: 1, key: "wild_deer",       name: "野鹿",     emoji: "🦌", category: "wildlife" },
    CodexEntry { bit: 2, key: "small_critter",   name: "小動物",   emoji: "🐿️", category: "wildlife" },
    CodexEntry { bit: 3, key: "wild_wolf",       name: "野狼",     emoji: "🐺", category: "wildlife" },
    CodexEntry { bit: 4, key: "wild_fox",        name: "野狐",     emoji: "🦊", category: "wildlife" },
    // ── 守護者怪物（bit 5..18，對齊 EnemyKind）──
    CodexEntry { bit: 5,  key: "scrap_drone",     name: "廢鐵無人機", emoji: "🤖", category: "guardian" },
    CodexEntry { bit: 6,  key: "ether_wisp",      name: "乙太鬼火",   emoji: "🔥", category: "guardian" },
    CodexEntry { bit: 7,  key: "flutter_sprite",  name: "飄舞精靈",   emoji: "🧚", category: "guardian" },
    CodexEntry { bit: 8,  key: "mushroom_stalker",name: "蕈菇潛行者", emoji: "🍄", category: "guardian" },
    CodexEntry { bit: 9,  key: "crystal_golem",   name: "晶石傀儡",   emoji: "💎", category: "guardian" },
    CodexEntry { bit: 10, key: "rune_guardian",   name: "符文守衛",   emoji: "🗿", category: "guardian" },
    CodexEntry { bit: 11, key: "coral_crab",      name: "珊瑚蟹",     emoji: "🦀", category: "guardian" },
    CodexEntry { bit: 12, key: "jade_wraith",     name: "翠幽魅影",   emoji: "💚", category: "guardian" },
    CodexEntry { bit: 13, key: "steam_construct", name: "蒸汽構裝",   emoji: "⚙️", category: "guardian" },
    CodexEntry { bit: 14, key: "void_phantom",    name: "虛空幽靈",   emoji: "🌑", category: "guardian" },
    CodexEntry { bit: 15, key: "aether_specter",  name: "霧醚幻靈",   emoji: "🌫️", category: "guardian" },
    CodexEntry { bit: 16, key: "origin_guardian", name: "源晶守護者", emoji: "🟡", category: "guardian" },
    CodexEntry { bit: 17, key: "rift_guardian",   name: "裂縫守護者", emoji: "🌀", category: "guardian" },
    CodexEntry { bit: 18, key: "ether_overlord",  name: "乙太霸主",   emoji: "👹", category: "guardian" },
];

/// 圖鑑總條目數（前端顯示「N / TOTAL」，亦為合法位元上界）。
pub const TOTAL: u32 = CATALOG.len() as u32;

/// 發現一種野生動物給的乙太獎勵（刻意壓小，近乎零經濟擾動）。
pub const REWARD_WILDLIFE: u32 = 3;
/// 發現一種守護者怪物給的乙太獎勵（比野生動物略高——更難遇到、更值得紀念）。
pub const REWARD_GUARDIAN: u32 = 12;

/// 玩家走進多少像素內即「發現」一種生物。
/// 刻意 ＞ 野生動物驚逃半徑（`wildlife::FLEE_RADIUS` 180），讓你在牠竄逃前就先記下牠。
pub const DISCOVER_RADIUS: f32 = 220.0;

/// 野生動物種 → 圖鑑位元（穩定對應；與 CATALOG 前 5 筆一致）。
pub fn bit_for_wildlife(kind: WildlifeKind) -> u8 {
    match kind {
        WildlifeKind::WildBird => 0,
        WildlifeKind::WildDeer => 1,
        WildlifeKind::SmallCritter => 2,
        WildlifeKind::WildWolf => 3,
        WildlifeKind::WildFox => 4,
    }
}

/// 守護者怪物種 → 圖鑑位元（穩定對應；與 CATALOG 後段一致）。
pub fn bit_for_enemy(kind: EnemyKind) -> u8 {
    match kind {
        EnemyKind::ScrapDrone => 5,
        EnemyKind::EtherWisp => 6,
        EnemyKind::FlutterSprite => 7,
        EnemyKind::MushroomStalker => 8,
        EnemyKind::CrystalGolem => 9,
        EnemyKind::RuneGuardian => 10,
        EnemyKind::CoralCrab => 11,
        EnemyKind::JadeWraith => 12,
        EnemyKind::SteamConstruct => 13,
        EnemyKind::VoidPhantom => 14,
        EnemyKind::AetherSpecter => 15,
        EnemyKind::OriginGuardian => 16,
        EnemyKind::RiftGuardian => 17,
        EnemyKind::EtherOverlord => 18,
    }
}

/// 某位元是否已發現。
pub fn is_discovered(mask: u64, bit: u8) -> bool {
    mask & (1u64 << bit) != 0
}

/// 在圖鑑記下一個位元。回傳 `(新 mask, 是否本次才首次發現)`。
/// 已發現過則 mask 不變、回 `false`（天然冪等、不可重複領獎）。
pub fn discover(mask: u64, bit: u8) -> (u64, bool) {
    if is_discovered(mask, bit) {
        (mask, false)
    } else {
        (mask | (1u64 << bit), true)
    }
}

/// 已發現的物種數（只計圖鑑合法位元，忽略任何高位雜訊）。
pub fn count(mask: u64) -> u32 {
    (mask & valid_mask()).count_ones()
}

/// 合法位元遮罩（0..TOTAL）。
fn valid_mask() -> u64 {
    if TOTAL >= 64 {
        u64::MAX
    } else {
        (1u64 << TOTAL) - 1
    }
}

/// 該位元首次發現時給的乙太獎勵（依分類；未知位元回 0，安全）。
pub fn reward_for_bit(bit: u8) -> u32 {
    CATALOG
        .iter()
        .find(|e| e.bit == bit)
        .map(|e| if e.category == "guardian" { REWARD_GUARDIAN } else { REWARD_WILDLIFE })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 所有野生動物種。
    const ALL_WILDLIFE: &[WildlifeKind] = &[
        WildlifeKind::WildBird,
        WildlifeKind::WildDeer,
        WildlifeKind::SmallCritter,
        WildlifeKind::WildWolf,
        WildlifeKind::WildFox,
    ];

    /// 所有守護者怪物種。
    const ALL_ENEMIES: &[EnemyKind] = &[
        EnemyKind::ScrapDrone,
        EnemyKind::EtherWisp,
        EnemyKind::FlutterSprite,
        EnemyKind::MushroomStalker,
        EnemyKind::CrystalGolem,
        EnemyKind::RuneGuardian,
        EnemyKind::CoralCrab,
        EnemyKind::JadeWraith,
        EnemyKind::SteamConstruct,
        EnemyKind::VoidPhantom,
        EnemyKind::AetherSpecter,
        EnemyKind::OriginGuardian,
        EnemyKind::RiftGuardian,
        EnemyKind::EtherOverlord,
    ];

    #[test]
    fn catalog_bits_are_unique_and_contiguous() {
        for (i, e) in CATALOG.iter().enumerate() {
            assert_eq!(e.bit as usize, i, "CATALOG 第 {i} 筆 bit 應等於索引（連續穩定）");
        }
        assert!(TOTAL <= 64, "圖鑑條目不可超過 64（u64 bitmask 上限）");
    }

    #[test]
    fn catalog_keys_unique() {
        for (i, a) in CATALOG.iter().enumerate() {
            for b in &CATALOG[i + 1..] {
                assert_ne!(a.key, b.key, "圖鑑 key 重複：{}", a.key);
            }
        }
    }

    #[test]
    fn every_wildlife_kind_maps_to_a_catalog_entry() {
        for &k in ALL_WILDLIFE {
            let bit = bit_for_wildlife(k);
            let entry = CATALOG.iter().find(|e| e.bit == bit).expect("野生動物 bit 必有對應條目");
            assert_eq!(entry.category, "wildlife");
        }
    }

    #[test]
    fn every_enemy_kind_maps_to_a_catalog_entry() {
        for &k in ALL_ENEMIES {
            let bit = bit_for_enemy(k);
            let entry = CATALOG.iter().find(|e| e.bit == bit).expect("怪物 bit 必有對應條目");
            assert_eq!(entry.category, "guardian");
        }
    }

    #[test]
    fn wildlife_and_enemy_bits_never_collide() {
        for &w in ALL_WILDLIFE {
            for &e in ALL_ENEMIES {
                assert_ne!(bit_for_wildlife(w), bit_for_enemy(e), "野生動物與怪物位元不可相撞");
            }
        }
    }

    #[test]
    fn discover_sets_bit_and_reports_first_time() {
        let (m1, new1) = discover(0, 3);
        assert!(new1, "首次發現應回 true");
        assert!(is_discovered(m1, 3));
        let (m2, new2) = discover(m1, 3);
        assert!(!new2, "再次發現同種應回 false（冪等、不可重複領獎）");
        assert_eq!(m1, m2, "重複發現不改 mask");
    }

    #[test]
    fn count_matches_discovered_bits() {
        assert_eq!(count(0), 0);
        let (m, _) = discover(0, 0);
        let (m, _) = discover(m, 5);
        let (m, _) = discover(m, 18);
        assert_eq!(count(m), 3);
    }

    #[test]
    fn count_ignores_out_of_range_high_bits() {
        // 即便存檔被塞了圖鑑範圍外的高位（壞值/未來欄位），計數也只算合法位元。
        let junk = 1u64 << 60;
        assert_eq!(count(junk), 0);
        let (m, _) = discover(junk, 2);
        assert_eq!(count(m), 1);
    }

    #[test]
    fn reward_positive_for_every_catalog_bit() {
        for e in CATALOG {
            assert!(reward_for_bit(e.bit) > 0, "每種生物首次發現都該有正獎勵：{}", e.key);
        }
        assert_eq!(reward_for_bit(99), 0, "未知位元回 0（安全）");
    }

    #[test]
    fn guardian_reward_exceeds_wildlife() {
        assert!(REWARD_GUARDIAN > REWARD_WILDLIFE, "守護者更難遇、獎勵應更高");
    }
}
