//! 乙太方界·居民會主動跟你聊起她的心事 v1（voxel-confide）。
//!
//! **北極星**：居民的內在生活至今只活在兩個地方——① 她自己的**渴望**（`voxel_desires`，
//! 玩家的話種下的夢想，驅動她去蓋家/做事）；② 玩家得**主動翻開日記**（`voxel_diary`）才
//! 讀得到的內心獨白。但這份內在，居民自己從來不會**主動開口**告訴你：你站在露娜面前，她
//! 頂多招呼一句、或回憶你們做過的事（`voxel_fond_greeting`），卻不會像個真朋友那樣，忽然
//! 掏心跟你說「其實我最近一直惦記著想蓋座瞭望台呢」。這一刀補上那一拍：夠熟的居民偶爾會
//! **主動把心裡當前那份渴望，當成心事對你說出口**——被動的日記內在，第一次變成她主動分享
//! 的話。而「對你掏了心」這件事本身，也記進她對你的記憶、讓交情更深一層。正中 PLAN_ETHERVOX
//! 核心信念「記憶要驅動行為、記憶是讓居民真的活著的土壤」：她的渴望不只驅動她的腳步，也第一
//! 次驅動她**對你說什麼**。
//!
//! **與既有招呼的定位區隔**：
//! - 老友情境問候（675，`voxel_fond_greeting`）回憶的是**你們一起做過的事**（著眼「我們」）；
//!   本刀說的是**她自己此刻的心事／渴望**（著眼「我」）——同樣是靠近時的一句話，一個朝外看
//!   你我、一個朝內看她自己。
//! - 名號招呼（774~777）著眼「你是誰」；本刀著眼「我最近在想什麼」。
//! - 日記（650）是玩家**主動翻開**才讀得到的被動內心；本刀是居民**主動開口**說出來。
//!
//! **純邏輯層**：是否開口（[`should_confide`]）、把渴望包成一句心事（[`confide_line`]）、
//! 掏心後記進記憶的摘要（[`confide_memory_line`]）全是確定性純函式，零 LLM、零鎖、零 IO。
//! 冷卻計時 / 渴望讀取 / 記憶寫入全在 `voxel_ws.rs`，沿用既有招呼那條已驗證的短鎖循序。
//!
//! **成本 / 濫用防護**：句子全走固定模板包住居民自己的渴望文字——渴望文字本就由 `voxel_desires`
//! 端規則擷取／截斷（≤ `DESIRE_MAX_CHARS`），**永不夾帶玩家原話**（無注入 / NSFW 風險）；
//! 只對好感達 [`CONFIDE_MIN_AFFINITY`] 的玩家開口（心事只對熟一點的人說），配合每位居民
//! [`CONFIDE_COOLDOWN_SECS`] 的長冷卻，稀有有份量、天然防洗版、也防好感（記憶筆數）被刷爆。
//! 零 migration、零新協議欄位、零前端改動、零新美術、FPS 零影響（純後端、僅招呼時序偶發）。

/// 居民願意對玩家掏心的最低好感（＝關於這位玩家的記憶筆數）。心事只對熟一點的人說：
/// 設 3——比陌生（0~2）多一點交情才開口，但不必到老友（`FOND_AFFINITY`=5）那麼深，
/// 好讓「她主動跟我聊心事」在情誼還在升溫的階段就有機會發生。
pub const CONFIDE_MIN_AFFINITY: usize = 3;

/// 同一位居民主動掏心的冷卻（秒）。設得長（240s＝4 分鐘）——掏心是偶爾為之的真心話，
/// 不是每次靠近都碎念，稀有才有份量，也把「靠聽心事刷好感」的速率天然夾死。
pub const CONFIDE_COOLDOWN_SECS: f32 = 240.0;

/// 心事泡泡的字元上限（與泡泡框上限一致，超出截斷不破框）。
pub const CONFIDE_SAY_MAX_CHARS: usize = 40;

/// 判斷此刻是否要主動掏心：好感夠（≥ [`CONFIDE_MIN_AFFINITY`]）＋ 冷卻到期 ＋ 過了機率門檻。
///
/// 純函式、確定性（機率骰由呼叫端傳入）。是否**有**心事可說（居民當前是否懷著渴望）由
/// 呼叫端另外查 `voxel_desires` 決定，本函式只把「熟不熟／該不該現在說」的門檻。
pub fn should_confide(affinity: usize, cooldown_ok: bool, roll: f32, chance: f32) -> bool {
    affinity >= CONFIDE_MIN_AFFINITY && cooldown_ok && roll < chance
}

