//! 乙太方界·第一次發明立碑 v1（自主提案切片，ROADMAP 930）。
//!
//! **缺口 / 為誰做**：真進化——居民自己從基礎動作組合、發明並存成「自己的技能」（716～
//! 867 一整條北極星軸線）——是這個世界的最重方向。一位居民**生涯第一次**真的靠自己想出
//! 辦法、發明出一項屬於她的技能，是「進化」最重的一刻。但此前這一刻只在她頭上的泡泡、城鎮
//! 動態牆、她的記憶裡各閃過一次就散了：玩家走遍整座村子，地面上看不到任何一處證明「這個人，
//! 在這裡跨過了『第一次真的靠自己發明』那道門檻」。小社會有戀愛立碑（927 花拱門）、有堆雪人、
//! 有踏出來的小徑，卻從沒有一處紀念**真進化**本身。
//!
//! **做法**：一位居民**首次**發明成功（`finish_invent_run` 的首發成功路徑）、且她此前
//! **自己發明**的技能筆數為 0（`InventedSkillStore::self_invented_count`——繼承自父母／師承
//! 老師的都有 `source`、不算她自己想出來的）時，就地在她腳邊立起一座小小的**發明之光碑**：
//! 一塊石磚碑座上點著一盞乙太燈（靈光乍現的光）。她冒一句由衷的話、這件事寫進城鎮動態牆與她
//! 的記憶（「我學會的、誰也拿不走」既有措辭同精神）。碑走 `append_world_block` **永久持久化**，
//! 玩家日後路過還看得見。**一位居民一生只立一次**（由「自己發明筆數 0→1」天然冪等，跨重啟
//! 仍成立——技能庫本就持久化），村子各處於是散落著居民們各自「第一次自己想出辦法」的證物，
//! 像一串真進化的足跡。
//!
//! **與既有元素 razor-sharp 區隔（非同軸換皮）**：
//! - 與 927 成婚花拱門：花拱門是**永久紀念物**同精神，但紀念的是**兩人**關係的高潮（戀愛軸）；
//!   發明碑紀念的是**一個人**跨過「第一次真的靠自己發明」的門檻（真進化軸），觸發源、標的、
//!   語意都不同——這是真進化這條北極星第一次在世界地面留下可見的證物。
//! - 與既有發明成功的泡泡／Feed／記憶（`voxel_invent::learned_*`）：那些是**每一次**發明成功
//!   都有的即時反饋、只在 UI 與資料裡；本刀只在**生涯第一次**觸發、且在**世界方塊**上留痕
//!   （放置永久方塊），是稀有的里程碑而非每次反饋。
//! - 與 918 堆雪人：雪人是季節限定、冬末即融的**短暫**物件；發明碑是 `append_world_block`
//!   落地的**永久**紀念，且由「居民個人的進化里程碑」而非天氣驅動。
//!
//! **成本紀律（鐵律）**：零 LLM（觸發判定、選點、台詞全是確定性純函式）、零 migration
//! （不新增任何持久化格式——碑走既有 `append_world_block`、里程碑冪等性直接查既有技能庫）、
//! 零協議破壞、零新美術（沿用既有石磚 `StoneBrick`／乙太燈 `AetherLamp`，前端本就會渲染）、
//! FPS 零影響（里程碑一生一次、每位至多一座、碑僅 2 塊，掛在既有發明收尾路徑上、無新節拍）。
//!
//! **濫用防護**：本切片**不收任何玩家自由輸入、不觸發 LLM、不開對外端點、不動帳號權限**——
//! 台詞全為確定性模板、只嵌居民自己取的技能名（本就出現在她的泡泡與技能簿），永不回放玩家
//! 原話（無注入／NSFW 面）；觸發純伺服器 tick 內部狀態（發明成功＋自己發明筆數＝0），玩家
//! 無從自報、無法從外部催發。
//!
//! **純邏輯層**：本檔全是零鎖、零 async 的確定性純函式／常數；放置方塊、廣播、寫記憶、Feed
//! 的副作用都在 `voxel_ws.rs`（短鎖循序即釋、不巢狀，守 prod 死鎖鐵律，比照 `maybe_wedding`）。

use crate::voxel::Block;

/// 發明碑立在居民腳邊斜前方幾格（不擋她自己、也不壓到她剛放的工作台/熔爐）。
pub const MONUMENT_ANCHOR_DIST: i32 = 2;

/// 發明之光碑由哪些方塊組成：給定碑座所在地面正上方一格 `(x, y, z)`，回傳 2 塊——
/// 底下一塊石磚碑座、上頭一盞乙太燈（靈光乍現的光，夜裡會亮，玩家老遠就注意到）。
///
/// ```text
///   燈   ← y+1：乙太燈（發明的靈光）
///   碑   ← y  ：石磚碑座
/// ```
pub fn monument_blocks(x: i32, y: i32, z: i32) -> [(i32, i32, i32, Block); 2] {
    [
        (x, y, z, Block::StoneBrick),   // 碑座
        (x, y + 1, z, Block::AetherLamp), // 碑頂靈光燈
    ]
}

