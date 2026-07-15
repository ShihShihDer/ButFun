//! 乙太方界·跨村商隊 v1／v2／v3（自主提案切片 ROADMAP 950／997／1007，PLAN_ETHERVOX §7
//! 「居民散佈世界各處住」×「記憶→行為」——殖民地生活閉環裡缺席的最後一塊：物資流通）。
//!
//! **真缺口（v1）**：殖民地真居住（943）讓拓荒者真的搬去了第二村，兩村相思（945）與相思成行·
//! 跨村探親（947）也接連讓兩村的老朋友彼此惦記、真的走上幾百格遠路重逢——但這些全是**情感**的
//! 流動：兩村之間至今從沒有一絲一毫的**物資**流通，主村與殖民地是兩座各自為政、老死不相往來
//! 的經濟孤島。v1 把「兩座聚落」第一次接上一條**跑商隊**的路：一位居民偶爾會帶著自己的
//! 特產物資，長途跋涉去另一座聚落，跟當地交換一批這裡沒有的東西，再帶著換來的貨踏上歸途。
//!
//! **真缺口（v2，ROADMAP 997）**：v1 自己的文件開頭就留白「v1 刻意有界，殖民地互跑留給未來
//! 一刀」——世界一旦奠基出第二座殖民地，商隊路線仍死死焊在「主村⇄最早那座殖民地」這一條線，
//! 其餘殖民地永遠只能眼巴巴看著主村商隊過門不入，殖民地與殖民地之間更是連一次都沒互跑過。
//! v2 把「主村」與「殖民地」原本兩條不對稱分支，攤平成同一份聚落名冊——任兩座聚落之間都能
//! 互跑商隊，商隊出發時不再區分「我是主村」或「我是殖民地」，一視同仁地從所有其他聚落裡挑
//! 一座去（[`pick_settlement_destination`]）。世界只有一座殖民地時自然退回 v1 行為（唯一
//! 候選必是它/主村，行為零回歸）。
//! **與既有系統 razor-sharp 區隔（非同軸換皮）**：
//! - **兩村相思／跨村探親（945／947）**＝**情感**驅動、**只在 Friend 級摯友之間**才會發生、
//!   目的是**見到特定一個人**（重逢），不涉及任何物資。本刀＝**經濟**驅動、**不吃好感度門檻**
//!   （任兩座聚落之間皆可跑商隊，無論居民彼此認不認識）、目的是**帶回一批貨**，不必找到特定
//!   個人——這是世界裡第一個「不看交情、只看聚落」的跨村互動。
//! - **居民互相以物易物（723，`voxel_resident_trade`）**＝**同村**內到訪時的隨機易物，觸發、
//!   雙方都要在場；本刀＝**跨聚落**、需要真的走上幾百格遠路才做得成生意，兩者現在共用同一份
//!   `voxel_trade::vocation_trade_pair` 生計對照表（見下方 v3），是同一套經濟分類第一次被
//!   「距離」放大出份量。
//! - **邊陲探友（821）**＝找**暫時遠行**的朋友，目的地是荒野營地；本刀目的地永遠是**聚落**
//!   （主村或殖民地中心），且商隊不找人，只找地方交易。
//!
//! **真缺口（v3，ROADMAP 1007）**：v1 帶出去的貨（[`crate::voxel_resident_trade`] 舊版
//! `specialty_item`，依 id 雜湊）從一開始就與商隊主角的生計身分無關——鐵匠可能揹著一把種子
//! 上路，跟她自己是誰毫無關係。989（玩家↔居民生計交易）與 992（居民↔居民生計互賴）早已
//! 分別把這條線接上生計，商隊卻是**唯一還沒補上**的第三條交易因果線，`voxel_resident_trade`
//! 自己的文件當初也誠實點名「商隊沿用舊版、換掉它屬另一刀的範圍」。v3 把商隊帶出去的貨
//! （`gave_item`，`voxel_ws.rs` 出發分支）改用 [`crate::voxel_trade::vocation_trade_pair`]——
//! 跟 989/992 共用同一份對照表，商人仍走既有雜貨輪替分支（零回歸）。目的地換回來的貨
//! （[`settlement_specialty_item`]）刻意不動：一座聚落住著多位不同生計的居民，硬指定成某一位
//! 的產物反而失真，這份「目的地象徵物」的抽象留給有需要時再議。舊版 `specialty_item` 自此
//! 無人呼叫，已隨此刀移除。
//!
//! **成本 / 濫用防護鐵律**：
//! - **零 LLM**：觸發、路線、交易物品、台詞、記憶全是確定性純函式，低頻節流（每趟至少隔
//!   [`COOLDOWN_SECS`] 秒），不洗版、不佔用任何 API 額度。
//! - **玩家無從觸發或催發**：聚落歸屬、殖民地座標皆伺服器內部狀態，不收任何玩家輸入、不開
//!   對外端點；台詞只嵌居民顯示名、聚落名與伺服器策展的物品名，無注入／NSFW 面。
//! - **純邏輯層**：本檔零鎖、零 IO、零 async；狀態機推進、鎖序、Feed／記憶落地全在 `voxel_ws.rs`
//!   （比照 947 跨村探親的短鎖慣例，守 prod 死鎖鐵律）。
//! - **零 migration、零協議破壞**：商隊是純記憶體暫態（重啟＝這趟商隊自然作罷、居民照常回家域），
//!   不新增任何持久化格式；快照不新增欄位。

