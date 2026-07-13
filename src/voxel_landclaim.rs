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
//! **做法**：重用既有告示牌歸屬機制（830）＋既有語氣分類（`voxel_readsign::classify`
//! 判斷牌面是不是「家」）——你在自家門前立一塊寫著「家」的牌，那塊牌方圓 [`CLAIM_RADIUS`]
//! 格內就成了你的領地：只有你自己能挖或放置，其他人（含訪客）一律被溫柔擋下，浮出提示
//! 告訴他這是誰的家。
//!
//! **與既有元素的分界**：與世界奇觀保護（940 `wonder_protected`）——那是**全世界唯一一株**
//! 天然奇觀對**所有人**（含奇觀所有者，因為它沒有主人）一視同仁地禁止破壞；本刀是**任何玩家**
//! 都能建立的**私有**領地，只擋外人、不擋自己。與居民認得你的家（830）——那是**居民**的行為
//! （認地方、登門拜訪，靠顯示名認人）；本刀是**伺服器規則**（誰能動這塊地，靠穩定帳號鍵判
//! 歸屬），兩者共用同一塊牌，但歸屬來源不同（見下段 review 修正）。
//!
//! **成本紀律**：零 LLM（純規則判定＋確定性選句）、零 migration（`SignEntry` 新欄位皆
//! `#[serde(default)]` 向後相容）、零新美術。
//!
//! **濫用防護**：本刀是**收斂**既有濫用面，不是新開一個——今日任何人都能拆光別人蓋的家，
//! 本刀之後只有立牌本人能動這塊地；領地判定純由伺服器內部資料算出（歸屬鍵由後端 cookie
//! 權威解出的帳號 email，身分不可能被客戶端偽造），玩家無從偽造他人身分騙過保護、也無法
//! 藉此鎖住別人未立牌的公共空間（沒有「家」牌就沒有領地，行為與今日一致）。
//!
//! **review 修正（PR #1249 第二輪，阻擋項 1~3）**：初版直接拿 `SignEntry.owner`（**可被
//! 改寫的顯示名**）當歸屬鍵，有三個洞——①`SignSet` 沒驗證就能覆寫既有牌的 owner，等於
//! 一句話搶走整塊領地；②只取「最近一塊牌」判歸屬，相鄰立牌能切走別人領地一角；③顯示名
//! 可透過改名功能變動，改名後會被自己的領地擋在外面、同名帳號也能互開領地。修法：
//! `SignEntry` 新增 `owner_key`（帳號 email，改名不變、不可偽造）專門用於權限判定，
//! `owner`（顯示名）只留給居民辨識／提示句用；`SignSet` 落地前與 `Break`/`Place` 共用
//! 同一顆 [`resolve_claim_block`]（掃描半徑內**所有**家牌而非只取最近）判斷歸屬。
//!
//! **review 修正（PR #1249 第三輪）**：上述修好後浮出新洞——「無上限插旗」，任何人走進別人
//! 蓋好但沒立牌的房子插一塊「家」牌，就能把整棟連同方圓 [`CLAIM_RADIUS`] 格搶成自己的領地，
//! 且沒有解除路徑。修法：`SignStore::demote_other_claims`——立新家牌時，若該帳號在別的
//! 座標已有一塊有主的家牌，舊的自動失效（牌面文字仍留著，只是不再算誰的、也不再受保護）。
//! 每帳號僅一塊有效領地，把單一惡意帳號的破壞面上界壓到「一個半徑 [`CLAIM_RADIUS`] 的圈」，
//! 搶佔代價從「無限插旗」降到「賭上自己唯一的家」。「別人蓋過但沒立牌的地能被搶」這個更難
//! 的問題留待下一刀（v1 可接受，見 review 意見）。
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
/// 永遠不保護，行為與今日完全一致；有主的領地只有**同一把穩定歸屬鍵**（帳號 email，不是
/// 可改的顯示名）本人能動，其他任何人（含訪客，`requester=None`）一律擋下。
pub fn dig_denied(owner: Option<&str>, requester: Option<&str>) -> bool {
    match owner {
        None => false,
        Some(o) => requester != Some(o),
    }
}

