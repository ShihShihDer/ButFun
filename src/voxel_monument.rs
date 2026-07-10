//! 乙太方界·村莊中央紀念柱（村碑）v1（自主提案切片，ROADMAP 885）。
//!
//! **真缺口**：`voxel_village_milestone.rs`（856）讓村莊有了「集體里程碑」——累計被居民
//! 記住的地標數跨過 3／8／15 座門檻時，全村居民一起冒歡呼泡泡、動態牆播一則全村公告。
//! 但那份集體成就至今**只停留在文字與泡泡**：村子明明「長大」成了小小聚落→村莊→市鎮，
//! 放眼望去世界卻沒有任何一處看得出這件事。村莊的成長第一次要有**實體的證據**。
//!
//! **本刀**：村莊每達成一個集體里程碑，居民就合力在**村莊中心廣場**豎起（或再拔高一段）
//! 一根**中央紀念柱**——每階往上長一段（兩塊石磚＋頂端一圈乙太燈），三階達成後成為
//! 一根九格高、亮著三圈燈火的地標。玩家探索回到村子，一眼就看見「村碑又長高了」，
//! 集體成就從此是眼睛看得見、夜裡發著光的實體。
//!
//! **設計**：本模組純邏輯、確定性、零 IO／零鎖／零 LLM——吃「村莊中心座標＋地表高度＋
//! 第幾階」，吐出該階要新增的方塊清單。放置／廣播／持久化都在 `voxel_ws.rs`（比照
//! 殖民奠基 884 的黃金安全模式：`surface_y` 鎖外算 → deltas 寫鎖批次即釋 → 鎖外廣播＋
//! append-only 落地）。
//!
//! **資料安全**：紀念柱一律蓋在**廣場地面正上方的空氣格**、逐格往上疊，呼叫端只在
//! `cur == Air` 時落子——**絕不覆蓋任何既有方塊**（居民的作品、廣場鋪面、火把皆不動）；
//! 冪等（已是目標方塊即跳過），重跑安全。

use crate::voxel::Block;

/// 每達成一個里程碑，紀念柱往上長的高度：兩塊石磚柱身 + 頂端一圈乙太燈。
pub const SEGMENT_HEIGHT: i32 = 3;

/// 產生「第 `tier_idx` 階」紀念柱該**新增**的方塊清單（絕對世界座標）。
///
/// - `cx, cz`：村莊中心（廣場中心）。
/// - `surface_y`：該格「地面正上方」的 y（`voxel_building::surface_y` 語意，即第一格空氣）。
/// - `tier_idx`：0＝小小聚落、1＝像模像樣的村莊、2＝熱鬧的市鎮（來自 `voxel_village_milestone::TIERS` 的序位）。
///
/// 每一段佔 `SEGMENT_HEIGHT` 格：底下兩格石磚柱身、最上一格乙太燈。各段接續往上疊，
/// 彼此不重疊——低階已放的方塊，高階只會蓋在它更上方，故呼叫端永遠只需 air-only 落子。
pub fn monument_cells(cx: i32, cz: i32, surface_y: i32, tier_idx: usize) -> Vec<(i32, i32, i32, Block)> {
    let base = surface_y + (tier_idx as i32) * SEGMENT_HEIGHT;
    vec![
        (cx, base, cz, Block::StoneBrick),
        (cx, base + 1, cz, Block::StoneBrick),
        (cx, base + 2, cz, Block::AetherLamp),
    ]
}

/// 達成第 `tier_idx` 階（0-based）後，紀念柱從廣場地面算起的總高（格）。
pub fn total_height(tier_idx: usize) -> i32 {
    (tier_idx as i32 + 1) * SEGMENT_HEIGHT
}

/// 全村動態牆播報句：居民合力把村碑拔高了。`tier_name_zh` 為此階村莊晉升成的稱號。
pub fn monument_feed_line(tier_name_zh: &str, height: i32) -> String {
    format!("🗿 居民合力在村莊中央立起村碑，為晉升「{tier_name_zh}」誌記——如今高達 {height} 格，燈火照亮整座廣場。")
}

/// 參與立碑的居民寫進記憶的一句（episodic，第一人稱內心，累積「和大家一起打造家園」的印象）。
/// 不含任何玩家名／私密渴望，適用於任何一位在場居民。
pub fn monument_memory_line(height: i32) -> String {
    format!("今天和大家一起，把村子中央的村碑又立高了一截，現在有 {height} 格那麼高了。看著它我覺得，這裡真的是我們的家。")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_tier_starts_at_surface_and_is_three_tall() {
        let cells = monument_cells(0, 0, 20, 0);
        assert_eq!(cells.len(), 3);
        // 底部兩格石磚、頂端乙太燈。
        assert_eq!(cells[0], (0, 20, 0, Block::StoneBrick));
        assert_eq!(cells[1], (0, 21, 0, Block::StoneBrick));
        assert_eq!(cells[2], (0, 22, 0, Block::AetherLamp));
    }

    #[test]
    fn later_tiers_stack_strictly_above_earlier_without_overlap() {
        let mut seen = std::collections::HashSet::new();
        let mut max_y_prev = i32::MIN;
        for tier in 0..3 {
            let cells = monument_cells(5, -3, 20, tier);
            let min_y = cells.iter().map(|c| c.1).min().unwrap();
            // 這一階最低格，必須嚴格高於上一階最高格（不重疊、不倒插）。
            assert!(min_y > max_y_prev, "第 {tier} 階與前一階重疊了");
            for c in &cells {
                assert!(seen.insert((c.0, c.1, c.2)), "座標 {:?} 被重複佔用", (c.0, c.1, c.2));
                // 全部都在同一根柱子上（x/z 對齊村莊中心）。
                assert_eq!((c.0, c.2), (5, -3));
            }
            max_y_prev = cells.iter().map(|c| c.1).max().unwrap();
        }
    }

    #[test]
    fn each_tier_is_capped_by_a_lamp() {
        for tier in 0..3 {
            let cells = monument_cells(0, 0, 10, tier);
            let top = cells.iter().max_by_key(|c| c.1).unwrap();
            assert_eq!(top.3, Block::AetherLamp, "第 {tier} 階頂端應是乙太燈");
        }
    }

    #[test]
    fn total_height_grows_by_segment_each_tier() {
        assert_eq!(total_height(0), 3);
        assert_eq!(total_height(1), 6);
        assert_eq!(total_height(2), 9);
    }

    #[test]
    fn feed_line_embeds_tier_name_and_height() {
        let line = monument_feed_line("熱鬧的市鎮", 9);
        assert!(line.contains("熱鬧的市鎮"));
        assert!(line.contains('9'));
        assert!(!line.contains('\n'));
        assert!(!line.is_empty());
    }

    #[test]
    fn memory_line_is_single_line_nonempty_and_has_no_player_leak() {
        let line = monument_memory_line(6);
        assert!(!line.is_empty());
        assert!(!line.contains('\n'));
        assert!(line.contains('6'));
        // episodic 內心句，不該外洩玩家名占位符之類。
        assert!(!line.contains('{'));
    }
}
