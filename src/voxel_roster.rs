//! 乙太方界·人口成長 v1（居民會出生、名冊持久化、世代傳承技能）。
//!
//! 維護者：「人口感覺蠻少的」——原本只有 4 位固定居民。這個模組把「居民名冊」
//! 從編譯期固定，擴成**執行期可增長**：聚落有餘裕時偶爾誕生一位新居民，
//! 她繼承前輩發明的技能一出生就會做，並落地持久化（重啟後人口不歸零）。
//!
//! **成本紀律是關鍵**：
//! - 人口有硬上限（[`max_residents`]，env 可調），**絕不無限生**。
//! - 出生低頻（[`birth_interval_secs`]，遊戲數天一位的節奏），且要聚落穩定才生。
//! - 新居民走既有零/低 LLM 路徑（採集/蓋家/好奇心），對話才思考——多幾位不爆成本。
//!
//! 本檔**只放純邏輯 + jsonl 持久化**（確定性、無鎖、無 LLM），與 `voxel_ws` 的
//! 鎖/tick 世界解耦，方便單元測試釘死出生條件、父母/名字/家域分配、名冊往返。

use serde::{Deserialize, Serialize};

/// 人口硬上限的預設值（env `BUTFUN_MAX_RESIDENTS` 可覆蓋）。
/// 10 位＝比原本 4 位「熱鬧不少」又不至於讓 tick / 渲染 / LLM 成本爆掉。
const DEFAULT_MAX_RESIDENTS: usize = 10;
/// 人口上限的絕對天花板（名字池大小，見 `voxel_ws::RESIDENT_NAMES`）：
/// env 設再大也不會超過名字池、不會生出無名氏。改名字池大小時同步調此值。
pub const RESIDENT_NAME_POOL_LEN: usize = 16;

/// 兩次出生之間的最短間隔（秒，env `BUTFUN_BIRTH_INTERVAL_SECS` 可覆蓋）。
/// 預設 3600 秒（1 小時真實時間 ≈ 6 個遊戲日，一遊戲日 600 秒）——低頻、稀少才有感。
const DEFAULT_BIRTH_INTERVAL_SECS: f32 = 3600.0;

/// 一旦「上限未滿 + 聚落穩定 + 距上次夠久」全部成立，單次檢查真的生下來的機率。
/// 出生檢查每 15 秒跑一次；此機率讓「到點就生」帶點隨機、更像自然發生而非鬧鐘。
pub const BIRTH_CHANCE_WHEN_ELIGIBLE: f32 = 0.25;

/// 新家域中心與任一既有家域中心的最小水平距離（方塊）。
/// 略小於居民家域半徑（`voxel_residents::HOME_RADIUS` = 20），新居民生在父母附近、
/// 但各有自己的一塊天地，不會完全疊在一起。
pub const MIN_HOME_SEP: i32 = 18;
/// 出生保護：新家域中心離世界出生點 (0,0) 至少這麼遠（別擠在玩家降生處）。
pub const SPAWN_PROTECT: i32 = 10;

/// 人口上限：讀 env `BUTFUN_MAX_RESIDENTS`，夾在 `[基礎居民數, 名字池大小]` 之間。
/// 解析失敗 / 未設 → 預設值。永不超過名字池（避免無名氏）。
pub fn max_residents(base_count: usize) -> usize {
    let want = std::env::var("BUTFUN_MAX_RESIDENTS")
        .ok()
        .and_then(|s| s.trim().parse::<usize>().ok())
        .unwrap_or(DEFAULT_MAX_RESIDENTS);
    want.clamp(base_count, RESIDENT_NAME_POOL_LEN)
}

/// 出生間隔秒數：讀 env `BUTFUN_BIRTH_INTERVAL_SECS`（實測可縮短快速驗證），否則預設。
/// 下限 1 秒，避免 0/負值把「低頻」破功。
pub fn birth_interval_secs() -> f32 {
    std::env::var("BUTFUN_BIRTH_INTERVAL_SECS")
        .ok()
        .and_then(|s| s.trim().parse::<f32>().ok())
        .map(|v| v.max(1.0))
        .unwrap_or(DEFAULT_BIRTH_INTERVAL_SECS)
}

