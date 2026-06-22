//! ROADMAP 498 全服里程碑喝采——世界戰報計數突破里程碑時，廣場 NPC 自發一句鼓舞語。
//!
//! 採集/收穫/擊殺/登入人次的累計數字第一次和廣場 NPC 的嘴連在一起：
//! 當今日採集突破 50/200/500/1000、收穫突破 30/100/300、擊殺突破 100/500/1000、
//! 登入突破 10/30/50 時，對應 NPC 在廣場位置發出一則就地泡泡（NpcSpeech）
//! 同時廣播到世界聊天頻道，讓玩家第一次感受到「廣場的人看著我們一起做了什麼」。
//!
//! 設計原則：
//! - **純查表、零 LLM、零 IO**：`check` 是純函式（只看 count），確定可測。
//! - **精確匹配**：`count == threshold` 才觸發——計數單調遞增，每個里程碑
//!   在整個伺服器生命週期恰好觸發一次，不需要額外的「已觸發」狀態。
//! - **面向玩家字串集中於本檔**（便於未來 i18n 替換）。

/// 里程碑觸發結果：NPC 識別碼、顯示名與鼓舞語。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MilestoneAnnouncement {
    /// VILLAGE_NPCS 中的 NPC 識別碼（用於 `npc_schedule::fallback_pos` 取座標）。
    pub npc_id: &'static str,
    /// 玩家看到的顯示名（中文，面向玩家字串）。
    pub npc_display: &'static str,
    /// 鼓舞語正文（面向玩家字串）。
    pub text: &'static str,
}

// ── 里程碑資料表 ─────────────────────────────────────────────────────────────
// 格式：(觸發門檻 count, 鼓舞語)
// bit 順序穩定；日後新增里程碑往末尾加，不插隊。

/// 採集里程碑（採購代理人諾亞負責採購，見到採集數字最興奮）。
const GATHER_MILESTONES: &[(u64, &str)] = &[
    (50,   "今天旅人們已採集了 50 份物資，倉庫快要放不下了，謝謝大家！"),
    (200,  "哇！今日採集突破 200 份，真是豐碩的一天，繼續加油！"),
    (500,  "難以置信！全體旅人合力採集了 500 份物資，這份勤勞讓我感動極了！"),
    (1000, "一千份！我這輩子從沒見過這麼大的採集量，今天是個傳說般的日子！"),
];

/// 收穫里程碑（評審卡特評審農博作物，最懂農耕成就）。
const HARVEST_MILESTONES: &[(u64, &str)] = &[
    (30,  "今日收穫達 30 次！旅人們的用心沒有白費，看得我心頭暖暖的。"),
    (100, "百次收穫！這樣的農耕盛況實屬罕見，每一株作物都是旅人的心血。"),
    (300, "三百次！今天是農耕的大日子，整個城鎮都聞得到豐收的芬芳。"),
];

/// 擊殺里程碑（獵手蘭卡追蹤城外威脅，擊殺數字就是她的語言）。
const KILL_MILESTONES: &[(u64, &str)] = &[
    (100,  "好樣的！今日共擊殺 100 隻怪物，城外要清靜一些了。"),
    (500,  "五百！今天的旅人們身手了得，怪物們應該瑟瑟發抖了！"),
    (1000, "一千隻！這是我見過最驍勇的一日，城鎮有你們真是幸運。"),
];

/// 今日登入人次里程碑（凱爾長老最在乎這塊土地上有多少人）。
const LOGIN_MILESTONES: &[(u64, &str)] = &[
    (10, "今日已有 10 位旅人踏上這片土地，廣場開始熱鬧了，歡迎歡迎！"),
    (30, "三十位旅人！城鎮從未如此鮮活，感謝每一位踏入這裡的人。"),
    (50, "半百！今天是個熱鬧的好日子，我感到這片大地因你們而煥然一新。"),
];

// ── 公開查詢函式 ─────────────────────────────────────────────────────────────

/// 採集計數里程碑：`count` 若精確落在門檻上，回傳對應公告；否則回 None。
pub fn gather_milestone(count: u64) -> Option<MilestoneAnnouncement> {
    check(count, GATHER_MILESTONES, "procurement_npc", "採購代理人諾亞")
}

/// 收穫計數里程碑。
pub fn harvest_milestone(count: u64) -> Option<MilestoneAnnouncement> {
    check(count, HARVEST_MILESTONES, "farm_fair_npc", "評審卡特")
}

/// 擊殺計數里程碑。
pub fn kill_milestone(count: u64) -> Option<MilestoneAnnouncement> {
    check(count, KILL_MILESTONES, "bounty_npc", "獵手蘭卡")
}

/// 今日登入人次里程碑。
pub fn login_milestone(count: u64) -> Option<MilestoneAnnouncement> {
    check(count, LOGIN_MILESTONES, "village_chief", "凱爾長老")
}