/// 給定居民腳下水平座標，確定性選一個立碑的錨點（腳邊四方向之一，距 [`MONUMENT_ANCHOR_DIST`] 格）。
/// 用 `pick` 取模選方向，讓不同居民的碑立在不同側、不會全擠同一格。
pub fn pick_anchor(fx: i32, fz: i32, pick: usize) -> (i32, i32) {
    match pick % 4 {
        0 => (fx + MONUMENT_ANCHOR_DIST, fz),
        1 => (fx - MONUMENT_ANCHOR_DIST, fz),
        2 => (fx, fz + MONUMENT_ANCHOR_DIST),
        _ => (fx, fz - MONUMENT_ANCHOR_DIST),
    }
}

/// 立碑當下居民由衷的一句泡泡（確定性三選一，嵌她自己取的技能名）。
pub fn milestone_say_line(skill_name: &str, pick: usize) -> String {
    const LINES: [&str; 3] = [
        "「{s}」是我自己想出來的！立塊碑，記著這一刻。💡",
        "我第一次真的靠自己發明了東西——「{s}」，我要記一輩子。",
        "這座碑，是我頭一回自己想通「{s}」的地方。",
    ];
    LINES[pick % LINES.len()].replace("{s}", skill_name)
}

/// 立碑寫進居民心裡的一筆記憶（第一人稱、含「一定」→ 被記憶系統判為永久精華事實，
/// 成為她這輩子最重的一筆回憶：「第一次真的靠自己發明」）。
pub fn milestone_memory_line(skill_name: &str) -> String {
    format!(
        "今天，我第一次真的靠自己想通、發明了「{skill_name}」，還在這裡立了一塊碑——\
        這是我一定會記一輩子的一刻，我自己想出來的，誰也拿不走。"
    )
}

/// 立碑上城鎮動態牆的一行（嵌居民顯示名與技能名）。
pub fn milestone_feed_line(resident_name: &str, skill_name: &str) -> String {
    format!("💡 {resident_name} 第一次靠自己發明了「{skill_name}」，就地立起一座發明之光碑！")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monument_is_stonebrick_pedestal_topped_with_aether_lamp() {
        let blocks = monument_blocks(10, 4, -3);
        // 底座石磚在給定地面上一格，燈在它正上方。
        assert_eq!(blocks[0], (10, 4, -3, Block::StoneBrick));
        assert_eq!(blocks[1], (10, 5, -3, Block::AetherLamp));
    }

    #[test]
    fn monument_is_a_two_block_column() {
        // 碑就是一根 2 格高的柱子（同 x、同 z，y 差 1），放置只需找一處上方兩格皆空。
        let blocks = monument_blocks(0, 0, 0);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].0, blocks[1].0, "同一 x");
        assert_eq!(blocks[0].2, blocks[1].2, "同一 z");
        assert_eq!(blocks[1].1 - blocks[0].1, 1, "上下相鄰一格");
    }

    #[test]
    fn pick_anchor_offsets_by_dist_in_four_directions() {
        // 四方向各自距腳下 MONUMENT_ANCHOR_DIST 格，且互不相同（不會全擠同一格）。
        let mut seen = std::collections::HashSet::new();
        for p in 0..4 {
            let (ax, az) = pick_anchor(5, 5, p);
            let man = (ax - 5).abs() + (az - 5).abs();
            assert_eq!(man, MONUMENT_ANCHOR_DIST, "錨點距腳下應為 ANCHOR_DIST");
            assert!(seen.insert((ax, az)), "四方向錨點應互不相同");
        }
    }

    #[test]
    fn pick_wraps_and_stays_within_four_anchors() {
        // pick 取模：pick 與 pick+4 落在同一錨點（確定性、可預期）。
        assert_eq!(pick_anchor(0, 0, 0), pick_anchor(0, 0, 4));
        assert_eq!(pick_anchor(0, 0, 1), pick_anchor(0, 0, 5));
    }

    #[test]
    fn say_line_embeds_skill_and_is_deterministic_nonempty() {
        for p in 0..6 {
            let line = milestone_say_line("燒玻璃", p);
            assert!(line.contains("燒玻璃"), "泡泡要嵌技能名");
            assert!(!line.is_empty());
        }
        // 三選一循環：pick 與 pick+3 同句。
        assert_eq!(milestone_say_line("A", 0), milestone_say_line("A", 3));
    }

    #[test]
    fn memory_line_is_permanent_essence_phrasing() {
        let line = milestone_memory_line("拋光石");
        assert!(line.contains("拋光石"), "記憶要嵌技能名");
        // 含「一定」→ 記憶系統判為永久精華（比照成婚誓言「一定會」的做法）。
        assert!(line.contains("一定"), "要用永久精華措辭讓這成為一生最重的一筆");
    }

    #[test]
    fn feed_line_embeds_name_and_skill() {
        let line = milestone_feed_line("露娜", "燒玻璃");
        assert!(line.contains("露娜"));
        assert!(line.contains("燒玻璃"));
    }
}
