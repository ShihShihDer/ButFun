//! ROADMAP 445「世界此刻一瞥」——登入畫面的活世界氛圍（純邏輯）。
//!
//! 每位新手／回訪玩家進場前都會停在登入畫面。443 給它輪播玩法提示、613 給它在線數，
//! 但畫面本身與「此刻真實世界發生什麼」毫無關係——不管你清晨還是深夜、春耕還是冬藏、
//! 晴天還是落雨，登入畫面永遠長一個樣。本模組讓登入畫面第一次「映出世界此刻的樣子」：
//! 依伺服器當下的**時辰、季節、天氣、線上人數**，組一句暖暖的「世界此刻」近況，
//! 讓玩家還沒進場就感到「這是個活著、每天都不一樣的世界」——first-impression 的留存。
//!
//! 純邏輯（無 I/O、無亂數、確定可測）；經 `/api/status` 彙總輸出，零個資、零持久化、零 migration。
//! 面向玩家字串集中在本模組（i18n 空間），繁中註解。

use crate::daynight::Phase;
use crate::season::Season;
use crate::weather::WeatherType;

/// 「世界此刻」一瞥。`theme` 是給前端套氛圍色的機器 key（不直接顯示），
/// `headline`／`subline` 是兩行面向玩家的繁中暖句。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Glimpse {
    /// 氛圍主題 key：`dawn` / `day` / `dusk` / `night`（對齊 `Phase`，前端據此套登入畫面色調）。
    pub theme: &'static str,
    /// 主行：時辰 × 季節，如「🌅 破曉的微光漫過秋日的大地。」
    pub headline: String,
    /// 副行：線上人數 ×（可選）天氣補述，如「此刻有 3 位旅人散落在世界各處，細雨正落。」
    pub subline: String,
}

/// 時辰主題 key（前端套色用）。
fn phase_theme(phase: Phase) -> &'static str {
    match phase {
        Phase::Dawn => "dawn",
        Phase::Day => "day",
        Phase::Dusk => "dusk",
        Phase::Night => "night",
    }
}

/// 時辰的 emoji（主行開頭）。
fn phase_emoji(phase: Phase) -> &'static str {
    match phase {
        Phase::Dawn => "🌅",
        Phase::Day => "🌞",
        Phase::Dusk => "🌇",
        Phase::Night => "🌙",
    }
}

/// 時辰的氛圍動詞片語（接「{季節}日的大地」）。
fn phase_phrase(phase: Phase) -> &'static str {
    match phase {
        Phase::Dawn => "破曉的微光漫過",
        Phase::Day => "日光鋪滿",
        Phase::Dusk => "黃昏的霞光斜照",
        Phase::Night => "夜色溫柔籠著",
    }
}

/// 季節的純名稱（不含 emoji，供「{季節}日」這種句法）。
fn season_plain(season: Season) -> &'static str {
    match season {
        Season::Spring => "春",
        Season::Summer => "夏",
        Season::Autumn => "秋",
        Season::Winter => "冬",
    }
}

/// 天氣補述（接在線上人數之後）。晴天回 `None`（不囉嗦）。
fn weather_note(weather: WeatherType) -> Option<&'static str> {
    match weather {
        WeatherType::Clear => None,
        WeatherType::GrasslandRain => Some("細雨正落"),
        WeatherType::DesertSandstorm => Some("沙塵漫天"),
        WeatherType::RockyCrystalDust => Some("晶塵在風裡微微發亮"),
        WeatherType::WaterSeaMist => Some("海霧瀰漫"),
    }
}

