//! 乙太方界·居民靈魂常駐角色 context v1
//!
//! 每位初始居民都有一段人工著墨的深邃「角色卡」（soul_prompt，第二人稱「你是露娜…」約
//! 300 字）——本模組把它載成常駐唯讀資料，讓 `spawn_resident_think` 組思考脈絡時把這份
//! 角色底**永遠置頂**注入 prompt：便宜即時腦（本地 ollama／免費 tier）從此被這份豐富角色
//! 墊高，居民扮好「自己這個人」而不再只是泛用 persona 口吻。
//!
//! 資料層設計（比照 `voxel_livelihood::load_livelihood` / `voxel_bonds::load_bonds` 慣例）：
//! - 來源檔 `data/voxel_resident_soul.jsonl`，每行 `{id, name, soul_prompt}`。
//! - **新 store、純 additive**：不動任何既有 store；檔案不存在或解析失敗 → 空 store，
//!   居民 fallback 現況行為（不注入），舊部署零感知、向後相容。
//! - 唯讀常駐：啟動 load 一次進 Hub，之後只讀不寫（無 save 函式——靈魂由維護者手工著墨，
//!   不由遊戲執行期改寫）。

use std::collections::HashMap;

/// jsonl 每行的形狀：`{"id":"vox_res_0","name":"露娜","soul_prompt":"你是露娜…"}`。
/// `name` 僅供人讀檔時對照，程式面以 `id` 為鍵；缺欄靠 serde default 安靜略過不 drop 整行。
#[derive(serde::Deserialize)]
struct SoulEntry {
    id: String,
    /// 顯示名（僅供維護者讀檔對照，載入後不使用）。
    #[serde(default)]
    #[allow(dead_code)]
    name: String,
    soul_prompt: String,
}

/// 居民靈魂 store：居民 id（"vox_res_{i}"）→ 常駐角色卡 soul_prompt。
pub struct SoulStore {
    souls: HashMap<String, String>,
}

impl SoulStore {
    /// 空 store（缺檔／解析全敗時的向後相容底）。
    pub fn new() -> Self {
        SoulStore { souls: HashMap::new() }
    }

    /// 取某位居民的靈魂角色卡；未登記回 `None`（呼叫端維持現況、不注入）。
    pub fn soul_of(&self, id: &str) -> Option<&str> {
        self.souls.get(id).map(String::as_str)
    }

    /// 已登記的靈魂數（測試／除錯用）。
    pub fn len(&self) -> usize {
        self.souls.len()
    }

    /// 是否一份靈魂都沒有。
    pub fn is_empty(&self) -> bool {
        self.souls.is_empty()
    }
}

impl Default for SoulStore {
    fn default() -> Self {
        Self::new()
    }
}

// ── 持久化 IO（只有讀，鎖在 voxel_ws.rs）────────────────────────────────────

const SOUL_FILE: &str = "data/voxel_resident_soul.jsonl";

/// 把 jsonl 內容解析成 SoulStore（純函式，抽出來可測）。
/// 解析失敗的行安靜略過；同 id 重覆時**後行為準**（append 友善：日後手工補一行即覆蓋）。
fn parse_souls(content: &str) -> SoulStore {
    let mut store = SoulStore::new();
    for line in content.lines() {
        if let Ok(e) = serde_json::from_str::<SoulEntry>(line) {
            store.souls.insert(e.id, e.soul_prompt);
        }
    }
    store
}

/// 從指定路徑載入（測試用注入點）；檔案不存在／讀取失敗 → 空 store。
fn load_souls_from(path: &str) -> SoulStore {
    match std::fs::read_to_string(path) {
        Ok(c) => parse_souls(&c),
        Err(_) => SoulStore::new(),
    }
}

/// 從 `data/voxel_resident_soul.jsonl` 載入全部居民靈魂。
/// 檔案不存在或解析失敗 → 空 store（居民 fallback 現況行為，向後相容）。
pub fn load_souls() -> SoulStore {
    load_souls_from(SOUL_FILE)
}

// ── 單元測試 ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_round_trip_keeps_soul_by_id() {
        // 以 serde_json 正向序列化再解析——round-trip 驗證行格式與載入互相咬合。
        let line = serde_json::json!({
            "id": "vox_res_0",
            "name": "露娜",
            "soul_prompt": "你是露娜，乙太方界最早醒來的居民。",
        })
        .to_string();
        let store = parse_souls(&line);
        assert_eq!(store.len(), 1);
        assert_eq!(
            store.soul_of("vox_res_0"),
            Some("你是露娜，乙太方界最早醒來的居民。")
        );
    }

    #[test]
    fn parse_skips_bad_lines_and_last_wins() {
        // 壞行安靜略過（向後相容）；同 id 重覆以後行為準（append 覆蓋友善）。
        let content = "\
{\"id\":\"vox_res_1\",\"name\":\"諾娃\",\"soul_prompt\":\"你是諾娃v1\"}
這不是 JSON
{\"id\":\"vox_res_1\",\"name\":\"諾娃\",\"soul_prompt\":\"你是諾娃v2\"}";
        let store = parse_souls(content);
        assert_eq!(store.len(), 1);
        assert_eq!(store.soul_of("vox_res_1"), Some("你是諾娃v2"));
    }

    #[test]
    fn missing_file_returns_empty_store() {
        // 缺檔回空（不 panic、不留半份）——舊部署沒放靈魂檔時居民維持現況行為。
        let store = load_souls_from("data/絕不存在的靈魂測試檔_soul_test.jsonl");
        assert!(store.is_empty());
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn soul_of_hit_and_miss() {
        let store = parse_souls(
            "{\"id\":\"vox_res_2\",\"name\":\"賽勒\",\"soul_prompt\":\"你是賽勒，村裡的漁夫。\"}",
        );
        // 命中：拿得到完整角色卡。
        assert_eq!(store.soul_of("vox_res_2"), Some("你是賽勒，村裡的漁夫。"));
        // 未命中：未登記的居民回 None（呼叫端不注入、維持現況）。
        assert_eq!(store.soul_of("vox_res_99"), None);
        assert!(!store.is_empty());
    }
}
