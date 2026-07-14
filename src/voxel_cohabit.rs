//! 乙太方界·邀居同住 v1（自主提案切片，ROADMAP 972）。
//!
//! **真缺口 / 為誰做**：830「居民認得你的家」讓 AI 居民認出你親手署名的家牌、偶爾登門拜訪——
//! 但拜訪完照樣走回自己在 `resident_home_base`（或 943 都更後）的老家睡覺。963~967/969「玩家
//! 個人領地保護」讓玩家安心地自己蓋，`voxel_relations`/`voxel_bonds`（708/723）讓居民彼此交情
//! 深厚，但**玩家與居民之間，從沒有一件事情真的因為交情深厚而改變**：不管你們多熟，她的家
//! 永遠是她自己的家，你的家永遠只是她偶爾路過的地方。這是「玩家共居」主軸至今唯一沒兌現的
//! 承諾——世界裡從沒有一位居民真的把你的家當成她的家。
//!
//! **做法**：站到深交的居民（`affinity_count` ≥ [`COHABIT_AFFINITY_THRESHOLD`]，比照
//! `voxel_player_recipe::TEACH_MIN_AFFINITY` 全庫最深交等級）身邊按下「🏠 邀居」——她若還沒
//! 跟任何人同住，就會把家搬到你登記的家牌座標（`voxel_landclaim::find_owner_home`，靠「每帳號
//! 僅一塊有效領地」的既有不變量找到你的家在哪），從此在你家附近閒晃、安頓。再按一次即請她
//! 搬回原本的家（[`CohabitEntry::prev_home_*`] 記得她邀居前住哪，永久保留、不會弄丟）。
//!
//! **v1 刻意的範圍收斂**：不做「橫刀奪愛」——已經跟別人同住的居民邀不動（[`CohabitAction::Blocked`]），
//! 想邀就先請她自己人先撤（把設計複雜度壓在最低，避免搶居民的搶奪戰）；不拆她原本的房子
//! （空屋留在原地當作「她曾經住過這裡」的痕跡，不觸碰 963 memory 記錄過的「拆除==放置目標」
//! 收斂陷阱）；一次只能同住一位玩家的家（不做多人合租）。
//!
//! **與既有元素 razor-sharp 區隔**：與 943「殖民地真居住 v1」`RelocationStore`——那是居民自己
//! 因都更／拓荒而搬進**村莊系統**認領的新地塊，全由伺服器 AI 決策；本刀是**玩家主動邀請**搬進
//! **玩家自己蓋的家**，觸發、目的地、能否撤回全由玩家一手主導，兩套家域覆寫（`home_x`/`home_z`）
//! 各自獨立疊加、互不牽動對方的狀態機（[`CohabitStore`] 不佔用 `RelocationStore` 的「一次一位」
//! 名額）。與 830 認得你的家——那是**偶然路過讀牌**才觸發的認知/拜訪；本刀是**玩家主動邀請**
//! 的持久同住，觸發完全不同、效果一個是拜訪一個是搬家。
//!
//! **成本紀律**：零 LLM（觸發、門檻、目的地全是確定性判定/查詢）、零既有格式 migration（新增
//! 獨立的 `data/voxel_cohabit.jsonl`，只 append、不動任何既有欄位）、零新美術、FPS 零影響（同住
//! 只是把既有 `home_x`/`home_z` 覆寫成另一個座標，沿用居民既有的閒晃/夜間安頓邏輯，無新 tick）。
//!
//! **濫用防護**：**必須已登入帳號**才能邀請（歸屬鍵需要伺服器權威解出的 email，比照瓶中信/
//! 領地信任名單護欄，訪客無法冒充邀居）；必須站在觸及範圍內才能對指定居民發動（伺服器驗證，
//! 玩家無法隔空遙控）；門檻吃既有的 `affinity_count`（累積互動次數，無法一次到位刷出深交）；
//! 目的地只能是你自己登記在案的家牌（伺服器查詢，玩家無法指定任意座標）；不收任何自由輸入
//! 文字、不觸發 LLM、不開對外端點。
//!
//! **純邏輯層**：本檔全是零鎖、零 async 的確定性純函式／常數（唯一 IO 是與其餘 `voxel_*`
//! 模組同款的 append-only jsonl 讀寫，鎖與副作用都在 `voxel_ws.rs`，短鎖循序即釋、不巢狀，
//! 守 prod 死鎖鐵律）。

use std::{
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
};

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 邀居互動觸及範圍（世界座標，方塊，XZ 平面）——比照贈禮／交易同量級的「站近才行」距離。
pub const INVITE_REACH: f32 = 5.0;

