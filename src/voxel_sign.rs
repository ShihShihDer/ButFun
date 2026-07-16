//! 乙太方界·告示牌系統 v1（ROADMAP 740）。
//!
//! 玩家合成告示牌方塊後放置於世界，右鍵互動寫上一行短字（如「露娜的家」「往礦坑↓」），
//! 文字浮在牌子上、所有人都看得到——讓「採集→合成→建造」的基地第一次能被玩家親手
//! 命名、標記、導覽。人類建造／導覽維度（`docs/PLAN_ETHERVOX.md`「蓋造：更多方塊型別」）。
//!
//! **設計**：告示牌文字以世界座標 `(wx, wy, wz)` 為鍵，值為一行短字（`SIGN_MAX_CHARS` 上限）。
//! 比照箱子（ROADMAP 692）的「每座標側資料 + append-only JSONL」範式：多位玩家共用同一
//! 世界，任何人都能改寫既有牌子（先寫先廣播，序列化由 WS handler 的 RwLock 解決）。
//! 告示牌被破壞時文字一併清除（不留孤兒文字）。
//!
//! **persist**：append-only JSONL（`data/voxel_signs.jsonl`），每次寫入記一行；
//! 重啟後 replay 取每座標「最新一筆」重建現況（空字串＝清除，與破壞語意一致）。
//!
//! 純邏輯層：零 async、零鎖、零 IO 外包；鎖/IO/廣播全在 `voxel_ws.rs`。

use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
};

use serde::{Deserialize, Serialize};

/// 持久化路徑。
pub const SIGN_PATH: &str = "data/voxel_signs.jsonl";

/// 告示牌文字上限（字元數，非 byte）——一行短標記，過長截斷。
pub const SIGN_MAX_CHARS: usize = 30;

/// 世界座標鍵（字串格式 "wx,wy,wz"，JSONL 序列化用；與箱子同格式）。
pub fn pos_key(wx: i32, wy: i32, wz: i32) -> String {
    format!("{wx},{wy},{wz}")
}

/// 反解座標鍵 "wx,wy,wz" → (wx, wy, wz)。格式不符回 None。確定性、可測。
pub fn parse_key(k: &str) -> Option<(i32, i32, i32)> {
    let mut it = k.split(',');
    let wx = it.next()?.parse::<i32>().ok()?;
    let wy = it.next()?.parse::<i32>().ok()?;
    let wz = it.next()?.parse::<i32>().ok()?;
    if it.next().is_some() {
        return None; // 多餘欄位＝格式錯誤
    }
    Some((wx, wy, wz))
}

/// 清洗玩家輸入的告示牌文字：去頭尾空白、控制字元（含換行/tab）換成空白、
/// 截到 `SIGN_MAX_CHARS` 字元、再去一次頭尾空白。確定性、無副作用、可測。
/// 回傳空字串代表「清除這面牌子」。
pub fn sanitize_text(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    cleaned.trim().chars().take(SIGN_MAX_CHARS).collect::<String>().trim().to_string()
}

/// 一筆告示牌寫入事件（append-only JSONL 最小單元）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SignEntry {
    /// 告示牌世界座標鍵。
    pub pos: String,
    /// 已清洗的文字（空字串＝清除該座標的牌子）。
    pub text: String,
    /// 單調遞增序號（replay 時取每座標最大 seq 者為現況）。
    pub seq: u64,
    /// 這塊牌是哪位玩家立的（居民認得你的家 v1，自主提案切片，ROADMAP 830）：伺服器在你
    /// 送出 `SignSet` 那刻權威記下你已登入的帳號**顯示名**；訪客／舊資料一律 `None`（不影響
    /// 既有讀牌／認鄰居行為，只是不會被認成「某位玩家的家」）。只給居民辨識／組提示句用，
    /// **不可**當權限判定的歸屬鍵——顯示名可被改名功能改動（見 `owner_key`）。additive、
    /// `#[serde(default)]` 向後相容——舊 JSONL 沒有這個欄位，載回時自動補 `None`。
    #[serde(default)]
    pub owner: Option<String>,
    /// 這塊牌的**穩定**歸屬鍵（玩家個人領地保護 v1，review 修正，ROADMAP 963）：伺服器權威
    /// 記下你已登入帳號的 email——改名不變、無法偽造，專供領地權限判定用（`owner` 顯示名
    /// 只是給居民/提示句看的招牌，不是真正的身分）。訪客／舊資料一律 `None`（該牌無主，
    /// 領地判定行為與今日一致，不保護）。additive、`#[serde(default)]` 向後相容。
    #[serde(default)]
    pub owner_key: Option<String>,
}

/// 全局告示牌 store：pos_key → 文字（只存非空）；`owners`（顯示名，居民辨識用）與
/// `owners_key`（穩定歸屬鍵，領地權限判定用，review 修正 ROADMAP 963）都只存「有主」的牌。
#[derive(Default)]
pub struct SignStore {
    signs: HashMap<String, String>,
    owners: HashMap<String, String>,
    owners_key: HashMap<String, String>,
    next_seq: u64,
}

