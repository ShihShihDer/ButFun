//! 乙太方界·居民會反過來拜託你幫個小忙 v1（voxel-request）。
//!
//! **北極星（PLAN_ETHERVOX item 3「記憶→行為·你的互動有後果」× 玩家遊玩「交織點」）**：
//! 至今「請人幫忙採集」永遠是**單向**的——玩家對居民下令（`voxel_fetch`「幫我採木頭」、
//! `voxel_directed_task`「幫我把這裡整平」），居民照做。居民自己有渴望（`voxel_desires`）、
//! 會蓋家、會掏心（781），卻從不會**反過來開口拜託你**：「我這陣子想弄點東西，手邊剛好缺塊
//! 木頭，你要是採到了，能不能勻我一塊呀？」這一刀補上那個對稱的另一半——**夠面熟的居民偶爾會
//! 主動向你討一樣好採集的小材料**；你去把它採來、當禮物送給她（走既有 `Gift` 送禮管線），她會
//! 特別歡欣地道謝、把「你在我開口時幫了我」這份人情**牢牢記進對你的記憶**，交情因此更深一層。
//!
//! 這是乙太方界第一次讓「採集」這條純人類樂趣，和「居民的需要」直接接上：你不再只是為自己攢材料，
//! 也第一次為了**幫一位居民的忙**而去採集——人類的樂趣與 AI 的生活，在一塊木頭上交織。
//!
//! **與既有的定位區隔**：
//! - `voxel_fetch`（幫我採集）是**玩家命令居民**去採；本刀是**居民拜託玩家**去採，方向相反。
//! - `voxel_desires` / 掏心（781）是居民**對自己渴望的表達**；本刀是她**對你提出一個你做得到的
//!   具體請求**，且**你的回應真的改變你們的交情**（送到＝人情＋好感；沒送＝過段時間她自己作罷）。
//! - 送對禮物（722，`classify_item_desire`）是「猜中她心願」的驚喜；本刀是她**明講**要什麼、你照
//!   單去採——一個靠猜、一個靠她開口。
//!
//! **v2（自主提案，補上 v1 文件說了但沒真的做到的另一半）**：v1 的道理寫著「沒送＝過段時間她自己
//! 作罷」，但實作從沒讓 `open_request` 逾期清掉——她開口一次，你沒送到，`open_request` 就永遠卡
//! 在 `Some`，往後這位居民**這輩子都問不出下一次**（`open_request.is_none()` 這道閘永遠不過，
//! 沒有任何報錯或提示，純粹靜默壞掉）。本刀補上真正的到期：等了 [`REQUEST_GIVEUP_SECS`] 材料還沒
//! 到，她就自己放下這份期待、清掉請求，之後還能再開口。同時第一次讓「你沒兌現」留下痕跡——不是
//! 扣好感的懲罰，而是**連續兩次**都落空在**同一個人**身上時，她輕聲說一句放下、記一筆淡淡的心情
//! （[`record_miss`] / [`should_note_unfulfilled`] / [`unfulfilled_say_line`] / [`unfulfilled_memory_line`]）。
//! 單次錯過（可能你剛好不在）給足夠的信任、不留痕跡；換了對象或中途被別人送到過都會重新起算。
//! 這是 PLAN_ETHERVOX「記憶→行為·你的互動有後果」第一次真的落在「你選擇不回應」這一側，而不只是
//! 「你回應了會怎樣」。
//!
//! **v3·村莊委託牆（自主提案切片，ROADMAP 1015）**：v1~v2 讓居民「反過來拜託你」，但發現一則
//! 請求純靠運氣——只有此刻正巧站在她面前才聽得到，全村從沒有一處能一次看到「現在到底有誰在
//! 等」、更遑論知道誰最快就要放棄。[`RequestBoardEntry`] / [`sort_board_by_urgency`] 把散落
//! 各居民身上的 `open_request` 彙整成一張依剩餘等待時間排序的清單，玩家第一次能**主動挑一則
//! 最急迫的委託去幫**，而非隨緣路過巧遇——與 `voxel_trade::nearby_trade_previews`（附近貨品
//! 一覽，1013）同款「彙整→依序瀏覽」介面手法，但彙整的是**居民主動開口的請求**而非**居民手上
//! 的貨品**，
//! 資料源頭與玩家意圖皆不同軸（一個是「誰缺什麼你能幫」，一個是「誰在賣什麼你能買」）。
//!
//! **純邏輯層**：是否開口（[`should_post_request`]）、討什麼（[`pick_request`]）、把請求包成一句話
//! （[`request_line`]）、送到後的道謝（[`fulfil_thanks_line`]）與記進記憶的摘要（[`fulfil_memory_line`]）、
//! 逾期未兌現的追蹤與台詞（v2，見上）全是確定性純函式，零 LLM、零鎖、零 IO。冷卻計時 / 好感讀取 /
//! 記憶寫入 / 送禮判定全在 `voxel_ws.rs`，沿用既有招呼那條已驗證的短鎖循序。
//!
//! **成本 / 濫用防護**：討的材料只從一份**固定白名單**（[`REQUESTABLE`]，都是好採的基礎資源）裡選，
//! 句子全走固定模板，**永不夾帶玩家原話**（無注入 / NSFW 風險）；只對好感達 [`REQUEST_MIN_AFFINITY`]
//! 的玩家開口、配合每位居民 [`REQUEST_COOLDOWN_SECS`] 的長冷卻＋「同時只掛一個未了請求」，稀有有份量、
//! 天然防洗版、也防「靠不停幫小忙刷好感」（一個請求只回饋一次、送到即清）。零 migration、零新協議欄位、
//! 零新美術、FPS 零影響（純後端、僅招呼時序偶發）。

