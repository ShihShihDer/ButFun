//! 乙太方界·居民居家生活 context v1（靈魂驅動的「在家做什麼」創意引擎）
//!
//! 每位居民的家都有其建築性格（見 [`crate::voxel_arch_style`]）；本模組再往前一步——
//! 依家的**點綴主題**（accent）推出「回到家最想做的家務事」（[`HomeAct`]），並替每位
//! 居民登記幾句**專屬的居家台詞**，讓「星禾傍晚回家點燈」這種畫面既一眼看得出是誰、
//! 又切合他家的靈魂，而非千篇一律的「歇息」。
//!
//! 兩層設計：
//! - **純邏輯層**：`act_for_accent` / `act_apt_now` / `act_verb_zh`，把 accent→家務、
//!   家務是否此刻合宜（點燈只入夜才做）、家務的中文動詞全做成確定性純函式，好測、無 IO。
//! - **資料層**（比照 `voxel_arch_style::load_arch_styles` 慣例）：來源檔
//!   `data/voxel_home_life.jsonl`，每行一位居民的台詞卡；**新 store、純 additive**，
//!   缺檔或壞行安靜略過（居民 fallback 回無台詞），舊部署零感知、向後相容、絕不 panic。

use std::collections::HashMap;
use std::sync::LazyLock;

// ── 純邏輯：居家家務 ──────────────────────────────────────────────────────────

/// 居民回到家最想做的一件家務事，由家的點綴主題（accent）推出。
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum HomeAct {
    /// 生火煮飯（hearth，守灶）。
    Cook,
    /// 打水（water，井邊）。
    DrawWater,
    /// 點燈（lights，僅入夜合宜）。
    LightLamps,
    /// 侍弄花圃（garden）。
    TendGarden,
    /// 翻書（library）。
    Read,
    /// 佇立碑前（monument）。
    GazeMonument,
    /// 迎客（open，好客敞門）。
    Host,
    /// 歇息（none／其他，最保底）。
    Rest,
}

/// 由家的點綴主題字串推出對應家務。
/// 對不到的主題（含空字串、`none`）一律落回 [`HomeAct::Rest`]。
pub fn act_for_accent(accent: &str) -> HomeAct {
    match accent {
        "hearth" => HomeAct::Cook,
        "water" => HomeAct::DrawWater,
        "lights" => HomeAct::LightLamps,
        "garden" => HomeAct::TendGarden,
        "library" => HomeAct::Read,
        "monument" => HomeAct::GazeMonument,
        "open" => HomeAct::Host,
        _ => HomeAct::Rest,
    }
}

/// 該家務此刻是否合宜。
/// [`HomeAct::LightLamps`] 只有入夜（黃昏或夜晚）才點得上；其餘家務全時段皆宜。
pub fn act_apt_now(act: HomeAct, is_dusk_or_night: bool) -> bool {
    match act {
        HomeAct::LightLamps => is_dusk_or_night,
        _ => true,
    }
}

/// 家務的中文動詞（供情境提示／日記描述）。
pub fn act_verb_zh(act: HomeAct) -> &'static str {
    match act {
        HomeAct::Cook => "生火煮飯",
        HomeAct::DrawWater => "打水",
        HomeAct::LightLamps => "點燈",
        HomeAct::TendGarden => "侍弄花圃",
        HomeAct::Read => "翻書",
        HomeAct::GazeMonument => "佇立碑前",
        HomeAct::Host => "迎客",
        HomeAct::Rest => "歇息",
    }
}

// ── 資料層：居家台詞 store ────────────────────────────────────────────────────

/// jsonl 每行的形狀，例：
/// `{"id":"vox_res_8","name":"星禾","accent":"lights","lines":["台詞1","台詞2"]}`。
///
/// 欄位皆以 `#[serde(default)]` 容缺——缺台詞就落回空陣列（該居民無台詞、pick 回 None）。
#[derive(Debug, Clone, serde::Deserialize)]
pub struct HomeLifeCard {
    /// 居民 id（"vox_res_{i}"），程式面以此為鍵。
    #[serde(default)]
    pub id: String,
    /// 顯示名（僅供維護者讀檔對照，載入後不使用）。
    #[serde(default)]
    #[allow(dead_code)]
    pub name: String,
    /// 家的點綴主題（僅供維護者讀檔對照；家務推導走 accent→[`HomeAct`] 的即時映射）。
    #[serde(default)]
    #[allow(dead_code)]
    pub accent: String,
    /// 專屬居家台詞（可為空；空則該居民 pick 回 None）。
    #[serde(default)]
    pub lines: Vec<String>,
}

/// 居民居家台詞 store：居民 id → 台詞清單。
pub struct HomeLifeStore {
    map: HashMap<String, Vec<String>>,
}

impl HomeLifeStore {
    /// 空 store（缺檔／解析全敗時的向後相容底）。
    pub fn new() -> Self {
        HomeLifeStore { map: HashMap::new() }
    }

    /// 已登記的居民數（測試／除錯用）。
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// 是否一位都沒登記。
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// 依 rid 與 salt 確定性地挑一句台詞；未登記／台詞空回 `None`。
    pub fn pick(&self, rid: &str, salt: u64) -> Option<String> {
        let lines = self.map.get(rid)?;
        if lines.is_empty() {
            return None;
        }
        let idx = (salt % lines.len() as u64) as usize;
        Some(lines[idx].clone())
    }
}

impl Default for HomeLifeStore {
    fn default() -> Self {
        Self::new()
    }
}

// ── 持久化 IO（只有讀）──────────────────────────────────────────────────────────

const HOME_LIFE_FILE: &str = "data/voxel_home_life.jsonl";

