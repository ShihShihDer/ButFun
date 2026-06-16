//! 天象圖鑑（ROADMAP 337）：把世界裡各種「天象奇觀」，第一次變成玩家親眼目睹蒐集、
//! 看得到進度的東西——開一條全新的「觀象」成長維度。
//!
//! 動機：333～335 開了「生態圖鑑」（走近生物即發現）、336 開了「探索圖鑑」（踏足地形即探索），
//! 把「世界裡活的生物」與「世界本身的地貌」都做成了可蒐集的成長線。可這片天空——它的雨、
//! 它的風沙、它劃過的流星、它圓滿的月——卻一直只是稍縱即逝的背景，沒有任何「我曾躬逢此景」
//! 的紀念被留下。本模組正面回應：**把那一票既有的天象奇觀，第一次轉成玩家親眼目睹、看得到
//! 進度的天象圖鑑**——身處某種天象之下即「目睹」它、永久記進圖鑑，每種第一次親見給一筆乙太。
//!
//! 與前兩條蒐集線的維度區隔（這正是 reviewer 自 #496/#498/#499 一再要的「不同成長維度」）：
//! - 生態圖鑑＝按「空間」蒐集（走到生物身邊）。
//! - 探索圖鑑＝按「空間」蒐集（走到奇景地形）。
//! - 天象圖鑑＝按「**時間**」蒐集（在對的時刻身處世界裡）——你無法靠跑去某個座標湊齊它，
//!   只能在流星雨劃過、滿月高懸的那一刻正好在線、正好抬頭。這是一種全新的「等待與躬逢」的蒐集味。
//!
//! 設計（刻意與 `terrain_atlas`／`field_guide` 同一套招數，最低風險、最易維護）：
//! - **純查表 / 純位元運算**，零 LLM、零額度、零平衡風險（獎勵刻意壓小、近乎零經濟擾動）。
//! - 已目睹集合壓成單一 `u64` bitmask（每位玩家一個整數），照 `atlas`（336）的模式持久化、
//!   跨重啟保留（蒐集進度若不存活就失去意義）。
//! - **天象→bit 的對應穩定不可重排**（持久化相容契約：bit 一旦指定就固定，
//!   日後新增天象一律往高位接，絕不插隊／重排，否則舊存檔的位元會錯位）。
//! - 面向玩家字串（名稱）集中在本檔 `CATALOG`，為 i18n 集中替換點。

/// 天象圖鑑一筆條目：給前端面板渲染與計數用。
#[derive(Debug, Clone, Copy)]
pub struct SkyEntry {
    /// 在 bitmask 裡的位元索引（穩定、不可重排）。
    pub bit: u8,
    /// 穩定 wire key（snake_case）：前端據此對應圖示與在地化字串。
    pub key: &'static str,
    /// 顯示名（繁中；i18n 集中替換點）。
    pub name: &'static str,
    /// 面板用 emoji。
    pub emoji: &'static str,
    /// 稀有度：`"common"` 常見 ／ `"rare"` 稀有 ／ `"legendary"` 傳奇（決定獎勵高低與面板配色）。
    pub tier: &'static str,
}

/// 全部天象圖鑑條目。bit 連續且穩定（0..N）。**順序與 bit 絕不可重排**（持久化相容）。
/// 大致由「常駐輪替、人人遲早遇得到」往「需守候時機、可遇難求」排。
pub const CATALOG: &[SkyEntry] = &[
    // ── 常見天象（四種世界天氣輪替而來，在線夠久遲早都遇得到）──
    SkyEntry { bit: 0, key: "grassland_rain",   name: "草原細雨", emoji: "🌧️", tier: "common" },
    SkyEntry { bit: 1, key: "desert_sandstorm", name: "沙漠風沙", emoji: "🌪️", tier: "common" },
    SkyEntry { bit: 2, key: "crystal_dust",     name: "岩地晶塵", emoji: "✨", tier: "common" },
    SkyEntry { bit: 3, key: "sea_mist",         name: "水域海霧", emoji: "🌊", tier: "common" },
    // ── 稀有天象（有時機限定，得正好在線才撞得上）──
    SkyEntry { bit: 4, key: "meteor_shower",    name: "流星雨",   emoji: "☄️", tier: "rare" },
    SkyEntry { bit: 5, key: "full_moon",        name: "滿月夜",   emoji: "🌕", tier: "rare" },
    // ── 傳奇天象（兩種稀有時機同時發生，可遇難求）──
    SkyEntry { bit: 6, key: "moonlit_meteor",   name: "滿月流星雨", emoji: "🌠", tier: "legendary" },
];

/// 天象圖鑑總條目數（前端顯示「N / TOTAL」，亦為合法位元上界）。
pub const TOTAL: u32 = CATALOG.len() as u32;

/// 各位元的索引常數（接線端引用，避免散落的魔術數字）。
pub const BIT_RAIN: u8 = 0;
pub const BIT_SANDSTORM: u8 = 1;
pub const BIT_CRYSTAL_DUST: u8 = 2;
pub const BIT_SEA_MIST: u8 = 3;
pub const BIT_METEOR: u8 = 4;
pub const BIT_FULL_MOON: u8 = 5;
pub const BIT_MOONLIT_METEOR: u8 = 6;

