//! 乙太方界·營火 v1（campfire）——玩家蓋一處火堆，夜幕降臨時附近醒著的居民會聚到火邊
//! 圍坐取暖、說句暖心話、心情變好；你也在旁時記一筆「一起圍爐」的交情。
//!
//! **這一刀補的缺口**：世界至今有火把（純照明）、乙太煙火（朝夜空一次性綻放），卻沒有一處
//! 能讓「玩家的建造真的改變居民行為」的溫暖聚點。營火把這一環補上——玩家親手在世界裡點一處
//! 火堆，**入夜後路過火邊的居民會不由自主駐足圍暖**：你蓋越多火堆、夜裡的世界越熱鬧溫暖。
//! 這是「玩家的建造 → 塑造居民的夜間社交場所」的第一刀，路線圖「小社會湧現」的環境驅動版。
//!
//! **與既有元素刻意區隔**：火把只照明、不牽動居民；乙太煙火是一次性升空綻放、朝天不聚人；
//! 睹物思人（784）是居民追憶**特定一件你送的紀念物**；本模組是**夜間限定、任何居民都適用、
//! 群聚感、由火堆位置吸引路過者**的取暖。
//!
//! **純函式層**：本模組只有確定性純函式（就近判定、三閘、台詞、掃描重建），零 LLM、零鎖、
//! 零 async、零 IO、可單元測試。連線／鎖／廣播／持久化觸發全留在 `voxel_ws.rs`（沿用
//! 既有短鎖循序＋鎖外事件佇列慣例，守 prod 死鎖鐵律）。

use crate::voxel;

/// 營火方塊／物品 ID（70：60~69 已被純物品佔用，70 是首個可放置方塊空號）。
pub const CAMPFIRE_ID: u8 = 70;

/// 居民駐足取暖半徑（世界方塊；水平距離）——夠近才算「圍到火邊」。
pub const WARM_RADIUS: f32 = 4.0;
/// 取暖冷卻（秒）：一位居民取暖後隔這麼久才會再取暖，防同一居民狂刷泡泡。
pub const WARM_COOLDOWN_SECS: f32 = 100.0;
/// 每次符合條件（夜晚＋靠近火＋冷卻到期）時的取暖觸發機率——其餘時候只是安靜路過。
pub const WARM_CHANCE: f32 = 0.30;
/// 冬寒圍爐 v1（ROADMAP 901）：冬季（非飄雪）取暖機率。冬天不分晝夜都冷，湊到火邊
/// 取暖的意願高於平時夜裡（WARM_CHANCE）。
pub const WINTER_WARM_CHANCE: f32 = 0.45;
/// 冬季正飄雪時的取暖機率——外頭下著雪，最想窩在火邊；為三檔之最高。
pub const SNOW_WARM_CHANCE: f32 = 0.60;
/// 「你也在火邊」的判定半徑（世界方塊）——你在這麼近，居民的暖語就會點你名、記進交情。
pub const WARM_PLAYER_RADIUS: f32 = 6.0;

/// 從營火座標清單中找出離 `(rx, rz)` 最近、且在 `radius` 內的一座（回索引）。
///
/// y 忽略（火光在水平面上吸引；居民與火堆通常同一地表高度）。同距取索引最小者，
/// None = 半徑內沒有火堆（居民這一 tick 不在任何火邊）。
pub fn nearest_campfire(spots: &[(i32, i32, i32)], rx: f32, rz: f32, radius: f32) -> Option<usize> {
    let r2 = radius * radius;
    let mut best: Option<(usize, f32)> = None;
    for (i, &(x, _, z)) in spots.iter().enumerate() {
        let dx = x as f32 + 0.5 - rx;
        let dz = z as f32 + 0.5 - rz;
        let d2 = dx * dx + dz * dz;
        if d2 <= r2 && best.map_or(true, |(_, bd)| d2 < bd) {
            best = Some((i, d2));
        }
    }
    best.map(|(i, _)| i)
}