/// 把 jsonl 內容解析成 store（純函式，抽出來可測）。
/// 解析失敗／缺 id 的行安靜略過；同 id 重覆時**後行為準**（append 覆蓋友善）。
fn parse_home_life(content: &str) -> HomeLifeStore {
    let mut store = HomeLifeStore::new();
    for line in content.lines() {
        if let Ok(c) = serde_json::from_str::<HomeLifeCard>(line) {
            if c.id.is_empty() {
                continue; // 沒 id 無從當鍵，跳過
            }
            store.map.insert(c.id.clone(), c.lines);
        }
    }
    store
}

/// 從指定路徑載入（測試用注入點）；檔案不存在／讀取失敗 → 空 store。
fn load_home_life_from(path: &str) -> HomeLifeStore {
    match std::fs::read_to_string(path) {
        Ok(c) => parse_home_life(&c),
        Err(_) => HomeLifeStore::new(),
    }
}

/// 從 `data/voxel_home_life.jsonl` 載入全部居民居家台詞。
/// 檔案不存在或解析失敗 → 空 store（居民無台詞，向後相容、不 panic）。
pub fn load_home_life() -> HomeLifeStore {
    load_home_life_from(HOME_LIFE_FILE)
}

/// process-global 常駐 store：啟動時載一次，之後只讀。
static HOME_LIFE: LazyLock<HomeLifeStore> = LazyLock::new(load_home_life);

/// 依 rid 與 salt 確定性地取一句居家台詞；未登記／無台詞回 `None`。
pub fn pick_line(rid: &str, salt: u64) -> Option<String> {
    HOME_LIFE.pick(rid, salt)
}

// ── 單元測試 ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn act_for_accent_maps_all_eight() {
        assert_eq!(act_for_accent("hearth"), HomeAct::Cook);
        assert_eq!(act_for_accent("water"), HomeAct::DrawWater);
        assert_eq!(act_for_accent("lights"), HomeAct::LightLamps);
        assert_eq!(act_for_accent("garden"), HomeAct::TendGarden);
        assert_eq!(act_for_accent("library"), HomeAct::Read);
        assert_eq!(act_for_accent("monument"), HomeAct::GazeMonument);
        assert_eq!(act_for_accent("open"), HomeAct::Host);
        // none／未知／空字串一律落回 Rest。
        assert_eq!(act_for_accent("none"), HomeAct::Rest);
        assert_eq!(act_for_accent(""), HomeAct::Rest);
        assert_eq!(act_for_accent("亂寫"), HomeAct::Rest);
    }

    #[test]
    fn light_lamps_only_apt_at_dusk_or_night() {
        // 點燈只有入夜才合宜。
        assert!(!act_apt_now(HomeAct::LightLamps, false));
        assert!(act_apt_now(HomeAct::LightLamps, true));
        // 其餘家務全時段皆宜（不受日夜影響）。
        assert!(act_apt_now(HomeAct::Cook, false));
        assert!(act_apt_now(HomeAct::Cook, true));
        assert!(act_apt_now(HomeAct::Rest, false));
    }

    #[test]
    fn verb_zh_matches_contract() {
        assert_eq!(act_verb_zh(HomeAct::Cook), "生火煮飯");
        assert_eq!(act_verb_zh(HomeAct::LightLamps), "點燈");
        assert_eq!(act_verb_zh(HomeAct::TendGarden), "侍弄花圃");
        assert_eq!(act_verb_zh(HomeAct::GazeMonument), "佇立碑前");
        assert_eq!(act_verb_zh(HomeAct::Rest), "歇息");
    }

    #[test]
    fn pick_line_is_deterministic() {
        let content = "\
{\"id\":\"vox_res_8\",\"name\":\"星禾\",\"accent\":\"lights\",\"lines\":[\"入夜了，我來點燈。\",\"燈火一盞盞亮起。\",\"讓路過的人看得見家。\"]}";
        let store = parse_home_life(content);
        assert_eq!(store.len(), 1);
        // 同 salt 同結果（確定性）。
        let a = store.pick("vox_res_8", 7);
        let b = store.pick("vox_res_8", 7);
        assert_eq!(a, b);
        assert!(a.is_some());
        // salt % 3 選第 1 句（索引 1）。
        assert_eq!(store.pick("vox_res_8", 1).as_deref(), Some("燈火一盞盞亮起。"));
        // salt 繞回：3 與 0 同句。
        assert_eq!(store.pick("vox_res_8", 3), store.pick("vox_res_8", 0));
    }

    #[test]
    fn pick_line_empty_or_missing_returns_none() {
        // 空 store：任何 rid 都回 None。
        let empty = HomeLifeStore::new();
        assert!(empty.pick("vox_res_8", 0).is_none());
        assert!(empty.is_empty());
        // 有登記但台詞空陣列：回 None（不 panic 於 % 0）。
        let store = parse_home_life("{\"id\":\"vox_res_9\",\"lines\":[]}");
        assert!(store.pick("vox_res_9", 5).is_none());
        // 未登記的居民：回 None。
        assert!(store.pick("vox_res_99", 0).is_none());
    }

    #[test]
    fn parse_skips_bad_lines_and_missing_id() {
        // 壞行／缺 id 安靜略過；缺檔回空。
        let content = "\
{\"id\":\"vox_res_1\",\"lines\":[\"一\"]}
這不是 JSON
{\"name\":\"沒有id\",\"lines\":[\"二\"]}";
        let store = parse_home_life(content);
        assert_eq!(store.len(), 1);
        assert_eq!(store.pick("vox_res_1", 0).as_deref(), Some("一"));

        let missing = load_home_life_from("data/絕不存在的居家台詞測試檔_home_test.jsonl");
        assert!(missing.is_empty());
    }
}