/// 掃描半徑內命中的一塊告示牌（供領地保護 review 修正用：`nearest_within_xz` 只取最近一塊
/// 會漏掉「範圍內另一塊其實是我自己的領地」，需要全部掃過才能判斷，見
/// [`crate::voxel_landclaim::resolve_claim_block`]）。
#[derive(Debug, Clone, PartialEq)]
pub struct SignHit {
    /// 牌子中心世界座標。
    pub cx: f32,
    pub cz: f32,
    /// 牌面文字。
    pub text: String,
    /// 立牌玩家顯示名（提示句用）。
    pub owner: Option<String>,
    /// 立牌玩家穩定歸屬鍵（權限判定用）。
    pub owner_key: Option<String>,
    /// 與查詢點的水平平方距離（供呼叫端按近到遠排序）。
    pub dist2: f32,
}

impl SignStore {
    /// 空 store（測試 / 首次啟動）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 由載入的歷史事件重建 store（重啟後從 JSONL replay，每座標取最新 seq）。
    pub fn from_entries(entries: Vec<SignEntry>) -> Self {
        // 先找每座標最新（seq 最大）那筆，避免事件亂序時舊蓋新。
        let mut latest: HashMap<String, &SignEntry> = HashMap::new();
        let mut max_seq = 0u64;
        for e in &entries {
            max_seq = max_seq.max(e.seq);
            match latest.get(&e.pos) {
                Some(prev) if prev.seq >= e.seq => {}
                _ => { latest.insert(e.pos.clone(), e); }
            }
        }
        let mut signs = HashMap::new();
        let mut owners = HashMap::new();
        let mut owners_key = HashMap::new();
        for (pos, e) in latest {
            if !e.text.is_empty() {
                signs.insert(pos.clone(), e.text.clone());
                if let Some(o) = &e.owner {
                    owners.insert(pos.clone(), o.clone());
                }
                if let Some(k) = &e.owner_key {
                    owners_key.insert(pos, k.clone());
                }
            }
        }
        Self { signs, owners, owners_key, next_seq: max_seq.saturating_add(1) }
    }

    /// 查詢某座標的告示牌文字（無牌子回 None）。
    pub fn get(&self, pos: &str) -> Option<&str> {
        self.signs.get(pos).map(|s| s.as_str())
    }

    /// 寫入／改寫告示牌文字（傳入已清洗文字）＋這塊牌是哪位玩家立的：`owner` 顯示名（居民
    /// 認得你的家 v1，辨識/提示句用）＋ `owner_key` 穩定歸屬鍵（領地權限判定用，review 修正
    /// ROADMAP 963）。皆 `None`＝訪客或非玩家親手寫的牌，行為與既有一致。空字串＝清除。
    /// 回傳持久化事件供呼叫方 append。
    pub fn set(
        &mut self,
        pos: &str,
        text: String,
        owner: Option<String>,
        owner_key: Option<String>,
    ) -> SignEntry {
        if text.is_empty() {
            self.signs.remove(pos);
            self.owners.remove(pos);
            self.owners_key.remove(pos);
        } else {
            self.signs.insert(pos.to_string(), text.clone());
            match &owner {
                Some(o) => { self.owners.insert(pos.to_string(), o.clone()); }
                None => { self.owners.remove(pos); }
            }
            match &owner_key {
                Some(k) => { self.owners_key.insert(pos.to_string(), k.clone()); }
                None => { self.owners_key.remove(pos); }
            }
        }
        let seq = self.next_seq;
        self.next_seq += 1;
        SignEntry { pos: pos.to_string(), text, seq, owner, owner_key }
    }

    /// 清除指定座標的牌子（破壞方塊時呼叫）。有牌子才回傳清除事件（供 append）。
    pub fn clear(&mut self, pos: &str) -> Option<SignEntry> {
        if self.signs.remove(pos).is_none() {
            return None;
        }
        self.owners.remove(pos);
        self.owners_key.remove(pos);
        let seq = self.next_seq;
        self.next_seq += 1;
        Some(SignEntry { pos: pos.to_string(), text: String::new(), seq, owner: None, owner_key: None })
    }

    /// 找 XZ 平面上距 (x, z) 最近、且水平距離在 `range`（方塊）內的告示牌文字
    /// （供居民「讀牌」偵測附近牌子）。回傳 (牌面文字, 水平平方距離)。純查詢、無副作用。
    /// 牌子稀疏（玩家手動立，數量少），全掃成本可忽略。座標取方塊中心 +0.5 比對。
    /// 找 `range` 內最近的一塊牌，回傳 `(牌子中心 x, 牌子中心 z, 牌面文字, 平方距離, 立牌玩家)`。
    /// 帶座標是為了居民讀牌 v3「重返心中的牌子」——讀到印象深刻的牌子時得記下它在哪，
    /// 日後才走得回去；帶立牌玩家是為了居民認得你的家 v1（830）——讀到玩家親手署名的牌時
    /// 認出這是誰的家。無牌在範圍內回 None。
    pub fn nearest_within_xz(
        &self,
        x: f32,
        z: f32,
        range: f32,
    ) -> Option<(f32, f32, String, f32, Option<String>)> {
        let r2 = range * range;
        let mut best: Option<(f32, f32, String, f32, Option<String>)> = None;
        for (k, text) in &self.signs {
            let Some((sx, _sy, sz)) = parse_key(k) else { continue };
            let cx = sx as f32 + 0.5;
            let cz = sz as f32 + 0.5;
            let dx = cx - x;
            let dz = cz - z;
            let d2 = dx * dx + dz * dz;
            if d2 <= r2 && best.as_ref().is_none_or(|(_, _, _, bd, _)| d2 < *bd) {
                best = Some((cx, cz, text.clone(), d2, self.owners.get(k).cloned()));
            }
        }
        best
    }

