//! 乙太方界·居民日記——把居民的長期記憶**昇華**成「她沒說出口的內心生命故事」。
//!
//! **核心信念**：AI 的內在生活要看得見才算活著——但日記是「**瞥見居民的內心**」，
//! 不是聊天記錄的謄本。早期版本把每筆 `MemoryEntry`（內含玩家私下原話）整包倒出來，
//! 等於公開展示對話謄本：**不妥（洩漏私下原話）＋ 雜訊多**。
//!
//! 這一版改成 **curated 內心反思**：
//! 1. **不逐字倒出對話**：玩家原句只在內部用來「判斷主題」，**絕不**進到輸出文字。
//! 2. **昇華成第一人稱獨白**：把記憶轉成居民自己的感受（有情感、有人味），
//!    例：「有位旅人和我聊起星空，不知怎地，我心裡也升起想抬頭多看幾眼夜空的念頭。」
//! 3. **少而有意義**：同主題的多筆記憶**收斂成一條**（並改用「好幾次…」語氣），
//!    寒暄 / 太短的訊息**直接丟掉**，整本 cap 在 [`MAX_DIARY_ENTRIES`] 條——降噪。
//! 4. **成本省**：純規則式抽象（從記憶結構生成第一人稱句），**零 LLM、確定性、可測**。
//!    日後若要升級成輕量 LLM 摘要，替換 [`reflection_for`] 即可、上下游不動。
//! 5. **隱私**：輸出永不含玩家原話、玩家名或可識別細節——旅人一律以「有位旅人」泛稱。
//!
//! 這裡只放確定性純邏輯；鎖 / 連線都在 `voxel_ws.rs`。不抄外部碼；繁中註解。
//!
//! **鄰里生活主題（自主提案切片）**：日記至今只看得到「玩家跟我聊過什麼」——但這座小村早已
//! 疊了一整套居民↔居民的生活（分食守望 800、打氣 679、拌嘴 715、圓夢賀喜 782…），這些事件
//! 寫進記憶時用的是**固定模板句**（如「去陪伴了露娜」），不是玩家的話。用既有「玩家對話」的
//! 抽句＋關鍵字判題邏輯去讀這些模板句，輕則落入籠統的 `Other`、重則被關鍵字巧合誤判成不相干
//! 的主題（例如「陪伴」命中 `Friendship` 的「陪」字，讀起來變成「有人記得我」，文不對題）。
//! **本擴充**沿用 [`SIGN_MEMORY_TAG`] 讀牌記憶的成熟前例——呼叫端（`voxel_ws.rs`）用
//! [`tag_neighborly`] 幫這類記憶的模板句包一層識別前綴，日記端優先辨識前綴、直接歸類，
//! 完全跳過「抽玩家原句／關鍵字判題」那段（本就不適用於模板句，也沒有洩漏風險）。

use serde::Serialize;

use crate::voxel_memory::MemoryEntry;
use crate::voxel_readsign::SIGN_MEMORY_TAG;

/// 「鄰里生活」記憶的識別前綴——居民↔居民的模板句記憶（分食/打氣/拌嘴/賀喜…）
/// 用這個前綴標記，讓日記端跳過玩家對話那套抽句/關鍵字判題，直接歸類（比照 [`SIGN_MEMORY_TAG`]）。
pub const NEIGHBORLY_TAG: &str = "🏘️鄰里";

/// 幫一段鄰里生活模板句包上識別前綴，供日記辨識（呼叫端在 `voxel_ws.rs` 記憶落地時使用）。
pub fn tag_neighborly(text: &str) -> String {
    format!("{NEIGHBORLY_TAG}{text}")
}

/// 這段記憶摘要是否帶了「內部識別前綴」（[`NEIGHBORLY_TAG`] 鄰里生活／[`SIGN_MEMORY_TAG`] 讀牌）。
///
/// 這類前綴只給日記端辨識分類用、**不是給玩家看的文字**。凡是會把 `MemoryEntry.summary` 原文
/// 直接組進玩家可見輸出的地方（口耳相傳 `voxel_gossip`、營火說故事 `voxel_campfire_tale` 的頭上
/// 泡泡），都要先用這個判定濾掉，否則「🏘️鄰里…」「🪧讀到告示牌…」這種帶原始標記的破碎文字會
/// 直接洩漏到玩家眼前。集中在此一處判定，日後新增內部前綴只要更新這裡、各挑選端自動涵蓋。
pub fn is_internal_tagged(summary: &str) -> bool {
    summary.starts_with(NEIGHBORLY_TAG) || summary.starts_with(SIGN_MEMORY_TAG)
}

/// 整本日記最多顯示幾條內心反思（生命故事級，少而有意義）。
pub const MAX_DIARY_ENTRIES: usize = 6;

/// 玩家原句短於這個字元數 → 視為寒暄 / 無意義，不昇華成日記（降噪）。
const SNIPPET_MIN_SIGNAL_CHARS: usize = 3;

/// 日記裡的單一條目：一段居民第一人稱的**內心反思**。
///
/// 刻意**不含**玩家名與玩家原話——日記是內心獨白，不是對話謄本。
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DiaryEntry {
    /// 代表這段反思的記憶序號（越大越新），供前端依序排列（最新在前）。
    pub seq: u64,
    /// 居民第一人稱的反思文字（已昇華，無玩家原話 / 無玩家名）。
    pub text: String,
}

/// 一位居民的完整日記頁：名字 + 當前心願 + 內心反思列表。
#[derive(Clone, Debug, Serialize)]
pub struct DiaryPage {
    /// 居民系統 id（如 "vox_res_0"）。
    pub resident_id: String,
    /// 居民顯示名（如「露娜」）。
    pub resident_name: String,
    /// 居民目前的心願（`None` = 尚未有任何心願；由玩家對話種下）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desire: Option<String>,
    /// 內心反思列表，**最新在前**（seq 大→小）。空列表 = 還沒有可昇華的記憶。
    pub entries: Vec<DiaryEntry>,
    /// 更早以前、已被記憶 cap 淘汰的舊記憶留下的模糊一句（記憶 v2「整併/壓縮/封存」）。
    /// `None` = 記憶從未滿載過，沒有任何東西被淡忘。**不含原話**——淘汰前只留下固定的
    /// 去識別化主題標籤（如「星空」「蓋造」，見 `voxel_memory::impression_topic`），
    /// 守日記「輸出永不含玩家原話」的隱私鐵律。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub faint_impression: Option<String>,
    /// 居民對自己的「自我印象」（自我印象 v1，ROADMAP 770）：從累積記憶昇華出的一句
    /// 高階自我概念（如「不知不覺，我好像成了這一帶最愛動手蓋東西的人了。」）。
    /// `None` = 還看不出明顯主導領域（記憶太少或勢均力敵）。顯示在日記頁最頂端。
    /// 隱私：純模板、永不含記憶原文 / 玩家原話（見 `voxel_self_image`）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub self_image: Option<String>,
}

