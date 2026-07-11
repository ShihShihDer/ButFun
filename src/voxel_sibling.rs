//! 乙太方界·手足相伴——同一對父母的孩子，一出生就認得彼此是手足 v1（自主提案切片，ROADMAP 941）。
//!
//! 家庭這條線一路蓋到 928「愛的結晶」：一對成婚夫妻共同迎來孩子，親子關係第一次在小社會裡成形。
//! 但盤點下來，家庭至今只長出了**縱向**的一環——「父母↔孩子」；當同一位父母日後再迎來下一個
//! 孩子，先來的孩子與新生兒之間，卻始終**毫無瓜葛**。世界裡會有兩、三個明明同出一源的居民各自
//! 閒晃，誰也不知道彼此是手足。家庭最基本的**橫向**關係——「手足」——一直缺席。
//!
//! 本切片補上那一環：**新生兒誕生時，若牠的父母此前已迎來過孩子（名冊裡查得到同一位父母的
//! 既有孩子），這個新生兒就與那些哥哥姐姐相認為手足**——新生兒把「我不是一個人來到這世界」
//! 記進心裡（含「一定會」→ 升為一生最重的永久精華記憶），既有的哥哥姐姐各記一筆「我當手足裡
//! 的兄姊了」，其中一位還會當場轉頭迎接這個小的，世界動態牆也以「手足」的口吻播報。血脈相連的
//! 手足情，第一次在乙太方界的小社會裡真正成形。
//!
//! **與既有系統 razor-sharp 區隔**：
//! - **928 愛的結晶（親子）**＝夫妻**一起對孩子**的縱向養育；本刀＝孩子**與孩子之間**的橫向手足，
//!   關係對子（父母↔子 vs 子↔子）與台詞／記憶全然不同軸。
//! - **交情／摯友（bonds）**＝居民透過互動**選擇**、經營出來的友誼；手足是**與生俱來、無從選擇**的
//!   血親，是categorically 不同的關係——你不會「結交」自己的手足。
//! - **誕辰紀念（birthday）**＝每年回望自己的年歲（時間刻度）；本刀＝出生那一刻與手足相認（關係刻度）。
//!
//! **純邏輯層**：手足篩選、台詞／記憶／Feed 文案全是確定性純函式、可窮舉測試；名冊讀取（IO）與
//! 記憶／residents 短鎖由 `voxel_ws.rs` 的出生節拍循序呼叫（每把鎖短取即釋、不巢狀，守 prod 死鎖鐵律）。
//! **零 LLM、零新持久化格式**（沿用既有名冊 `load_roster` 讀、記憶 append）、零新協議欄位、零新美術、
//! 零前端改動、FPS 零影響（出生本就低頻）。**零玩家輸入**（居民自發、無濫用面）。

/// 顯示名截斷上限（防超長顯示名在泡泡／Feed 破框，比照 `voxel_family` 慣例）。
const NAME_MAX: usize = 12;

/// 截斷一個顯示名到上限長度（以字元計，中文安全）。
fn clip(name: &str) -> String {
    name.chars().take(NAME_MAX).collect()
}

/// 截斷；空名退回泛稱（記憶／Feed 永不出現空洞的名字）。
fn clip_or(name: &str, fallback: &str) -> String {
    let s = clip(name);
    if s.is_empty() {
        fallback.to_string()
    } else {
        s
    }
}

/// 從名冊列（`(resident_id, name, parent_id)`）中，篩出新生兒的**既有手足**：
/// 與新生兒同一位父母（`parent_id` 相符）、且不是新生兒自己（`resident_id != newborn_id`）。
///
/// 回傳 `(resident_id, name)` 對，順序保留名冊順序（＝出生先後，先出生的哥哥姐姐排前面）。
/// 呼叫端在名冊已 append 新生兒之後呼叫也安全——排除自身那條靠 `newborn_id` 過濾。
pub fn filter_older_siblings(
    rows: &[(String, String, String)],
    parent_id: &str,
    newborn_id: &str,
) -> Vec<(String, String)> {
    rows.iter()
        .filter(|(rid, _, pid)| pid == parent_id && rid != newborn_id)
        .map(|(rid, name, _)| (rid.clone(), name.clone()))
        .collect()
}

