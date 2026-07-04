//! 乙太方界·木長椅 v1（bench）——玩家合成一張木長椅擺在世界裡，白天路過、閒著的居民
//! 會**停下腳步坐上去歇一會兒**：說句輕鬆的歇腳話、心情變好，你也在旁時把「和你坐同一張長椅
//! 歇腳」記進交情。
//!
//! **這一刀補的缺口**：乙太方界至今所有「駐足／聚集」的溫暖時刻**全發生在夜裡**——營火圍暖
//! （791）、圍火說故事（792）、繁星共賞（783）都是入夜限定；白天的世界只有居民一刻不停地
//! 走來走去採集、蓋造、串門子，**少了「大白天走累了，找張椅子坐下來歇口氣」這種最日常的一拍**。
//! 木長椅把這一環補上：玩家親手做一張長椅擺在村口、田邊、家門前，白天路過的居民會不由自主
//! 坐下歇腳——你擺越多長椅，白天的村子越有「有人在這兒歇著」的生活氣息。這是「玩家的建造
//! → 塑造居民的**日間**日常」的第一刀，與營火那條夜間線對成白天／夜晚一對。
//!
//! **與既有元素 razor-sharp 區隔（非同軸重複）**：
//! - **營火（791）**＝**夜間**限定、圍**火**取暖、常是**群聚**、烤火念暖語；本刀＝**白天**、坐**椅**
//!   歇腳、**獨自**一人、輕鬆歇口氣——時段（夜／日）、吸引物（火／椅）、社交性（群聚／獨歇）皆不同。
//! - **關鍵新行為**：居民歇腳時會**真的停下移動、原地坐著歇一會兒**（設 `wait_timer`）——這是
//!   世界第一次讓居民「主動停下腳步休息」的行為動詞，不是邊走邊念一句（營火不打斷移動）。
//! - **集會鐘（74）**＝玩家主動敲鐘、居民**循聲走來**聚集；本刀＝被動、居民**恰好路過**才坐下，
//!   玩家不催、不召，只是擺了張椅子在那兒。
//!
//! **純函式層**：本模組只有確定性純函式（就近判定、三閘、台詞、掃描重建），零 LLM、零鎖、
//! 零 async、零 IO、可單元測試。連線／鎖／廣播／持久化觸發全留在 `voxel_ws.rs`（沿用既有
//! 短鎖循序＋鎖外事件佇列慣例，守 prod 死鎖鐵律）。

use crate::voxel;

/// 木長椅方塊／物品 ID（79：0~78 已被純物品／方塊佔用，79 是首個可放置方塊空號）。
pub const BENCH_ID: u8 = 79;

/// 居民坐下歇腳半徑（世界方塊；水平距離）——夠近才算「走到長椅邊」。
pub const REST_RADIUS: f32 = 3.0;
/// 歇腳冷卻（秒）：一次坐下歇腳後隔這麼久才會再歇，防同一居民狂刷歇腳泡泡。
pub const REST_COOLDOWN_SECS: f32 = 120.0;
/// 每次符合條件（白天＋靠近椅＋冷卻到期）時的歇腳觸發機率——其餘時候只是安靜路過。
pub const REST_CHANCE: f32 = 0.28;
/// 坐下歇腳時原地停留的秒數（設進 `wait_timer`）：居民真的停下腳步、坐著歇一會兒。
pub const REST_SIT_SECS: f32 = 5.0;
/// 「你也坐在旁邊」的判定半徑（世界方塊）——你在這麼近，居民的歇腳話就會點你名、記進交情。
pub const REST_PLAYER_RADIUS: f32 = 5.0;

