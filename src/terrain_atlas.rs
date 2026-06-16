//! 探索圖鑑（ROADMAP 336）：把世界裡各種「奇景地形」，第一次變成玩家踏足蒐集、
//! 看得到進度的東西——開一條全新的「探索」成長維度。
//!
//! 動機：333～335 開了「生態圖鑑」這條蒐集維度（走近生物即發現、集滿給獎、配戴稱號），
//! reviewer 自 #496/#498/#499 一再要「不同成長維度、玩家明顯獲得感」。生態圖鑑蒐集的是
//! 「世界裡活的生物」，可這片廣袤無垠的開放世界本身——熔岩荒原、虛空晶簇、霧醚秘境、
//! 星源寶地——卻一直只是供人路過的背景，沒有任何「我走遍了世界」的成就被記下。
//! 本模組正面回應：**把那一大票既有的奇景地形，第一次轉成玩家親腳踏足、看得到進度的探索圖鑑**——
//! 走近某種奇景地形即「探索」它、永久記進圖鑑，每種第一次踏足給一筆乙太獎勵（愈稀有愈豐厚），
//! 給開放世界第一個「值得遠行去看看」的理由，與戰鬥／種田／社交／生態蒐集並列。
//!
//! 設計（刻意與 `field_guide` 同一套招數，最低風險、最易維護）：
//! - **純查表 / 純位元運算**，零 LLM、零額度、零平衡風險（獎勵刻意壓小）。
//! - 已探索集合壓成單一 `u64` bitmask（每位玩家一個整數），照 `codex`（333）的模式
//!   持久化、跨重啟保留（探索進度若不存活就失去蒐集意義）。
//! - **地形→bit 的對應穩定不可重排**（持久化相容契約：bit 一旦指定就固定，
//!   日後新增地形一律往高位接，絕不插隊／重排，否則舊存檔的位元會錯位）。
//! - 只收錄「奇景」天然地形——基本土／石／空地與人造城牆無處不在、無「探索」可言，故不收錄。
//! - 面向玩家字串（名稱）集中在本檔 `CATALOG`，為 i18n 集中替換點。

use world_core::TileKind;

/// 探索圖鑑一筆條目：給前端面板渲染與計數用。
#[derive(Debug, Clone, Copy)]
pub struct AtlasEntry {
    /// 在 bitmask 裡的位元索引（穩定、不可重排）。
    pub bit: u8,
    /// 穩定 wire key（snake_case）：前端據此對應圖示與在地化字串。
    pub key: &'static str,
    /// 顯示名（繁中；i18n 集中替換點）。
    pub name: &'static str,
    /// 面板用 emoji。
    pub emoji: &'static str,
    /// 稀有度：`"common"` 常見 ／ `"rare"` 稀有 ／ `"legendary"` 傳奇（決定獎勵高低與面板配色）。
    pub tier: &'static str,
}

/// 全部探索圖鑑條目。bit 連續且穩定（0..N）。**順序與 bit 絕不可重排**（持久化相容）。
/// 大致由「離家近／常見」往「遠方／稀有」排，扣合各星球生態的地形特產。
pub const CATALOG: &[AtlasEntry] = &[
    // ── 常見奇景（故鄉與近郊隨處可見）──
    AtlasEntry { bit: 0, key: "ore",            name: "乙太礦脈", emoji: "⛏️", tier: "common" },
    AtlasEntry { bit: 1, key: "crystal",        name: "晶石地",   emoji: "💎", tier: "common" },
    AtlasEntry { bit: 2, key: "mushroom",       name: "蕈菇林",   emoji: "🍄", tier: "common" },
    AtlasEntry { bit: 3, key: "wild_flower",    name: "野花原",   emoji: "🌸", tier: "common" },
    // ── 稀有奇景（遠郊與翠幽星一帶）──
    AtlasEntry { bit: 4, key: "ancient_ruin",   name: "遠古遺跡", emoji: "🏛️", tier: "rare" },
    AtlasEntry { bit: 5, key: "coral_reef",     name: "珊瑚礁",   emoji: "🪸", tier: "rare" },
    AtlasEntry { bit: 6, key: "jade_vine",      name: "翠蔓地",   emoji: "🌿", tier: "rare" },
    AtlasEntry { bit: 7, key: "lava_rock",      name: "熔岩荒原", emoji: "🌋", tier: "rare" },
    // ── 傳奇奇景（虛空／霧醚／星源等險遠秘境）──
    AtlasEntry { bit: 8,  key: "void_crystal",   name: "虛空晶簇", emoji: "🌌", tier: "legendary" },
    AtlasEntry { bit: 9,  key: "aether_mist",    name: "霧醚秘境", emoji: "🌫️", tier: "legendary" },
    AtlasEntry { bit: 10, key: "origin_crystal", name: "星源寶地", emoji: "🟡", tier: "legendary" },
];