/// 記憶哨兵鍵（比照 `crate::voxel_expedition::EXPEDITION_MEMORY_PLAYER`）：商隊面對的是聚落
/// 而非特定個人，記憶掛這個哨兵鍵而非某位居民/玩家名下，日記／內心可據此引用。
pub const CARAVAN_MEMORY_PLAYER: &str = "__voxel_caravan__";

/// 觸發機率（每次低頻掃描，比照邊陲探友 [`crate::voxel_frontier_visit::VISIT_CHANCE`] 同量級，
/// 跑一趟商隊是鄭重的大事、稀有才有份量）。
pub const CARAVAN_CHANCE: f32 = 0.02;

/// 商隊冷卻秒數（30 分鐘，比照跨村探親 [`crate::voxel_farvisit::COOLDOWN_SECS`] 同量級）：
/// 一趟商隊（成交／折返）後至少隔這麼久才可能再出發。
pub const COOLDOWN_SECS: f32 = 1800.0;

/// 抵達目的地聚落中心的判定距離（世界座標）：聚落是一片開闊的小廣場，不必走到分毫不差。
pub const ARRIVE_DIST: f32 = 6.0;

/// 抵達後的交易/逗留秒數：做生意比敘舊快，比 947 重逢小聚（75 秒）短。
pub const STAY_SECS: f32 = 40.0;

/// 去程逾時秒數（比照跨村探親同量級：兩村相距至少數百格，居民步速約 2.6 格/秒）。
pub const TIMEOUT_SECS: f32 = 480.0;

/// 抵達逗留中的閒晃半徑（比照跨村探親 [`crate::voxel_farvisit::WANDER_RADIUS`] 同量級）：
/// 做生意的居民在目的地聚落廣場一小片範圍走動，不散開到整座村。
pub const WANDER_RADIUS: f32 = 8.0;

/// 目的地聚落「換回來的貨」調色盤（種子/石頭/木頭/玻璃，即商人生計的雜貨組合，見
/// `voxel_trade::resident_trade_slot` 商人分支）；v3 起商隊帶**出去**的貨改走
/// [`crate::voxel_trade::vocation_trade_pair`]，不再共用這份調色盤。
const ITEMS: [u8; 4] = [14, 3, 5, 10];

/// 是否該出發跑一趟商隊：閒置自由 + 醒著 + 冷卻已到期 + 沒在說話（不搶話） + 機率骰過。
/// 刻意**不吃好感度／交情門檻**——商隊只看聚落，不看交情，這正是與 945/947/821 的分界。
/// 純函式、可測。
pub fn should_embark(idle_free: bool, asleep: bool, cooldown: f32, say_empty: bool, roll: f32) -> bool {
    idle_free && !asleep && cooldown <= 0.0 && say_empty && roll < CARAVAN_CHANCE
}

