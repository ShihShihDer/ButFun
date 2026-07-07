//! 乙太方界·居民注意到你身邊跟著的馴服動物 v1（自主提案切片）。
//!
//! **真缺口**：850/851（餵野兔馴服＋跟隨）、870（放養雞）讓玩家能馴服一隻動物、牠從此
//! 像小跟班一樣跟著你走——但這份羈絆完全是玩家與動物之間的私密互動，**居民從未感知過**。
//! 同樣是「玩家做了一件事、居民路過該有反應」的模式，773（建造讚賞）／774（種田讚賞，
//! 見 `voxel_farm_admire`）都已示範過，卻唯獨「你馴服的動物跟在你身邊走」這件事，居民
//! 視而不見——是 wildlife 系統唯一還沒被「記憶要驅動行為」北極星碰過的一角。
//!
//! **與既有元素 razor-sharp 區隔（非同軸重複）**：
//! - 不是 773/774 的重複——那兩刀觸發於玩家「連續動作」（放置方塊／翻土播種）；
//!   本刀觸發於「玩家身邊有已馴服且正在跟隨的動物」這個**狀態**，判定管線、冷卻鍵
//!   完全獨立，也不需要連段（一次靠近就可能觸發，畢竟「牽著寵物散步」本身就是持續的）。
//! - 不是 850/851/870 本身——那三刀已完工的是「動物怎麼看你」；本刀補的是「別人怎麼看
//!   你和你的寵物」，wildlife 與居民關係系統第一次交會。
//!
//! **純邏輯層**：是否「寵物就在身邊」（[`has_pet_nearby`]）、是否該觸發讚賞
//! （[`admire_triggers`]）、讚賞台詞（[`admire_say_line`]）、記憶摘要（[`admire_memory_line`]）
//! 全是確定性純函式，零 LLM、零鎖、零 IO。鎖／廣播／記憶寫入全在 `voxel_ws.rs`，且沿用
//! 既有 773/774 那條已驗證的「短鎖即釋、不巢狀」慣例，於低頻（15 秒）tick 節拍檢查。
//!
//! **成本 / 濫用防護**：台詞全為固定模板、永不回放玩家原話（無注入／NSFW 風險）；每位
//! 居民設 [`PET_ADMIRE_COOLDOWN_SECS`] 全域冷卻（不分是哪位玩家的寵物），天然防洗版、
//! 也防好感（＝記憶筆數）被刷爆。零 migration、零新協議欄位、零前端改動、零新美術、
//! FPS 零影響（純後端、低頻 tick）。

/// 居民能注意到寵物的讚賞觸及半徑（方塊距離，水平 XZ 平面，居民↔玩家）。
/// 與 773/774 建造/種田讚賞 `ADMIRE_RADIUS` 同量級。
pub const PET_ADMIRE_RADIUS: f32 = 6.0;

/// 「寵物就在你身邊」的判定半徑（方塊距離，動物↔玩家）：已跟隨的寵物通常會跟到
/// `FOLLOW_STOP_DIST`（2.5）附近就不再擠，本值稍微寬鬆一點，容許牠跟丟一兩步仍算數。
pub const PET_NEARBY_RADIUS: f32 = 5.0;

/// 同一位居民的讚賞冷卻（秒，全域——不分是哪位玩家的寵物）。與 773/774 讚賞冷卻同量級，
/// 讓讚賞稀有有份量，也把「牽著寵物繞著居民走來走去刷好感」的速率天然夾死。
pub const PET_ADMIRE_COOLDOWN_SECS: u64 = 150;

/// 依「寵物與玩家的距離平方」判斷寵物是否算「就在身邊」（純函式）。
pub fn has_pet_nearby(pet_player_dist_sq: f32) -> bool {
    pet_player_dist_sq <= PET_NEARBY_RADIUS * PET_NEARBY_RADIUS
}

/// 是否該觸發居民讚賞（純函式）：寵物在身邊 ＋ 居民離你夠近 ＋ 冷卻已過。
/// 「居民此刻是否有空／有沒有正在冒別的泡泡」由呼叫端另外把關（見 `voxel_ws.rs`）。
pub fn admire_triggers(pet_nearby: bool, resident_dist_sq: f32, cooldown_ok: bool) -> bool {
    pet_nearby && resident_dist_sq <= PET_ADMIRE_RADIUS * PET_ADMIRE_RADIUS && cooldown_ok
}

