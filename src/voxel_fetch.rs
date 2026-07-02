//! 乙太方界·居民跑腿採集 v1（指令→任務第三刀：「幫我採集」）。
//!
//! **架構分層（同 voxel_directed_task 的鐵律）**：本模組全是零 LLM、零鎖、零 async 的純邏輯——
//! 偵測玩家的跑腿指令、任務資料模型、台詞。鎖／廣播／世界寫入／持久化觸發全留在 `voxel_ws.rs`。
//!
//! 706（整地）、707（跟我來）之後，這是第三種「玩家明確交代、居民真的做得到」的小事：
//! 對居民說「幫我採集 3 塊木頭」，她放下手邊的事，走去採到指定數量，再親手走回你身邊交給你——
//! 讓「你的話有後果」延伸到具體的物資，而不只是地形或跟隨。

use crate::voxel_skills::GatherResource;

/// 沒指定數量時的預設份數（小而有感，不必等太久）。
pub const FETCH_DEFAULT_COUNT: u32 = 3;

/// 單次跑腿最多能要求的份數（v1 刻意保守，別讓一趟任務拖太久）。
pub const FETCH_MAX_COUNT: u32 = 6;

/// 整趟跑腿任務逾時（秒）：找資源＋走＋挖＋走回交付都算在內，給得寬鬆但不無限。
pub const FETCH_TASK_DEADLINE_SECS: f32 = 240.0;

/// 視為「走到玩家面前、可以交付材料」的水平距離（方塊，與 GIFT_REACH 同量級）。
pub const FETCH_DELIVER_REACH: f32 = 3.0;

/// 「跑腿」意圖詞：命中即視為在拜託居民去採集東西。刻意收斂成「幫我+動詞」組合，
/// 一般閒聊不會含這些詞，不誤觸發。
const FETCH_VERB_TOKENS: &[&str] = &["幫我採集", "幫我採", "幫我拿", "幫我找", "幫我搬", "幫我準備"];

/// 材料關鍵詞 → 對應資源型別（涵蓋 [`GatherResource`] 全部五種）。
const MATERIAL_TOKENS: &[(&str, GatherResource)] = &[
    ("木頭", GatherResource::Wood),
    ("木材", GatherResource::Wood),
    ("原木", GatherResource::Wood),
    ("石頭", GatherResource::Stone),
    ("石塊", GatherResource::Stone),
    ("石材", GatherResource::Stone),
    ("沙子", GatherResource::Sand),
    ("細沙", GatherResource::Sand),
    ("泥土", GatherResource::Dirt),
    ("泥巴", GatherResource::Dirt),
    ("草皮", GatherResource::Grass),
];

/// 從文字裡取出第一串連續 ASCII 數字並解析成份數（找不到／解析失敗回 `None`）。
/// 純函式、確定性。
fn extract_count(text: &str) -> Option<u32> {
    let mut digits = String::new();
    for ch in text.chars() {
        if ch.is_ascii_digit() {
            digits.push(ch);
        } else if !digits.is_empty() {
            break;
        }
    }
    if digits.is_empty() {
        None
    } else {
        digits.parse::<u32>().ok()
    }
}

/// 偵測：這句話是否在拜託居民「跑腿採集某種材料」。命中「跑腿動詞 + 材料關鍵詞」才算，
/// 兩者缺一都回 `None`（別誤觸發純聊天，如「木頭好漂亮」「幫我找露娜」）。
/// 份數：文字裡有數字就依數字（夾在 [1, FETCH_MAX_COUNT]）、沒有就用 [`FETCH_DEFAULT_COUNT`]。
/// 純函式、確定性、可測、零 LLM。
pub fn detect_fetch_command(text: &str) -> Option<(GatherResource, u32)> {
    if !FETCH_VERB_TOKENS.iter().any(|t| text.contains(t)) {
        return None;
    }
    let resource = MATERIAL_TOKENS.iter().find(|(kw, _)| text.contains(kw)).map(|(_, r)| *r)?;
    let count = extract_count(text).unwrap_or(FETCH_DEFAULT_COUNT).clamp(1, FETCH_MAX_COUNT);
    Some((resource, count))
}