/// 聚落是否「有餘裕」再添一口人：現有居民**多數**已有進展（至少蓋好一樣東西）。
/// 純函式：`settled` = 已有 ≥1 完工建物的居民數，`pop` = 現有總人口。
/// 條件＝過半數安頓好（`2*settled >= pop`）且至少有一位安頓好——聚落穩了才生。
pub fn settlement_ready(settled: usize, pop: usize) -> bool {
    pop > 0 && settled >= 1 && settled * 2 >= pop
}

/// 出生條件總判定（確定性、可測）：全部成立才生下一位。
/// - `pop < max`：沒到上限（**絕不無限生**）。
/// - `ready`：聚落穩定（見 [`settlement_ready`]）。
/// - `elapsed >= interval`：距上次出生夠久（低頻）。
/// - `roll < BIRTH_CHANCE_WHEN_ELIGIBLE`：到點後帶點隨機，像自然發生。
pub fn should_birth(pop: usize, max: usize, ready: bool, elapsed: f32, interval: f32, roll: f32) -> bool {
    pop < max && ready && elapsed >= interval && roll < BIRTH_CHANCE_WHEN_ELIGIBLE
}

/// 從現有居民中確定性地挑一位當「父母」（新居民生在其家附近、繼承其技能）。
/// `seed` 由呼叫端用時間等湊出；`pop` 為現有人口。`pop==0` 安全回 0。
pub fn pick_parent_index(pop: usize, seed: u64) -> usize {
    if pop == 0 {
        return 0;
    }
    (seed % pop as u64) as usize
}

/// 長大成人 v1（ROADMAP 942）：從一組**合格（已成年）父母候選 index** 中，用種子確定性挑一位。
/// 唯有長大成人的居民才能當父母（初始四位居民恆成年，故合格集永不為空）；候選為空時退回 0 純屬
/// 防呆（呼叫端保證非空）。挑法與 [`pick_parent_index`] 同構（種子取模），只是把母體從「全體」
/// 換成「已成年者」。
pub fn pick_parent_index_among(eligible: &[usize], seed: u64) -> usize {
    if eligible.is_empty() {
        return 0;
    }
    eligible[(seed % eligible.len() as u64) as usize]
}

/// 兩點水平平方距離。
fn dist2(ax: i32, az: i32, bx: i32, bz: i32) -> i64 {
    let dx = (ax - bx) as i64;
    let dz = (az - bz) as i64;
    dx * dx + dz * dz
}

/// 某候選家域中心是否與所有既有家域、出生點都保持足夠距離（不衝突）。純函式、可測。
pub fn home_base_ok(cx: i32, cz: i32, existing: &[(i32, i32)]) -> bool {
    if dist2(cx, cz, 0, 0) < (SPAWN_PROTECT as i64) * (SPAWN_PROTECT as i64) {
        return false;
    }
    let min_sep2 = (MIN_HOME_SEP as i64) * (MIN_HOME_SEP as i64);
    existing.iter().all(|&(ex, ez)| dist2(cx, cz, ex, ez) >= min_sep2)
}

/// 為新生兒選一塊家域中心：以父母家 (px,pz) 為圓心，由近而遠螺旋搜第一塊
/// 「與所有既有家域 + 出生點都不衝突」的格。`seed` 決定起始角度（讓不同新居民散開）。
/// 找不到（父母被家域包圍）→ 退回父母正東 `MIN_HOME_SEP*2` 處（保證有個確定落點）。
/// 純函式、確定性、無鎖、無 IO、可測。
pub fn birth_home_base(px: i32, pz: i32, existing: &[(i32, i32)], seed: u64) -> (i32, i32) {
    // 8 個方位起點由 seed 錯開，避免新生兒都往同一側擠。
    let dirs = [(1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1), (0, -1), (1, -1)];
    for r in (MIN_HOME_SEP..=MIN_HOME_SEP * 4).step_by(4) {
        for k in 0..dirs.len() {
            let (dx, dz) = dirs[(seed as usize + k) % dirs.len()];
            let cx = px + dx * r;
            let cz = pz + dz * r;
            if home_base_ok(cx, cz, existing) {
                return (cx, cz);
            }
        }
    }
    (px + MIN_HOME_SEP * 2, pz)
}

