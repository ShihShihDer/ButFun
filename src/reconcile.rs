//! 居民和解委託（ROADMAP 364）——玩家第一次能「牽動」NPC↔NPC 的人際關係。
//!
//! 故鄉七大 NPC 之間有一張隱形的好惡網（見 `npc_relations.rs`）：世界事件會讓某些
//! 對子鬧僵、某些對子變親；ROADMAP 355 的派系面板讓玩家「看得到」這張網，但玩家
//! 至今**完全無法插手**——關係只被世界事件與時間衰減牽動，玩家是局外人。
//!
//! 本模組補上玩家對社交網的第一個**能動性**：當鎮上某對 NPC 鬧僵（雙向平均好惡值
//! 落在「可修補」帶），其中一位會託玩家捎一份和解信物給對方；玩家跑腿送達後，
//! 兩人的好惡值**真的回暖**（玩家成了和事佬），城鎮記憶石記下這樁、世界頻道同慶；
//! 若回暖到足以越過結盟門檻，既有 355 派系系統會在下一輪自然宣告「結盟」——
//! 玩家促成的和解，化成看得見的社會結構變化。
//!
//! 設計鐵律：
//! - **純邏輯、零 LLM**：信物與請求台詞全查表，背景社交不燒任何額度。
//! - **零 migration、純記憶體**：玩家身上的委託不入快照、不持久化（鏡像觀星 bitmask）；
//!   關係調整走 `npc_relations` 的記憶體狀態，重啟自然清零。
//! - **天然防 farm、零額外帳本**：送達後好惡值被推出「可修補」帶，要靠既有的緩慢
//!   衰減（每 5 分鐘 1 點）才會再落回——同一對天然進入長冷卻，不需任何冷卻表。
//! - **近零經濟擾動**：獎勵刻意壓小（少量乙太＋探索熟練度），不碰戰力／經濟核心。
//! - 面向玩家字串集中於本檔，保留 i18n 空間；繁中註解。

use crate::npc_factions::{mutual_avg, npc_display_name};
use crate::npc_relations::NpcRelationsState;

/// 雙向平均好惡值低於此值（且高於 0）即視為「鬧僵、可修補」，會生出和解委託。
/// 刻意設在中性（50）之下一截：明顯偏冷才值得玩家居中緩頰。
pub const MENDABLE_CEIL: i32 = 48;

/// 送達後對雙向各加的好惡值。設得夠大，一次和解即把該對推出「可修補」帶，
/// 之後靠既有緩慢衰減才會再落回——形成天然長冷卻（約一小時尺度）。
pub const RECONCILE_BUMP: i32 = 18;

/// 送達成功給玩家的乙太獎勵（壓小，近零經濟擾動）。
pub const REWARD_ETHER: u32 = 8;

/// 送達成功給玩家的探索熟練度經驗。
pub const EXPLORER_XP: u32 = 10;

/// 交付時，玩家需走到對方 NPC 工位的此半徑內（像素）才算送達。
pub const DELIVER_REACH: f32 = 120.0;

/// 故鄉七大 NPC（與 `npc_relations.rs` / `npc_factions.rs` 一致）。
const ALL_NPCS: &[&str] = &[
    "merchant",
    "workshop_npc",
    "bounty_npc",
    "expedition_npc",
    "procurement_npc",
    "farm_fair_npc",
    "village_chief",
];

/// 一樁待促成的和解：委託人 `from` 託玩家把信物送給 `to`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Errand {
    /// 委託人（開口請玩家幫忙緩頰的 NPC）id。
    pub from: String,
    /// 和解對象（玩家要把信物送達的 NPC）id。
    pub to: String,
}

/// 一對 NPC 是否落在「鬧僵、可修補」帶（雙向平均好惡值 < `MENDABLE_CEIL`）。
pub fn is_mendable(relations: &NpcRelationsState, a: &str, b: &str) -> bool {
    let avg = mutual_avg(relations, a, b);
    avg < MENDABLE_CEIL
}

/// 找出鎮上**最該和解**的一對 NPC（雙向平均好惡值最低、且落在可修補帶）。
///
/// 以 `ALL_NPCS` 的固定次序列舉所有無序對，取平均值最低者；平手時取先出現的對，
/// 保證確定性（同一關係狀態永遠選同一對，便於測試與玩家預期）。
/// 全鎮皆親近（無對落在可修補帶）時回 `None`——此刻沒有和解委託可接。
pub fn most_strained_pair(relations: &NpcRelationsState) -> Option<Errand> {
    let mut best: Option<(i32, &str, &str)> = None;
    for (i, &a) in ALL_NPCS.iter().enumerate() {
        for &b in &ALL_NPCS[i + 1..] {
            let avg = mutual_avg(relations, a, b);
            if avg < MENDABLE_CEIL {
                // 嚴格小於：只有更低的才取代，平手保留先出現者（確定性）。
                if best.map_or(true, |(best_avg, _, _)| avg < best_avg) {
                    best = Some((avg, a, b));
                }
            }
        }
    }
    best.map(|(_, a, b)| Errand {
        from: a.to_string(),
        to: b.to_string(),
    })
}

