//! 旅途明信片（ROADMAP 417）——一鍵把「此刻的世界」框成一張可收藏／分享的明信片。
//!
//! cozy 遊戲常備、ButFun 從無的「捕捉留念（keepsake）」維度：玩家在世界任一處按下「📷 留影」，
//! 伺服器以當下的權威狀態（所在地名／地誌氛圍／季節／時辰／旅人資歷）組出一張明信片的文字內容，
//! 前端框成風景卡，玩家可下載收藏／分享。純療癒向、零經濟擾動、零 LLM。
//!
//! 設計鐵律：
//! - **純邏輯、確定性、可測**：零 IO／零鎖／零 LLM／零持久化（只讀既有狀態算出明信片內容）。
//! - 同一組輸入永遠組出同一張明信片（時辰×季節決定「此刻風景印記」），同一個地方留影氛圍一致。
//! - 面向玩家字串集中在本模組（單一可替換來源，留 i18n 空間）；地名／副標沿用 region_name 既有權威字串。

use crate::daynight::Phase;
use crate::season::Season;

/// 旅人資歷稱號（依等級分四檔，純顯示用；門檻對齊 player_title 的等級里程碑精神）。
/// 壞值（理論上 level 不為負）也保守落在最低檔，不冤枉玩家。
pub fn rank_for(level: u32) -> &'static str {
    if level >= 30 {
        "傳說旅人"
    } else if level >= 20 {
        "冒險家"
    } else if level >= 10 {
        "旅者"
    } else {
        "見習旅人"
    }
}

/// 時辰的風景標題詞（明信片標頭用）。
pub fn phase_label(phase: Phase) -> &'static str {
    match phase {
        Phase::Dawn => "晨光",
        Phase::Day => "日午",
        Phase::Dusk => "暮色",
        Phase::Night => "星夜",
    }
}

/// 一句「此刻風景印記」：依（時辰, 季節）確定性挑一句溫柔短語。
/// 四時辰 × 四季 = 16 句，皆非空、皆守療癒基調（明信片底下那行手寫感的話）。
pub fn flavor_for(phase: Phase, season: Season) -> &'static str {
    match (phase, season) {
        (Phase::Dawn, Season::Spring) => "晨露沾著新芽，一天剛要醒來。",
        (Phase::Dawn, Season::Summer) => "天光早早就亮了，連風都帶著暖意。",
        (Phase::Dawn, Season::Autumn) => "薄霧裡飄著第一片落葉，清涼得剛剛好。",
        (Phase::Dawn, Season::Winter) => "霜把世界鋪成淡白，呼吸都看得見。",
        (Phase::Day, Season::Spring) => "陽光把花香曬得滿地都是。",
        (Phase::Day, Season::Summer) => "午後的光很滿，影子縮成了一小團。",
        (Phase::Day, Season::Autumn) => "金黃的光從葉縫漏下來，慢慢的。",
        (Phase::Day, Season::Winter) => "冷冽的晴空下，遠方格外清楚。",
        (Phase::Dusk, Season::Spring) => "晚霞把花田染成一片溫柔的橘。",
        (Phase::Dusk, Season::Summer) => "暑氣散去，晚風替整片大地降了溫。",
        (Phase::Dusk, Season::Autumn) => "夕陽和落葉同一個顏色，美得安靜。",
        (Phase::Dusk, Season::Winter) => "天色暗得早，遠處亮起了第一盞燈。",
        (Phase::Night, Season::Spring) => "夜裡花還醒著，星子也跟著眨眼。",
        (Phase::Night, Season::Summer) => "夏夜很長，銀河在頭頂慢慢地流。",
        (Phase::Night, Season::Autumn) => "秋夜清朗，星星數都數不完。",
        (Phase::Night, Season::Winter) => "寒夜寂靜，每一顆星都格外亮。",
    }
}

/// 組明信片的輸入（呼叫端從既有權威狀態填入：地名／副標來自 region_name，季節／時辰來自世界時鐘）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostcardInput {
    /// 旅人等級（決定資歷稱號）。
    pub level: u32,
    /// 所在地名（region_name::Locale.name）。
    pub place: String,
    /// 地誌氛圍副標（region_name::Locale.subtitle）。
    pub subtitle: String,
    /// 當下時辰。
    pub phase: Phase,
    /// 當下季節。
    pub season: Season,
}