/// 親見一種天象給的乙太獎勵，依稀有度遞增（刻意壓小、近乎零經濟擾動，但愈難遇愈值得守候）。
pub const REWARD_COMMON: u32 = 4;
pub const REWARD_RARE: u32 = 10;
pub const REWARD_LEGENDARY: u32 = 20;

/// 世界天氣 wire key（對應 `weather::WeatherState::view()`）→ 天象圖鑑位元。
/// 晴天（`"clear"`）與任何未知值無「天象」可言，一律回 `None`。
pub fn bit_for_weather(weather_key: &str) -> Option<u8> {
    match weather_key {
        "grassland_rain" => Some(BIT_RAIN),
        "desert_sandstorm" => Some(BIT_SANDSTORM),
        "rocky_crystal_dust" => Some(BIT_CRYSTAL_DUST),
        "water_sea_mist" => Some(BIT_SEA_MIST),
        _ => None,
    }
}

/// 計算「當下這一刻天空裡，正在發生、可被目睹」的天象位元集合（純函式，是本模組可測的核心）。
///
/// 輸入皆為 game.rs 每幀已握有的全域訊號：
/// - `weather_key`：目前世界天氣的 wire key（`weather::WeatherState::view().kind`）。
/// - `meteor_active`：流星雨是否進行中（`meteor_shower::MeteorShowerState::is_active()`）。
/// - `full_moon_night`：是否「滿月」且「夜晚」（月亮要掛在夜空才看得見圓滿）。
///
/// 傳奇「滿月流星雨」需流星雨與滿月夜**同時**成立——兩種稀有時機湊在一起，可遇難求。
pub fn active_bits(weather_key: &str, meteor_active: bool, full_moon_night: bool) -> u64 {
    let mut mask = 0u64;
    if let Some(b) = bit_for_weather(weather_key) {
        mask |= 1u64 << b;
    }
    if meteor_active {
        mask |= 1u64 << BIT_METEOR;
    }
    if full_moon_night {
        mask |= 1u64 << BIT_FULL_MOON;
    }
    if meteor_active && full_moon_night {
        mask |= 1u64 << BIT_MOONLIT_METEOR;
    }
    mask
}

/// 某位元的乙太獎勵（依該條目稀有度查表）。位元越界回 0。
pub fn reward_for_bit(bit: u8) -> u32 {
    match CATALOG.iter().find(|e| e.bit == bit).map(|e| e.tier) {
        Some("legendary") => REWARD_LEGENDARY,
        Some("rare") => REWARD_RARE,
        Some("common") => REWARD_COMMON,
        _ => 0,
    }
}

/// 某位元是否已目睹。
pub fn is_witnessed(mask: u64, bit: u8) -> bool {
    mask & (1u64 << bit) != 0
}

/// 在天象圖鑑記下一個位元。回傳 `(新 mask, 是否本次才首次目睹)`。
/// 已目睹過則 mask 不變、回 `false`（天然冪等、不可重複領獎）。
pub fn witness(mask: u64, bit: u8) -> (u64, bool) {
    if is_witnessed(mask, bit) {
        (mask, false)
    } else {
        (mask | (1u64 << bit), true)
    }
}

/// 已目睹的天象數（只計合法位元，忽略任何高位雜訊）。
pub fn count(mask: u64) -> u32 {
    (mask & valid_mask()).count_ones()
}