/// 新生兒與手足相認的出生泡泡：一出生就報上自己有手足陪著。
/// - 恰一位手足 → 點名那位；多位 → 報數量，凸顯「這個家好熱鬧」。
pub fn newborn_sibling_say(newborn: &str, sibling_names: &[String]) -> String {
    let n = clip_or(newborn, "我");
    match sibling_names.len() {
        0 => format!("我是{n}，剛來到這片天地～"), // 呼叫端理應不會在無手足時呼叫，純函式自保
        1 => format!("我是{n}，一出生就有手足{}陪著我，好幸福！", clip_or(&sibling_names[0], "哥哥姐姐")),
        k => format!("我是{n}，我有{k}位手足，這個家好熱鬧！"),
    }
}

/// 既有哥哥姐姐迎接新生手足的泡泡（確定性三選一，讓不同手足各說一句不同的）。
pub fn elder_sibling_say(newborn: &str, pick: usize) -> String {
    const LINES: [&str; 3] = [
        "家裡多了個小家伙——{n}，往後哥哥姐姐罩著你！",
        "{n}，從今天起你就是我的手足了，我們一起長大。",
        "歡迎你，{n}！我會好好陪著你這個小的。",
    ];
    LINES[pick % LINES.len()].replace("{n}", &clip_or(newborn, "小家伙"))
}

/// 新生兒把「我不是一個人來到這世界」記進心裡的一筆記憶（第一人稱、含「一定會」→ 被記憶系統
/// 判為永久精華事實，成為這輩子最重的一筆手足記憶）。
pub fn newborn_sibling_memory_line(sibling_names: &[String]) -> String {
    match sibling_names.len() {
        0 | 1 => {
            let s = sibling_names.first().map(|s| clip_or(s, "手足")).unwrap_or_else(|| "手足".to_string());
            format!("我不是一個人來到這片天地——我有手足{s}，往後我們一定會互相扶持、一起長大。")
        }
        k => format!("我不是一個人來到這片天地——我有{k}位手足，往後我們一定會互相扶持、一起長大。"),
    }
}

/// 既有哥哥姐姐把「我當兄姊了」記進心裡的一筆記憶（第一人稱、含「一定會」→ 永久精華）。
pub fn elder_sibling_memory_line(newborn: &str) -> String {
    format!(
        "家裡多了個手足{}，我當兄姊了——我一定會像哥哥姐姐一樣，好好照顧這個小的。",
        clip_or(newborn, "小家伙")
    )
}

/// 世界動態牆的手足版播報：某家又添了新成員，先來的孩子多了個手足。
pub fn sibling_feed_line(parent_name: &str, newborn: &str, sibling_count: usize) -> String {
    let p = clip_or(parent_name, "一位居民");
    let n = clip_or(newborn, "新生兒");
    if sibling_count <= 1 {
        format!("{p}家又添了新成員{n}，先來的孩子多了個手足")
    } else {
        format!("{p}家又添了新成員{n}，{sibling_count}個孩子從此手足相伴")
    }
}

