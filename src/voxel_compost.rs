//! 乙太方界·乙太沃肥 v1——把割下的雜草與泥土漚成一撮沃肥，撒在幼苗上讓它抽長一截。
//!
//! **玩家遊玩並行主軸**（PLAN_ETHERVOX「維護者也想享受這世界」）：此前種田只有「撒種→乾等」
//! 一種節奏，作物長多快完全由系統決定（自然 90/60/120s、鄰水加速一半），玩家**沒有任何主動
//! 加速的動詞**——只能站著等。這一刀補上那一拍：把採集雜草與挖土時囤下、平常用不完的
//! 雜草(Grass=1)＋泥土(Dirt=2)，在工作台漚成「乙太沃肥」；手持沃肥對準一株還在長的幼苗一撒，
//! 它的生長計時**立刻往前跳一截**（沿用農地既有 [`crate::voxel_farm::FarmStore::nudge_growth`]，
//! 與居民路過照料 753 同一套機制、同樣持久化），急著要收成時第一次能「主動催熟」。
//!
//! **與既有機制的分界（換維度·非同軸重複）**：
//!   - 鄰水加速（水耕 686）是**被動、環境決定**（種在水邊就快，種下後無從介入）；
//!   - 居民照料（753）是**居民自發**、玩家不可控；
//!   - 本刀是**玩家主動施放**、消耗親手漚的沃肥、對指定那株催熟——採集→合成→施肥的新玩法動詞，
//!     把「用不完的雜草泥土」第一次接上「主動加速農業循環」。
//!
//! **成本鐵律**：純規則式（零 LLM、零 IO、零鎖、零 async），可單元測試。連線 / 鎖 / delta /
//! 農地計時 / 廣播全留在 `voxel_ws.rs`（沿用 Plant handler 的短鎖循序慣例，守 prod 死鎖鐵律）。
//! **濫用防護**：施肥座標由伺服器以玩家自身位置做觸及驗證、目標方塊型別後端權威判定（前端不自報
//! 合法性）；放不了不消耗沃肥（白嫖不到）；沃肥產出走既有工作台合成、天然節流。不抄外部碼；繁中註解。

use crate::voxel::Block;

/// 乙太沃肥物品 id（純物品，住背包、不可放置於世界；施肥即消耗一份）。
/// 接續料理(67)/煙火(68) 之後的下一個空閒 id；`Block::from_u8(69) = None`（非方塊）。
pub const FERTILIZER_ID: u8 = 69;

/// 一撮沃肥把作物生長計時往前推進的秒數。
///
/// 取 40s：對最快的胡蘿蔔(60s)是「省下大半」、對最慢的馬鈴薯(120s)是「催掉三分之一」，
/// 一撮有感、卻不會一撮就秒熟任何作物（要真的想催熟得連撒幾撮），維持節奏張力。
pub const FERTILIZER_BOOST_SECS: u64 = 40;

/// 目標方塊是否為「還在長的幼苗」（三種作物的 Seeded 狀態）——可施肥的唯一目標。
///
/// 成熟作物 / 農田土 / 其他方塊都不是幼苗，撒沃肥無意義（伺服器據此拒絕、不消耗沃肥）。
/// 純函式、確定性，供 `voxel_ws` 施肥前權威判定。
pub fn is_growing_crop(block: Block) -> bool {
    matches!(
        block,
        Block::FarmSoilSeeded | Block::CarrotSeeded | Block::PotatoSeeded
    )
}

/// 玩家成功施肥後冒的回饋字串（飄字用；確定性輪替、點名作物）。
///
/// `crop`＝作物顯示名（如「胡蘿蔔」）、`pick`＝呼叫端給的挑選數（取真隨機）。
pub fn fertilize_say_line(crop: &str, pick: usize) -> String {
    let variants: [&str; 3] = [
        "撒下一撮沃肥，{crop}抽長了一截～",
        "沃肥入土，{crop}長得更精神了！",
        "催一催——{crop}離收成又近了一步。",
    ];
    variants[pick % variants.len()].replace("{crop}", crop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fertilizer_id_is_pure_item() {
        // 69 不對應任何方塊（純物品，住背包、不可放置）——與料理/煙火同款。
        assert!(Block::from_u8(FERTILIZER_ID).is_none());
        // 契約鎖定：前後端對齊此 id。
        assert_eq!(FERTILIZER_ID, 69);
    }

    #[test]
    fn only_seeded_crops_are_fertilizable() {
        // 三種作物的幼苗皆可施肥。
        assert!(is_growing_crop(Block::FarmSoilSeeded));
        assert!(is_growing_crop(Block::CarrotSeeded));
        assert!(is_growing_crop(Block::PotatoSeeded));
        // 成熟作物、農田土、雜項方塊皆不可施肥（撒了無意義、不該消耗沃肥）。
        assert!(!is_growing_crop(Block::WheatMature));
        assert!(!is_growing_crop(Block::CarrotMature));
        assert!(!is_growing_crop(Block::PotatoMature));
        assert!(!is_growing_crop(Block::FarmSoil));
        assert!(!is_growing_crop(Block::Grass));
        assert!(!is_growing_crop(Block::Dirt));
        assert!(!is_growing_crop(Block::Air));
    }

    #[test]
    fn boost_is_meaningful_but_not_instant() {
        // 一撮沃肥對最慢的馬鈴薯(120s)也催不到秒熟，維持「要多撒幾撮」的節奏張力。
        assert!(FERTILIZER_BOOST_SECS > 0);
        assert!(FERTILIZER_BOOST_SECS < crate::voxel_farm::POTATO_GROW_SECS);
    }

    #[test]
    fn say_line_rotates_and_embeds_crop() {
        // 三句輪替、皆嵌入作物名、皆非空。
        for pick in 0..6 {
            let s = fertilize_say_line("胡蘿蔔", pick);
            assert!(!s.is_empty());
            assert!(s.contains("胡蘿蔔"));
        }
        // 確定性：同輸入同輸出。
        assert_eq!(
            fertilize_say_line("小麥", 0),
            fertilize_say_line("小麥", 0)
        );
        // 不同 pick 取到不同句（至少 0 與 1 不同）。
        assert_ne!(
            fertilize_say_line("小麥", 0),
            fertilize_say_line("小麥", 1)
        );
    }
}