/// 一樣居民可能向你討的材料：物品 id ＋ 面向玩家的中文名（留 i18n 空間）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequestItem {
    /// 物品 / 方塊 id（與 `voxel_gift::item_name_zh` 對照的同一套 id）。
    pub item_id: u8,
    /// 面向玩家顯示的材料名。
    pub name: &'static str,
}

/// 居民會向你討的材料白名單——刻意只放「玩家隨手採集就有」的基礎資源，讓「幫這個忙」門檻低、
/// 暖而不擾（不會討稀有／需長途跋涉的東西，那會變成負擔而非療癒）。id 與 `item_name_zh` 一致。
pub const REQUESTABLE: [RequestItem; 4] = [
    RequestItem { item_id: 5, name: "木頭" },  // 砍樹就有
    RequestItem { item_id: 3, name: "石頭" },  // 挖礦就有
    RequestItem { item_id: 20, name: "煤礦" }, // 挖礦就有
    RequestItem { item_id: 4, name: "沙" },    // 河邊 / 沙漠就有
];

/// 居民願意向玩家開口討東西的最低好感（＝關於這位玩家的記憶筆數）。設 2——比全然陌生（0~1）
/// 多一點面熟才好意思拜託人幫忙，但門檻低於掏心（781 的 3），因為「討塊木頭」比「說心事」更隨性。
pub const REQUEST_MIN_AFFINITY: usize = 2;

/// 同一位居民再次開口討東西的冷卻（秒）。設得長（300s＝5 分鐘）——拜託人幫忙是偶爾為之的事，
/// 不是每次靠近都伸手要，稀有才有份量，也把「靠幫小忙刷好感」的速率天然夾死。
pub const REQUEST_COOLDOWN_SECS: f32 = 300.0;

/// 請求泡泡的字元上限（與泡泡框上限一致，超出截斷不破框）。
pub const REQUEST_SAY_MAX_CHARS: usize = 40;

/// 判斷此刻是否要主動向玩家開口討東西：好感夠（≥ [`REQUEST_MIN_AFFINITY`]）＋ 冷卻到期 ＋
/// 手邊**沒有**尚未了結的請求（同時只掛一個，天然防洗版）＋ 過了機率門檻。
///
/// 純函式、確定性（機率骰由呼叫端傳入）。「討什麼」由 [`pick_request`] 另外決定。
pub fn should_post_request(
    affinity: usize,
    cooldown_ok: bool,
    has_open_request: bool,
    roll: f32,
    chance: f32,
) -> bool {
    affinity >= REQUEST_MIN_AFFINITY && cooldown_ok && !has_open_request && roll < chance
}