/// 居民「答應跑腿」的回覆（誠實而願意——這是她真的做得到的小事）。純函式、可測、零 LLM。
pub fn accept_line(resource_name: &str, count: u32, pick: usize) -> String {
    const POOL: [&str; 4] = [
        "好，我這就去採{n}份{item}，帶回來給你！",
        "沒問題，交給我吧——我去找{n}份{item}，找到就送過去～",
        "好呀，我這就出發採{n}份{item}，等我一下下！",
        "交給我！我去採{n}份{item}，湊齊了就親自送到你手上。",
    ];
    let tpl = POOL[pick % POOL.len()];
    tpl.replace("{n}", &count.to_string()).replace("{item}", resource_name)
}

/// 居民「交付材料」的完成台詞。`requested` 是原本答應的份數、`delivered` 是實際帶回的份數——
/// 兩者相等就是圓滿完成；`delivered < requested` 代表沒找齊、老實只交出採到的份（誠實而不硬撐）。
/// 純函式、可測、零 LLM。
pub fn deliver_line(resource_name: &str, requested: u32, delivered: u32, pick: usize) -> String {
    if delivered >= requested {
        const POOL: [&str; 4] = [
            "採到了！這{n}份{item}給你～",
            "久等啦，{n}份{item}都採齊了，拿去用吧！",
            "任務完成！{n}份{item}親手交給你。",
            "呼～總算湊齊{n}份{item}了，這是你要的！",
        ];
        let tpl = POOL[pick % POOL.len()];
        tpl.replace("{n}", &delivered.to_string()).replace("{item}", resource_name)
    } else {
        const POOL: [&str; 3] = [
            "抱歉，附近只找到{n}份{item}，先給你這些，之後找到再補！",
            "沒能湊滿，只採到{n}份{item}——先拿去用，我之後再幫你找找！",
            "找了好一陣子，目前只有{n}份{item}，先交給你，別的之後再說～",
        ];
        let tpl = POOL[pick % POOL.len()];
        tpl.replace("{n}", &delivered.to_string()).replace("{item}", resource_name)
    }
}

/// 一份都沒採到就徹底放棄時的道歉台詞（誠實而不無窮重試）。純函式、可測、零 LLM。
pub fn fail_line(resource_name: &str, pick: usize) -> String {
    const POOL: [&str; 3] = [
        "抱歉，這附近找不到{item}，我先回來了，之後找到再幫你採！",
        "找了一圈都沒看到{item}……先跟你說一聲，晚點再試試看。",
        "唉，附近沒有{item}可以採，這次先放棄，別的地方再找找看！",
    ];
    let tpl = POOL[pick % POOL.len()];
    tpl.replace("{item}", resource_name)
}

/// 一件跑腿採集任務：她答應了要採 `requested` 份 `resource`，目前還差 `remaining` 份，
/// 身上已經帶著 `carried` 份。`remaining` 歸零＝該回去交付了。純資料 + 純方法、可測。
#[derive(Clone, Debug, PartialEq)]
pub struct FetchTask {
    /// 下令的玩家身份鍵（供交付對象辨識、記憶／Feed 記錄「是誰請她跑的」）。
    pub requester: String,
    /// 要採的資源型別。
    pub resource: GatherResource,
    /// 原本答應的份數（供交付時比較「有沒有湊齊」）。
    pub requested: u32,
    /// 還差幾份才夠（歸零＝該去交付了）。
    pub remaining: u32,
    /// 身上已經採到、尚未交付的份數。
    pub carried: u32,
    /// 剩餘逾時（秒）：整趟任務（找+走+挖+送）的總預算，歸零就帶著目前有的先回去交差。
    pub deadline: f32,
}

impl FetchTask {
    /// 建一個全新任務（remaining=requested、carried=0、deadline 滿格）。
    pub fn new(requester: String, resource: GatherResource, count: u32) -> Self {
        Self {
            requester,
            resource,
            requested: count,
            remaining: count,
            carried: 0,
            deadline: FETCH_TASK_DEADLINE_SECS,
        }
    }