/// 組出「世界此刻」一瞥。純函式、確定可測。
pub fn compose(phase: Phase, season: Season, weather: WeatherType, online: usize) -> Glimpse {
    let headline = format!(
        "{} {}{}日的大地。",
        phase_emoji(phase),
        phase_phrase(phase),
        season_plain(season),
    );

    // 線上人數：0 人也不冷場，溫柔邀請；1 人單數句；多人散落各處。
    let people = match online {
        0 => "世界正靜靜等著第一位旅人到來".to_string(),
        1 => "此刻有 1 位旅人在世界裡忙著".to_string(),
        n => format!("此刻有 {} 位旅人散落在世界各處", n),
    };
    let subline = match weather_note(weather) {
        Some(note) => format!("{}，{}。", people, note),
        None => format!("{}。", people),
    };

    Glimpse { theme: phase_theme(phase), headline, subline }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_tracks_phase() {
        assert_eq!(compose(Phase::Dawn, Season::Spring, WeatherType::Clear, 1).theme, "dawn");
        assert_eq!(compose(Phase::Day, Season::Spring, WeatherType::Clear, 1).theme, "day");
        assert_eq!(compose(Phase::Dusk, Season::Spring, WeatherType::Clear, 1).theme, "dusk");
        assert_eq!(compose(Phase::Night, Season::Spring, WeatherType::Clear, 1).theme, "night");
    }

    #[test]
    fn headline_weaves_phase_and_season() {
        let g = compose(Phase::Dawn, Season::Autumn, WeatherType::Clear, 3);
        assert!(g.headline.contains("🌅"), "破曉應帶 🌅");
        assert!(g.headline.contains("秋日的大地"), "應織入季節：{}", g.headline);
        // 冬夜不同詞
        let n = compose(Phase::Night, Season::Winter, WeatherType::Clear, 0);
        assert!(n.headline.contains("🌙") && n.headline.contains("冬日的大地"));
    }

    #[test]
    fn people_line_handles_zero_one_many() {
        assert!(compose(Phase::Day, Season::Spring, WeatherType::Clear, 0).subline.contains("第一位旅人"));
        let one = compose(Phase::Day, Season::Spring, WeatherType::Clear, 1).subline;
        assert!(one.contains("1 位旅人") && one.contains("在世界裡忙著"));
        assert!(compose(Phase::Day, Season::Spring, WeatherType::Clear, 42).subline.contains("42 位旅人散落"));
    }

    #[test]
    fn clear_weather_adds_no_note_but_others_do() {
        // 晴天：副行不含逗號接的天氣補述（以句號收尾、無「，」）。
        let clear = compose(Phase::Day, Season::Spring, WeatherType::Clear, 2).subline;
        assert!(clear.ends_with("。") && !clear.contains("，"));
        // 雨天：補上「細雨正落」。
        let rain = compose(Phase::Day, Season::Spring, WeatherType::GrasslandRain, 2).subline;
        assert!(rain.contains("細雨正落"));
        // 其餘天氣各有補述。
        assert!(compose(Phase::Day, Season::Spring, WeatherType::DesertSandstorm, 2).subline.contains("沙塵漫天"));
        assert!(compose(Phase::Day, Season::Spring, WeatherType::RockyCrystalDust, 2).subline.contains("晶塵"));
        assert!(compose(Phase::Day, Season::Spring, WeatherType::WaterSeaMist, 2).subline.contains("海霧"));
    }

    #[test]
    fn all_combinations_produce_nonempty_lines() {
        let phases = [Phase::Dawn, Phase::Day, Phase::Dusk, Phase::Night];
        let seasons = [Season::Spring, Season::Summer, Season::Autumn, Season::Winter];
        let weathers = [
            WeatherType::Clear,
            WeatherType::GrasslandRain,
            WeatherType::DesertSandstorm,
            WeatherType::RockyCrystalDust,
            WeatherType::WaterSeaMist,
        ];
        for &p in &phases {
            for &s in &seasons {
                for &w in &weathers {
                    let g = compose(p, s, w, 5);
                    assert!(!g.headline.is_empty() && !g.subline.is_empty());
                    assert!(!g.theme.is_empty());
                }
            }
        }
    }
}
