//! 乙太方界·居民堆雪人 v1（自主提案切片，ROADMAP 918）。
//!
//! **缺口 / 為誰做**：冬季飄雪（900）已經讓天地換上白雪、居民抬頭感嘆初雪（900）、冷天更想
//! 圍爐取暖（901）——但雪始終只是「從天上落下的視覺」，居民從沒有因為下雪而**在世界裡留下
//! 一個屬於冬天的、看得見摸得著的東西**。舊 2D 世界玩家能堆署名雪人（ROADMAP 478），voxel
//! 世界卻從沒有過任何「雪人」；冬天過去只剩一則初雪的文字回憶，地上什麼都沒多。
//!
//! **做法**：冬天飄雪時，閒著、醒著、沒在朝聖／遠行的居民偶爾童心大起，在身旁空地**堆起一個
//! 小雪人**（兩顆雪塊疊成身與頭、頂上一盞冰燈當會發光的帽子，清楚是「做出來的東西」而非天然
//! 雪柱）；牠冒一句歡快的話、把「今年冬天我堆了個雪人」記進心裡，城鎮動態牆也留一則。雪人是
//! 冬天限定的短暫歡樂：**冬天一結束就全部融化**（清回空氣、動態牆記一句「雪人們融化了」）。
//!
//! **與既有元素的區隔**：901 冬寒圍爐＝居民靠近**玩家蓋的**營火取暖（只念台詞、不動世界）；
//! 本刀＝居民**主動在世界裡放置方塊**堆出一個新物件，是「記憶／季節驅動行為→世界長出新東西」。
//!
//! **成本紀律（鐵律）**：零 LLM（觸發、選點、台詞全是確定性純函式）、零 migration、零新持久化
//! 格式（雪人只存在記憶體＋世界 delta，**重啟即消失、冬末即融化**，比照 smelt/invent 的純記憶體
//! 狀態）、零新美術（沿用既有的雪塊 55／冰燈 57，前端本就會渲染）、FPS 零影響（全世界至多
//! [`MAX_SNOWMEN`] 個、每個 3 塊，掛在低頻節拍上）。
//!
//! **純邏輯層**：本檔全是零 IO、零鎖、零 async 的確定性純函式／常數；放置方塊、廣播、清除、
//! 記憶、Feed 的副作用都在 `voxel_ws.rs`（短鎖循序即釋、不巢狀，守 prod 死鎖鐵律）。

use crate::voxel::Block;

/// 堆雪人記憶掛的偽玩家標籤（比照 `voxel_bedtime` 的 `REFLECT_MEMORY_PLAYER`）：雪人不是關於
/// 某位玩家的事，掛在世界級哨兵鍵下，讓這筆記憶可被辨識、不誤算進對任何玩家的好感。
pub const SNOWMAN_MEMORY_PLAYER: &str = "__voxel_snowman__";

/// 動態牆播報種類：堆起一個雪人。
pub const FEED_KIND_BUILD: &str = "堆雪人";
/// 動態牆播報種類：冬末雪人融化。
pub const FEED_KIND_MELT: &str = "雪人融化";

/// 全村堆雪人冷卻（秒）：至多每這麼久新添一個雪人——刻意拉長，讓雪人稀少而有份量、不洗版佔地。
pub const BUILD_COOLDOWN_SECS: u64 = 150;

/// 世界同時最多幾個雪人：超過就不再堆（防洗版、防佔滿空地、守 FPS）。
pub const MAX_SNOWMEN: usize = 4;

/// 通過前置閘（飄雪＋冷卻到期＋未達上限）後仍要擲骰的觸發機率：堆雪人是偶爾的童心，不是每拍必成。
pub const BUILD_CHANCE: f32 = 0.35;

/// 兩個雪人之間最小水平間距（方塊）：別擠成一堆，散落各處才好看。
pub const MIN_SEPARATION: f32 = 6.0;

/// 雪人堆在居民身旁幾格外（不堆在腳下擋路）。
pub const ANCHOR_DIST: i32 = 2;

