//! 乙太方界·居民生計真身分 v1（A3-livelihood）——居民不再只是「人設口吻」，
//! 而是有一份**真身分**（Role）：農夫、商人、館長、遊者，各有其生計。
//!
//! **設計依據**：既有 `persona_for(i % 4)` 只給了 LLM 對話口吻（MarketBrowser/
//! FarmWorker/TownSquare/Wanderer），沒有一份「這居民靠什麼過活」的持久身分。
//! 本模組補上 `Role`（對應四種 persona 的生計）+ 持久化帳本 `ResidentLivelihood`，
//! 讓「轉職」成為可能——居民的身分能隨世界演進被記住、被改寫，跨重啟不忘。
//!
//! 純邏輯層（無 IO、無鎖、無 async），IO 在 `voxel_ws.rs`。
//! 持久化格式：`data/voxel_livelihood.jsonl`（每筆一位居民的當前身分 `LivelihoodEntry`，
//! append-only；重啟時讀整檔、同一居民以最新 `since_seq` 為準遮蓋舊記錄）。
//!
//! **接線在後續 PR**：本模組不改 `voxel_ws.rs`，只提供純資料結構與 IO 函式。

use std::collections::HashMap;
use serde::{Deserialize, Serialize};

use crate::resident_npc::ResidentPersona;

// ── 生計身分 ──────────────────────────────────────────────────────────────────

/// 居民的生計真身分（比 persona 口吻更進一步——這是「靠什麼過活」）。
///
/// 四種角色一一對應既有四種 persona，讓「檔案不存在時」能用 `persona_for` 的
/// 現況行為 seed，接線後不改變任何居民當下表現，只是把身分顯性化、可持久、可轉職。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    /// 農夫——耕作農田（對應 `FarmWorker`）。
    Farmer,
    /// 商人——市場攤區買賣（對應 `MarketBrowser`）。
    Merchant,
    /// 館長——守著廣場 / 鎮中心的公共空間（對應 `TownSquare`）。
    Curator,
    /// 遊者——四處遊走、居無定所（對應 `Wanderer`）。
    Wanderer,
}

impl Role {
    /// 由 persona seed 出對應 Role（檔案不存在時用 `persona_for` 保持現況行為）。
    pub fn from_persona(persona: ResidentPersona) -> Self {
        match persona {
            ResidentPersona::FarmWorker => Role::Farmer,
            ResidentPersona::MarketBrowser => Role::Merchant,
            ResidentPersona::TownSquare => Role::Curator,
            ResidentPersona::Wanderer => Role::Wanderer,
        }
    }

    /// 穩定字串鍵（供 API / 前端序列化用，與顯示文案分開，避免文案改動牽動協議）。
    pub fn key(self) -> &'static str {
        match self {
            Role::Farmer => "farmer",
            Role::Merchant => "merchant",
            Role::Curator => "curator",
            Role::Wanderer => "wanderer",
        }
    }
}

// ── 持久化格式 ────────────────────────────────────────────────────────────────

/// 一筆持久化記錄：一位居民在某個 seq 起擔任的生計身分。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LivelihoodEntry {
    /// 居民 id（或名字，比照 bonds 的慣例由呼叫端統一）。
    pub resident: String,
    /// 這位居民當前的生計身分。
    pub role: Role,
    /// 這份身分自哪個世界序號（seq）起生效——同一居民多筆時，以最大 `since_seq` 為準。
    pub since_seq: u64,
}

// ── 生計帳本 ──────────────────────────────────────────────────────────────────

/// 居民生計帳本（純同步資料結構，由呼叫端包進 `RwLock`）。
#[derive(Default, Debug)]
pub struct ResidentLivelihood {
    /// key = 居民 id → (當前身分, 自哪個 seq 起生效)。
    roles: HashMap<String, (Role, u64)>,
}

impl ResidentLivelihood {
    pub fn new() -> Self {
        Self::default()
    }

    /// 從持久化記錄還原（啟動時呼叫一次）。
    /// 同一居民出現多筆時，以 **最新（最大）`since_seq`** 那筆為準——
    /// append-only 快照下，後寫的轉職記錄自然遮蓋前身。
    pub fn from_entries(entries: impl IntoIterator<Item = LivelihoodEntry>) -> Self {
        let mut l = Self::new();
        for e in entries {
            match l.roles.get(&e.resident) {
                // 只有更新（>=）的 seq 才覆蓋；相等時後到的優先（沿檔案順序）。
                Some(&(_, seq)) if seq > e.since_seq => {}
                _ => {
                    l.roles.insert(e.resident.clone(), (e.role, e.since_seq));
                }
            }
        }
        l
    }

    /// 查詢某居民的當前生計身分（未登記回 `None`——呼叫端可 fallback 到 persona seed）。
    pub fn role_of(&self, resident: &str) -> Option<Role> {
        self.roles.get(resident).map(|&(role, _)| role)
    }