/// 從「全世界所有聚落」名冊裡，替一座出發聚落挑商隊目的地（v2，取代 v1 的
/// `pick_colony_destination`）：候選＝`settlements` 中**排除自己**的其餘每一座（主村＋每座
/// 已奠基殖民地一視同仁），依聚落 id 排序後用 `roll`（`[0,1)`）等分索引挑一座——同輸入同
/// 輸出、純函式、可測。世界只有兩座聚落（主村＋一座殖民地）時，候選必只剩對方那一座，
/// `roll` 不影響結果、與 v1 行為一致（零回歸）；三座以上聚落時，`roll` 才真的讓不同趟商隊
/// 散落去不同目的地，殖民地與殖民地之間第一次真的能互跑。
///
/// `settlements` 為 `(settlement_id, name, cx, cz)` 快照；`self_id` 是出發聚落自己的 id。
/// 候選為空（世界還沒有第二座聚落、或名冊只有自己）→ `None`（商隊無處可去）。
pub fn pick_settlement_destination(
    settlements: &[(u64, String, f32, f32)],
    self_id: u64,
    roll: f32,
) -> Option<(f32, f32, String, u64)> {
    let mut candidates: Vec<&(u64, String, f32, f32)> =
        settlements.iter().filter(|(sid, ..)| *sid != self_id).collect();
    if candidates.is_empty() {
        return None;
    }
    candidates.sort_by_key(|(sid, ..)| *sid); // 確定性排序：同輸入同輸出
    let idx = ((roll.clamp(0.0, 0.999_999) * candidates.len() as f32) as usize).min(candidates.len() - 1);
    let (sid, name, cx, cz) = candidates[idx];
    Some((*cx, *cz, name.clone(), *sid))
}

/// 目的地聚落換回來的特產物品（沿用 [`ITEMS`] 調色盤，依聚落名確定性雜湊挑選）；
/// `exclude` 是商隊帶去的那樣物品——保證換回來的必與帶去的不同款（否則「交換」沒有意義）。
/// 純函式、可測。
pub fn settlement_specialty_item(dest_name: &str, exclude: u8) -> u8 {
    let sum: u64 = dest_name.bytes().map(|b| b as u64).sum();
    let idx = (sum % ITEMS.len() as u64) as usize;
    if ITEMS[idx] != exclude {
        return ITEMS[idx];
    }
    ITEMS[(idx + 1) % ITEMS.len()]
}

/// Feed 動態牆：出發（面向玩家字串，留 i18n 空間）。
pub fn depart_feed_line(name: &str, dest: &str, item_name: &str) -> String {
    format!("{name} 帶著一批{item_name}，動身跑一趟商隊前往{dest}。")
}

/// 出發時的頭頂泡泡。
pub fn depart_bubble(dest: &str, pick: usize) -> String {
    const LINES: [&str; 3] = [
        "去{dest}跑一趟商隊，換點東西回來。",
        "帶上這些，去{dest}換點稀罕貨。",
        "商隊要出發了，目標{dest}！",
    ];
    LINES[pick % LINES.len()].replace("{dest}", dest)
}

/// Feed 動態牆：抵達並成交。
pub fn arrive_feed_line(name: &str, dest: &str, gave_name: &str, got_name: &str) -> String {
    format!("{name} 抵達了{dest}，用帶去的{gave_name}換回一批{got_name}。")
}

/// 抵達成交時的頭頂泡泡。
pub fn arrive_bubble(got_name: &str, pick: usize) -> String {
    const LINES: [&str; 3] = [
        "成交！換到了{item}，這趟沒白跑。",
        "這裡的人真爽快，{item}到手了。",
        "生意談成了，帶著{item}回家去。",
    ];
    LINES[pick % LINES.len()].replace("{item}", got_name)
}

