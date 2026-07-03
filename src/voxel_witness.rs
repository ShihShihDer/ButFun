//! 乙太方界·居民為鄰居圓夢而賀喜 v1（voxel-witness）。
//!
//! **北極星**：居民的渴望（`voxel_desires`）此前只在「她自己」與「玩家」兩端之間流動——
//! 玩家送對禮物圓了她的心願（722），她記一筆、動態牆廣播「心願送到了」，但**旁邊那位跟她
//! 朝夕相處的鄰居，卻從不曾為她的圓夢說一句話**。整個小村對彼此的成就是沉默的：居民會在對方
//! 心情低落時去陪伴（`voxel_cheer`），卻不會在對方**夢想成真時一起高興**。這一刀補上那一拍：
//! 當你送對禮物、圓了某位居民的心願，若身邊剛好有另一位醒著的鄰居，**她會看見這一幕、由衷替
//! 對方道賀一句**；圓夢者也回一句暖暖的謝，這份「一起見證圓夢」的共同喜悅讓兩人情誼升溫
//! （`voxel_bonds` 記一次往來），雙方也各把這一刻記進心裡。世界第一次有了「居民為彼此的成就
//! 道賀」——小社會的溫度，不再只靠玩家介入才亮起。
//!
//! **與既有社交的定位區隔**：
//! - 相互打氣（679，`voxel_cheer`）是在對方**心情低落**時去陪伴（雪中送炭）；本刀是在對方
//!   **夢想成真**時一起高興（錦上添花）——一個接住低谷、一個共享高光，方向相反。
//! - 情誼到訪（672，`voxel_bonds`）靠**反覆串門子**慢慢累積；本刀是靠**一次共同見證的喜悅**
//!   當場加溫一格，情境全然不同。
//!
//! **純邏輯層**：是否在見證範圍內（[`in_witness_range`]）、從候選鄰居裡挑最近的一位
//! （[`nearest_witness_index`]）、道賀／回謝台詞（[`witness_say_line`]／[`witness_reply_line`]）、
//! 雙方記憶摘要與動態牆句全是確定性純函式，零 LLM、零鎖、零 IO。鎖 / 快照 / 記憶寫入全在
//! `voxel_ws.rs`，沿用送禮圓夢那條已驗證的短鎖循序。
//!
//! **成本 / 濫用防護**：只在**送對禮物圓夢**這個本就稀有、且 [`mark_fulfilled`] 保證一次性
//! （冪等）的事件上觸發——無每 tick 迴圈、無新對外端點、不觸發 LLM，天然防洗版與白嫖。
//! 道賀／回謝／記憶／動態牆句全走固定模板，**只嵌居民自己的顯示名**（本就是系統內建、非玩家
//! 自由輸入），**永不夾帶玩家原話或渴望原文**（無注入 / NSFW 面）。零 migration（借既有記憶／
//! 情誼／動態牆管線）、零新協議欄位、零前端改動、零新美術、FPS 零影響（純後端偶發事件）。
//!
//! [`mark_fulfilled`]: crate::voxel_desires::DesireStore::mark_fulfilled

/// 鄰居要能「看見」這場圓夢、進而道賀的最大水平距離（方塊，XZ 平面）。
/// 設 16——比讚賞（`voxel_admire`，6）遠、比打氣（`voxel_cheer`，15）略寬：圓夢是
/// 值得整條巷子探頭的大事，稍遠一點的鄰居也該有機會共襄盛舉；但仍夠近，確保玩家
/// 看得到道賀的那位鄰居就在同一畫面裡。
pub const WITNESS_RANGE: f32 = 16.0;

/// 道賀／回謝泡泡的字元上限（與泡泡框上限一致，超出截斷不破框）。
pub const WITNESS_SAY_MAX_CHARS: usize = 40;

/// 動態牆分類（圓夢賀喜）。
pub const FEED_KIND: &str = "圓夢賀喜";

/// 這對水平位移是否落在見證範圍內（平方距離比較，省一次開根號）。
///
/// 座標壞掉（NaN / 無限）時保守回 `false`——寧可不觸發，也不憑髒資料亂挑鄰居。
pub fn in_witness_range(dx: f32, dz: f32) -> bool {
    if !dx.is_finite() || !dz.is_finite() {
        return false;
    }
    dx * dx + dz * dz <= WITNESS_RANGE * WITNESS_RANGE
}

/// 從候選鄰居的水平位移清單裡，挑「在見證範圍內、且離圓夢者最近」的那一位，回其索引。
///
/// `offsets[i]` = 第 i 位候選鄰居相對圓夢者的 `(dx, dz)`（呼叫端已濾掉圓夢者本人與睡著的）。
/// 全部都在範圍外、或清單為空 → `None`（誠實地「這次沒人在場見證」，不硬湊）。
/// 純函式、確定性：平手（距離完全相同）時取索引較小者，穩定可測。
pub fn nearest_witness_index(offsets: &[(f32, f32)]) -> Option<usize> {
    let mut best: Option<(usize, f32)> = None;
    for (i, &(dx, dz)) in offsets.iter().enumerate() {
        if !in_witness_range(dx, dz) {
            continue;
        }
        let d2 = dx * dx + dz * dz;
        match best {
            // 嚴格小於才更新 → 平手保留較小索引（確定性）。
            Some((_, bd2)) if d2 >= bd2 => {}
            _ => best = Some((i, d2)),
        }
    }
    best.map(|(i, _)| i)
}