    /// 查詢某居民身分自哪個 seq 起生效（未登記回 `None`）。
    pub fn since_of(&self, resident: &str) -> Option<u64> {
        self.roles.get(resident).map(|&(_, seq)| seq)
    }

    /// 設定 / 轉職某居民的身分（**冪等**）。
    /// 回傳 `true` 表示「真的改變了身分」（轉職成功、需廣播 / 持久化）；
    /// 回傳 `false` 表示「身分未變」（原本就是這個 role，僅刷新 seq 不算轉職）。
    ///
    /// 冪等語意：對同一 `(resident, role)` 重複呼叫只有第一次回 `true`。
    /// 即使 role 相同，也把 `since_seq` 更新為較新的值（不倒退），
    /// 讓「這身分最近一次被確認」的時間保持前進，但不謊報成轉職。
    pub fn set_role(&mut self, resident: &str, role: Role, since_seq: u64) -> bool {
        match self.roles.get(resident).copied() {
            Some((old_role, old_seq)) => {
                let changed = old_role != role;
                // seq 只前進不倒退（防呆：舊事件晚到不該把身分往回撥）。
                let new_seq = since_seq.max(old_seq);
                self.roles.insert(resident.to_string(), (role, new_seq));
                changed
            }
            None => {
                self.roles.insert(resident.to_string(), (role, since_seq));
                true
            }
        }
    }

    /// 轉成持久化記錄（快照，寫入 jsonl 用）。順序不保證（HashMap 迭代）。
    pub fn to_entries(&self) -> Vec<LivelihoodEntry> {
        self.roles
            .iter()
            .map(|(resident, &(role, since_seq))| LivelihoodEntry {
                resident: resident.clone(),
                role,
                since_seq,
            })
            .collect()
    }

    /// 清除某位居民的身分記錄（居民退休時用）。
    pub fn forget(&mut self, resident: &str) {
        self.roles.remove(resident);
    }

    /// 目前登記在案的居民身分數（測試／除錯用）。
    pub fn len(&self) -> usize {
        self.roles.len()
    }

    /// 是否無任何身分登記。
    pub fn is_empty(&self) -> bool {
        self.roles.is_empty()
    }
}

// ── 持久化 IO（只有函式，鎖在 voxel_ws.rs）──────────────────────────────────

const LIVELIHOOD_FILE: &str = "data/voxel_livelihood.jsonl";

/// 從 `data/voxel_livelihood.jsonl` 讀取所有記錄（檔案不存在回空 Vec）。
/// 比照 `voxel_bonds::load_bonds` / `voxel_romance::load_romance`：純 IO，
/// 解析失敗的行安靜略過（向後相容，欄位缺漏靠 serde default，不 drop 既有資料）。
pub fn load_livelihood() -> Vec<LivelihoodEntry> {
    let content = match std::fs::read_to_string(LIVELIHOOD_FILE) {
        Ok(c) => c,
        Err(_) => return vec![],
    };
    content
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// 把整份生計帳本快照 append 一行到 `data/voxel_livelihood.jsonl`（比照 `save_bonds`：
/// 居民身分數極少——最多與居民數同量級，每次轉職時整份快照重寫也不會無限長大）。
pub fn save_livelihood(livelihood: &ResidentLivelihood) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(LIVELIHOOD_FILE)
    {
        for entry in livelihood.to_entries() {
            if let Ok(line) = serde_json::to_string(&entry) {
                let _ = writeln!(f, "{line}");
            }
        }
    }
}

