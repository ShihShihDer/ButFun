//! 乙太方界·居民夜間生活 v1（voxel-nightlife，自主提案切片）——**入夜後的溫柔日常**。
//!
//! **缺口 / 為誰做**：這片世界的夜晚，居民能做的事至今都很「功能性」——回家躺下睡覺
//! （739 睡覺／744 睡前反思），或見到暗影點燈守望（`voxel_nightwatch`，那是防禦）。夜晚
//! 本身的**獨特生活氛圍**還沒真的長出來：白天採集建造，一入夜就只剩「等天亮」。玩家夜裡
//! 路過村子，看不到「夜有它自己的節奏」這回事。本切片補上兩樣**只在入夜後才發生、療癒的
//! 居民日常**，讓夜晚和白日的忙碌區隔開來：
//!   - **抬頭看夜空許個願**（[`wish_line`]）：一位閒著、還沒就寢的居民，獨自停下腳步，抬頭
//!     望向夜空，輕輕許下一個小小的願望。**不需要玩家在場**——這是居民自己的內心一拍，是
//!     這片夜色即使沒人看著也在悄悄發生的溫柔。
//!   - **睡前互道晚安**（[`goodnight_line`]／[`goodnight_reply_line`]）：入夜後，兩位還醒著、
//!     恰好走得夠近的居民，在各自回家歇息前，彼此道一聲晚安。夜的尾聲，第一次有了「大家互相
//!     道別、各自安歇」這份人情味。
//!
//! **與既有夜間元素的分界（非同軸重複換皮）**：
//!   - **不是望星邀約**（783 `voxel_stargaze`）：那是「居民**記得你愛看星星**→**點名喚玩家**
//!     一起賞星」，由玩家在場＋偏好記憶驅動；本檔的許願是**居民自己**對夜空許願，**無玩家亦
//!     發生**、不點名任何人，是純粹的居民內心氛圍。
//!   - **不是圍火講古**（791/792 `voxel_campfire_tale`）：那要兩人同在**一座營火邊**、講述一段
//!     **過去的記憶**；本檔的互道晚安不綁營火、不翻記憶，只是**回家前的一聲道別**。
//!   - **不是就寢反思**（744 `voxel_bedtime`）：那是**躺下那一刻、獨自向內**回味今天；本檔是
//!     **還沒躺下、向外**對夜空／對夥伴的一拍。**就寢優先**——真的想睡的居民照樣去睡，本檔
//!     只挑「還沒就寢」的居民，絕不攔著誰不讓睡。
//!   - **不是點燈守望**（`voxel_nightwatch`）：那是對暗影的**集體防禦**；本檔純粹是**溫柔日常**，
//!     沒有威脅、不放方塊。
//!
//! **選址（近光源／營火）**：夜裡，居民若剛好停在一盞火把／一座營火這類**光源**近旁，許願／道
//! 晚安的台詞會扣著「就著這點暖光」的意象（[`is_near_light`]）——把夜生活自然聚到玩家點亮的
//! 那些暖光邊，讓夜景更有層次；離光源遠則是曠野星空下的版本。選址只是**氛圍分支**，不改觸發。
//!
//! **成本 / 濫用防護鐵律**：
//!   - **純邏輯層**：本檔只放確定性純函式（時段／觸發判定、選址、台詞、Feed 選句），零 LLM、
//!     零鎖、零 IO、零 async、可窮舉單元測試。鎖與副作用（say／冷卻／pending 回應／Feed）全在
//!     `voxel_ws.rs`，沿用 stargaze／圍火講古那條已驗證的短鎖循序＋鎖外事件佇列慣例，守 prod
//!     死鎖鐵律。
//!   - 台詞全為固定模板、確定性選句，**永不回放玩家原話**；許願不嵌任何玩家名（純氛圍）、
//!     互道晚安只嵌居民**顯示名**（既有安全字串），無注入／NSFW 面。
//!   - 每居民長冷卻（[`GOODNIGHT_COOLDOWN_SECS`]／[`WISH_COOLDOWN_SECS`]，兩者共用同一個
//!     `nightlife_cooldown` 欄位）＋每 tick 極低機率＋僅入夜時段觸發＝天然節流，不洗版泡泡／Feed。
//!   - 零持久化、零 migration（冷卻純記憶體、重啟歸零，比照 stargaze／哼歌慣例）；
//!     零新協議欄位、零前端改動、零新美術、FPS 零影響（純後端、低頻併入既有居民 tick）。

use crate::voxel_time::TimePhase;

// ── 面向玩家字串／參數（集中一處，i18n 友善、日後平衡好調）────────────────────────