/// 鄰居（見證者）為圓夢者道賀的一句話（泡泡）。
///
/// 依 `pick` 在幾組固定語氣模板間確定性輪替；只嵌圓夢者顯示名（系統內建、非玩家輸入）。
/// 以字元為單位截到 [`WITNESS_SAY_MAX_CHARS`] 內，永不破泡泡框、永不回空。
pub fn witness_say_line(achiever: &str, pick: usize) -> String {
    let a = achiever.trim();
    // 圓夢者名字理論上恆非空；真空掉時落回不倚賴名字的通用賀詞，仍是「為你高興」的味道。
    if a.is_empty() {
        const FALLBACK: [&str; 3] = [
            "你的心願成真啦，我真替你高興！",
            "夢想成真的這一刻，我也跟著開心呢！",
            "太好了，這下你惦記好久的事終於圓滿了！",
        ];
        return FALLBACK[pick % FALLBACK.len()]
            .chars()
            .take(WITNESS_SAY_MAX_CHARS)
            .collect();
    }
    const TEMPLATES: [&str; 5] = [
        "{}，你的心願成真啦，我真替你高興！",
        "{}，看你夢想成真，我也跟著樂開懷了！",
        "太好了{}！你惦記好久的事，終於圓滿啦！",
        "{}，這一刻我也在呢——恭喜你圓夢！",
        "{}，你等這一天等好久了吧，恭喜恭喜！",
    ];
    let line = TEMPLATES[pick % TEMPLATES.len()].replace("{}", a);
    line.chars().take(WITNESS_SAY_MAX_CHARS).collect()
}

/// 圓夢者回謝道賀鄰居的一句話（泡泡）。
///
/// 依 `pick` 確定性輪替；只嵌鄰居顯示名。截到框內、永不回空。
pub fn witness_reply_line(witness: &str, pick: usize) -> String {
    let w = witness.trim();
    if w.is_empty() {
        const FALLBACK: [&str; 3] = [
            "謝謝你來替我高興，這份喜悅更暖了！",
            "有人一起分享，這份開心好像加倍了呢～",
            "謝謝你的道賀，我會一直記得這一刻的。",
        ];
        return FALLBACK[pick % FALLBACK.len()]
            .chars()
            .take(WITNESS_SAY_MAX_CHARS)
            .collect();
    }
    const TEMPLATES: [&str; 4] = [
        "謝謝你，{}！有你來道賀，這份喜悅更暖了。",
        "{}，謝謝你替我高興～這份開心好像加倍了！",
        "有你在，{}，這一刻更圓滿了，謝謝你。",
        "謝謝你的祝福，{}，我會一直記得的。",
    ];
    let line = TEMPLATES[pick % TEMPLATES.len()].replace("{}", w);
    line.chars().take(WITNESS_SAY_MAX_CHARS).collect()
}

/// 見證者（鄰居）記進心裡的一筆記憶摘要（第一人稱、episodic，掛在圓夢者名下）。
///
/// 刻意停在情節層——記「我為某人的圓夢道賀、替她高興」這件事本身，累積兩人交情（記憶筆數），
/// **不夾帶渴望原文**（與 `voxel_confide` 同款輕記憶，也杜絕渴望文字被日後八卦回放）。
pub fn witness_memory_for_witness(achiever: &str) -> String {
    format!("我親眼看著{achiever}的心願成真，打從心底替她高興，還上前道賀了一句。")
}

/// 圓夢者記進心裡的一筆記憶摘要（第一人稱，掛在道賀鄰居名下）。
///
/// 記「圓夢的那一刻，某位鄰居特地來道賀」——這份被在乎的暖意，讓她記住這位鄰居。
pub fn witness_memory_for_achiever(witness: &str) -> String {
    format!("我圓夢的那一刻，{witness}特地過來替我高興、道賀了一句，這份心意我記下了。")
}