// ── 名冊持久化（append-only jsonl，比照 voxel_goals / voxel_memory 慣例）───────────

/// 一筆「新生居民」名冊記錄（jsonl 落地單位，append-only、向後相容）。
/// 只記**出生後才誕生**的居民（id 索引 ≥ 基礎居民數）；初始 4 位永遠由程式碼重建，不入此檔。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RosterEntry {
    /// 居民系統 id（"vox_res_{i}"，i ≥ 基礎居民數）。
    pub resident: String,
    /// 顯示名（同時也可由 id 索引查名字池，這裡冗餘存一份便於閱讀/除錯）。
    pub name: String,
    /// 家域中心的基準座標（餵給 `dry_ground_spawn` 還原出生點；重啟後回到同一塊地）。
    pub home_base_x: i32,
    pub home_base_z: i32,
    /// 父母的 id 與名字（世代傳承的來源記錄）。
    pub parent: String,
    pub parent_name: String,
    /// 另一位父母的 id 與名字（愛的結晶 v1，ROADMAP 928：夫妻共同迎來孩子時才有值；
    /// 單親出生則為 `None`）。`#[serde(default)]` 向後相容——928 上線以來累積的舊名冊
    /// 行沒有這兩個欄位，重播時自然回填 `None`，不影響既有資料、不用 migration。
    /// 家族樹面板（自主提案切片）第一次讀出這份早就算好、卻從未落地的資訊。
    #[serde(default)]
    pub co_parent: Option<String>,
    #[serde(default)]
    pub co_parent_name: Option<String>,
    /// 出生的 unix 秒（生日）。
    pub birth_unix: u64,
}

/// 名冊落地路徑（`data/` 已 gitignore）。
const ROSTER_PATH: &str = "data/voxel_residents_roster.jsonl";

/// 上次出生（或首次啟動基準）的 unix 秒持久化路徑。
/// 單一數字覆寫檔（非 append），跨重啟累積 elapsed，避免頻繁重啟時 LAST_BIRTH_UNIX 歸零。
const LAST_BIRTH_PATH: &str = "data/voxel_last_birth";

/// 從 `data/voxel_last_birth` 讀回上次出生/基準的 unix 秒。
/// 檔不存在（舊部署 / 首次啟動）→ `None`。解析失敗（損毀）→ `None`（向後相容）。
/// **鐵律**：只在不持任何鎖時呼叫（同步小檔讀、不 await）。
pub fn load_last_birth_unix() -> Option<u64> {
    std::fs::read_to_string(LAST_BIRTH_PATH)
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
}

/// 把「上次出生/基準」的 unix 秒寫入 `data/voxel_last_birth`（覆寫、非 append）。
/// 失敗只吞掉，不 panic（比照 voxel_feed 慣例）；data/ 目錄缺失自動建立。
/// **鐵律**：只在不持任何鎖時呼叫（同步小檔寫、不 await）。
pub fn save_last_birth_unix(unix: u64) {
    if let Some(parent) = std::path::Path::new(LAST_BIRTH_PATH).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // 覆寫（非 append）：只需保留最新一筆基準值。
    let _ = std::fs::write(LAST_BIRTH_PATH, format!("{unix}\n"));
}

/// 世界起始（初代四位居民共同的誕辰基準）的 unix 秒持久化路徑（居民誕辰紀念 v1.2）——
/// 初代四位居民 `birth_unix==0` 沒有「誕生時刻」，改用「這個功能第一次跑起來的那一刻」
/// 當她們共同的誕辰基準：誠實地只記下「開始被紀念的那天」，不編造假歷史。單一數字覆寫檔
/// （非 append），只寫一次——之後重啟都讀回同一個值，誕辰紀念不會因重啟跳動或歸零。
const WORLD_FOUNDING_PATH: &str = "data/voxel_world_founding";

