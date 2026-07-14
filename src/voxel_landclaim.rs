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
//! **review 修正（PR #1252 第一輪，阻擋項 1、2）**：v2 初版用同一顆「附近同名玩家」查找
//! 同時做加入與撤銷，浮出兩個洞——①**撤銷綁距離／在線**：信任錯人後只要對方不再靠近，
//! 領主永遠撤不掉，他能保有你家箱子存取權、趁你離線回來搬空；②**用顯示名 `find` 挑目標**：
//! 附近有兩位同名已登入玩家時挑到誰不保證，攻擊者改名成你朋友的名字站你旁邊，你按 T 就可能
//! 把箱子與地的完整寫入權發給他。修法：[`TrustStore::add`]／[`TrustStore::remove`] 拆開，
//! 撤銷只查自己既有的信任名單（[`TrustStore::find_by_name`]）、不再要求對方在線／在附近；
//! [`resolve_trust_target`] 統一判斷「該加入還是撤銷」，只要同名（附近或名單裡）不只一位就
//! 一律拒絕，寧可失敗也別默默信任錯人。新增 [`TrustStore::list`] 供信任名單查詢（`voxel_ws.rs`
//! 的 `ClaimTrustList`），讓玩家能確認自己到底信任了誰。
//!
//! **v4（自主提案切片，ROADMAP 967）：立牌／拆牌不吃信任通行證**——review 在 PR #1252 done
//! 訊息點名 v2 接上的信任是不分場合的全通行證，浮出兩個洞：①被信任者能拆掉/`SignSet` 覆寫
//! 你的「家」牌本身，等於親手解散你的領地；②被信任者能就地立一塊自己的新家牌，靠**自己的**
//! 領地保有重疊區寫入權，撤銷信任後仍搬得動、拆得動。兩者同源：立牌／拆牌這個動作不該吃信任
//! 通行證。修法在 `voxel_ws.rs`——`claim_blocking_owner`/`deny_if_claimed` 新增 `allow_trust`
//! 參數，`SignSet` 與「`Break` 目標是 `Block::Sign`」兩處呼叫端傳 `false`（信任一律不算數，
//! 只有本人放行），其餘七個入口維持 `true` 不動。信任朋友依然能開你家箱子、幫你鋤地種田，
//! 動不了你的「家」牌本身。
//!
//! **純邏輯層**：零 async、零鎖、零 IO；確定性純函式，窮舉可測。鎖 / 距離掃描 / 廣播在
//! `voxel_ws.rs`（短鎖即釋、不巢狀，守死鎖鐵律）。