    /// 找 XZ 平面上距 (x, z) 在 `range`（方塊）內的**所有**告示牌（領地保護 review 修正
    /// 用：`nearest_within_xz` 只取最近一塊會漏掉「範圍內另一塊其實是我自己的領地」，
    /// 需要掃過全部才能正確判斷歸屬，見 [`crate::voxel_landclaim::resolve_claim_block`]）。
    /// 按距離由近到遠排序（呼叫端據此取「離我最近的別人領地」組提示句）。純查詢、無副作用；
    /// 牌子稀疏，全掃成本可忽略。
    pub fn all_within_xz(&self, x: f32, z: f32, range: f32) -> Vec<SignHit> {
        let r2 = range * range;
        let mut hits: Vec<SignHit> = self
            .signs
            .iter()
            .filter_map(|(k, text)| {
                let (sx, _sy, sz) = parse_key(k)?;
                let cx = sx as f32 + 0.5;
                let cz = sz as f32 + 0.5;
                let dx = cx - x;
                let dz = cz - z;
                let dist2 = dx * dx + dz * dz;
                (dist2 <= r2).then(|| SignHit {
                    cx,
                    cz,
                    text: text.clone(),
                    owner: self.owners.get(k).cloned(),
                    owner_key: self.owners_key.get(k).cloned(),
                    dist2,
                })
            })
            .collect();
        hits.sort_by(|a, b| a.dist2.partial_cmp(&b.dist2).unwrap_or(std::cmp::Ordering::Equal));
        hits
    }

    /// 世界上所有告示牌（不限距離；供邀居同住 v1 之類「不管我人在哪，都要找到我登記的家在
    /// 哪」的查詢使用，見 [`crate::voxel_landclaim::find_owner_home`]）。牌子稀疏（玩家手動
    /// 立，數量少），全掃成本可忽略。`dist2` 恆為 0（未使用查詢中心，呼叫端不依賴距離排序）。
    pub fn all_hits(&self) -> Vec<SignHit> {
        self.signs
            .iter()
            .filter_map(|(k, text)| {
                let (sx, _sy, sz) = parse_key(k)?;
                Some(SignHit {
                    cx: sx as f32 + 0.5,
                    cz: sz as f32 + 0.5,
                    text: text.clone(),
                    owner: self.owners.get(k).cloned(),
                    owner_key: self.owners_key.get(k).cloned(),
                    dist2: 0.0,
                })
            })
            .collect()
    }

    /// 每帳號僅一塊有效領地（玩家個人領地保護 review 修正 第三輪，堵住「無限插旗」濫用面，
    /// ROADMAP 963）：立新家牌時，若該帳號在別的座標已經有一塊有主的家牌，舊的自動失效——
    /// 只保留最新這塊當領地／居民辨識用，牌面文字仍留著（不刪牌，只是不再算誰的），把單一
    /// 帳號的破壞面上界壓到「一個半徑 [`crate::voxel_landclaim::CLAIM_RADIUS`] 的圈」。
    /// 回傳失效事件（供呼叫端 append 持久化；牌面文字沒變，不必廣播浮字）。
    pub fn demote_other_claims(&mut self, owner_key_val: &str, except_pos: &str) -> Vec<SignEntry> {
        let stale: Vec<String> = self
            .owners_key
            .iter()
            .filter(|(pos, k)| pos.as_str() != except_pos && k.as_str() == owner_key_val)
            .map(|(pos, _)| pos.clone())
            .collect();
        let mut events = Vec::new();
        for pos in stale {
            let Some(text) = self.signs.get(&pos).cloned() else { continue };
            self.owners.remove(&pos);
            self.owners_key.remove(&pos);
            let seq = self.next_seq;
            self.next_seq += 1;
            events.push(SignEntry { pos, text, seq, owner: None, owner_key: None });
        }
        events
    }

