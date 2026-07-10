//! 乙太方界·居民口耳相傳 v1（ROADMAP 694）——見聞透過拜訪在居民之間流通。
//!
//! 居民已有情誼（陌生→相識→老朋友，`voxel_bonds`）、也已各自累積長期記憶（`voxel_memory`），
//! 但兩件事至今互不相干：好感只是一個數字，記憶只留在各自腦中，居民彼此**不知道**對方經歷過什麼。
//! 本模組讓「老朋友」到訪時，主人把自己**最近一則見聞**轉述給訪客——訪客把這則見聞記進**自己**的
//! 記憶庫（沿用既有 `add_memory`／`append_memory`，零新持久化格式）。從此一位居民做的事、認識的人，
//! 會經由朋友網絡間接傳到另一位居民「知道」——小社會第一次有了消息流動，不只是好感度數字。
//!
//! 純邏輯層：挑見聞來源、防重複轉述、組轉述文字皆為確定性純函式，可單元測試。
//! 鎖／隨機擲骰／持久化觸發全在 `voxel_ws.rs`（讀 host 記憶 → 寫 visitor 記憶，短鎖不巢狀）。

use crate::voxel_bonds::BondTier;
use crate::voxel_memory::MemoryEntry;

/// 轉述文字最多保留原見聞的字元數（含前綴後仍遠低於 `voxel_memory::SUMMARY_MAX_CHARS`，避免爆長）。
const GOSSIP_SNIPPET_MAX_CHARS: usize = 50;

/// 每次「老朋友」到訪時，觸發口耳相傳的機率。只有老朋友夠熟稔才會分享見聞；
/// 陌生／相識還停留在客套問候，沒有到「跟你說個八卦」的交情。
pub fn gossip_chance(tier: BondTier) -> f32 {
    match tier {
        BondTier::Friend => 0.35,
        _ => 0.0,
    }
}

/// 從主人的長期記憶（不限順序，函式內部自行取最新）挑一則可轉述給訪客的見聞。
///
/// 排除規則：
/// - 關於訪客自己的記憶（轉述給訪客聽訪客自己的事沒有意義）。
/// - 摘要為空。
/// - 已經是「轉述」本身（摘要以「聽」開頭）——只轉述第一手見聞，避免八卦鏈無限累加文字、
///   也避免「A 聽 B 說聽 C 說…」的無窮遞迴摘要。
/// - 帶了內部識別前綴（`voxel_diary::is_internal_tagged`，如 `🏘️鄰里`／`🪧讀到告示牌`）——這些前綴
///   只給日記端分類用、不是給玩家看的文字，若被挑中會把原始標記直接組進轉述文字洩漏出去。
///
/// 純函式、確定性：多筆候選時取 `seq` 最大（最新）者。
pub fn pick_gossip<'a>(host_memories: &'a [MemoryEntry], visitor_name: &str) -> Option<&'a MemoryEntry> {
    host_memories
        .iter()
        .filter(|e| {
            e.player != visitor_name
                && !e.summary.is_empty()
                && !e.summary.starts_with('聽')
                && !crate::voxel_diary::is_internal_tagged(&e.summary)
        })
        .max_by_key(|e| e.seq)
}

/// 組轉述文字：「聽{host}說：{見聞片段}」。見聞片段截斷避免單筆記憶爆長。
pub fn format_gossip(host_name: &str, original_summary: &str) -> String {
    let clipped: String = original_summary.chars().take(GOSSIP_SNIPPET_MAX_CHARS).collect();
    format!("聽{host_name}說：{clipped}")
}