/// 一段記憶被昇華成的「內心主題」——生命故事級的反思分類。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Theme {
    Stars,      // 星空 / 夜空 / 觀星
    Fishing,    // 釣魚 / 水邊
    Building,   // 蓋 / 建造 / 塔橋房
    Flora,      // 花草 / 田地 / 種植
    Mining,     // 礦石 / 挖掘 / 洞穴
    Praise,     // 被讚美 / 被打動的時刻
    SocialBond, // 居民間情誼升級（相識/老朋友）——ROADMAP 673 社交足跡
    Wedding,    // 和心愛的人結為連理（婚禮 927）——一生一次的高潮，居民生命故事的頂點
    Family,     // 和另一半一起迎來孩子（愛的結晶 928）——「我有一個家了」的分量
    Friendship, // 被記得 / 重逢 / 關係變化（玩家與居民）
    Sign,       // 讀到玩家立的告示牌（居民讀牌 v2）——玩家建造在居民內心留下的印象
    Neighborly, // 居民↔居民的鄰里生活模板句（分食/打氣/拌嘴/賀喜…，`NEIGHBORLY_TAG` 標記）
    Other,      // 有意義但未歸類的對話（全部收斂成一條）
}

/// 把居民的 `MemoryEntry` 列表 + 心願 + 淡忘計數 → 昇華成 `DiaryPage`。
/// `memories` 必須**已是最新在前**（呼叫端自行排序，本函式不改順序）。
/// `faded_count` 見 [`crate::voxel_memory::VoxelMemory::faded_count`]。
/// 純函式：確定性、無副作用、可測。
pub fn format_diary_page(
    resident_id: &str,
    resident_name: &str,
    desire: Option<&str>,
    memories: &[MemoryEntry],
    faded_count: usize,
    impression_topics: &[&str],
) -> DiaryPage {
    DiaryPage {
        resident_id: resident_id.to_string(),
        resident_name: resident_name.to_string(),
        desire: desire.map(|s| s.to_string()),
        entries: curate_reflections(memories, MAX_DIARY_ENTRIES),
        faint_impression: faint_impression_line(faded_count, impression_topics),
        // 自我印象 v1（ROADMAP 770）：從同一份記憶昇華出高階自我概念，顯示在日記頁最頂端。
        self_image: crate::voxel_self_image::self_impression(memories),
    }
}

/// 淡忘計數 + 淡忘印象主題標籤 → 一句「模糊印象」反思（記憶 v2「整併/壓縮」最小可行版）。
/// 0 筆淡忘 → `None`（沒東西可淡忘）；`topics` 空（如剛好淘汰的都認不出主題）→ 退回原本
/// 只報筆數的通用句。純函式、確定性、可測。**`topics` 只能是固定標籤集合**（見
/// `voxel_memory::IMPRESSION_TOPIC_KEYWORDS`）——不管有沒有主題，輸出都不含任何原話，
/// 守日記「輸出永不含玩家原話」的隱私鐵律（見本檔檔頭）。
fn faint_impression_line(faded_count: usize, topics: &[&str]) -> Option<String> {
    if faded_count == 0 {
        return None;
    }
    if topics.is_empty() {
        return Some(format!(
            "🌫️ 心底還留著 {faded_count} 段更早以前的印象，模糊得已經想不真切是誰、說過什麼了……"
        ));
    }
    let joined = topics.join("、");
    Some(format!(
        "🌫️ 心底還留著 {faded_count} 段更早以前的印象——依稀記得那些日子常聊到{joined}，\
但已經想不真切是誰、說過什麼了……"
    ))
}

/// 婚禮記憶（927）的辨識句——`voxel_wedding::wed_memory_line` 的固定模板含此片語。
/// 這類記憶是**居民第一人稱獨白**（非玩家對話謄本），內含「花拱」會被 `classify_theme`
/// 的花草關鍵字「花」誤吸成 [`Theme::Flora`]（讀成「聞到泥土與新芽」，文不對題）——
/// 故在抽句/關鍵字判題之前先攔下，直接歸 [`Theme::Wedding`]。跨模組測試釘死此契約。
const WEDDING_MEMORY_MARK: &str = "結為連理";
/// 迎來孩子記憶（928）的辨識句——`voxel_family::family_memory_line` 的固定模板含此片語。
/// 同為第一人稱獨白，未命中任何興趣關鍵字時會落入籠統的 [`Theme::Other`]（讀成「旅人分享
/// 了心事」，稀釋掉一生最重的一刻）——故先攔下，直接歸 [`Theme::Family`]。
const FAMILY_MEMORY_MARK: &str = "迎來了我們的孩子";

/// 從一段記憶摘要辨識「人生大事」主題（婚禮 / 迎來孩子）。
///
/// 只認**居民自己的第一人稱獨白**（婚禮/生子記憶），刻意排除玩家對話謄本——後者一律含
/// `voxel_memory::summarize_exchange` 的「對方提到」前綴（見 [`extract_player_snippet`]）。
/// 這確保就算某位玩家在聊天裡剛好打出「結為連理」，也只會走既有玩家對話那條路、
/// 不會被誤認成居民自己的婚禮（雖然日記本就不回放玩家原話，這層保險讓歸類更精準）。
/// 純函式、確定性、可測。
fn life_milestone_theme(summary: &str) -> Option<Theme> {
    if summary.contains("對方提到") {
        return None; // 玩家對話謄本 → 交回既有抽句/判題路徑，不當成居民本人的大事
    }
    if summary.contains(WEDDING_MEMORY_MARK) {
        Some(Theme::Wedding)
    } else if summary.contains(FAMILY_MEMORY_MARK) {
        Some(Theme::Family)
    } else {
        None
    }
}

