//! 乙太方界·時令作物 v1（ROADMAP 811）——讓季節第一次真的牽動玩法。
//!
//! 季節輪替（798）至今只是替天地換個色調、換季那一刻居民抬頭感嘆一句——對「玩家怎麼玩」
//! **零影響**：不論春夏秋冬，種小麥都是等 90 秒、種胡蘿蔔都是等 60 秒，季節純屬背景裝飾。
//! 本刀給季節長出第一顆玩法牙齒：**每種作物都有它的「時令」季節**，種在時令裡的作物一種下
//! 就先抽長一截（沿用居民照料 753／沃肥 789 那套 `nudge_growth` head-start），比平時更快成熟。
//! 你看過整片天地換上夏色，如今那份夏意會真的讓你田裡的麥子長得更起勁——季節不再只是好看。
//!
//! **療癒優先、只獎不罰**：非時令**不減速、不枯萎、不損資料**（守資料安全鐵律），只是沒有那份
//! 時令加成。冬天萬物歇息（冬藏），四種作物皆非時令——冬天種田照長，只是沒有誰特別得寵。
//!
//! **時令對應**（取現實節氣意象，確定性）：
//! - 🥕 胡蘿蔔 → **春**（初春播種的根菜）
//! - 🌾 小麥 → **夏**（金黃的盛夏麥浪）
//! - 🥔 馬鈴薯 → **秋**（秋收的塊莖）
//! - ❄️ 冬 → 無時令作物（冬藏、萬物歇息）
//!
//! **純邏輯層**：確定性、零 LLM、零持久化、零 migration、可測；季節取得、`nudge_growth`
//! head-start、回饋廣播全在 `voxel_ws.rs` 的種植 handler 接線。

use crate::voxel_farm::CropKind;
use crate::voxel_season::Season;

/// 時令加成的 head-start 比例分母：加成 = 該作物基礎生長秒數 / [`HEAD_START_DIVISOR`]。
/// 取 3（≈提早三分之一路程）——有感但不至於讓時令種植瞬間收成、破壞療癒節奏。
/// 例：小麥基礎 90 秒 → 提早 30 秒；胡蘿蔔 60 → 20；馬鈴薯 120 → 40。
pub const HEAD_START_DIVISOR: u64 = 3;

/// 回傳某作物的「時令」季節。四種作物各對應春／夏／秋其一；冬天不是任何作物的時令。
pub fn favored_season(kind: CropKind) -> Season {
    match kind {
        CropKind::Carrot => Season::Spring, // 初春播種的根菜
        CropKind::Wheat => Season::Summer,  // 金黃的盛夏麥浪
        CropKind::Potato => Season::Autumn, // 秋收的塊莖
        CropKind::Pumpkin => Season::Autumn, // 秋收的沉甸甸果實（季限作物·秋南瓜 v1；也只在秋天種得起來）
    }
}

/// 此刻的季節是否正是這種作物的時令。
pub fn is_in_season(kind: CropKind, season: Season) -> bool {
    favored_season(kind) == season
}

/// 時令加成給的 head-start 秒數：基礎生長秒數 / [`HEAD_START_DIVISOR`]。
///
/// - 以整數除法計算，`saturating`／飽和不溢位；
/// - 基礎秒數 > 0 時至少回 1 秒（除法歸零也給一點點加成，別讓時令變成無感的空頭支票）。
pub fn head_start_secs(base_grow_secs: u64) -> u64 {
    if base_grow_secs == 0 {
        return 0;
    }
    (base_grow_secs / HEAD_START_DIVISOR).max(1)
}

/// 作物中文名（面向玩家字串集中此處，便於日後 i18n）。
pub fn crop_name_zh(kind: CropKind) -> &'static str {
    match kind {
        CropKind::Wheat => "小麥",
        CropKind::Carrot => "胡蘿蔔",
        CropKind::Potato => "馬鈴薯",
        CropKind::Pumpkin => "南瓜",
    }
}

