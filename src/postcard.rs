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

/// 星塵印記的稀有度（ROADMAP 447）：這張明信片有沒有封進流星雨採集來的星塵。
/// `None`＝一般明信片；`Stardust`＝封進星塵；`Rainbow`＝封進每場流星雨僅一粒的彩虹星塵。
/// 把長久以來只能堆在背包 / 賣 NPC 的星塵（ROADMAP 133／134）接到「留念」這個既有去處。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StarTier {
    /// 沒封星塵的一般明信片。
    None,
    /// 封進一粒星塵。
    Stardust,
    /// 封進罕見的彩虹星塵。
    Rainbow,
}

impl StarTier {
    /// wire key（snake_case 契約；前端據此切換星光呈現，留 i18n 空間）。
    pub fn wire_key(self) -> &'static str {
        match self {
            StarTier::None => "none",
            StarTier::Stardust => "stardust",
            StarTier::Rainbow => "rainbow",
        }
    }
}

/// 星塵印記的一句話（封進星塵後，明信片底下多出的那行星空留言）。
/// 一般明信片（`None`）沒有星塵印記、回 `None`；封了星塵則依時辰確定性挑一句，
/// 彩虹星塵更稀有、用另一套更亮的句子。同一時辰永遠回同一句（可重現、零亂數）。
pub fn star_line_for(phase: Phase, tier: StarTier) -> Option<&'static str> {
    match tier {
        StarTier::None => None,
        StarTier::Stardust => Some(match phase {
            Phase::Dawn => "晨星未散，把一撮星塵也收進了這張卡裡。",
            Phase::Day => "白日裡握著的星塵，仍記得昨夜的那場流星。",
            Phase::Dusk => "暮色剛起，星塵在指間悄悄亮了一下。",
            Phase::Night => "流星雨落下的星塵，正和滿天星子一起閃。",
        }),
        StarTier::Rainbow => Some(match phase {
            Phase::Dawn => "晨光裡，一粒彩虹星塵把七彩都揉進了卡角。",
            Phase::Day => "罕見的彩虹星塵在掌心折出一道小小的虹。",
            Phase::Dusk => "晚霞與彩虹星塵同色，這一刻被永遠框住了。",
            Phase::Night => "整場流星雨只此一粒的彩虹星塵，在卡上靜靜生輝。",
        }),
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
    /// 星塵印記（ROADMAP 447）：呼叫端依玩家是否消耗了星塵／彩虹星塵填入。一般明信片用 `StarTier::None`。
    pub star: StarTier,
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
    /// 星塵印記稀有度（ROADMAP 447）：前端據此切換星光呈現。
    pub star_tier: StarTier,
    /// 星塵印記留言（封了星塵才有；一般明信片為 `None`）。
    pub star_line: Option<&'static str>,
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
        star_tier: input.star,
        star_line: star_line_for(input.phase, input.star),
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
            star: StarTier::None,
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
            star: StarTier::None,
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
            star: StarTier::None,
        };
        assert_eq!(compose(mk()), compose(mk()));
    }

    // ── 星塵印記（ROADMAP 447）────────────────────────────────────────────

    /// 一般明信片（未封星塵）沒有星塵留言，所有時辰皆回 None。
    #[test]
    fn star_line_none_for_plain_postcard() {
        for p in [Phase::Dawn, Phase::Day, Phase::Dusk, Phase::Night] {
            assert_eq!(star_line_for(p, StarTier::None), None);
        }
    }

    /// 封了星塵／彩虹星塵後，每個時辰都有一句非空的星塵留言。
    #[test]
    fn star_line_present_and_non_empty_when_starlit() {
        for p in [Phase::Dawn, Phase::Day, Phase::Dusk, Phase::Night] {
            for t in [StarTier::Stardust, StarTier::Rainbow] {
                let line = star_line_for(p, t).expect("封了星塵應有留言");
                assert!(!line.is_empty(), "({:?},{:?}) 星塵留言不可為空", p, t);
            }
        }
    }

    /// 彩虹星塵的留言與一般星塵不同（更稀有的呈現），同時辰兩者不撞句。
    #[test]
    fn rainbow_line_differs_from_stardust() {
        for p in [Phase::Dawn, Phase::Day, Phase::Dusk, Phase::Night] {
            assert_ne!(
                star_line_for(p, StarTier::Stardust),
                star_line_for(p, StarTier::Rainbow),
                "{:?} 的彩虹星塵留言不該與一般星塵相同",
                p
            );
        }
    }

    /// 星塵留言確定性：同（時辰, 稀有度）每次都回同一句。
    #[test]
    fn star_line_is_deterministic() {
        assert_eq!(
            star_line_for(Phase::Night, StarTier::Rainbow),
            star_line_for(Phase::Night, StarTier::Rainbow)
        );
    }

    /// wire key 三檔互不相同（前端切換呈現的契約）。
    #[test]
    fn star_tier_wire_keys_distinct() {
        let keys = [
            StarTier::None.wire_key(),
            StarTier::Stardust.wire_key(),
            StarTier::Rainbow.wire_key(),
        ];
        assert_eq!(keys[0], "none");
        assert_eq!(keys[1], "stardust");
        assert_eq!(keys[2], "rainbow");
        // 兩兩不同
        assert_ne!(keys[0], keys[1]);
        assert_ne!(keys[1], keys[2]);
        assert_ne!(keys[0], keys[2]);
    }

    /// compose 帶過星塵印記：封了星塵時 star_tier／star_line 都對得上 star_line_for。
    #[test]
    fn compose_carries_star_imprint() {
        let pc = compose(PostcardInput {
            level: 18,
            place: "流星崖".into(),
            subtitle: "星塵在夜風裡打轉".into(),
            phase: Phase::Night,
            season: Season::Autumn,
            star: StarTier::Rainbow,
        });
        assert_eq!(pc.star_tier, StarTier::Rainbow);
        assert_eq!(pc.star_line, star_line_for(Phase::Night, StarTier::Rainbow));
        assert!(pc.star_line.is_some());
        // 其餘欄位不受星塵影響（風景印記仍照時辰季節組）。
        assert_eq!(pc.flavor, flavor_for(Phase::Night, Season::Autumn));
    }

    /// compose 一般明信片：未封星塵時 star_tier=None、star_line=None。
    #[test]
    fn compose_plain_has_no_star_imprint() {
        let pc = compose(PostcardInput {
            level: 3,
            place: "微風原".into(),
            subtitle: "野花一路鋪到天邊".into(),
            phase: Phase::Day,
            season: Season::Summer,
            star: StarTier::None,
        });
        assert_eq!(pc.star_tier, StarTier::None);
        assert_eq!(pc.star_line, None);
    }
}