/// 訪客是否已經知道這則見聞（比對訪客「聽自 host」的既有記憶，避免同一則見聞重複灌爆記憶庫）。
pub fn already_knows(existing_from_host: &[MemoryEntry], gossip_text: &str) -> bool {
    existing_from_host.iter().any(|e| e.summary == gossip_text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem(resident: &str, player: &str, summary: &str, seq: u64) -> MemoryEntry {
        MemoryEntry {
            resident: resident.to_string(),
            player: player.to_string(),
            summary: summary.to_string(),
            seq,
        }
    }

    #[test]
    fn gossip_chance_only_for_friend_tier() {
        assert_eq!(gossip_chance(BondTier::Stranger), 0.0);
        assert_eq!(gossip_chance(BondTier::Acquaintance), 0.0);
        assert!(gossip_chance(BondTier::Friend) > 0.0);
    }

    #[test]
    fn pick_gossip_empty_returns_none() {
        assert!(pick_gossip(&[], "露娜").is_none());
    }

    #[test]
    fn pick_gossip_skips_memories_about_visitor() {
        let mems = vec![mem("vox_res_0", "露娜", "跟露娜聊過星星", 5)];
        assert!(pick_gossip(&mems, "露娜").is_none());
    }

    #[test]
    fn pick_gossip_skips_empty_summary() {
        let mems = vec![mem("vox_res_0", "玩家甲", "", 5)];
        assert!(pick_gossip(&mems, "露娜").is_none());
    }

    #[test]
    fn pick_gossip_skips_already_relayed_gossip() {
        let mems = vec![mem("vox_res_0", "諾娃", "聽諾娃說：她蓋了一口井", 9)];
        assert!(pick_gossip(&mems, "露娜").is_none());
    }

    #[test]
    fn pick_gossip_skips_internal_tagged_memories() {
        // 鄰里生活記憶帶 NEIGHBORLY_TAG 前綴、讀牌記憶帶 SIGN_MEMORY_TAG 前綴，兩者都不該
        // 被挑去轉述（否則「🏘️鄰里…」原始標記會洩漏進訪客記憶）。挑選端只剩乾淨的舊記憶。
        let mems = vec![
            mem("vox_res_0", "玩家甲", "跟玩家甲聊過採礦", 3),
            mem("vox_res_0", "玩家乙", &crate::voxel_diary::tag_neighborly("去陪伴了露娜"), 8),
            mem(
                "vox_res_0",
                "玩家丙",
                &format!("{}村口那面告示牌", crate::voxel_readsign::SIGN_MEMORY_TAG),
                9,
            ),
        ];
        let picked = pick_gossip(&mems, "露娜").expect("應退回乾淨舊記憶");
        assert_eq!(picked.summary, "跟玩家甲聊過採礦");
    }

    #[test]
    fn pick_gossip_picks_newest_eligible_by_seq() {
        let mems = vec![
            mem("vox_res_0", "玩家甲", "跟玩家甲聊過採礦", 3),
            mem("vox_res_0", "玩家乙", "跟玩家乙聊過種田", 7),
            mem("vox_res_0", "露娜", "跟露娜聊過星星", 10), // 被排除（關於訪客本人）
        ];
        let picked = pick_gossip(&mems, "露娜").expect("應挑到一則");
        assert_eq!(picked.summary, "跟玩家乙聊過種田");
    }

    #[test]
    fn format_gossip_has_host_prefix() {
        let text = format_gossip("諾娃", "蓋了一座瞭望台");
        assert_eq!(text, "聽諾娃說：蓋了一座瞭望台");
    }

    #[test]
    fn format_gossip_clips_long_summary() {
        let long: String = "字".repeat(200);
        let text = format_gossip("諾娃", &long);
        // 前綴「聽諾娃說：」(5 字) + 最多 GOSSIP_SNIPPET_MAX_CHARS 字。
        assert_eq!(text.chars().count(), 5 + GOSSIP_SNIPPET_MAX_CHARS);
    }

    #[test]
    fn already_knows_true_on_exact_match() {
        let existing = vec![mem("vox_res_1", "諾娃", "聽諾娃說：她蓋了一口井", 1)];
        assert!(already_knows(&existing, "聽諾娃說：她蓋了一口井"));
    }

    #[test]
    fn already_knows_false_when_absent() {
        let existing = vec![mem("vox_res_1", "諾娃", "聽諾娃說：她蓋了一口井", 1)];
        assert!(!already_knows(&existing, "聽諾娃說：她種了小麥"));
    }

    #[test]
    fn already_knows_false_on_empty_history() {
        assert!(!already_knows(&[], "任何見聞"));
    }
}
