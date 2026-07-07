//! 乙太方界·長椅拌嘴/和好 v1（bench tiff）——長椅並坐閒聊（`voxel_bench_chat.rs`）讓交情到
//! 相識以上的居民偶爾並肩閒聊，但那始終是**只升不降**的單向暖流；情誼帳本（`voxel_bonds.rs`）
//! 的三段情誼從陌生走到老朋友，中間從沒有過一絲摩擦——兩位老朋友即使一路只有甜蜜互動，也不
//! 曾真正吵過一次架、鬧過一次彆扭再和好。本刀讓長椅第一次成為**摩擦與修復**都會發生的地方：
//! 交情已到老朋友的兩人，偶爾會因為一點小事在長椅上拌幾句嘴、心裡悶一陣子；下次再恰好碰上同
//! 一張長椅，彆扭就會化解——把事情說開、和好如初，交情反而更進一步。
//!
//! **設計依據**：`docs/PLAN_ETHERVOX.md` §4 居民↔居民關係——「熟識/幫過/**吵過**」；情誼帳本
//! 至今只實作了前兩者，「吵過」從未落地。此設計已在舊 2D 鄰里系統驗證過（ROADMAP 559
//! `resident_bonds.rs::begin_tiff`/`make_up`），本刀是同一套機制原創移植進 voxel 世界。
//!
//! **與既有元素 razor-sharp 區隔（非同軸重複）**：
//! - **長椅並坐閒聊（`voxel_bench_chat.rs`）**＝相識以上、單一情緒方向（暖）、每次都加溫；
//!   本刀＝**老朋友限定**、雙情緒方向（拌嘴＝冷／和好＝更暖），三者在同一個配對掃描裡互斥——
//!   同一次相遇只會發生其中一種，不會疊加。
//! - **戀人牽掛（852）／羈絆讚賞（875）**＝居民對玩家或對寵物的單向好感流露；本刀＝**居民↔居民**
//!   之間第一次出現負向情緒事件，是情誼帳本的新維度，不是又一種正向甜蜜互動。
//!
//! **純函式層**：情誼閘、觸發機率閘、拌嘴／和好台詞、記憶、Feed 皆為確定性純函式，零 LLM、零鎖、
//! 零 async、零 IO、可窮舉單元測試。配對／鎖／擲骰／持久化觸發、`sulking` 旗標讀寫全留在
//! `voxel_ws.rs`（沿用長椅並坐閒聊那條已驗證的居民配對快照＋鎖外事件佇列＋短鎖循序慣例，守
//! prod 死鎖鐵律）。
//!
//! **成本 / 濫用防護鐵律**：零 LLM（拌嘴／和好台詞／記憶／Feed 全為固定模板、確定性選句）、零
//! 新持久化格式（`sulking` 旗標純記憶體、重啟歸零，比照冷卻/心情等既有短命狀態慣例；情誼帳本
//! 的拜訪次數本身仍照舊走 `voxel_bonds.rs` 既有 jsonl 持久化）、零新協議欄位（只改居民 say／內部
//! 狀態，走既有泡泡廣播）、零新美術、零前端改動、FPS 零影響（沿用長椅並坐閒聊同一份配對掃描與
//! 冷卻節流，觸發機率遠低於閒聊，天然更稀少）。**永不回放記憶原文或玩家原話**——台詞全嵌居民
//! **顯示名**（本就出現在問候／動態牆），無注入／NSFW 面。

/// 這次長椅相遇的結果——與「長椅並坐閒聊」共用同一個配對掃描，互斥、每次相遇只擇一。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BenchOutcome {
    /// 一般並坐閒聊（`voxel_bench_chat.rs`）。
    Chat,
    /// 拌嘴：交情暫時冷一格，標記彆扭中。
    Tiff,
    /// 和好：解除彆扭、交情回暖一格。
    MakeUp,
}

/// 拌嘴觸發機率——比並坐閒聊（`CHAT_CHANCE` 0.45）稀少得多，是可遇不可求的小摩擦，
/// 不是每次老朋友相遇都吵。只在老朋友（`BondTier::Friend`）且未在彆扭中時才會擲這個骰。
pub const TIFF_CHANCE: f32 = 0.10;

/// 只有老朋友之間才可能拌嘴——相識以下交情還沒深到會為小事鬥氣（記憶驅動行為的閘）。
pub fn tier_allows_tiff(tier: crate::voxel_bonds::BondTier) -> bool {
    tier == crate::voxel_bonds::BondTier::Friend
}