/// 動態牆一行（`vfeed::append_feed(FEED_KIND, witness, detail)` 的 detail 部分）。
///
/// 讓玩家在動態牆看見「AI 居民為彼此的成就道賀」——小社會的溫度被看見。只嵌兩位居民顯示名。
pub fn witness_feed_line(witness: &str, achiever: &str) -> String {
    format!("{witness}看見{achiever}圓了心願，特地上前替她道賀、一同歡喜")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_range_basic() {
        assert!(in_witness_range(0.0, 0.0));
        assert!(in_witness_range(WITNESS_RANGE, 0.0));
        assert!(in_witness_range(0.0, WITNESS_RANGE));
        // 剛好超出。
        assert!(!in_witness_range(WITNESS_RANGE + 0.01, 0.0));
        // 對角線超出（16,16 的距離遠大於 16）。
        assert!(!in_witness_range(WITNESS_RANGE, WITNESS_RANGE));
    }

    #[test]
    fn in_range_rejects_bad_coords() {
        assert!(!in_witness_range(f32::NAN, 0.0));
        assert!(!in_witness_range(0.0, f32::INFINITY));
        assert!(!in_witness_range(f32::NEG_INFINITY, f32::NAN));
    }

    #[test]
    fn nearest_picks_closest_in_range() {
        // 索引 1 最近（距離 3），索引 0（距離 5）、索引 2（距離 10）。
        let offs = [(3.0, 4.0), (3.0, 0.0), (6.0, 8.0)];
        assert_eq!(nearest_witness_index(&offs), Some(1));
    }

    #[test]
    fn nearest_skips_out_of_range() {
        // 索引 0 超出範圍（20>16）、索引 1 在範圍內 → 選 1。
        let offs = [(20.0, 0.0), (10.0, 0.0)];
        assert_eq!(nearest_witness_index(&offs), Some(1));
    }

    #[test]
    fn nearest_none_when_all_out_or_empty() {
        assert_eq!(nearest_witness_index(&[]), None);
        assert_eq!(nearest_witness_index(&[(20.0, 0.0), (17.0, 0.0)]), None);
    }

    #[test]
    fn nearest_ties_prefer_lower_index() {
        // 兩位距離完全相同 → 取較小索引，確定性。
        let offs = [(3.0, 0.0), (3.0, 0.0), (0.0, 3.0)];
        assert_eq!(nearest_witness_index(&offs), Some(0));
    }

    #[test]
    fn nearest_ignores_bad_coord_candidate() {
        // 髒座標候選被 in_witness_range 擋掉，仍能挑到乾淨的那位。
        let offs = [(f32::NAN, 0.0), (5.0, 0.0)];
        assert_eq!(nearest_witness_index(&offs), Some(1));
    }

    #[test]
    fn say_line_wraps_name_and_fits_frame() {
        for pick in 0..12 {
            let line = witness_say_line("露娜", pick);
            assert!(!line.is_empty(), "道賀句不該為空");
            assert!(line.contains("露娜"), "道賀應嵌圓夢者名：{line}");
            assert!(
                line.chars().count() <= WITNESS_SAY_MAX_CHARS,
                "道賀句不該破泡泡框：{line}"
            );
        }
    }

    #[test]
    fn say_line_truncates_overlong_name() {
        // 名字本身就頂到上限，加上模板必然超框 → 必須截到框內、且非空。
        let long_name: String = "諾".repeat(WITNESS_SAY_MAX_CHARS + 5);
        let line = witness_say_line(&long_name, 0);
        assert!(line.chars().count() <= WITNESS_SAY_MAX_CHARS);
        assert!(!line.is_empty());
    }

    #[test]
    fn say_line_empty_name_falls_back() {
        for pick in 0..6 {
            let line = witness_say_line("   ", pick);
            assert!(!line.is_empty(), "空名應落回通用賀詞、非空");
            assert!(line.chars().count() <= WITNESS_SAY_MAX_CHARS);
        }
    }

    #[test]
    fn reply_line_wraps_name_and_fits_frame() {
        for pick in 0..10 {
            let line = witness_reply_line("諾娃", pick);
            assert!(!line.is_empty());
            assert!(line.contains("諾娃"), "回謝應嵌鄰居名：{line}");
            assert!(line.chars().count() <= WITNESS_SAY_MAX_CHARS);
        }
    }

    #[test]
    fn reply_line_empty_name_falls_back() {
        let line = witness_reply_line("", 1);
        assert!(!line.is_empty());
        assert!(line.chars().count() <= WITNESS_SAY_MAX_CHARS);
    }

    #[test]
    fn say_and_reply_deterministic_by_pick() {
        assert_eq!(witness_say_line("賽勒", 2), witness_say_line("賽勒", 2));
        assert_eq!(witness_reply_line("奧瑞", 3), witness_reply_line("奧瑞", 3));
    }

    #[test]
    fn memory_lines_contain_names_and_no_desire_text() {
        let mw = witness_memory_for_witness("露娜");
        assert!(mw.contains("露娜"), "見證者記憶應含圓夢者名");
        assert!(!mw.is_empty());
        let ma = witness_memory_for_achiever("諾娃");
        assert!(ma.contains("諾娃"), "圓夢者記憶應含道賀者名");
        assert!(!ma.is_empty());
        // 記憶刻意不含渴望原文（停在情節層、不夾帶內容，杜絕八卦回放）。
        assert!(!mw.contains("玻璃") && !ma.contains("玻璃"));
    }

    #[test]
    fn feed_line_contains_both_names() {
        let f = witness_feed_line("露娜", "諾娃");
        assert!(f.contains("露娜") && f.contains("諾娃"), "動態牆應含兩位居民名：{f}");
        assert!(!f.is_empty());
    }
}