/// 依 `pick` 在白名單裡確定性選一樣材料來討。永不 panic（對長度取模）。
pub fn pick_request(pick: usize) -> RequestItem {
    REQUESTABLE[pick % REQUESTABLE.len()]
}

/// 把「我想討 {材料}」包成一句主動開口的話，依 `pick` 在幾組固定語氣模板間確定性輪替。
/// 整句以字元為單位截到 [`REQUEST_SAY_MAX_CHARS`] 內，永不破泡泡框、永不回空。
pub fn request_line(item_name: &str, pick: usize) -> String {
    const TEMPLATES: [&str; 5] = [
        "欸，你要是採到{}，能不能勻我一點呀？",
        "跟你討個小忙——手邊剛好缺塊{}呢。",
        "我這陣子想弄點東西，正缺{}，你有的話…？",
        "不好意思，能幫我帶塊{}來嗎？我這正需要。",
        "要是你路上採到{}，記得留一份給我喔！",
    ];
    let line = TEMPLATES[pick % TEMPLATES.len()].replacen("{}", item_name, 1);
    line.chars().take(REQUEST_SAY_MAX_CHARS).collect()
}

/// 開口討東西時，同步在城鎮動態牆留一行（第三人稱旁白，讓不在場 / 回來的玩家也讀到「某居民
/// 正想要某材料」）。面向玩家字串、留 i18n 空間。
pub fn request_feed_line(resident: &str, item_name: &str) -> String {
    format!("{resident}正想要一些{item_name}，盼著有人能勻她一點。")
}

/// 玩家真的把居民開口討的材料送到時的道謝台詞（比一般贈禮更歡欣，因為「你在我開口時幫了我」）。
/// 依 `pick` 確定性輪替；截到框內、永不回空。
pub fn fulfil_thanks_line(player: &str, item_name: &str, pick: usize) -> String {
    const TEMPLATES: [&str; 4] = [
        "哇，你真的幫我把{item}帶來了！{player}，太謝謝你了，我記著這份人情。",
        "{player}你居然記得我要{item}——這份心意我可放在心上了，謝謝你！",
        "正需要{item}的時候你就來了，{player}，有你這樣的朋友真好。",
        "你替我把{item}採來啦？{player}，你這個忙我會一直記得的。",
    ];
    let line = TEMPLATES[pick % TEMPLATES.len()]
        .replace("{item}", item_name)
        .replace("{player}", player);
    line.chars().take(REQUEST_SAY_MAX_CHARS).collect()
}

/// 請求被滿足後，記進居民「關於這位玩家」的一筆記憶摘要（第一人稱、episodic）。
/// 停在「你在我開口時幫了我」這個情節層——累積好感（記憶筆數），供日後回想 / 日記昇華。
pub fn fulfil_memory_line(player: &str, item_name: &str) -> String {
    format!("我開口向{player}討{item_name}，對方真的替我採來了——這份幫忙我記在心裡。")
}

/// 依 `item_id` 從白名單反查材料名。找不到（理論上不會發生，材料一律出自 [`pick_request`]）就
/// 回退成通用字「材料」，永不 panic、永不回空。
pub fn item_name_by_id(item_id: u8) -> &'static str {
    REQUESTABLE
        .iter()
        .find(|w| w.item_id == item_id)
        .map(|w| w.name)
        .unwrap_or("材料")
}

/// 開口討東西後，等這麼久（秒）材料還沒送到，就自己放下這份期待、清掉請求——不然 `open_request`
/// 卡在 `Some` 永遠不清，這位居民往後再也開不了口。刻意比 [`REQUEST_COOLDOWN_SECS`] 短：給到期後
/// 剩餘的冷卻補完，讓「從開口到下次能再開口」的總時距貼齊原本設計的稀有節奏，同時不再永久卡死。
pub const REQUEST_GIVEUP_SECS: f32 = 120.0;

/// 連續幾次都落空在同一個人身上，才算「值得留下痕跡」——單次錯過（可能對方剛好不在）給足夠的
/// 信任，不記在心上；連續兩次才代表這不是巧合。
pub const REQUEST_UNFULFILLED_TRACE_THRESHOLD: u8 = 2;