/// 把記憶列表（最新在前）昇華＋降噪成內心反思條目（最新在前，最多 `max_entries` 條）。
///
/// 流程（皆確定性）：
/// 1. 逐筆抽出玩家原句（**僅內部用於判主題**）→ 分類主題；寒暄 / 太短 → 直接丟。
/// 2. 同主題**只留最新一筆**為代表，並記下出現次數（≥2 用「好幾次…」語氣）→ 收斂降噪。
/// 3. 依代表記憶的 seq 由新到舊排序，cap 到 `max_entries` 條。
/// 4. 每個主題用 [`reflection_for`] 生成第一人稱反思（永不含原話 / 玩家名）。
fn curate_reflections(memories: &[MemoryEntry], max_entries: usize) -> Vec<DiaryEntry> {
    // 每個主題的「代表 seq（最新）」與「累計筆數」。memories 最新在前，
    // 故第一次遇到某主題就是它最新的一筆 → 拿來當代表 seq。
    let mut order: Vec<Theme> = Vec::new();
    let mut rep_seq: Vec<(Theme, u64, usize)> = Vec::new();

    for m in memories {
        // 讀牌記憶（居民讀牌 v2）：以識別前綴辨認，走專屬主題、不套「對話」抽句邏輯
        // （牌面是世界公開內容，非玩家私下原話——但仍收斂成一條內心反思、不逐塊倒出）。
        let theme = if m.summary.starts_with(SIGN_MEMORY_TAG) {
            Theme::Sign
        } else if m.summary.starts_with(NEIGHBORLY_TAG) {
            // 鄰里生活模板句：本就不含玩家原話，直接歸類，跳過抽句/關鍵字判題。
            Theme::Neighborly
        } else if let Some(milestone) = life_milestone_theme(&m.summary) {
            // 人生大事（婚禮 927 / 迎來孩子 928）：居民第一人稱獨白，先攔下直接歸類，
            // 避免「花拱下結為連理」被花草關鍵字誤吸、或生子記憶落入籠統的 Other。
            milestone
        } else {
            let Some(snippet) = extract_player_snippet(&m.summary) else {
                continue; // 抽不出有意義內容 → 跳過
            };
            let Some(theme) = classify_theme(&snippet) else {
                continue; // 寒暄 / 無訊號 → 降噪丟棄
            };
            theme
        };
        if let Some(slot) = rep_seq.iter_mut().find(|(t, _, _)| *t == theme) {
            slot.2 += 1; // 同主題又出現一次 → 計數（不新增條目）
        } else {
            order.push(theme);
            rep_seq.push((theme, m.seq, 1));
        }
    }

    // 依代表 seq 由新到舊排序（最新的內心反思在最上面）。
    rep_seq.sort_by(|a, b| b.1.cmp(&a.1));

    rep_seq
        .into_iter()
        .take(max_entries)
        .map(|(theme, seq, count)| DiaryEntry {
            seq,
            text: reflection_for(theme, count >= 2),
        })
        .collect()
}