/// 三閘判定：靠近火（`near`）＋冷卻到期（`cooldown <= 0`）＋過機率門檻（`roll < chance`）
/// → 這一 tick 駐足取暖。純函式，好窮舉測邊界。
pub fn should_warm(near: bool, cooldown: f32, roll: f32, chance: f32) -> bool {
    near && cooldown <= 0.0 && roll < chance
}

/// 取暖泡泡台詞（通用、不點名）——五句輪替，字數短不破泡泡框。`pick` 由呼叫端用
/// 座標 bits 合成，讓每次挑到的句子自然分散。
pub fn warm_bubble(pick: usize) -> &'static str {
    const LINES: [&str; 5] = [
        "這火真暖，讓我烤一會兒。",
        "夜裡有堆火，心也跟著暖了。",
        "呼……火邊真舒服。",
        "圍著火坐著，什麼煩惱都散了。",
        "這火堆真好，謝謝有人點起它。",
    ];
    LINES[pick % LINES.len()]
}

/// 你也在火邊時的取暖泡泡（點名玩家，更暖）——四句輪替，玩家名截斷不破泡泡框。
pub fn warm_bubble_with_player(player: &str, pick: usize) -> String {
    let name = clip_name(player);
    const TEMPLATES: [&str; 4] = [
        "{name}，一起烤烤火吧，夜裡暖和些。",
        "有{name}在火邊，這夜就不冷了。",
        "{name}，你也來圍火啦？真好。",
        "跟{name}一起圍著火，暖進心裡。",
    ];
    TEMPLATES[pick % TEMPLATES.len()].replace("{name}", &name)
}

/// 昇華成一筆「和你一起圍爐」的記憶（點名玩家、不含換行，走既有 append-only 記憶管線）。
pub fn warm_memory_line(player: &str) -> String {
    format!("夜裡和{}一起圍著營火取暖，暖進了心裡。", clip_name(player)).replace('\n', " ")
}

/// 動態牆播報（非同步層，訪客回來能讀到誰在火邊取過暖）。
pub fn warm_feed_line(rname: &str) -> String {
    format!("{rname}在營火邊烤了會兒火，暖暖的。")
}

// ── 冬寒圍爐 v1（ROADMAP 901）：把冬雪（900）× 季節（798）× 營火（791）第一次扣在一起 ──
// 換維度而非疊維度：營火取暖原本「夜間限定＋路過被動」，本刀讓「冬季寒冷」第一次真正驅動
// 居民行為——冬天裡（尤其飄雪時）不分晝夜都想湊到火邊取暖，念一句點明天寒的冷天暖語。
// 純函式層零 LLM／零鎖／可測；接線（何時備妥火堆快照、選機率／台詞／Feed）留在 voxel_ws.rs。

/// 這一刻營火是否「值得取暖」——夜裡（原本行為）或冬天（不分晝夜都冷）。
/// 決定 voxel_ws 是否備妥營火座標快照：兩者皆非時整段判定零成本跳過。
pub fn warming_active(is_night: bool, is_winter: bool) -> bool {
    is_night || is_winter
}

/// 依季節／天氣算這一刻的取暖觸發機率：非冬季沿用 WARM_CHANCE；冬季更想烤火、
/// 冬季又正飄雪時最想烤火。回傳值恆夾在 `[0,1]`。
pub fn warm_chance_for(is_winter: bool, snowing: bool) -> f32 {
    let c = if is_winter {
        if snowing {
            SNOW_WARM_CHANCE
        } else {
            WINTER_WARM_CHANCE
        }
    } else {
        WARM_CHANCE
    };
    c.clamp(0.0, 1.0)
}

