//! 乙太方界·玩家裝飾傢俱 v1（furniture）——給玩家幾樣**可合成、可放置的純裝飾傢俱**，
//! 讓親手蓋的「家」多幾分生活感與個人風格。
//!
//! **這一刀補的缺口**：世界至今可放的方塊絕大多數是**建材**（木板/石磚/玻璃/陶磚…）、
//! **工具站**（工作台/熔爐/箱子）、**功能家具**（床能睡、長椅招居民坐、告示牌能寫字、
//! 鐘能召集）——每一樣都「有用途」。唯獨少了最日常的一類：**純為好看、擺著佈置家園的
//! 小傢俱**。玩家想把小屋裡鋪張地毯、窗邊擺盆花、角落放張小圓桌、牆上掛面旗子，讓家看
//! 起來像「有人住、有品味」，卻沒有任何一塊方塊是為此而生。本刀補上世界第一批**純裝飾
//! 傢俱**：不睡、不招居民、不寫字、不召集，就只是好看——把「佈置一個屬於自己的角落」
//! 這件最療癒的建造樂趣補齊。
//!
//! **與既有元素 razor-sharp 區隔（非同軸重複）**：
//! - **床(45)**＝睡覺跳夜；**長椅(79)**＝白天招居民坐；**告示牌(66)**＝寫字命名；
//!   **集會鐘(74)**＝敲響召集——四者皆**有互動用途**。本刀四樣**零互動、零居民行為、
//!   零計時**，純粹是擺設，一眼與功能家具分開。
//! - **染色陶磚(89~92)/建材**＝鋪牆鋪地的**整格建材**；本刀四樣走**矮塊／薄片**造型
//!   （地毯超薄貼地、花盆矮墩、圓桌矮桌、掛旗薄面），一眼是「一件傢俱」而非一塊建材。
//!
//! **純函式層**：本模組只有 id 常數與確定性純函式（`is_furniture` 判定），零 LLM、
//! 零鎖、零 async、零 IO、零計時、零持久化狀態（傢俱只是普通可放置方塊，走既有 world
//! delta 持久化管線，放下即持久、破壞回收自身、重啟 replay 自然還原）。放置／破壞語意
//! 與其餘建材完全一致，因此不需要像長椅/雞舍那樣的 in-memory 索引或 tick——這正是「純
//! 裝飾」最省的形態。連線／鎖／廣播全留在 `voxel_ws.rs`（沿用既有慣例，守 prod 死鎖鐵律）。

use crate::voxel::Block;

/// 小地毯（furniture v1）——方塊／物品 ID 102（101 鐵劍是目前最大已用 id，102 是首個空號）。
/// 超薄貼地的一片織毯，鋪在家裡的地板上暖一暖腳。合成：3 葉片(6) → 2 小地毯。
pub const CARPET_ID: u8 = 102;
/// 花盆（furniture v1）——方塊／物品 ID 103。矮墩上冒一小簇綠意，窗邊桌角擺一盆。
/// 合成：2 紅陶磚(89) + 1 葉片(6) → 1 花盆（陶盆盛土栽一株）。
pub const FLOWERPOT_ID: u8 = 103;
/// 小圓桌（furniture v1）——方塊／物品 ID 104。一張矮矮的木圓桌，屋裡待客擺茶的中心。
/// 合成：2 木板(8) + 1 石磚(9) → 1 小圓桌（木桌面＋石底座）。
pub const TABLE_ID: u8 = 104;
/// 掛旗（furniture v1）——方塊／物品 ID 105。一面垂掛的暖色旗幟，掛在牆上宣告「這是我家」。
/// 合成：1 木頭(5) + 2 葉片(6) → 1 掛旗（木旗桿＋織布旗面）。
pub const BANNER_ID: u8 = 105;

/// 該方塊是否為本刀新增的「純裝飾傢俱」（放置／破壞走通用建材路徑，不需特殊索引/tick）。
/// 純函式、確定性、無副作用、可測。i18n 無涉（純判定）。
pub fn is_furniture(b: Block) -> bool {
    matches!(b, Block::Carpet | Block::FlowerPot | Block::Table | Block::Banner)
}

/// 該物品 id 是否為本刀新增的裝飾傢俱（前端／背包等以 u8 判定時用，與 `is_furniture` 對齊）。
/// 純函式、確定性、可測。
pub fn is_furniture_id(id: u8) -> bool {
    matches!(id, CARPET_ID | FLOWERPOT_ID | TABLE_ID | BANNER_ID)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_match_block_enum() {
        // 常數與 Block 列舉一致（防日後改 id 忘了同步）。
        assert_eq!(CARPET_ID, Block::Carpet as u8);
        assert_eq!(FLOWERPOT_ID, Block::FlowerPot as u8);
        assert_eq!(TABLE_ID, Block::Table as u8);
        assert_eq!(BANNER_ID, Block::Banner as u8);
    }

    #[test]
    fn ids_are_the_first_free_slots() {
        // 102~105：101（鐵劍 SWORD_IRON_ID）是先前最大已用 id，本刀四樣接續其後、不撞號。
        assert_eq!(CARPET_ID, 102);
        assert_eq!(FLOWERPOT_ID, 103);
        assert_eq!(TABLE_ID, 104);
        assert_eq!(BANNER_ID, 105);
    }

    #[test]
    fn is_furniture_recognises_all_four_and_nothing_else() {
        for b in [Block::Carpet, Block::FlowerPot, Block::Table, Block::Banner] {
            assert!(is_furniture(b), "{b:?} 應被認出是裝飾傢俱");
        }
        // 功能家具（床/長椅/告示牌/鐘）與建材皆不算「純裝飾傢俱」。
        for b in [
            Block::Bed,
            Block::Bench,
            Block::Sign,
            Block::Bell,
            Block::Plank,
            Block::Stone,
            Block::Air,
            Block::TerracottaRed,
        ] {
            assert!(!is_furniture(b), "{b:?} 不該被認成裝飾傢俱");
        }
    }

    #[test]
    fn is_furniture_id_matches_block_helper() {
        // u8 版與 Block 版判定一致：對四樣傢俱皆 true，對其餘常見 id 皆 false。
        for id in [CARPET_ID, FLOWERPOT_ID, TABLE_ID, BANNER_ID] {
            assert!(is_furniture_id(id));
        }
        for id in [0u8, 45, 66, 74, 79, 8, 3, 89, 101] {
            assert!(!is_furniture_id(id), "id={id} 不該被認成裝飾傢俱");
        }
    }

    #[test]
    fn all_furniture_are_placeable_and_solid() {
        // 四樣皆為玩家可放置、且視為實心方塊（走通用建材放置／破壞／回收路徑）。
        for b in [Block::Carpet, Block::FlowerPot, Block::Table, Block::Banner] {
            assert!(b.is_placeable(), "{b:?} 應可放置");
            assert!(b.is_solid(), "{b:?} 應為實心（走通用掉落自身路徑）");
        }
    }
}