/// 合法位元遮罩（0..TOTAL）。
fn valid_mask() -> u64 {
    if TOTAL >= 64 {
        u64::MAX
    } else {
        (1u64 << TOTAL) - 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// bit 連續、唯一、且在合法上界內（持久化相容契約）。
    #[test]
    fn bits_are_contiguous_unique_and_bounded() {
        for (i, e) in CATALOG.iter().enumerate() {
            assert_eq!(e.bit as usize, i, "bit 必須與索引一致（連續、不可重排）");
            assert!((e.bit as u32) < 64, "bit 必須落在 u64 範圍內");
        }
        assert_eq!(TOTAL as usize, CATALOG.len());
    }

    /// 索引常數與 CATALOG 順位一致（接線端引用的就是這些）。
    #[test]
    fn bit_constants_match_catalog() {
        assert_eq!(CATALOG[BIT_RAIN as usize].key, "grassland_rain");
        assert_eq!(CATALOG[BIT_SANDSTORM as usize].key, "desert_sandstorm");
        assert_eq!(CATALOG[BIT_CRYSTAL_DUST as usize].key, "crystal_dust");
        assert_eq!(CATALOG[BIT_SEA_MIST as usize].key, "sea_mist");
        assert_eq!(CATALOG[BIT_METEOR as usize].key, "meteor_shower");
        assert_eq!(CATALOG[BIT_FULL_MOON as usize].key, "full_moon");
        assert_eq!(CATALOG[BIT_MOONLIT_METEOR as usize].key, "moonlit_meteor");
    }

    /// key 不重複（前端據此對應圖示與在地化字串）。
    #[test]
    fn keys_are_unique_and_nonempty() {
        let mut seen = std::collections::HashSet::new();
        for e in CATALOG {
            assert!(!e.key.is_empty() && !e.name.is_empty() && !e.emoji.is_empty());
            assert!(seen.insert(e.key), "key 重複：{}", e.key);
        }
    }

    /// 四種世界天氣都對到正確位元；晴天與未知值不收錄。
    #[test]
    fn weather_keys_map_correctly() {
        let cases = [
            ("grassland_rain", BIT_RAIN),
            ("desert_sandstorm", BIT_SANDSTORM),
            ("rocky_crystal_dust", BIT_CRYSTAL_DUST),
            ("water_sea_mist", BIT_SEA_MIST),
        ];
        for (key, bit) in cases {
            assert_eq!(bit_for_weather(key), Some(bit), "{key} 應對到 bit {bit}");
        }
        assert_eq!(bit_for_weather("clear"), None, "晴天不算天象");
        assert_eq!(bit_for_weather("???"), None, "未知天氣不收錄");
    }

    /// active_bits：晴朗白天、無流星——天空無可目睹的天象。
    #[test]
    fn active_bits_clear_day_is_empty() {
        assert_eq!(active_bits("clear", false, false), 0);
    }

    /// active_bits：下雨時點亮細雨位元。
    #[test]
    fn active_bits_rain_lights_rain() {
        let m = active_bits("grassland_rain", false, false);
        assert!(is_witnessed(m, BIT_RAIN));
        assert!(!is_witnessed(m, BIT_METEOR));
        assert!(!is_witnessed(m, BIT_FULL_MOON));
    }

    /// active_bits：流星雨單獨進行——點亮流星，但不點亮傳奇（沒滿月夜）。
    #[test]
    fn active_bits_meteor_alone_no_legendary() {
        let m = active_bits("clear", true, false);
        assert!(is_witnessed(m, BIT_METEOR));
        assert!(!is_witnessed(m, BIT_MOONLIT_METEOR), "缺滿月夜不該點亮傳奇");
    }

    /// active_bits：滿月夜單獨——點亮滿月，但不點亮傳奇（沒流星雨）。
    #[test]
    fn active_bits_full_moon_alone_no_legendary() {
        let m = active_bits("clear", false, true);
        assert!(is_witnessed(m, BIT_FULL_MOON));
        assert!(!is_witnessed(m, BIT_MOONLIT_METEOR), "缺流星雨不該點亮傳奇");
    }

    /// active_bits：滿月夜 × 流星雨同時——點亮傳奇「滿月流星雨」（連同兩個稀有來源）。
    #[test]
    fn active_bits_meteor_and_full_moon_lights_legendary() {
        let m = active_bits("grassland_rain", true, true);
        assert!(is_witnessed(m, BIT_RAIN));
        assert!(is_witnessed(m, BIT_METEOR));
        assert!(is_witnessed(m, BIT_FULL_MOON));
        assert!(is_witnessed(m, BIT_MOONLIT_METEOR), "滿月夜＋流星雨同時應點亮傳奇");
    }

    /// witness 設位元、回報首次；重複目睹冪等不改 mask。
    #[test]
    fn witness_sets_bit_and_is_idempotent() {
        let (m1, first1) = witness(0, BIT_METEOR);
        assert!(first1);
        assert!(is_witnessed(m1, BIT_METEOR));
        let (m2, first2) = witness(m1, BIT_METEOR);
        assert!(!first2, "重複目睹不應再回首次");
        assert_eq!(m1, m2, "冪等：mask 不變");
    }

    /// count 正確計數，且忽略合法位元以外的高位雜訊。
    #[test]
    fn count_ignores_out_of_range_noise() {
        assert_eq!(count(0), 0);
        let full = valid_mask();
        assert_eq!(count(full), TOTAL);
        assert_eq!(count(full | (1u64 << 63)), TOTAL);
    }

    /// 獎勵依稀有度遞增、皆為正；位元越界回 0。
    #[test]
    fn rewards_increase_with_rarity() {
        assert!(REWARD_COMMON > 0);
        assert!(REWARD_RARE > REWARD_COMMON);
        assert!(REWARD_LEGENDARY > REWARD_RARE);
        for e in CATALOG {
            let r = reward_for_bit(e.bit);
            assert!(r > 0, "{} 的獎勵應為正", e.key);
            match e.tier {
                "common" => assert_eq!(r, REWARD_COMMON),
                "rare" => assert_eq!(r, REWARD_RARE),
                "legendary" => assert_eq!(r, REWARD_LEGENDARY),
                other => panic!("未知稀有度 {other}"),
            }
        }
        assert_eq!(reward_for_bit(200), 0, "越界位元無獎勵");
    }

    /// 每種稀有度至少各有一筆（面板三段配色都有內容）。
    #[test]
    fn every_tier_present() {
        for tier in ["common", "rare", "legendary"] {
            assert!(CATALOG.iter().any(|e| e.tier == tier), "缺少稀有度 {tier}");
        }
    }
}