/// 居民注意到你身邊跟著寵物的台詞（繁中、面向玩家、i18n 集中於此；確定性依 `pick` 選句）。
/// 刻意**不含玩家原話**、只帶玩家名與寵物種類——無注入／NSFW 風險；語氣與 773/774
/// 建造/種田讚賞刻意區隔（著眼「陪伴」，不提「蓋」「種田」）。
pub fn admire_say_line(player_name: &str, pet_label: &str, pick: usize) -> String {
    const LINES: &[&str] = &[
        "{p}，你這隻{k}跟你跟得真緊，看起來感情很好呢。",
        "咦，{p}身邊那隻{k}是你養的嗎？好乖，一直跟著你。",
        "看{p}走到哪、{k}就跟到哪，這畫面真讓人會心一笑。",
        "{p}，你和你的{k}走在一起，感覺特別溫馨。",
    ];
    LINES[pick % LINES.len()].replace("{p}", player_name).replace("{k}", pet_label)
}

/// 把「注意到這位旅人身邊跟著一隻馴服動物」寫成一段居民的記憶摘要（第一人稱、episodic）。
///
/// 刻意避開 [`crate::voxel_memory::classify_importance`] 的目標／偏好／承諾／身份關鍵詞
/// （不含「要蓋／想要／喜歡／記住／我是」等），讓它停在情節記憶層、只累積好感，
/// 不誤升級成語意精華——與 773/774 讚賞記憶同款設計。
pub fn admire_memory_line(player_name: &str, pet_label: &str) -> String {
    format!("看見{player_name}身邊跟著一隻{pet_label}，兩個一起散步的樣子，我都看在眼裡。")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_pet_nearby_within_radius() {
        assert!(has_pet_nearby(4.9 * 4.9));
        assert!(!has_pet_nearby(5.1 * 5.1));
    }

    #[test]
    fn has_pet_nearby_boundary_is_inclusive() {
        assert!(has_pet_nearby(PET_NEARBY_RADIUS * PET_NEARBY_RADIUS));
    }

    #[test]
    fn triggers_needs_all_three() {
        assert!(admire_triggers(true, 0.0, true));
        assert!(!admire_triggers(false, 0.0, true), "寵物不在身邊不該觸發");
        assert!(!admire_triggers(true, 0.0, false), "冷卻未到不該觸發");
        let just_out = PET_ADMIRE_RADIUS * PET_ADMIRE_RADIUS + 0.01;
        assert!(!admire_triggers(true, just_out, true), "居民太遠不該觸發");
    }

    #[test]
    fn triggers_at_exact_radius() {
        let on_edge = PET_ADMIRE_RADIUS * PET_ADMIRE_RADIUS;
        assert!(admire_triggers(true, on_edge, true));
    }

    #[test]
    fn say_line_is_deterministic_and_carries_name_and_pet() {
        let a = admire_say_line("露娜客", "兔子", 0);
        let b = admire_say_line("露娜客", "兔子", 0);
        assert_eq!(a, b);
        assert!(a.contains("露娜客"));
        assert!(a.contains("兔子"));
        assert!(!a.contains("{p}") && !a.contains("{k}"));
    }

    #[test]
    fn say_line_supports_chicken_label_too() {
        let line = admire_say_line("旅人", "雞", 1);
        assert!(line.contains("雞"));
    }

    #[test]
    fn say_lines_are_distinct() {
        let lines: std::collections::HashSet<String> =
            (0..4).map(|p| admire_say_line("旅人", "兔子", p)).collect();
        assert_eq!(lines.len(), 4, "四句應各不相同");
    }

    #[test]
    fn say_line_pick_wraps_without_panic() {
        let line = admire_say_line("旅人", "兔子", usize::MAX);
        assert!(!line.is_empty());
    }

    #[test]
    fn memory_line_stays_episodic() {
        let line = admire_memory_line("農夫", "兔子");
        assert!(matches!(
            crate::voxel_memory::classify_importance(&line),
            crate::voxel_memory::Importance::Ephemeral
        ));
        assert!(line.contains("農夫"));
        assert!(line.contains("兔子"));
    }

    #[test]
    fn memory_line_no_newline() {
        assert!(!admire_memory_line("旅人", "雞").contains('\n'));
    }
}