use std::{
    collections::HashMap,
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

/// 依帳號歸屬鍵找這位玩家目前登記的家牌世界座標（邀居同住 v1，ROADMAP 972）：靠「每帳號
/// 僅一塊有效領地」的既有不變量（[`crate::voxel_sign::SignStore::demote_other_claims`]，舊牌
/// 會被清空 `owner_key`）保證這裡天然只會命中最新那一塊，不必額外去重。呼叫端傳入
/// [`crate::voxel_sign::SignStore::all_hits`] 的全量掃描結果（不限距離——玩家邀居時人可能
/// 站在居民旁邊，不是站在自己家門口）；純函式、零 IO。找不到已登記的家牌 → `None`。
pub fn find_owner_home<'a, I>(hits: I, owner_key: &str) -> Option<(f32, f32)>
where
    I: IntoIterator<Item = &'a crate::voxel_sign::SignHit>,
{
    hits.into_iter()
        .find(|h| h.owner_key.as_deref() == Some(owner_key) && is_home_sign(&h.text))
        .map(|h| (h.cx, h.cz))
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
/// **只在加入信任時要求**；解除信任查自己的信任名單即可執行，不需要對方在線／在附近
/// （review PR #1252 阻擋項 1：信任錯人後，只要對方不再靠近，領主永遠撤不掉，他會一路
/// 保有你家箱子的存取權）。
pub const TRUST_REACH: f32 = 5.0;

/// 持久化路徑（append-only JSONL，比照 [`crate::voxel_sign::SIGN_PATH`] 範式）。
pub const TRUST_PATH: &str = "data/voxel_landclaim_trust.jsonl";

/// 一筆信任名單寫入事件。`trusted=true` 加入信任、`false` 解除信任；replay 時取每
/// `(owner_key, trusted_key)` 最新一筆為現況。`trusted_name` 是操作當下對方的顯示名，
/// 只供人類可讀的提示／查詢與「打名字撤銷」比對用——**權限判定全程只看
/// `owner_key`/`trusted_key`（帳號 email），與顯示名無關**，改名或撞名不影響誰能開誰家的箱子。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TrustEntry {
    /// 領地主人的穩定歸屬鍵（帳號 email）。
    pub owner_key: String,
    /// 被信任者的穩定歸屬鍵（帳號 email）。
    pub trusted_key: String,
    /// 被信任者當下的顯示名（供查詢／提示用，可能隨改名而過期，不影響權限判定）。
    #[serde(default)]
    pub trusted_name: String,
    /// true=加入信任、false=解除信任。
    pub trusted: bool,
    /// 單調遞增序號。
    pub seq: u64,
}

/// 信任名單裡的一筆現有關係（供 [`TrustStore::find_by_name`]／[`TrustStore::list`] 回傳）。
pub struct TrustedFriend {
    pub trusted_key: String,
    pub trusted_name: String,
}

/// 全局信任名單 store：owner_key（帳號 email）→ 被他信任的帳號 email → 對方顯示名。
#[derive(Default)]
pub struct TrustStore {
    trusted: HashMap<String, HashMap<String, String>>,
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
        let mut trusted: HashMap<String, HashMap<String, String>> = HashMap::new();
        for ((owner_key, trusted_key), e) in latest {
            if e.trusted {
                trusted.entry(owner_key).or_default().insert(trusted_key, e.trusted_name.clone());
            }
        }
        Self { trusted, next_seq: max_seq.saturating_add(1) }
    }

    /// `requester_key` 是否被 `owner_key` 這帳號信任（供 `voxel_ws.rs` 組 `resolve_claim_block`
    /// 的呼叫端輸入）。
    pub fn is_trusted(&self, owner_key: &str, requester_key: &str) -> bool {
        self.trusted.get(owner_key).is_some_and(|set| set.contains_key(requester_key))
    }

    /// 加入信任（呼叫端已驗證對方在附近且在線、且不是重複請求）。已信任則不重複寫入，
    /// 回傳 `None`（呼叫端當成功處理即可，冪等）。
    pub fn add(&mut self, owner_key: &str, trusted_key: &str, trusted_name: &str) -> Option<TrustEntry> {
        let set = self.trusted.entry(owner_key.to_string()).or_default();
        if set.contains_key(trusted_key) {
            return None;
        }
        set.insert(trusted_key.to_string(), trusted_name.to_string());
        Some(self.next_entry(owner_key, trusted_key, trusted_name, true))
    }

    /// 解除信任——只需要 `owner_key` 與 `trusted_key`，**不要求對方在線／在附近**（見上方
    /// [`TRUST_REACH`] 註解）。未信任則回傳 `None`。
    pub fn remove(&mut self, owner_key: &str, trusted_key: &str) -> Option<TrustEntry> {
        let name = self.trusted.get_mut(owner_key)?.remove(trusted_key)?;
        Some(self.next_entry(owner_key, trusted_key, &name, false))
    }

    fn next_entry(&mut self, owner_key: &str, trusted_key: &str, trusted_name: &str, trusted: bool) -> TrustEntry {
        let seq = self.next_seq;
        self.next_seq += 1;
        TrustEntry {
            owner_key: owner_key.to_string(),
            trusted_key: trusted_key.to_string(),
            trusted_name: trusted_name.to_string(),
            trusted,
            seq,
        }
    }

    /// `owner_key` 這帳號的信任名單裡，顯示名等於 `name` 的所有人——可能因改名或重名而
    /// 不只一位，呼叫端（[`resolve_trust_target`]）據此判斷能否無歧義地直接撤銷。
    pub fn find_by_name(&self, owner_key: &str, name: &str) -> Vec<TrustedFriend> {
        self.trusted.get(owner_key).into_iter().flatten()
            .filter(|(_, n)| n.as_str() == name)
            .map(|(k, n)| TrustedFriend { trusted_key: k.clone(), trusted_name: n.clone() })
            .collect()
    }

    /// 查詢用：`owner_key` 這帳號目前信任的所有人顯示名（供「信任名單」查詢指令，依字母排序
    /// 讓結果穩定好讀）。
    pub fn list(&self, owner_key: &str) -> Vec<String> {
        let mut names: Vec<String> = self.trusted.get(owner_key)
            .into_iter().flatten()
            .map(|(_, n)| n.clone())
            .collect();
        names.sort();
        names
    }
}

