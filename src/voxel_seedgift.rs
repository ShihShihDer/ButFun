//! 乙太方界·居民種下你送的種子，長成她自己的一畦菜園 v1（ROADMAP 754）。
//!
//! **核心信念**：「你的互動有後果」＋ PLAN_ETHERVOX 反覆點名的「交織點」——人類種田的
//! 樂趣（種子/採集）與 AI 居民的生活（記憶/能動性）在同一片方塊天地交織。至今種田（659~）
//! 一路是玩家單機的樂趣，居民從不下田；而玩家送居民的種子（660 贈禮），也只化成一句
//! 「謝謝」的記憶，種子本身沒有去處。本切片把兩條線第一次接起來：**當你把種子送給一位
//! 已經和你要好的居民，她會把這份心意真的種進家旁的土裡**——居民第一次參與種田系統、
//! 成為「生產者」，而你的餽贈不再只是一句記憶，而是在世界裡生根、隨既有農地 tick 長大，
//! 你日後路過還看得到那畦因你而生的菜園。
//!
//! **與既有系統的分界**：這不是 732「紀念物」（把可展示的禮物擺成**靜態**裝飾方塊），而是
//! **會生長的作物**——由既有 `FarmStore` 計時、隨 `tick_farm` 成熟，長成活的菜園，是居民第一次
//! 踏進「種田」這條玩家玩法線。也不是 748「居民互贈」（居民↔居民搬既有背包材料），而是
//! **玩家餽贈→居民把它種下→世界長出新作物**的全新後果鏈。
//!
//! **純邏輯層**：種子判定、可耕地挑選（closure 取方塊、確定性、可窮舉測試）、台詞/記憶/Feed
//! 文案。無 WS / 鎖 / IO 細節——由 `voxel_ws.rs` 的贈禮流程包進鎖後呼叫；確定性、可測、零 LLM。

use crate::voxel_farm::{CARROT_SEEDS_ID, POTATO_SEEDS_ID, SEEDS_ID};

/// 居民願意「幫你把種子種下」的好感門檻（沿用「友人」＝金心的 FRIEND_AFFINITY_THRESHOLD=3）。
/// 得先和你要好，她才會把你的餽贈當一回事、鄭重地種進自家門前。
pub const PLANT_AFFINITY: usize = 3;

/// 開墾菜園時，居民願意在自身周圍搜尋可耕地的水平半徑（方塊）。
/// 略大於贈禮/放置的伸手範圍：她會在家門口附近走幾步找塊好地。
pub const GARDEN_RADIUS: i32 = 3;

/// 可耕地相對居民腳下允許的最大高低差（搆得到、不爬牆不跳坑）。
const MAX_DY: i32 = 2;

// 判定可耕地用的方塊 id（對齊 `voxel::Block`：0=空氣、1=草、2=泥土）。
const AIR: u8 = 0;
const GRASS: u8 = 1;
const DIRT: u8 = 2;

/// 這顆種子（贈禮 item_id）能不能種？能的話回傳中文作物名（供台詞/記憶/Feed）。
/// 對齊 `voxel_farm` 三種作物的種子 id；非種子（其他禮物）回 None → 走既有贈禮路徑、行為零改變。
pub fn plantable_crop_name(item_id: u8) -> Option<&'static str> {
    match item_id {
        SEEDS_ID => Some("小麥"),
        CARROT_SEEDS_ID => Some("胡蘿蔔"),
        POTATO_SEEDS_ID => Some("馬鈴薯"),
        _ => None,
    }
}

