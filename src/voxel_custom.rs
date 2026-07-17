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

/// 廣場邊閒話家常的泡泡台詞（通用、不點名，字數短不破泡泡框）。
///
/// 依「在場人數 `headcount`」×「座標合成的 `pick`」挑句：獨自坐著、三兩人搭話、一群人熱鬧，
/// 語氣各不同，讓黃昏暮聚的閒聊不再千篇一律。`pick` 由呼叫端用座標 bits 合成，
/// 讓同一人數檔內每次挑到的句子也自然分散。
///
/// `has_monument`：這場暮聚是不是圍在主村真的立起來的中央紀念柱（村碑）腳下——殖民地暮聚 v1
/// 只有奠基小廣場、沒有紀念柱，故換一組不提「村碑」的台詞，避免殖民地居民講出不存在的地標。
pub fn chatter_bubble(pick: usize, headcount: usize, has_monument: bool) -> &'static str {
    // 三檔語氣（與 memory_templates 的 headcount_tier 同分檔精神）：獨自 / 三兩人 / 一群人。
    // 每檔各兩組：有村碑（主村）與無村碑（殖民地），保零回歸又依人數散開。
    const ALONE: [&str; 3] = [
        "黃昏了，一個人先在廣場邊坐坐、看著天色暗下來。",
        "這時候靜一靜，心裡也踏實。",
        "夕陽把周遭染暖了，我一個人也不覺得冷清。",
    ];
    const ALONE_MON: [&str; 3] = [
        "黃昏了，一個人先到村碑邊坐坐、看著天色暗下來。",
        "在村碑邊靜一靜，心裡也踏實。",
        "夕陽把村子染暖了，我一個人也不覺得冷清。",
    ];
    const SMALL: [&str; 3] = [
        "你今天過得怎麼樣？我來聽聽。",
        "難得碰上了，來廣場邊聊幾句。",
        "你也來啦？正好一塊兒說說話。",
    ];
    const SMALL_MON: [&str; 3] = [
        "你今天過得怎麼樣？我來聽聽。",
        "在村碑邊搭句話，這黃昏就不悶了。",
        "你也來啦？正好在村碑下說說話。",
    ];
    const CROWD: [&str; 3] = [
        "大家都在，這一天就算圓滿了。",
        "人這麼齊，廣場邊熱熱鬧鬧的真好。",
        "每到這時候大夥都湊過來，像趕集似的。",
    ];
    const CROWD_MON: [&str; 3] = [
        "大家都在，這一天就算圓滿了。",
        "一村子的人聚在村碑下，說笑聲不斷。",
        "每到這時候大夥都湊到村碑邊，像趕集似的。",
    ];
    let tier = match headcount {
        0 | 1 => 0,
        2..=3 => 1,
        _ => 2,
    };
    let pool: &[&str; 3] = match (tier, has_monument) {
        (0, true) => &ALONE_MON,
        (0, false) => &ALONE,
        (1, true) => &SMALL_MON,
        (1, false) => &SMALL,
        (_, true) => &CROWD_MON,
        (_, false) => &CROWD,
    };
    pool[pick % pool.len()]
}

/// 動態牆播報句（帶季節、人數與「本季第 N 場」，有「來歷感」——道出這是這座聚落入夜前的固定
/// 習俗，且愈積愈多場、成了整季反覆的老習慣）。
///
/// - `season_zh`：當前季節顯示名（如「深秋」）。
/// - `count`：這場暮聚的參與人數。
/// - `place`：聚集地點描述（主村＝「村莊廣場的村碑邊」、殖民地＝「『風禾屯』的村心廣場」）。
/// - `label`：收尾指稱這座聚落的詞（主村＝「村子」、殖民地＝聚落名本身，如「風禾屯」）。
/// - `nth_this_season`：本季第幾場暮聚（1 起算；0 視同第 1 次）——第一場帶「今季頭一回」的
///   起頭感，往後帶「本季第 N 場」的積累感，讓動態牆播報不再每天同一句。
pub fn gather_feed_line(season_zh: &str, count: usize, place: &str, label: &str, nth_this_season: usize) -> String {
    let nth = nth_this_season.max(1);
    if nth == 1 {
        return format!(
            "🌆 {season_zh}第一次黃昏，{count} 位居民不約而同地聚到{place}閒話家常——這一季的暮聚，就從今天在{label}起頭了。"
        );
    }
    format!("🌆 {season_zh}的黃昏，{count} 位居民又不約而同地聚到{place}閒話家常——這已是本季第 {nth} 場，成了{label}入夜前的老習慣。")
}

