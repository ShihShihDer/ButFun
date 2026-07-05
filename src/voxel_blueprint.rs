//! 乙太方界·建築藍圖 v1（自主提案切片，PLAN_ETHERVOX「你的互動有後果」核心信念）。
//!
//! **真缺口**：居民想蓋什麼，至今全靠「猜」——`voxel_building::classify_desire` 從她自己
//! 隨口許的願望文字（或你聊天時提到的話）裡挑關鍵詞猜建物種類，猜不猜得中全看運氣：
//! 你想看她蓋一座涼亭，只能等她剛好許到「好想有個乘涼避雨的地方」這類願望，或反覆聊天
//! 碰運氣。玩家從沒有一種「直接指定」的辦法——建築創作弧（822~824）讓每間房子的樣式
//! 因居民而異，卻從沒讓玩家自己選過「這位居民蓋哪一種」。
//!
//! 本模組補上這條缺口：合成一張建築藍圖（五種、各對應一種既有建物：小屋／水井／瞭望台／
//! 花圃／涼亭），送給居民後，她的心願被**直接改寫**成那座建物——下次她挑建造活動時，
//! 蓋的必然是你指定的那一種，完工時也會指名感謝你（沿用既有 `BuildPlan.inspired_by`）。
//!
//! **這不是新的建造機制**：藍圖只是換一種「種下心願」的方式，沿用玩家聊天種願望的同一套
//! `DesireStore::set_desire`（`sparked_by`=玩家名）／`classify_desire` 機制——零新狀態機、
//! 零新鎖、零協議破壞、零 migration。`voxel_building`／`voxel_skills::choose_activity` 完全
//! 不用改一行：她的心願文字變了，既有邏輯自然而然蓋出對應的建物（哪怕這種類早蓋過，也會
//! 走既有的擴建路徑，行為與玩家聊天種下的心願完全一致）。
//!
//! **零新方塊種類、零 RNG 掉落**：藍圖是純物品（不可放置），走既有工作台合成路徑取得
//! （比照箱子/雞舍等既有做法），刻意不做「挖礦機率掉落」——守既有「新資源走合成，不開新
//! 掉落 RNG」的慣例（見莓果叢 806 的設計取捨）。
//!
//! **純邏輯層**：`blueprint_kind`／`blueprint_item_id`／`blueprint_desire_text`／台詞皆為
//! 確定性純函式，零 LLM、零鎖、零 IO，可單元測試。鎖／`set_desire`／持久化全在
//! `voxel_ws.rs`（沿用玩家聊天種願望的既有短鎖手法，守死鎖鐵律）。

use crate::voxel_building::BuildKind;

/// 五張藍圖的物品 id（純物品，不可放置；接續空玻璃瓶 83 之後的空號）。
pub const BLUEPRINT_HOUSE: u8 = 84;
pub const BLUEPRINT_WELL: u8 = 85;
pub const BLUEPRINT_TOWER: u8 = 86;
pub const BLUEPRINT_GARDEN: u8 = 87;
pub const BLUEPRINT_PAVILION: u8 = 88;

/// 由物品 id 查出它指定的建物種類（非藍圖 id 回 `None`）——供 `voxel_ws.rs` 判斷這份禮
/// 是不是一張藍圖。
pub fn blueprint_kind(item_id: u8) -> Option<BuildKind> {
    match item_id {
        BLUEPRINT_HOUSE => Some(BuildKind::House),
        BLUEPRINT_WELL => Some(BuildKind::Well),
        BLUEPRINT_TOWER => Some(BuildKind::Tower),
        BLUEPRINT_GARDEN => Some(BuildKind::Garden),
        BLUEPRINT_PAVILION => Some(BuildKind::Pavilion),
        _ => None,
    }
}

/// 由建物種類反查它的藍圖物品 id（`blueprint_kind` 的反函式；供合成配方常數對照）。
pub fn blueprint_item_id(kind: BuildKind) -> u8 {
    match kind {
        BuildKind::House => BLUEPRINT_HOUSE,
        BuildKind::Well => BLUEPRINT_WELL,
        BuildKind::Tower => BLUEPRINT_TOWER,
        BuildKind::Garden => BLUEPRINT_GARDEN,
        BuildKind::Pavilion => BLUEPRINT_PAVILION,
    }
}

/// 這座建物種類的「心願文字」——刻意採用與 `voxel_building::classify_desire` 既有回歸
/// 測試同款的關鍵詞句，保證餵回 `classify_desire` 一定能正確分類回同一種（見下方測試）。
/// 面向玩家字串集中此處（i18n 友善）。
pub fn blueprint_desire_text(kind: BuildKind) -> &'static str {
    match kind {
        BuildKind::House => "我想要一間小屋",
        BuildKind::Well => "我想要一口水井",
        BuildKind::Tower => "我想要一座瞭望台",
        BuildKind::Garden => "我想要一個花圃",
        BuildKind::Pavilion => "我想要一座涼亭",
    }
}

