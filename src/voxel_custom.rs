//! 乙太方界·村莊自發習俗 v1（暮聚·village custom）——村子第一次自己長出一個**會重複的小習俗**：
//! 每到黃昏，住在村莊廣場（中央紀念柱）附近、手邊正閒著的居民，會不約而同地慢慢晃到廣場邊，
//! 三三兩兩地聚在一塊閒話家常、看著天色一點點暗下來，直到夜色漸濃才各自散去回家。
//!
//! **這一刀補的缺口**：村子至今有廣場、有路、有中央紀念柱（885 村碑）、有季節與日夜循環，
//! 也有各式各樣的**單次**活動（圍火講古、小圈子相約、集會鐘），但**沒有一個「全村共享、
//! 週而復始、玩家路過就撞得見」的習俗**——沒有「文化」。本刀讓村莊自發長出第一個：一個
//! **會週期觸發（每日黃昏一次）、以村莊廣場為圓心、任何在場閒著的居民都會自然加入**的暮聚。
//! 這是路線圖北極星「AI 居民湧現出一個有文化的小社會」的第一塊文化拼圖。
//!
//! **來歷感（呼應「每一磚都有來歷」）**：暮聚不是憑空排程的活動——它**只在村子已經豎起中央
//! 紀念柱（有了廣場中心）之後才會發生**，聚集點正是居民自己合力立起的那根村碑腳下。習俗因
//! 「這裡成了我們的家、有了一處大家都認得的中心」而自然生長出來，而非天上掉下來的節目表。
//!
//! **與既有聚集刻意區隔**：
//! - 圍火講古（campfire_tale）＝夜裡、玩家蓋的營火邊、兩人分享往事；
//! - 小圈子聚會（clique 711）＝互為老朋友的特定小團體、偶發、地點在某位成員家；
//! - 集會鐘（bell）＝玩家主動敲鐘召集。
//! 本刀＝**黃昏、村莊廣場、全村不限交情、週期自發**——時段、地點、成員、觸發全不同，是村子
//! **自己**的固定習俗，不需玩家或特定交情牽線。
//!
//! **純函式層**：本模組只有確定性純函式（觸發時機三閘、參與者挑選、閒聊台詞、Feed／記憶句），
//! 零 LLM、零鎖、零 async、零 IO，可單元測試。走路／等待／say／持久化觸發全留在 `voxel_ws.rs`
//! （沿用既有小圈子聚會的閒晃中心偏移與鎖外事件佇列慣例，守 prod 死鎖鐵律）。
//!
//! **殖民地暮聚 v1（自主提案切片，接續「殖民地真居住」943 v1 明確不動清單「暮聚…主村限定」）**：
//! 暮聚原本只認主村廣場中心——像風禾屯這樣遷居而成的殖民地離主村太遠，從沒被暮聚吸引過，第二村
//! 至今沒有自己的黃昏聚會。呼叫端把「聚落中心」一般化成每座已奠基聚落各自獨立的暮聚候選地
//! （主村＋每座殖民地，各自的「今天聚過沒」互不影響），本檔的純函式維持聚落無關（`should_hold`／
//! `select_participants` 本就只吃座標與人數，不需改動）；只有 `chatter_bubble`／`gather_feed_line`
//! 兩處面向玩家的字串需要「這是不是主村」的旗標，換一組不提「村碑」（殖民地無紀念柱）的台詞、
//! 點名是哪座聚落聚起來了。