/// 動態牆播報種類名稱（睡前互道晚安）。
pub const GOODNIGHT_FEED_KIND: &str = "互道晚安";

/// 泡泡／Feed 字元上限（與既有社交泡泡同框，超長截斷不破框）。
pub const SAY_CHARS: usize = 40;

/// 兩位居民要靠得多近（方塊，XZ 平面）才會在回家前互道晚安。比社交招呼半徑略窄——
/// 道晚安是「就在你身邊」的一聲道別，不是隔街喊話。
pub const GOODNIGHT_RADIUS: f32 = 7.0;

/// 每居民「互道晚安／許願」共用冷卻（秒）：一次夜生活後隔這麼久才會再有下一拍——
/// 偶爾一拍才有感、不洗版。各居民初始錯開。純記憶體、重啟歸零。
pub const GOODNIGHT_COOLDOWN_SECS: f32 = 220.0;

/// 每居民許願冷卻（秒）——與互道晚安共用同一個 `nightlife_cooldown` 欄位，此常數供選句／測試
/// 對照；獨自許願比互道晚安更常見一點（不需湊到夥伴），但仍設長冷卻防洗版。
pub const WISH_COOLDOWN_SECS: f32 = 200.0;

/// 每次「符合條件的 tick」真的互道晚安的機率（其餘時候只是靜靜錯身）——配合長冷卻＝天然節流。
pub const GOODNIGHT_CHANCE: f32 = 0.5;

/// 每次「符合條件的 tick」真的抬頭許願的機率（極低）：許願是偶爾滿溢的一拍、不是每站定就許。
pub const WISH_CHANCE: f32 = 0.03;

/// 被夥伴道了晚安後，延遲幾秒才回一聲晚安（沿用社交回應的自然節奏，別讓兩人同一 tick 齊聲）。
pub const GOODNIGHT_REPLY_DELAY_SECS: f32 = 2.5;

/// 「算在光源近旁」的半徑（方塊）：居民離某盞火把／營火這麼近，夜生活台詞就扣著暖光意象。
pub const LIGHT_NEAR_RADIUS: f32 = 6.0;

// ── 時段 / 觸發判定（純函式、可測）─────────────────────────────────────────────

/// 夜間生活時段：沿用睡覺時段（入夜過渡 Evening＋深夜 Night，`voxel_time::is_sleepable` 單一
/// 事實來源）。白天／黎明／黃昏不觸發夜生活。
pub fn is_nightlife_time(phase: TimePhase) -> bool {
    crate::voxel_time::is_sleepable(phase)
}

/// 是否此刻獨自抬頭許願：冷卻已過 ＋ 過機率門檻。say 是否為空、是否醒著、是否還沒就寢由
/// 呼叫端在外層先確認（沿用 stargaze／哼歌慣例）。純函式、可測。
pub fn should_wish(cooldown_ready: bool, roll: f32, chance: f32) -> bool {
    cooldown_ready && roll < chance
}

/// 兩閘判定：互道晚安冷卻到期（`cooldown_ready`）＋過機率門檻（`roll < chance`）→ 這一 tick
/// 互道晚安。「兩人夠近、都醒著、都閒著」由呼叫端配對時判定（見 [`within_range`]），不進本函式
/// （保持純粹好窮舉測）。純函式、可測。
pub fn should_bid_goodnight(cooldown_ready: bool, roll: f32, chance: f32) -> bool {
    cooldown_ready && roll < chance
}

/// 兩位居民在 XZ 平面是否靠得夠近（用距離平方比較，避免開根號）。純函式、可測。
pub fn within_range(x1: f32, z1: f32, x2: f32, z2: f32, radius: f32) -> bool {
    let dx = x1 - x2;
    let dz = z1 - z2;
    dx * dx + dz * dz <= radius * radius
}

// ── 選址：近光源／營火（純函式、可測）─────────────────────────────────────────

/// 居民腳下（rx, rz）[`LIGHT_NEAR_RADIUS`] 內是否有任一光源（火把／營火柱座標）。
/// 只比水平距離（夜裡抬頭／道別不在意高度差）。純函式、可測——供台詞選「暖光版」或「星空版」。
pub fn is_near_light(rx: f32, rz: f32, lights: &[(i32, i32, i32)], radius: f32) -> bool {
    let r2 = radius * radius;
    lights.iter().any(|&(lx, _ly, lz)| {
        // 光源柱中心（+0.5）對齊居民浮點座標，量水平距離。
        let dx = (lx as f32 + 0.5) - rx;
        let dz = (lz as f32 + 0.5) - rz;
        dx * dx + dz * dz <= r2
    })
}

// ── 台詞（確定性選句、截字防溢框；集中此處 i18n 友善）──────────────────────────