/// 藍圖送到時，居民頭頂冒的道謝泡泡（點名收到的藍圖種類、且明確表態「這就照著蓋」——
/// 與一般贈禮道謝（`voxel_gift::gift_thanks_line`）的區別：那句只謝禮物本身，這句點出
/// 「心願被直接指定」這件事，讓玩家清楚感受到自己的選擇真的算數）。依 `pick` 輪替不機械。
pub fn blueprint_thanks_line(kind: BuildKind, pick: usize) -> String {
    let kn = kind.display_name();
    let lines = [
        format!("一張{kn}的藍圖！謝謝你，我這就照著蓋一座出來～"),
        format!("你想看我蓋{kn}呀？這藍圖我收下了，很快就動工！"),
        format!("{kn}的藍圖……好，就照你說的蓋！"),
    ];
    lines[pick % lines.len()].clone()
}

/// 藍圖生效時（居民心願被改寫）留下的記憶摘要（掛真實玩家名下、累積好感，供日後回想／
/// 日記引用）——記下的是「你指定我蓋什麼，我聽你的」，而非泛泛的贈禮記憶。
pub fn blueprint_memory_line(player_name: &str, kind: BuildKind) -> String {
    format!(
        "{player_name}送了我一張{}的藍圖，指定要我蓋這個——我聽你的。",
        kind.display_name()
    )
}

/// 藍圖生效的動態牆播報（面向玩家、集中可 i18n）：不在場的玩家回來也讀得到
/// 「誰指定了哪位居民蓋什麼」。
pub fn blueprint_feed_line(player_name: &str, resident_name: &str, kind: BuildKind) -> String {
    format!(
        "{player_name}送給{resident_name}一張{}的藍圖，她決定照著蓋",
        kind.display_name()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_KINDS: [BuildKind; 5] = [
        BuildKind::House,
        BuildKind::Well,
        BuildKind::Tower,
        BuildKind::Garden,
        BuildKind::Pavilion,
    ];

    #[test]
    fn blueprint_kind_and_item_id_round_trip() {
        for kind in ALL_KINDS {
            let id = blueprint_item_id(kind);
            assert_eq!(blueprint_kind(id), Some(kind), "{kind:?} 的藍圖 id 應能查回同一種類");
        }
    }

    #[test]
    fn blueprint_item_ids_are_distinct_and_in_range() {
        let mut ids: Vec<u8> = ALL_KINDS.iter().map(|&k| blueprint_item_id(k)).collect();
        let before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), before, "五張藍圖的物品 id 不應重複");
        for id in ids {
            assert!((84..=88).contains(&id), "藍圖 id 應落在 84..=88（接續空玻璃瓶 83 之後）");
        }
    }

    #[test]
    fn unknown_item_id_is_not_a_blueprint() {
        assert_eq!(blueprint_kind(0), None);
        assert_eq!(blueprint_kind(83), None, "空玻璃瓶不是藍圖");
        assert_eq!(blueprint_kind(255), None);
    }

    #[test]
    fn blueprint_desire_text_classifies_back_to_same_kind() {
        // 藍圖的心願文字必須能被 classify_desire 正確分類回同一種——這是本模組能運作的地基：
        // 若文字選得不好被誤分類，藍圖就會蓋出玩家沒指定的建物。
        use crate::voxel_building::classify_desire;
        for kind in ALL_KINDS {
            let text = blueprint_desire_text(kind);
            assert_eq!(
                classify_desire(text),
                Some(kind),
                "藍圖心願文字「{text}」應分類回 {kind:?}"
            );
        }
    }

    #[test]
    fn thanks_lines_nonempty_and_mention_kind_and_rotate() {
        for kind in ALL_KINDS {
            let kn = kind.display_name();
            for pick in 0..3 {
                let line = blueprint_thanks_line(kind, pick);
                assert!(!line.is_empty());
                assert!(line.contains(kn), "道謝泡泡應點名建物種類：{line}");
            }
            // 至少兩種不同措辭（輪替、不永遠同一句）。
            let a = blueprint_thanks_line(kind, 0);
            let b = blueprint_thanks_line(kind, 1);
            let c = blueprint_thanks_line(kind, 2);
            assert!(a != b || b != c);
        }
        // pick 取模不越界。
        let _ = blueprint_thanks_line(BuildKind::Tower, usize::MAX);
    }

    #[test]
    fn memory_and_feed_lines_mention_player_resident_and_kind() {
        for kind in ALL_KINDS {
            let mem = blueprint_memory_line("小美", kind);
            assert!(mem.contains("小美") && mem.contains(kind.display_name()));
            let feed = blueprint_feed_line("小美", "露娜", kind);
            assert!(feed.contains("小美") && feed.contains("露娜") && feed.contains(kind.display_name()));
        }
    }
}