    /// 舊無主家牌歸戶（玩家個人領地保護 v1，ROADMAP 963；audit-batch2 BROKEN 2）：回傳需 append
    /// 的回填事件（每筆補上 `owner`/`owner_key`，牌面文字與座標不變、`seq` 由 store 續號），並
    /// 就地更新 store 內歸屬（供同進程後續判定即時生效）。`resolve_owner(name)` 由呼叫端提供：
    /// **只在名字恰好對應到唯一登入帳號時回 `Some((顯示名, email))`**，同名多帳號或對應不到回
    /// `None`（見 [`crate::users::UserStore::resolve_unique_email`]）。
    ///
    /// 冪等：只回填「最新現況為家語氣、`owner_key` 為 `None`、且能唯一對應帳號」的座標；已有
    /// `owner_key` 的牌不碰，故重跑不產生新事件。回傳事件已按座標鍵排序求穩定、可測。
    ///
    /// **每帳號僅一塊有效領地**（比照 [`Self::demote_other_claims`] 的既有不變量）：若同一帳號
    /// 同時有多塊舊無主家牌可回填，只認**座標鍵排序最後**那一塊；若該帳號**已有**一塊有主牌
    /// （`owner_key` 已佔用），則這批全部跳過、不再新增第二塊。
    pub fn backfill_owner_key_events<F>(&mut self, mut resolve_owner: F) -> Vec<SignEntry>
    where
        F: FnMut(&str) -> Option<(String, String)>,
    {
        use std::collections::HashSet;
        // 該帳號在回填前是否已佔用「一塊有效領地」名額（以 owner_key 為準，比照
        // demote_other_claims）——若有，這帳號的舊無主家牌一律不再回填，免得冒出第二塊。
        let already_owned: HashSet<String> = self.owners_key.values().cloned().collect();

        // 收集候選座標（最新現況：有文字、家語氣、無 owner_key），避免借用 signs 期間改動。
        let mut candidates: Vec<(String, String)> = self
            .signs
            .iter()
            .filter(|(pos, text)| {
                !self.owners_key.contains_key(pos.as_str())
                    && crate::voxel_readsign::classify(text)
                        == crate::voxel_readsign::SignTone::Home
            })
            .map(|(pos, text)| (pos.clone(), text.clone()))
            .collect();
        candidates.sort_by(|a, b| a.0.cmp(&b.0)); // 同帳號多塊時取排序最後那塊

        // 每個 email 唯一化到一塊：座標排序後者覆蓋前者 → 留最後一塊。
        let mut chosen: HashMap<String, (String, String)> = HashMap::new(); // email → (pos, display)
        for (pos, text) in &candidates {
            let Some(name) = parse_home_owner_name(text) else { continue };
            let Some((display, email)) = resolve_owner(name) else { continue };
            if already_owned.contains(&email) {
                continue; // 該帳號已有一塊有效領地，不再新增
            }
            chosen.insert(email, (pos.clone(), display));
        }

        let mut picks: Vec<(String, String, String)> = chosen
            .into_iter()
            .map(|(email, (pos, display))| (pos, display, email))
            .collect();
        picks.sort_by(|a, b| a.0.cmp(&b.0)); // 依座標鍵排序輸出求穩定

        let mut events = Vec::new();
        for (pos, display, email) in picks {
            let Some(text) = self.signs.get(&pos).cloned() else { continue };
            self.owners.insert(pos.clone(), display.clone());
            self.owners_key.insert(pos.clone(), email.clone());
            let seq = self.next_seq;
            self.next_seq += 1;
            events.push(SignEntry {
                pos,
                text,
                seq,
                owner: Some(display),
                owner_key: Some(email),
            });
        }
        events
    }

    /// 目前所有告示牌（供新玩家連線時一次送出），已按座標鍵排序求穩定。
    pub fn all(&self) -> Vec<(String, String)> {
        let mut v: Vec<(String, String)> =
            self.signs.iter().map(|(k, t)| (k.clone(), t.clone())).collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    }
}

// ── 舊無主家牌歸戶 migration（玩家個人領地保護 v1，ROADMAP 963；audit-batch2 BROKEN 2）─────
//
// **真缺口**：領地保護（`voxel_landclaim`）全走穩定歸屬鍵 `owner_key`（帳號 email）——但
// `owner_key` 這個欄位是 963 之後才加的（`#[serde(default)]`），在那之前立的「XX的家」牌
// `owner`/`owner_key` 都是 `None`，於是**所有既有家牌**永遠落在 `resolve_claim_block` 的
// 「無主＝永不保護」分支，964 的箱子保護、966/967 的信任/拆牌規則對它們全數失效。
//
// **這支 migration 做什麼**：啟動時掃過既有牌，把「已可靠對應到某個真實登入帳號」的舊無主
// 家牌，補上該帳號的 `owner_key`（＋顯示名 `owner`），讓它們納入領地保護。純資料回填、
// append-only、不改牌面文字、不刪任何行。
//
// **資料安全鐵律（絕不誤把 A 的家歸給 B）**：
//   1. **只認唯一對應**：牌面「XX的家」的「XX」必須在帳號名冊裡**恰好對應到一個** email
//      才回填（`resolve_owner` 回 `Some`）；同名多帳號、或對應不到（例：牌名其實是 AI 居民
//      角色名、不是登入帳號）一律**保守留 `None` 不動**——寧可不保護，也不亂歸。
//   2. **只補、不覆蓋**：已經有 `owner_key` 的牌完全不碰（冪等；重跑不產生新事件）。
//   3. **只碰「家」語氣**：非家牌（路標/留言）不圈領地，跳過。
//   4. **只看每座標最新現況**：被清除（最新文字為空）的座標不回填。
//
// 純函式、零 IO；讀名冊的 resolver 與 append/備份由 `voxel_ws.rs` 呼叫端提供（那裡才有帳號
// 名冊與磁碟）。

/// 從「XX的家」這類家牌牌面萃取署名者的名字（「XX」）。沿用居民立牌的署名慣例
/// （`voxel_player_home` 註解：玩家在自家門前立「{名字}的家」）——取**第一個**「的」字前的
/// 整段當名字。找不到「的」、或「的」在開頭（沒有名字）、或不是家語氣，一律回 `None`
/// （保守：無法可靠判定署名者就不歸戶）。純函式、確定性、可測。
pub fn parse_home_owner_name(text: &str) -> Option<&str> {
    if crate::voxel_readsign::classify(text) != crate::voxel_readsign::SignTone::Home {
        return None;
    }
    let idx = text.find('的')?;
    if idx == 0 {
        return None; // 「的家」開頭沒有署名者
    }
    let name = &text[..idx];
    // 去頭尾空白後仍需非空。
    let name = name.trim();
    if name.is_empty() { None } else { Some(name) }
}

// ── 持久化 IO（在 voxel_ws.rs 的鎖外呼叫）────────────────────────────────────────────

