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
//! **v2（自主提案切片，ROADMAP 966）：領地信任名單／共享**——review 在 PR #1250 第二輪點名
//! 「963 自己列的活口，最有感的一項」：領地保護目前是全有或全無，連你邀來同住的朋友都被自己
//! 的地盤擋在箱子外、鋤頭外。本刀補上 [`TrustStore`]——站到朋友身邊，把他加進你這帳號的信任
//! 名單，之後他就跟你自己一樣不被 [`resolve_claim_block`] 擋下（能開你家箱子、動你家的地）；
//! 再對同一人做一次即解除信任。信任名單以**你這帳號**（`owner_key`）為鍵——因每帳號僅一塊
//! 有效領地（`demote_other_claims`），不必先立好家牌才能設定，之後不管你把家搬到哪塊新牌，
//! 信任名單原封不動跟著你的帳號走。`Break`/`Place`/`SignSet`/`ChestPut`/…… 全部九個入口共用
//! 同一顆 `claim_blocking_owner`，故信任一經接上就自動套用全部入口，不必逐一改呼叫端。
//! 持久化走 append-only JSONL（比照 [`crate::voxel_sign::SignStore`] 範式），重啟 replay 取每
//! (owner_key, trusted_key) 最新一筆。
//!
//! **純邏輯層**：零 async、零鎖、零 IO；確定性純函式，窮舉可測。鎖 / 距離掃描 / 廣播在
//! `voxel_ws.rs`（短鎖即釋、不巢狀，守死鎖鐵律）。

use std::{
    collections::{HashMap, HashSet},
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
};