/// 邀居同住門檻（`affinity_count` 累積互動次數）：比照全庫目前最深交的
/// `voxel_player_recipe::TEACH_MIN_AFFINITY`，讓「把家讓給她」這種等級的信任要靠長期
/// 互動累積，不是隨口一句就能達成。
pub const COHABIT_AFFINITY_THRESHOLD: usize = 8;

/// 一筆同住紀錄（append-only jsonl 落地單位；重啟由 seq 最大者還原每位居民目前狀態）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CohabitEntry {
    /// 同住的居民 id（"vox_res_{i}"）。
    pub resident: String,
    /// 收留她的玩家帳號歸屬鍵（email，不可偽造）。
    pub player_key: String,
    /// 新家（玩家家牌）世界座標，方塊格（XZ；y 對家域邏輯無意義，同住不拆不蓋，故不記）。
    pub home_x: i32,
    pub home_z: i32,
    /// 邀居前她原本住哪——搬走時據此還原，永久保留、不因反覆邀居/搬走而遺失。
    pub prev_home_x: i32,
    pub prev_home_z: i32,
    /// 目前是否仍同住（`false` = 已搬走）。
    pub active: bool,
    /// 單調遞增序號（越大越新；還原時同居民取最新一筆）。
    pub seq: u64,
}

/// 同住 store：每位居民至多同時跟一位玩家同住。純資料；鎖 / 落地由呼叫端（voxel_ws）管。
#[derive(Default)]
pub struct CohabitStore {
    by_resident: HashMap<String, CohabitEntry>,
    next_seq: u64,
}

/// 按下「邀居」這個按鈕，依目前狀態該做什麼（純函式、可測，供呼叫端分支）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CohabitAction {
    /// 目前沒跟任何人同住 → 邀請入住。
    Invite,
    /// 目前正跟你同住 → 再按一次＝請她搬回原本的家。
    Revoke,
    /// 目前跟別人同住 → v1 不做橫刀奪愛，邀不動。
    Blocked,
}

/// `current_host`＝目前同住她的玩家歸屬鍵（無人同住則 `None`）；`player_key`＝按下邀居這位
/// 玩家的歸屬鍵。純函式、窮舉可測。
pub fn decide_action(current_host: Option<&str>, player_key: &str) -> CohabitAction {
    match current_host {
        Some(h) if h == player_key => CohabitAction::Revoke,
        Some(_) => CohabitAction::Blocked,
        None => CohabitAction::Invite,
    }
}

impl CohabitStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 目前同住她的玩家歸屬鍵（無人同住／已搬走 → `None`）。
    pub fn host_of(&self, resident: &str) -> Option<&str> {
        self.by_resident
            .get(resident)
            .filter(|e| e.active)
            .map(|e| e.player_key.as_str())
    }

    /// 目前有效的同住家域覆寫座標（無人同住 → `None`，家域回退到既有的
    /// `resident_home_base`／`RelocationStore` 邏輯）。
    pub fn active_home(&self, resident: &str) -> Option<(i32, i32)> {
        self.by_resident
            .get(resident)
            .filter(|e| e.active)
            .map(|e| (e.home_x, e.home_z))
    }

    /// 開始同住（v1 硬閘：已經跟人同住 → `None`，不做橫刀奪愛）。成功回落地事件供 append。
    pub fn invite(
        &mut self,
        resident: &str,
        player_key: &str,
        home: (i32, i32),
        prev_home: (i32, i32),
    ) -> Option<CohabitEntry> {
        if self.host_of(resident).is_some() {
            return None;
        }
        let rec = CohabitEntry {
            resident: resident.to_string(),
            player_key: player_key.to_string(),
            home_x: home.0,
            home_z: home.1,
            prev_home_x: prev_home.0,
            prev_home_z: prev_home.1,
            active: true,
            seq: self.next_seq,
        };
        self.next_seq = self.next_seq.wrapping_add(1);
        self.by_resident.insert(resident.to_string(), rec.clone());
        Some(rec)
    }

    /// 搬走（僅目前正同住她的那位玩家能撤回；別人喊不動、已搬走的再喊也沒反應）。
    /// 成功回落地事件（含 `prev_home_*` 供呼叫端還原家域座標）。
    pub fn revoke(&mut self, resident: &str, player_key: &str) -> Option<CohabitEntry> {
        let cur = self.by_resident.get(resident)?;
        if !cur.active || cur.player_key != player_key {
            return None;
        }
        let mut rec = cur.clone();
        rec.active = false;
        rec.seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        self.by_resident.insert(resident.to_string(), rec.clone());
        Some(rec)
    }

    /// 從 jsonl 記錄還原（重啟後接續）：依 seq 重放，同居民取最新一筆即為現況。
    pub fn from_entries(entries: Vec<CohabitEntry>) -> Self {
        let mut es = entries;
        es.sort_by_key(|e| e.seq);
        let mut next_seq = 0u64;
        let mut by_resident = HashMap::new();
        for e in es {
            if e.seq >= next_seq {
                next_seq = e.seq.wrapping_add(1);
            }
            by_resident.insert(e.resident.clone(), e);
        }
        Self { by_resident, next_seq }
    }
}