/// 純函式核心：在 `milestones` 表裡找第一個與 `count` 精確相等的門檻。
fn check(
    count: u64,
    milestones: &[(u64, &'static str)],
    npc_id: &'static str,
    npc_display: &'static str,
) -> Option<MilestoneAnnouncement> {
    milestones
        .iter()
        .find(|(threshold, _)| *threshold == count)
        .map(|(_, text)| MilestoneAnnouncement { npc_id, npc_display, text })
}

// ── 測試 ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── 採集里程碑 ─────────────────────────────────────────────────────────────

    #[test]
    fn gather_50_triggers_with_correct_npc() {
        let m = gather_milestone(50).expect("50 應觸發採集里程碑");
        assert_eq!(m.npc_id, "procurement_npc");
        assert!(!m.text.is_empty());
    }

    #[test]
    fn gather_49_no_trigger() {
        assert!(gather_milestone(49).is_none(), "49 不應觸發");
    }

    #[test]
    fn gather_51_no_trigger() {
        assert!(gather_milestone(51).is_none(), "51 不應觸發");
    }

    #[test]
    fn gather_200_triggers() {
        assert!(gather_milestone(200).is_some());
    }

    #[test]
    fn gather_500_triggers() {
        assert!(gather_milestone(500).is_some());
    }

    #[test]
    fn gather_1000_triggers() {
        assert!(gather_milestone(1000).is_some());
    }

    #[test]
    fn gather_0_no_trigger() {
        assert!(gather_milestone(0).is_none(), "0 不應觸發");
    }

    #[test]
    fn gather_u64_max_no_trigger() {
        assert!(gather_milestone(u64::MAX).is_none());
    }

    // ── 收穫里程碑 ─────────────────────────────────────────────────────────────

    #[test]
    fn harvest_30_triggers_with_correct_npc() {
        let m = harvest_milestone(30).expect("30 應觸發收穫里程碑");
        assert_eq!(m.npc_id, "farm_fair_npc");
        assert!(!m.text.is_empty());
    }

    #[test]
    fn harvest_29_no_trigger() {
        assert!(harvest_milestone(29).is_none());
    }

    #[test]
    fn harvest_100_and_300_trigger() {
        assert!(harvest_milestone(100).is_some());
        assert!(harvest_milestone(300).is_some());
    }

    // ── 擊殺里程碑 ─────────────────────────────────────────────────────────────

    #[test]
    fn kill_100_triggers_with_correct_npc() {
        let m = kill_milestone(100).expect("100 應觸發擊殺里程碑");
        assert_eq!(m.npc_id, "bounty_npc");
        assert!(!m.text.is_empty());
    }

    #[test]
    fn kill_99_no_trigger() {
        assert!(kill_milestone(99).is_none());
    }

    #[test]
    fn kill_999_no_trigger() {
        assert!(kill_milestone(999).is_none());
    }

    #[test]
    fn kill_500_and_1000_trigger() {
        assert!(kill_milestone(500).is_some());
        assert!(kill_milestone(1000).is_some());
    }

    // ── 登入里程碑 ─────────────────────────────────────────────────────────────

    #[test]
    fn login_10_triggers_with_correct_npc() {
        let m = login_milestone(10).expect("10 應觸發登入里程碑");
        assert_eq!(m.npc_id, "village_chief");
        assert!(!m.text.is_empty());
    }

    #[test]
    fn login_9_no_trigger() {
        assert!(login_milestone(9).is_none());
    }

    #[test]
    fn login_30_and_50_trigger() {
        assert!(login_milestone(30).is_some());
        assert!(login_milestone(50).is_some());
    }

    // ── 整體完整性 ────────────────────────────────────────────────────────────

    #[test]
    fn all_milestone_texts_nonempty() {
        // 所有定義的里程碑都有非空文字
        for c in [50u64, 200, 500, 1000] {
            assert!(!gather_milestone(c).unwrap().text.is_empty(), "gather {c}");
        }
        for c in [30u64, 100, 300] {
            assert!(!harvest_milestone(c).unwrap().text.is_empty(), "harvest {c}");
        }
        for c in [100u64, 500, 1000] {
            assert!(!kill_milestone(c).unwrap().text.is_empty(), "kill {c}");
        }
        for c in [10u64, 30, 50] {
            assert!(!login_milestone(c).unwrap().text.is_empty(), "login {c}");
        }
    }

    #[test]
    fn intermediate_counts_no_trigger() {
        // 里程碑之間的隨機計數都不觸發
        for c in [1u64, 49, 51, 199, 201, 499, 501, 999, 1001] {
            assert!(gather_milestone(c).is_none(), "gather {c} 不應觸發");
        }
        for c in [1u64, 29, 31, 99, 101, 299, 301] {
            assert!(harvest_milestone(c).is_none(), "harvest {c} 不應觸發");
        }
        for c in [1u64, 99, 101, 499, 501, 999, 1001] {
            assert!(kill_milestone(c).is_none(), "kill {c} 不應觸發");
        }
        for c in [1u64, 9, 11, 29, 31, 49, 51] {
            assert!(login_milestone(c).is_none(), "login {c} 不應觸發");
        }
    }
}