use serde::{Deserialize, Serialize};

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
/// 永遠不保護，行為與今日完全一致；有主的領地本人、或被領地主人加進信任名單的人
/// （`trusted=true`，見 [`TrustStore`]）都能動，其他任何人（含訪客，`requester=None`）
/// 一律擋下。
pub fn dig_denied(owner: Option<&str>, requester: Option<&str>, trusted: bool) -> bool {
    match owner {
        None => false,
        Some(o) => requester != Some(o) && !trusted,
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
/// `home_claims` 必須是**已按距離由近到遠排序**、且已過濾只剩「家」語氣的牌清單，每項為
/// `(領地主人顯示名, 領地主人的穩定歸屬鍵, 我是否被這塊領地的主人信任)`（信任名單 v2，
/// ROADMAP 966——呼叫端在 `voxel_ws.rs` 查 [`TrustStore`] 算出每塊牌各自的信任結果，本函式
/// 純比對不碰鎖）；`owner 鍵=None`（舊資料/訪客立的牌）一律跳過、不算保護，與今日行為一致。
/// 信任只放行**那一塊**牌、不是全域捷徑——被 A 信任不代表能動範圍內 B 的地，只有「本人」
/// 才享有「其餘全部忽略」的全域放行。
pub fn resolve_claim_block<'a, I>(home_claims: I, requester: Option<&str>) -> Option<&'a str>
where
    I: IntoIterator<Item = (&'a str, Option<&'a str>, bool)>,
{
    let mut nearest_other: Option<&str> = None;
    for (display, owner_key, trusted) in home_claims {
        if owner_key.is_some() && requester == owner_key {
            // 範圍內找到一塊是我自己的領地：其餘全部忽略，直接放行。
            return None;
        }
        if !dig_denied(owner_key, requester, trusted) {
            // 無主、或被這塊牌的主人信任——這一塊放行，但只對這一塊，繼續看範圍內其他牌
            // （信任是逐塊判定，不像「本人」那樣是全域捷徑：被 A 信任不代表能動 B 的地）。
            continue;
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

// ── 領地信任名單 v2（ROADMAP 966，自主提案切片）───────────────────────────────────────

/// 站在朋友身邊、按 T 邀請信任時，對方需在這個水平距離內（世界座標，方塊，XZ 平面）——
/// 比照 [`crate::voxel_gift::GIFT_REACH`]，需要走近才能互相信任，避免隔空亂點名。
pub const TRUST_REACH: f32 = 5.0;

/// 持久化路徑（append-only JSONL，比照 [`crate::voxel_sign::SIGN_PATH`] 範式）。
pub const TRUST_PATH: &str = "data/voxel_landclaim_trust.jsonl";

/// 一筆信任名單寫入事件。`trusted=true` 加入信任、`false` 解除信任；replay 時取每
/// `(owner_key, trusted_key)` 最新一筆為現況。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TrustEntry {
    /// 領地主人的穩定歸屬鍵（帳號 email）。
    pub owner_key: String,
    /// 被信任者的穩定歸屬鍵（帳號 email）。
    pub trusted_key: String,
    /// true=加入信任、false=解除信任。
    pub trusted: bool,
    /// 單調遞增序號。
    pub seq: u64,
}

/// 全局信任名單 store：owner_key（帳號 email）→ 被他信任的帳號 email 集合。
#[derive(Default)]
pub struct TrustStore {
    trusted: HashMap<String, HashSet<String>>,
    next_seq: u64,
}

impl TrustStore {
    /// 空 store（測試 / 首次啟動）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 由載入的歷史事件重建 store（重啟後從 JSONL replay，每 (owner_key, trusted_key) 取最新 seq）。
    pub fn from_entries(entries: Vec<TrustEntry>) -> Self {
        let mut latest: HashMap<(String, String), &TrustEntry> = HashMap::new();
        let mut max_seq = 0u64;
        for e in &entries {
            max_seq = max_seq.max(e.seq);
            let key = (e.owner_key.clone(), e.trusted_key.clone());
            match latest.get(&key) {
                Some(prev) if prev.seq >= e.seq => {}
                _ => { latest.insert(key, e); }
            }
        }
        let mut trusted: HashMap<String, HashSet<String>> = HashMap::new();
        for ((owner_key, trusted_key), e) in latest {
            if e.trusted {
                trusted.entry(owner_key).or_default().insert(trusted_key);
            }
        }
        Self { trusted, next_seq: max_seq.saturating_add(1) }
    }

    /// `requester_key` 是否被 `owner_key` 這帳號信任（供 `voxel_ws.rs` 組 `resolve_claim_block`
    /// 的呼叫端輸入）。
    pub fn is_trusted(&self, owner_key: &str, requester_key: &str) -> bool {
        self.trusted.get(owner_key).is_some_and(|set| set.contains(requester_key))
    }

    /// 切換信任狀態：已信任 → 解除；未信任 → 加入。回傳 (切換後是否信任, 供 append 的事件)。
    pub fn toggle(&mut self, owner_key: &str, trusted_key: &str) -> (bool, TrustEntry) {
        let set = self.trusted.entry(owner_key.to_string()).or_default();
        let now_trusted = if set.remove(trusted_key) {
            false
        } else {
            set.insert(trusted_key.to_string());
            true
        };
        let seq = self.next_seq;
        self.next_seq += 1;
        (now_trusted, TrustEntry {
            owner_key: owner_key.to_string(),
            trusted_key: trusted_key.to_string(),
            trusted: now_trusted,
            seq,
        })
    }
}

/// 從磁碟載入所有信任事件（啟動時呼叫一次）。
pub fn load_trust() -> Vec<TrustEntry> {
    let Ok(f) = fs::File::open(TRUST_PATH) else { return vec![]; };
    BufReader::new(f)
        .lines()
        .map_while(Result::ok)
        .filter_map(|l| serde_json::from_str::<TrustEntry>(&l).ok())
        .collect()
}

/// Append 單筆事件。
pub fn append_trust(entry: &TrustEntry) {
    let Ok(line) = serde_json::to_string(entry) else { return; };
    let Ok(mut f) = OpenOptions::new().create(true).append(true).open(TRUST_PATH) else { return; };
    let _ = writeln!(f, "{line}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_owner_never_denied() {
        assert!(!dig_denied(None, None, false));
        assert!(!dig_denied(None, Some("露娜"), false));
    }

    #[test]
    fn owner_can_dig_own_claim() {
        assert!(!dig_denied(Some("阿星"), Some("阿星"), false));
    }

    #[test]
    fn other_player_denied() {
        assert!(dig_denied(Some("阿星"), Some("小夜"), false));
    }

    #[test]
    fn guest_denied_on_owned_claim() {
        assert!(dig_denied(Some("阿星"), None, false));
    }

    #[test]
    fn trusted_other_player_allowed() {
        // 信任名單 v2（ROADMAP 966）：trusted=true 時，即使不是本人也放行。
        assert!(!dig_denied(Some("阿星"), Some("小夜"), true));
    }

    #[test]
    fn owner_ignores_trusted_flag() {
        // 本人一律放行，trusted 參數不影響（本人不需要「被信任」）。
        assert!(!dig_denied(Some("阿星"), Some("阿星"), true));
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
        let claims = vec![("阿星", None, false)];
        assert_eq!(resolve_claim_block(claims.clone(), Some("stranger@example.com")), None);
        assert_eq!(resolve_claim_block(claims, None), None);
    }

    #[test]
    fn resolve_claim_block_denies_stranger() {
        let claims = vec![("阿星", Some("astar@example.com"), false)];
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
            ("陌生人", Some("stranger@example.com"), false), // 較近
            ("阿星", Some("astar@example.com"), false),       // 較遠，但這是我自己的
        ];
        assert_eq!(resolve_claim_block(claims, Some("astar@example.com")), None);
    }

    #[test]
    fn resolve_claim_block_blocks_with_nearest_other_when_none_are_mine() {
        // 兩塊都不是我的：擋下，回傳「較近」那塊（清單已按距離排序，取第一個命中）。
        let claims = vec![
            ("陌生人A", Some("a@example.com"), false),
            ("陌生人B", Some("b@example.com"), false),
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
        let claims = vec![("阿星", Some("astar@example.com"), false)];
        // 帳號改名成「阿星」的另一人（email 不同）——擋。
        assert_eq!(
            resolve_claim_block(claims.clone(), Some("imposter@example.com")),
            Some("阿星")
        );
        // 阿星本人改了顯示名，但 email 沒變——仍放行（呼叫端傳的 requester 一律是 email）。
        assert_eq!(resolve_claim_block(claims, Some("astar@example.com")), None);
    }

    // ── resolve_claim_block × 信任名單 v2（ROADMAP 966）────────────────────────────────

    #[test]
    fn resolve_claim_block_allows_trusted_stranger() {
        // 陌生人的領地，但這塊牌的主人把我加進了信任名單（呼叫端算好帶進來的 bool）——放行。
        let claims = vec![("阿星", Some("astar@example.com"), true)];
        assert_eq!(resolve_claim_block(claims, Some("friend@example.com")), None);
    }

    #[test]
    fn resolve_claim_block_untrusted_still_denied() {
        // 沒被信任的陌生人仍照舊擋下。
        let claims = vec![("阿星", Some("astar@example.com"), false)];
        assert_eq!(
            resolve_claim_block(claims, Some("stranger@example.com")),
            Some("阿星")
        );
    }

    #[test]
    fn resolve_claim_block_trust_is_per_claim_not_global() {
        // 我被 A 信任、但沒被 B 信任——兩塊都不是我的領地，B 那塊仍該擋下（trusted 逐項帶入，
        // 不是「只要有一塊信任我就全放行」；owner 本人放行才是全域捷徑，信任不是）。
        let claims = vec![
            ("A", Some("a@example.com"), true),
            ("B", Some("b@example.com"), false),
        ];
        assert_eq!(resolve_claim_block(claims, Some("friend@example.com")), Some("B"));
    }

    // ── TrustStore（信任名單 v2，ROADMAP 966）──────────────────────────────────────────

    #[test]
    fn trust_store_toggle_adds_then_removes() {
        let mut store = TrustStore::new();
        assert!(!store.is_trusted("astar@example.com", "friend@example.com"));
        let (now_trusted, ev) = store.toggle("astar@example.com", "friend@example.com");
        assert!(now_trusted);
        assert!(ev.trusted);
        assert!(store.is_trusted("astar@example.com", "friend@example.com"));
        // 再切一次：解除信任。
        let (now_trusted, ev) = store.toggle("astar@example.com", "friend@example.com");
        assert!(!now_trusted);
        assert!(!ev.trusted);
        assert!(!store.is_trusted("astar@example.com", "friend@example.com"));
    }

    #[test]
    fn trust_store_is_scoped_per_owner() {
        let mut store = TrustStore::new();
        store.toggle("astar@example.com", "friend@example.com");
        // 我信任了 friend，不代表 friend 也信任我，也不代表別的領地主人信任 friend。
        assert!(!store.is_trusted("friend@example.com", "astar@example.com"));
        assert!(!store.is_trusted("someoneelse@example.com", "friend@example.com"));
    }

    #[test]
    fn trust_store_from_entries_takes_latest_per_pair() {
        let entries = vec![
            TrustEntry { owner_key: "a@example.com".into(), trusted_key: "b@example.com".into(), trusted: true, seq: 0 },
            TrustEntry { owner_key: "a@example.com".into(), trusted_key: "b@example.com".into(), trusted: false, seq: 1 },
        ];
        let store = TrustStore::from_entries(entries);
        assert!(!store.is_trusted("a@example.com", "b@example.com"), "最新一筆是解除，應恢復未信任");
    }

    #[test]
    fn trust_store_from_entries_out_of_order_seq_takes_max() {
        let entries = vec![
            TrustEntry { owner_key: "a@example.com".into(), trusted_key: "b@example.com".into(), trusted: false, seq: 5 },
            TrustEntry { owner_key: "a@example.com".into(), trusted_key: "b@example.com".into(), trusted: true, seq: 2 },
        ];
        let store = TrustStore::from_entries(entries);
        assert!(!store.is_trusted("a@example.com", "b@example.com"), "應取 seq 較大的那筆（解除）");
    }
}