/// 在居民 (rx, ry, rz) 周圍找一塊最近、可以開墾的地面格（草或泥土、正上方為空氣、搆得到）。
/// 回傳那塊**要被翻成作物**的地面格座標；找不到就 None（誠實不種＝天然防洗版，地一滿就停）。
///
/// `block_at(x,y,z)` 由呼叫端提供（包進 delta 讀鎖），回傳該格方塊 id。純函式、無副作用、可窮舉測試。
pub fn find_garden_spot<F>(rx: f32, ry: f32, rz: f32, block_at: F) -> Option<(i32, i32, i32)>
where
    F: Fn(i32, i32, i32) -> u8,
{
    let (fx, fy, fz) = (rx.floor() as i32, ry.floor() as i32, rz.floor() as i32);
    let mut best: Option<((i32, i32, i32), i32)> = None;
    for dx in -GARDEN_RADIUS..=GARDEN_RADIUS {
        for dz in -GARDEN_RADIUS..=GARDEN_RADIUS {
            if dx == 0 && dz == 0 {
                continue; // 不種在她自己腳下那格
            }
            let (x, z) = (fx + dx, fz + dz);
            // 由高往低找這一欄最上面的地面：草/泥土且正上方是空氣 = 可耕地。
            for y in (fy - MAX_DY..=fy + 1).rev() {
                let b = block_at(x, y, z);
                if b == AIR {
                    continue; // 空中，再往下找地面
                }
                // 撞到實心：只有草/泥土、且頭頂是空氣、且搆得到才算可耕地。
                if (b == GRASS || b == DIRT)
                    && block_at(x, y + 1, z) == AIR
                    && (y - fy).abs() <= MAX_DY
                {
                    let d2 = dx * dx + dz * dz;
                    if best.map_or(true, |(_, bd2)| d2 < bd2) {
                        best = Some(((x, y, z), d2));
                    }
                }
                break; // 撞到第一塊實心就停（不挖它底下、被石頭/建物擋住就放棄這欄）
            }
        }
    }
    best.map(|(c, _)| c)
}

/// 居民種下時頭頂的暖句（四句輪替，`take(40)` 由呼叫端收框）。
/// `crop`＝中文作物名，`player`＝送禮玩家名（空字串則用「你」）。
pub fn plant_say_line(player: &str, crop: &str, pick: usize) -> String {
    let who = if player.is_empty() { "你" } else { player };
    let lines = [
        format!("{who}送的{crop}種子，我種在家門口了，讓它好好長大！"),
        format!("這麼好的{crop}種子怎麼捨得吃掉，種下去才對嘛～"),
        format!("謝謝{who}！我把{crop}種在家旁，開了一小畦菜園～"),
        format!("有了{who}的{crop}種子，我也來當個農夫試試看！"),
    ];
    lines[pick % lines.len()].clone()
}

/// 居民記憶摘要（掛玩家名下，`🌱` 前綴供日記/回想歸類）。
pub fn plant_memory_line(player: &str, crop: &str) -> String {
    let who = if player.is_empty() { "你" } else { player };
    format!("🌱把{who}送的{crop}種子種在家旁，盼它長成一畦菜園")
}