/// 給上一輪的追蹤狀態（`(上次落空對象, 連續次數)`，若有）與這次落空的玩家名，算出新的追蹤狀態。
/// 同一人再次落空 → 次數 +1；換了對象、或這是第一次追蹤 → 從 1 重新起算（不遷怒別人）。
/// 純函式、飽和加法（`u8` 不會溢位 panic）。
pub fn record_miss(prev: Option<(&str, u8)>, player: &str) -> (String, u8) {
    match prev {
        Some((p, misses)) if p == player => (player.to_string(), misses.saturating_add(1)),
        _ => (player.to_string(), 1),
    }
}

/// 這次落空的連續次數是否已達 [`REQUEST_UNFULFILLED_TRACE_THRESHOLD`]，值得留下一句話與一筆記憶。
pub fn should_note_unfulfilled(misses: u8) -> bool {
    misses >= REQUEST_UNFULFILLED_TRACE_THRESHOLD
}

/// 連續落空達門檻時，居民輕聲放下這份期待的一句話——不指責、不扣好感，純粹不再等了。
/// 依 `pick` 確定性輪替；截到泡泡框內、永不回空。
pub fn unfulfilled_say_line(pick: usize) -> String {
    const TEMPLATES: [&str; 3] = [
        "算了，你大概是忙，我不勉強你。",
        "嗯……看來這陣子不方便，那就算了吧。",
        "沒關係，我不等了，下次有緣再說。",
    ];
    TEMPLATES[pick % TEMPLATES.len()]
        .chars()
        .take(REQUEST_SAY_MAX_CHARS)
        .collect()
}

/// 連續落空達門檻時，記進「關於這位玩家」的一筆淡淡記憶（第一人稱、不指責，純粹記下這份期待
/// 沒被接住）。供日後回想 / 日記昇華引用。
pub fn unfulfilled_memory_line(player: &str, item_name: &str) -> String {
    format!("我跟{player}討過{item_name}，接連兩次都沒等到，這次我打算不再開口了。")
}

/// **村莊委託牆 v1（自主提案切片，ROADMAP 1015）**：v1~v2 讓居民能「反過來拜託你」，但發現
/// 的路徑純粹靠運氣——只有此刻正巧站在她面前才聽得到那句泡泡話；就算城鎮動態牆飄過一句
/// 「某某正想要某材料」，那也只是一閃即逝的日誌，讀到時她可能早就等到、或早就放棄了。全村
/// 從沒有一處能讓玩家一次看到「現在到底有誰在等」，也無從得知**哪一位最快就要放棄**——你永遠
/// 只能隨緣路過，不能主動選擇去幫誰。本結構把散落各居民身上的 `open_request` 彙整成一張依
/// 剩餘等待時間（越快沒耐性排越前面）排序的清單，讓玩家第一次能**主動挑一則最急迫的委託去幫**，
/// 而非碰運氣巧遇。
#[derive(Clone, Debug, PartialEq)]
pub struct RequestBoardEntry {
    pub resident_id: String,
    pub resident_name: &'static str,
    pub item_id: u8,
    pub item_name: &'static str,
    /// 距離這位居民放棄等待還剩幾秒（`request_timer`，越小越急迫）。
    pub remaining_secs: f32,
}

