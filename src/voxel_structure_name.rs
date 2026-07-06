//! 乙太方界·居民為你的建造作品取名字 v1（voxel-structure-name）。
//!
//! **真缺口**：居民會注意到你親手蓋的東西 v1（773，`voxel_admire`）讓居民第一次「看見」
//! 玩家的建造，停下來讚賞一句、記進心裡——但讚賞台詞永遠只講「你蓋的東西」這種泛稱，
//! 讚賞完，這件作品在世界裡依然沒有名字。真正活著的居民看到一件用心的作品，不會每次
//! 都只講場面話，而會忍不住幫它取個名字、下次路過還認得出「就是這個」。這正是
//! PLAN_ETHERVOX 核心信念「記憶要驅動行為」的又一塊拼圖：**同一個地點被記住的名字，
//! 讓居民對同一件作品的反應第一次會因為「認得它」而不同**（初次命名 vs. 再次造訪）。
//!
//! **純邏輯層**：地點分格（[`cell_key`]）、命名選字（[`pick_name`]）、命名 / 重逢台詞
//! （[`name_announce_line`] / [`named_revisit_line`]）、帶名記憶摘要（[`admire_memory_line_named`]）
//! 全是確定性純函式，零 LLM、零鎖、零 IO。命名結果的儲存 / 廣播 / 記憶寫入在 `voxel_ws.rs`。
//!
//! **成本 / 護欄**：命名純記憶體（`HashMap<(i32,i32), String>`，重啟歸零，比照既有 v1 慣例，
//! 世界裡的建物本身不受影響，只是「名字」這層裝飾記憶消失，可再次被讚賞重新命名）；
//! 台詞全為固定模板、永不回放玩家原話（無注入 / NSFW 風險）；沿用 773 既有的連段門檻 +
//! 冷卻節流，本模組不新增觸發面，純粹豐富既有讚賞那一刻的內容。

/// 地點分格邊長（世界方塊座標）：同一件作品附近的讚賞落在同一格，才能重複認出「就是這個」。
/// 略大於 [`crate::voxel_admire::STREAK_NEAR_DIST`]（8.0），讓一整面牆/一座小屋落在同一格內。
pub const CELL_SIZE: f32 = 12.0;

/// 把世界座標換成分格鍵（純函式、確定性）：對 [`CELL_SIZE`] 取底除，一格代表一件「作品」。
pub fn cell_key(x: f32, z: f32) -> (i32, i32) {
    ((x / CELL_SIZE).floor() as i32, (z / CELL_SIZE).floor() as i32)
}

/// 居民會取的作品名字候選池（繁中、面向玩家字串集中於此，i18n 友善）。
/// 刻意挑「兩三個字、帶點詩意但不浮誇」的詞——像鄰居隨口取的暱稱，不是官方地名。
const NAME_POOL: &[&str] = &[
    "風車丘", "望遠橋", "月台", "拾光閣", "晨露居", "星語塔",
    "小憩坊", "暖爐屋", "遠眺台", "青苔牆", "回聲廊", "微光庭",
    "織夢屋", "曦光台", "靜心閣", "拾穗居", "望星橋", "半山亭",
];

/// 依 `pick` 從候選池取一個名字（確定性、越界安全取模不 panic）。
pub fn pick_name(pick: usize) -> &'static str {
    NAME_POOL[pick % NAME_POOL.len()]
}

/// 居民第一次幫這件作品取名的台詞（含玩家名 + 新名字，輪替、確定性）。
pub fn name_announce_line(player_name: &str, structure_name: &str, pick: usize) -> String {
    const LINES: &[&str] = &[
        "{p}，我想幫它取個名字——就叫「{n}」吧！",
        "這麼用心的作品該有個名字，{p}，我幫它取叫「{n}」。",
        "以後我就叫這裡「{n}」了，{p}，你不介意吧？",
        "「{n}」，嗯，我覺得這個名字很配它，{p}。",
    ];
    LINES[pick % LINES.len()]
        .replace("{p}", player_name)
        .replace("{n}", structure_name)
}

/// 居民再次路過已命名作品時的台詞（不再是泛泛讚賞，而是喚出名字，證明「記得」）。
pub fn named_revisit_line(player_name: &str, structure_name: &str, pick: usize) -> String {
    const LINES: &[&str] = &[
        "「{n}」還是這麼好看呢，{p}。",
        "每次經過「{n}」我都會多看兩眼，{p}。",
        "{p}，「{n}」在我心裡一直有個位置。",
        "又見到「{n}」了，還是老樣子，真好。",
    ];
    LINES[pick % LINES.len()]
        .replace("{p}", player_name)
        .replace("{n}", structure_name)
}

