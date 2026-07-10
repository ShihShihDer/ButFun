//! 乙太方界·流星許願 v1（voxel-meteor，
//! PLAN_ETHERVOX「記憶／渴望驅動行為」＋§5「日記／生命故事」，ROADMAP 904）。
//!
//! **真缺口**：繁星夜空 v1（783，`voxel_stargaze`）讓夜空第一次掛滿繁星、升起明月，雨後彩虹
//! （780）讓白天的天空有了「非晴即雨」以外的驚喜——但**夜空至今永遠只是靜止的一片深藍**：
//! 星星不動、月亮不語，夜裡什麼都不會「發生」。世界另一頭，居民心裡各自懷著一個**渴望**
//! （`voxel_desires`：露娜想要一個家、賽勒想要玻璃……由你隨口的一句話種下、持久記著、驅動
//! 行為）——可是這份渴望至今只在對話與蓋家時浮現，**夜裡抬頭望天的那種「對著什麼許個願」的
//! 私密時刻，從來沒有過**。夜空缺一個會發生的事件，渴望缺一個被輕聲說出口的出口。
//!
//! **本刀**：把「夜空」接上「居民的渴望」——夜裡偶爾一顆**流星**劃過天際（前端渲染一道
//! 短促的光痕，伺服器隨快照廣播 `meteor:bool`，比照彩虹 780 的「伺服器偵測事件→快照 bool→
//! 前端純視覺」慣例）。流星劃過那一刻，附近**醒著**的居民會抬頭望見而**許下一個願**——而最
//! 動人的一拍是：若這位居民**心裡正懷著一個渴望**（`DesireStore` 有當前心願），牠許的**就是
//! 那個藏了好久的願**（「流星啊，願我真能有個家……」）；沒有心願的居民則許一個泛泛的祈願。
//! 你隨口說過、被牠記著的那句話，第一次不只換來蓋家的行動，還在某個流星夜被牠對著天上輕聲
//! 說了出來。渴望第一次有了「許願」這個私密而溫柔的出口——這正是 PLAN_ETHERVOX 北極星
//! 「記憶／渴望驅動行為、不只用來聊天」與 §5「窺見沒說出口的內心」的交會。
//!
//! **與既有元素 razor-sharp 區隔（非同軸重複）**：
//! - **雨後彩虹（780）**＝白天、天氣事件、居民**齊聲歡呼**（純情緒反應、與記憶無關）；本刀
//!   ＝**夜裡**、天象事件、居民**各自許願**且**願的內容由自己的渴望決定**（記憶驅動）——
//!   時段（晝／夜）、反應（歡呼／許願）、是否吃記憶（否／是）三者皆不同。
//! - **繁星共賞（783）**＝記得你愛看星星的居民**邀你一起看**（對象是你、動作是同賞）；本刀
//!   ＝居民**對流星許自己的願**（對象是天上、動作是許願、內容是自己的渴望）——依附對象與
//!   行為意圖截然不同。
//! - **念頭播種／許願→蓋家（desires→prayers）**＝把渴望**化為行動**（採方塊蓋屋）；本刀
//!   ＝把渴望**輕聲說出口**（對流星許願），是同一份渴望的「內心獨白」面，不觸發任何建造。
//!
//! **純邏輯層**：本檔全是零 IO／零鎖／零 LLM／零 async 的確定性純函式（夜間判定、流星觸發
//! 擲骰、流星可見 tick 推進、心願文字清洗、許願／動態牆文案），可獨立窮舉單元測試。流星狀態機
//! （擲骰＋可見 tick）與居民反應（讀 `DesireStore` 快照→設 say）全留在 `voxel_ws.rs`，沿用
//! 彩虹 780 的短鎖循序＋鎖外落地慣例，守 prod 死鎖鐵律。
//!
//! **成本／安全紀律**：零 LLM（觸發、許願台詞皆確定性）、零 migration（流星可見 tick 是純
//! 記憶體暫態、重啟歸零，比照彩虹 `rainbow_ticks` 慣例，不新增任何持久欄位）、零協議破壞
//! （快照只**新增**一個 `meteor:bool`，前端讀不到就當沒有、向後相容）、零新美術（流星＝前端
//! 程序生成的一道加法混合光痕，單一可重用物件、平時隱藏零成本、守 FPS 鐵律）。
//! **濫用防護**：不收玩家輸入、不觸發 LLM、不開對外端點、不動帳號權限——流星何時劃過純由
//! 伺服器內部低機率擲骰決定，玩家無法自報或催發。許願台詞若引用居民心願，該心願**源自
//! 居民 LLM 回覆的規則萃取**（間接受玩家的話影響），故一律先過 [`sanitize_wish_fragment`]
//! 去換行／控制字元＋收合空白＋字元數上限，**絕不讓越獄／注入／洗版字串直達泡泡框**；泛用
//! 許願句與動態牆文案皆內建常數，無任何玩家原文回放面。

