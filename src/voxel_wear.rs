//! 乙太方界·染色頭巾 v1（ROADMAP 1023，自主提案切片）——世界第一件穿戴外觀。
//!
//! 世界近 200 刀以來，居民有詳盡的家/建物/稱號當視覺身分，玩家與居民的方塊小人卻從
//! 出生到終老連一絲外觀都沒變過——命名（petname）/稱謂（epithet）/名牌（nameplate）
//! 全是**文字層**，沒有任何系統動過「看起來長什麼樣」。本刀補上：玩家合成頭巾戴上，
//! 其他玩家立刻看得見（`voxel_ws::VoxelPlayer.hat`）；把頭巾送給居民，她會一直戴著
//! （`voxel_ws::VoxelResident.hat`）——身分第一次不只是文字，而是看得見的外觀。
//!
//! 純邏輯層（道謝/動態牆台詞，字元截斷防破框），零鎖/零 IO/零 async；頭巾物品 id、
//! 合成配方、`is_hat`/`can_wear_hat` 判定都留在 `voxel_craft.rs`（與木筏/獨輪車等其他
//! 穿戴/代步物品同一套慣例）。

const SAY_MAX_CHARS: usize = 50;

fn truncate(s: String) -> String {
    s.chars().take(SAY_MAX_CHARS).collect()
}

/// 送頭巾給居民時的道謝泡泡（比一般贈禮更雀躍——這是「你送的頭巾」第一次真的戴上她的頭）。
/// `pick` 由呼叫端輪替，確定性。
pub fn hat_gift_thanks_line(item_name: &str, pick: usize) -> String {
    let pool = [
        format!("{item_name}？我最喜歡了，這就戴上！好看嗎？"),
        format!("謝謝你的{item_name}，我要天天戴著它。"),
        format!("哇，{item_name}！戴上的感覺真好，你自己也弄一頂吧。"),
    ];
    truncate(pool[pick % pool.len()].clone())
}

/// 動態牆：某居民戴上了玩家送的頭巾。
pub fn hat_gift_feed_line(resident_name: &str, player_name: &str, item_name: &str) -> String {
    format!("{resident_name}戴上了{player_name}送的{item_name}，看起來煥然一新。")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hat_gift_thanks_line_rotates_and_contains_item_name() {
        let seen: std::collections::HashSet<String> =
            (0..6).map(|pick| hat_gift_thanks_line("紅頭巾", pick)).collect();
        assert!(seen.len() >= 2, "應有多種輪替台詞，而非永遠同一句");
        for line in &seen {
            assert!(line.contains("紅頭巾"), "台詞應提到頭巾名稱：{line}");
            assert!(!line.is_empty());
        }
    }

    #[test]
    fn hat_gift_thanks_line_truncates_long_item_name_safely() {
        // 超長字串也不該 panic（多位元組中文邊界安全）。
        let long_name: String = "巾".repeat(200);
        let line = hat_gift_thanks_line(&long_name, 0);
        assert!(line.chars().count() <= SAY_MAX_CHARS);
    }

    #[test]
    fn hat_gift_feed_line_embeds_all_three_names_single_line() {
        let line = hat_gift_feed_line("露娜", "旅人", "藍頭巾");
        assert!(line.contains("露娜"));
        assert!(line.contains("旅人"));
        assert!(line.contains("藍頭巾"));
        assert!(!line.contains('\n'), "動態牆條目應為單行，避免排版跑掉");
    }
}