    /// 是否已採齊該去交付了（remaining 歸零）。
    pub fn is_gathering_done(&self) -> bool {
        self.remaining == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── detect_fetch_command：該中 / 不誤觸發 ────────────────────────────────────

    #[test]
    fn detects_all_material_kinds_with_verb() {
        assert_eq!(detect_fetch_command("幫我採集木頭"), Some((GatherResource::Wood, FETCH_DEFAULT_COUNT)));
        assert_eq!(detect_fetch_command("幫我採集石頭"), Some((GatherResource::Stone, FETCH_DEFAULT_COUNT)));
        assert_eq!(detect_fetch_command("幫我拿一些沙子"), Some((GatherResource::Sand, FETCH_DEFAULT_COUNT)));
        assert_eq!(detect_fetch_command("幫我找泥土"), Some((GatherResource::Dirt, FETCH_DEFAULT_COUNT)));
        assert_eq!(detect_fetch_command("幫我搬草皮"), Some((GatherResource::Grass, FETCH_DEFAULT_COUNT)));
    }

    #[test]
    fn parses_explicit_count_and_clamps_to_max() {
        assert_eq!(detect_fetch_command("幫我採集5塊木頭"), Some((GatherResource::Wood, 5)));
        assert_eq!(detect_fetch_command("幫我採集1塊石頭"), Some((GatherResource::Stone, 1)));
        // 超過上限 → 夾到 FETCH_MAX_COUNT。
        assert_eq!(detect_fetch_command("幫我採集99塊木頭"), Some((GatherResource::Wood, FETCH_MAX_COUNT)));
        // 0 份 → 夾到最少 1 份（不該有 0 份任務）。
        assert_eq!(detect_fetch_command("幫我採集0塊木頭"), Some((GatherResource::Wood, 1)));
    }

    #[test]
    fn ignores_chitchat_and_material_mentions_without_verb() {
        // 提到材料但沒有跑腿動詞 → 只是聊天，不誤觸發。
        assert_eq!(detect_fetch_command("木頭好漂亮"), None);
        assert_eq!(detect_fetch_command("這附近石頭好多"), None);
        assert_eq!(detect_fetch_command("你好呀，今天天氣真好"), None);
        assert_eq!(detect_fetch_command(""), None);
    }

    #[test]
    fn ignores_verb_without_recognized_material() {
        // 有跑腿動詞但沒提到可辨識材料 → 不誤觸發（別跟「幫我找露娜」這類請求衝突）。
        assert_eq!(detect_fetch_command("幫我找露娜"), None);
        assert_eq!(detect_fetch_command("幫我拿一下"), None);
    }

    #[test]
    fn first_matching_material_wins_deterministically() {
        // 同時提到兩種材料時，取文字裡先命中的那個（確定性、不隨機）。
        let a = detect_fetch_command("幫我採集木頭跟石頭");
        assert_eq!(a, Some((GatherResource::Wood, FETCH_DEFAULT_COUNT)));
    }

    // ── 台詞：非空、依 pick 有變化 ─────────────────────────────────────────────────

    #[test]
    fn accept_line_is_warm_and_varied_and_mentions_item() {
        let a = accept_line("木頭", 3, 0);
        let b = accept_line("木頭", 3, 1);
        assert!(!a.is_empty());
        assert_ne!(a, b);
        assert!(a.contains("木頭"));
        assert!(a.contains('3'));
    }

    #[test]
    fn deliver_line_full_vs_partial_wording_differs() {
        let full = deliver_line("石頭", 3, 3, 0);
        let partial = deliver_line("石頭", 3, 1, 0);
        assert!(!full.is_empty() && !partial.is_empty());
        assert_ne!(full, partial, "圓滿 vs 短交的口吻應不同");
        assert!(partial.contains('1'));
        assert!(full.contains('3'));
    }

    #[test]
    fn fail_line_is_nonempty_and_mentions_item() {
        let a = fail_line("木頭", 0);
        assert!(!a.is_empty());
        assert!(a.contains("木頭"));
    }

    // ── FetchTask：狀態轉換 ────────────────────────────────────────────────────────

    #[test]
    fn fetch_task_tracks_progress_to_completion() {
        let mut task = FetchTask::new("濕濕的".into(), GatherResource::Wood, 3);
        assert_eq!(task.remaining, 3);
        assert_eq!(task.carried, 0);
        assert!(!task.is_gathering_done());
        task.remaining -= 1;
        task.carried += 1;
        task.remaining -= 1;
        task.carried += 1;
        task.remaining -= 1;
        task.carried += 1;
        assert!(task.is_gathering_done());
        assert_eq!(task.carried, 3);
    }

    #[test]
    fn extract_count_parses_and_ignores_non_digits() {
        assert_eq!(extract_count("採集5塊木頭"), Some(5));
        assert_eq!(extract_count("採集99塊木頭"), Some(99));
        assert_eq!(extract_count("採集木頭"), None);
        assert_eq!(extract_count(""), None);
    }
}