/// review 修正（PR #1249 第二輪）：目標格該不該被別人的領地擋下——掃過範圍內**所有**
/// 「家」牌（不只最近一塊），只要其中一塊是我自己的領地、整格立即放行；否則回傳離我
/// 最近那塊別人領地主人的顯示名（供組提示句）供呼叫端擋下。
///
/// 修的是「只取最近一塊牌」的漏洞：有人在你家 7 格外立牌，你家靠那側的方塊會被判給
/// 陌生人的牌（因為離陌生人的牌比較近）——你修不了自己的牆，也拆不掉他的牌。改成「只要
/// 範圍內有一塊是我的、就放行」對領主友善，兩塊領地重疊時邊界重疊區塊誰的都能動，但誰都
/// 拿不走對方領地的核心。
///
/// `home_claims` 必須是**已按距離由近到遠排序**、且已過濾只剩「家」語氣的牌清單，
/// 每項為 `(領地主人顯示名, 領地主人的穩定歸屬鍵)`；`owner 鍵=None`（舊資料/訪客立的牌）
/// 一律跳過、不算保護，與今日行為一致。
pub fn resolve_claim_block<'a, I>(home_claims: I, requester: Option<&str>) -> Option<&'a str>
where
    I: IntoIterator<Item = (&'a str, Option<&'a str>)>,
{
    let mut nearest_other: Option<&str> = None;
    for (display, owner_key) in home_claims {
        if !dig_denied(owner_key, requester) {
            if owner_key.is_some() {
                // 範圍內找到一塊是我自己的領地：其餘全部忽略，直接放行。
                return None;
            }
            continue; // 無主，跳過繼續看其他牌
        }
        if nearest_other.is_none() {
            nearest_other = Some(display);
        }
    }
    nearest_other
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

    // ── resolve_claim_block（review 修正：掃全部牌，不只取最近一塊）─────────────────────

    #[test]
    fn resolve_claim_block_empty_range_allows() {
        assert_eq!(resolve_claim_block(vec![], Some("a@example.com")), None);
    }

    #[test]
    fn resolve_claim_block_no_owner_never_blocks() {
        // 範圍內有牌但都無主（舊資料/訪客立的牌）——不算保護，行為與今日一致。
        let claims = vec![("阿星", None)];
        assert_eq!(resolve_claim_block(claims.clone(), Some("stranger@example.com")), None);
        assert_eq!(resolve_claim_block(claims, None), None);
    }

    #[test]
    fn resolve_claim_block_denies_stranger() {
        let claims = vec![("阿星", Some("astar@example.com"))];
        assert_eq!(
            resolve_claim_block(claims.clone(), Some("stranger@example.com")),
            Some("阿星")
        );
        // 訪客（無帳號）一律擋。
        assert_eq!(resolve_claim_block(claims, None), Some("阿星"));
    }

    #[test]
    fn resolve_claim_block_allows_owner_even_if_not_nearest() {
        // 修的正是這個場景：最近那塊是陌生人的牌，但範圍內另一塊是我自己的領地——
        // 只要有一塊是我的，整格就該放行，不能只看「最近」那塊。
        let claims = vec![
            ("陌生人", Some("stranger@example.com")), // 較近
            ("阿星", Some("astar@example.com")),       // 較遠，但這是我自己的
        ];
        assert_eq!(resolve_claim_block(claims, Some("astar@example.com")), None);
    }

    #[test]
    fn resolve_claim_block_blocks_with_nearest_other_when_none_are_mine() {
        // 兩塊都不是我的：擋下，回傳「較近」那塊（清單已按距離排序，取第一個命中）。
        let claims = vec![
            ("陌生人A", Some("a@example.com")),
            ("陌生人B", Some("b@example.com")),
        ];
        assert_eq!(
            resolve_claim_block(claims, Some("stranger@example.com")),
            Some("陌生人A")
        );
    }

    #[test]
    fn resolve_claim_block_rename_proof() {
        // 顯示名可以改，但歸屬鍵（email）不變——即使呼叫端傳進來的顯示名剛好撞名，
        // 只要 email 對得上就放行；email 對不上，顯示名再像也擋。
        let claims = vec![("阿星", Some("astar@example.com"))];
        // 帳號改名成「阿星」的另一人（email 不同）——擋。
        assert_eq!(
            resolve_claim_block(claims.clone(), Some("imposter@example.com")),
            Some("阿星")
        );
        // 阿星本人改了顯示名，但 email 沒變——仍放行（呼叫端傳的 requester 一律是 email）。
        assert_eq!(resolve_claim_block(claims, Some("astar@example.com")), None);
    }
}