/// 泡泡／Feed 統一截字（≤ [`SAY_CHARS`]，防溢框）。
fn clip(line: String) -> String {
    line.chars().take(SAY_CHARS).collect()
}

/// 獨自抬頭許願的一句（純氛圍，不嵌任何玩家名）。`near_light` 為真時扣暖光意象，否則是星空版。
/// 依 `pick` 確定性選句（循環取模，永遠有值）。
pub fn wish_line(near_light: bool, pick: usize) -> String {
    let pool: [&str; 4] = if near_light {
        [
            "就著這點暖光，我悄悄許個願……",
            "守著這盞燈，願明天也是好天氣。",
            "火光暖暖的，我在心裡許了個小小的願。",
            "有這點光陪著，許願都覺得會成真呢。",
        ]
    } else {
        [
            "抬頭一看，星星好多……我許個願吧。",
            "夜空好靜，願大家都平平安安的。",
            "對著今晚的星星，我悄悄許了個願。",
            "這麼美的夜空，值得許一個溫柔的願望。",
        ]
    };
    clip(pool[pick % pool.len()].to_string())
}

/// 睡前向身邊夥伴道晚安的一句（發起者）。`near_light` 為真時扣暖光意象。只嵌對方顯示名，
/// 依 `pick` 確定性選句、截 [`SAY_CHARS`] 防溢框。`other` 由呼叫端提供（居民顯示名，非空）。
pub fn goodnight_line(other: &str, near_light: bool, pick: usize) -> String {
    let pool: [&str; 4] = if near_light {
        [
            "{o}，夜深了，就著這點燈火，早點歇著吧，晚安。",
            "{o}，今天辛苦了，趁著暖光還在，回去好好睡。",
            "{o}，燈也快熄了，我們各自回家吧，晚安。",
            "{o}，這點火光陪你走一段，晚安囉。",
        ]
    } else {
        [
            "{o}，夜深了，早點回去歇著吧，晚安。",
            "{o}，今天辛苦啦，好好睡一覺，明天見。",
            "{o}，星星都出來了，我們各自回家吧，晚安。",
            "{o}，願你有個好夢，晚安囉。",
        ]
    };
    clip(pool[pick % pool.len()].replace("{o}", other))
}

/// 被道了晚安後回的一聲晚安（回應者，通用、四句輪替，不嵌名亦可讀）。依 `pick` 確定性選句、
/// 截 [`SAY_CHARS`]。
pub fn goodnight_reply_line(pick: usize) -> &'static str {
    const LINES: [&str; 4] = [
        "嗯，你也早點睡，晚安～",
        "晚安，明天見囉！",
        "好，你也是，做個好夢。",
        "謝謝你，晚安，路上小心。",
    ];
    LINES[pick % LINES.len()]
}

/// 睡前互道晚安的動態牆一句（第三人稱、含雙方名，讓離線回訪的玩家也讀得到這則溫柔的夜日常）。
pub fn goodnight_feed_line(a: &str, b: &str) -> String {
    clip(format!("{a}和{b}在回家前互道了一聲晚安，各自安歇。"))
}

