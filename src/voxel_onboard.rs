//! 乙太方界·居民迎新 v1（自主提案切片 ROADMAP 948）。
//!
//! **真缺口 / 為誰做**：舊 2D 世界有完整的新手引導（ROADMAP 396「最初幾步」、413），
//! 但乙太方界（現在的遊戲入口）**一條引導都沒有**——全新玩家連進來只看到一片方塊世界，
//! 沒人告訴他能挖、能合成、能蓋，甚至不知道**居民是可以點擊交談、會記得你的**——而那正是
//! 這個世界最獨特的魔法。新玩家的第一分鐘決定了他會不會留下來；現在的第一分鐘是一片沉默。
//!
//! 本切片補上乙太方界自己的新手引導，而且**用這個世界自己的方式**：不是冷冰冰的教學彈窗，
//! 而是**由一位居民親自迎接你**——你一進場，最近的一位醒著的居民便向你招呼「歡迎來到
//! 乙太方界，走近點我、跟我說說話吧」；接著她溫柔地陪你走完最初四小步（**交談 → 採集 →
//! 合成 → 放置**＝這個世界的核心循環），HUD 一張小卡逐步點亮；四步走完，她送你幾支火把
//! 當見面禮（夜裡有光，療癒且實用）、把「他是我迎進這個世界的旅人」**記進她的長期記憶**
//! （登入玩家）、城鎮動態牆留下一行迎新記事。世界的招牌魔法——「居民記得你」——
//! 第一次成為每位新旅人的**第一個體驗**。
//!
//! **與既有系統 razor-sharp 區隔**：
//! - **久別重逢（721 摘要／747 奔迎）**＝針對**回來的老玩家**（記憶已厚）；本刀針對**全新
//!   玩家**（世界對他一無所知），一個是重逢、一個是初見，觸發條件正好互斥。
//! - **舊 2D 新手引導（396/413）**＝另一個已封存世界的系統，與 voxel 零共用碼；本刀是
//!   乙太方界原生的，且骨架不同——2D 版是純 UI 清單，本刀由**居民具身迎接**（泡泡＋記憶
//!   ＋Feed），引導本身就是一次與居民的關係開端。
//! - **居民教你獨門配方（849）**＝好感達門檻後的**進階**驚喜；本刀是好感為零時的**第一步**。
//!
//! **成本 / 濫用防護鐵律**：
//! - **零 LLM**：迎新台詞、步驟提示、記憶、Feed 全是確定性模板；「交談」那一步玩家真的開口
//!   時走的是**既有** Talk 管線（its 冷卻／per-IP 限流／內容審查全套照舊），本模組不新增任何
//!   LLM 觸發面。
//! - **只迎登入玩家**：與居民對話需登入（治安三件套③），「交談」這一步訪客走不通；而且
//!   畢業旗標與居民記憶都要掛在持久身分上才有意義。訪客照樣自由遊逛，只是沒有引導卡。
//! - **玩家無從催發洗版**：迎新居民的招呼泡泡受**全域冷卻**（`GREET_BUBBLE_COOLDOWN_SECS`，
//!   在 `voxel_ws.rs` 以原子時戳把關）——批量註冊新帳號重複連線也不能讓居民頭上刷屏；
//!   引導卡是單播私訊，不佔任何公共面。畢業禮（幾支火把）走後端權威 `InvStore::give`，
//!   每個帳號一生一次（畢業旗標持久化、寫鎖 insert 判重），無利可圖。
//! - **不收自由輸入**：本模組所有字串都是伺服器策展模板，只嵌入玩家顯示名。
//!
//! 純邏輯層：除持久化小節（append-only jsonl，比照其餘 store）外零 IO、零鎖、零 async；
//! 挑迎新居民、泡泡、記憶、Feed、送禮的鎖序全在 `voxel_ws.rs`（短取即釋、循序不巢狀）。

use std::collections::HashSet;
use std::io::Write as _;

