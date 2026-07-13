//! 乙太方界·玩家個人領地保護 v1（自主提案切片）。
//!
//! **真缺口 / 為誰做**：玩家立牌署名「XX的家」後，居民認得出這是你的家、會登門拜訪你
//! （830 `voxel_player_home`）——但這塊「家」在世界規則裡毫無份量：**任何路過的其他玩家
//! 都能一鎬子把你辛苦蓋的家拆光**，voxel 世界至今沒有任何一種「這是我的地盤，別人動不得」
//! 的保障。與此同時，居民↔居民（723）、玩家↔玩家（832 `voxel_stall`）都已經有成熟的
//! 互動系統，玩家彼此之間卻唯獨在「保護自己蓋的東西」這件事上毫無防線——這是多人共居世界
//! 最基本、也最容易被忽略的一塊（PR #1248 眾力共築讓玩家第一次「一起蓋」，本刀讓玩家第一次
//! 「安心地自己蓋」，兩者互補：一個是共築的信任，一個是個人領地的保障）。
//!
//! **做法**：重用既有告示牌歸屬機制（830，`SignEntry.owner` 已由伺服器權威記下是哪個已登入
//! 帳號立的牌）＋既有語氣分類（`voxel_readsign::classify` 判斷牌面是不是「家」）——你在自家
//! 門前立一塊寫著「家」的牌，那塊牌方圓 [`CLAIM_RADIUS`] 格內就成了你的領地：只有你自己能挖
//! 或放置，其他人（含訪客）一律被溫柔擋下，浮出提示告訴他這是誰的家。
//!
//! **與既有元素的分界**：與世界奇觀保護（940 `wonder_protected`）——那是**全世界唯一一株**
//! 天然奇觀對**所有人**（含奇觀所有者，因為它沒有主人）一視同仁地禁止破壞；本刀是**任何玩家**
//! 都能建立的**私有**領地，只擋外人、不擋自己。與居民認得你的家（830）——那是**居民**的行為
//! （認地方、登門拜訪）；本刀是**伺服器規則**（誰能動這塊地），兩者共用同一塊牌、同一份
//! `owner`，但服務對象不同（居民 vs 世界規則）。
//!
//! **成本紀律**：零 LLM（純規則判定＋確定性選句）、零新協議欄位（沿用既有 `SignEntry.owner`）、
//! 零 migration（告示牌本已持久化，本刀不新增任何欄位）、零新美術。
//!
//! **濫用防護**：本刀是**收斂**既有濫用面，不是新開一個——今日任何人都能拆光別人蓋的家，
//! 本刀之後只有登入帳號立牌的人自己能動這塊地；領地判定純由伺服器內部資料算出（牌子
//! `owner` 早已由後端 cookie 權威寫入、身分不可能被客戶端偽造），玩家無從偽造他人身分
//! 騙過保護、也無法藉此鎖住別人未立牌的公共空間（沒有「家」牌就沒有領地，行為與今日一致）。
//!
//! **純邏輯層**：零 async、零鎖、零 IO；確定性純函式，窮舉可測。鎖 / 距離掃描 / 廣播在
//! `voxel_ws.rs`（短鎖即釋、不巢狀，守死鎖鐵律）。

/// 領地保護半徑（世界座標，方塊，XZ 平面）：立牌方圓這麼近都算「你的地盤」。
/// 比居民認家的「在家」判定（[`crate::voxel_player_home::PLAYER_HOME_DIST`] = 5.0）
/// 稍大一點，讓保護範圍蓋住整間小屋、不只是牌子本身那一格。
pub const CLAIM_RADIUS: f32 = 6.0;

/// 這塊牌面是不是「家」語氣——只有這種牌才會圈出領地（隨手寫的路標/留言不算，
/// 沿用既有 [`crate::voxel_readsign::classify`] 分類，不重造一套規則）。
pub fn is_home_sign(text: &str) -> bool {
    matches!(crate::voxel_readsign::classify(text), crate::voxel_readsign::SignTone::Home)
}

/// 這一鎬（或這一放）該不該被擋下：領地無主（`owner=None`，訪客立的牌或今日以前的舊牌）
/// 永遠不保護，行為與今日完全一致；有主的領地只有**同一個已登入帳號**本人能動，
/// 其他任何人（含訪客，`requester=None`）一律擋下。
pub fn dig_denied(owner: Option<&str>, requester: Option<&str>) -> bool {
    match owner {
        None => false,
        Some(o) => requester != Some(o),
    }
}

/// 被領地擋下時浮出的提示（確定性選句，由呼叫端傳 `pick` 索引；面向玩家字串，i18n 友善）。
const DENY_LINES: [&str; 3] = [
    "這裡是 {owner} 的家，別人的鎬子伸不進來喔。",
    "{owner} 在這片地上蓋了家，還是繞道去別的地方挖吧。",
    "這一帶已經有主人了——{owner} 的領地，只有本人能動這裡。",
];

/// 依 `pick` 索引挑一句提示，把 `{owner}` 換成領地主人的名字。
pub fn claim_deny_line(owner: &str, pick: usize) -> String {
    DENY_LINES[pick % DENY_LINES.len()].replace("{owner}", owner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_owner_never_denied() {
        assert!(!dig_denied(None, None));
        assert!(!dig_denied(None, Some("露娜")));
    }

    #[test]
    fn owner_can_dig_own_claim() {
        assert!(!dig_denied(Some("阿星"), Some("阿星")));
    }

    #[test]
    fn other_player_denied() {
        assert!(dig_denied(Some("阿星"), Some("小夜")));
    }

    #[test]
    fn guest_denied_on_owned_claim() {
        assert!(dig_denied(Some("阿星"), None));
    }

    #[test]
    fn is_home_sign_matches_home_tone_only() {
        assert!(is_home_sign("阿星的家"));
        assert!(!is_home_sign("往礦坑↓"));
        assert!(!is_home_sign("今天天氣真好"));
    }

    #[test]
    fn claim_deny_line_non_empty_contains_owner_and_wraps() {
        for pick in 0..8 {
            let line = claim_deny_line("阿星", pick);
            assert!(!line.is_empty());
            assert!(line.contains("阿星"));
        }
        assert_eq!(claim_deny_line("阿星", 0), claim_deny_line("阿星", DENY_LINES.len()));
    }

    #[test]
    fn claim_deny_line_no_leftover_placeholder() {
        for pick in 0..DENY_LINES.len() {
            assert!(!claim_deny_line("阿星", pick).contains("{owner}"));
        }
    }
}
