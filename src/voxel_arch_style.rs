//! 乙太方界·居民建築性格 context v1（靈魂驅動的個人建築「創意引擎」）
//!
//! 每位初始居民都有一段人工著墨的**建築性格卡**（牆材／屋頂／尖頂／點綴／樓層傾向／
//! 色調），讓他們親手蓋的家一眼看得出是「這個人」蓋的，反映其靈魂——不再十棟同模板。
//! 本模組把它載成常駐唯讀資料，供 [`crate::voxel_building::BuildStyle::for_resident`]
//! 在算完 hash-based 樣式後**確定性地套用**（同居民同錨點永遠同一份家）。
//!
//! 資料層設計（比照 `voxel_resident_soul::load_souls` 慣例）：
//! - 來源檔 `data/voxel_arch_style.jsonl`，每行一位居民的建築性格。
//! - **新 store、純 additive**：不動任何既有 store；檔案不存在或某行解析失敗 → 跳過該行／
//!   空 store，居民 fallback 回原本 hash-based 樣式，舊部署零感知、向後相容。
//! - 唯讀常駐：process-global 載一次（[`LazyLock`]），之後只讀不寫（建築性格由維護者手工
//!   著墨，不由遊戲執行期改寫）。

use std::collections::HashMap;
use std::sync::LazyLock;

/// jsonl 每行的形狀，例：
/// `{"id":"vox_res_0","name":"露娜","wall":"wood","roof":"wood","peaked":true,`
/// `"accent":"hearth","stories_bias":0,"palette":"warm","note":"…"}`。
///
/// 欄位皆以 `#[serde(default)]` 容缺——缺牆材／缺屋頂等就落回空字串，套用端找不到對應
/// Block 時保留原 hash-based 值（安靜降級、不 drop 整行）。
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ArchStyle {
    /// 居民 id（"vox_res_{i}"），程式面以此為鍵。
    #[serde(default)]
    pub id: String,
    /// 顯示名（僅供維護者讀檔對照，載入後不使用）。
    #[serde(default)]
    #[allow(dead_code)]
    pub name: String,
    /// 牆體主建材：wood|plank|stone|brick|smoothstone|sand。
    #[serde(default)]
    pub wall: String,
    /// 屋頂建材：wood|stone|smoothstone|leaves。
    #[serde(default)]
    pub roof: String,
    /// 尖頂：平頂上再疊一層縮小的脊。
    #[serde(default)]
    pub peaked: bool,
    /// 點綴主題：garden|water|lights|monument|hearth|library|open|none。
    #[serde(default)]
    pub accent: String,
    /// 樓層傾向：0（矮 1-2）｜1（中 2）｜2（高偏 3）。
    #[serde(default)]
    pub stories_bias: i32,
    /// 色調（僅供維護者對照，載入後不使用；日後前端配色可能消費）。
    #[serde(default)]
    #[allow(dead_code)]
    pub palette: String,
    /// 設計註記（僅供維護者讀檔對照，載入後不使用）。
    #[serde(default)]
    #[allow(dead_code)]
    pub note: String,
}

/// 居民建築性格 store：居民 id → 建築性格卡。
pub struct ArchStyleStore {
    map: HashMap<String, ArchStyle>,
}

impl ArchStyleStore {
    /// 空 store（缺檔／解析全敗時的向後相容底）。
    pub fn new() -> Self {
        ArchStyleStore { map: HashMap::new() }
    }

    /// 取某位居民的建築性格；未登記回 `None`（呼叫端維持原 hash-based 樣式）。
    pub fn get(&self, id: &str) -> Option<&ArchStyle> {
        self.map.get(id)
    }