/// 拌嘴閘：冷卻到期（`cooldown <= 0`）＋過機率門檻（`roll < TIFF_CHANCE`）→ 這一次相遇拌嘴。
/// 「老朋友」與「未在彆扭中」由呼叫端配對時判定，不進本函式（保持純粹好窮舉測）。
pub fn should_tiff(cooldown: f32, roll: f32) -> bool {
    cooldown <= 0.0 && roll < TIFF_CHANCE
}

/// 發起者起頭抱怨的拌嘴泡泡（點名對方，語氣悶但不刻薄——療癒向的小彆扭，不是惡言相向）。
/// 五句輪替，`pick` 由呼叫端用座標 bits 合成。超長名截斷不破泡泡框。
pub fn tiff_opener_line(other: &str, pick: usize) -> String {
    let name = clip_name(other);
    const TEMPLATES: [&str; 5] = [
        "{name}，你上次答應我的事，怎麼還沒做啊……",
        "哼，{name}，你剛剛是不是沒在聽我說話？",
        "{name}，你都沒發現我今天不太開心嗎？",
        "說真的{name}，你有時候真的很讓人生氣耶。",
        "{name}，我們是不是該好好談談？",
    ];
    TEMPLATES[pick % TEMPLATES.len()].replace("{name}", &name)
}

/// 被抱怨者不服氣的回嘴泡泡（點名發起者，一來一往，鬥嘴收在無傷大雅）。
pub fn tiff_reply_line(opener: &str, pick: usize) -> String {
    let name = clip_name(opener);
    const TEMPLATES: [&str; 5] = [
        "哼，{name}，我才想說你也有不對的地方吧！",
        "{name}，好啦好啦，是我的錯還不行嗎？",
        "誰叫{name}自己也沒說清楚嘛。",
        "{name}，講這什麼話，我可沒故意的。",
        "{name}，先冷靜一下啦，別這麼激動。",
    ];
    TEMPLATES[pick % TEMPLATES.len()].replace("{name}", &name)
}

/// 和好時發起者的開場泡泡（點名對方，主動放下彆扭）。
pub fn makeup_opener_line(other: &str, pick: usize) -> String {
    let name = clip_name(other);
    const TEMPLATES: [&str; 5] = [
        "{name}，上次的事，是我太衝動了，別放心上。",
        "欸{name}，還在生氣嗎？我們和好啦好不好？",
        "{name}，想了想，其實沒什麼大不了的事。",
        "{name}，別彆扭了，坐下來說開吧。",
        "{name}，老朋友哪有隔夜仇，來，握個手。",
    ];
    TEMPLATES[pick % TEMPLATES.len()].replace("{name}", &name)
}

/// 和好時被招呼者的回應泡泡（點名發起者，欣然接受、交情反而更親近）。
pub fn makeup_reply_line(opener: &str, pick: usize) -> String {
    let name = clip_name(opener);
    const TEMPLATES: [&str; 5] = [
        "早就沒事啦，{name}，我們還是好朋友！",
        "哈哈，{name}，我也正想找你和好呢。",
        "{name}，能說開真好，感覺更親近了。",
        "好啦{name}，這樣才對嘛，別再彆扭了。",
        "🤝 {name}，和好如初！",
    ];
    TEMPLATES[pick % TEMPLATES.len()].replace("{name}", &name)
}

/// 昇華成一筆「和某人在長椅上拌了幾句嘴」的摩擦記憶（點名對方，不含換行）。
/// 空名退泛稱，不留突兀的「和，」。
pub fn tiff_memory_line(other: &str) -> String {
    let name = clip_name(other);
    let who = if name.trim().is_empty() { "一位朋友" } else { &name };
    format!("在長椅上跟{who}為了點小事拌了幾句嘴，心裡有點悶。").replace('\n', " ")
}

/// 昇華成一筆「和某人在長椅上和好」的修復記憶（點名對方，不含換行）。
pub fn makeup_memory_line(other: &str) -> String {
    let name = clip_name(other);
    let who = if name.trim().is_empty() { "一位朋友" } else { &name };
    format!("跟{who}在長椅上把上次的彆扭說開了，和好後感覺更親近了。").replace('\n', " ")
}

/// 動態 Feed：拌嘴（訪客回來能讀到誰跟誰鬧了小彆扭）。任一空名退泛稱。
pub fn tiff_feed_line(a: &str, b: &str) -> String {
    let na = if a.trim().is_empty() { "一位居民" } else { a };
    let nb = if b.trim().is_empty() { "一位朋友" } else { b };
    format!("{na}和{nb}在長椅上拌了幾句嘴，看起來鬧了點小彆扭。")
}