/// 四個引導步驟（bitmask，可任意順序完成——玩家先亂挖也算數，教學不逼人照本宣科）。
pub const STEP_TALK: u8 = 1 << 0;
/// 採集：第一次敲下任何方塊。
pub const STEP_BREAK: u8 = 1 << 1;
/// 合成：第一次在背包/工作台合成成功。
pub const STEP_CRAFT: u8 = 1 << 2;
/// 放置：第一次把方塊放進世界。
pub const STEP_PLACE: u8 = 1 << 3;
/// 全部步驟的集合（畢業判定）。
pub const ALL_STEPS: u8 = STEP_TALK | STEP_BREAK | STEP_CRAFT | STEP_PLACE;

/// 畢業見面禮：火把（[`crate::voxel::Block::Torch`]）×4——新旅人的第一個夜晚有光。
pub const GIFT_ITEM: u8 = 31;
pub const GIFT_COUNT: u32 = 4;

/// 迎新居民招呼泡泡的全域冷卻（秒）：訪客反覆重連也不能讓居民頭上刷屏
/// （引導卡照發，只有「泡泡」受此限）。
pub const GREET_BUBBLE_COOLDOWN_SECS: u64 = 120;

/// 泡泡台詞字元上限（與本專案其他社交泡泡一致）。
pub const SAY_MAX_CHARS: usize = 50;

/// Feed 事件種類名稱（面向玩家、集中可 i18n）。
pub const FEED_KIND: &str = "迎新";

/// 畢業旗標持久化路徑（append-only；一行一個已畢業的登入玩家名）。
pub const ONBOARD_DONE_PATH: &str = "data/voxel_onboard_done.jsonl";

/// 每條連線的迎新狀態（純記憶體；只有「登入且背包全空且未畢業」的連線才會持有，
/// 畢業後由持久旗標擋住不再觸發；連線中途斷線＝下次連線重新來過，無資料風險）。
pub struct Onboard {
    /// 已完成步驟 bitmask。
    pub done: u8,
    /// 迎新居民 id（畢業時她冒祝福泡泡、記進她的記憶）。
    pub greeter_id: String,
    /// 迎新居民顯示名。
    pub greeter_name: &'static str,
}

/// 是否該對這位剛連線的玩家啟動迎新：**登入帳號**、背包空空如也（世界對他還一無所知）
/// 且未畢業過。只迎登入玩家的原因：①「交談」是引導的靈魂一步，而與居民對話需登入
/// （治安三件套③，訪客走到這步會卡死畢不了業）；②畢業旗標與居民記憶都要掛在持久身分上
/// 才有意義。老玩家背包極少全空；萬一真的全空被再迎接一次，也只是多一份溫暖，不是 bug。
pub fn should_onboard(is_account: bool, bag_empty: bool, already_graduated: bool) -> bool {
    is_account && bag_empty && !already_graduated
}

/// 推進一步。回傳（新 bitmask，這一步是否**新**完成，是否恰好在這一步畢業）。
/// 冪等：重複完成同一步回 `newly=false`，畢業旗標只在「補上最後一塊拼圖」那一次為 true。
pub fn advance(done: u8, step: u8) -> (u8, bool, bool) {
    let newly = done & step == 0 && ALL_STEPS & step != 0;
    let new_done = done | (step & ALL_STEPS);
    let graduated = newly && new_done == ALL_STEPS;
    (new_done, newly, graduated)
}

/// 建議的下一步（引導卡頂端的提示挑這一步的文案）：照「交談→採集→合成→放置」的
/// 自然順序挑第一個還沒完成的；全完成回 `None`。
pub fn next_step(done: u8) -> Option<u8> {
    [STEP_TALK, STEP_BREAK, STEP_CRAFT, STEP_PLACE]
        .into_iter()
        .find(|s| done & s == 0)
}

/// 擷取字串前 [`SAY_MAX_CHARS`] 個字元（安全截斷、不破多位元組中文）。
fn cap(s: String) -> String {
    s.chars().take(SAY_MAX_CHARS).collect()
}

/// 迎新居民的招呼泡泡（確定性選句）。
pub fn greet_bubble(pick: usize) -> String {
    const LINES: [&str; 3] = [
        "歡迎來到乙太方界！走近點我，跟我說說話吧",
        "新來的旅人？別怕生，過來和我聊聊吧",
        "好久沒有新面孔了！來，先跟我打聲招呼吧",
    ];
    cap(LINES[pick % LINES.len()].to_string())
}

