//! 乙太方界·愛的結晶——成婚戀人一起迎來孩子 v1（自主提案切片，ROADMAP 928）。
//!
//! 戀愛弧一路走到 927「戀人成婚」收尾：一對交情夠深的戀人白天就地結為連理、立起花拱門。
//! 但村裡的「新居民出生」（人口成長 v1）至今與婚姻完全無關——出生系統只確定性挑**一位**
//! 父母無性繁衍、繼承其技能，成婚的夫妻和一般居民一樣，成家之後再無下文。`docs/PLAN_ETHERVOX.md`
//! §3 反覆點名的「小社會湧現」裡，**家庭**這塊始終缺席：兩位相愛、成婚的居民，從沒真正
//! 「一起」迎來過一個孩子。
//!
//! 本切片把兩條線第一次接起來：**當村裡有一對成婚夫妻（雙方都還在人口內），下一個新生命
//! 就優先由這對夫妻共同迎來**——孩子是**兩位**父母的孩子、承繼**雙方**各一點本事、雙親各自
//! 把「我和另一半一起迎來了我們的孩子」記進心裡（含「一定會」→ 升為一生最重的永久精華記憶），
//! 世界動態牆也以「一家人」的口吻播報。家庭第一次在乙太方界的小社會裡真正成形。
//!
//! **與既有系統 razor-sharp 區隔**：不是 927 婚禮的重演（婚禮＝兩人**彼此**締結、立拱門；
//! 本刀＝夫妻**一起對第三者·孩子**的共同養育，是婚後家庭的下一拍）；也不改動出生系統的
//! 頻率／上限／選址／技能繼承機制本身（無婚配時完全走既有單親路徑，向後相容）——只是在
//! 「有夫妻可當共同父母」時，把出生從「一位居民的事」變成「一家人的事」。
//!
//! **純邏輯層**：配對挑選、台詞／記憶／Feed 文案全是確定性純函式、可窮舉測試；鎖／WS／IO
//! 由 `voxel_ws.rs` 的出生節拍包進既有短鎖循序呼叫。**零 LLM、零新持久化格式**（沿用既有
//! `voxel_weddings.jsonl` 讀、`roster` append）、零新協議欄位、零新美術、零前端改動、
//! FPS 零影響（出生本就低頻）。**零玩家輸入**（居民自發、無濫用面）。

/// 顯示名截斷上限（防超長顯示名在泡泡／Feed 破框，比照其他社交模組慣例）。
const NAME_MAX: usize = 12;

/// 截斷一個顯示名到上限長度（以字元計，中文安全）。
fn clip(name: &str) -> String {
    name.chars().take(NAME_MAX).collect()
}

/// 截斷；空名退回泛稱（記憶／Feed 永不出現空洞的「」）。
fn clip_or(name: &str, fallback: &str) -> String {
    let s = clip(name);
    if s.is_empty() {
        fallback.to_string()
    } else {
        s
    }
}

/// 從已婚配對（每筆都是 pop 內合法、互異的 index 對）中確定性挑一對當「共同父母」。
/// - `couples`：呼叫端已把婚書名映射成當前人口內的 index 對（不合法者已濾除）。
/// - `seed`：由呼叫端用時間等湊出，讓不同出生確定性地輪到不同夫妻。
///
/// 空 → `None`（呼叫端回退既有單親出生，向後相容）。此處仍再過濾一次
/// （`a < pop && b < pop && a != b`）以防呼叫端漏濾，純函式自保。
pub fn pick_married_couple(pop: usize, couples: &[(usize, usize)], seed: u64) -> Option<(usize, usize)> {
    let valid: Vec<(usize, usize)> =
        couples.iter().copied().filter(|&(a, b)| a < pop && b < pop && a != b).collect();
    if valid.is_empty() {
        return None;
    }
    let idx = (seed % valid.len() as u64) as usize;
    Some(valid[idx])
}

/// 出生泡泡（雙親版）：孩子一出生就報上自己是這對夫妻的孩子；若承繼了技能則點名是誰教的。
pub fn child_birth_say(child: &str, parent_a: &str, parent_b: &str, inherited: Option<&str>) -> String {
    let c = clip_or(child, "孩子");
    let a = clip_or(parent_a, "爸爸");
    let b = clip_or(parent_b, "媽媽");
    match inherited {
        Some(skill) => format!("我是{c}，{a}和{b}的孩子～{a}把「{}」教給了我！", clip(skill)),
        None => format!("我是{c}，{a}和{b}的孩子，剛在這片天地誕生，請多指教！"),
    }
}

/// 父母歡迎新生兒的泡泡（確定性三選一，父方 `pick=0`、母方 `pick=1`，讓兩人各說一句不同的）。
pub fn parent_welcome_say(child: &str, pick: usize) -> String {
    const LINES: [&str; 3] = [
        "歡迎來到世界，{c}！你是我們的寶貝。",
        "我的孩子{c}，往後有爸爸媽媽陪著你慢慢長大。",
        "{c}，這片天地從今天起因你更完整了。💕",
    ];
    LINES[pick % LINES.len()].replace("{c}", &clip_or(child, "孩子"))
}