// ── 單元測試 ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_persona_seeds_each_role() {
        // seed 映射須對得上 persona_for(i % 4) 的四種現況行為。
        assert_eq!(Role::from_persona(ResidentPersona::FarmWorker), Role::Farmer);
        assert_eq!(Role::from_persona(ResidentPersona::MarketBrowser), Role::Merchant);
        assert_eq!(Role::from_persona(ResidentPersona::TownSquare), Role::Curator);
        assert_eq!(Role::from_persona(ResidentPersona::Wanderer), Role::Wanderer);
    }

    #[test]
    fn role_key_stable_strings() {
        assert_eq!(Role::Farmer.key(), "farmer");
        assert_eq!(Role::Merchant.key(), "merchant");
        assert_eq!(Role::Curator.key(), "curator");
        assert_eq!(Role::Wanderer.key(), "wanderer");
    }

    #[test]
    fn empty_ledger_role_of_is_none() {
        let l = ResidentLivelihood::new();
        assert_eq!(l.role_of("露娜"), None);
        assert!(l.is_empty());
        assert_eq!(l.len(), 0);
    }

    #[test]
    fn set_role_new_resident_returns_true() {
        let mut l = ResidentLivelihood::new();
        let changed = l.set_role("露娜", Role::Farmer, 10);
        assert!(changed, "新登記身分應回 true");
        assert_eq!(l.role_of("露娜"), Some(Role::Farmer));
        assert_eq!(l.since_of("露娜"), Some(10));
    }

    #[test]
    fn set_role_is_idempotent() {
        let mut l = ResidentLivelihood::new();
        assert!(l.set_role("露娜", Role::Farmer, 10), "第一次設定應回 true");
        // 相同 role 重複設定不算轉職。
        assert!(!l.set_role("露娜", Role::Farmer, 20), "相同身分應回 false（冪等）");
        assert!(!l.set_role("露娜", Role::Farmer, 30), "再重複仍 false");
        assert_eq!(l.role_of("露娜"), Some(Role::Farmer));
        // 相同 role 時 seq 仍往前刷新（不倒退）。
        assert_eq!(l.since_of("露娜"), Some(30));
    }

    #[test]
    fn set_role_change_returns_true() {
        let mut l = ResidentLivelihood::new();
        l.set_role("露娜", Role::Farmer, 10);
        let changed = l.set_role("露娜", Role::Merchant, 20);
        assert!(changed, "轉職到不同身分應回 true");
        assert_eq!(l.role_of("露娜"), Some(Role::Merchant));
        assert_eq!(l.since_of("露娜"), Some(20));
    }

    #[test]
    fn set_role_seq_never_goes_backwards() {
        let mut l = ResidentLivelihood::new();
        l.set_role("露娜", Role::Farmer, 100);
        // 舊事件（seq 更小）晚到，不該把身分時間往回撥。
        l.set_role("露娜", Role::Merchant, 50);
        assert_eq!(l.role_of("露娜"), Some(Role::Merchant), "身分仍更新");
        assert_eq!(l.since_of("露娜"), Some(100), "seq 只前進不倒退");
    }

    #[test]
    fn from_entries_latest_since_seq_wins() {
        // 同一居民多筆，最新（最大）since_seq 為準。
        let entries = vec![
            LivelihoodEntry { resident: "露娜".into(), role: Role::Farmer, since_seq: 10 },
            LivelihoodEntry { resident: "露娜".into(), role: Role::Merchant, since_seq: 50 },
            LivelihoodEntry { resident: "露娜".into(), role: Role::Curator, since_seq: 30 },
        ];
        let l = ResidentLivelihood::from_entries(entries);
        assert_eq!(l.role_of("露娜"), Some(Role::Merchant), "should pick since_seq=50");
        assert_eq!(l.since_of("露娜"), Some(50));
    }

    #[test]
    fn from_entries_multiple_residents() {
        let entries = vec![
            LivelihoodEntry { resident: "露娜".into(), role: Role::Farmer, since_seq: 10 },
            LivelihoodEntry { resident: "諾娃".into(), role: Role::Wanderer, since_seq: 5 },
        ];
        let l = ResidentLivelihood::from_entries(entries);
        assert_eq!(l.role_of("露娜"), Some(Role::Farmer));
        assert_eq!(l.role_of("諾娃"), Some(Role::Wanderer));
        assert_eq!(l.len(), 2);
    }

    #[test]
    fn to_entries_round_trips_through_from_entries() {
        let mut l = ResidentLivelihood::new();
        l.set_role("露娜", Role::Farmer, 10);
        l.set_role("諾娃", Role::Merchant, 20);
        let entries = l.to_entries();
        let restored = ResidentLivelihood::from_entries(entries);
        assert_eq!(restored.role_of("露娜"), Some(Role::Farmer));
        assert_eq!(restored.role_of("諾娃"), Some(Role::Merchant));
        assert_eq!(restored.since_of("露娜"), Some(10));
        assert_eq!(restored.since_of("諾娃"), Some(20));
    }

    #[test]
    fn forget_clears_resident() {
        let mut l = ResidentLivelihood::new();
        l.set_role("露娜", Role::Farmer, 10);
        l.set_role("諾娃", Role::Merchant, 20);
        l.forget("露娜");
        assert_eq!(l.role_of("露娜"), None, "forget 後清除");
        assert_eq!(l.role_of("諾娃"), Some(Role::Merchant), "無關居民不受影響");
        assert_eq!(l.len(), 1);
    }

    #[test]
    fn from_persona_seed_preserves_current_behavior() {
        // 檔案不存在情境：用 persona_for 的四種 persona seed 出帳本，
        // 每位居民的 role 應與其 persona 一一對應（接線後不改變當下表現）。
        let personas = [
            ResidentPersona::MarketBrowser,
            ResidentPersona::FarmWorker,
            ResidentPersona::TownSquare,
            ResidentPersona::Wanderer,
        ];
        let mut l = ResidentLivelihood::new();
        for (i, &p) in personas.iter().enumerate() {
            l.set_role(&format!("居民{i}"), Role::from_persona(p), 0);
        }
        assert_eq!(l.role_of("居民0"), Some(Role::Merchant));
        assert_eq!(l.role_of("居民1"), Some(Role::Farmer));
        assert_eq!(l.role_of("居民2"), Some(Role::Curator));
        assert_eq!(l.role_of("居民3"), Some(Role::Wanderer));
    }
}