/// 引導卡上「下一步」的提示文案（面向玩家、集中一處可 i18n）。
///
/// 不再是孤立的動作清單，而是**迎新居民的第一人稱故事線**：每一步都先給一句「為什麼」，
/// 讓玩家不只知道「按什麼」，更知道「這一步要帶我去哪」。回傳型別維持 `&'static str`——
/// 迎新居民的名字由 `t:"onboard"` 訊息另一個 `greeter` 欄位與她的招呼泡泡帶出，這裡不嵌名，
/// 前端 `hint` 契約與 schema 不變。
pub fn step_hint(step: u8) -> &'static str {
    match step {
        // 交談——先認識彼此，這裡的居民會記得你。
        STEP_TALK => "你好，我是這裡的居民。點我坐下聊聊，先讓我知道你是誰",
        // 採集——為了生火，我們得先有柴。
        STEP_BREAK => "看見那邊的樹了嗎？走過去敲幾下，採點材料，我們拿來生火",
        // 合成——把材料變成有用的東西。
        STEP_CRAFT => "把採來的材料放進合成格，看好囉——它會變成一塊木板呢",
        // 放置——用木板蓋下屬於你的第一塊。
        STEP_PLACE => "有木板了！在地上放下它，先蓋出一個家的輪廓吧",
        // 全走完之後的過場（畢業祝福另在 grad_bubble 送出）。
        _ => "歡迎回家，旅人——這片天地從今天起也是你的了",
    }
}

/// 引導卡上每一步的短標籤。
pub fn step_label(step: u8) -> &'static str {
    match step {
        STEP_TALK => "交談",
        STEP_BREAK => "採集",
        STEP_CRAFT => "合成",
        STEP_PLACE => "放置",
        _ => "",
    }
}

/// 畢業時迎新居民的祝福泡泡（嵌玩家顯示名，確定性選句）。
pub fn grad_bubble(player: &str, pick: usize) -> String {
    let lines = [
        format!("{player}，你已經是乙太方界的一份子了！這幾支火把送你"),
        format!("看你上手得真快，{player}！收下火把，夜裡就有光了"),
        format!("{player}，往後這片天地就是你的家了，常來找我聊聊"),
    ];
    cap(lines[pick % lines.len()].clone())
}

/// 畢業時寫進迎新居民長期記憶的一句（掛玩家名下——她從此記得「是我迎他進來的」）。
pub fn grad_memory_line(player: &str) -> String {
    format!("{player}是我親手迎進乙太方界的旅人——看著他學會交談、採集、合成、蓋下第一塊，真替他高興。")
}

/// 畢業的城鎮動態牆一行。
pub fn grad_feed_line(greeter: &str, player: &str) -> String {
    format!("{greeter}迎接了初來乍到的{player}，陪他走完了在乙太方界的最初幾步")
}

// ── 持久化（append-only、向後相容）─────────────────────────────────────────────

/// 載回所有已畢業的登入玩家名（伺服器啟動時呼叫一次）。檔不存在／壞行皆容忍。
pub fn load_graduated() -> HashSet<String> {
    let Ok(raw) = std::fs::read_to_string(ONBOARD_DONE_PATH) else {
        return HashSet::new();
    };
    raw.lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter_map(|v| v.get("player").and_then(|p| p.as_str()).map(str::to_string))
        .collect()
}