/// 委託人依其行當會塞給玩家的和解信物（查表，蒸汽龐克療癒風）。
/// 集中於此一處，未來在地化或調整只動這裡。
pub fn peace_token(from_id: &str) -> &'static str {
    match from_id {
        "merchant" => "一瓶溫好的城鎮特釀",
        "workshop_npc" => "一枚親手打的小銅環",
        "bounty_npc" => "一包野地帶回的乾糧",
        "expedition_npc" => "一塊遠途撿來的奇石",
        "procurement_npc" => "一札細心謄好的好話",
        "farm_fair_npc" => "一籃自家園子的鮮果",
        "village_chief" => "一封誠懇的手寫信",
        _ => "一份小小的心意",
    }
}

/// 委託人開口請玩家居中緩頰的台詞。
pub fn plea_line(from_id: &str, to_id: &str) -> String {
    format!(
        "{}：我和{}前陣子鬧得有點僵……能替我把{}捎去給他、順道說聲對不起嗎？",
        npc_display_name(from_id),
        npc_display_name(to_id),
        peace_token(from_id),
    )
}

/// 送達成功後，給城鎮記憶石 / 世界頻道的同慶文字。
pub fn celebrate_line(from_id: &str, to_id: &str, warmth: i32) -> String {
    format!(
        "🕊️ 在旅人的居中緩頰下，{}與{}冰釋前嫌，情誼回暖到 {}/100。",
        npc_display_name(from_id),
        npc_display_name(to_id),
        warmth,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::npc_relations::NpcRelationsState;

    #[test]
    fn initial_world_has_a_mendable_pair() {
        // 初始關係裡 獵手↔工匠 = 43（< 48），應被選為最該和解的一對。
        let r = NpcRelationsState::new();
        let e = most_strained_pair(&r).expect("初始應有一對鬧僵可修補");
        let pair = [e.from.as_str(), e.to.as_str()];
        assert!(
            pair.contains(&"bounty_npc") && pair.contains(&"workshop_npc"),
            "初始最該和解的應是獵手↔工匠（43），實得 {pair:?}"
        );
    }

    #[test]
    fn picks_the_lowest_pair() {
        let mut r = NpcRelationsState::new();
        // 人為造一對更僵的：商人↔里長壓到 20。
        r.set_pair_for_test("merchant", "village_chief", 20, 20);
        let e = most_strained_pair(&r).expect("應有可修補對");
        let pair = [e.from.as_str(), e.to.as_str()];
        assert!(
            pair.contains(&"merchant") && pair.contains(&"village_chief"),
            "應選平均值最低的商人↔里長（20），實得 {pair:?}"
        );
    }

    #[test]
    fn no_errand_when_all_warm() {
        // 全部對拉到 60（皆 >= 48），無可修補對 → None。
        let mut r = NpcRelationsState::default();
        for &a in ALL_NPCS {
            for &b in ALL_NPCS {
                if a != b {
                    r.set_pair_for_test(a, b, 60, 60);
                }
            }
        }
        assert!(most_strained_pair(&r).is_none(), "全鎮親近時不應有和解委託");
    }

    #[test]
    fn deterministic_pick_on_ties() {
        // 兩對同為 30，應穩定選 ALL_NPCS 次序中先出現的對（商人最靠前）。
        let mut r = NpcRelationsState::new();
        r.set_pair_for_test("merchant", "bounty_npc", 30, 30);
        r.set_pair_for_test("expedition_npc", "village_chief", 30, 30);
        let e1 = most_strained_pair(&r).unwrap();
        let e2 = most_strained_pair(&r).unwrap();
        assert_eq!(e1, e2, "同一狀態應永遠選同一對（確定性）");
        assert_eq!(e1.from, "merchant", "平手時應選次序最前的對");
    }

    #[test]
    fn nudge_lifts_pair_out_of_mendable_band() {
        // 送達後該對被推出可修補帶 → 天然進入冷卻（不再被選）。
        let mut r = NpcRelationsState::new();
        let e = most_strained_pair(&r).unwrap();
        let before = mutual_avg(&r, &e.from, &e.to);
        let after = r.nudge_pair(&e.from, &e.to, RECONCILE_BUMP);
        assert!(after > before, "和解後好惡值應回暖");
        assert!(
            !is_mendable(&r, &e.from, &e.to),
            "回暖後不應再落在可修補帶（天然冷卻），after={after}"
        );
    }

    #[test]
    fn tokens_and_lines_non_empty_for_all_npcs() {
        for &id in ALL_NPCS {
            assert!(!peace_token(id).is_empty(), "{id} 信物應非空");
            assert!(!plea_line(id, "merchant").is_empty(), "{id} 請求台詞應非空");
        }
        assert!(!celebrate_line("merchant", "workshop_npc", 60).is_empty());
    }
}