/// 一個雪人＝它實際佔用的方塊座標（供冬末融化時逐格清除）＋建造者身分。
#[derive(Debug, Clone)]
pub struct Snowman {
    /// 這個雪人由哪些世界座標的方塊組成（融化時逐格設回空氣）。
    pub blocks: Vec<(i32, i32, i32)>,
    /// 建造者居民 id。
    pub builder_id: String,
    /// 建造者名字（供 Feed）。
    pub builder_name: &'static str,
}

/// 雪人由哪些方塊堆成：給定地面正上方一格 `(x, y, z)`，回傳（身：雪、頭：雪、頂：冰燈）。
/// 頂上那盞冰燈讓它一眼看出是「做出來的」而非天然雪柱，夜裡還會發光。
pub fn snowman_blocks(x: i32, y: i32, z: i32) -> [(i32, i32, i32, Block); 3] {
    [
        (x, y, z, Block::Snow),         // 身
        (x, y + 1, z, Block::Snow),     // 頭
        (x, y + 2, z, Block::IceLantern), // 會發光的帽子
    ]
}

/// 這個方塊型別是否「屬於雪人」——融化時只清這些型別，避免誤刪玩家後來放在原地的其他東西。
pub fn is_snowman_block(b: Block) -> bool {
    matches!(b, Block::Snow | Block::IceLantern)
}

/// 前置閘：這一 tick 是否有資格堆雪人（純判定，實際選點／放置在呼叫端）。
/// - `snowing`：冬季 ∧ 下雨（與 900 冬雪同一事實來源）。
/// - `cooldown_ready`：全村冷卻已到期。
/// - `current_count`：目前世界上已有幾個雪人。
/// - `roll`：擲骰（0.0..1.0）。
pub fn should_build(snowing: bool, cooldown_ready: bool, current_count: usize, roll: f32) -> bool {
    snowing && cooldown_ready && current_count < MAX_SNOWMEN && roll < BUILD_CHANCE
}

/// 給定建造者水平座標，確定性選一個堆雪人的錨點（身旁四方向之一，距 [`ANCHOR_DIST`] 格）。
/// 用 `pick` 取模選方向，讓不同時機／不同居民堆在不同側，不會全擠同一格。
pub fn pick_anchor(rx: f32, rz: f32, pick: usize) -> (i32, i32) {
    let bx = rx.floor() as i32;
    let bz = rz.floor() as i32;
    match pick % 4 {
        0 => (bx + ANCHOR_DIST, bz),
        1 => (bx - ANCHOR_DIST, bz),
        2 => (bx, bz + ANCHOR_DIST),
        _ => (bx, bz - ANCHOR_DIST),
    }
}

/// 錨點是否離所有既有雪人都夠遠（水平距離 ≥ [`MIN_SEPARATION`]）——別把新雪人堆到舊雪人身上。
pub fn far_enough(ax: i32, az: i32, existing: &[(i32, i32)]) -> bool {
    let min_sq = MIN_SEPARATION * MIN_SEPARATION;
    existing.iter().all(|&(ex, ez)| {
        let dx = (ax - ex) as f32;
        let dz = (az - ez) as f32;
        dx * dx + dz * dz >= min_sq
    })
}

/// 堆雪人時居民冒的歡快泡泡（確定性三選一）。
pub fn build_say_line(pick: usize) -> String {
    const LINES: [&str; 3] = [
        "下雪啦！來堆個雪人～⛄",
        "嘿嘿，給這個冬天留個小夥伴！",
        "堆好了！你看牠是不是在對我笑？",
    ];
    LINES[pick % LINES.len()].to_string()
}

/// 堆雪人記進居民心裡的一筆記憶（第一人稱、冬天限定的小確幸）。
pub fn build_memory_line() -> String {
    "今年冬天下雪時，我在空地上堆了一個小雪人，頂上還擺了盞會發光的冰燈。".to_string()
}

/// 堆雪人上城鎮動態牆的一行。
pub fn build_feed_line(name: &str) -> String {
    format!("⛄ {name}在雪地裡堆了一個小雪人！")
}