/// 探索圖鑑總條目數（前端顯示「N / TOTAL」，亦為合法位元上界）。
pub const TOTAL: u32 = CATALOG.len() as u32;

/// 踏足一種地形給的乙太獎勵，依稀有度遞增（刻意壓小、近乎零經濟擾動，但愈遠愈值得跑）。
pub const REWARD_COMMON: u32 = 4;
pub const REWARD_RARE: u32 = 10;
pub const REWARD_LEGENDARY: u32 = 20;

/// 玩家走進多少像素內即「探索」一種地形。
/// 地形多為實心格、玩家被碰撞擋在外緣，故取略大於一格半（48 = 1.5 × TILE_PX）的取樣半徑，
/// 讓你貼近某片奇景地形（即便進不去）就能把它記下。
pub const EXPLORE_REACH: f32 = 48.0;

/// 地形種 → 探索圖鑑位元（穩定對應；與 CATALOG 一致）。
/// 基本地形（空地／土／石）與人造城牆無處不在、不算「探索」，一律回 `None`。
pub fn bit_for_tile(kind: TileKind) -> Option<u8> {
    match kind {
        TileKind::Ore => Some(0),
        TileKind::Crystal => Some(1),
        TileKind::Mushroom => Some(2),
        TileKind::WildFlower => Some(3),
        TileKind::AncientRuin => Some(4),
        TileKind::CoralReef => Some(5),
        TileKind::JadeVine => Some(6),
        TileKind::LavaRock => Some(7),
        TileKind::VoidCrystal => Some(8),
        TileKind::AetherMist => Some(9),
        TileKind::OriginCrystal => Some(10),
        // 無處不在的基本地形與人造物，不收錄。
        TileKind::Empty | TileKind::Dirt | TileKind::Stone | TileKind::TownWall => None,
    }
}

/// 某位元的乙太獎勵（依該條目稀有度查表）。位元越界回 0。
pub fn reward_for_bit(bit: u8) -> u32 {
    match CATALOG.iter().find(|e| e.bit == bit).map(|e| e.tier) {
        Some("legendary") => REWARD_LEGENDARY,
        Some("rare") => REWARD_RARE,
        Some("common") => REWARD_COMMON,
        _ => 0,
    }
}

/// 某位元是否已探索。
pub fn is_explored(mask: u64, bit: u8) -> bool {
    mask & (1u64 << bit) != 0
}

/// 在探索圖鑑記下一個位元。回傳 `(新 mask, 是否本次才首次探索)`。
/// 已探索過則 mask 不變、回 `false`（天然冪等、不可重複領獎）。
pub fn explore(mask: u64, bit: u8) -> (u64, bool) {
    if is_explored(mask, bit) {
        (mask, false)
    } else {
        (mask | (1u64 << bit), true)
    }
}

/// 已探索的地形數（只計合法位元，忽略任何高位雜訊）。
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

#[cfg(test)]
mod tests {
    use super::*;

    /// bit 連續、唯一、且在合法上界內（持久化相容契約）。
    #[test]
    fn bits_are_contiguous_unique_and_bounded() {
        for (i, e) in CATALOG.iter().enumerate() {
            assert_eq!(e.bit as usize, i, "bit 必須與索引一致（連續、不可重排）");
            assert!((e.bit as u32) < 64, "bit 必須落在 u64 範圍內");
        }
        assert_eq!(TOTAL as usize, CATALOG.len());
    }