/// Feed 動態牆：帶著貨踏上歸途。
pub fn depart_home_feed_line(name: &str, dest: &str, got_name: &str) -> String {
    format!("{name} 帶著從{dest}換來的{got_name}，踏上了回家的路。")
}

/// Feed 動態牆：路太遠太難走，半途折返（誠實失敗，不無限跋涉）。
pub fn giveup_feed_line(name: &str, dest: &str) -> String {
    format!("去{dest}的路上地形太難走，商隊半途折返了。")
}

/// 寫進商隊居民自己記憶的摘要（單方視角——商隊面對的是聚落而非特定個人，
/// 不比照 945/947 寫雙方記憶）。
pub fn memory_line(dest: &str, gave_name: &str, got_name: &str) -> String {
    format!("我帶著{gave_name}跑了一趟{dest}，換回了{got_name}——這趟商隊沒有白跑。")
}

// ── 測試 ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_embark_requires_every_gate() {
        assert!(should_embark(true, false, 0.0, true, 0.0));
        assert!(!should_embark(false, false, 0.0, true, 0.0), "沒閒置自由不出發");
        assert!(!should_embark(true, true, 0.0, true, 0.0), "睡著不出發");
        assert!(!should_embark(true, false, 1.0, true, 0.0), "冷卻中不出發");
        assert!(!should_embark(true, false, 0.0, false, 0.0), "正在說話不搶話");
        assert!(!should_embark(true, false, 0.0, true, CARAVAN_CHANCE), "骰不中不出發");
    }

    #[test]
    fn should_embark_chance_boundary() {
        assert!(should_embark(true, false, 0.0, true, CARAVAN_CHANCE - 0.001));
        assert!(!should_embark(true, false, 0.0, true, CARAVAN_CHANCE));
    }

    #[test]
    fn pick_settlement_destination_empty_is_none() {
        assert_eq!(pick_settlement_destination(&[], 0, 0.0), None);
    }

    #[test]
    fn pick_settlement_destination_only_self_is_none() {
        let settlements = vec![(0u64, "主村".to_string(), 0.0, 0.0)];
        assert_eq!(pick_settlement_destination(&settlements, 0, 0.5), None, "名冊只有自己，無處可去");
    }

    #[test]
    fn pick_settlement_destination_single_other_ignores_roll() {
        // 世界只有主村＋一座殖民地：候選必只剩對方那一座，roll 不影響結果（v1 行為零回歸）。
        let settlements = vec![
            (0u64, "主村".to_string(), 0.0, 0.0),
            (7u64, "霜語屯".to_string(), 500.0, -300.0),
        ];
        for roll in [0.0, 0.3, 0.7, 0.999] {
            assert_eq!(
                pick_settlement_destination(&settlements, 0, roll),
                Some((500.0, -300.0, "霜語屯".to_string(), 7)),
                "roll={roll}"
            );
        }
        // 反向：從殖民地出發，候選必只剩主村。
        assert_eq!(
            pick_settlement_destination(&settlements, 7, 0.5),
            Some((0.0, 0.0, "主村".to_string(), 0))
        );
    }

    #[test]
    fn pick_settlement_destination_excludes_self_among_many() {
        let settlements = vec![
            (0u64, "主村".to_string(), 0.0, 0.0),
            (3u64, "霜語屯".to_string(), -50.0, 60.0),
            (9u64, "風禾屯".to_string(), 100.0, 200.0),
        ];
        // 從霜語屯（id 3）出發：候選只剩主村(0)/風禾屯(9)，永遠不會選到自己。
        for roll in [0.0, 0.25, 0.5, 0.75, 0.99] {
            let dest = pick_settlement_destination(&settlements, 3, roll).unwrap();
            assert_ne!(dest.3, 3, "roll={roll} 不該選到自己");
        }
    }

    #[test]
    fn pick_settlement_destination_roll_spreads_across_candidates() {
        // 殖民地互跑（ROADMAP 997）：三座以上聚落時，不同 roll 真的能落在不同目的地，
        // 而非永遠焊死同一座——這正是 v2 相較 v1「固定挑最早奠基那座」補上的缺口。
        let settlements = vec![
            (0u64, "主村".to_string(), 0.0, 0.0),
            (3u64, "霜語屯".to_string(), -50.0, 60.0),
            (9u64, "風禾屯".to_string(), 100.0, 200.0),
        ];
        let low = pick_settlement_destination(&settlements, 0, 0.0).unwrap();
        let high = pick_settlement_destination(&settlements, 0, 0.99).unwrap();
        assert_ne!(low.3, high.3, "低 roll 與高 roll 應落在不同候選");
    }

    #[test]
    fn pick_settlement_destination_deterministic() {
        let settlements = vec![
            (0u64, "主村".to_string(), 0.0, 0.0),
            (3u64, "霜語屯".to_string(), -50.0, 60.0),
        ];
        let a = pick_settlement_destination(&settlements, 0, 0.42);
        let b = pick_settlement_destination(&settlements, 0, 0.42);
        assert_eq!(a, b, "確定性：同輸入同輸出");
    }

    #[test]
    fn settlement_specialty_item_in_known_set() {
        for name in ["主村", "霜語屯", "風禾屯", "任意聚落"] {
            let item = settlement_specialty_item(name, 255);
            assert!(ITEMS.contains(&item), "settlement_specialty_item({name})={item} 應落在已知集合");
        }
    }

    #[test]
    fn settlement_specialty_item_never_equals_exclude() {
        for name in ["主村", "霜語屯", "風禾屯", "壬子路"] {
            for &ex in &ITEMS {
                assert_ne!(
                    settlement_specialty_item(name, ex),
                    ex,
                    "換回來的物品必與帶去的不同款"
                );
            }
        }
    }

    #[test]
    fn settlement_specialty_item_deterministic() {
        assert_eq!(
            settlement_specialty_item("霜語屯", 14),
            settlement_specialty_item("霜語屯", 14)
        );
    }

    #[test]
    fn depart_feed_line_contains_name_dest_item() {
        let line = depart_feed_line("露娜", "霜語屯", "種子");
        assert!(line.contains("露娜"));
        assert!(line.contains("霜語屯"));
        assert!(line.contains("種子"));
    }

    #[test]
    fn depart_bubble_replaces_placeholder_safely() {
        for pick in 0..6 {
            let line = depart_bubble("霜語屯", pick);
            assert!(line.contains("霜語屯"));
            assert!(!line.contains("{dest}"));
        }
    }

    #[test]
    fn arrive_feed_line_contains_all_parts() {
        let line = arrive_feed_line("露娜", "霜語屯", "種子", "石頭");
        assert!(line.contains("露娜"));
        assert!(line.contains("霜語屯"));
        assert!(line.contains("種子"));
        assert!(line.contains("石頭"));
    }

    #[test]
    fn arrive_bubble_replaces_placeholder_safely() {
        for pick in 0..6 {
            let line = arrive_bubble("石頭", pick);
            assert!(line.contains("石頭"));
            assert!(!line.contains("{item}"));
        }
    }

    #[test]
    fn bubble_pick_wraps_safely() {
        let line = depart_bubble("風禾屯", 9999);
        assert!(line.contains("風禾屯"));
        let line2 = arrive_bubble("玻璃", 9999);
        assert!(line2.contains("玻璃"));
    }

    #[test]
    fn depart_home_and_giveup_feed_lines_contain_name_and_dest() {
        let home = depart_home_feed_line("諾娃", "主村", "木頭");
        assert!(home.contains("諾娃"));
        assert!(home.contains("主村"));
        assert!(home.contains("木頭"));

        let giveup = giveup_feed_line("諾娃", "主村");
        assert!(giveup.contains("主村"));
    }

    #[test]
    fn memory_line_mentions_dest_and_items() {
        let line = memory_line("霜語屯", "種子", "石頭");
        assert!(line.contains("霜語屯"));
        assert!(line.contains("種子"));
        assert!(line.contains("石頭"));
    }
}