/// 參與暮聚的居民寫進記憶的一句（episodic、第一人稱內心，累積「村子有了自己的習俗、我屬於這裡」
/// 的歸屬感）。不含任何玩家名／私密渴望，適用於任何一位在場居民；單行、無換行（jsonl 一行一筆）。
///
/// 依「本季第 `nth_this_season` 次暮聚」×「在場人數 `headcount`」×「季節中文名 `season_zh`」
/// 借用記憶模板庫的 [`crate::voxel_memory_templates::dusk_gather_line`]，把 prod 上反覆同句
/// （曾達 1410 次）的黃昏記憶依上下文散開、去罐頭。
pub fn gather_memory_line(nth_this_season: usize, headcount: usize, season_zh: &str) -> String {
    crate::voxel_memory_templates::dusk_gather_line(nth_this_season, headcount, season_zh)
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
            for hc in [1usize, 3, 8] {
                for p in 0..12 {
                    let b = chatter_bubble(p, hc, has_monument);
                    assert!(!b.is_empty());
                    assert!(b.chars().count() <= CHATTER_CHARS, "閒聊泡泡應在上限內：{b}");
                }
                assert_ne!(
                    chatter_bubble(0, hc, has_monument),
                    chatter_bubble(1, hc, has_monument),
                    "同一人數檔內台詞應輪替"
                );
            }
        }
    }

    #[test]
    fn chatter_headcount_changes_wording() {
        // 同 pick、同紀念柱狀態，只有人數不同 → 至少獨自 vs 一群人措辭不同（依人數散開）。
        use std::collections::HashSet;
        let alone = chatter_bubble(0, 1, true);
        let small = chatter_bubble(0, 3, true);
        let crowd = chatter_bubble(0, 8, true);
        let set: HashSet<_> = [alone, small, crowd].into_iter().collect();
        assert_eq!(set.len(), 3, "獨自/三兩人/一群人的閒聊應各不同句");
    }

    #[test]
    fn chatter_no_monument_pool_never_mentions_pillar() {
        // 殖民地暮聚沒有紀念柱：三檔人數的替代台詞皆不得提「村碑」，避免居民講出不存在的地標。
        for hc in [1usize, 3, 8] {
            for p in 0..12 {
                assert!(!chatter_bubble(p, hc, false).contains('碑'), "殖民地台詞不應提及村碑");
            }
        }
        // 主村台詞池至少有一句仍提及村碑（零回歸：原有村碑氣氛不變）。
        assert!(
            (0..3).any(|p| chatter_bubble(p, 8, true).contains('碑')),
            "主村一群人的台詞應仍有村碑指涉"
        );
    }

    #[test]
    fn feed_line_embeds_season_count_place_label_no_newline() {
        let f = gather_feed_line("深秋", 3, "村莊廣場的村碑邊", "村子", 2);
        assert!(f.contains("深秋"));
        assert!(f.contains('3'));
        assert!(f.contains("村莊廣場的村碑邊"));
        assert!(f.contains("村子"));
        assert!(!f.contains('\n'), "Feed 不得含換行");
        assert!(!f.is_empty());
    }

    #[test]
    fn feed_line_colony_variant_names_colony_not_village() {
        let f = gather_feed_line("盛夏", 2, "「風禾屯」的村心廣場", "風禾屯", 2);
        assert!(f.contains("風禾屯"));
        assert!(!f.contains("村子"), "殖民地播報不該說成『村子』入夜前的習慣");
        assert!(!f.contains('\n'));
    }

    #[test]
    fn feed_line_first_of_season_differs_from_later() {
        // 本季第一場 vs 往後場，措辭不同（不再每天同一句）。
        let first = gather_feed_line("深秋", 3, "村莊廣場的村碑邊", "村子", 1);
        let later = gather_feed_line("深秋", 3, "村莊廣場的村碑邊", "村子", 3);
        assert_ne!(first, later, "本季頭一場與往後場的播報應措辭不同");
        assert!(later.contains("第 3 場"), "往後場應嵌入本季場次：{later}");
        assert!(!first.contains('\n') && !later.contains('\n'));
    }

    #[test]
    fn memory_line_diversifies_by_context() {
        use std::collections::HashSet;
        // 黃昏記憶去罐頭核心保證：同一件事（暮聚）放進多種上下文 → 多樣句子，不再千篇一律。
        let mut seen: HashSet<String> = HashSet::new();
        for nth in [1usize, 2, 5] {
            for hc in [1usize, 3, 8] {
                for season in ["春天", "秋天"] {
                    let m = gather_memory_line(nth, hc, season);
                    assert!(!m.is_empty());
                    assert!(!m.contains('\n'), "記憶不得含換行（jsonl 一行一筆）");
                    // episodic 內心句，不該外洩玩家名占位符。
                    assert!(!m.contains('{'));
                    assert!(m.contains(season), "應嵌入季節名：{m}");
                    seen.insert(m);
                }
            }
        }
        // 3 nth × 3 headcount × 2 season = 18 組上下文，去重後應遠多於 1（不罐頭）。
        assert!(seen.len() >= 10, "黃昏記憶在不同上下文應產出多樣句子，實得 {}", seen.len());
    }
}