    /// key 不重複（前端據此對應圖示與在地化字串）。
    #[test]
    fn keys_are_unique_and_nonempty() {
        let mut seen = std::collections::HashSet::new();
        for e in CATALOG {
            assert!(!e.key.is_empty() && !e.name.is_empty() && !e.emoji.is_empty());
            assert!(seen.insert(e.key), "key 重複：{}", e.key);
        }
    }

    /// 每個收錄地形都對到正確的位元，且兩邊一致。
    #[test]
    fn tile_maps_to_catalog_bit() {
        let kinds = [
            (TileKind::Ore, "ore"),
            (TileKind::Crystal, "crystal"),
            (TileKind::Mushroom, "mushroom"),
            (TileKind::WildFlower, "wild_flower"),
            (TileKind::AncientRuin, "ancient_ruin"),
            (TileKind::CoralReef, "coral_reef"),
            (TileKind::JadeVine, "jade_vine"),
            (TileKind::LavaRock, "lava_rock"),
            (TileKind::VoidCrystal, "void_crystal"),
            (TileKind::AetherMist, "aether_mist"),
            (TileKind::OriginCrystal, "origin_crystal"),
        ];
        for (kind, key) in kinds {
            let bit = bit_for_tile(kind).expect("收錄地形必須有位元");
            let entry = CATALOG.iter().find(|e| e.bit == bit).unwrap();
            assert_eq!(entry.key, key, "{kind:?} 應對到 {key}");
        }
        // 全收錄起來剛好涵蓋整本圖鑑。
        assert_eq!(kinds.len() as u32, TOTAL);
    }

    /// 基本地形與城牆不收錄（無處不在、不算探索）。
    #[test]
    fn basic_terrain_not_catalogued() {
        for kind in [TileKind::Empty, TileKind::Dirt, TileKind::Stone, TileKind::TownWall] {
            assert_eq!(bit_for_tile(kind), None, "{kind:?} 不該被收錄");
        }
    }

    /// explore 設位元、回報首次；重複探索冪等不改 mask。
    #[test]
    fn explore_sets_bit_and_is_idempotent() {
        let (m1, first1) = explore(0, 3);
        assert!(first1);
        assert!(is_explored(m1, 3));
        let (m2, first2) = explore(m1, 3);
        assert!(!first2, "重複探索不應再回首次");
        assert_eq!(m1, m2, "冪等：mask 不變");
    }

    /// count 正確計數，且忽略合法位元以外的高位雜訊。
    #[test]
    fn count_ignores_out_of_range_noise() {
        assert_eq!(count(0), 0);
        let full = valid_mask();
        assert_eq!(count(full), TOTAL);
        // 在合法範圍外再塞一個高位，count 不受影響。
        assert_eq!(count(full | (1u64 << 63)), TOTAL);
    }

    /// 獎勵依稀有度遞增、皆為正；位元越界回 0。
    #[test]
    fn rewards_increase_with_rarity() {
        assert!(REWARD_COMMON > 0);
        assert!(REWARD_RARE > REWARD_COMMON);
        assert!(REWARD_LEGENDARY > REWARD_RARE);
        for e in CATALOG {
            let r = reward_for_bit(e.bit);
            assert!(r > 0, "{} 的獎勵應為正", e.key);
            match e.tier {
                "common" => assert_eq!(r, REWARD_COMMON),
                "rare" => assert_eq!(r, REWARD_RARE),
                "legendary" => assert_eq!(r, REWARD_LEGENDARY),
                other => panic!("未知稀有度 {other}"),
            }
        }
        assert_eq!(reward_for_bit(200), 0, "越界位元無獎勵");
    }

    /// 每種稀有度至少各有一筆（面板三段配色都有內容）。
    #[test]
    fn every_tier_present() {
        for tier in ["common", "rare", "legendary"] {
            assert!(CATALOG.iter().any(|e| e.tier == tier), "缺少稀有度 {tier}");
        }
    }
}