/// 觸發暮聚所需的最少在場閒人數：至少這麼多位居民同時有空，才值得聚（一個人不算聚會）。
pub const MIN_PARTICIPANTS: usize = 2;
/// 一場暮聚最多拉進幾位居民（村子人多時也不必全員到齊，留一部分各過各的日子才自然）。
pub const MAX_PARTICIPANTS: usize = 6;
/// 「算在場」的半徑（世界方塊，XZ 平面）：住在村莊這麼大範圍內、手邊正閒著的居民都會被暮聚
/// 吸引到廣場——取「涵蓋整座村莊 footprint」的尺度（居民家域由村莊佈局散在村中心四周數十格內），
/// 好讓暮聚是**全村**的習俗、而非只有廣場正旁那一兩位。真正遠在荒野邊陲遠行／已遷去分村的居民
/// 不受影響（他們帶著 expedition／frontier_visit 等旗標，早被在場閒人判定排除，不會被硬拉回來）。
pub const GATHER_RADIUS: f32 = 90.0;
/// 抵達廣場後「聚在一塊」的閒晃半徑（方塊）：比家域小，讓一群人看起來湊在廣場邊，不散開。
pub const GATHER_WANDER_RADIUS: f32 = 5.0;
/// 一場暮聚的逗留時長（秒）：被拉進暮聚的居民朝廣場走、聚著閒晃這麼久，然後各自散去回家
/// （也兼作走不到的逾時保險：等這麼久還沒融入就放棄，守「卡住自救」不鬼打牆）。取值需涵蓋
/// 「最遠的居民以夜間降速走到廣場」＋「到場後聚著閒聊一陣」——黃昏起算會延續到入夜初，
/// 天更暗前各自散去回家（不會拖到深夜就地睡在廣場）。
pub const LINGER_SECS: f32 = 150.0;
/// 站到廣場邊、算「到場了」的判定距離（方塊）——夠近才會冒閒聊泡泡。
pub const CHATTER_NEAR_DIST: f32 = 7.0;
/// 每 tick 在廣場邊冒一句閒話家常的機率——低頻，讓聊天零零星星像真的閒聊、不洗版。
pub const CHATTER_CHANCE: f32 = 0.02;
/// 閒聊泡泡字元上限（與既有社交泡泡同框，超長截斷不破框）。
pub const CHATTER_CHARS: usize = 40;
/// 動態牆 Feed 分類。
pub const FEED_KIND: &str = "村莊習俗";

/// 觸發三閘：現在是黃昏（`is_dusk`）＋今天還沒聚過（`!already_today`）＋村子已有廣場中心
/// （`has_center`）＋在場閒人數達門檻（`free_in_radius >= min`）→ 這一 tick 開一場暮聚。
///
/// 純函式，好窮舉測邊界。「今天有沒有聚過」由呼叫端以世界累計日數比對（每日黃昏至多一場），
/// 「在場閒人數」由 [`select_participants`] 的結果長度提供。
pub fn should_hold(is_dusk: bool, already_today: bool, has_center: bool, free_in_radius: usize, min: usize) -> bool {
    is_dusk && !already_today && has_center && free_in_radius >= min
}