// ── 台詞 / 記憶 / 動態牆（確定性模板，零 LLM）───────────────────────────────────────

/// 入住當下對玩家說的一句話（直接對話，不重述玩家名字）。
pub fn move_in_line(pick: usize) -> String {
    const LINES: [&str; 4] = [
        "真的可以嗎……那我就把行李搬過來、和你住在一起了！",
        "跟你住在同一個屋簷下，光是想到就開心呢。",
        "以後這裡也是我的家了，往後要多多指教囉。",
        "我把最喜歡的幾樣東西也一起搬過來了，謝謝你邀請我。",
    ];
    LINES[pick % LINES.len()].to_string()
}

/// 入住這件事寫進她自己的長期記憶（第一人稱、掛在玩家名下累積好感）。
pub fn move_in_memory(player: &str) -> String {
    format!("{player}邀請我搬去和他同住，我答應了，往後那裡也是我的家")
}

/// 城鎮動態牆一則（不在場的人回來也讀得到）。
pub fn move_in_feed(resident: &str, player: &str) -> String {
    format!("{resident}搬去和{player}同住了")
}

/// 搬走當下對玩家說的一句話。
pub fn move_out_line(pick: usize) -> String {
    const LINES: [&str; 3] = [
        "我決定先搬回自己原本的家，這段時間真的很謝謝你。",
        "還是想搬回老地方住一陣子，別在意，我們還是朋友。",
        "我把行李收一收，先搬回自己家去了，有空還是會來找你。",
    ];
    LINES[pick % LINES.len()].to_string()
}

/// 搬走這件事寫進她自己的長期記憶。
pub fn move_out_memory(player: &str) -> String {
    format!("我搬離了{player}的家，回到自己原本住的地方")
}

/// 城鎮動態牆一則。
pub fn move_out_feed(resident: &str, player: &str) -> String {
    format!("{resident}搬離了{player}的家，回到了自己原本的住處")
}

// ── 持久化 IO（在 voxel_ws.rs 的鎖外呼叫）────────────────────────────────────────────

const VOXEL_COHABIT_PATH: &str = "data/voxel_cohabit.jsonl";

/// 從磁碟載入所有同住事件（啟動時呼叫一次）。
pub fn load_cohabit() -> Vec<CohabitEntry> {
    let Ok(f) = fs::File::open(VOXEL_COHABIT_PATH) else {
        return vec![];
    };
    BufReader::new(f)
        .lines()
        .map_while(Result::ok)
        .filter_map(|l| serde_json::from_str::<CohabitEntry>(&l).ok())
        .collect()
}