/// 讚賞記憶摘要（純函式）：已命名時把名字寫進記憶，讓「這件作品叫什麼」成為居民記得住的一部分；
/// 未命名（呼叫端第一次尚未決定名字前的備援路徑）則落回原本的泛稱寫法。
/// 刻意避開 [`crate::voxel_memory::classify_importance`] 的目標／偏好／承諾／名字關鍵詞
/// （沿用 773 `admire_memory_line` 設計），讓它停在情節記憶層、只累積好感。
pub fn admire_memory_line_named(player_name: &str, structure_name: Option<&str>) -> String {
    match structure_name {
        Some(n) => format!("我看著{player_name}在附近親手蓋起了「{n}」，真了不起。"),
        None => format!("我看著{player_name}在附近親手蓋起了一片方塊，真了不起。"),
    }
}

/// 命名事件的城鎮動態摘要（純函式）。
pub fn name_feed_line(resident_name: &str, player_name: &str, structure_name: &str) -> String {
    format!("{resident_name}幫{player_name}蓋的作品取名「{structure_name}」")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_key_groups_nearby_points() {
        // 同一件作品的兩塊（都落在同一個 CELL_SIZE 分格內）落在同一格。
        assert_eq!(cell_key(1.0, 1.0), cell_key(10.0, 10.0));
    }

    #[test]
    fn cell_key_separates_far_points() {
        // 相距超過一格 → 不同格，才不會把兩件不相干的作品混成一件。
        assert_ne!(cell_key(0.0, 0.0), cell_key(100.0, 100.0));
    }

    #[test]
    fn cell_key_handles_negative_coords() {
        // 世界座標可能是負值（地圖中心外），floor 除法要正確分格不 panic。
        assert_eq!(cell_key(-1.0, -1.0), cell_key(-5.0, -5.0));
        assert_ne!(cell_key(-1.0, -1.0), cell_key(1.0, 1.0));
    }

    #[test]
    fn pick_name_is_deterministic_and_in_pool() {
        let a = pick_name(3);
        let b = pick_name(3);
        assert_eq!(a, b);
        assert!(NAME_POOL.contains(&a));
    }

    #[test]
    fn pick_name_overflow_safe() {
        // 超大 pick（如來自時間戳）安全取模，不 panic。
        let _ = pick_name(usize::MAX);
    }

    #[test]
    fn name_pool_entries_are_distinct_and_nonempty() {
        use std::collections::HashSet;
        let set: HashSet<&&str> = NAME_POOL.iter().collect();
        assert_eq!(set.len(), NAME_POOL.len(), "候選名字不應有重複");
        assert!(NAME_POOL.iter().all(|s| !s.is_empty()));
    }

    #[test]
    fn announce_line_carries_player_and_name() {
        let s = name_announce_line("旅人", "風車丘", 0);
        assert!(s.contains("旅人"));
        assert!(s.contains("風車丘"));
        assert!(!s.contains("{p}") && !s.contains("{n}"));
    }

    #[test]
    fn revisit_line_carries_player_and_name() {
        let s = named_revisit_line("旅人", "風車丘", 1);
        assert!(s.contains("旅人"));
        assert!(s.contains("風車丘"));
    }

    #[test]
    fn announce_and_revisit_lines_vary_by_pick() {
        let a = name_announce_line("旅人", "風車丘", 0);
        let b = name_announce_line("旅人", "風車丘", 1);
        assert_ne!(a, b);
        let c = named_revisit_line("旅人", "風車丘", 0);
        let d = named_revisit_line("旅人", "風車丘", 1);
        assert_ne!(c, d);
    }

    #[test]
    fn memory_line_named_variant_contains_name() {
        let s = admire_memory_line_named("旅人", Some("風車丘"));
        assert!(s.contains("風車丘"));
        assert!(s.contains("旅人"));
    }

    #[test]
    fn memory_line_falls_back_without_name() {
        let s = admire_memory_line_named("旅人", None);
        assert!(!s.contains('「'));
        assert!(s.contains("旅人"));
    }

    #[test]
    fn memory_line_stays_episodic() {
        // 記憶摘要不得誤觸重要性關鍵詞（否則會被升級成語意精華、亂佔上限），
        // 沿用 773 admire_memory_line 的既有斷言。
        let named = admire_memory_line_named("建築師", Some("風車丘"));
        assert!(matches!(
            crate::voxel_memory::classify_importance(&named),
            crate::voxel_memory::Importance::Ephemeral
        ));
    }

    #[test]
    fn feed_line_contains_all_three_names() {
        let s = name_feed_line("露娜", "旅人", "風車丘");
        assert!(s.contains("露娜") && s.contains("旅人") && s.contains("風車丘"));
    }
}