/// 從長椅座標清單中找出離 `(rx, rz)` 最近、且在 `radius` 內的一張（回索引）。
///
/// y 忽略（長椅在水平面上吸引；居民與長椅通常同一地表高度）。同距取索引最小者，
/// None = 半徑內沒有長椅（居民這一 tick 不在任何椅邊）。
pub fn nearest_bench(spots: &[(i32, i32, i32)], rx: f32, rz: f32, radius: f32) -> Option<usize> {
    let r2 = radius * radius;
    let mut best: Option<(usize, f32)> = None;
    for (i, &(x, _, z)) in spots.iter().enumerate() {
        let dx = x as f32 + 0.5 - rx;
        let dz = z as f32 + 0.5 - rz;
        let d2 = dx * dx + dz * dz;
        if d2 <= r2 && best.map_or(true, |(_, bd)| d2 < bd) {
            best = Some((i, d2));
        }
    }
    best.map(|(i, _)| i)
}

/// 三閘判定：靠近椅（`near`）＋冷卻到期（`cooldown <= 0`）＋過機率門檻（`roll < chance`）
/// → 這一 tick 坐下歇腳。純函式，好窮舉測邊界。
pub fn should_rest(near: bool, cooldown: f32, roll: f32, chance: f32) -> bool {
    near && cooldown <= 0.0 && roll < chance
}

/// 歇腳泡泡台詞（通用、不點名）——五句輪替，字數短不破泡泡框。`pick` 由呼叫端用
/// 座標 bits 合成，讓每次挑到的句子自然分散。
pub fn rest_bubble(pick: usize) -> &'static str {
    const LINES: [&str; 5] = [
        "走累了，坐下來歇口氣。",
        "呼……這張長椅坐著真舒服。",
        "在這兒歇會兒，看看天也好。",
        "有張椅子能坐，日子真愜意。",
        "腳痠了，坐一會兒再走吧。",
    ];
    LINES[pick % LINES.len()]
}

/// 你也坐在旁邊時的歇腳泡泡（點名玩家，更親近）——四句輪替，玩家名截斷不破泡泡框。
pub fn rest_bubble_with_player(player: &str, pick: usize) -> String {
    let name = clip_name(player);
    const TEMPLATES: [&str; 4] = [
        "{name}，一起坐下歇會兒吧。",
        "有{name}一起坐著，歇得真安穩。",
        "{name}，你也走累啦？坐這兒歇歇。",
        "跟{name}並肩坐著歇腳，真好。",
    ];
    TEMPLATES[pick % TEMPLATES.len()].replace("{name}", &name)
}

/// 昇華成一筆「和你一起坐著歇腳」的記憶（點名玩家、不含換行，走既有 append-only 記憶管線）。
pub fn rest_memory_line(player: &str) -> String {
    format!("白天走累了，和{}一起坐在長椅上歇了會兒腳。", clip_name(player)).replace('\n', " ")
}

/// 動態牆播報（非同步層，訪客回來能讀到誰在長椅上歇過腳）。
pub fn rest_feed_line(rname: &str) -> String {
    format!("{rname}在長椅上坐著歇了會兒腳，愜意得很。")
}

/// 掃描整個 world delta，找出所有仍是長椅的方塊座標（啟動時重建歇腳清單用）。
///
/// 純函式（吃 delta overlay 的當前值）：只認 delta 裡目前值 == 長椅的格；被破壞成空氣的
/// 舊長椅格值已是 Air，自然不會被撈出來。反解 chunk 局部索引 → 世界座標，與 `local_index`
/// 的行主序（`lx + lz*CHUNK + ly*CHUNK*CHUNK`）對齊。
pub fn scan_benches(world: &voxel::WorldDelta) -> Vec<(i32, i32, i32)> {
    let c = voxel::CHUNK;
    let mut out = Vec::new();
    for (coord, cd) in world.iter() {
        for (&li, &b) in cd.iter() {
            if b as u8 == BENCH_ID {
                let li = li as i32;
                let lx = li % c;
                let lz = (li / c) % c;
                let ly = li / (c * c);
                out.push((coord.cx * c + lx, coord.cy * c + ly, coord.cz * c + lz));
            }
        }
    }
    out
}