/// 把「目前掛著未了請求的居民」快照（呼叫端已在鎖內濾出，本函式零鎖零 IO）依剩餘秒數由小到大
/// 排序——快沒耐性的排最前面，讓玩家一眼看到最該優先去幫誰。剩餘秒數相同時退回 `resident_name`
/// 確定性排序，結果不受輸入順序影響、可窮舉測試。
pub fn sort_board_by_urgency(mut entries: Vec<RequestBoardEntry>) -> Vec<RequestBoardEntry> {
    entries.sort_by(|a, b| {
        a.remaining_secs
            .partial_cmp(&b.remaining_secs)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.resident_name.cmp(b.resident_name))
    });
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_post_needs_affinity_cooldown_noopen_and_roll() {
        // 四條件齊備才開口。
        assert!(should_post_request(REQUEST_MIN_AFFINITY, true, false, 0.0, 0.5));
        assert!(should_post_request(10, true, false, 0.49, 0.5));
        // 好感不足 → 否決。
        assert!(!should_post_request(REQUEST_MIN_AFFINITY - 1, true, false, 0.0, 0.5));
        // 冷卻未到 → 否決。
        assert!(!should_post_request(10, false, false, 0.0, 0.5));
        // 已有未了請求 → 否決（同時只掛一個）。
        assert!(!should_post_request(10, true, true, 0.0, 0.5));
        // 骰子未過門檻 → 否決。
        assert!(!should_post_request(10, true, false, 0.5, 0.5));
        assert!(!should_post_request(10, true, false, 0.9, 0.5));
    }

    #[test]
    fn pick_request_is_deterministic_and_in_whitelist() {
        for pick in 0..20 {
            let r = pick_request(pick);
            // 選出的一定在白名單內。
            assert!(REQUESTABLE.iter().any(|w| *w == r), "選出的材料必須在白名單");
            // 同 pick → 同結果（確定性）。
            assert_eq!(pick_request(pick), r);
        }
        // 覆蓋整個白名單（0..len 各對到不同一項）。
        let picked: Vec<u8> = (0..REQUESTABLE.len()).map(|i| pick_request(i).item_id).collect();
        for w in &REQUESTABLE {
            assert!(picked.contains(&w.item_id), "白名單每項都應被選到：{}", w.name);
        }
    }

    #[test]
    fn whitelist_ids_unique_and_named() {
        for (i, a) in REQUESTABLE.iter().enumerate() {
            assert!(!a.name.is_empty(), "材料須有名字");
            for b in &REQUESTABLE[i + 1..] {
                assert_ne!(a.item_id, b.item_id, "白名單 id 不得重複");
            }
        }
    }

    #[test]
    fn request_line_names_item_and_fits_frame() {
        for pick in 0..10 {
            let line = request_line("木頭", pick);
            assert!(!line.is_empty(), "請求句不該為空");
            assert!(
                line.chars().count() <= REQUEST_SAY_MAX_CHARS,
                "請求句不該破泡泡框：{line}"
            );
            assert!(!line.contains("{}"), "佔位符應已被材料名替換：{line}");
        }
        assert!(request_line("石頭", 0).contains("石頭"), "請求句應點名材料");
        // 同 pick、同材料 → 同一句（確定性）。
        assert_eq!(request_line("煤礦", 3), request_line("煤礦", 3));
    }

    #[test]
    fn fulfil_thanks_names_player_item_and_fits_frame() {
        for pick in 0..8 {
            let line = fulfil_thanks_line("諾瓦", "木頭", pick);
            assert!(!line.is_empty());
            assert!(
                line.chars().count() <= REQUEST_SAY_MAX_CHARS,
                "道謝句不該破泡泡框：{line}"
            );
            assert!(!line.contains("{item}") && !line.contains("{player}"), "佔位符應已替換：{line}");
        }
        let l = fulfil_thanks_line("諾瓦", "石頭", 0);
        assert!(l.contains("諾瓦") && l.contains("石頭"), "道謝應點名玩家與材料");
    }

    #[test]
    fn fulfil_memory_names_player_and_item() {
        let m = fulfil_memory_line("諾瓦", "煤礦");
        assert!(m.contains("諾瓦"), "記憶應含玩家名");
        assert!(m.contains("煤礦"), "記憶應含材料名");
        assert!(!m.is_empty());
    }

    #[test]
    fn feed_line_names_resident_and_item() {
        let f = request_feed_line("露娜", "沙");
        assert!(f.contains("露娜") && f.contains("沙"), "動態牆應點名居民與材料");
    }

    #[test]
    fn item_name_by_id_finds_whitelist_and_falls_back() {
        for w in &REQUESTABLE {
            assert_eq!(item_name_by_id(w.item_id), w.name);
        }
        // 不在白名單的 id → 回退固定字串，不 panic。
        assert_eq!(item_name_by_id(255), "材料");
    }

    #[test]
    fn record_miss_same_player_increments_else_resets() {
        // 首次追蹤 → 1。
        assert_eq!(record_miss(None, "諾瓦"), ("諾瓦".to_string(), 1));
        // 同一人連續落空 → 累加。
        assert_eq!(record_miss(Some(("諾瓦", 1)), "諾瓦"), ("諾瓦".to_string(), 2));
        assert_eq!(record_miss(Some(("諾瓦", 2)), "諾瓦"), ("諾瓦".to_string(), 3));
        // 換了對象 → 從 1 重新起算，不遷怒別人。
        assert_eq!(record_miss(Some(("諾瓦", 5)), "露娜"), ("露娜".to_string(), 1));
        // u8 飽和加法不 panic。
        assert_eq!(record_miss(Some(("諾瓦", u8::MAX)), "諾瓦").1, u8::MAX);
    }

    #[test]
    fn should_note_unfulfilled_respects_threshold_boundary() {
        assert!(!should_note_unfulfilled(REQUEST_UNFULFILLED_TRACE_THRESHOLD - 1), "未達門檻不留痕跡");
        assert!(should_note_unfulfilled(REQUEST_UNFULFILLED_TRACE_THRESHOLD), "達門檻即留痕跡");
        assert!(should_note_unfulfilled(REQUEST_UNFULFILLED_TRACE_THRESHOLD + 3), "超過門檻仍留痕跡");
    }

    #[test]
    fn unfulfilled_say_line_fits_frame_and_never_empty() {
        for pick in 0..10 {
            let line = unfulfilled_say_line(pick);
            assert!(!line.is_empty(), "放下的話不該為空");
            assert!(
                line.chars().count() <= REQUEST_SAY_MAX_CHARS,
                "放下的話不該破泡泡框：{line}"
            );
        }
        // 同 pick → 同一句（確定性）。
        assert_eq!(unfulfilled_say_line(2), unfulfilled_say_line(2));
    }

    #[test]
    fn unfulfilled_memory_names_player_and_item() {
        let m = unfulfilled_memory_line("諾瓦", "木頭");
        assert!(m.contains("諾瓦"), "記憶應含玩家名");
        assert!(m.contains("木頭"), "記憶應含材料名");
        assert!(!m.is_empty());
    }

    fn board_entry(id: &str, name: &'static str, remaining: f32) -> RequestBoardEntry {
        RequestBoardEntry {
            resident_id: id.to_string(),
            resident_name: name,
            item_id: 5,
            item_name: "木頭",
            remaining_secs: remaining,
        }
    }

    #[test]
    fn sort_board_by_urgency_empty_in_empty_out() {
        assert!(sort_board_by_urgency(vec![]).is_empty());
    }

    #[test]
    fn sort_board_by_urgency_ascending_remaining() {
        let entries = vec![
            board_entry("a", "露娜", 80.0),
            board_entry("b", "諾娃", 5.0),
            board_entry("c", "賽勒", 40.0),
        ];
        let sorted = sort_board_by_urgency(entries);
        let names: Vec<&str> = sorted.iter().map(|e| e.resident_name).collect();
        assert_eq!(names, vec!["諾娃", "賽勒", "露娜"], "最急迫（剩餘秒數最少）應排最前面");
    }

    #[test]
    fn sort_board_by_urgency_tie_breaks_by_name() {
        let entries = vec![
            board_entry("a", "露娜", 10.0),
            board_entry("b", "奧瑞", 10.0),
        ];
        let sorted = sort_board_by_urgency(entries);
        let names: Vec<&str> = sorted.iter().map(|e| e.resident_name).collect();
        assert_eq!(names, vec!["奧瑞", "露娜"], "剩餘秒數相同時應以名字確定性排序");
    }

    #[test]
    fn sort_board_by_urgency_does_not_panic_on_nan() {
        let entries = vec![
            board_entry("a", "露娜", f32::NAN),
            board_entry("b", "諾娃", 10.0),
        ];
        let sorted = sort_board_by_urgency(entries);
        assert_eq!(sorted.len(), 2, "壞資料不該讓排序整支 panic，只影響相對順序");
    }

    #[test]
    fn sort_board_by_urgency_preserves_all_entries() {
        let entries = vec![
            board_entry("a", "露娜", 30.0),
            board_entry("b", "諾娃", 10.0),
            board_entry("c", "賽勒", 20.0),
        ];
        let sorted = sort_board_by_urgency(entries);
        assert_eq!(sorted.len(), 3);
        for id in ["a", "b", "c"] {
            assert!(sorted.iter().any(|e| e.resident_id == id), "不應遺漏任何一筆");
        }
    }
}
