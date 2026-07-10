//! 乙太方界·寵愛你的夥伴 v1（自主提案切片，ROADMAP 899）——羈絆線的日常收尾。
//!
//! **真缺口**：馴養的羈絆線至今疊了「餵食馴服（850/870）→ 跟隨（851）→ 取名（895）→
//! 安置／召回（898）」四刀，卻在「命令」那裡戛然而止——你把小夥伴取了名、叫得動牠跟上或
//! 待命，但**日常裡沒有任何一種「疼牠」的方式**。更刺眼的是：馴服之後再拿胡蘿蔔／種子對著
//! 牠，後端只回一句冷冰冰的「牠已經不怕你了，不用再餵」把你擋回去——這條羈絆線在馴服那一刻
//! 就對「餵食」這個最自然的親密動作**關上了門**。一隻你天天帶在身邊、取了名的小夥伴，和一顆
//! 只會跟著你走的移動方塊，情感上其實沒差多少。
//!
//! **本刀補的**：把「對已馴服的小夥伴餵食」從一句拒絕，變成一次**寵愛**——遞上一份零食
//! （野兔一根胡蘿蔔／雞一把種子），牠會蹭你、繞著你蹦跳、心滿意足地咕咕，頭頂浮起一串愛心。
//! 沿用玩家早已熟悉的「手持食物 + 準心對準動物」同一套手勢（零新協議請求、零新按鈕），只是把
//! 馴服後的死路，接成了羈絆線的日常暖收尾。
//!
//! **與馴服（850/870）razor-sharp 區隔**：馴服是**一次性、改變狀態**（`tamed=false→true`）；
//! 本刀是**已馴服後、可重複**的純情感回饋（不改任何持久狀態，只花一份零食換一次撒嬌）。
//! 與取名（895，署名）／安置召回（898，下指令）也刻意相異：那兩者是「標記」與「命令」，
//! 本刀是「疼愛」——羈絆的溫度，不是羈絆的功能。
//!
//! **純邏輯層**：確定性挑句、零 LLM、零鎖、零 IO；鎖／背包消耗／廣播全在 `voxel_ws.rs`。
//! 成本鐵律：零 LLM、零 migration、零新協議請求欄位（沿用既有 `feed_wildlife`/`feed_chicken`
//! 請求；只新增一則 `pet_treat_ok` 回應）。繁中註解；面向玩家字串集中此處便於日後在地化。

use crate::voxel_wildlife::TAME_REACH;

/// 一句撒嬌回饋的字元上限（泡泡框友善；模板本身刻意寫短，含一般長度的寵物名仍不超）。
pub const TREAT_LINE_MAX_CHARS: usize = 42;

/// 已馴服的小夥伴是否在「逗弄得到」的近身範圍內——沿用馴服的 [`TAME_REACH`]，
/// 單一事實來源：要疼牠得跟餵牠馴服時一樣湊到牠面前。純距離判定，可測。
pub fn in_treat_reach(dist_sq: f32) -> bool {
    dist_sq < TAME_REACH * TAME_REACH
}

/// 已馴服的小夥伴收到零食時的撒嬌回饋句（確定性挑選，零 LLM）。
///
/// - `is_rabbit`：`true`＝野兔、`false`＝雞——兩種動物的可愛姿態語氣略異。
/// - `pet_name`：玩家為牠取的名字（895）；`None` 或空白時以「小夥伴」泛稱，讓沒取名的也讀得順。
/// - `pick`：輪替索引（呼叫端給隨機數，函式內 `% 池長` 收斂），確定性可測。
pub fn treat_line(is_rabbit: bool, pet_name: Option<&str>, pick: usize) -> String {
    let name = pet_name
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("小夥伴");
    let pool: &[&str] = if is_rabbit {
        &[
            "🥕 {name} 蹭了蹭你的手心，眼睛瞇成一條線～",
            "🥕 {name} 開心地啃著零食，長耳朵一抖一抖。",
            "🥕 {name} 繞著你蹦跳了兩圈，尾巴翹得老高。",
            "🥕 {name} 把小臉埋進你掌心，滿足地哼了一聲。",
        ]
    } else {
        &[
            "🌾 {name} 咕咕叫著啄食，親暱地靠向你。",
            "🌾 {name} 拍了拍翅膀，繞著你的腳邊打轉。",
            "🌾 {name} 心滿意足地咕了兩聲，蹭了蹭你。",
            "🌾 {name} 歪著頭望你，眼裡閃著信賴的光。",
        ]
    };
    pool[pick % pool.len()].replace("{name}", name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_treat_reach_matches_tame_reach() {
        // 恰在 TAME_REACH 邊界外不算、內側算——與馴服近身距離同一把尺。
        let edge = TAME_REACH * TAME_REACH;
        assert!(in_treat_reach(edge - 0.01), "略近於 TAME_REACH 應逗得到");
        assert!(!in_treat_reach(edge + 0.01), "略遠於 TAME_REACH 不該逗得到");
        assert!(in_treat_reach(0.0), "貼著牠當然逗得到");
    }

    #[test]
    fn treat_line_is_deterministic_and_cycles() {
        // 同 pick 同輸出；pick 循環回池頭（確定性，便於測試與重放）。
        for &is_rabbit in &[true, false] {
            let a = treat_line(is_rabbit, Some("小星"), 0);
            let b = treat_line(is_rabbit, Some("小星"), 0);
            assert_eq!(a, b, "同輸入必同輸出");
            let wrapped = treat_line(is_rabbit, Some("小星"), 4);
            let head = treat_line(is_rabbit, Some("小星"), 0);
            assert_eq!(wrapped, head, "pick=4 應循環回池頭（池長 4）");
        }
    }

    #[test]
    fn treat_line_injects_pet_name() {
        // 有名字就叫名字；名字一定出現在回饋句裡（羈絆的署名被喊出來）。
        let s = treat_line(true, Some("雪球"), 1);
        assert!(s.contains("雪球"), "撒嬌句該喊出寵物名：{s}");
        assert!(!s.contains("{name}"), "佔位符必須被替換乾淨：{s}");
    }

    #[test]
    fn treat_line_falls_back_when_unnamed() {
        // 沒取名（None）或空白名 → 用「小夥伴」泛稱，句子仍讀得順、不留佔位符。
        for pet in [None, Some(""), Some("   ")] {
            let s = treat_line(false, pet, 2);
            assert!(s.contains("小夥伴"), "未命名應以小夥伴泛稱：{s}");
            assert!(!s.contains("{name}"), "佔位符必須被替換乾淨：{s}");
        }
    }

    #[test]
    fn treat_line_stays_within_bubble_cap() {
        // 一般長度的名字下，所有回饋句都在泡泡上限內（不撐爆對話泡泡）。
        for &is_rabbit in &[true, false] {
            for pick in 0..8 {
                let s = treat_line(is_rabbit, Some("小星"), pick);
                assert!(
                    s.chars().count() <= TREAT_LINE_MAX_CHARS,
                    "撒嬌句超出泡泡上限（{} 字）：{s}",
                    s.chars().count()
                );
                assert!(!s.is_empty(), "撒嬌句不該為空");
            }
        }
    }
}