/// 落地一位剛畢業的登入玩家（append-only；失敗只記 log，不影響遊戲流程）。
pub fn append_graduated(player: &str) {
    let line = serde_json::json!({ "player": player }).to_string();
    let _ = std::fs::create_dir_all("data");
    match std::fs::OpenOptions::new().create(true).append(true).open(ONBOARD_DONE_PATH) {
        Ok(mut f) => {
            let _ = writeln!(f, "{line}");
        }
        Err(e) => tracing::warn!("迎新畢業旗標落地失敗：{e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_onboard_only_for_fresh_accounts() {
        assert!(should_onboard(true, true, false), "登入＋背包空＋未畢業＝全新旅人");
        assert!(!should_onboard(false, true, false), "訪客不迎（交談步需登入才走得通）");
        assert!(!should_onboard(true, false, false), "背包有東西＝玩過了");
        assert!(!should_onboard(true, true, true), "已畢業不再迎");
        assert!(!should_onboard(true, false, true));
    }

    #[test]
    fn advance_marks_steps_and_is_idempotent() {
        let (d, newly, grad) = advance(0, STEP_TALK);
        assert_eq!(d, STEP_TALK);
        assert!(newly && !grad);
        // 重複同一步：冪等、不再是「新完成」。
        let (d2, newly2, grad2) = advance(d, STEP_TALK);
        assert_eq!(d2, d);
        assert!(!newly2 && !grad2);
    }

    #[test]
    fn advance_any_order_graduates_exactly_once() {
        // 亂序完成（先放置後交談）也照樣畢業，且畢業旗標只在最後一步那一次為 true。
        let (d, _, g) = advance(0, STEP_PLACE);
        assert!(!g);
        let (d, _, g) = advance(d, STEP_CRAFT);
        assert!(!g);
        let (d, _, g) = advance(d, STEP_BREAK);
        assert!(!g);
        let (d, newly, g) = advance(d, STEP_TALK);
        assert!(newly && g, "補上最後一塊拼圖那一次才畢業");
        assert_eq!(d, ALL_STEPS);
        // 畢業後再送任何步：不再觸發。
        let (_, newly, g) = advance(d, STEP_BREAK);
        assert!(!newly && !g);
    }

    #[test]
    fn advance_ignores_unknown_step_bits() {
        // 未知位元（防未來手滑）不推進、不畢業。
        let (d, newly, grad) = advance(0, 1 << 6);
        assert_eq!(d, 0);
        assert!(!newly && !grad);
    }

    #[test]
    fn next_step_follows_natural_order() {
        assert_eq!(next_step(0), Some(STEP_TALK), "先學交談——這個世界的魔法");
        assert_eq!(next_step(STEP_TALK), Some(STEP_BREAK));
        assert_eq!(next_step(STEP_TALK | STEP_BREAK), Some(STEP_CRAFT));
        assert_eq!(next_step(ALL_STEPS & !STEP_PLACE), Some(STEP_PLACE));
        assert_eq!(next_step(ALL_STEPS), None, "全完成沒有下一步");
        // 亂序時挑最前面沒完成的。
        assert_eq!(next_step(STEP_PLACE), Some(STEP_TALK));
    }

    #[test]
    fn bubbles_cycle_non_empty_and_capped() {
        for pick in 0..7 {
            let g = greet_bubble(pick);
            assert!(!g.is_empty());
            assert!(g.chars().count() <= SAY_MAX_CHARS);
            let b = grad_bubble("小樹", pick);
            assert!(!b.is_empty());
            assert!(b.contains("小樹"), "畢業祝福點名玩家");
            assert!(b.chars().count() <= SAY_MAX_CHARS);
        }
        // 選句循環穩定（同 pick 同句）。
        assert_eq!(greet_bubble(1), greet_bubble(1 + 3));
    }

    #[test]
    fn hints_and_labels_cover_all_steps() {
        for s in [STEP_TALK, STEP_BREAK, STEP_CRAFT, STEP_PLACE] {
            assert!(!step_hint(s).is_empty());
            assert!(!step_label(s).is_empty());
        }
    }

    #[test]
    fn memory_and_feed_lines_name_the_player() {
        let m = grad_memory_line("小樹");
        assert!(m.contains("小樹") && !m.contains("{"), "記憶點名玩家、無佔位符外洩");
        let f = grad_feed_line("露娜", "小樹");
        assert!(f.contains("露娜") && f.contains("小樹"));
    }

    #[test]
    fn gift_is_torch() {
        assert_eq!(GIFT_ITEM, crate::voxel::Block::Torch as u8, "見面禮與方塊表對齊");
        assert!(GIFT_COUNT > 0);
    }
}