/// 種在時令季節時給玩家的暖回饋句（確定性、嵌作物名與季節名）。
///
/// 僅在 [`is_in_season`] 為真時呼叫；非時令不冒此句（只獎不罰、不嘮叨）。
pub fn in_season_line(kind: CropKind, season: Season) -> String {
    format!(
        "☀️ 此刻正是{}的時令，它在這{}裡會長得特別起勁！",
        crop_name_zh(kind),
        season.display_name(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 時令對應表：三種作物各自對應春／夏／秋，且互不相同。
    #[test]
    fn favored_season_table() {
        assert_eq!(favored_season(CropKind::Carrot), Season::Spring);
        assert_eq!(favored_season(CropKind::Wheat), Season::Summer);
        assert_eq!(favored_season(CropKind::Potato), Season::Autumn);
        // 三種作物的時令兩兩相異（各佔一季，不重疊）。
        let seasons = [
            favored_season(CropKind::Carrot),
            favored_season(CropKind::Wheat),
            favored_season(CropKind::Potato),
        ];
        for i in 0..seasons.len() {
            for j in (i + 1)..seasons.len() {
                assert_ne!(seasons[i], seasons[j], "作物時令不應重疊");
            }
        }
    }

    /// 冬天不是任何作物的時令（冬藏）。
    #[test]
    fn winter_favors_nothing() {
        for kind in [CropKind::Wheat, CropKind::Carrot, CropKind::Potato] {
            assert!(
                !is_in_season(kind, Season::Winter),
                "冬天不該是任何作物的時令"
            );
        }
    }

    /// `is_in_season` 對四季 × 三作物窮舉：每種作物恰好只有一季為真。
    #[test]
    fn is_in_season_exhaustive() {
        let all_seasons = [
            Season::Spring,
            Season::Summer,
            Season::Autumn,
            Season::Winter,
        ];
        for kind in [CropKind::Wheat, CropKind::Carrot, CropKind::Potato] {
            let hits = all_seasons
                .iter()
                .filter(|&&s| is_in_season(kind, s))
                .count();
            assert_eq!(hits, 1, "每種作物應恰好只有一季為時令");
            // 那一季就是 favored_season。
            assert!(is_in_season(kind, favored_season(kind)));
        }
    }

    /// head-start 秒數：基礎 / 3，且基礎為正時至少 1 秒。
    #[test]
    fn head_start_math() {
        assert_eq!(head_start_secs(90), 30); // 小麥
        assert_eq!(head_start_secs(60), 20); // 胡蘿蔔
        assert_eq!(head_start_secs(120), 40); // 馬鈴薯
        assert_eq!(head_start_secs(0), 0); // 沒基礎就沒加成
        assert_eq!(head_start_secs(1), 1); // 除法歸零也保底 1 秒
        assert_eq!(head_start_secs(2), 1);
    }

    /// head-start 對極大基礎秒數不 panic（飽和／整數除法安全）。
    #[test]
    fn head_start_no_overflow() {
        let h = head_start_secs(u64::MAX);
        assert!(h > 0 && h <= u64::MAX);
    }

    /// 時令回饋句：非空、嵌得到作物名與季節名。
    #[test]
    fn in_season_line_content() {
        let line = in_season_line(CropKind::Wheat, Season::Summer);
        assert!(line.contains("小麥"));
        assert!(line.contains("夏天"));
        assert!(!line.is_empty());
        // 其餘作物也各自嵌名正確。
        assert!(in_season_line(CropKind::Carrot, Season::Spring).contains("胡蘿蔔"));
        assert!(in_season_line(CropKind::Potato, Season::Autumn).contains("馬鈴薯"));
    }

    /// 作物名對照。
    #[test]
    fn crop_names() {
        assert_eq!(crop_name_zh(CropKind::Wheat), "小麥");
        assert_eq!(crop_name_zh(CropKind::Carrot), "胡蘿蔔");
        assert_eq!(crop_name_zh(CropKind::Potato), "馬鈴薯");
        assert_eq!(crop_name_zh(CropKind::Pumpkin), "南瓜");
    }

    /// 季限作物·秋南瓜 v1：南瓜的時令為秋天（吃得到 811 種植 head-start 與 812 收成豐收），
    /// 冬天仍非其時令。
    #[test]
    fn pumpkin_favored_autumn() {
        assert_eq!(favored_season(CropKind::Pumpkin), Season::Autumn);
        assert!(is_in_season(CropKind::Pumpkin, Season::Autumn));
        assert!(!is_in_season(CropKind::Pumpkin, Season::Winter));
    }
}