/// 流星劃過後，`meteor:bool` 旗標維持為真的天氣檢查 tick 數（每 tick≈15 秒，見
/// `voxel_weather::WEATHER_CHECK_INTERVAL_SECS`）。設 1＝旗標亮一個檢查窗（約 15 秒），
/// 前端只在「旗標由假轉真」的上升緣播一次約 1 秒多的光痕動畫、之後自行隱藏——旗標亮這麼久
/// 只為確保各連線都能在某份快照裡撞見那道上升緣（不會因快照節拍錯過），視覺長度由前端自控。
pub const METEOR_VISIBLE_TICKS: u32 = 1;

/// 夜裡每次天氣檢查（≈15 秒一次）中，一顆流星劃過的機率。刻意偏小——流星該是**難得**撞見的
/// 驚喜，不是夜夜刷屏的日常。約每 15 秒骰一次、命中 6%，平均一個夜段（數分鐘）零星幾顆。
pub const METEOR_NIGHT_CHANCE: f32 = 0.06;

/// 許願台詞引用居民心願時，心願片段的字元數上限（超過截斷，防破泡泡框／洗版）。
pub const WISH_FRAG_MAX_CHARS: usize = 24;

/// 是否「夠暗、適合流星」的時段：`t`（time_of_day 0.0–1.0）落在深夜或入夜（星星大致可見的
/// 那段）才算。邊界（< 0.22 或 ≥ 0.80）刻意比 `voxel_time` 的 Night/Evening 稍寬一點點，
/// 好與前端 `nightFactor`（星星淡入淡出區間）大致對齊——流星只在看得見星星的夜空劃過。
pub fn is_night(t: f32) -> bool {
    t < 0.22 || t >= 0.80
}

/// 純函式：這一輪天氣檢查是否有一顆流星劃過（夜裡才可能、再過低機率擲骰）。
/// `roll` 由呼叫端傳 `rand::random::<f32>()`；`is_night` 由呼叫端據世界時鐘算好餵進來。
pub fn should_streak(is_night: bool, roll: f32) -> bool {
    is_night && roll < METEOR_NIGHT_CHANCE
}

/// 推進流星可見 tick：`started`（這一輪剛有流星劃過）→ 重設為 [`METEOR_VISIBLE_TICKS`]；
/// 否則每輪遞減 1、減到 0 為止（`saturating_sub` 自然淡出、永不下溢）。
/// 確定性、無副作用、可窮舉測試。呼叫端據「上一輪為 0、這一輪 > 0」判定「流星剛劃過」的
/// 一次性事件（觸發居民許願）。
pub fn next_meteor_ticks(prev: u32, started: bool) -> u32 {
    if started {
        METEOR_VISIBLE_TICKS
    } else {
        prev.saturating_sub(1)
    }
}