/// 動態牆一行。
pub fn plant_feed_line(rname: &str, player: &str, crop: &str) -> String {
    let who = if player.is_empty() { "有人" } else { player };
    format!("{rname}把{who}送的{crop}種子種下，開墾了自己的一畦菜園～")
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── 種子判定 ──────────────────────────────────────────────────────────
    #[test]
    fn plantable_covers_three_seeds() {
        assert_eq!(plantable_crop_name(SEEDS_ID), Some("小麥"));
        assert_eq!(plantable_crop_name(CARROT_SEEDS_ID), Some("胡蘿蔔"));
        assert_eq!(plantable_crop_name(POTATO_SEEDS_ID), Some("馬鈴薯"));
    }

    #[test]
    fn non_seed_gift_not_plantable() {
        assert_eq!(plantable_crop_name(5), None); // 木頭
        assert_eq!(plantable_crop_name(0), None); // 空氣
        assert_eq!(plantable_crop_name(19), None); // 麵包
    }

    // ── 可耕地挑選 ────────────────────────────────────────────────────────
    // 世界：y=4 一整層草地，y>=5 皆空氣，y<=3 皆泥土（實心地基）。
    fn flat_grass(x: i32, y: i32, z: i32) -> u8 {
        let _ = (x, z);
        if y >= 5 {
            AIR
        } else if y == 4 {
            GRASS
        } else {
            DIRT
        }
    }

    #[test]
    fn finds_nearest_grass_around_resident() {
        // 居民站在 (0.5, 5.0, 0.5)（腳在 y=5，草在 y=4）。
        let spot = find_garden_spot(0.5, 5.0, 0.5, flat_grass);
        // 找得到、是草層(y=4)、且是最近的一格（chebyshev=1）。
        let (x, y, z) = spot.expect("平坦草地應找得到可耕地");
        assert_eq!(y, 4, "要種在草層");
        assert_eq!(x * x + z * z, 1, "應挑最近的一格（距離平方=1）");
    }

    #[test]
    fn skips_own_feet_cell() {
        // 只有 (0,4,0) 一格是草，其餘皆空氣/泥土無草——但那格在她腳下，不該選。
        let only_under = |x: i32, y: i32, z: i32| {
            if x == 0 && z == 0 && y == 4 {
                GRASS
            } else if y <= 3 {
                DIRT
            } else {
                AIR
            }
        };
        // 泥土層(y<=3)頭頂被草或空氣蓋著：(0,3,0) 頭頂是草(非空氣)不算；其餘泥土頭頂是空氣→可耕。
        // 重點：不會選到 (0,4,0) 她腳下那格草。
        let spot = find_garden_spot(0.5, 5.0, 0.5, only_under);
        if let Some((x, _, z)) = spot {
            assert!(!(x == 0 && z == 0), "不可種在她自己腳下");
        }
    }

    #[test]
    fn no_ground_returns_none() {
        // 四周全是空氣（懸空）→ 找不到可耕地，誠實回 None。
        let all_air = |_x: i32, _y: i32, _z: i32| AIR;
        assert_eq!(find_garden_spot(0.5, 5.0, 0.5, all_air), None);
    }

    #[test]
    fn stone_covered_ground_not_tillable() {
        // 地面是石頭(3)不是草/土 → 不可耕，回 None。
        let stone = |_x: i32, y: i32, _z: i32| if y >= 5 { AIR } else { 3 };
        assert_eq!(find_garden_spot(0.5, 5.0, 0.5, stone), None);
    }

    #[test]
    fn covered_grass_not_tillable() {
        // 草地頭頂全壓著石頭(3) → 正上方非空氣，不可種。
        let capped = |_x: i32, y: i32, _z: i32| match y {
            4 => GRASS,
            5 => 3, // 石頭蓋頂
            _ if y <= 3 => DIRT,
            _ => AIR,
        };
        assert_eq!(find_garden_spot(0.5, 5.0, 0.5, capped), None);
    }

    #[test]
    fn out_of_reach_ground_skipped() {
        // 可耕草地在 y=1（居民腳在 y=5，高低差 4 > MAX_DY=2）→ 搆不到，跳過。
        let deep = |_x: i32, y: i32, _z: i32| {
            if y == 1 {
                GRASS
            } else if y == 0 {
                DIRT
            } else {
                AIR
            }
        };
        assert_eq!(find_garden_spot(0.5, 5.0, 0.5, deep), None);
    }

    // ── 文案 ──────────────────────────────────────────────────────────────
    #[test]
    fn say_line_rotates_and_names_crop_and_player() {
        let a = plant_say_line("露娜", "小麥", 0);
        let b = plant_say_line("露娜", "小麥", 1);
        assert_ne!(a, b, "不同 pick 給不同台詞");
        for p in 0..8 {
            let s = plant_say_line("露娜", "馬鈴薯", p);
            assert!(s.contains("馬鈴薯"), "台詞要提到作物");
        }
        assert!(plant_say_line("諾娃", "胡蘿蔔", 0).contains("諾娃"));
    }

    #[test]
    fn say_line_empty_player_uses_you() {
        let s = plant_say_line("", "小麥", 0);
        assert!(s.contains("你"));
    }

    #[test]
    fn memory_line_has_prefix_and_names() {
        let m = plant_memory_line("露娜", "小麥");
        assert!(m.starts_with("🌱"), "記憶要帶前綴供日記歸類");
        assert!(m.contains("露娜") && m.contains("小麥"));
    }

    #[test]
    fn feed_line_names_both_and_crop() {
        let f = plant_feed_line("諾娃", "旅人", "胡蘿蔔");
        assert!(f.contains("諾娃") && f.contains("旅人") && f.contains("胡蘿蔔"));
    }
}
