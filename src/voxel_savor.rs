//! 乙太方界·你送的食物，她會細細享用 v1（食物餽贈的延遲享用）。
//!
//! **核心信念**：「你的互動有後果」（PLAN_ETHERVOX 路線圖 item 3·記憶→行為）。此前玩家把
//! 一份食物送給居民，居民即時道一句謝、食物就此**憑空消失**——那份心意只在送出的那一秒閃過，
//! 之後沒有任何痕跡。這一刀讓食物餽贈長出「第二拍」：居民收下你的食物後不立刻吃掉，而是**捧著**，
//! 稍後在一個閒下來的安靜片刻**真的細細享用**那份心意——冒出一句滿足的暖泡泡、在城鎮動態牆留一筆
//! 「吃了你送的Ｘ，暖上心頭」，而這份滋養會**重新點亮牠的心情**（沿用既有 `mood_boost`）。
//!
//! **為什麼不只是裝飾**：心情在乙太方界是**驅動行為的真狀態**（Lonely 會尋伴、心情層級改變自語/
//! 社交傾向）——「吃飽了、暖起來」把送禮那一刻的好心情延續到更晚、甚至在它快消退時再拉回一格，
//! 是實打實的行為後果，不是純美術。玩家每餵一次食物，稍後都會**親眼看到**居民享用、並在動態牆
//! 讀到那一幕，餵食第一次有了「被好好享用」的溫暖回響。
//!
//! **成本鐵律**：純規則式（台詞確定性挑選、零 LLM、零 IO、零鎖、零 async），可單元測試。
//! 是否為「可享用的食物」直接**沿用** [`crate::voxel_gift::is_food_gift`]（不重複一份食物清單，
//! 單一真相來源）。連線 / 鎖 / 心情 / Feed 觸發全留在 `voxel_ws.rs`（沿用撲空感應 763 的短鎖循序
//! 慣例，守 prod 死鎖鐵律）。**隱私/濫用防護**：送禮者名字由玩家提供，寫進泡泡/動態牆前一律
//! 截斷長度（見 [`clamp_giver`]），Feed 落地再經 `voxel_feed` sanitize；不觸發 LLM、不開對外端點。
//!
//! 這裡只放確定性純邏輯；狀態欄位與觸發時機都在 `voxel_ws.rs`。不抄外部碼；繁中註解。

/// 收下食物到「真的享用」之間的延遲秒數。
///
/// 刻意不即時吃：讓享用發生在稍後一個閒下來的安靜片刻（居民正忙時會一直等到閒下來才享用），
/// 這樣「送禮的暖」與「享用的暖」錯開成兩拍、也讓 mood 補助在稍晚再被拉起一次、延續得更久。
pub const SAVOR_DELAY_SECS: f32 = 25.0;

/// 送禮者名字寫進面向玩家字串前的長度上限（字元數）——防超長名字洗版泡泡/動態牆。
const GIVER_MAX_CHARS: usize = 16;

/// 截斷送禮者名字到安全長度（濫用防護：玩家自報名字不可無限長）。
fn clamp_giver(giver: &str) -> String {
    giver.chars().take(GIVER_MAX_CHARS).collect()
}

/// 居民享用你送的食物時冒的暖泡泡（第一人稱、滿足、點名食物與送禮者）。
///
/// `food`＝食物顯示名（如「烤魚」）、`giver`＝送禮玩家名、`pick`＝呼叫端給的挑選數（取真隨機/座標雜湊）。
/// 確定性：同輸入永遠同輸出，好單元測試。
pub fn savor_bubble_line(food: &str, giver: &str, pick: usize) -> String {
    let g = clamp_giver(giver);
    let variants: [&str; 4] = [
        "細細品嚐著{giver}送的{food}，暖意一路暖到心底……真好。",
        "{food}真好吃，謝謝你{giver}，我會記得這份暖的。",
        "嗯——{giver}送的{food}，我留到現在才捨得吃，果然值得。",
        "一口一口享用著{giver}的{food}，這一刻，覺得世界溫柔了起來。",
    ];
    variants[pick % variants.len()]
        .replace("{giver}", &g)
        .replace("{food}", food)
}

/// 居民享用食物後在城鎮動態牆留下的一筆（給不在線上的玩家回來也讀得到這一幕）。
///
/// `resident`＝居民名、`giver`＝送禮玩家名、`food`＝食物名。
pub fn savor_feed_line(resident: &str, giver: &str, food: &str) -> String {
    let g = clamp_giver(giver);
    format!("{resident}細細享用了{g}送的{food}，暖上心頭～")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_is_positive() {
        assert!(SAVOR_DELAY_SECS > 0.0, "享用延遲必須正值（稍後才吃）");
    }

    #[test]
    fn bubble_mentions_food_and_giver() {
        for pick in 0..8 {
            let line = savor_bubble_line("烤魚", "阿明", pick);
            assert!(line.contains("烤魚"), "泡泡要點名食物：{line}");
            assert!(line.contains("阿明"), "泡泡要點名送禮者：{line}");
            assert!(!line.contains("{giver}"), "佔位符要全部被取代：{line}");
            assert!(!line.contains("{food}"), "佔位符要全部被取代：{line}");
        }
    }

    #[test]
    fn bubble_is_deterministic() {
        assert_eq!(
            savor_bubble_line("麵包", "小美", 3),
            savor_bubble_line("麵包", "小美", 3),
            "同輸入應同輸出"
        );
    }

    #[test]
    fn bubble_pick_wraps() {
        // pick 遠大於變體數也不 panic、且與模數等價。
        assert_eq!(
            savor_bubble_line("胡蘿蔔", "客", 4),
            savor_bubble_line("胡蘿蔔", "客", 0),
        );
    }

    #[test]
    fn feed_mentions_all_three() {
        let f = savor_feed_line("諾娃", "阿明", "烤地薯");
        assert!(f.contains("諾娃"));
        assert!(f.contains("阿明"));
        assert!(f.contains("烤地薯"));
    }

    #[test]
    fn long_giver_name_is_clamped() {
        let long = "超長的名字".repeat(20); // 100 字元
        let line = savor_bubble_line("烤魚", &long, 0);
        // 泡泡不應把整串超長名字倒進去（截斷到 GIVER_MAX_CHARS）。
        let giver_in_line = "超長的名字".repeat(20);
        assert!(!line.contains(&giver_in_line), "超長送禮者名字要被截斷");
        let feed = savor_feed_line("露娜", &long, "麵包");
        assert!(!feed.contains(&giver_in_line), "Feed 也要截斷超長名字");
    }
}
