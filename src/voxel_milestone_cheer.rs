//! 乙太方界·居民為你的個人里程碑喝采 v1（自主提案切片，ROADMAP 主軸至 867 全打勾後）。
//!
//! **真缺口**：`try_unlock_milestone`（ROADMAP 724）解鎖任何一枚成就徽章時，注解自己就
//! 寫著「單播…給該玩家自己（**不廣播全員，是私人旅程**）」——你人生第一次採集、第一次
//! 建造、第一次交易、第一次跟居民混熟，二十一種「第一次」全部只有你自己知道，身邊哪怕
//! 站著一位居民，也不會有任何反應。856「村莊集體里程碑」做的是**地標累計數**（村莊本身
//! 的里程碑），設計說明明講與玩家個人成就刻意區隔；845「居民關心你挨餓」是居民**先**
//! 注意到你有難才開口。**居民為你「達成」而非「有難」開心**這一格，至今是空的。
//!
//! **做法**：不新開一條判定管線，直接在既有 `try_unlock_milestone` 冪等解鎖成功的那一刻
//! 接一手——比照 773/863 讚賞的「挑一位近旁有空的居民」手法，找到玩家當下位置附近一位
//! 閒著的居民，讓她為你喝采、記進心裡、動態牆播報一句。**里程碑本身全庫只會對每位玩家
//! 觸發一次**（`unlock` 冪等），天然不會刷版，因此本刀刻意不另設冷卻——稀有本身就是
//! 節流。
//!
//! **與既有元素 razor-sharp 區隔（非同軸重複）**：
//! - **856 村莊集體里程碑**＝地標數跨門檻時全村一起歡呼，慶祝的是「村莊」；本刀慶祝的是
//!   「你」這位玩家的個人旅程，兩者觸發鍵完全不同（地標數 vs. 個人徽章）、互不影響。
//! - **845 居民關心你挨餓**＝居民主動偵測你的困境並先開口關心；本刀是**你先達成了什麼**，
//!   居民才回應——因果方向相反，一個是「難」一個是「成」。
//! - **773/863 建造/種田讚賞**＝針對「連續動作」的過程性欣賞；本刀針對的是**里程碑系統
//!   本身已判定過的、一次性、跨全部療癒循環面向的「第一次」瞬間**，不局限單一動作類型。
//!
//! **純邏輯層**：喝采觸及半徑判定（[`cheer_eligible`]）、喝采台詞（[`cheer_say_line`]）、
//! 記憶摘要（[`cheer_memory_line`]）、動態牆播報句（[`cheer_feed_line`]）全是確定性純
//! 函式，零 LLM、零鎖、零 IO。鎖 / 廣播 / 記憶寫入在 `voxel_ws.rs`，沿用既有短取即釋、
//! 不巢狀慣例。
//!
//! **成本 / 濫用防護**：台詞全為固定模板、永不回放玩家原話（無注入 / NSFW 風險）；
//! 觸發鍵完全由伺服器內部的里程碑冪等解鎖驅動，玩家無法自報或重複觸發。零 migration、
//! 零新協議欄位、零前端改動、零新美術、FPS 零影響（僅在成就解鎖瞬間觸發一次）。

/// 喝采觸及半徑（方塊距離，水平 XZ 平面）：居民要離你夠近才「聽得見」這份喜訊。
/// 里程碑是稀有的一次性事件（而非連續判定），半徑刻意比 773/863 常態讚賞（6）略寬，
/// 讓這個一生只有一次的瞬間更容易被身邊的人分享到。
pub const CHEER_RADIUS: f32 = 10.0;

/// 是否在喝采觸及範圍內（純函式、含界）。
pub fn cheer_eligible(dist_sq: f32) -> bool {
    dist_sq <= CHEER_RADIUS * CHEER_RADIUS
}