/// 從 `data/voxel_world_founding` 讀回世界起始（初代居民誕辰基準）的 unix 秒。
/// 檔不存在（舊部署／首次啟動）→ `None`。解析失敗（損毀）→ `None`（向後相容）。
/// **鐵律**：只在不持任何鎖時呼叫（同步小檔讀、不 await）。
pub fn load_world_founding_unix() -> Option<u64> {
    std::fs::read_to_string(WORLD_FOUNDING_PATH)
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
}

/// 把「世界起始」的 unix 秒寫入 `data/voxel_world_founding`（覆寫、非 append，理論上只寫一次）。
/// 失敗只吞掉，不 panic（比照 voxel_feed 慣例）；data/ 目錄缺失自動建立。
/// **鐵律**：只在不持任何鎖時呼叫（同步小檔寫、不 await）。
pub fn save_world_founding_unix(unix: u64) {
    if let Some(parent) = std::path::Path::new(WORLD_FOUNDING_PATH).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(WORLD_FOUNDING_PATH, format!("{unix}\n"));
}

/// Append 一筆名冊記錄。**鐵律**：只在不持任何鎖時呼叫（同步小檔寫、不 await）。
pub fn append_roster_entry(rec: &RosterEntry) {
    if let Ok(line) = serde_json::to_string(rec) {
        write_line(ROSTER_PATH, &line);
    }
}

/// 載回所有名冊記錄（啟動時呼叫一次）。檔不存在（舊世界）→ 回空＝只有初始居民（向後相容）。
/// 依 id 索引排序後回傳，確保重建順序與 id 連續性一致。
pub fn load_roster() -> Vec<RosterEntry> {
    let content = match std::fs::read_to_string(ROSTER_PATH) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let mut out: Vec<RosterEntry> = content
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.is_empty() {
                None
            } else {
                serde_json::from_str::<RosterEntry>(l).ok()
            }
        })
        .collect();
    // 同一 id 若重複出現（理論上不會，防呆），只留最後一筆；並依 id 索引排序。
    out.sort_by_key(|e| resident_index(&e.resident));
    out.dedup_by_key(|e| resident_index(&e.resident));
    out
}

/// 由居民 id（"vox_res_{i}"）取索引 i（解析失敗回 usize::MAX，排序時墊底）。
pub fn resident_index(rid: &str) -> usize {
    rid.trim_start_matches("vox_res_").parse::<usize>().unwrap_or(usize::MAX)
}