/// 冬寒版通用暖語（無玩家在旁）——明確點出天寒，與泛用 `warm_bubble` 完全不重疊。
/// 刻意不寫死「正在下雪」（冬季未必飄雪），只扣「冷」，避免不飄雪時說謊。
pub fn cold_warm_bubble(pick: usize) -> &'static str {
    const LINES: [&str; 4] = [
        "天寒地凍的，這火堆真是救命。",
        "外頭冷得直哆嗦，火邊暖和多了。",
        "呼……手都凍紅了，烤烤火才活過來。",
        "這樣的冷天，就得守著一堆火。",
    ];
    LINES[pick % LINES.len()]
}

/// 冬寒版點名暖語（玩家在火邊）——點你名、記進交情，語氣扣著冬天。
pub fn cold_warm_bubble_with_player(player: &str, pick: usize) -> String {
    let name = clip_name(player);
    const TEMPLATES: [&str; 4] = [
        "{name}，天這麼冷，快來火邊暖暖手。",
        "有{name}一起守著火，這寒冬也不難熬了。",
        "{name}，這麼冷的天，湊近點一起烤火吧。",
        "跟{name}一起圍著火過冬，心裡也暖。",
    ];
    TEMPLATES[pick % TEMPLATES.len()].replace("{name}", &name)
}

/// 冬寒版動態牆文案——上 Feed 時明確帶出「寒冬／取暖」。
pub fn cold_warm_feed_line(rname: &str) -> String {
    format!("{rname}在寒冬裡守著營火烤暖了身子。")
}

/// 冬寒版記憶文案——玩家在旁時掛玩家名下（把「一起圍火過冬」記進交情）。
pub fn cold_warm_memory_line(player: &str) -> String {
    format!("寒冬裡和{}一起圍著營火取暖，暖進了心裡。", clip_name(player)).replace('\n', " ")
}

// ── 雨澆營火 v1：天氣（下雨 700）× 營火（791／901）第一次交會 ──────────────────
// 換維度而非疊維度：901「冬寒圍爐」讓寒冷「更想」圍暖（推高機率），本刀反向——下雨把火堆
// 打得又濕又弱、圍不到暖：雨中營火取暖整段被抑制（不再湊到火邊），偶爾路過熄弱火堆的居民
// 停下腳步、對著滋滋作響的濕柴嘆一句。雨一停、火重新旺起來，大家又聚回火邊。
// 純函式層零 LLM／零鎖／可測；接線（雨閘、選機率／台詞）留在 voxel_ws.rs。

/// 雨中路過熄弱火堆時、停下嘆一句的觸發機率——刻意低：多數時候只是安靜淋著雨走過。
/// 比取暖機率更罕見（取暖是常態、雨中感嘆是偶爾），與 [`WARM_CHANCE`] 拉開。
pub const RAIN_DOUSE_CHANCE: f32 = 0.12;

/// 雨閘：下雨時營火取暖是否被抑制。純函式（`raining` 即抑制），給接線一個具名可測接縫。
/// 語意＝「火被雨澆得太弱、圍不到暖」，回 `true` 表示這一刻不該觸發取暖。
pub fn warming_suppressed_by_rain(raining: bool) -> bool {
    raining
}

/// 雨中感嘆判定：靠近火（`near`）＋冷卻到期（`cooldown <= 0`）＋過機率門檻（`roll < RAIN_DOUSE_CHANCE`）
/// → 這一 tick 停下對著濕柴嘆一句。與 [`should_warm`] 同型三閘，好窮舉測邊界。
pub fn should_douse_lament(near: bool, cooldown: f32, roll: f32) -> bool {
    near && cooldown <= 0.0 && roll < RAIN_DOUSE_CHANCE
}