/// 動態 Feed：和好（訪客回來能讀到誰跟誰和好如初）。任一空名退泛稱。
pub fn makeup_feed_line(a: &str, b: &str) -> String {
    let na = if a.trim().is_empty() { "一位居民" } else { a };
    let nb = if b.trim().is_empty() { "一位朋友" } else { b };
    format!("🤝 {na}和{nb}在長椅上把心結解開，和好如初了！")
}

/// 居民名截斷到 8 字（中文安全，避免超長名撐破泡泡框）。
fn clip_name(name: &str) -> String {
    name.chars().take(8).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voxel_bonds::BondTier;

    #[test]
    fn tier_gate_only_friend() {
        assert!(!tier_allows_tiff(BondTier::Stranger));
        assert!(!tier_allows_tiff(BondTier::Acquaintance));
        assert!(tier_allows_tiff(BondTier::Friend));
    }

    #[test]
    fn should_tiff_two_gates_and_boundary() {
        assert!(should_tiff(0.0, 0.05));
        // 冷卻未到 → 否。
        assert!(!should_tiff(5.0, 0.05));
        // 邊界：roll == chance 不觸發（嚴格小於）。
        assert!(!should_tiff(0.0, TIFF_CHANCE));
        // 骰過頭 → 否。
        assert!(!should_tiff(0.0, 0.99));
    }

    #[test]
    fn tiff_chance_is_rarer_than_chat() {
        // 拌嘴要比並坐閒聊稀少得多，不然「老朋友」反而比「相識」更容易吵架，違反直覺。
        assert!(TIFF_CHANCE < crate::voxel_bench_chat::CHAT_CHANCE);
    }

    #[test]
    fn tiff_opener_and_reply_rotate_embed_name_in_frame() {
        let o0 = tiff_opener_line("諾娃", 0);
        let o1 = tiff_opener_line("諾娃", 1);
        assert!(o0.contains("諾娃"));
        assert_ne!(o0, o1, "不同 pick 應輪替出不同句子");
        let r0 = tiff_reply_line("露娜", 0);
        assert!(r0.contains("露娜"));
    }

    #[test]
    fn makeup_opener_and_reply_rotate_embed_name_in_frame() {
        let o0 = makeup_opener_line("諾娃", 0);
        let o1 = makeup_opener_line("諾娃", 1);
        assert!(o0.contains("諾娃"));
        assert_ne!(o0, o1, "不同 pick 應輪替出不同句子");
        let r0 = makeup_reply_line("露娜", 0);
        assert!(r0.contains("露娜"));
    }

    #[test]
    fn tiff_and_makeup_lines_all_pick_values_stay_in_frame() {
        for pick in 0..10 {
            assert!(tiff_opener_line("諾娃", pick).chars().count() <= 40);
            assert!(tiff_reply_line("諾娃", pick).chars().count() <= 40);
            assert!(makeup_opener_line("諾娃", pick).chars().count() <= 40);
            assert!(makeup_reply_line("諾娃", pick).chars().count() <= 40);
        }
    }

    #[test]
    fn tiff_memory_line_names_other_and_is_single_line() {
        let s = tiff_memory_line("諾娃");
        assert!(s.contains("諾娃"));
        assert!(!s.contains('\n'));
    }

    #[test]
    fn tiff_memory_line_empty_name_falls_back_to_generic() {
        let s = tiff_memory_line("");
        assert!(s.contains("一位朋友"));
    }

    #[test]
    fn makeup_memory_line_names_other_and_is_single_line() {
        let s = makeup_memory_line("露娜");
        assert!(s.contains("露娜"));
        assert!(!s.contains('\n'));
    }

    #[test]
    fn tiff_feed_line_names_both_empty_falls_back() {
        let s = tiff_feed_line("露娜", "諾娃");
        assert!(s.contains("露娜") && s.contains("諾娃"));
        let s2 = tiff_feed_line("", "");
        assert!(s2.contains("一位居民") && s2.contains("一位朋友"));
    }

    #[test]
    fn makeup_feed_line_names_both_empty_falls_back() {
        let s = makeup_feed_line("露娜", "諾娃");
        assert!(s.contains("露娜") && s.contains("諾娃"));
        let s2 = makeup_feed_line("", "");
        assert!(s2.contains("一位居民") && s2.contains("一位朋友"));
    }

    #[test]
    fn clip_name_truncates_long_names() {
        let long = "阿".repeat(20);
        let s = tiff_opener_line(&long, 0);
        // 截斷到 8 字後嵌入模板，句子本身仍在合理長度內（不含模板固定字）。
        assert!(s.chars().filter(|&c| c == '阿').count() <= 8);
    }
}