/// 打 `T` 輸入一個名字時，該做加入信任還是撤銷信任——review PR #1252 阻擋項 1、2 的核心修正：
/// - 名單裡已經有唯一一位這個名字的人 → [`TrustLookup::Revoke`]（撤銷，不看對方在不在附近）。
/// - 名單裡沒有，但附近唯一一位在線玩家叫這個名字 → [`TrustLookup::Add`]（加入，維持原本
///   「要走近」的邀請語意）。
/// - 名單裡或附近同時有超過一位同名的人 → [`TrustLookup::Ambiguous`]，寧可拒絕也別猜錯人
///   （阻擋項 2：攻擊者改名成你朋友的名字站你旁邊，不該讓 `find` 隨機選到他）。
/// - 兩邊都沒有 → [`TrustLookup::NotFound`]。
pub enum TrustLookup {
    Revoke { trusted_key: String },
    Add { trusted_key: String },
    Ambiguous,
    NotFound,
}

pub fn resolve_trust_target(
    trusted_matches: &[TrustedFriend],
    nearby_matches: &[String],
) -> TrustLookup {
    if trusted_matches.len() > 1 || nearby_matches.len() > 1 {
        return TrustLookup::Ambiguous;
    }
    if let Some(f) = trusted_matches.first() {
        return TrustLookup::Revoke { trusted_key: f.trusted_key.clone() };
    }
    if let Some(k) = nearby_matches.first() {
        return TrustLookup::Add { trusted_key: k.clone() };
    }
    TrustLookup::NotFound
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
    use crate::voxel_sign::SignHit;

    fn hit(cx: f32, cz: f32, text: &str, owner_key: Option<&str>) -> SignHit {
        SignHit {
            cx,
            cz,
            text: text.to_string(),
            owner: None,
            owner_key: owner_key.map(|s| s.to_string()),
            dist2: 0.0,
        }
    }

    #[test]
    fn find_owner_home_matches_home_toned_sign() {
        let hits = vec![hit(12.0, 34.0, "阿星的家", Some("astar@example.com"))];
        assert_eq!(
            find_owner_home(&hits, "astar@example.com"),
            Some((12.0, 34.0))
        );
    }

    #[test]
    fn find_owner_home_ignores_non_home_sign_same_owner() {
        // 同一帳號立的路標牌（非「家」語氣）不該被當成家的座標。
        let hits = vec![hit(5.0, 5.0, "往礦坑↓", Some("astar@example.com"))];
        assert_eq!(find_owner_home(&hits, "astar@example.com"), None);
    }

    #[test]
    fn find_owner_home_ignores_other_owner() {
        let hits = vec![hit(1.0, 1.0, "小夜的家", Some("yoru@example.com"))];
        assert_eq!(find_owner_home(&hits, "astar@example.com"), None);
    }

    #[test]
    fn find_owner_home_empty_hits_returns_none() {
        let hits: Vec<SignHit> = vec![];
        assert_eq!(find_owner_home(&hits, "astar@example.com"), None);
    }

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
    fn trust_store_add_then_remove() {
        let mut store = TrustStore::new();
        assert!(!store.is_trusted("astar@example.com", "friend@example.com"));
        let ev = store.add("astar@example.com", "friend@example.com", "Friend").unwrap();
        assert!(ev.trusted);
        assert!(store.is_trusted("astar@example.com", "friend@example.com"));
        let ev = store.remove("astar@example.com", "friend@example.com").unwrap();
        assert!(!ev.trusted);
        assert!(!store.is_trusted("astar@example.com", "friend@example.com"));
    }

    #[test]
    fn trust_store_add_twice_is_idempotent() {
        let mut store = TrustStore::new();
        assert!(store.add("astar@example.com", "friend@example.com", "Friend").is_some());
        // 已經信任了，重複加入不重寫、也不 panic。
        assert!(store.add("astar@example.com", "friend@example.com", "Friend").is_none());
        assert!(store.is_trusted("astar@example.com", "friend@example.com"));
    }

    #[test]
    fn trust_store_remove_unknown_is_none() {
        let mut store = TrustStore::new();
        assert!(store.remove("astar@example.com", "nobody@example.com").is_none());
    }

    #[test]
    fn trust_store_is_scoped_per_owner() {
        let mut store = TrustStore::new();
        store.add("astar@example.com", "friend@example.com", "Friend");
        // 我信任了 friend，不代表 friend 也信任我，也不代表別的領地主人信任 friend。
        assert!(!store.is_trusted("friend@example.com", "astar@example.com"));
        assert!(!store.is_trusted("someoneelse@example.com", "friend@example.com"));
    }

    #[test]
    fn trust_store_from_entries_takes_latest_per_pair() {
        let entries = vec![
            TrustEntry { owner_key: "a@example.com".into(), trusted_key: "b@example.com".into(), trusted_name: "B".into(), trusted: true, seq: 0 },
            TrustEntry { owner_key: "a@example.com".into(), trusted_key: "b@example.com".into(), trusted_name: "B".into(), trusted: false, seq: 1 },
        ];
        let store = TrustStore::from_entries(entries);
        assert!(!store.is_trusted("a@example.com", "b@example.com"), "最新一筆是解除，應恢復未信任");
    }

    #[test]
    fn trust_store_from_entries_out_of_order_seq_takes_max() {
        let entries = vec![
            TrustEntry { owner_key: "a@example.com".into(), trusted_key: "b@example.com".into(), trusted_name: "B".into(), trusted: false, seq: 5 },
            TrustEntry { owner_key: "a@example.com".into(), trusted_key: "b@example.com".into(), trusted_name: "B".into(), trusted: true, seq: 2 },
        ];
        let store = TrustStore::from_entries(entries);
        assert!(!store.is_trusted("a@example.com", "b@example.com"), "應取 seq 較大的那筆（解除）");
    }

    #[test]
    fn trust_store_find_by_name_and_list() {
        let mut store = TrustStore::new();
        store.add("astar@example.com", "b@example.com", "小明");
        store.add("astar@example.com", "c@example.com", "小華");
        let found = store.find_by_name("astar@example.com", "小明");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].trusted_key, "b@example.com");
        assert_eq!(store.list("astar@example.com"), vec!["小明".to_string(), "小華".to_string()]);
    }

    // ── resolve_trust_target（review PR #1252 阻擋項 1、2 的核心修正）────────────────────

    #[test]
    fn resolve_trust_target_revoke_does_not_need_nearby() {
        // review 阻擋項 1：信任名單裡已有唯一一位「小明」，附近沒人（對方離線／不在附近）
        // 也該能直接撤銷，不該卡在「要對方在線在附近」。
        let trusted = vec![TrustedFriend { trusted_key: "b@example.com".into(), trusted_name: "小明".into() }];
        match resolve_trust_target(&trusted, &[]) {
            TrustLookup::Revoke { trusted_key } => assert_eq!(trusted_key, "b@example.com"),
            _ => panic!("應判定為撤銷"),
        }
    }

    #[test]
    fn resolve_trust_target_two_nearby_same_name_is_ambiguous() {
        // review 阻擋項 2：附近有兩位同名已登入玩家（例如攻擊者改名冒充你朋友），
        // 不該用 `find` 隨機選一位發出完整寫入權，寧可拒絕。
        let nearby = vec!["real-friend@example.com".to_string(), "impostor@example.com".to_string()];
        assert!(matches!(resolve_trust_target(&[], &nearby), TrustLookup::Ambiguous));
    }

    #[test]
    fn resolve_trust_target_add_needs_unique_nearby() {
        let nearby = vec!["friend@example.com".to_string()];
        match resolve_trust_target(&[], &nearby) {
            TrustLookup::Add { trusted_key } => assert_eq!(trusted_key, "friend@example.com"),
            _ => panic!("唯一一位附近同名在線玩家，應判定為加入"),
        }
    }

    #[test]
    fn resolve_trust_target_neither_is_not_found() {
        assert!(matches!(resolve_trust_target(&[], &[]), TrustLookup::NotFound));
    }

    #[test]
    fn resolve_trust_target_two_trusted_same_name_is_ambiguous() {
        let trusted = vec![
            TrustedFriend { trusted_key: "b@example.com".into(), trusted_name: "小明".into() },
            TrustedFriend { trusted_key: "d@example.com".into(), trusted_name: "小明".into() },
        ];
        assert!(matches!(resolve_trust_target(&trusted, &[]), TrustLookup::Ambiguous));
    }
}