/// 居民為玩家達成里程碑喝采的台詞（繁中、面向玩家、i18n 集中於此；確定性依 `pick` 選句）。
/// 刻意**不含玩家原話**，只點名里程碑名稱與圖示，無注入 / NSFW 風險。
pub fn cheer_say_line(player_name: &str, milestone_name_zh: &str, milestone_icon: &str, pick: usize) -> String {
    const LINES: &[&str] = &[
        "{p}！我聽說你剛達成「{m}」了，{i} 真心為你高興！",
        "「{m}」達成啦？{i} {p}，你真的很厲害！",
        "{p}，這一步我都看在眼裡——「{m}」，值得慶祝一下！{i}",
        "{i} 「{m}」！{p}，這一步你走得很扎實。",
        "聽到{p}達成「{m}」，我忍不住想跟你說聲恭喜！{i}",
    ];
    LINES[pick % LINES.len()]
        .replace("{p}", player_name)
        .replace("{m}", milestone_name_zh)
        .replace("{i}", milestone_icon)
}

/// 把「看著這位旅人達成里程碑」寫成一段居民的記憶摘要（第一人稱、episodic）。
///
/// 刻意避開 [`crate::voxel_memory::classify_importance`] 的身份／目標／偏好／承諾關鍵詞
/// （不含「要蓋 / 想要 / 喜歡 / 記住 / 我是」等），讓它停在情節記憶層、只累積好感，
/// 不誤升級成語意精華——與 773/863 讚賞記憶同款設計。
pub fn cheer_memory_line(player_name: &str, milestone_name_zh: &str) -> String {
    format!("看著{player_name}達成「{milestone_name_zh}」這一刻，我也跟著開心。")
}

/// 動態牆播報句（比照 773/863 讚賞的 Feed 慣例）。
pub fn cheer_feed_line(resident_name: &str, player_name: &str, milestone_name_zh: &str, milestone_icon: &str) -> String {
    format!("{resident_name}聽到{player_name}達成「{milestone_name_zh}」{milestone_icon}，由衷喝采。")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cheer_eligible_within_radius() {
        assert!(cheer_eligible(0.0));
        assert!(cheer_eligible(CHEER_RADIUS * CHEER_RADIUS - 1.0));
    }

    #[test]
    fn cheer_eligible_exact_boundary_included() {
        assert!(cheer_eligible(CHEER_RADIUS * CHEER_RADIUS));
    }

    #[test]
    fn cheer_eligible_just_outside_excluded() {
        assert!(!cheer_eligible(CHEER_RADIUS * CHEER_RADIUS + 0.01));
    }

    #[test]
    fn say_line_is_deterministic_and_carries_name_and_milestone() {
        let a = cheer_say_line("旅人", "初次採集", "⛏️", 0);
        let b = cheer_say_line("旅人", "初次採集", "⛏️", 0);
        assert_eq!(a, b);
        assert!(a.contains("旅人"));
        assert!(a.contains("初次採集"));
        assert!(a.contains("⛏️"));
        assert!(!a.contains("{p}"));
        assert!(!a.contains("{m}"));
        assert!(!a.contains("{i}"));
    }

    #[test]
    fn say_lines_are_distinct() {
        let lines: std::collections::HashSet<String> =
            (0..5).map(|p| cheer_say_line("旅人", "初次建造", "🧱", p)).collect();
        assert_eq!(lines.len(), 5, "五句應各不相同");
    }

    #[test]
    fn memory_line_stays_episodic() {
        let line = cheer_memory_line("旅人", "初次交易");
        assert!(matches!(
            crate::voxel_memory::classify_importance(&line),
            crate::voxel_memory::Importance::Ephemeral
        ));
        assert!(line.contains("旅人"));
        assert!(line.contains("初次交易"));
    }

    #[test]
    fn memory_line_no_newline() {
        assert!(!cheer_memory_line("旅人", "初次採集").contains('\n'));
    }

    #[test]
    fn feed_line_carries_all_names() {
        let line = cheer_feed_line("露娜", "旅人", "初次熟識", "💛");
        assert!(line.contains("露娜"));
        assert!(line.contains("旅人"));
        assert!(line.contains("初次熟識"));
        assert!(line.contains("💛"));
    }
}