/// 清洗要嵌進許願泡泡的居民心願片段（源自居民 LLM 回覆的規則萃取，間接受玩家的話影響 →
/// 一律視為不可信）：去換行／控制字元、收合連續空白、去頭尾空白、按**字元**截到
/// [`WISH_FRAG_MAX_CHARS`]。清洗後為空 → 回 `None`（改許泛用願）。
///
/// 純函式、確定性、對任意輸入（含長字串／emoji／控制字元）都不 panic。
pub fn sanitize_wish_fragment(desire: &str) -> Option<String> {
    let mut out = String::new();
    let mut prev_space = false;
    for c in desire.chars() {
        // 換行與其他控制字元一律視為空白（避免破泡泡框／夾帶注入換行）。
        let is_ws = c.is_whitespace() || c.is_control();
        if is_ws {
            if !out.is_empty() && !prev_space {
                out.push(' ');
                prev_space = true;
            }
            continue;
        }
        out.push(c);
        prev_space = false;
    }
    let trimmed = out.trim();
    if trimmed.is_empty() {
        return None;
    }
    // 按字元（非位元組）截斷，避免切壞多位元組字元。
    let clipped: String = trimmed.chars().take(WISH_FRAG_MAX_CHARS).collect();
    let clipped = clipped.trim().to_string();
    if clipped.is_empty() {
        None
    } else {
        Some(clipped)
    }
}

/// 沒有當前心願的居民，抬頭望見流星時許的泛用祈願池（確定性選句，呼叫端傳 `pick`）。
const WISH_GENERIC: [&str; 4] = [
    "一顆流星劃過夜空……我悄悄許了個願。",
    "是流星！快許願——但願往後的日子都平平安安。",
    "看見流星了，願這片天地和大家都一切安好。",
    "流星耶！我閉上眼，許下一個說不出口的小小心願。",
];

/// 依 `pick` 選一句泛用許願台詞（`pick % len`，永遠有值、確定性、可測、不 panic）。
pub fn wish_bubble_generic(pick: usize) -> &'static str {
    WISH_GENERIC[pick % WISH_GENERIC.len()]
}

/// 引用居民心願的許願台詞模板（`{}` 處嵌**已清洗**的心願片段；心願本身是完整子句，
/// 如「真希望有玻璃」「我想要一個家」，故用「引述」框住讀來自然）。
const WISH_WITH_DESIRE: [&str; 3] = [
    "一顆流星劃過夜空……我閉上眼，把藏了好久的那句「{}」當成願望許了出去。",
    "是流星！我趕緊許願——{}……但願這次，天上會聽見。",
    "流星耶！我對著它悄悄許下心願：{}。",
];

/// 依 `pick` 選一句「引用心願」的許願台詞，把已清洗的 `frag` 嵌進模板。
/// 確定性、對任意 `pick` 取模不 panic；`frag` 應為 [`sanitize_wish_fragment`] 的輸出。
pub fn wish_bubble_with_desire(frag: &str, pick: usize) -> String {
    let tpl = WISH_WITH_DESIRE[pick % WISH_WITH_DESIRE.len()];
    tpl.replacen("{}", frag, 1)
}

/// 城鎮動態牆播報種類名稱（流星劃過那一刻寫一則，不在線上的玩家回來也讀得到今晚有流星）。
pub const FEED_KIND: &str = "流星許願";