/// 冬末雪人全部融化時，動態牆的一行（`count` = 這次融化了幾個）。
pub fn melt_feed_line(count: usize) -> String {
    format!("天回暖了，{count} 個雪人靜靜地融化在春光裡。")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snowman_is_three_blocks_snow_snow_lantern() {
        let b = snowman_blocks(10, 64, 20);
        assert_eq!(b[0], (10, 64, 20, Block::Snow));
        assert_eq!(b[1], (10, 65, 20, Block::Snow));
        assert_eq!(b[2], (10, 66, 20, Block::IceLantern));
    }

    #[test]
    fn snowman_blocks_share_one_column() {
        let b = snowman_blocks(3, 40, 7);
        assert!(b.iter().all(|&(x, _, z, _)| x == 3 && z == 7));
    }

    #[test]
    fn is_snowman_block_only_snow_and_lantern() {
        assert!(is_snowman_block(Block::Snow));
        assert!(is_snowman_block(Block::IceLantern));
        assert!(!is_snowman_block(Block::Air));
        assert!(!is_snowman_block(Block::Stone));
        assert!(!is_snowman_block(Block::Wood));
    }

    #[test]
    fn should_build_requires_snowing() {
        assert!(!should_build(false, true, 0, 0.0));
    }

    #[test]
    fn should_build_requires_cooldown_ready() {
        assert!(!should_build(true, false, 0, 0.0));
    }

    #[test]
    fn should_build_respects_cap() {
        assert!(!should_build(true, true, MAX_SNOWMEN, 0.0));
        assert!(should_build(true, true, MAX_SNOWMEN - 1, 0.0));
    }

    #[test]
    fn should_build_respects_chance() {
        assert!(should_build(true, true, 0, BUILD_CHANCE - 0.01));
        assert!(!should_build(true, true, 0, BUILD_CHANCE + 0.01));
        // 邊界：roll == BUILD_CHANCE 不觸發（嚴格小於）。
        assert!(!should_build(true, true, 0, BUILD_CHANCE));
    }

    #[test]
    fn pick_anchor_offsets_by_direction_and_is_deterministic() {
        // 四個方向各距 ANCHOR_DIST，且同 pick 恆得同結果。
        assert_eq!(pick_anchor(5.5, 5.5, 0), (5 + ANCHOR_DIST, 5));
        assert_eq!(pick_anchor(5.5, 5.5, 1), (5 - ANCHOR_DIST, 5));
        assert_eq!(pick_anchor(5.5, 5.5, 2), (5, 5 + ANCHOR_DIST));
        assert_eq!(pick_anchor(5.5, 5.5, 3), (5, 5 - ANCHOR_DIST));
        assert_eq!(pick_anchor(5.5, 5.5, 4), pick_anchor(5.5, 5.5, 0));
    }

    #[test]
    fn pick_anchor_floors_negative_coords() {
        // -0.5 floor 為 -1，確保負座標也正確落格。
        assert_eq!(pick_anchor(-0.5, -0.5, 0), (-1 + ANCHOR_DIST, -1));
    }

    #[test]
    fn far_enough_true_when_no_existing() {
        assert!(far_enough(0, 0, &[]));
    }

    #[test]
    fn far_enough_false_when_too_close() {
        // 距 3 格 < MIN_SEPARATION(6)。
        assert!(!far_enough(0, 0, &[(3, 0)]));
    }

    #[test]
    fn far_enough_true_when_all_far() {
        // 距 10 格 ≥ 6。
        assert!(far_enough(0, 0, &[(10, 0), (0, 10)]));
    }

    #[test]
    fn far_enough_boundary_exactly_min_separation() {
        // 恰好 6 格（≥）視為夠遠。
        assert!(far_enough(0, 0, &[(6, 0)]));
    }

    #[test]
    fn build_say_line_varies_and_nonempty() {
        let a = build_say_line(0);
        let b = build_say_line(1);
        assert_ne!(a, b);
        assert!(!build_say_line(2).is_empty());
        // pick 取模循環。
        assert_eq!(build_say_line(0), build_say_line(3));
    }

    #[test]
    fn build_memory_line_mentions_snowman() {
        assert!(build_memory_line().contains("雪人"));
    }

    #[test]
    fn build_feed_line_includes_name() {
        let line = build_feed_line("露娜");
        assert!(line.contains("露娜"));
        assert!(line.contains("雪人"));
    }

    #[test]
    fn melt_feed_line_includes_count() {
        let line = melt_feed_line(3);
        assert!(line.contains('3'));
        assert!(line.contains("融化"));
    }
}