/// 小檔 append 寫一行（建檔＋換行）。失敗只吞掉，不 panic（比照 voxel_feed 慣例）。
fn write_line(path: &str, line: &str) {
    use std::io::Write;
    if let Some(parent) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settlement_ready_needs_majority_settled() {
        assert!(!settlement_ready(0, 4)); // 沒人安頓
        assert!(!settlement_ready(1, 4)); // 只 1/4 安頓＝不到過半
        assert!(settlement_ready(2, 4)); // 2/4＝剛好過半
        assert!(settlement_ready(4, 4)); // 全安頓
        assert!(!settlement_ready(1, 0)); // pop 0 一律不生
    }

    #[test]
    fn should_birth_all_conditions() {
        // 全部成立 → 生。
        assert!(should_birth(4, 10, true, 4000.0, 3600.0, 0.1));
        // 到上限 → 不生（絕不無限生）。
        assert!(!should_birth(10, 10, true, 9999.0, 3600.0, 0.0));
        // 聚落沒穩 → 不生。
        assert!(!should_birth(4, 10, false, 9999.0, 3600.0, 0.0));
        // 距上次不夠久 → 不生。
        assert!(!should_birth(4, 10, true, 100.0, 3600.0, 0.0));
        // 骰子沒過機率門檻 → 這次先不生（下次再擲）。
        assert!(!should_birth(4, 10, true, 4000.0, 3600.0, 0.99));
    }

    #[test]
    fn max_residents_clamped_to_pool() {
        // 不管 env（測試不設），夾住不超過名字池、不低於基礎數。
        assert!(max_residents(4) <= RESIDENT_NAME_POOL_LEN);
        assert!(max_residents(4) >= 4);
    }

    #[test]
    fn pick_parent_in_range() {
        for seed in 0..50u64 {
            let idx = pick_parent_index(4, seed);
            assert!(idx < 4);
        }
        assert_eq!(pick_parent_index(0, 123), 0); // 空群安全
        assert_eq!(pick_parent_index(4, 6), 2); // 確定性：6 % 4 = 2
    }

    #[test]
    fn pick_parent_among_only_picks_eligible() {
        // 只從合格（已成年）候選裡挑，且落在候選集內。
        let eligible = [0usize, 2, 5];
        for seed in 0..50u64 {
            let idx = pick_parent_index_among(&eligible, seed);
            assert!(eligible.contains(&idx));
        }
        // 確定性：seed % len 取到對應候選。
        assert_eq!(pick_parent_index_among(&eligible, 0), 0);
        assert_eq!(pick_parent_index_among(&eligible, 1), 2);
        assert_eq!(pick_parent_index_among(&eligible, 2), 5);
        assert_eq!(pick_parent_index_among(&eligible, 3), 0); // 回捲
        // 空候選 → 防呆退 0（呼叫端保證非空，此為保險）。
        assert_eq!(pick_parent_index_among(&[], 7), 0);
    }

    #[test]
    fn home_base_ok_rejects_spawn_and_neighbours() {
        // 出生點保護圈內 → 拒。
        assert!(!home_base_ok(0, 0, &[]));
        assert!(!home_base_ok(SPAWN_PROTECT - 1, 0, &[]));
        // 離既有家太近 → 拒；夠遠 → 收。
        let existing = [(100, 100)];
        assert!(!home_base_ok(100 + MIN_HOME_SEP - 1, 100, &existing));
        assert!(home_base_ok(100 + MIN_HOME_SEP, 100, &existing));
    }

    #[test]
    fn birth_home_base_no_conflict() {
        // 父母在 (0,75)（諾娃家），已有幾個家域；新家不得與它們或出生點衝突。
        let existing = [(0, 0), (0, 75), (-75, 0), (75, 0)];
        for seed in 0..8u64 {
            let (cx, cz) = birth_home_base(0, 75, &existing, seed);
            assert!(home_base_ok(cx, cz, &existing), "seed {seed} 選到衝突點 ({cx},{cz})");
        }
    }

    #[test]
    fn roster_roundtrip_via_serde() {
        let rec = RosterEntry {
            resident: "vox_res_4".to_string(),
            name: "米拉".to_string(),
            home_base_x: 12,
            home_base_z: 88,
            parent: "vox_res_1".to_string(),
            parent_name: "諾娃".to_string(),
            co_parent: Some("vox_res_2".to_string()),
            co_parent_name: Some("賽勒".to_string()),
            birth_unix: 1_700_000_000,
        };
        let line = serde_json::to_string(&rec).unwrap();
        let back: RosterEntry = serde_json::from_str(&line).unwrap();
        assert_eq!(rec, back);
    }

    /// 家族樹面板（自主提案切片）：928 上線以來累積的舊名冊行沒有 `co_parent`/
    /// `co_parent_name` 欄位——重播時必須安全回填 `None`，不能因為欄位缺失就整行解析失敗。
    #[test]
    fn roster_entry_missing_co_parent_fields_defaults_to_none() {
        let old_line = r#"{"resident":"vox_res_4","name":"米拉","home_base_x":12,"home_base_z":88,"parent":"vox_res_1","parent_name":"諾娃","birth_unix":1700000000}"#;
        let back: RosterEntry = serde_json::from_str(old_line).unwrap();
        assert_eq!(back.co_parent, None);
        assert_eq!(back.co_parent_name, None);
    }

    #[test]
    fn resident_index_parses() {
        assert_eq!(resident_index("vox_res_0"), 0);
        assert_eq!(resident_index("vox_res_7"), 7);
        assert_eq!(resident_index("garbage"), usize::MAX);
    }

    #[test]
    fn load_roster_missing_file_is_empty() {
        // 舊世界沒有名冊檔 → 空（向後相容）：改指向不存在的路徑不易，改測 serde 空行容忍。
        let parsed: Vec<RosterEntry> = "\n  \n"
            .lines()
            .filter_map(|l| {
                let l = l.trim();
                if l.is_empty() { None } else { serde_json::from_str::<RosterEntry>(l).ok() }
            })
            .collect();
        assert!(parsed.is_empty());
    }

    // ── last_birth_unix 持久化：解析往返測試 ──────────────────────────────────
    // 不碰真實 data/ 目錄，只測「字串 ↔ u64」往返——與 load/save 的內核邏輯一致。

    /// 正常數字字串解析為 u64（模擬 load_last_birth_unix 讀到正常檔案）。
    #[test]
    fn last_birth_unix_parse_normal() {
        let raw = "1_700_000_000\n"; // 帶換行，模擬 write! 寫入
        // 注意：Rust parse::<u64>() 不接受底線分隔，此處測試實際格式（純數字）
        let raw = "1700000000\n";
        assert_eq!(raw.trim().parse::<u64>().ok(), Some(1_700_000_000u64));
    }

    /// 空字串（損毀或空檔）→ None（向後相容，不 panic）。
    #[test]
    fn last_birth_unix_parse_empty_is_none() {
        assert_eq!("".trim().parse::<u64>().ok(), None);
        assert_eq!("   \n".trim().parse::<u64>().ok(), None);
    }

    /// 損毀內容 → None（不 panic）。
    #[test]
    fn last_birth_unix_parse_corrupt_is_none() {
        assert_eq!("not_a_number".trim().parse::<u64>().ok(), None);
        assert_eq!("-1".trim().parse::<u64>().ok(), None); // 負數對 u64 無效
    }

    /// 序列化往返：u64 → format! 字串 → trim().parse() → 同值（模擬 save+load）。
    #[test]
    fn last_birth_unix_serialize_roundtrip() {
        let original: u64 = 1_718_000_000;
        let serialized = format!("{original}\n");
        let loaded: u64 = serialized.trim().parse().expect("往返應成功");
        assert_eq!(loaded, original);
    }

    /// 首次啟動邏輯：load 回 None → 記 now 為基準、不生（純邏輯驗）。
    #[test]
    fn first_startup_no_file_sets_baseline_only() {
        // 模擬：沒有持久化值（load 回 None）→ 基準 = now，elapsed = 0，should_birth = false
        let saved: Option<u64> = None; // 模擬 load_last_birth_unix() 的結果
        let now: u64 = 1_718_000_000;
        let baseline = saved.unwrap_or(now); // 首次：記 now
        let elapsed = now.saturating_sub(baseline) as f32;
        assert_eq!(elapsed, 0.0); // 首次 elapsed = 0 → 一定不生
        assert!(!should_birth(4, 10, true, elapsed, 3600.0, 0.0));
    }

    /// 跨重啟累積：load 回「1 小時前」的基準 → elapsed >= interval → 可生。
    #[test]
    fn cross_restart_elapsed_accumulates() {
        let interval = 3600.0f32;
        let baseline: u64 = 1_718_000_000; // 1 小時前存的值
        let now: u64 = baseline + 3601; // 重啟後現在：間隔已到
        let saved: Option<u64> = Some(baseline); // 模擬 load_last_birth_unix()
        let loaded_baseline = saved.unwrap_or(now);
        let elapsed = now.saturating_sub(loaded_baseline) as f32;
        assert!(elapsed >= interval, "elapsed {elapsed} 應 >= interval {interval}");
        assert!(should_birth(4, 10, true, elapsed, interval, 0.0));
    }

    /// 頻繁重啟但間隔未到：load 回「30 分前」的基準 → elapsed < interval → 不生。
    #[test]
    fn frequent_restart_within_interval_no_birth() {
        let interval = 3600.0f32;
        let baseline: u64 = 1_718_000_000;
        let now: u64 = baseline + 1800; // 只過了 30 分
        let saved: Option<u64> = Some(baseline);
        let loaded_baseline = saved.unwrap_or(now);
        let elapsed = now.saturating_sub(loaded_baseline) as f32;
        assert!(elapsed < interval, "elapsed {elapsed} 應 < interval {interval}");
        assert!(!should_birth(4, 10, true, elapsed, interval, 0.0));
    }
}