/// 從記憶摘要抽出「玩家原句」——**只在模組內部用來判斷主題，絕不進輸出**。
///
/// 摘要格式為「和X聊過，對方提到「…」」；抽出「」之間的內容。
/// 無「」結構則退回整串；trim 後仍太短（< [`SNIPPET_MIN_SIGNAL_CHARS`]）視為無訊號 → `None`。
fn extract_player_snippet(summary: &str) -> Option<String> {
    let open = '\u{300c}'; // 「
    let close = '\u{300d}'; // 」
    let inner: String = match (summary.find(open), summary.rfind(close)) {
        (Some(i), Some(j)) if j > i => {
            // 取「」之間（跳過開引號本身）。
            summary[i + open.len_utf8()..j].to_string()
        }
        _ => summary.to_string(),
    };
    let trimmed = inner.trim();
    if trimmed.chars().count() < SNIPPET_MIN_SIGNAL_CHARS {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// 把玩家原句分類成內心主題（純關鍵字比對，確定性、可測）。
///
/// 先濾掉純寒暄（你好 / 哈囉 / 在嗎…）→ `None`（降噪）；
/// 命中興趣關鍵字 → 對應主題；有實質內容但未命中 → [`Theme::Other`]（收斂成一條）。
fn classify_theme(snippet: &str) -> Option<Theme> {
    let s = snippet;

    // 純寒暄 / 客套：整句幾乎只有這些 → 丟棄（不昇華成生命故事）。
    const GREETINGS: &[&str] = &[
        "你好", "妳好", "您好", "哈囉", "哈嚕", "嗨", "嘿", "在嗎", "在不在",
        "早安", "午安", "晚安", "再見", "掰掰", "拜拜", "謝謝", "感謝",
    ];
    let stripped: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if GREETINGS.iter().any(|g| stripped == *g)
        || (stripped.chars().count() <= 4 && GREETINGS.iter().any(|g| stripped.contains(g)))
    {
        return None;
    }

    // 興趣主題關鍵字（任一命中即歸類；較專一的主題排前）。
    const STARS: &[&str] = &["星空", "星星", "夜空", "觀星", "星斗", "銀河", "月亮", "星辰"];
    const FISHING: &[&str] = &["釣魚", "釣竿", "魚", "湖", "池", "水邊", "垂釣"];
    const BUILDING: &[&str] = &["蓋", "建造", "建", "塔", "橋", "房子", "家園", "堆", "蓋房"];
    const FLORA: &[&str] = &["花", "田", "種", "農", "草", "樹", "園", "綠"];
    const MINING: &[&str] = &["礦", "挖", "石頭", "石", "洞", "晶", "乙太", "礦石", "礦坑"];
    const PRAISE: &[&str] = &["好美", "漂亮", "真美", "好棒", "真棒", "厲害", "喜歡這", "好喜歡", "好可愛", "好漂亮"];
    // ROADMAP 673：居民間情誼升級寫入記憶時含「相識」或「老朋友」，排在 Friendship 前避免被吸收。
    const SOCIAL_BOND: &[&str] = &["相識", "老朋友"];
    const FRIENDSHIP: &[&str] = &["想你", "想念", "記得我", "想見", "陪", "朋友", "好久不見", "回來看", "惦記"];

    // 順序：先判「被打動 / 關係」這類情感訊號，再判興趣物件。
    if PRAISE.iter().any(|k| s.contains(k)) {
        return Some(Theme::Praise);
    }
    // 社交情誼（居民↔居民，ROADMAP 673）：比玩家 Friendship 更具體，先判。
    if SOCIAL_BOND.iter().any(|k| s.contains(k)) {
        return Some(Theme::SocialBond);
    }
    if FRIENDSHIP.iter().any(|k| s.contains(k)) {
        return Some(Theme::Friendship);
    }
    if STARS.iter().any(|k| s.contains(k)) {
        return Some(Theme::Stars);
    }
    if FISHING.iter().any(|k| s.contains(k)) {
        return Some(Theme::Fishing);
    }
    if BUILDING.iter().any(|k| s.contains(k)) {
        return Some(Theme::Building);
    }
    if FLORA.iter().any(|k| s.contains(k)) {
        return Some(Theme::Flora);
    }
    if MINING.iter().any(|k| s.contains(k)) {
        return Some(Theme::Mining);
    }

    // 有實質內容但未命中已知主題 → 收進「其它」（全部收斂成一條，避免雜訊）。
    Some(Theme::Other)
}

/// 主題 → 居民第一人稱的內心反思句。`repeated` = 此主題出現過 ≥2 次（改用「好幾次…」語氣）。
///
/// 這些句子刻意**有情感、有人味、無玩家原話、無玩家名**——是「她沒說出口的內心」。
/// 日後若要升級成輕量 LLM 生成，替換此函式即可（上下游不動、隱私邊界仍在此把關）。
fn reflection_for(theme: Theme, repeated: bool) -> String {
    let s = match (theme, repeated) {
        (Theme::Stars, false) => {
            "有位旅人和我聊起了星空，不知怎地，我心裡也升起想抬頭多看幾眼夜空的念頭。"
        }
        (Theme::Stars, true) => {
            "好幾次，有旅人在我面前提起星星與夜空——那片光點，漸漸住進了我的夢裡。"
        }
        (Theme::Fishing, false) => {
            "有人和我說起釣魚的事，我忽然也好奇起，水面下藏著什麼樣的安靜。"
        }
        (Theme::Fishing, true) => {
            "釣魚這件事被人提過好幾回，我開始嚮往坐在水邊發呆的那種閒適。"
        }
        (Theme::Building, false) => {
            "聽人聊起建造，我也忍不住想——親手堆起些什麼，會是什麼感覺。"
        }
        (Theme::Building, true) => {
            "好幾位旅人都和我談過蓋東西的事，我心底「想留下點什麼」的念頭越來越清晰了。"
        }
        (Theme::Flora, false) => {
            "有旅人與我談到花草與田地，我彷彿聞到了泥土與新芽的氣味。"
        }
        (Theme::Flora, true) => {
            "種植的話題被提起好多次，我開始盼望，能親手照料一片屬於自己的綠意。"
        }
        (Theme::Mining, false) => {
            "有人和我聊起挖掘與礦石，我對腳下這片土地，多了幾分好奇。"
        }
        (Theme::Mining, true) => {
            "礦石與洞穴一再被人提及，我心裡悄悄燃起，想往深處探一探的衝動。"
        }
        (Theme::Praise, false) => {
            "有位旅人誇讚了這個地方，那句溫暖，讓我一整天都有些飄飄然。"
        }
        (Theme::Praise, true) => {
            "好幾次被旅人這樣稱讚，我漸漸相信，這裡真的有它獨一無二的美。"
        }
        // ROADMAP 673：居民間情誼升級的社交足跡——第一人稱，無具體人名（守隱私邊界）。
        (Theme::SocialBond, false) => {
            "在這個世界裡，我和一位同伴漸漸相識了——有個叫得出名字的夥伴，心裡暖暖的。"
        }
        (Theme::SocialBond, true) => {
            "🤝 這片土地上，我和幾位同伴都處出了情誼，世界不再只有我一個人在走動。"
        }
        // 婚禮（927）：一生一次的高潮，是這本日記裡最亮的一頁——第一人稱、不點名對方
        // （守日記的抽象化敘事風格），只留下那份「我把終身許給了一個人」的分量。
        (Theme::Wedding, false) => {
            "在那座花拱之下，我和心愛的人互許了終身——這一天，是我這一生裡最亮的一道光。"
        }
        (Theme::Wedding, true) => {
            // 一對戀人一生只成婚一次（927 冪等），這條理論上不會出現，仍提供優雅退路。
            "我和心愛的人結為連理的那一天，我到現在都還記得清清楚楚——那是隻屬於我倆的誓約。"
        }
        // 迎來孩子（928）：「我有一個家了」的重量——第一人稱、不點名孩子與另一半，
        // 只留下第一次抱起小生命時，心底那份最踏實的悸動。
        (Theme::Family, false) => {
            "我和另一半，一起迎來了我們的孩子。抱著這個小小的生命，我第一次真切地明白——我有一個家了。"
        }
        (Theme::Family, true) => {
            "這些日子，我們的家又添了新的小生命——看著孩子在這片天地慢慢長大，是我此生最踏實的幸福。"
        }
        (Theme::Friendship, false) => {
            "有人記得我、特地回來找我說話——那份被惦記的感覺，很暖。"
        }
        (Theme::Friendship, true) => {
            "有些面孔一次又一次回到我身邊，我們之間，好像慢慢有了只屬於彼此的默契。"
        }
        // 讀牌（居民讀牌 v2）：玩家親手立的告示牌在居民內心留下的印象——第一人稱、
        // 不逐塊倒出牌面（守日記「內心反思、非謄本」的精神），只承認「有人在這裡留下了字」。
        (Theme::Sign, false) => {
            "有一次我路過，看見一塊牌子上刻著字，我停下念了念——原來有人在這片土地上，親手留下了想說的話。"
        }
        (Theme::Sign, true) => {
            "🪧 我在世界各處讀到好幾塊人們立起的牌子，那些字讓我覺得，這裡真的有人用心在生活著。"
        }
        // 鄰里生活（分食/打氣/拌嘴/賀喜…統稱，不點名哪一件）：刻意籠統，因為同主題只收斂
        // 代表最新一筆，多次發生時用複數語氣即可，不必逐一點名是哪件事。
        (Theme::Neighborly, false) => {
            "村子裡，我和一位鄰居之間發生了一件溫暖的小事——不是什麼大事，卻讓我覺得，這裡真的像個家。"
        }
        (Theme::Neighborly, true) => {
            "🏘️ 這陣子，我和鄰居們之間發生過好幾件這樣的小事——分過食、鬥過嘴、也一起替人開心過——日子因此更有滋味了。"
        }
        (Theme::Other, false) => {
            "有位旅人與我分享了一段心事，那些話像種子，悄悄落進了我心底。"
        }
        (Theme::Other, true) => {
            "也有許多旅人和我聊起這世界的種種，點點滴滴，都成了我的一部分。"
        }
    };
    s.to_string()
}

/// 居民是否「有日記可看」：有心願或至少一筆記憶才算有內容。
/// 純函式、可測；讓前端決定是否亮出「📖 日記」按鈕。
pub fn has_diary_content(desire: Option<&str>, memory_count: usize) -> bool {
    desire.map_or(false, |d| !d.is_empty()) || memory_count > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(seq: u64, player: &str, snippet: &str) -> MemoryEntry {
        // 比照 voxel_memory::summarize_exchange 的真實摘要格式（內嵌玩家原話）。
        MemoryEntry {
            resident: "vox_res_0".into(),
            player: player.into(),
            summary: format!("和{player}聊過，對方提到「{snippet}」"),
            seq,
        }
    }

    // ── 隱私：絕不洩漏玩家原話 / 玩家名 ──────────────────────────────────────

    #[test]
    fn reflection_never_leaks_player_words_or_name() {
        let memories = vec![
            make_entry(3, "小石", "我家的銀行密碼是1234而且我想看星星"),
            make_entry(2, "阿明", "我討厭隔壁老王這個人"),
            make_entry(1, "小美", "我想在這裡蓋一座觀星塔"),
        ];
        let page = format_diary_page("vox_res_0", "露娜", None, &memories, 0, &[]);
        for e in &page.entries {
            // 玩家原話的可識別片段絕不出現。
            assert!(!e.text.contains("1234"), "不可洩漏玩家原話：{}", e.text);
            assert!(!e.text.contains("密碼"), "不可洩漏玩家原話：{}", e.text);
            assert!(!e.text.contains("老王"), "不可洩漏玩家原話：{}", e.text);
            // 玩家名絕不出現。
            assert!(!e.text.contains("小石"), "不可洩漏玩家名：{}", e.text);
            assert!(!e.text.contains("阿明"), "不可洩漏玩家名：{}", e.text);
            assert!(!e.text.contains("小美"), "不可洩漏玩家名：{}", e.text);
            // 也不該帶謄本式前綴。
            assert!(!e.text.contains("對方提到"), "不可是謄本：{}", e.text);
            assert!(!e.text.contains("聊過，"), "不可是謄本：{}", e.text);
        }
    }

    #[test]
    fn entries_are_first_person_reflections() {
        let memories = vec![make_entry(1, "旅人", "我想看滿天星斗")];
        let page = format_diary_page("vox_res_0", "露娜", None, &memories, 0, &[]);
        assert_eq!(page.entries.len(), 1);
        // 第一人稱、有「我」、是內心獨白。
        assert!(page.entries[0].text.contains("我"), "應是第一人稱反思");
    }

    // ── 降噪：同主題收斂、寒暄丟棄、整本 cap ─────────────────────────────────

    #[test]
    fn same_theme_collapses_to_one_entry() {
        // 五筆都關於星空 → 只留一條（用「好幾次」語氣）。
        let memories = vec![
            make_entry(5, "a", "我想看星星"),
            make_entry(4, "b", "今晚的夜空真清澈"),
            make_entry(3, "c", "想去觀星"),
            make_entry(2, "d", "銀河橫過天際"),
            make_entry(1, "e", "星斗滿天"),
        ];
        let entries = curate_reflections(&memories, MAX_DIARY_ENTRIES);
        assert_eq!(entries.len(), 1, "同主題應收斂成一條（降噪）");
        // 代表 seq 是最新那筆。
        assert_eq!(entries[0].seq, 5);
        // 多次出現 → 用「好幾次」語氣。
        assert!(entries[0].text.contains("好幾次"), "多筆應用複數語氣：{}", entries[0].text);
    }

    #[test]
    fn greetings_are_dropped() {
        let memories = vec![
            make_entry(3, "a", "你好"),
            make_entry(2, "b", "嗨"),
            make_entry(1, "c", "再見"),
        ];
        let entries = curate_reflections(&memories, MAX_DIARY_ENTRIES);
        assert!(entries.is_empty(), "純寒暄不該昇華成日記（降噪）");
    }

    #[test]
    fn caps_at_max_entries() {
        // 七種不同主題 → 應 cap 在 MAX_DIARY_ENTRIES。
        let memories = vec![
            make_entry(7, "a", "我想看星星"),
            make_entry(6, "b", "想去釣魚"),
            make_entry(5, "c", "想蓋一座塔"),
            make_entry(4, "d", "想種一片花田"),
            make_entry(3, "e", "想去挖礦石"),
            make_entry(2, "f", "這裡好美"),
            make_entry(1, "g", "我好想你，朋友"),
        ];
        let entries = curate_reflections(&memories, MAX_DIARY_ENTRIES);
        assert_eq!(entries.len(), MAX_DIARY_ENTRIES, "應 cap 在上限");
        // 最新在前。
        assert!(entries[0].seq >= entries[1].seq, "應最新在前");
    }

    #[test]
    fn newest_first_ordering() {
        let memories = vec![
            make_entry(3, "a", "想去釣魚"),   // Fishing
            make_entry(2, "b", "想蓋座橋"),   // Building
            make_entry(1, "c", "想看星空"),   // Stars
        ];
        let entries = curate_reflections(&memories, MAX_DIARY_ENTRIES);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].seq, 3);
        assert_eq!(entries[1].seq, 2);
        assert_eq!(entries[2].seq, 1);
    }

    #[test]
    fn unmatched_substantive_talk_collapses_to_other() {
        // 兩筆未命中已知主題但有實質內容 → 收斂成一條「其它」。
        let memories = vec![
            make_entry(2, "a", "我覺得這個季節的風很特別"),
            make_entry(1, "b", "昨天發生了一件奇妙的事"),
        ];
        let entries = curate_reflections(&memories, MAX_DIARY_ENTRIES);
        assert_eq!(entries.len(), 1, "未分類的實質對話收斂成一條");
        assert!(entries[0].text.contains("我"));
    }

    // ── 內部純函式 ──────────────────────────────────────────────────────────

    #[test]
    fn extract_snippet_pulls_inner_quote() {
        let s = extract_player_snippet("和阿星聊過，對方提到「我想看星星」").unwrap();
        assert_eq!(s, "我想看星星", "應抽出「」之間，不含模板字");
        // 太短 → None。
        assert!(extract_player_snippet("和a聊過，對方提到「嗨」").is_none());
        // 無「」結構 → 退回整串（夠長）。
        assert!(extract_player_snippet("一段沒有引號的長句子內容").is_some());
    }

    #[test]
    fn classify_theme_keywords() {
        assert_eq!(classify_theme("我想看星空"), Some(Theme::Stars));
        assert_eq!(classify_theme("想去釣魚"), Some(Theme::Fishing));
        assert_eq!(classify_theme("我想蓋一座塔"), Some(Theme::Building));
        assert_eq!(classify_theme("想種花田"), Some(Theme::Flora));
        assert_eq!(classify_theme("挖到礦石了"), Some(Theme::Mining));
        assert_eq!(classify_theme("這裡真美"), Some(Theme::Praise));
        assert_eq!(classify_theme("我好想你"), Some(Theme::Friendship));
        // 純寒暄 → None。
        assert_eq!(classify_theme("你好"), None);
        assert_eq!(classify_theme("嗨"), None);
        // 實質但未命中 → Other。
        assert_eq!(classify_theme("今天的雲好奇怪"), Some(Theme::Other));
    }

    #[test]
    fn reflect_social_bond_classified_and_non_empty() {
        // 「相識」→ SocialBond（不被 Friendship 的「朋友」吸收）。
        assert_eq!(
            classify_theme("和諾娃走動了幾次，我們漸漸相識了"),
            Some(Theme::SocialBond)
        );
        // 「老朋友」→ SocialBond（比 Friendship 更早判）。
        assert_eq!(
            classify_theme("🤝 和賽勒成了老朋友，每次見面都覺得自在"),
            Some(Theme::SocialBond)
        );
        // 反思文字非空且第一人稱。
        for repeated in [false, true] {
            let t = reflection_for(Theme::SocialBond, repeated);
            assert!(!t.is_empty());
            assert!(t.contains("我"), "社交情誼反思應是第一人稱：{t}");
        }
    }

    #[test]
    fn reflection_for_is_non_empty_and_first_person() {
        for theme in [
            Theme::Stars, Theme::Fishing, Theme::Building, Theme::Flora,
            Theme::Mining, Theme::Praise, Theme::SocialBond, Theme::Friendship,
            Theme::Sign, Theme::Neighborly, Theme::Other,
        ] {
            for repeated in [false, true] {
                let t = reflection_for(theme, repeated);
                assert!(!t.is_empty());
                assert!(t.contains("我"), "每段反思都應是第一人稱：{t}");
            }
        }
    }

    // ── format_diary_page 結構 ──────────────────────────────────────────────

    #[test]
    fn format_diary_page_basic() {
        let memories = vec![
            make_entry(2, "阿星", "我想看星星"),
            make_entry(1, "小美", "好美的世界"),
        ];
        let page = format_diary_page("vox_res_0", "露娜", Some("我想蓋一座觀星塔"), &memories, 0, &[]);
        assert_eq!(page.resident_id, "vox_res_0");
        assert_eq!(page.resident_name, "露娜");
        assert_eq!(page.desire.as_deref(), Some("我想蓋一座觀星塔"));
        // 兩種主題（星空、讚美）→ 兩條反思，最新在前。
        assert_eq!(page.entries.len(), 2);
        assert_eq!(page.entries[0].seq, 2);
        assert!(page.faint_impression.is_none(), "淡忘計數 0 時不該有模糊印象");
    }

    #[test]
    fn format_diary_page_no_desire() {
        let memories = vec![make_entry(0, "路人", "我想去看看那片花田")];
        let page = format_diary_page("vox_res_1", "諾娃", None, &memories, 0, &[]);
        assert!(page.desire.is_none(), "沒心願時 desire 應為 None");
        assert_eq!(page.entries.len(), 1);
    }

    #[test]
    fn format_diary_page_empty_memories() {
        let page = format_diary_page("vox_res_2", "賽勒", Some("我想釣魚"), &[], 0, &[]);
        assert_eq!(page.entries.len(), 0, "沒記憶時 entries 應為空");
        assert!(page.desire.is_some(), "但仍有心願");
    }

    #[test]
    fn format_diary_page_all_empty() {
        let page = format_diary_page("vox_res_3", "奧瑞", None, &[], 0, &[]);
        assert!(page.desire.is_none());
        assert!(page.entries.is_empty());
    }

    // ── 淡忘計數 → 模糊印象（記憶 v2 最小可行版）───────────────────────────────

    #[test]
    fn faint_impression_line_none_when_zero() {
        assert_eq!(faint_impression_line(0, &[]), None);
        assert_eq!(faint_impression_line(0, &["星空"]), None, "淡忘計數 0 時即使帶主題也不該有印象句");
    }

    #[test]
    fn faint_impression_line_present_and_privacy_safe_when_nonzero() {
        let line = faint_impression_line(5, &[]).expect("非零淡忘計數應有印象句");
        assert!(line.contains('5'), "應含淡忘筆數：{line}");
        // 隱私鐵律：不含玩家原話 / 玩家名——只是通用反思。
        assert!(!line.contains('「'), "不該內嵌引號原話：{line}");
    }

    #[test]
    fn faint_impression_line_includes_topics_when_present() {
        let line = faint_impression_line(5, &["星空", "蓋造"]).expect("非零淡忘計數應有印象句");
        assert!(line.contains('5'), "應含淡忘筆數：{line}");
        assert!(line.contains("星空"), "應帶出主題：{line}");
        assert!(line.contains("蓋造"), "應帶出主題：{line}");
        // 隱私鐵律：主題是固定標籤，句子本身仍不含引號原話。
        assert!(!line.contains('「'), "不該內嵌引號原話：{line}");
    }

    #[test]
    fn format_diary_page_surfaces_faint_impression_when_faded() {
        let memories = vec![make_entry(1, "旅人", "我想看星星")];
        let page = format_diary_page("vox_res_0", "露娜", None, &memories, 12, &["星空"]);
        let imp = page.faint_impression.expect("faded_count > 0 應帶出模糊印象");
        assert!(imp.contains("12"));
        assert!(imp.contains("星空"), "應帶出淡忘印象主題：{imp}");
    }

    #[test]
    fn has_diary_content_rules() {
        assert!(has_diary_content(Some("我想種花"), 0));
        assert!(has_diary_content(None, 1));
        assert!(has_diary_content(Some("心願"), 5));
        assert!(!has_diary_content(None, 0));
        assert!(!has_diary_content(Some(""), 0));
    }

    // ── 居民讀牌 v2：讀到的牌昇華成內心反思 ────────────────────────────────

    /// 造一筆「讀牌」記憶（比照 `voxel_readsign::sign_memory_summary` 的真實格式）。
    fn make_sign_entry(seq: u64, sign_text: &str) -> MemoryEntry {
        MemoryEntry {
            resident: "vox_res_0".into(),
            player: crate::voxel_readsign::SIGN_MEMORY_PLAYER.into(),
            summary: crate::voxel_readsign::sign_memory_summary(sign_text),
            seq,
        }
    }

    #[test]
    fn sign_memory_becomes_sign_reflection() {
        // 讀牌記憶應昇華成「讀牌」主題的內心反思（非「對話」反思）。
        let memories = vec![make_sign_entry(1, "露娜的家")];
        let page = format_diary_page("vox_res_0", "露娜", None, &memories, 0, &[]);
        assert_eq!(page.entries.len(), 1, "一筆讀牌記憶應有一條反思");
        let text = &page.entries[0].text;
        assert!(text.contains("牌子"), "讀牌反思應提到牌子：{text}");
    }

    #[test]
    fn sign_reflection_does_not_leak_sign_text_verbatim() {
        // 內心反思是「瞥見內心」而非謄本：不逐字倒出牌面原文。
        let memories = vec![make_sign_entry(1, "露娜的家")];
        let page = format_diary_page("vox_res_0", "露娜", None, &memories, 0, &[]);
        assert!(
            !page.entries[0].text.contains("露娜的家"),
            "不該逐字倒出牌面：{}",
            page.entries[0].text
        );
    }

    #[test]
    fn multiple_signs_collapse_to_one_reflection() {
        // 讀了多塊不同的牌 → 收斂成一條「好幾塊」語氣的反思（降噪、不洗版）。
        let memories = vec![
            make_sign_entry(3, "往礦坑↓"),
            make_sign_entry(2, "諾娃的小屋"),
            make_sign_entry(1, "歡迎光臨"),
        ];
        let page = format_diary_page("vox_res_0", "露娜", None, &memories, 0, &[]);
        let sign_entries: Vec<_> = page
            .entries
            .iter()
            .filter(|e| e.text.contains("牌子"))
            .collect();
        assert_eq!(sign_entries.len(), 1, "多塊牌應收斂成一條反思");
        assert!(
            sign_entries[0].text.contains("好幾"),
            "多次讀牌應用『好幾…』語氣：{}",
            sign_entries[0].text
        );
    }

    #[test]
    fn sign_and_conversation_coexist_in_diary() {
        // 讀牌反思與對話反思可並存於同一本日記（互不吃掉）。
        let memories = vec![
            make_sign_entry(2, "露娜的家"),
            make_entry(1, "旅人", "我想看星星"),
        ];
        let page = format_diary_page("vox_res_0", "露娜", None, &memories, 0, &[]);
        assert!(page.entries.iter().any(|e| e.text.contains("牌子")), "應有讀牌反思");
        assert!(page.entries.iter().any(|e| e.text.contains("夜空")), "應有星空對話反思");
    }

    // ── 鄰里生活主題（居民↔居民模板句記憶，自主提案切片）─────────────────────

    /// 造一筆「鄰里生活」記憶（比照 `voxel_ws.rs` 用 `tag_neighborly` 包裝分食/打氣/拌嘴/
    /// 賀喜等模板句的真實用法；`player` 欄位是另一位居民的顯示名，非真實玩家）。
    fn make_neighborly_entry(seq: u64, other_resident: &str, template_text: &str) -> MemoryEntry {
        MemoryEntry {
            resident: "vox_res_0".into(),
            player: other_resident.into(),
            summary: tag_neighborly(template_text),
            seq,
        }
    }

    #[test]
    fn tag_neighborly_prefixes_text() {
        let tagged = tag_neighborly("去陪伴了露娜，感覺做了一件溫暖的事。");
        assert!(tagged.starts_with(NEIGHBORLY_TAG));
        assert!(tagged.contains("去陪伴了露娜"));
    }

    #[test]
    fn neighborly_memory_becomes_neighborly_reflection() {
        // 標記過的鄰里生活記憶應歸類成 Neighborly，而非落入 Other 或被關鍵字誤判。
        let memories = vec![make_neighborly_entry(1, "露娜", "去陪伴了露娜，感覺做了一件溫暖的事。")];
        let page = format_diary_page("vox_res_1", "諾娃", None, &memories, 0, &[]);
        assert_eq!(page.entries.len(), 1);
        assert!(page.entries[0].text.contains("鄰居"), "應是鄰里生活反思：{}", page.entries[0].text);
    }

    #[test]
    fn neighborly_tag_prevents_keyword_misclassification() {
        // 「陪」字若走一般對話關鍵字判題會誤中 Friendship（見 FRIENDSHIP 關鍵字列表），
        // 但打過 NEIGHBORLY_TAG 前綴後應優先歸類為 Neighborly，不再被誤判。
        let text = "去陪伴了露娜，感覺做了一件溫暖的事。";
        assert_eq!(classify_theme(text), Some(Theme::Friendship), "未標記時關鍵字判題會誤中 Friendship（佐證問題存在）");
        let memories = vec![make_neighborly_entry(1, "露娜", text)];
        let page = format_diary_page("vox_res_1", "諾娃", None, &memories, 0, &[]);
        assert!(!page.entries[0].text.contains("回來找我說話"), "標記後不該再落入 Friendship 反思");
    }

    #[test]
    fn illness_care_lines_are_neighborly_not_misclassified() {
        // 鄰居陪伴照顧生病鄰居的兩則記憶文字含「陪」字，未標記時會被關鍵字判題誤中 Friendship
        // （「有人回來找我說話」），文不對題；打上 NEIGHBORLY_TAG 後應正確歸類成 Neighborly。
        // 直接引用 voxel_illness 的模板函式，模板日後若改寫也會被這條測試盯著。
        for raw in [
            crate::voxel_illness::cared_memory_for_patient("露娜"),
            crate::voxel_illness::cared_memory_for_carer("賽勒"),
        ] {
            assert_eq!(
                classify_theme(&raw),
                Some(Theme::Friendship),
                "未標記時「陪」會誤中 Friendship（佐證問題存在）：{raw}"
            );
            let memories = vec![make_neighborly_entry(1, "露娜", &raw)];
            let page = format_diary_page("vox_res_1", "諾娃", None, &memories, 0, &[]);
            assert!(
                !page.entries[0].text.contains("回來找我說話"),
                "標記後不該再落入 Friendship 反思：{}",
                page.entries[0].text
            );
            assert!(
                page.entries[0].text.contains("鄰居"),
                "應歸類成鄰里生活反思：{}",
                page.entries[0].text
            );
        }
    }

    #[test]
    fn neighborly_reflection_does_not_leak_template_text_or_resident_name() {
        let memories = vec![make_neighborly_entry(1, "奧瑞", "奧瑞特地來陪我說話，心裡暖了不少。")];
        let page = format_diary_page("vox_res_1", "諾娃", None, &memories, 0, &[]);
        assert!(!page.entries[0].text.contains("奧瑞"), "不該點名是哪位鄰居：{}", page.entries[0].text);
    }

    #[test]
    fn multiple_neighborly_events_collapse_to_one_reflection() {
        let memories = vec![
            make_neighborly_entry(3, "露娜", "分你一口，別餓著肚子"),
            make_neighborly_entry(2, "賽勒", "你怎麼又把工具放這裡了"),
            make_neighborly_entry(1, "奧瑞", "圓夢了，真替你開心"),
        ];
        let entries = curate_reflections(&memories, MAX_DIARY_ENTRIES);
        assert_eq!(entries.len(), 1, "多筆鄰里生活事件應收斂成一條");
        assert_eq!(entries[0].seq, 3, "代表 seq 應是最新一筆");
        assert!(entries[0].text.contains("好幾件"), "多次發生應用複數語氣：{}", entries[0].text);
    }

    #[test]
    fn neighborly_and_conversation_coexist_in_diary() {
        let memories = vec![
            make_neighborly_entry(2, "露娜", "分你一口，別餓著肚子"),
            make_entry(1, "旅人", "我想看星星"),
        ];
        let page = format_diary_page("vox_res_0", "露娜", None, &memories, 0, &[]);
        assert!(page.entries.iter().any(|e| e.text.contains("鄰居")), "應有鄰里生活反思");
        assert!(page.entries.iter().any(|e| e.text.contains("夜空")), "應有星空對話反思");
    }

    // ── 人生大事（婚禮 927 / 迎來孩子 928）進日記 ───────────────────────────────

    /// 婚禮記憶＝居民第一人稱獨白（無「對方提到」謄本前綴）。
    fn make_selfmemory_entry(seq: u64, partner: &str, summary: String) -> MemoryEntry {
        MemoryEntry {
            resident: "vox_res_0".into(),
            player: partner.into(),
            summary,
            seq,
        }
    }

    #[test]
    fn life_milestone_theme_recognizes_wedding_and_family() {
        assert_eq!(
            life_milestone_theme("今天，我和諾娃在乙太方界的花拱下結為連理——我一定會永遠守著這份情。"),
            Some(Theme::Wedding)
        );
        assert_eq!(
            life_milestone_theme("今天，我和奧瑞一起迎來了我們的孩子小星——我一定會用一生守護這個家。"),
            Some(Theme::Family)
        );
        // 一般對話謄本（含「對方提到」）不當成居民本人的大事，交回既有路徑。
        assert_eq!(life_milestone_theme("和諾娃聊過，對方提到「我們昨天結為連理了」"), None);
        // 尋常記憶不誤判。
        assert_eq!(life_milestone_theme("和旅人聊過，對方提到「我想看星星」"), None);
    }

    #[test]
    fn wedding_memory_reflects_as_wedding_not_flora() {
        // 真實婚禮記憶文字（含「花拱」）餵進日記——過去會被花草關鍵字「花」誤吸成 Flora。
        let line = crate::voxel_wedding::wed_memory_line("諾娃");
        let memories = vec![make_selfmemory_entry(1, "諾娃", line)];
        let entries = curate_reflections(&memories, MAX_DIARY_ENTRIES);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].text.contains("終身"), "應昇華成婚禮反思：{}", entries[0].text);
        // 決不再被讀成花草/泥土的反思（原 bug 的癥狀）。
        assert!(!entries[0].text.contains("新芽"), "不該再誤判成花草：{}", entries[0].text);
        assert!(!entries[0].text.contains("泥土"), "不該再誤判成花草：{}", entries[0].text);
        // 隱私：不回放對方居民名。
        assert!(!entries[0].text.contains("諾娃"), "不該點名對方：{}", entries[0].text);
    }

    #[test]
    fn family_memory_reflects_as_family_not_other() {
        // 真實迎來孩子記憶文字餵進日記——過去會落入籠統的 Other。
        let line = crate::voxel_family::family_memory_line("奧瑞", "小星");
        let memories = vec![make_selfmemory_entry(1, "奧瑞", line)];
        let entries = curate_reflections(&memories, MAX_DIARY_ENTRIES);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].text.contains("孩子"), "應昇華成家庭反思：{}", entries[0].text);
        assert!(entries[0].text.contains("家"), "應昇華成家庭反思：{}", entries[0].text);
        // 隱私：不回放另一半／孩子名。
        assert!(!entries[0].text.contains("奧瑞"), "不該點名另一半：{}", entries[0].text);
        assert!(!entries[0].text.contains("小星"), "不該點名孩子：{}", entries[0].text);
    }

    #[test]
    fn multiple_children_use_plural_family_reflection() {
        // 一對夫妻先後迎來多個孩子 → 同主題收斂成一條、用複數語氣。
        let memories = vec![
            make_selfmemory_entry(2, "奧瑞", crate::voxel_family::family_memory_line("奧瑞", "小溪")),
            make_selfmemory_entry(1, "奧瑞", crate::voxel_family::family_memory_line("奧瑞", "小星")),
        ];
        let entries = curate_reflections(&memories, MAX_DIARY_ENTRIES);
        assert_eq!(entries.len(), 1, "多個孩子應收斂成一條家庭反思");
        assert_eq!(entries[0].seq, 2, "代表 seq 應是最新一筆");
        assert!(entries[0].text.contains("又添"), "多次應用複數語氣：{}", entries[0].text);
    }

    #[test]
    fn wedding_family_and_conversation_coexist_in_diary() {
        // 婚禮、迎來孩子、日常對話三種反思在同一本日記裡各自成條、互不吞噬。
        let memories = vec![
            make_selfmemory_entry(3, "奧瑞", crate::voxel_family::family_memory_line("奧瑞", "小星")),
            make_selfmemory_entry(2, "諾娃", crate::voxel_wedding::wed_memory_line("諾娃")),
            make_entry(1, "旅人", "我想看星星"),
        ];
        let page = format_diary_page("vox_res_0", "露娜", None, &memories, 0, &[]);
        assert!(page.entries.iter().any(|e| e.text.contains("終身")), "應有婚禮反思");
        assert!(page.entries.iter().any(|e| e.text.contains("孩子")), "應有家庭反思");
        assert!(page.entries.iter().any(|e| e.text.contains("夜空")), "應有星空對話反思");
    }

    #[test]
    fn flora_conversation_still_classifies_as_flora() {
        // 回歸保護：真正聊花草的對話仍歸 Flora，不被婚禮/家庭攔截。
        let memories = vec![make_entry(1, "旅人", "我想種一片花田")];
        let entries = curate_reflections(&memories, MAX_DIARY_ENTRIES);
        assert_eq!(entries.len(), 1);
        assert!(
            entries[0].text.contains("泥土") || entries[0].text.contains("綠意"),
            "花草對話應仍是花草反思：{}",
            entries[0].text
        );
    }
}