/// Append 一筆同住事件。**鐵律**：只在不持任何鎖時呼叫（同步小檔寫、不 await）。
pub fn append_cohabit(rec: &CohabitEntry) {
    if let Some(parent) = std::path::Path::new(VOXEL_COHABIT_PATH).parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(mut f) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(VOXEL_COHABIT_PATH)
    {
        if let Ok(line) = serde_json::to_string(rec) {
            let _ = writeln!(f, "{line}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decide_action_invite_when_nobody_home() {
        assert_eq!(decide_action(None, "a@example.com"), CohabitAction::Invite);
    }

    #[test]
    fn decide_action_revoke_when_same_host() {
        assert_eq!(
            decide_action(Some("a@example.com"), "a@example.com"),
            CohabitAction::Revoke
        );
    }

    #[test]
    fn decide_action_blocked_when_other_host() {
        assert_eq!(
            decide_action(Some("b@example.com"), "a@example.com"),
            CohabitAction::Blocked
        );
    }

    #[test]
    fn invite_succeeds_when_not_cohabiting() {
        let mut s = CohabitStore::new();
        let ev = s
            .invite("vox_res_0", "a@example.com", (10, 20), (0, 0))
            .expect("應該成功入住");
        assert_eq!(ev.resident, "vox_res_0");
        assert_eq!(ev.player_key, "a@example.com");
        assert_eq!((ev.home_x, ev.home_z), (10, 20));
        assert_eq!((ev.prev_home_x, ev.prev_home_z), (0, 0));
        assert!(ev.active);
        assert_eq!(s.active_home("vox_res_0"), Some((10, 20)));
        assert_eq!(s.host_of("vox_res_0"), Some("a@example.com"));
    }

    #[test]
    fn invite_fails_when_already_cohabiting_with_anyone() {
        let mut s = CohabitStore::new();
        s.invite("vox_res_0", "a@example.com", (10, 20), (0, 0));
        // 同一人再邀一次 → 失敗（沒有「續住」語意，先撤才能重邀）。
        assert!(s
            .invite("vox_res_0", "a@example.com", (10, 20), (0, 0))
            .is_none());
        // 另一人想橫刀奪愛 → v1 硬閘擋下。
        assert!(s
            .invite("vox_res_0", "b@example.com", (99, 99), (0, 0))
            .is_none());
    }

    #[test]
    fn revoke_fails_when_not_active() {
        let mut s = CohabitStore::new();
        assert!(s.revoke("vox_res_0", "a@example.com").is_none());
    }

    #[test]
    fn revoke_fails_when_different_player_tries() {
        let mut s = CohabitStore::new();
        s.invite("vox_res_0", "a@example.com", (10, 20), (0, 0));
        assert!(s.revoke("vox_res_0", "b@example.com").is_none());
        // 別人喊不動，狀態不受影響。
        assert_eq!(s.host_of("vox_res_0"), Some("a@example.com"));
    }

    #[test]
    fn revoke_succeeds_and_restores_prev_home() {
        let mut s = CohabitStore::new();
        s.invite("vox_res_0", "a@example.com", (10, 20), (3, 4));
        let ev = s
            .revoke("vox_res_0", "a@example.com")
            .expect("應該成功搬走");
        assert!(!ev.active);
        assert_eq!((ev.prev_home_x, ev.prev_home_z), (3, 4));
        assert_eq!(s.active_home("vox_res_0"), None);
        assert_eq!(s.host_of("vox_res_0"), None);
    }

    #[test]
    fn revoke_then_invite_again_by_new_host_succeeds() {
        let mut s = CohabitStore::new();
        s.invite("vox_res_0", "a@example.com", (10, 20), (0, 0));
        s.revoke("vox_res_0", "a@example.com");
        // 搬走後，另一位玩家可以重新邀請（不再被舊房客卡住）。
        let ev = s
            .invite("vox_res_0", "b@example.com", (30, 40), (1, 1))
            .expect("搬走後應該能被新玩家邀請");
        assert_eq!(ev.player_key, "b@example.com");
        assert_eq!(s.active_home("vox_res_0"), Some((30, 40)));
    }

    #[test]
    fn from_entries_restores_active_invite() {
        let mut s = CohabitStore::new();
        let ev = s.invite("vox_res_0", "a@example.com", (10, 20), (0, 0)).unwrap();
        let restored = CohabitStore::from_entries(vec![ev]);
        assert_eq!(restored.active_home("vox_res_0"), Some((10, 20)));
        assert_eq!(restored.host_of("vox_res_0"), Some("a@example.com"));
    }

    #[test]
    fn from_entries_restores_after_revoke_out_of_order() {
        let mut s = CohabitStore::new();
        let ev1 = s.invite("vox_res_0", "a@example.com", (10, 20), (0, 0)).unwrap();
        let ev2 = s.revoke("vox_res_0", "a@example.com").unwrap();
        // 亂序餵進去（reader 不保證 jsonl 讀取順序），replay 仍要以 seq 為準。
        let restored = CohabitStore::from_entries(vec![ev2, ev1]);
        assert_eq!(restored.active_home("vox_res_0"), None);
        assert_eq!(restored.host_of("vox_res_0"), None);
    }

    #[test]
    fn residents_are_independent() {
        let mut s = CohabitStore::new();
        s.invite("vox_res_0", "a@example.com", (10, 20), (0, 0));
        assert_eq!(s.active_home("vox_res_1"), None);
        assert_eq!(s.host_of("vox_res_1"), None);
    }

    #[test]
    fn move_lines_are_deterministic_and_in_range() {
        for pick in 0..10 {
            assert!(!move_in_line(pick).is_empty());
            assert!(!move_out_line(pick).is_empty());
        }
        assert_eq!(move_in_line(1), move_in_line(1));
    }
}