/// 雨澆營火感嘆台詞（通用、不點名）——四句輪替，字數短不破泡泡框，明確扣「雨／濕／澆熄」，
/// 與泛用 [`warm_bubble`]／冬寒 [`cold_warm_bubble`] 完全不重疊（沒有一句說「暖」）。
/// 語氣仍守療癒基調：是對火被雨奪走那份暖的一點惋惜，不是抱怨或沮喪。
pub fn rain_douse_line(pick: usize) -> &'static str {
    const LINES: [&str; 4] = [
        "這雨把火都快澆熄了，圍不到暖啊。",
        "唉，濕柴滋滋響，火怎麼也旺不起來。",
        "雨天的火堆冷冷清清，等雨停了再來烤吧。",
        "雨水順著火堆滴下來，暖意都被澆走了。",
    ];
    LINES[pick % LINES.len()]
}

/// 掃描整個 world delta，找出所有仍是營火的方塊座標（啟動時重建取暖清單用）。
///
/// 純函式（吃 delta overlay 的當前值）：只認 delta 裡目前值 == 營火的格；被破壞成空氣的
/// 舊營火格值已是 Air，自然不會被撈出來。反解 chunk 局部索引 → 世界座標，與 `local_index`
/// 的行主序（`lx + lz*CHUNK + ly*CHUNK*CHUNK`）對齊。
pub fn scan_campfires(world: &voxel::WorldDelta) -> Vec<(i32, i32, i32)> {
    let c = voxel::CHUNK;
    let mut out = Vec::new();
    for (coord, cd) in world.iter() {
        for (&li, &b) in cd.iter() {
            if b as u8 == CAMPFIRE_ID {
                let li = li as i32;
                let lx = li % c;
                let lz = (li / c) % c;
                let ly = li / (c * c);
                out.push((coord.cx * c + lx, coord.cy * c + ly, coord.cz * c + lz));
            }
        }
    }
    out
}