/// 父母把「和另一半一起迎來孩子」記進心裡的一筆記憶（第一人稱、含「一定會」→ 被記憶系統
/// 判為永久精華 Promise 事實，成為為人父母這輩子最重的一筆回憶）。
pub fn family_memory_line(partner: &str, child: &str) -> String {
    format!(
        "今天，我和{}一起迎來了我們的孩子{}——我一定會用一生守護這個家。",
        clip_or(partner, "另一半"),
        clip_or(child, "孩子")
    )
}

/// 世界動態牆的家庭版播報：一對夫妻迎來新生命。
pub fn family_feed_line(parent_a: &str, parent_b: &str, child: &str) -> String {
    format!(
        "{}與{}一家，迎來了新生命{}",
        clip_or(parent_a, "一位居民"),
        clip_or(parent_b, "另一位居民"),
        clip_or(child, "孩子")
    )
}

/// 家庭出生的 Feed 分類標籤（與一般單親「新居民誕生」區隔，讓動態牆一眼看出「這是一家人的喜事」）。
pub const BIRTH_FAMILY_FEED_KIND: &str = "愛的結晶";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_couples_returns_none() {
        assert_eq!(pick_married_couple(5, &[], 0), None);
    }

    #[test]
    fn out_of_pop_couples_filtered_out() {
        // 兩對都有人超出當前人口（pop=3，合法 index 為 0..3）→ 全濾掉。
        let couples = [(0usize, 5usize), (4, 1)];
        assert_eq!(pick_married_couple(3, &couples, 0), None);
    }

    #[test]
    fn self_pair_filtered_out() {
        assert_eq!(pick_married_couple(4, &[(2, 2)], 7), None);
    }

    #[test]
    fn picks_only_valid_couple() {
        // 一對合法、一對越界 → 必回合法那對，且順序保留。
        let couples = [(0usize, 9usize), (1, 2)];
        assert_eq!(pick_married_couple(3, &couples, 0), Some((1, 2)));
        assert_eq!(pick_married_couple(3, &couples, 999), Some((1, 2)));
    }

    #[test]
    fn deterministic_same_seed_same_couple() {
        let couples = [(0usize, 1usize), (2, 3)];
        let a = pick_married_couple(4, &couples, 12345);
        let b = pick_married_couple(4, &couples, 12345);
        assert_eq!(a, b);
        assert!(a.is_some());
    }

    #[test]
    fn seed_distributes_across_couples() {
        let couples = [(0usize, 1usize), (2, 3)];
        // 偶數 seed → 第 0 對，奇數 seed → 第 1 對（len=2 取模）。
        assert_eq!(pick_married_couple(4, &couples, 0), Some((0, 1)));
        assert_eq!(pick_married_couple(4, &couples, 1), Some((2, 3)));
    }

    #[test]
    fn child_birth_say_names_both_parents() {
        let s = child_birth_say("小星", "露娜", "奧瑞", None);
        assert!(s.contains("小星"));
        assert!(s.contains("露娜"));
        assert!(s.contains("奧瑞"));
        assert!(!s.contains("「")); // 無承繼技能時不冒引號技能句
    }

    #[test]
    fn child_birth_say_mentions_inherited_skill() {
        let s = child_birth_say("小星", "露娜", "奧瑞", Some("釣魚"));
        assert!(s.contains("釣魚"));
        assert!(s.contains("露娜")); // 承自父方（第一位）
    }

    #[test]
    fn parent_welcome_rotates_and_embeds_child() {
        let a = parent_welcome_say("小星", 0);
        let b = parent_welcome_say("小星", 1);
        assert_ne!(a, b);
        assert!(a.contains("小星"));
        assert!(b.contains("小星"));
        // pick 超出長度會取模回捲。
        assert_eq!(parent_welcome_say("小星", 3), parent_welcome_say("小星", 0));
    }

    #[test]
    fn family_memory_is_promise_and_names_child() {
        let m = family_memory_line("奧瑞", "小星");
        assert!(m.contains("一定會")); // → 記憶系統升為永久精華
        assert!(m.contains("奧瑞"));
        assert!(m.contains("小星"));
    }

    #[test]
    fn family_feed_names_family_and_child() {
        let f = family_feed_line("露娜", "奧瑞", "小星");
        assert!(f.contains("露娜"));
        assert!(f.contains("奧瑞"));
        assert!(f.contains("小星"));
    }

    #[test]
    fn long_names_truncated_to_bound() {
        let long = "一二三四五六七八九十甲乙丙丁";
        let s = child_birth_say(long, long, long, Some(long));
        // 每處嵌名都被截到 NAME_MAX 字元，不會出現第 13 個字「丁」。
        assert!(!s.contains("丁"));
        let m = family_memory_line(long, long);
        assert!(!m.contains("丁"));
    }

    #[test]
    fn empty_names_fall_back_to_generic() {
        let m = family_memory_line("", "");
        assert!(m.contains("另一半"));
        assert!(m.contains("孩子"));
        let f = family_feed_line("", "", "");
        assert!(f.contains("一位居民"));
        assert!(f.contains("孩子"));
        // 出生泡泡空名也退泛稱。
        let s = child_birth_say("", "", "", None);
        assert!(s.contains("孩子"));
        assert!(s.contains("爸爸"));
        assert!(s.contains("媽媽"));
    }
}