/// 從磁碟載入所有告示牌事件（啟動時呼叫一次）。
pub fn load_signs() -> Vec<SignEntry> {
    let Ok(f) = fs::File::open(SIGN_PATH) else { return vec![]; };
    BufReader::new(f)
        .lines()
        .map_while(Result::ok)
        .filter_map(|l| serde_json::from_str::<SignEntry>(&l).ok())
        .collect()
}

/// Append 單筆事件。
pub fn append_sign(entry: &SignEntry) {
    let Ok(line) = serde_json::to_string(entry) else { return; };
    let Ok(mut f) = OpenOptions::new().create(true).append(true).open(SIGN_PATH) else { return; };
    let _ = writeln!(f, "{line}");
}

// ── 單元測試 ─────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_trims_and_strips_control() {
        assert_eq!(sanitize_text("  露娜的家  "), "露娜的家");
        assert_eq!(sanitize_text("往礦坑\n往下"), "往礦坑 往下");
        assert_eq!(sanitize_text("a\tb"), "a b");
    }

    #[test]
    fn sanitize_caps_length() {
        let long = "字".repeat(50);
        let out = sanitize_text(&long);
        assert_eq!(out.chars().count(), SIGN_MAX_CHARS);
    }

    #[test]
    fn sanitize_empty_stays_empty() {
        assert_eq!(sanitize_text("   "), "");
        assert_eq!(sanitize_text("\n\t "), "");
    }

    #[test]
    fn set_and_get() {
        let mut store = SignStore::new();
        store.set("1,2,3", "家".to_string(), None, None);
        assert_eq!(store.get("1,2,3"), Some("家"));
        assert_eq!(store.get("9,9,9"), None);
    }

    #[test]
    fn set_empty_clears() {
        let mut store = SignStore::new();
        store.set("0,0,0", "臨時".to_string(), None, None);
        store.set("0,0,0", String::new(), None, None);
        assert_eq!(store.get("0,0,0"), None);
    }

    #[test]
    fn clear_removes_and_returns_event() {
        let mut store = SignStore::new();
        store.set("5,5,5", "礦坑".to_string(), None, None);
        let ev = store.clear("5,5,5").expect("有牌子應回清除事件");
        assert_eq!(ev.text, "");
        assert_eq!(store.get("5,5,5"), None);
        // 沒牌子的座標清除回 None（不產生多餘事件）。
        assert!(store.clear("5,5,5").is_none());
    }

    #[test]
    fn from_entries_takes_latest_seq() {
        let entries = vec![
            SignEntry { pos: "0,0,0".into(), text: "舊".into(), seq: 0, owner: None, owner_key: None },
            SignEntry { pos: "0,0,0".into(), text: "新".into(), seq: 2, owner: None, owner_key: None },
            SignEntry { pos: "0,0,0".into(), text: "中".into(), seq: 1, owner: None, owner_key: None },
        ];
        let store = SignStore::from_entries(entries);
        assert_eq!(store.get("0,0,0"), Some("新"), "應取 seq 最大者");
        assert_eq!(store.next_seq, 3); // max_seq + 1
    }

    #[test]
    fn from_entries_empty_text_removes() {
        let entries = vec![
            SignEntry { pos: "0,0,0".into(), text: "立牌".into(), seq: 0, owner: None, owner_key: None },
            SignEntry { pos: "0,0,0".into(), text: "".into(), seq: 1, owner: None, owner_key: None }, // 破壞
        ];
        let store = SignStore::from_entries(entries);
        assert_eq!(store.get("0,0,0"), None, "最新是空＝已清除");
    }

    #[test]
    fn all_sorted_and_excludes_empty() {
        let mut store = SignStore::new();
        store.set("2,0,0", "乙".to_string(), None, None);
        store.set("1,0,0", "甲".to_string(), None, None);
        store.set("3,0,0", "".to_string(), None, None); // 空的不列
        let all = store.all();
        assert_eq!(all, vec![("1,0,0".into(), "甲".into()), ("2,0,0".into(), "乙".into())]);
    }

    // ── 立牌玩家 owner（居民認得你的家 v1，自主提案切片，ROADMAP 830）──────────────────────

    #[test]
    fn set_records_owner_and_nearest_within_xz_returns_it() {
        let mut store = SignStore::new();
        store.set("2,4,2", "阿宅的家".to_string(), Some("阿宅".to_string()), None);
        let hit = store.nearest_within_xz(2.5, 2.5, 3.0).expect("範圍內應有牌");
        assert_eq!(hit.4, Some("阿宅".to_string()), "應帶回立牌玩家");
    }

    #[test]
    fn set_without_owner_returns_none() {
        let mut store = SignStore::new();
        store.set("2,4,2", "往礦坑↓".to_string(), None, None);
        let hit = store.nearest_within_xz(2.5, 2.5, 3.0).expect("範圍內應有牌");
        assert_eq!(hit.4, None, "無主的牌（訪客／指路牌）應回 None");
    }

    #[test]
    fn rewriting_sign_without_owner_clears_previous_owner() {
        let mut store = SignStore::new();
        store.set("0,0,0", "阿宅的家".to_string(), Some("阿宅".to_string()), None);
        // 改寫成別的內容、這次沒帶 owner（比照訪客改寫或程式內部改寫）——舊 owner 應被清掉，
        // 不留孤兒歸屬（誤導居民認錯家）。
        store.set("0,0,0", "往礦坑↓".to_string(), None, None);
        let hit = store.nearest_within_xz(0.5, 0.5, 3.0).expect("範圍內應有牌");
        assert_eq!(hit.4, None);
    }

    #[test]
    fn clear_removes_owner_too() {
        let mut store = SignStore::new();
        store.set("5,5,5", "阿宅的家".to_string(), Some("阿宅".to_string()), None);
        store.clear("5,5,5");
        store.set("5,5,5", "新的牌".to_string(), None, None);
        let hit = store.nearest_within_xz(5.5, 5.5, 3.0).expect("範圍內應有牌");
        assert_eq!(hit.4, None, "破壞後重立不應殘留舊 owner");
    }

    #[test]
    fn from_entries_restores_owner_from_latest_seq() {
        let entries = vec![
            SignEntry { pos: "0,0,0".into(), text: "阿宅的家".into(), seq: 0, owner: Some("阿宅".into()), owner_key: None },
            SignEntry { pos: "1,0,0".into(), text: "舊資料無主".into(), seq: 0, owner: None, owner_key: None },
        ];
        let store = SignStore::from_entries(entries);
        assert_eq!(
            store.nearest_within_xz(0.5, 0.5, 1.0).and_then(|h| h.4),
            Some("阿宅".to_string())
        );
        assert_eq!(store.nearest_within_xz(1.5, 0.5, 1.0).and_then(|h| h.4), None);
    }

    #[test]
    fn pos_key_format() {
        assert_eq!(pos_key(1, -2, 300), "1,-2,300");
    }

    #[test]
    fn parse_key_roundtrip_and_reject_bad() {
        assert_eq!(parse_key("1,-2,300"), Some((1, -2, 300)));
        assert_eq!(parse_key(&pos_key(7, 8, -9)), Some((7, 8, -9)));
        assert_eq!(parse_key("1,2"), None); // 欄位不足
        assert_eq!(parse_key("1,2,3,4"), None); // 欄位過多
        assert_eq!(parse_key("a,b,c"), None); // 非整數
    }

    #[test]
    fn nearest_within_finds_closest_in_range() {
        let mut store = SignStore::new();
        store.set("10,4,10", "遠牌".to_string(), None, None);
        store.set("2,4,2", "近牌".to_string(), None, None);
        // 站在 (2.5, 2.5)：近牌在腳下、遠牌 ~11 格外。範圍 3 只找得到近牌。
        let hit = store.nearest_within_xz(2.5, 2.5, 3.0);
        assert_eq!(hit.as_ref().map(|(_, _, t, _, _)| t.clone()), Some("近牌".to_string()));
        // 回傳的座標應為牌子中心（2,2 → 2.5, 2.5）。
        let (cx, cz, _, _, _) = hit.unwrap();
        assert_eq!((cx, cz), (2.5, 2.5));
        // 站得離兩牌都很遠：範圍內沒牌。
        assert!(store.nearest_within_xz(50.0, 50.0, 3.0).is_none());
    }

    #[test]
    fn nearest_within_picks_the_closer_of_two() {
        let mut store = SignStore::new();
        store.set("0,4,0", "A".to_string(), None, None);
        store.set("4,4,0", "B".to_string(), None, None);
        // 站在 (3.6, 0.5)：離 B(4.5,0.5) 比離 A(0.5,0.5) 近。
        assert_eq!(
            store.nearest_within_xz(3.6, 0.5, 8.0).map(|(_, _, t, _, _)| t),
            Some("B".to_string())
        );
    }

    // ── all_within_xz（領地保護 review 修正：掃全部牌，不只取最近一塊）─────────────────────

    #[test]
    fn all_within_xz_returns_every_hit_sorted_by_distance() {
        let mut store = SignStore::new();
        store.set(
            "4,4,0",
            "遠牌".to_string(),
            Some("陌生人".to_string()),
            Some("stranger@example.com".to_string()),
        );
        store.set(
            "0,4,0",
            "近牌".to_string(),
            Some("阿星".to_string()),
            Some("astar@example.com".to_string()),
        );
        // 站在 (0.6, 0.5)：兩塊牌都在範圍 8 內，近牌（0.5,0.5）比遠牌（4.5,0.5）近。
        let hits = store.all_within_xz(0.6, 0.5, 8.0);
        assert_eq!(hits.len(), 2, "範圍內兩塊牌都該回傳，不只最近一塊");
        assert_eq!(hits[0].text, "近牌");
        assert_eq!(hits[0].owner_key.as_deref(), Some("astar@example.com"));
        assert_eq!(hits[1].text, "遠牌");
        assert_eq!(hits[1].owner_key.as_deref(), Some("stranger@example.com"));
        assert!(hits[0].dist2 < hits[1].dist2);
    }

    #[test]
    fn all_within_xz_excludes_out_of_range() {
        let mut store = SignStore::new();
        store.set("0,4,0", "近牌".to_string(), None, None);
        store.set("50,4,50", "遠牌".to_string(), None, None);
        let hits = store.all_within_xz(0.5, 0.5, 3.0);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].text, "近牌");
    }

    #[test]
    fn all_within_xz_carries_owner_key_independent_of_display_name() {
        // owner（顯示名）與 owner_key（穩定歸屬鍵）各自攜帶，互不影響。
        let mut store = SignStore::new();
        store.set(
            "0,0,0",
            "阿宅的家".to_string(),
            Some("阿宅".to_string()),
            Some("azhai@example.com".to_string()),
        );
        let hit = store.all_within_xz(0.5, 0.5, 1.0).into_iter().next().expect("應命中");
        assert_eq!(hit.owner.as_deref(), Some("阿宅"));
        assert_eq!(hit.owner_key.as_deref(), Some("azhai@example.com"));
    }

    // ── demote_other_claims（領地保護 review 修正 第三輪：每帳號僅一塊有效領地）────────────

    #[test]
    fn demote_other_claims_strips_owner_of_stale_sign_only() {
        let mut store = SignStore::new();
        store.set("0,0,0", "舊家".to_string(), Some("阿星".to_string()), Some("astar@example.com".to_string()));
        store.set("10,0,10", "新家".to_string(), Some("阿星".to_string()), Some("astar@example.com".to_string()));
        let events = store.demote_other_claims("astar@example.com", "10,0,10");
        assert_eq!(events.len(), 1, "只有舊的那塊該失效");
        assert_eq!(events[0].pos, "0,0,0");
        assert_eq!(events[0].owner, None);
        assert_eq!(events[0].owner_key, None);
        assert_eq!(events[0].text, "舊家", "牌面文字不變，只是不再算誰的");
        // store 內狀態也同步：舊牌文字還在，但查歸屬應已清空。
        assert_eq!(store.get("0,0,0"), Some("舊家"));
        let hits = store.all_within_xz(0.5, 0.5, 1.0);
        assert_eq!(hits[0].owner_key, None);
        // 新牌不受影響。
        let new_hits = store.all_within_xz(10.5, 10.5, 1.0);
        assert_eq!(new_hits[0].owner_key.as_deref(), Some("astar@example.com"));
    }

    #[test]
    fn demote_other_claims_ignores_other_accounts_and_no_stale() {
        let mut store = SignStore::new();
        store.set("0,0,0", "小夜的家".to_string(), Some("小夜".to_string()), Some("yoru@example.com".to_string()));
        // 阿星立第一塊家牌：不該動到小夜的牌，也沒有自己的舊牌可失效。
        let events = store.demote_other_claims("astar@example.com", "5,0,5");
        assert!(events.is_empty());
        let hits = store.all_within_xz(0.5, 0.5, 1.0);
        assert_eq!(hits[0].owner_key.as_deref(), Some("yoru@example.com"), "別人的領地不受影響");
    }

    // ── 舊無主家牌歸戶 migration（ROADMAP 963，audit-batch2 BROKEN 2）──────────────────────

    #[test]
    fn parse_home_owner_name_extracts_prefix() {
        assert_eq!(parse_home_owner_name("露娜的家"), Some("露娜"));
        assert_eq!(parse_home_owner_name("阿星的小屋"), Some("阿星")); // 「屋」也是家語氣
        assert_eq!(parse_home_owner_name("施育群的家"), Some("施育群"));
    }

    #[test]
    fn parse_home_owner_name_rejects_non_home_or_unparseable() {
        assert_eq!(parse_home_owner_name("往礦坑↓"), None); // 非家語氣
        assert_eq!(parse_home_owner_name("的家"), None); // 沒有署名者
        assert_eq!(parse_home_owner_name("溫暖的窩"), Some("溫暖")); // 有「的」＋家語氣→取前段
        assert_eq!(parse_home_owner_name("家"), None); // 家語氣但無「的」→無法判定署名者
        assert_eq!(parse_home_owner_name("  的家"), None); // 「的」前只有空白
    }

    #[test]
    fn backfill_assigns_owner_when_uniquely_resolvable() {
        let mut store = SignStore::new();
        store.set("0,0,0", "露娜的家".to_string(), None, None); // 舊無主家牌
        // resolver：只有「露娜」對應到唯一帳號 luna@x.com。
        let events = store.backfill_owner_key_events(|name| {
            (name == "露娜").then(|| ("露娜".to_string(), "luna@x.com".to_string()))
        });
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].pos, "0,0,0");
        assert_eq!(events[0].owner_key.as_deref(), Some("luna@x.com"));
        assert_eq!(events[0].text, "露娜的家", "牌面文字不變");
        // store 內狀態即時生效：查歸屬應已補上。
        let hit = store.all_within_xz(0.5, 0.5, 1.0).into_iter().next().unwrap();
        assert_eq!(hit.owner_key.as_deref(), Some("luna@x.com"));
    }

    #[test]
    fn backfill_leaves_unresolvable_untouched() {
        // 對應不到帳號（例：牌名其實是 AI 居民角色名，不是登入帳號）→ 保守留 None 不動。
        let mut store = SignStore::new();
        store.set("0,0,0", "米拉的家".to_string(), None, None);
        let events = store.backfill_owner_key_events(|_name| None);
        assert!(events.is_empty(), "對應不到不該回填");
        let hit = store.all_within_xz(0.5, 0.5, 1.0).into_iter().next().unwrap();
        assert_eq!(hit.owner_key, None, "仍為無主");
    }

    #[test]
    fn backfill_skips_non_home_and_already_owned() {
        let mut store = SignStore::new();
        store.set("0,0,0", "往礦坑↓".to_string(), None, None); // 非家牌
        store.set(
            "10,0,10",
            "阿星的家".to_string(),
            Some("阿星".to_string()),
            Some("astar@x.com".to_string()),
        ); // 已有 owner_key
        let events = store.backfill_owner_key_events(|name| {
            Some((name.to_string(), format!("{name}@x.com")))
        });
        assert!(events.is_empty(), "非家牌與已有主的牌都不該被回填");
    }

    #[test]
    fn backfill_is_idempotent() {
        let mut store = SignStore::new();
        store.set("0,0,0", "露娜的家".to_string(), None, None);
        let resolver =
            |name: &str| (name == "露娜").then(|| ("露娜".to_string(), "luna@x.com".to_string()));
        let first = store.backfill_owner_key_events(resolver);
        assert_eq!(first.len(), 1);
        // 第二次跑：已回填，不再產生事件。
        let second = store.backfill_owner_key_events(resolver);
        assert!(second.is_empty(), "重跑應冪等、零新事件");
    }

    #[test]
    fn backfill_does_not_misassign_across_accounts() {
        // 兩塊不同署名的舊牌，各自對應到各自的帳號——絕不張冠李戴。
        let mut store = SignStore::new();
        store.set("0,0,0", "露娜的家".to_string(), None, None);
        store.set("5,0,5", "阿星的家".to_string(), None, None);
        let events = store.backfill_owner_key_events(|name| match name {
            "露娜" => Some(("露娜".to_string(), "luna@x.com".to_string())),
            "阿星" => Some(("阿星".to_string(), "astar@x.com".to_string())),
            _ => None,
        });
        assert_eq!(events.len(), 2);
        let by_pos: std::collections::HashMap<_, _> =
            events.iter().map(|e| (e.pos.as_str(), e.owner_key.as_deref())).collect();
        assert_eq!(by_pos["0,0,0"], Some("luna@x.com"));
        assert_eq!(by_pos["5,0,5"], Some("astar@x.com"));
    }

    #[test]
    fn backfill_one_claim_per_account_when_multiple_signs() {
        // 同一帳號有兩塊舊無主家牌：每帳號僅一塊有效領地，只回填座標排序最後那塊。
        let mut store = SignStore::new();
        store.set("0,0,0", "露娜的家".to_string(), None, None);
        store.set("9,0,9", "露娜的家".to_string(), None, None);
        let events = store.backfill_owner_key_events(|name| {
            (name == "露娜").then(|| ("露娜".to_string(), "luna@x.com".to_string()))
        });
        assert_eq!(events.len(), 1, "同帳號多塊只回填一塊");
        assert_eq!(events[0].pos, "9,0,9", "取座標鍵排序最後那塊");
        // 另一塊仍無主。
        let hit = store.all_within_xz(0.5, 0.5, 1.0).into_iter().next().unwrap();
        assert_eq!(hit.owner_key, None);
    }

    #[test]
    fn backfill_skips_account_that_already_has_a_claim() {
        // 帳號已有一塊有主家牌，另有一塊同署名的舊無主牌——不新增第二塊有效領地。
        let mut store = SignStore::new();
        store.set(
            "0,0,0",
            "露娜的家".to_string(),
            Some("露娜".to_string()),
            Some("luna@x.com".to_string()),
        );
        store.set("9,0,9", "露娜的家".to_string(), None, None); // 舊無主
        let events = store.backfill_owner_key_events(|name| {
            (name == "露娜").then(|| ("露娜".to_string(), "luna@x.com".to_string()))
        });
        assert!(events.is_empty(), "該帳號已有領地，舊無主牌不再回填");
        let hit = store.all_within_xz(9.5, 9.5, 1.0).into_iter().next().unwrap();
        assert_eq!(hit.owner_key, None);
    }

    #[test]
    fn backfill_replayed_events_survive_restart() {
        // 回填事件 append 後，重啟（from_entries replay）應還原出有主的家牌。
        let mut store = SignStore::new();
        store.set("0,0,0", "露娜的家".to_string(), None, None);
        let events = store.backfill_owner_key_events(|name| {
            (name == "露娜").then(|| ("露娜".to_string(), "luna@x.com".to_string()))
        });
        // 模擬持久化：把原始 set 事件與回填事件一起 replay。
        let mut all = vec![SignEntry {
            pos: "0,0,0".into(),
            text: "露娜的家".into(),
            seq: 0,
            owner: None,
            owner_key: None,
        }];
        all.extend(events);
        let restored = SignStore::from_entries(all);
        let hit = restored.all_within_xz(0.5, 0.5, 1.0).into_iter().next().unwrap();
        assert_eq!(hit.owner_key.as_deref(), Some("luna@x.com"), "重啟後仍有主");
    }

    #[test]
    fn demote_other_claims_handles_multiple_stale_signs() {
        let mut store = SignStore::new();
        store.set("0,0,0", "家一".to_string(), Some("阿星".to_string()), Some("astar@example.com".to_string()));
        store.set("20,0,20", "家二".to_string(), Some("阿星".to_string()), Some("astar@example.com".to_string()));
        store.set("40,0,40", "家三".to_string(), Some("阿星".to_string()), Some("astar@example.com".to_string()));
        let events = store.demote_other_claims("astar@example.com", "40,0,40");
        assert_eq!(events.len(), 2, "兩塊舊牌都該失效，只留最新那塊");
        let positions: Vec<&str> = events.iter().map(|e| e.pos.as_str()).collect();
        assert!(positions.contains(&"0,0,0"));
        assert!(positions.contains(&"20,0,20"));
    }
}