    /// 已登記的性格數（測試／除錯用）。
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// 是否一份性格都沒有。
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

impl Default for ArchStyleStore {
    fn default() -> Self {
        Self::new()
    }
}

// ── 持久化 IO（只有讀）──────────────────────────────────────────────────────────

const ARCH_FILE: &str = "data/voxel_arch_style.jsonl";

/// 把 jsonl 內容解析成 store（純函式，抽出來可測）。
/// 解析失敗／缺 id 的行安靜略過；同 id 重覆時**後行為準**（append 覆蓋友善）。
fn parse_arch_styles(content: &str) -> ArchStyleStore {
    let mut store = ArchStyleStore::new();
    for line in content.lines() {
        if let Ok(a) = serde_json::from_str::<ArchStyle>(line) {
            if a.id.is_empty() {
                continue; // 沒 id 無從當鍵，跳過
            }
            store.map.insert(a.id.clone(), a);
        }
    }
    store
}

/// 從指定路徑載入（測試用注入點）；檔案不存在／讀取失敗 → 空 store。
fn load_arch_styles_from(path: &str) -> ArchStyleStore {
    match std::fs::read_to_string(path) {
        Ok(c) => parse_arch_styles(&c),
        Err(_) => ArchStyleStore::new(),
    }
}

/// 從 `data/voxel_arch_style.jsonl` 載入全部居民建築性格。
/// 檔案不存在或解析失敗 → 空 store（居民 fallback 原 hash-based 樣式，向後相容）。
pub fn load_arch_styles() -> ArchStyleStore {
    load_arch_styles_from(ARCH_FILE)
}

/// process-global 常駐 store：啟動時載一次，之後只讀。
static ARCH_STYLES: LazyLock<ArchStyleStore> = LazyLock::new(load_arch_styles);

/// 取某位居民的建築性格（回 clone，避免呼叫端持有 static 借用）；
/// 未登記回 `None`（呼叫端維持原 hash-based 樣式）。
pub fn arch_of(id: &str) -> Option<ArchStyle> {
    ARCH_STYLES.get(id).cloned()
}

// ── 單元測試 ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_round_trip_keeps_fields_by_id() {
        // 以 serde_json 正向序列化再解析——round-trip 驗證行格式與載入互相咬合。
        let line = serde_json::json!({
            "id": "vox_res_0",
            "name": "露娜",
            "wall": "wood",
            "roof": "wood",
            "peaked": true,
            "accent": "hearth",
            "stories_bias": 0,
            "palette": "warm",
            "note": "照顧者的守灶人",
        })
        .to_string();
        let store = parse_arch_styles(&line);
        assert_eq!(store.len(), 1);
        let a = store.get("vox_res_0").expect("應取得露娜的建築性格");
        assert_eq!(a.wall, "wood");
        assert_eq!(a.roof, "wood");
        assert!(a.peaked);
        assert_eq!(a.accent, "hearth");
        assert_eq!(a.stories_bias, 0);
    }

    #[test]
    fn parse_skips_bad_lines_and_last_wins() {
        // 壞行／缺 id 安靜略過（向後相容）；同 id 重覆以後行為準（append 覆蓋友善）。
        let content = "\
{\"id\":\"vox_res_1\",\"wall\":\"brick\",\"accent\":\"library\",\"stories_bias\":2}
這不是 JSON
{\"name\":\"沒有id\",\"wall\":\"stone\"}
{\"id\":\"vox_res_1\",\"wall\":\"plank\",\"accent\":\"garden\",\"stories_bias\":1}";
        let store = parse_arch_styles(content);
        assert_eq!(store.len(), 1);
        let a = store.get("vox_res_1").unwrap();
        assert_eq!(a.wall, "plank");
        assert_eq!(a.accent, "garden");
        assert_eq!(a.stories_bias, 1);
    }

    #[test]
    fn missing_fields_default_safely() {
        // 缺欄位靠 serde default 補：bool→false、String→""、i32→0（不 drop 整行）。
        let store = parse_arch_styles("{\"id\":\"vox_res_2\"}");
        let a = store.get("vox_res_2").unwrap();
        assert_eq!(a.wall, "");
        assert_eq!(a.roof, "");
        assert!(!a.peaked);
        assert_eq!(a.accent, "");
        assert_eq!(a.stories_bias, 0);
    }

    #[test]
    fn missing_file_returns_empty_store() {
        // 缺檔回空（不 panic、不留半份）——舊部署沒放性格檔時居民維持原樣式。
        let store = load_arch_styles_from("data/絕不存在的建築性格測試檔_arch_test.jsonl");
        assert!(store.is_empty());
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn get_hit_and_miss() {
        let store = parse_arch_styles(
            "{\"id\":\"vox_res_3\",\"wall\":\"stone\",\"roof\":\"stone\",\"accent\":\"water\"}",
        );
        assert_eq!(store.get("vox_res_3").map(|a| a.accent.as_str()), Some("water"));
        // 未命中：未登記的居民回 None（呼叫端維持原樣式）。
        assert!(store.get("vox_res_99").is_none());
        assert!(!store.is_empty());
    }
}