/// 手足相認的 Feed 分類標籤（與 928「愛的結晶」的親子喜事區隔，讓動態牆一眼看出「這是手足的一刻」）。
pub const SIBLING_FEED_KIND: &str = "手足相伴";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voxel_memory::{classify_importance, Importance};

    fn rows(items: &[(&str, &str, &str)]) -> Vec<(String, String, String)> {
        items.iter().map(|(a, b, c)| (a.to_string(), b.to_string(), c.to_string())).collect()
    }

    #[test]
    fn filter_picks_only_same_parent_excluding_self() {
        let r = rows(&[
            ("vox_res_4", "小星", "vox_res_0"), // 露娜(0)的長子
            ("vox_res_5", "小月", "vox_res_0"), // 露娜(0)的次子＝新生兒
            ("vox_res_6", "小河", "vox_res_1"), // 別家的孩子
        ]);
        let sibs = filter_older_siblings(&r, "vox_res_0", "vox_res_5");
        assert_eq!(sibs.len(), 1);
        assert_eq!(sibs[0].0, "vox_res_4");
        assert_eq!(sibs[0].1, "小星");
    }

    #[test]
    fn filter_first_born_has_no_older_sibling() {
        // 長子出生時，名冊裡同父母只有牠自己 → 無既有手足。
        let r = rows(&[("vox_res_4", "小星", "vox_res_0")]);
        assert!(filter_older_siblings(&r, "vox_res_0", "vox_res_4").is_empty());
    }

    #[test]
    fn filter_preserves_birth_order_for_many_siblings() {
        let r = rows(&[
            ("vox_res_4", "老大", "vox_res_0"),
            ("vox_res_5", "老二", "vox_res_0"),
            ("vox_res_7", "老么", "vox_res_0"), // 新生兒
            ("vox_res_6", "路人", "vox_res_2"),
        ]);
        let sibs = filter_older_siblings(&r, "vox_res_0", "vox_res_7");
        assert_eq!(sibs.iter().map(|(_, n)| n.as_str()).collect::<Vec<_>>(), vec!["老大", "老二"]);
    }

    #[test]
    fn newborn_say_names_single_sibling_but_counts_many() {
        let one = newborn_sibling_say("小月", &["小星".to_string()]);
        assert!(one.contains("小月"));
        assert!(one.contains("小星"));
        let many = newborn_sibling_say("小么", &["a".into(), "b".into(), "c".into()]);
        assert!(many.contains("3位手足"));
    }

    #[test]
    fn elder_say_rotates_and_embeds_newborn() {
        let a = elder_sibling_say("小月", 0);
        let b = elder_sibling_say("小月", 1);
        assert_ne!(a, b);
        assert!(a.contains("小月"));
        assert!(b.contains("小月"));
        // pick 超長取模回捲。
        assert_eq!(elder_sibling_say("小月", 3), elder_sibling_say("小月", 0));
    }

    #[test]
    fn newborn_memory_is_persistent_and_mentions_sibling() {
        let m = newborn_sibling_memory_line(&["小星".to_string()]);
        assert!(m.contains("小星"));
        assert!(m.contains("一定會"));
        // 真跑分類器：這筆手足記憶確實被判為永久精華（不是短期記憶，滿容量不會被淘汰）。
        assert!(matches!(classify_importance(&m), Importance::Persistent(_)));
    }

    #[test]
    fn newborn_memory_many_siblings_reports_count_and_persistent() {
        let m = newborn_sibling_memory_line(&["a".into(), "b".into()]);
        assert!(m.contains("2位手足"));
        assert!(matches!(classify_importance(&m), Importance::Persistent(_)));
    }

    #[test]
    fn elder_memory_is_persistent_and_names_newborn() {
        let m = elder_sibling_memory_line("小月");
        assert!(m.contains("小月"));
        assert!(m.contains("一定會"));
        assert!(matches!(classify_importance(&m), Importance::Persistent(_)));
    }

    #[test]
    fn feed_line_names_family_and_newborn_singular_and_plural() {
        let one = sibling_feed_line("露娜", "小月", 1);
        assert!(one.contains("露娜"));
        assert!(one.contains("小月"));
        assert!(one.contains("手足"));
        let many = sibling_feed_line("露娜", "小么", 3);
        assert!(many.contains("3個孩子"));
    }

    #[test]
    fn long_names_truncated_to_bound() {
        let long = "一二三四五六七八九十甲乙丙丁";
        let s = newborn_sibling_say(long, &[long.to_string()]);
        assert!(!s.contains("丁")); // 第 13 個字被截掉
        let m = elder_sibling_memory_line(long);
        assert!(!m.contains("丁"));
        let f = sibling_feed_line(long, long, 1);
        assert!(!f.contains("丁"));
    }

    #[test]
    fn empty_names_fall_back_to_generic() {
        let s = newborn_sibling_say("", &["".to_string()]);
        assert!(s.contains("哥哥姐姐"));
        let m = newborn_sibling_memory_line(&[]);
        assert!(m.contains("手足"));
        assert!(matches!(classify_importance(&m), Importance::Persistent(_)));
        let e = elder_sibling_memory_line("");
        assert!(e.contains("小家伙"));
        let f = sibling_feed_line("", "", 1);
        assert!(f.contains("一位居民"));
        assert!(f.contains("新生兒"));
    }
}