/// 組好的一張明信片（純文字內容；前端據此框成風景卡，留 i18n 空間）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Postcard {
    /// 標頭，如「晨光・🌸 春」。
    pub title: String,
    /// 地名。
    pub place: String,
    /// 地誌氛圍副標。
    pub subtitle: String,
    /// 旅人資歷稱號。
    pub rank: &'static str,
    /// 此刻風景印記（手寫感的一句話）。
    pub flavor: &'static str,
    /// 旅人等級（明信片落款用）。
    pub level: u32,
}

/// 以當下世界狀態組一張明信片。確定性、零副作用。
pub fn compose(input: PostcardInput) -> Postcard {
    let title = format!("{}・{}", phase_label(input.phase), input.season.display_name());
    Postcard {
        title,
        place: input.place,
        subtitle: input.subtitle,
        rank: rank_for(input.level),
        flavor: flavor_for(input.phase, input.season),
        level: input.level,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 資歷稱號門檻：邊界值準確分檔。
    #[test]
    fn rank_thresholds() {
        assert_eq!(rank_for(0), "見習旅人");
        assert_eq!(rank_for(9), "見習旅人");
        assert_eq!(rank_for(10), "旅者");
        assert_eq!(rank_for(19), "旅者");
        assert_eq!(rank_for(20), "冒險家");
        assert_eq!(rank_for(29), "冒險家");
        assert_eq!(rank_for(30), "傳說旅人");
        assert_eq!(rank_for(u32::MAX), "傳說旅人"); // 極端值仍落在最高檔，不 panic
    }

    /// 時辰標題詞四種皆有對應、皆非空。
    #[test]
    fn phase_labels_all_present() {
        for p in [Phase::Dawn, Phase::Day, Phase::Dusk, Phase::Night] {
            assert!(!phase_label(p).is_empty());
        }
    }

    /// 風景印記涵蓋全部 4×4 組合，皆非空。
    #[test]
    fn flavor_covers_all_combos_non_empty() {
        for p in [Phase::Dawn, Phase::Day, Phase::Dusk, Phase::Night] {
            for s in [Season::Spring, Season::Summer, Season::Autumn, Season::Winter] {
                assert!(!flavor_for(p, s).is_empty(), "({:?},{:?}) 風景印記不可為空", p, s);
            }
        }
    }

    /// 確定性：同一組（時辰, 季節）每次都回同一句。
    #[test]
    fn flavor_is_deterministic() {
        let a = flavor_for(Phase::Dusk, Season::Autumn);
        let b = flavor_for(Phase::Dusk, Season::Autumn);
        assert_eq!(a, b);
    }

    /// compose 標頭含時辰詞與季節名。
    #[test]
    fn compose_title_includes_phase_and_season() {
        let pc = compose(PostcardInput {
            level: 12,
            place: "晨露谷".into(),
            subtitle: "薄霧在草尖上打盹".into(),
            phase: Phase::Dawn,
            season: Season::Spring,
        });
        assert!(pc.title.contains("晨光"));
        assert!(pc.title.contains("春"));
    }

    /// compose 原樣帶過地名／副標／等級，並依等級填對資歷。
    #[test]
    fn compose_carries_fields() {
        let pc = compose(PostcardInput {
            level: 25,
            place: "翡翠林".into(),
            subtitle: "苔蘚把石頭都養綠了".into(),
            phase: Phase::Night,
            season: Season::Winter,
        });
        assert_eq!(pc.place, "翡翠林");
        assert_eq!(pc.subtitle, "苔蘚把石頭都養綠了");
        assert_eq!(pc.level, 25);
        assert_eq!(pc.rank, "冒險家");
        assert_eq!(pc.flavor, flavor_for(Phase::Night, Season::Winter));
    }

    /// compose 確定性：同輸入組出完全相同的明信片。
    #[test]
    fn compose_is_deterministic() {
        let mk = || PostcardInput {
            level: 7,
            place: "微風原".into(),
            subtitle: "野花一路鋪到天邊".into(),
            phase: Phase::Day,
            season: Season::Summer,
        };
        assert_eq!(compose(mk()), compose(mk()));
    }
}