/// 玩家名截斷到 8 字（中文安全，避免超長名撐破泡泡框）。
fn clip_name(name: &str) -> String {
    name.chars().take(8).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voxel::{self, Block};

    #[test]
    fn nearest_picks_closest_within_radius() {
        // (10,0,10) 較遠、(3,0,2) 較近；居民在 (2,2) 附近 → 挑第二張（索引 1）。
        let spots = [(10, 64, 10), (3, 64, 2)];
        assert_eq!(nearest_bench(&spots, 2.0, 2.0, REST_RADIUS), Some(1));
    }

    #[test]
    fn nearest_none_when_all_out_of_radius() {
        let spots = [(50, 64, 50)];
        assert_eq!(nearest_bench(&spots, 0.0, 0.0, REST_RADIUS), None);
        // 空清單也回 None（沒任何長椅）。
        assert_eq!(nearest_bench(&[], 0.0, 0.0, REST_RADIUS), None);
    }

    #[test]
    fn nearest_same_dist_takes_smallest_index() {
        // 兩張對 rx=0.5 等距 → 取索引最小者（索引 0）。
        let sym = [(0, 64, 0), (1, 64, 0)];
        assert_eq!(nearest_bench(&sym, 0.5, 0.5, REST_RADIUS), Some(0));
    }

    #[test]
    fn should_rest_needs_all_three_gates() {
        // 三閘齊備才觸發。
        assert!(should_rest(true, 0.0, 0.1, REST_CHANCE));
        // 不在椅邊 → 否。
        assert!(!should_rest(false, 0.0, 0.1, REST_CHANCE));
        // 冷卻未到 → 否。
        assert!(!should_rest(true, 5.0, 0.1, REST_CHANCE));
        // 骰過門檻 → 否；邊界 roll == chance 不觸發（嚴格小於）。
        assert!(!should_rest(true, 0.0, REST_CHANCE, REST_CHANCE));
        assert!(!should_rest(true, 0.0, 0.99, REST_CHANCE));
    }

    #[test]
    fn bubbles_rotate_and_stay_in_frame() {
        // 通用歇腳語輪替、非空。
        for p in 0..10 {
            assert!(!rest_bubble(p).is_empty());
        }
        assert_ne!(rest_bubble(0), rest_bubble(1));
        // 點名版含玩家名、輪替、超長名截斷不破框。
        let s = rest_bubble_with_player("旅人", 0);
        assert!(s.contains("旅人"));
        let long = rest_bubble_with_player("超級無敵長長長長長長長名字", 2);
        assert!(long.chars().count() < 40, "超長名應被截斷不破泡泡框：{long}");
    }

    #[test]
    fn memory_and_feed_embed_names_no_newline() {
        let m = rest_memory_line("諾娃\n注入");
        assert!(m.contains("諾娃"));
        assert!(!m.contains('\n'), "記憶不得含換行：{m}");
        let f = rest_feed_line("露娜");
        assert!(f.contains("露娜"));
    }

    #[test]
    fn scan_finds_only_benches() {
        // 在 delta 裡放兩張長椅與一塊石頭 → scan 只撈出兩張長椅的座標。
        let mut world = voxel::WorldDelta::new();
        voxel::set_block(&mut world, 5, 64, 7, Block::Bench);
        voxel::set_block(&mut world, -3, 30, 12, Block::Bench);
        voxel::set_block(&mut world, 1, 1, 1, Block::Stone);
        let mut found = scan_benches(&world);
        found.sort();
        let mut want = vec![(5, 64, 7), (-3, 30, 12)];
        want.sort();
        assert_eq!(found, want);
    }

    #[test]
    fn scan_skips_broken_bench() {
        // 放一張長椅再破壞成空氣（delta 覆蓋 Air）→ scan 不再撈出它。
        let mut world = voxel::WorldDelta::new();
        voxel::set_block(&mut world, 2, 64, 2, Block::Bench);
        voxel::set_block(&mut world, 2, 64, 2, Block::Air);
        assert!(scan_benches(&world).is_empty());
    }

    #[test]
    fn bench_id_matches_block() {
        // 常數與 Block 列舉一致（防日後改 id 忘了同步）。
        assert_eq!(BENCH_ID, Block::Bench as u8);
    }
}