/// 玩家名截斷到 8 字（中文安全，避免超長名撐破泡泡框）。
fn clip_name(name: &str) -> String {
    name.chars().take(8).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voxel::{self, Block};

    #[test]
    fn nearest_picks_closest_within_radius() {
        // (10,0,10) 較遠、(3,0,2) 較近；居民在 (2,2) 附近 → 挑第二座（索引 1）。
        let spots = [(10, 64, 10), (3, 64, 2)];
        assert_eq!(nearest_campfire(&spots, 2.0, 2.0, WARM_RADIUS), Some(1));
    }

    #[test]
    fn nearest_none_when_all_out_of_radius() {
        let spots = [(50, 64, 50)];
        assert_eq!(nearest_campfire(&spots, 0.0, 0.0, WARM_RADIUS), None);
        // 空清單也回 None（沒任何火堆）。
        assert_eq!(nearest_campfire(&[], 0.0, 0.0, WARM_RADIUS), None);
    }

    #[test]
    fn nearest_same_dist_takes_smallest_index() {
        // 兩座對稱等距 → 取索引最小者（索引 0）。
        let spots = [(1, 64, 0), (-1, 64, 0)];
        // 居民在原點旁 (0.5,0.0)：到 (1,0) dx=1.0；到 (-1,0) dx=-1.5 → 其實不等距。
        // 改用真正對稱點：居民在 x=0.5 兩側 → (0,_,0) 與 (1,_,0) 對 rx=0.5 等距。
        let sym = [(0, 64, 0), (1, 64, 0)];
        assert_eq!(nearest_campfire(&sym, 0.5, 0.5, WARM_RADIUS), Some(0));
        let _ = spots;
    }

    #[test]
    fn should_warm_needs_all_three_gates() {
        // 三閘齊備才觸發。
        assert!(should_warm(true, 0.0, 0.1, WARM_CHANCE));
        // 不在火邊 → 否。
        assert!(!should_warm(false, 0.0, 0.1, WARM_CHANCE));
        // 冷卻未到 → 否。
        assert!(!should_warm(true, 5.0, 0.1, WARM_CHANCE));
        // 骰過門檻 → 否；邊界 roll == chance 不觸發（嚴格小於）。
        assert!(!should_warm(true, 0.0, WARM_CHANCE, WARM_CHANCE));
        assert!(!should_warm(true, 0.0, 0.99, WARM_CHANCE));
    }

    #[test]
    fn bubbles_rotate_and_stay_in_frame() {
        // 通用暖語輪替、非空。
        for p in 0..10 {
            assert!(!warm_bubble(p).is_empty());
        }
        assert_ne!(warm_bubble(0), warm_bubble(1));
        // 點名版含玩家名、輪替、超長名截斷不破框（≤ 名截斷 8 字 + 模板短語）。
        let s = warm_bubble_with_player("旅人", 0);
        assert!(s.contains("旅人"));
        let long = warm_bubble_with_player("超級無敵長長長長長長長名字", 2);
        assert!(long.chars().count() < 40, "超長名應被截斷不破泡泡框：{long}");
    }

    #[test]
    fn memory_and_feed_embed_names_no_newline() {
        let m = warm_memory_line("諾娃\n注入");
        assert!(m.contains("諾娃"));
        assert!(!m.contains('\n'), "記憶不得含換行：{m}");
        let f = warm_feed_line("露娜");
        assert!(f.contains("露娜"));
    }

    #[test]
    fn scan_finds_only_campfires() {
        // 在 delta 裡放兩座營火與一塊石頭 → scan 只撈出兩座營火的座標。
        let mut world = voxel::WorldDelta::new();
        voxel::set_block(&mut world, 5, 64, 7, Block::Campfire);
        voxel::set_block(&mut world, -3, 30, 12, Block::Campfire);
        voxel::set_block(&mut world, 1, 1, 1, Block::Stone);
        let mut found = scan_campfires(&world);
        found.sort();
        let mut want = vec![(5, 64, 7), (-3, 30, 12)];
        want.sort();
        assert_eq!(found, want);
    }

    #[test]
    fn scan_skips_broken_campfire() {
        // 放一座營火再破壞成空氣（delta 覆蓋 Air）→ scan 不再撈出它。
        let mut world = voxel::WorldDelta::new();
        voxel::set_block(&mut world, 8, 64, 8, Block::Campfire);
        voxel::set_block(&mut world, 8, 64, 8, Block::Air);
        assert!(scan_campfires(&world).is_empty());
    }

    // ── 冬寒圍爐 v1（ROADMAP 901）── //

    #[test]
    fn warming_active_night_or_winter() {
        // 夜裡（原本行為）或冬天皆需備妥火堆快照；白天且非冬季才整段跳過。
        assert!(warming_active(true, false), "夜裡：需取暖");
        assert!(warming_active(false, true), "冬季白天：也需取暖（本刀新行為）");
        assert!(warming_active(true, true), "冬夜：更需取暖");
        assert!(!warming_active(false, false), "非冬季白天：跳過");
    }

    #[test]
    fn warm_chance_scales_with_cold() {
        // 三檔嚴格遞增：非冬季 < 冬季不飄雪 < 冬季飄雪；且皆等於對應常數。
        let base = warm_chance_for(false, false);
        let winter = warm_chance_for(true, false);
        let snowing = warm_chance_for(true, true);
        assert_eq!(base, WARM_CHANCE);
        assert_eq!(winter, WINTER_WARM_CHANCE);
        assert_eq!(snowing, SNOW_WARM_CHANCE);
        assert!(base < winter && winter < snowing, "越冷越想烤火");
        // 非冬季時 snowing 旗標無意義（不飄雪），仍取基礎值。
        assert_eq!(warm_chance_for(false, true), WARM_CHANCE);
    }

    #[test]
    fn warm_chance_always_in_range() {
        for &w in &[true, false] {
            for &s in &[true, false] {
                let c = warm_chance_for(w, s);
                assert!((0.0..=1.0).contains(&c), "機率須夾在[0,1]：{c}");
            }
        }
    }

    #[test]
    fn cold_bubbles_rotate_distinct_and_in_frame() {
        // 冬寒通用暖語輪替、非空、且與泛用版本完全不重疊。
        for p in 0..8 {
            assert!(!cold_warm_bubble(p).is_empty());
            assert_ne!(
                cold_warm_bubble(p),
                warm_bubble(p),
                "冬寒版須與泛用版互異，避免同軸重複"
            );
        }
        assert_ne!(cold_warm_bubble(0), cold_warm_bubble(1));
        // 點名版含玩家名、輪替、超長名截斷不破泡泡框。
        let s = cold_warm_bubble_with_player("旅人", 0);
        assert!(s.contains("旅人"));
        let long = cold_warm_bubble_with_player("超級無敵長長長長長長長名字", 2);
        assert!(long.chars().count() < 40, "超長名應被截斷不破框：{long}");
    }

    #[test]
    fn cold_memory_and_feed_embed_names_no_newline() {
        // 記憶不得含換行（防注入破壞記憶庫）；動態牆帶名。
        let m = cold_warm_memory_line("諾娃\n注入");
        assert!(m.contains("諾娃"));
        assert!(!m.contains('\n'), "記憶不得含換行：{m}");
        let f = cold_warm_feed_line("露娜");
        assert!(f.contains("露娜") && f.contains("寒冬"));
    }

    // ── 雨澆營火 v1（天氣 700 × 營火）─────────────────────────────────────────

    #[test]
    fn warming_suppressed_only_when_raining() {
        // 雨閘：下雨才抑制取暖，晴天不抑制。
        assert!(warming_suppressed_by_rain(true));
        assert!(!warming_suppressed_by_rain(false));
    }

    #[test]
    fn douse_lament_needs_near_cooldown_and_roll() {
        // 三閘：近火＋冷卻到期＋roll < RAIN_DOUSE_CHANCE 才嘆一句。
        assert!(should_douse_lament(true, 0.0, 0.0));
        assert!(should_douse_lament(true, 0.0, RAIN_DOUSE_CHANCE - 0.001));
        // 機率門檻邊界：roll == chance 不觸發（嚴格小於）。
        assert!(!should_douse_lament(true, 0.0, RAIN_DOUSE_CHANCE));
        // 不近火不觸發。
        assert!(!should_douse_lament(false, 0.0, 0.0));
        // 冷卻未到期不觸發。
        assert!(!should_douse_lament(true, 1.0, 0.0));
    }

    #[test]
    fn douse_lament_rarer_than_warming() {
        // 雨中感嘆刻意比取暖罕見：門檻低於平時取暖機率。
        assert!(RAIN_DOUSE_CHANCE < WARM_CHANCE);
    }

    #[test]
    fn rain_douse_line_non_empty_and_wraps() {
        // 每句非空、pick 循環穩定、字數短不破泡泡框。
        for pick in 0..12 {
            let s = rain_douse_line(pick);
            assert!(!s.is_empty());
            assert!(s.chars().count() <= 25, "台詞應短不破框：{s}");
        }
        assert_eq!(rain_douse_line(0), rain_douse_line(4));
        assert_eq!(rain_douse_line(1), rain_douse_line(5));
    }

    #[test]
    fn rain_douse_lines_mention_rain_imagery() {
        // 雨澆台詞必扣「雨／濕／澆／滴」其一（與泛用／冬寒暖語 razor-sharp 區隔：暖語從不提雨）。
        for pick in 0..4 {
            let s = rain_douse_line(pick);
            assert!(
                s.contains('雨') || s.contains('濕') || s.contains('澆') || s.contains('滴'),
                "雨澆台詞應扣雨意象：{s}"
            );
        }
        // 反向確認：泛用暖語與冬寒暖語都不提雨（區隔清晰）。
        for pick in 0..5 {
            assert!(!warm_bubble(pick).contains('雨'));
        }
    }
}
