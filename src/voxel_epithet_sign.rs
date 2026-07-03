//! 乙太方界·居民把你掙得的名號刻成一塊牌，立在自家門旁 v1（自主提案切片）。
//!
//! **設計依據**：774（`voxel_playerepithet`）讓一位居民把「關於某位玩家的累積作為」
//! 昇華成一個**名號**（造物者／慷慨的人／老搭檔／常來的老友），打招呼時改用名號稱呼你；
//! 775（`voxel_epithet_spread`）讓這個名號在居民之間口耳相傳。但至此為止，你掙得的名號
//! 只活在**居民的嘴上與心裡**——是一句招呼、一則動態，說過就散在空氣裡，世界的方塊天地
//! 裡不留任何痕跡。你可以在露娜家旁蓋了半天、被喚了無數次「造物者」，可轉身離開，這片
//! 世界看起來跟你來之前一模一樣。
//!
//! 本切片把名號**從口說變成實體**：當一位居民第一次為你安下名號，她會在自家門旁**刻一塊
//! 告示牌**，把你的名號留在世界裡——「此地常客·造物者」。你日後路過，還看得見那塊牌立在
//! 那兒。你掙得的名聲第一次成為這片方塊天地裡一處**永久、可走近、可讀**的印記。
//!
//! **換維度（非同軸重複）**：774／775 是名號的「口說」面（招呼／傳聞／動態）；本刀是名號的
//! 「**實體世界後果**」面——比照 keepsake（732，把玩家送的禮物擺成世界方塊）把「你的互動有
//! 後果」推進到**永久改變世界佈局**的維度，只是這回被實體化的不是一份禮物，而是你掙得的**身分**。
//! 全新維度（聲望→世界方塊），不與 774／775 的任何口說分支重疊。
//!
//! **純邏輯層**：本檔全是零 IO、零鎖、零 LLM、零 async 的確定性純函式——牌面文字、刻牌泡泡、
//! 動態牆旁白皆可單元測試；找空地立牌／持久化／廣播／去重掃描全在 `voxel_ws.rs`（沿用
//! 749 立牌命名同一套 Sign 方塊＋SignStore＋JSONL 管線）。
//!
//! **濫用防護**：不新增對外端點、不觸發 LLM、不收玩家自由文字；牌面／泡泡／動態全為固定模板、
//! 只嵌玩家**顯示名**（本就出現在招呼／動態牆），**永不回放記憶原文或玩家原話**（無注入／NSFW 面）；
//! 名號本身仍需 774 的持續主導作為（≥4 筆且領先次多 ≥2）才昇華＝源頭天然防「刷一兩次就立牌」；
//! 立牌每位居民對每位玩家至多一塊（呼叫端以既有 SignStore 掃描去重，重啟安全），無洗版風險。

use crate::voxel_playerepithet::PlayerRole;
use crate::voxel_sign::sanitize_text;

/// 牌面的固定前綴：一個穩定、可辨識的標記，讓「這是一塊名號榮譽牌」既一眼可讀、
/// 又能被呼叫端拿來**去重掃描**（判斷某塊牌是否已是給這位玩家的名號牌）。
pub const HONOR_PREFIX: &str = "此地常客·";

/// Feed 播報種類名稱（前端未知種類安全落回 📌，additive）。
pub const FEED_KIND: &str = "名號立牌";

/// 產生居民為某位玩家刻的名號榮譽牌牌面文字（如「此地常客·造物者」）。
///
/// 走與玩家立牌、居民立牌命名同一套 [`sanitize_text`] 清洗（去控制字元、截 `SIGN_MAX_CHARS`）。
/// **只嵌名號本身**（`role.epithet()`）、不嵌玩家顯示名——牌面短而莊重、也免去玩家名清洗顧慮；
/// 玩家從招呼／動態自然知道這塊牌講的是自己。空名號不可能（enum 窮舉），故恆非空。
pub fn honor_sign_text(role: PlayerRole) -> String {
    sanitize_text(&format!("{HONOR_PREFIX}{}", role.epithet()))
}

/// 判斷某塊既有牌面文字**是否**一塊名號榮譽牌（去重掃描用）。
///
/// 以固定前綴辨識即可——呼叫端只需知道「這位居民家旁是否已立過名號牌」就不再重立，
/// 不必逐一比對是給哪個名號（同一位居民對同一位玩家至多一塊，語意上一塊就夠）。
pub fn is_honor_sign(text: &str) -> bool {
    text.starts_with(HONOR_PREFIX)
}

/// 城鎮動態牆旁白（第三人稱、含居民名＋玩家名＋名號）。空玩家名時退成泛稱、不露破碎字串。
pub fn honor_feed_line(resident_name: &str, player_name: &str, role: PlayerRole) -> String {
    let e = role.epithet();
    if player_name.is_empty() {
        format!("{resident_name} 在自家門旁刻了一塊牌，把這位旅人記作這一帶的「{e}」")
    } else {
        format!("{resident_name} 在自家門旁刻了一塊牌，把 {player_name} 記作這一帶的「{e}」")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn honor_text_carries_epithet_and_prefix() {
        let t = honor_sign_text(PlayerRole::Maker);
        assert_eq!(t, "此地常客·造物者");
        assert!(t.starts_with(HONOR_PREFIX));
        assert!(is_honor_sign(&t));
    }

    #[test]
    fn honor_text_for_every_role_is_nonempty_and_recognized() {
        for role in [
            PlayerRole::Maker,
            PlayerRole::Giver,
            PlayerRole::Trader,
            PlayerRole::Companion,
        ] {
            let t = honor_sign_text(role);
            assert!(!t.is_empty());
            assert!(is_honor_sign(&t), "{t} 應被辨識為名號牌");
            // 牌面確實含該名號字樣。
            assert!(t.contains(role.epithet()));
        }
    }

    #[test]
    fn is_honor_sign_rejects_ordinary_signs() {
        // 749 居民立牌命名、玩家自寫的牌都不該被誤判成名號牌。
        assert!(!is_honor_sign("露娜的家"));
        assert!(!is_honor_sign("往礦坑↓"));
        assert!(!is_honor_sign(""));
        assert!(!is_honor_sign("常客")); // 少了前綴的分隔點，不誤判
    }

    #[test]
    fn honor_feed_embeds_name_role_and_handles_empty() {
        let f = honor_feed_line("露娜", "阿海", PlayerRole::Trader);
        assert!(f.contains("露娜"));
        assert!(f.contains("阿海"));
        assert!(f.contains("老搭檔"));
        // 空玩家名安全退成泛稱、不露破碎字串。
        let f2 = honor_feed_line("露娜", "", PlayerRole::Trader);
        assert!(f2.contains("露娜"));
        assert!(f2.contains("老搭檔"));
        assert!(!f2.contains("記作這一帶的「老搭檔」的"));
    }
}