/// 流星劃過時，城鎮動態牆的一則播報詳情（每顆流星一則、非每位居民一則，不洗版）。
pub fn feed_detail() -> &'static str {
    "一顆流星劃過乙太方界的夜空，抬頭望見的居民悄悄許下了心願。"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 夜間判定_只在夠暗時為真() {
        // 深夜、入夜 → 真。
        assert!(is_night(0.0));
        assert!(is_night(0.10));
        assert!(is_night(0.21));
        assert!(is_night(0.80));
        assert!(is_night(0.95));
        // 白天、黃昏未夠暗 → 假。
        assert!(!is_night(0.22));
        assert!(!is_night(0.42)); // 正午前後
        assert!(!is_night(0.5));
        assert!(!is_night(0.79));
    }

    #[test]
    fn 流星觸發_夜裡低機率才劃過() {
        // 夜裡、骰值低於門檻 → 劃過。
        assert!(should_streak(true, 0.0));
        assert!(should_streak(true, METEOR_NIGHT_CHANCE - 0.001));
        // 夜裡但骰值到門檻／偏高 → 不劃過。
        assert!(!should_streak(true, METEOR_NIGHT_CHANCE));
        assert!(!should_streak(true, 0.99));
        // 白天無論骰值都不劃過。
        assert!(!should_streak(false, 0.0));
        assert!(!should_streak(false, 0.99));
    }

    #[test]
    fn 可見tick_剛劃過重設離開遞減() {
        // 剛劃過 → 不論上一輪剩多少，一律重設為滿值。
        assert_eq!(next_meteor_ticks(0, true), METEOR_VISIBLE_TICKS);
        assert_eq!(next_meteor_ticks(5, true), METEOR_VISIBLE_TICKS);
        // 未劃過 → 遞減 1、減到 0 為止（不下溢）。
        assert_eq!(next_meteor_ticks(METEOR_VISIBLE_TICKS, false), METEOR_VISIBLE_TICKS - 1);
        assert_eq!(next_meteor_ticks(1, false), 0);
        assert_eq!(next_meteor_ticks(0, false), 0);
    }

    #[test]
    fn 剛劃過緣_可由0到大於0偵測() {
        // 呼叫端用「prev==0 && next>0」判定「流星剛劃過」的一次性事件——確認此判定成立。
        let prev = 0;
        let next = next_meteor_ticks(prev, true);
        assert!(prev == 0 && next > 0, "流星劃過該可偵測到上升緣");
    }

    #[test]
    fn 心願清洗_去換行控制字元並收合空白() {
        assert_eq!(
            sanitize_wish_fragment("真希望\n有玻璃").as_deref(),
            Some("真希望 有玻璃")
        );
        assert_eq!(
            sanitize_wish_fragment("  我想要一個家  ").as_deref(),
            Some("我想要一個家")
        );
        // 連續空白收合成單一空白。
        assert_eq!(
            sanitize_wish_fragment("我   想     要").as_deref(),
            Some("我 想 要")
        );
    }

    #[test]
    fn 心願清洗_空白與純控制字元回None() {
        assert_eq!(sanitize_wish_fragment(""), None);
        assert_eq!(sanitize_wish_fragment("   "), None);
        assert_eq!(sanitize_wish_fragment("\n\t\r"), None);
    }

    #[test]
    fn 心願清洗_超長截斷到字元上限不panic() {
        let long = "家".repeat(200);
        let out = sanitize_wish_fragment(&long).unwrap();
        assert_eq!(out.chars().count(), WISH_FRAG_MAX_CHARS);
        // emoji／多位元組字元不切壞、不 panic。
        let emoji = "🏠".repeat(100);
        let out2 = sanitize_wish_fragment(&emoji).unwrap();
        assert_eq!(out2.chars().count(), WISH_FRAG_MAX_CHARS);
    }

    #[test]
    fn 泛用許願台詞_非空且輪替有界() {
        for pick in 0..(WISH_GENERIC.len() * 3) {
            assert!(!wish_bubble_generic(pick).is_empty());
        }
        // 取模輪替：同餘者同句。
        assert_eq!(wish_bubble_generic(0), wish_bubble_generic(WISH_GENERIC.len()));
        assert_eq!(wish_bubble_generic(999), wish_bubble_generic(999 % WISH_GENERIC.len()));
    }

    #[test]
    fn 引用心願台詞_嵌入片段且不破框() {
        let frag = "我想要一個家";
        for pick in 0..(WISH_WITH_DESIRE.len() * 3) {
            let line = wish_bubble_with_desire(frag, pick);
            assert!(line.contains(frag), "台詞應含心願片段");
            assert!(!line.contains("{}"), "模板佔位符應已被替換");
            assert!(!line.contains('\n'), "台詞不該有換行破框");
        }
        // 同餘者同模板。
        assert_eq!(
            wish_bubble_with_desire(frag, 0),
            wish_bubble_with_desire(frag, WISH_WITH_DESIRE.len())
        );
    }

    #[test]
    fn 動態牆詳情非空() {
        assert!(!feed_detail().is_empty());
        assert!(!FEED_KIND.is_empty());
    }
}