/// 把居民當前的渴望文字，包成一句「主動對你說出口的心事」。
///
/// 依 `pick` 在幾組固定語氣模板間確定性輪替；渴望文字本身原封放進模板（已由 desires 端截過長、
/// 不含玩家原話）。整句以字元為單位截到 [`CONFIDE_SAY_MAX_CHARS`] 內，永不破泡泡框、永不回空。
///
/// `desire` 應為已 trim、非空的渴望摘要（呼叫端保證；空字串在此保守回一句通用心事）。
pub fn confide_line(desire: &str, pick: usize) -> String {
    let d = desire.trim();
    // 渴望空掉（理論上呼叫端已濾）→ 落回一句不倚賴內容的通用心事，仍是「主動分享」的味道。
    if d.is_empty() {
        const FALLBACK: [&str; 3] = [
            "其實啊，我心裡最近一直藏著一個小小的念頭呢。",
            "跟你說個心事——我最近老是想著一件事。",
            "不知怎地，最近我心裡總惦記著點什麼。",
        ];
        return FALLBACK[pick % FALLBACK.len()]
            .chars()
            .take(CONFIDE_SAY_MAX_CHARS)
            .collect();
    }
    // 幾組把渴望包成「主動掏心」語氣的前綴；渴望文字放最後，若整句過長就從尾端自然截斷。
    const TEMPLATES: [&str; 5] = [
        "跟你說個心事——我最近一直想著：",
        "其實啊，我心裡一直惦記著：",
        "偷偷跟你講，我最近好想：",
        "不瞞你說，我心裡有個念頭：",
        "我最近老是想著這件事：",
    ];
    let line = format!("{}{}", TEMPLATES[pick % TEMPLATES.len()], d);
    line.chars().take(CONFIDE_SAY_MAX_CHARS).collect()
}

/// 掏心之後，記進居民「關於這位玩家」的一筆記憶摘要（第一人稱、episodic）。
///
/// 刻意停在「我跟這位旅人說起了自己的心事」這個情節層——累積好感（記憶筆數），
/// 但不夾帶渴望原文、不誤觸目標／承諾等升級關鍵詞（維持與 `voxel_admire` 同款輕記憶）。
pub fn confide_memory_line(player: &str) -> String {
    format!("我跟{player}說起了自己藏在心裡的一個念頭，把心事分了一點給對方。")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confide_needs_affinity_cooldown_and_roll() {
        // 三條件齊備才開口。
        assert!(should_confide(CONFIDE_MIN_AFFINITY, true, 0.0, 0.5));
        assert!(should_confide(10, true, 0.49, 0.5));
        // 好感不足 → 否決。
        assert!(!should_confide(CONFIDE_MIN_AFFINITY - 1, true, 0.0, 0.5));
        // 冷卻未到 → 否決。
        assert!(!should_confide(10, false, 0.0, 0.5));
        // 骰子未過門檻 → 否決。
        assert!(!should_confide(10, true, 0.5, 0.5));
        assert!(!should_confide(10, true, 0.9, 0.5));
    }

    #[test]
    fn confide_line_wraps_desire_and_fits_frame() {
        for pick in 0..12 {
            let line = confide_line("想蓋一座能看見遠方的瞭望台", pick);
            assert!(!line.is_empty(), "心事句不該為空");
            assert!(
                line.chars().count() <= CONFIDE_SAY_MAX_CHARS,
                "心事句不該破泡泡框：{line}"
            );
        }
    }

    #[test]
    fn confide_line_truncates_overlong_desire() {
        // 渴望本身就頂到上限，加上前綴必然超框 → 必須截到框內、且非空。
        let long = "想去很遠很遠的水邊靜靜坐著釣一整個下午的魚順便看看夕陽";
        let line = confide_line(long, 0);
        assert!(line.chars().count() <= CONFIDE_SAY_MAX_CHARS);
        assert!(!line.is_empty());
    }

    #[test]
    fn confide_line_empty_desire_falls_back() {
        for pick in 0..6 {
            let line = confide_line("   ", pick);
            assert!(!line.is_empty(), "空渴望應落回通用心事、非空");
            assert!(line.chars().count() <= CONFIDE_SAY_MAX_CHARS);
        }
    }

    #[test]
    fn confide_line_deterministic_by_pick() {
        // 同 pick、同渴望 → 同一句（確定性，可測、可重現）。
        assert_eq!(confide_line("想種一片花田", 2), confide_line("想種一片花田", 2));
    }

    #[test]
    fn memory_line_contains_player_and_no_desire_text() {
        let m = confide_memory_line("諾瓦");
        assert!(m.contains("諾瓦"), "記憶應含玩家名");
        assert!(!m.is_empty());
        // 記憶刻意不含渴望原文（停在情節層、不夾帶內容）。
        assert!(!m.contains("瞭望台"));
    }
}