/// 從候選居民中挑出這場暮聚的參與者（回索引清單）。
///
/// `candidates`：每位居民一筆 `(索引, 到廣場中心距離平方, 是否有空)`。
/// `radius`：吸引半徑（方塊）——只有「有空且在半徑內」的居民才會被暮聚吸引。
/// `max`：一場最多拉幾位。
///
/// 規則：濾掉沒空或太遠的 → 依距廣場中心由近到遠排序（近的先加入，畫面上先在廣場邊聚起來）→
/// 取前 `max` 位。同距時取索引較小者（穩定、可測）。純函式、確定性。
pub fn select_participants(candidates: &[(usize, f32, bool)], radius: f32, max: usize) -> Vec<usize> {
    let r2 = radius * radius;
    let mut in_range: Vec<(usize, f32)> = candidates
        .iter()
        .filter(|&&(_, d2, free)| free && d2 <= r2)
        .map(|&(i, d2, _)| (i, d2))
        .collect();
    // 由近到遠；同距取索引小者（穩定排序 + 次鍵）。
    in_range.sort_by(|a, b| {
        a.1.partial_cmp(&b.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
    in_range.into_iter().take(max).map(|(i, _)| i).collect()
}

/// 廣場邊閒話家常的泡泡台詞（通用、不點名、六句輪替，字數短不破泡泡框）。
/// `pick` 由呼叫端用座標 bits 合成，讓每次挑到的句子自然分散。
///
/// `has_monument`：這場暮聚是不是圍在主村真的立起來的中央紀念柱（村碑）腳下——殖民地暮聚 v1
/// 只有奠基小廣場、沒有紀念柱，故換一句不提「村碑」的台詞，避免殖民地居民講出不存在的地標。
pub fn chatter_bubble(pick: usize, has_monument: bool) -> &'static str {
    const LINES: [&str; 6] = [
        "黃昏了，來廣場邊坐坐、說說話。",
        "今天過得怎麼樣？我來聽聽。",
        "每到這時候聚一聚，心裡就踏實。",
        "你看這夕陽，把村子都染暖了。",
        "大家都在，這一天就算圓滿了。",
        "在村碑邊閒聊幾句，真好。",
    ];
    const LINES_NO_MONUMENT: [&str; 6] = [
        "黃昏了，來廣場邊坐坐、說說話。",
        "今天過得怎麼樣？我來聽聽。",
        "每到這時候聚一聚，心裡就踏實。",
        "你看這夕陽，把這裡都染暖了。",
        "大家都在，這一天就算圓滿了。",
        "能在這片新落腳的地方聚聚，真好。",
    ];
    let pool = if has_monument { &LINES } else { &LINES_NO_MONUMENT };
    pool[pick % pool.len()]
}

/// 動態牆播報句（帶季節與人數，有「來歷感」——道出這是這座聚落入夜前的固定習俗）。
/// `season_zh` 為當前季節顯示名（如「深秋」）、`count` 為這場暮聚的參與人數、`place` 為聚集地點
/// 描述（主村＝「村莊廣場的村碑邊」、殖民地＝「『風禾屯』的村心廣場」）、`label` 為收尾指稱這座
/// 聚落的詞（主村＝「村子」、殖民地＝聚落名本身，如「風禾屯」）。
pub fn gather_feed_line(season_zh: &str, count: usize, place: &str, label: &str) -> String {
    format!("🌆 {season_zh}的黃昏，{count} 位居民又不約而同地聚到{place}閒話家常——這已成了{label}入夜前的老習慣。")
}

/// 一段字串的確定性雜湊種子（FNV-1a 64-bit）——把居民 id、季節、聚落名等上下文
/// 攪成一個數，供模板選句用。純函式、跨平台結果一致（不同於 `DefaultHasher` 不保證穩定），
/// 讓「同一位居民＋同一天＋同一場」永遠選到同一句、但換人／換天／換季就自然錯開。
fn seed_of(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// 參與暮聚的居民寫進記憶的一句（episodic、第一人稱內心，累積「村子有了自己的習俗、我屬於這裡」
/// 的歸屬感）。不含任何玩家名／私密渴望，適用於任何一位在場居民；單行、無換行（jsonl 一行一筆）。
///
/// **多樣性（純程式、零 LLM）**：舊版對每位參與者、每一天、每一季都寫同一句固定罐頭
/// （prod 稽核：這一句在 24405 筆記憶裡重複 1410 次，是最大單一重複源）。現在把記憶句拆成
/// 三段可交叉的維度——①**開場**帶當季（季節詞 × 4）與地點（有無村碑 × 主村／聚落名）、
/// ②**中段**描寫這場的氣氛（隨在場人數多寡分「小聚／熱鬧」兩路各數句）、③**收尾**那句
/// 歸屬感——每一段都由 `(居民 id ⊕ 季節 ⊕ 聚落 ⊕ 世界日)` 的確定性種子各自選句。
/// 於是同輸入永遠同句（可重現、好測），但**換人／換季／換天／換聚落任一維度變動就換句**，
/// 把單一罐頭爆成上百種組合。語義不變：仍是「黃昏聚到廣場、屬於這裡」的第一人稱歸屬感。
///
/// 參數：`resident_id` 居民識別碼（如 `vox_res_3`，穩定的個人維度）、`season_zh` 當季顯示名
/// （如「秋天」）、`count` 這場參與人數、`place_desc` 聚集地點描述（同 [`gather_feed_line`]，
/// 主村＝「村莊廣場的村碑邊」、殖民地＝「『風禾屯』的村心廣場」）、`label` 收尾指稱這座聚落
/// 的詞（主村＝「村子」、殖民地＝聚落名）、`has_monument` 是否圍在真的村碑腳下（殖民地無）、
/// `day` 世界累計日數（讓同一位居民在不同天寫下不同句、記憶隨日子推移而變）。
pub fn gather_memory_line(
    resident_id: &str,
    season_zh: &str,
    count: usize,
    place_desc: &str,
    label: &str,
    has_monument: bool,
    day: u64,
) -> String {
    // 三段各用不同鹽攪出獨立種子，避免三段被同一數字綁死、選到相關聯的句子。
    let base = seed_of(resident_id) ^ seed_of(season_zh) ^ seed_of(label) ^ day.wrapping_mul(0x9e37_79b9);
    let s_open = (base ^ 0x1111_1111_1111_1111) as usize;
    let s_mood = (base ^ 0x2222_2222_2222_2222) as usize;
    let s_belong = (base ^ 0x3333_3333_3333_3333) as usize;

    // ── ① 開場：帶當季 + 地點。村碑腳下與無村碑的聚落各一組（不讓殖民地講出不存在的村碑）。
    let open = if has_monument {
        const OPEN_MON: [&str; 4] = [
            "{season}的黃昏一到，我又晃到了{place}，",
            "每逢{season}入夜前，我總不由自主走向{place}，",
            "{season}的暮色裡，我照例來到{place}，",
            "天要暗了，這{season}的黃昏我還是慢慢踱到{place}，",
        ];
        OPEN_MON[s_open % OPEN_MON.len()]
    } else {
        const OPEN_NOMON: [&str; 4] = [
            "{season}的黃昏一到，我又晃到了{place}，",
            "每逢{season}入夜前，我總會走到{place}，",
            "{season}的暮色裡，我照例來到{place}，",
            "天要暗了，這{season}的黃昏我還是慢慢踱到{place}，",
        ];
        OPEN_NOMON[s_open % OPEN_NOMON.len()]
    };

    // ── ② 中段：氣氛隨在場人數分兩路。人少＝三兩人小聚、人多＝一村熱鬧。
    let mood = if count <= 2 {
        const MOOD_FEW: [&str; 4] = [
            "和三兩位鄰居坐下說幾句話。",
            "就我們幾個湊在一塊，慢慢聊著今天。",
            "人不多，倒也自在，彼此問候幾句。",
            "和身邊的老鄰居閒閒地說著話。",
        ];
        MOOD_FEW[s_mood % MOOD_FEW.len()]
    } else {
        const MOOD_MANY: [&str; 4] = [
            "大夥兒都聚了過來，說說笑笑好不熱鬧。",
            "一村的人都到了，你一句我一句地聊開。",
            "這麼多人圍在一起，笑聲一直沒停過。",
            "熟面孔一個個都來了，聚成滿滿一圈。",
        ];
        MOOD_MANY[s_mood % MOOD_MANY.len()]
    };

    // ── ③ 收尾：歸屬感那句（點名聚落，語義不變）。
    const BELONG: [&str; 4] = [
        "這已成了{label}的習慣，也讓我覺得，這裡真的是我的家。",
        "這樣的黃昏聚會，是{label}才有的暖，讓我打從心底安穩。",
        "一天到這時候才算圓滿——{label}的人在，我就在家。",
        "日子這樣過著，{label}於我，早不只是落腳處了。",
    ];
    let belong = BELONG[s_belong % BELONG.len()];

    format!("{open}{mood}{belong}")
        .replace("{season}", season_zh)
        .replace("{place}", place_desc)
        .replace("{label}", label)
        .replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_hold_needs_all_gates() {
        // 四閘齊備才觸發。
        assert!(should_hold(true, false, true, 2, MIN_PARTICIPANTS));
        // 不是黃昏 → 否。
        assert!(!should_hold(false, false, true, 5, MIN_PARTICIPANTS));
        // 今天已聚過 → 否（每日至多一場）。
        assert!(!should_hold(true, true, true, 5, MIN_PARTICIPANTS));
        // 村子還沒有廣場中心 → 否（習俗尚未生根）。
        assert!(!should_hold(true, false, false, 5, MIN_PARTICIPANTS));
        // 在場閒人不足門檻 → 否（一個人不算聚會）。
        assert!(!should_hold(true, false, true, 1, MIN_PARTICIPANTS));
        // 邊界：恰好達門檻 → 是。
        assert!(should_hold(true, false, true, MIN_PARTICIPANTS, MIN_PARTICIPANTS));
    }

    #[test]
    fn select_filters_free_and_in_range() {
        // idx0 有空近；idx1 沒空（濾掉）；idx2 有空但太遠（濾掉）；idx3 有空稍遠但在半徑內。
        let cands = vec![
            (0, 4.0, true),
            (1, 1.0, false),
            (2, 10000.0, true),
            (3, 100.0, true),
        ];
        let got = select_participants(&cands, GATHER_RADIUS, MAX_PARTICIPANTS);
        assert_eq!(got, vec![0, 3], "只留有空且在半徑內者，並由近到遠排序");
    }

    #[test]
    fn select_sorts_by_distance_then_index() {
        // 全部有空、皆在半徑內：由近到遠；idx2 與 idx4 同距 → 取索引小者在前。
        let cands = vec![
            (0, 50.0, true),
            (2, 9.0, true),
            (4, 9.0, true),
            (1, 1.0, true),
        ];
        let got = select_participants(&cands, GATHER_RADIUS, MAX_PARTICIPANTS);
        assert_eq!(got, vec![1, 2, 4, 0], "近的先、同距取索引小者");
    }

    #[test]
    fn select_caps_at_max() {
        let cands: Vec<(usize, f32, bool)> = (0..10).map(|i| (i, i as f32, true)).collect();
        let got = select_participants(&cands, GATHER_RADIUS, 3);
        assert_eq!(got, vec![0, 1, 2], "最多取 max 位（最近的幾位）");
    }

    #[test]
    fn select_empty_when_none_eligible() {
        // 全沒空 → 空；全太遠 → 空；空清單 → 空。
        assert!(select_participants(&[(0, 1.0, false)], GATHER_RADIUS, MAX_PARTICIPANTS).is_empty());
        assert!(select_participants(&[(0, 1e9, true)], GATHER_RADIUS, MAX_PARTICIPANTS).is_empty());
        assert!(select_participants(&[], GATHER_RADIUS, MAX_PARTICIPANTS).is_empty());
    }

    #[test]
    fn chatter_rotates_and_fits_frame() {
        for has_monument in [true, false] {
            for p in 0..12 {
                let b = chatter_bubble(p, has_monument);
                assert!(!b.is_empty());
                assert!(b.chars().count() <= CHATTER_CHARS, "閒聊泡泡應在上限內：{b}");
            }
            assert_ne!(
                chatter_bubble(0, has_monument),
                chatter_bubble(1, has_monument),
                "台詞應輪替"
            );
        }
    }

    #[test]
    fn chatter_no_monument_pool_never_mentions_pillar() {
        // 殖民地暮聚沒有紀念柱：整組替代台詞不得提「村碑」，避免居民講出不存在的地標。
        for p in 0..12 {
            assert!(!chatter_bubble(p, false).contains('碑'), "殖民地台詞不應提及村碑");
        }
        // 主村台詞池至少有一句仍照舊提及村碑（零回歸：原有台詞氣氛不變）。
        assert!((0..6).any(|p| chatter_bubble(p, true).contains('碑')));
    }

    #[test]
    fn feed_line_embeds_season_count_place_label_no_newline() {
        let f = gather_feed_line("深秋", 3, "村莊廣場的村碑邊", "村子");
        assert!(f.contains("深秋"));
        assert!(f.contains('3'));
        assert!(f.contains("村莊廣場的村碑邊"));
        assert!(f.contains("村子"));
        assert!(!f.contains('\n'), "Feed 不得含換行");
        assert!(!f.is_empty());
    }

    #[test]
    fn feed_line_colony_variant_names_colony_not_village() {
        let f = gather_feed_line("盛夏", 2, "「風禾屯」的村心廣場", "風禾屯");
        assert!(f.contains("風禾屯"));
        assert!(!f.contains("村子"), "殖民地播報不該說成『村子』入夜前的習慣");
        assert!(!f.contains('\n'));
    }

    #[test]
    fn feed_line_main_village_wording_unchanged() {
        // 零回歸鎖點：呼叫端傳入主村原本寫死的 place/label 時，字面必須與這一刀之前逐字相同。
        let f = gather_feed_line("深秋", 3, "村莊廣場的村碑邊", "村子");
        assert_eq!(
            f,
            "🌆 深秋的黃昏，3 位居民又不約而同地聚到村莊廣場的村碑邊閒話家常——這已成了村子入夜前的老習慣。"
        );
    }

    #[test]
    fn memory_line_single_line_nonempty_no_leak() {
        let m = gather_memory_line("vox_res_0", "秋天", 3, "村莊廣場的村碑邊", "村子", true, 12);
        assert!(!m.is_empty());
        assert!(!m.contains('\n'), "記憶不得含換行（jsonl 一行一筆）");
        // 占位符全數被替換，不該外洩 {season}/{place}/{label} 骨架。
        assert!(!m.contains('{'), "占位符未替換乾淨：{m}");
    }

    #[test]
    fn memory_line_deterministic_same_input_same_output() {
        // 同一位居民、同一天、同一場 → 永遠同句（可重現、好稽核、方便測）。
        let a = gather_memory_line("vox_res_3", "夏天", 5, "村莊廣場的村碑邊", "村子", true, 7);
        let b = gather_memory_line("vox_res_3", "夏天", 5, "村莊廣場的村碑邊", "村子", true, 7);
        assert_eq!(a, b, "同輸入應產生同句");
    }

    #[test]
    fn memory_line_embeds_season_and_label() {
        // 語義維度真的進到句子裡：當季詞、聚落指稱都看得到。
        let m = gather_memory_line("vox_res_1", "冬天", 4, "村莊廣場的村碑邊", "村子", true, 3);
        assert!(m.contains("冬天"), "應帶當季：{m}");
        assert!(m.contains("村子"), "應點名聚落：{m}");
    }

    #[test]
    fn memory_line_colony_never_mentions_monument() {
        // 殖民地（無村碑）不論怎麼交叉都不得講出不存在的村碑地標。
        for id in ["vox_res_0", "vox_res_4", "vox_res_7", "vox_res_9"] {
            for season in ["春天", "夏天", "秋天", "冬天"] {
                for day in 0..8u64 {
                    for count in [1usize, 3, 6] {
                        let m = gather_memory_line(
                            id, season, count, "「風禾屯」的村心廣場", "風禾屯", false, day,
                        );
                        assert!(!m.contains('碑'), "殖民地記憶不應提及村碑：{m}");
                    }
                }
            }
        }
    }

    #[test]
    fn memory_line_diversity_explodes_across_context() {
        // 稽核止血核心：舊版對每位參與者每天每季都寫同一句（prod 24405 筆裡重複 1410 次）。
        // 現在跨（居民 × 季節 × 世界日 × 人數）交叉，唯一句數應遠多於 1——這裡窮舉一小片
        // 上下文空間，斷言產出至少數十種不同句子（實際組合遠不止此）。
        let ids = ["vox_res_0", "vox_res_1", "vox_res_2", "vox_res_3", "vox_res_4"];
        let seasons = ["春天", "夏天", "秋天", "冬天"];
        let mut set = std::collections::HashSet::new();
        for id in ids {
            for season in seasons {
                for day in 0..6u64 {
                    for count in [1usize, 5] {
                        set.insert(gather_memory_line(
                            id, season, count, "村莊廣場的村碑邊", "村子", true, day,
                        ));
                    }
                }
            }
        }
        // 240 種輸入組合。舊版全部塌成 1 句；新版應開出大量不同句子。
        assert!(
            set.len() >= 40,
            "暮聚記憶多樣性不足（僅 {} 種），罐頭化未解決",
            set.len()
        );
    }

    #[test]
    fn memory_line_count_switches_mood_wording() {
        // 人少走「小聚」、人多走「熱鬧」兩路措辭——確認人數維度真的影響句子。
        // 掃過各種上下文，確保「兩三人」與「一村人」在同一開場/收尾種子下能產生不同中段。
        let few_only_words = ["三兩", "幾個", "人不多"];
        let many_only_words = ["大夥兒", "一村", "這麼多人", "一個個"];
        let mut saw_few_marker = false;
        let mut saw_many_marker = false;
        for id in ["vox_res_0", "vox_res_2", "vox_res_5"] {
            let few = gather_memory_line(id, "秋天", 1, "村莊廣場的村碑邊", "村子", true, 4);
            let many = gather_memory_line(id, "秋天", 6, "村莊廣場的村碑邊", "村子", true, 4);
            if few_only_words.iter().any(|w| few.contains(w)) {
                saw_few_marker = true;
            }
            if many_only_words.iter().any(|w| many.contains(w)) {
                saw_many_marker = true;
            }
        }
        assert!(saw_few_marker, "人少場應出現『小聚』措辭");
        assert!(saw_many_marker, "人多場應出現『熱鬧』措辭");
    }
}