// ── 單元測試 ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nightlife_time_matches_sleepable() {
        // 夜生活時段必須與「睡得著」的時段一致（單一事實來源 is_sleepable）。
        assert!(is_nightlife_time(TimePhase::Night), "深夜有夜生活");
        assert!(is_nightlife_time(TimePhase::Evening), "入夜過渡就開始有夜生活");
        assert!(!is_nightlife_time(TimePhase::Dawn), "黎明不算夜生活");
        assert!(!is_nightlife_time(TimePhase::Day), "白天不算夜生活");
        assert!(!is_nightlife_time(TimePhase::Dusk), "黃昏還沒入夜");
    }

    #[test]
    fn should_wish_needs_cooldown_and_chance() {
        assert!(should_wish(true, 0.0, WISH_CHANCE));
        // 冷卻未過 → 否。
        assert!(!should_wish(false, 0.0, WISH_CHANCE));
        // roll 達門檻（含）不觸發（嚴格小於）。
        assert!(!should_wish(true, WISH_CHANCE, WISH_CHANCE));
        assert!(!should_wish(true, 0.99, WISH_CHANCE));
        assert!(should_wish(true, WISH_CHANCE - 0.001, WISH_CHANCE));
    }

    #[test]
    fn should_bid_goodnight_needs_cooldown_and_chance() {
        assert!(should_bid_goodnight(true, 0.1, GOODNIGHT_CHANCE));
        // 冷卻未過 → 否。
        assert!(!should_bid_goodnight(false, 0.1, GOODNIGHT_CHANCE));
        // 邊界 roll == chance 不觸發（嚴格小於）。
        assert!(!should_bid_goodnight(true, GOODNIGHT_CHANCE, GOODNIGHT_CHANCE));
        assert!(!should_bid_goodnight(true, 0.99, GOODNIGHT_CHANCE));
    }

    #[test]
    fn within_range_uses_radius() {
        // 剛好在半徑上算近（<=）。
        assert!(within_range(0.0, 0.0, GOODNIGHT_RADIUS, 0.0, GOODNIGHT_RADIUS));
        // 超出半徑一點點算遠。
        assert!(!within_range(0.0, 0.0, GOODNIGHT_RADIUS + 0.1, 0.0, GOODNIGHT_RADIUS));
        // 對角距離也正確判定。
        assert!(within_range(1.0, 1.0, 3.0, 3.0, 3.0));
        assert!(!within_range(0.0, 0.0, 5.0, 5.0, 3.0));
    }

    #[test]
    fn is_near_light_detects_within_radius() {
        let lights = vec![(10, 5, 10), (100, 5, 100)];
        // 站在第一盞燈柱中心附近 → 近光源。
        assert!(is_near_light(10.5, 10.5, &lights, LIGHT_NEAR_RADIUS));
        // 離所有燈都遠 → 不近光源（曠野星空版）。
        assert!(!is_near_light(50.0, 50.0, &lights, LIGHT_NEAR_RADIUS));
        // 沒有任何光源 → 一定不近。
        assert!(!is_near_light(10.5, 10.5, &[], LIGHT_NEAR_RADIUS));
    }

    #[test]
    fn is_near_light_respects_boundary() {
        let lights = vec![(0, 5, 0)]; // 柱中心 (0.5, 0.5)
        // 恰在半徑邊界（水平距離 == 半徑）算近。
        assert!(is_near_light(0.5 + LIGHT_NEAR_RADIUS, 0.5, &lights, LIGHT_NEAR_RADIUS));
        // 略超出算遠。
        assert!(!is_near_light(0.5 + LIGHT_NEAR_RADIUS + 0.2, 0.5, &lights, LIGHT_NEAR_RADIUS));
    }

    #[test]
    fn wish_line_varies_by_light_and_fits_frame() {
        for pick in 0..8 {
            let dark = wish_line(false, pick);
            let lit = wish_line(true, pick);
            assert!(!dark.is_empty() && !lit.is_empty());
            assert!(dark.chars().count() <= SAY_CHARS);
            assert!(lit.chars().count() <= SAY_CHARS);
            // 許願不得嵌佔位符、也不嵌任何玩家名（純氛圍）。
            assert!(!dark.contains('{') && !lit.contains('{'));
        }
        // pick 輪替換句；暖光版與星空版不同。
        assert_ne!(wish_line(false, 0), wish_line(false, 1));
        assert_ne!(wish_line(true, 0), wish_line(false, 0));
        // 取模循環：pick 與 pick+len 同句（確定性）。
        assert_eq!(wish_line(false, 0), wish_line(false, 4));
    }

    #[test]
    fn goodnight_line_names_other_and_fits_frame() {
        for pick in 0..8 {
            let s = goodnight_line("露娜", pick % 2 == 0, pick);
            assert!(s.contains("露娜"), "道晚安要點名對方");
            assert!(s.chars().count() <= SAY_CHARS, "不得超過泡泡上限：{s}");
            assert!(!s.contains("{o}"), "佔位符須全數替換");
        }
        // pick 輪替換句。
        assert_ne!(goodnight_line("露娜", false, 0), goodnight_line("露娜", false, 1));
    }

    #[test]
    fn goodnight_line_truncates_long_name() {
        let long = "超級無敵冗長的居民顯示名字一二三四五六七八九十".repeat(3);
        let s = goodnight_line(&long, true, 0);
        assert!(s.chars().count() <= SAY_CHARS, "超長名字也不得破框");
    }

    #[test]
    fn goodnight_reply_rotates_and_fits_frame() {
        for pick in 0..8 {
            let l = goodnight_reply_line(pick);
            assert!(!l.is_empty());
            assert!(l.chars().count() <= SAY_CHARS);
        }
        assert_ne!(goodnight_reply_line(0), goodnight_reply_line(1));
        assert_eq!(goodnight_reply_line(0), goodnight_reply_line(4));
    }

    #[test]
    fn feed_line_contains_both_names_and_fits() {
        let f = goodnight_feed_line("露娜", "諾娃");
        assert!(f.contains("露娜") && f.contains("諾娃"), "Feed 要含雙方名");
        assert!(f.chars().count() <= SAY_CHARS);
    }
}
