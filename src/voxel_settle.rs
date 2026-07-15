//! 乙太方界·殖民地真居住 v1——拓荒者真的遷居殖民地、第二村活起來（架構級大弧，
//! 承 PLAN_ETHERVOX §7「居民散佈世界各處住」；維護者已點頭）。
//!
//! **真缺口**：分村殖民 v1（`voxel_colony.rs`）讓主村外派拓荒隊在遠方奠下「有名字、有立村
//! 故事」的野外村落殘核——但**只是個空殼**：奠基紀錄裡的「拓荒者」只是故事字段，居民全都
//! 還住在主村。玩家跋涉到風禾屯，讀得到來歷，卻看不到任何人在那裡生活。
//!
//! **本刀範圍（v1，刻意有界）**：把「單一村莊中心」的資料模型**漸進式一般化**成「居民屬於
//! 某個聚落（settlement）」——主村＝聚落 0、殖民地 seq=s ＝聚落 s+1：
//!
//! 1. [`SettlementStore`]：居民 → 聚落歸屬（append-only jsonl、向後相容：沒有記錄＝主村）。
//! 2. **拓荒者真搬家**：奠基紀錄裡的 founders 在奠基後真的遷居——重用既有搬家引擎
//!    （`voxel_village::RelocationStore`，#1145 蓋新家/拆舊回收/錨點遷移），只是目的地改成
//!    殖民地中心旁的小地塊。已存在的殖民地（如風禾屯）部署後自動觸發（冪等、跨重啟可恢復）。
//! 3. **殖民地自己長**：遷居後的居民認領 [`colony_plots`]（殖民地中心周圍的小地塊環），
//!    蓋家/擴建全在殖民地；認領時從殖民地中心鋪一小段路（重用 `pave_path_cells`，只加不拆）。
//!    人口成長（maybe_birth）的孩子承繼父母的聚落歸屬——殖民地會自己添丁。
//! 4. **v1 明確不動（留後續、主村限定）**：暮聚、紀念柱、夜燈守望 far_from_village、
//!    村莊集體里程碑、挖掘離村禁區（殖民地周邊不設禁挖）、村莊地圖端點（只畫主村）——
//!    那些系統仍以主村中心為基準，等聚落模型站穩再逐一搬。
//!
//! **v2·殖民地安全網（真缺口，接續上方第 4 點留後續清單的前兩項）**：v1 上線後風禾屯真的
//! 有人住了，但「村莊庇護」從沒跟著搬過去——夜間暗影生成的 `far_from_village` 判定、居民
//! 自主開挖的離村禁區，兩者至今**只認主村中心**，殖民地半徑之外形同不設防：暗影可以貼著
//! 拓荒者的新家生成，居民自己閒晃備料時也可能就地把剛蓋好的殖民地挖出坑洞。本刀補上
//! [`nearest_settlement_center`]——把「離某個聚落中心多遠」泛化成「離**最近一個**聚落中心
//! （主村或任一殖民地）多遠」，讓兩套既有保護在殖民地也生效，不需要為每座殖民地各開一份
//! 判定。紀念柱／村莊集體里程碑／村莊地圖端點三項留待後續刀（各自要接的資料流不同，一次
//! 一塊）。
//!
//! **v3·殖民地擴建（自主提案切片，ROADMAP 1004）**：`colony_plots` 頭註自己誠實寫著
//! 「v1 上限 8 戶——殖民地是小村，有界成長」——但 965 已補上安全網、996 也讓殖民地有了
//! 自己的集體里程碑（2/4/8 三檔，8＝「住滿的殖民地」），風禾屯這種第二村撞頂之後就此
//! 凍結：待遷居名單裡的居民永遠遷不進去、里程碑衝到頂也再無下文。本刀不改「殖民地是
//! 小村」的療癒調性（不做無界成長），而是老實把「小村」的天花板抬高一階：
//! [`colony_plots_outer`] 補一圈更外圍的 8 塊地（[`colony_plots`] 那圈住滿才會用到），
//! 讓一座殖民地能繼續長到 16 戶——仍是有界成長，只是把界往外挪了一圈。
//! **與既有系統區隔**：不動 [`colony_plots`] 本身（既有玩家的地塊座標零回歸），純粹在
//! 它全滿時多一個備援環可認領；`voxel_colony_milestone.rs` 對應補兩檔更高門檻（12/16）。
//!
//! **純邏輯層鐵律**：本檔零 LLM、零鎖、零 async、零世界 IO——聚落歸屬/遷居名單/殖民地
//! 小地塊佈局/句式池全是確定性純函式，方便單元測試釘死。真正動世界（認領、鋪路、搬家 tick）
//! 全在 `voxel_ws.rs`，嚴守短鎖鐵律。
//! **資料安全**：append-only 落地、`#[serde(default)]` 向後相容、絕不破壞既有居民資料。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::voxel_village::Plot;

// ── 居民圈院（純幾何）────────────────────────────────────────────────────────────

/// 房門所在的 footprint 邊；House v1 的正面固定是南側（+z）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DoorSide { North, South, West, East }

/// 房屋在 x/z 平面的含界矩形。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HouseFootprint {
    pub min_x: i32,
    pub max_x: i32,
    pub min_z: i32,
    pub max_z: i32,
}

/// 一根籬笆的世界座標。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FenceCell { pub x: i32, pub y: i32, pub z: i32 }

/// footprint 外擴兩格的矩形環；門側中央留相鄰兩格作院門。
/// `surface_y` 可注入不同坡度，`occupied` 讓道路／建物等既有方塊原地保留。
pub fn fence_cells(
    fp: HouseFootprint,
    door: DoorSide,
    mut surface_y: impl FnMut(i32, i32) -> i32,
    mut occupied: impl FnMut(i32, i32, i32) -> bool,
) -> Vec<FenceCell> {
    let (x0, x1, z0, z1) = (fp.min_x - 2, fp.max_x + 2, fp.min_z - 2, fp.max_z + 2);
    let gate_x = (fp.min_x + fp.max_x) / 2;
    let gate_z = (fp.min_z + fp.max_z) / 2;
    let is_gate = |x, z| match door {
        DoorSide::North => z == z0 && (x == gate_x || x == gate_x + 1),
        DoorSide::South => z == z1 && (x == gate_x || x == gate_x + 1),
        DoorSide::West => x == x0 && (z == gate_z || z == gate_z + 1),
        DoorSide::East => x == x1 && (z == gate_z || z == gate_z + 1),
    };
    let mut out = Vec::new();
    for x in x0..=x1 {
        for z in z0..=z1 {
            if !(x == x0 || x == x1 || z == z0 || z == z1) || is_gate(x, z) { continue; }
            let y = surface_y(x, z);
            if !occupied(x, y, z) { out.push(FenceCell { x, y, z }); }
        }
    }
    out
}

/// 環上已有至少一半（且至少四根）即視為已圈院；避免週期掃描重複疊放。
pub fn fence_ring_present(existing: usize, ring_len: usize) -> bool {
    ring_len > 0 && existing >= (ring_len / 2).max(4)
}

/// 院子柴堆填充格——貼在圍籬環內側、院門對側的兩個背側角落。yard 夾層（房屋牆體與圍籬
/// 之間）只有 1 格寬，背側角落各恰好 1 格可用，天然避開院門走道與房屋牆體，不必額外碰撞判定。
/// 只在已圈院的家才會被呼叫（呼叫端先驗 `fence_ring_present`）——柴堆是圈院後的下一刀，
/// 敘事順序接續「先圈院、才擺柴堆」。
pub fn yard_clutter_cells(fp: HouseFootprint, door: DoorSide) -> [(i32, i32); 2] {
    match door {
        DoorSide::South => [(fp.min_x - 1, fp.min_z - 1), (fp.max_x + 1, fp.min_z - 1)],
        DoorSide::North => [(fp.min_x - 1, fp.max_z + 1), (fp.max_x + 1, fp.max_z + 1)],
        DoorSide::East => [(fp.min_x - 1, fp.min_z - 1), (fp.min_x - 1, fp.max_z + 1)],
        DoorSide::West => [(fp.max_x + 1, fp.min_z - 1), (fp.max_x + 1, fp.max_z + 1)],
    }
}

/// 聚落歸屬落地路徑（`data/` 已 gitignore）。
pub const SETTLE_PATH: &str = "data/voxel_settlements.jsonl";

/// 主村的聚落 id（沒有任何記錄的居民都屬於主村——向後相容的預設）。
pub const MAIN_SETTLEMENT: u64 = 0;

/// 殖民地 seq → 聚落 id（錯開 0，讓「沒記錄＝主村」永遠成立）。
pub fn colony_settlement_id(colony_seq: u64) -> u64 {
    colony_seq + 1
}

/// 聚落 id → 殖民地 seq（主村（0）回 None）。
pub fn settlement_colony_seq(settlement: u64) -> Option<u64> {
    settlement.checked_sub(1)
}

// ── 聚落歸屬（持久化單位 + store）────────────────────────────────────────────────────

/// 一筆聚落歸屬記錄（jsonl 落地單位）：某居民從此屬於某聚落。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SettlementRecord {
    /// 居民 id（"vox_res_{i}"）。
    pub resident: String,
    /// 聚落 id（0＝主村；殖民地 seq=s → s+1）。
    pub settlement: u64,
    /// 單調遞增序號（越大越新；還原時同居民取最新一筆）。
    pub seq: u64,
}

/// 聚落歸屬 store：居民 → 聚落。純資料，鎖/落地由呼叫端（voxel_ws）管。
#[derive(Default)]
pub struct SettlementStore {
    assign: HashMap<String, u64>,
    next_seq: u64,
}

impl SettlementStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 此居民屬於哪個聚落（沒有記錄＝主村 0，向後相容）。
    pub fn settlement_of(&self, resident: &str) -> u64 {
        self.assign.get(resident).copied().unwrap_or(MAIN_SETTLEMENT)
    }

    /// 把某居民劃歸某聚落；回傳落地記錄供 append。重複劃歸同聚落＝冪等（仍回新記錄，
    /// 呼叫端自行決定要不要重複 append——正常路徑只在歸屬真的改變時呼叫）。
    pub fn assign(&mut self, resident: &str, settlement: u64) -> SettlementRecord {
        self.assign.insert(resident.to_string(), settlement);
        let rec = SettlementRecord { resident: resident.to_string(), settlement, seq: self.next_seq };
        self.next_seq = self.next_seq.wrapping_add(1);
        rec
    }

    /// 某聚落目前有哪些（明確劃歸的）居民 id，排序後回傳（確定性）。
    /// **註**：主村（0）的隱含成員（從沒被劃歸過的居民）不在此列——查主村人口請用
    /// 總人口減去 `nonmain_assigned().len()`。
    pub fn residents_of(&self, settlement: u64) -> Vec<String> {
        let mut out: Vec<String> = self
            .assign
            .iter()
            .filter(|&(_, &s)| s == settlement)
            .map(|(rid, _)| rid.clone())
            .collect();
        out.sort();
        out
    }

    /// 已遷居到任一非主村聚落的居民 id（排序）。供主村都更名單排除：這些人的家在殖民地，
    /// 不在主村地塊上是**正確狀態**，絕不能被主村都更拉回來。
    pub fn nonmain_assigned(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .assign
            .iter()
            .filter(|&(_, &s)| s != MAIN_SETTLEMENT)
            .map(|(rid, _)| rid.clone())
            .collect();
        out.sort();
        out
    }

    /// 從 jsonl 記錄還原（重啟後仍記得誰住哪個聚落）。同居民多筆取 seq 最大（最新）。
    pub fn from_entries(entries: Vec<SettlementRecord>) -> Self {
        let mut es = entries;
        es.sort_by_key(|e| e.seq);
        let mut s = Self::default();
        for e in es {
            if e.seq >= s.next_seq {
                s.next_seq = e.seq.wrapping_add(1);
            }
            s.assign.insert(e.resident, e.settlement);
        }
        s
    }
}

// ── jsonl 持久化（append-only，比照 voxel_village 慣例）─────────────────────────────

/// Append 一筆聚落歸屬記錄。**鐵律**：只在不持任何鎖時呼叫（同步小檔寫、不 await）。
pub fn append_settlement(rec: &SettlementRecord) {
    use std::io::Write;
    let Ok(line) = serde_json::to_string(rec) else { return };
    if let Some(parent) = std::path::Path::new(SETTLE_PATH).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::OpenOptions::new().create(true).append(true).open(SETTLE_PATH) {
        Ok(mut f) => {
            let _ = writeln!(f, "{line}");
        }
        Err(e) => tracing::warn!("無法寫入聚落歸屬 {SETTLE_PATH}: {e}"),
    }
}

/// 載回所有聚落歸屬記錄（啟動時呼叫一次）。檔不存在（舊世界）→ 空（向後相容：全員主村）。
pub fn load_settlements() -> Vec<SettlementRecord> {
    let content = match std::fs::read_to_string(SETTLE_PATH) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    content
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.is_empty() { None } else { serde_json::from_str::<SettlementRecord>(l).ok() }
        })
        .collect()
}

// ── 殖民地小地塊佈局（純函式、確定性）────────────────────────────────────────────────

/// 殖民地小地塊距殖民地中心的距離（格）：奠基殘核廣場半徑 2 ＋ 家園佔地半徑 10 ＋ 餘裕。
/// 取 22 ＝ 沿用主村 `PLOT_STRIDE` 的間距尺度——相鄰兩塊（正向與對角）Chebyshev 距離恰為
/// 22 ≥ 2×[`Plot::HOMESTEAD_HALF`]（20），彼此的建物群不相撞。
pub const COLONY_PLOT_DIST: i32 = 22;

/// 殖民地擴建圈（v3，ROADMAP 1004）距中心的距離：取 [`COLONY_PLOT_DIST`] 的兩倍——
/// 與內圈同款四正向＋四對角佈局在外圍多排一圈，內外兩圈任兩塊的 Chebyshev 距離最小值
/// 仍 ≥ [`COLONY_PLOT_DIST`]（＞ 2×[`Plot::HOMESTEAD_HALF`]），彼此建物群不相撞
/// （見 `colony_rings_never_overlap` 測試窮舉核對）。
pub const COLONY_PLOT_DIST_2: i32 = COLONY_PLOT_DIST * 2;

/// 以 `d` 為距中心距離，鋪出四正向＋四對角共 8 塊地——[`colony_plots`]／
/// [`colony_plots_outer`] 共用的同一份幾何，只是半徑不同。
fn plot_ring(cx: i32, cz: i32, d: i32) -> Vec<Plot> {
    vec![
        Plot { cx: cx + d, cz },
        Plot { cx: cx - d, cz },
        Plot { cx, cz: cz + d },
        Plot { cx, cz: cz - d },
        Plot { cx: cx + d, cz: cz + d },
        Plot { cx: cx + d, cz: cz - d },
        Plot { cx: cx - d, cz: cz + d },
        Plot { cx: cx - d, cz: cz - d },
    ]
}

/// 殖民地中心周圍的小地塊內圈（8 塊：四正向＋四對角，全距中心 [`COLONY_PLOT_DIST`]）。
/// 純函式、確定性（同中心同佈局，重啟一致）。住滿後見 [`colony_plots_outer`]。
pub fn colony_plots(cx: i32, cz: i32) -> Vec<Plot> {
    plot_ring(cx, cz, COLONY_PLOT_DIST)
}

/// 殖民地擴建圈（v3，ROADMAP 1004）：內圈（[`colony_plots`]）全滿時的備援 8 塊地，
/// 距中心 [`COLONY_PLOT_DIST_2`]（內圈的兩倍遠）。純函式、確定性、可窮舉測試。
pub fn colony_plots_outer(cx: i32, cz: i32) -> Vec<Plot> {
    plot_ring(cx, cz, COLONY_PLOT_DIST_2)
}

/// 殖民地「全部」可認領地塊：內圈接擴建圈（16 塊）。供「這座標是不是這座殖民地的
/// 地塊」之類的成員檢查共用（PR #1298 review：只有 [`colony_plots`] 會漏掉外圈居民）；
/// 認領時的內圈優先序仍走 `colony_plots` → 全滿才試 `colony_plots_outer`，見呼叫端。
pub fn colony_all_plots(cx: i32, cz: i32) -> Vec<Plot> {
    let mut all = colony_plots(cx, cz);
    all.extend(colony_plots_outer(cx, cz));
    all
}

// ── 遷居名單（純函式、確定性）────────────────────────────────────────────────────────

/// 待遷居名單：每座殖民地的拓荒者（已換算成居民 id）中，**還住在主村、還沒遷去任何殖民地**的那些。
/// `colony_founders`＝[(殖民地 seq, 拓荒者居民 id 們)]；輸出依（殖民地 seq、id）排序——
/// 穩定順序讓「一次一位」輪流遷居時不跳號。冪等：已定居任一殖民地的不再列（部署後自動收斂）。
///
/// **只挑「仍在主村」的拓荒者**（而非「未劃歸到此殖民地」）是關鍵：同一位居民可能同時是**兩座**
/// 殖民地的拓荒者（湧現系統裡她先後奠基了兩村），若只看「未劃歸到此殖民地」，她定居 A 後就會被
/// 列為「未劃歸 B」→ 遷去 B → 又被列為「未劃歸 A」→ 遷回 A……在兩村間**無限來回搬家**（每輪
/// 都 append 一筆聚落/搬家/記憶記錄，資料檔無界成長）。改看「仍在主村」後：她一旦定居最先掃到
/// 的那座（seq 最小、確定性），`settlement_of != MAIN` 便從此不再列入，thrash 根絕。
pub fn pending_migrations(
    store: &SettlementStore,
    colony_founders: &[(u64, Vec<String>)],
) -> Vec<(String, u64)> {
    let mut out: Vec<(String, u64)> = Vec::new();
    let mut sorted: Vec<&(u64, Vec<String>)> = colony_founders.iter().collect();
    sorted.sort_by_key(|(seq, _)| *seq);
    for (seq, rids) in sorted {
        let sid = colony_settlement_id(*seq);
        let mut rs: Vec<&String> = rids.iter().collect();
        rs.sort();
        for rid in rs {
            // 已遷去自己這座殖民地或**任何一座**殖民地＝已安身，不再列（防身兼多村拓荒者的來回 thrash）。
            if store.settlement_of(rid) == MAIN_SETTLEMENT {
                out.push((rid.clone(), sid));
            }
        }
    }
    out
}

/// 從待遷居名單濾掉正邀居同住中的居民（乙太方界·邀居同住 v1，ROADMAP 972 review 修復）：
/// 家域已由玩家邀居覆寫、房子刻意不拆，拓荒隊若把她一併帶去殖民地，會造成「家在玩家家、
/// 建物錨點卻落在殖民地」的永久分裂（見 review PR #1258）。純函式、可測；`cohabiting`＝
/// 目前正同住中的居民 id 集合（`voxel_cohabit::CohabitStore::cohabiting_residents`）。
pub fn exclude_cohabiting(pending: Vec<(String, u64)>, cohabiting: &[String]) -> Vec<(String, u64)> {
    pending.into_iter().filter(|(rid, _)| !cohabiting.iter().any(|c| c == rid)).collect()
}

// ── 拓荒者候選（純函式、確定性）──────────────────────────────────────────────────────

/// 拓荒者只從主村挑（#1210 震盪 bug 的**源頭端**根治）：從（顯示名, 居民 id）清單裡濾出
/// **目前仍住主村**的顯示名，供 `voxel_colony::pick_founders` 當候選池。
///
/// 為什麼必要：`pending_migrations` 的「住定即止」（上方）只治**遷居端**——已定居殖民地的
/// 居民不再被自動遷居；但若奠基端仍從全體居民挑拓荒者，一位已住殖民地 A 的居民可能被選為
/// 新殖民地 B 的 founder：她永遠不會搬去 B（遷居端擋住了），B 的立村故事卻寫著她「離開
/// 擁擠的主村」——名實不符的幽靈拓荒者。從源頭排除：拓荒隊只從主村人口裡選，敘事與遷居
/// 永遠一致。保序（照輸入順序回傳）＝不打亂 `pick_founders` 依 seq 錯開起點的確定性。
pub fn main_settlement_names(
    store: &SettlementStore,
    residents: &[(String, String)],
) -> Vec<String> {
    residents
        .iter()
        .filter(|(_, rid)| store.settlement_of(rid) == MAIN_SETTLEMENT)
        .map(|(name, _)| name.clone())
        .collect()
}

// ── 殖民者補蓋（純函式、確定性）──────────────────────────────────────────────────────

/// 這位殖民地居民是否「缺一間在自己地塊上的家」（殖民地補蓋的判定核心）。
///
/// **背景（prod 真 bug）**：遠古 GoalRecord 沒有座標欄，重啟 replay 後 `house_of` 失傳
/// （None）——migration_kickoff 把「有小屋但座標失傳」誤判為「沒小屋」走了即刻遷家，
/// 居民的家域落在殖民地空地上、卻從來沒有人替她蓋家。本函式判定：`house` 為 None、
/// 或小屋座標不在她地塊的家園佔地（Chebyshev ≤ `cheb`，取 [`Plot::HOMESTEAD_HALF`]）內
/// → 該補蓋。純函式、可測；呼叫端另需排除「搬家進行中」的居民（她的新家正在蓋）。
pub fn house_missing_near(
    house: Option<(i32, i32, i32)>,
    plot_cx: i32,
    plot_cz: i32,
    cheb: i32,
) -> bool {
    match house {
        None => true,
        Some((hx, _, hz)) => (hx - plot_cx).abs().max((hz - plot_cz).abs()) > cheb,
    }
}

// ── 句式池（面向玩家、i18n 友善集中此處；零 LLM）────────────────────────────────────

/// 殖民者補蓋動工 Feed：遷居時漏了蓋家（遠古資料座標失傳），現在補上。
pub fn repair_build_feed_line(name: &str, colony: &str) -> String {
    format!("{name}在「{colony}」的地塊上動工蓋起自己的新家。")
}

/// 殖民者補蓋動工泡泡。
pub fn repair_build_say_line(colony: &str) -> String {
    format!("在「{colony}」安了家，怎麼能沒有屋子——動工！")
}

/// 遷居動工 Feed：拓荒者動身去自己奠基的村子蓋新家。
pub fn migrate_start_feed_line(name: &str, colony: &str) -> String {
    format!("{name}動身遷居遠方的「{colony}」——要在自己奠基的村子裡蓋起新家。")
}

/// 遷居動工泡泡。
pub fn migrate_start_say_line(colony: &str) -> String {
    format!("「{colony}」在等我——我要搬過去，在那裡生活！")
}

/// 遷居完成 Feed：正式定居殖民地。
pub fn migrate_done_feed_line(name: &str, colony: &str) -> String {
    format!("{name}正式遷居「{colony}」——新家的燈在遠方的村落亮了起來。")
}

/// 遷居完成泡泡。
pub fn migrate_done_say_line(colony: &str) -> String {
    format!("從今天起，我就是「{colony}」的居民了！")
}

/// 遷居完成記憶摘要（記在殖民地記憶分類下，日記/回想可引用）。
pub fn migrate_memory_summary(colony: &str) -> String {
    format!("我正式遷居到「{colony}」，在那裡蓋起了自己的新家——第二座村從此有人真正住著。")
}

/// 主村想念 Feed（療癒調性：搬家是新生活，留下的人惦記著）。
pub fn village_miss_feed_line(name: &str, colony: &str) -> String {
    format!("主村的大家聊起了遠方的{name}——聽說在「{colony}」過得很好，大家都惦記著。")
}

/// 玩家發現殖民地時，若已有人定居，附在立村故事後的一句現居人口。
pub fn colony_population_line(n: usize) -> String {
    format!("如今有 {n} 位居民在此定居生活。")
}

// ── 聚落中心一般化（v2：夜間庇護／挖掘紀律共用）───────────────────────────────────────

/// 給定一個點，回傳**最近**一個聚落中心（主村或任一殖民地）。純函式、確定性、零 IO——
/// 呼叫端自行組裝主村中心（`Option`，極舊/乾淨世界可能尚未釘死）與殖民地中心列表。
/// 兩者皆空 → `None`（不設防，呼叫端自行決定要不要 fallback）。
/// 用途：把「離村莊多遠」的判定（夜間暗影庇護、居民自主開挖離村禁區）從「只認主村」
/// 泛化成「離最近聚落多遠」，殖民地不再是保護死角。
pub fn nearest_settlement_center(
    x: f32,
    z: f32,
    main: Option<(i32, i32)>,
    colonies: &[(i32, i32)],
) -> Option<(i32, i32)> {
    main.into_iter()
        .chain(colonies.iter().copied())
        .min_by(|a, b| {
            let da = (x - a.0 as f32).powi(2) + (z - a.1 as f32).powi(2);
            let db = (x - b.0 as f32).powi(2) + (z - b.1 as f32).powi(2);
            da.total_cmp(&db)
        })
}

// ── 單元測試 ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settlement_id_mapping_roundtrip() {
        // 主村永遠是 0；殖民地 seq 錯開 0。
        assert_eq!(colony_settlement_id(0), 1);
        assert_eq!(settlement_colony_seq(1), Some(0));
        assert_eq!(settlement_colony_seq(MAIN_SETTLEMENT), None, "主村不是殖民地");
        for seq in [0u64, 3, 7] {
            assert_eq!(settlement_colony_seq(colony_settlement_id(seq)), Some(seq));
        }
    }

    #[test]
    fn store_defaults_to_main_settlement() {
        let s = SettlementStore::new();
        assert_eq!(s.settlement_of("vox_res_0"), MAIN_SETTLEMENT, "沒有記錄＝主村（向後相容）");
        assert!(s.nonmain_assigned().is_empty());
    }

    #[test]
    fn assign_and_query() {
        let mut s = SettlementStore::new();
        let rec = s.assign("vox_res_0", 1);
        assert_eq!(rec.settlement, 1);
        assert_eq!(s.settlement_of("vox_res_0"), 1);
        assert_eq!(s.residents_of(1), vec!["vox_res_0".to_string()]);
        assert_eq!(s.nonmain_assigned(), vec!["vox_res_0".to_string()]);
        // 再劃歸別的聚落＝覆蓋。
        s.assign("vox_res_0", 2);
        assert_eq!(s.settlement_of("vox_res_0"), 2);
        assert!(s.residents_of(1).is_empty());
    }

    #[test]
    fn from_entries_keeps_latest_per_resident() {
        let entries = vec![
            SettlementRecord { resident: "vox_res_0".into(), settlement: 1, seq: 0 },
            SettlementRecord { resident: "vox_res_1".into(), settlement: 1, seq: 1 },
            SettlementRecord { resident: "vox_res_0".into(), settlement: 2, seq: 2 },
        ];
        let s = SettlementStore::from_entries(entries);
        assert_eq!(s.settlement_of("vox_res_0"), 2, "同居民取最新一筆");
        assert_eq!(s.settlement_of("vox_res_1"), 1);
        // next_seq 接續在最大序號之後（新 assign 不撞號）。
        let mut s = s;
        assert_eq!(s.assign("vox_res_2", 1).seq, 3);
    }

    #[test]
    fn from_entries_empty_is_backward_compatible() {
        let s = SettlementStore::from_entries(vec![]);
        assert_eq!(s.settlement_of("vox_res_0"), MAIN_SETTLEMENT);
    }

    #[test]
    fn colony_plots_are_deterministic_and_apart() {
        let a = colony_plots(484, 173);
        assert_eq!(a, colony_plots(484, 173), "同中心同佈局（重啟一致）");
        assert_eq!(a.len(), 8, "內圈每座殖民地 8 塊小地塊");
        // 彼此不重疊（用主村同一套「家園佔地」重疊判定）。
        for i in 0..a.len() {
            for j in (i + 1)..a.len() {
                assert!(!a[i].overlaps(&a[j]), "殖民地小地塊 {i} 與 {j} 重疊：{:?} vs {:?}", a[i], a[j]);
            }
        }
        // 每塊距中心恰為 COLONY_PLOT_DIST（Chebyshev），家園佔地不侵入奠基殘核廣場（半徑 2）。
        for p in &a {
            let cheb = (p.cx - 484).abs().max((p.cz - 173).abs());
            assert_eq!(cheb, COLONY_PLOT_DIST);
            assert!(cheb - Plot::HOMESTEAD_HALF > crate::voxel_colony::NUCLEUS_PLAZA_RADIUS,
                "家園佔地不得侵入奠基殘核廣場");
        }
    }

    #[test]
    fn colony_plots_outer_are_deterministic_and_farther() {
        let outer = colony_plots_outer(484, 173);
        assert_eq!(outer, colony_plots_outer(484, 173), "同中心同佈局（重啟一致）");
        assert_eq!(outer.len(), 8, "擴建圈也是 8 塊小地塊（合計 16 戶上限）");
        for p in &outer {
            let cheb = (p.cx - 484).abs().max((p.cz - 173).abs());
            assert_eq!(cheb, COLONY_PLOT_DIST_2, "擴建圈距中心恰為內圈的兩倍遠");
        }
        // 擴建圈內部彼此也不重疊。
        for i in 0..outer.len() {
            for j in (i + 1)..outer.len() {
                assert!(!outer[i].overlaps(&outer[j]), "擴建圈小地塊 {i} 與 {j} 重疊");
            }
        }
    }

    #[test]
    fn colony_rings_never_overlap() {
        // 內圈住滿才會用到擴建圈——兩圈的地塊必須彼此不相撞，建物群才不會互相侵入。
        let inner = colony_plots(0, 0);
        let outer = colony_plots_outer(0, 0);
        for p in &inner {
            for q in &outer {
                assert!(!p.overlaps(q), "內圈 {p:?} 與擴建圈 {q:?} 重疊");
            }
        }
    }

    #[test]
    fn pending_migrations_lists_unassigned_founders_sorted() {
        let mut store = SettlementStore::new();
        let founders = vec![
            (1u64, vec!["vox_res_3".to_string(), "vox_res_1".to_string()]),
            (0u64, vec!["vox_res_2".to_string(), "vox_res_0".to_string()]),
        ];
        let pending = pending_migrations(&store, &founders);
        // 依（殖民地 seq、居民 id）排序：先 seq 0 的兩位（id 升冪）、再 seq 1 的兩位。
        assert_eq!(
            pending,
            vec![
                ("vox_res_0".to_string(), 1),
                ("vox_res_2".to_string(), 1),
                ("vox_res_1".to_string(), 2),
                ("vox_res_3".to_string(), 2),
            ]
        );
        // 劃歸一位後冪等收斂：她不再列入。
        store.assign("vox_res_0", 1);
        let pending = pending_migrations(&store, &founders);
        assert!(!pending.iter().any(|(rid, _)| rid == "vox_res_0"));
        assert_eq!(pending.len(), 3);
        // 全劃歸 → 空（部署後自動收斂、不重複遷）。
        store.assign("vox_res_2", 1);
        store.assign("vox_res_1", 2);
        store.assign("vox_res_3", 2);
        assert!(pending_migrations(&store, &founders).is_empty());
    }

    #[test]
    fn exclude_cohabiting_filters_out_only_named_residents() {
        // 回歸（review PR #1258）：同住中的居民不該被拓荒隊一併帶去殖民地，否則家域
        // （已由玩家邀居覆寫）與建物錨點（拓荒隊蓋在殖民地）永久分裂。
        let pending = vec![
            ("vox_res_0".to_string(), 1u64),
            ("vox_res_1".to_string(), 1u64),
            ("vox_res_2".to_string(), 2u64),
        ];
        let cohabiting = vec!["vox_res_1".to_string()];
        let filtered = exclude_cohabiting(pending, &cohabiting);
        assert_eq!(
            filtered,
            vec![("vox_res_0".to_string(), 1u64), ("vox_res_2".to_string(), 2u64)]
        );
        // 沒有人同住 → 原樣通過。
        let pending2 = vec![("vox_res_3".to_string(), 1u64)];
        assert_eq!(exclude_cohabiting(pending2.clone(), &[]), pending2);
    }

    #[test]
    fn dual_colony_founder_does_not_thrash_between_settlements() {
        // 回歸：同一位居民同時是兩座殖民地的拓荒者（湧現系統裡她先後奠基了兩村）。
        // 修復前：她定居 A 後會被判「未劃歸 B」→ 遷 B → 又「未劃歸 A」→ 遷回 A，每輪來回。
        let mut store = SettlementStore::new();
        let founders = vec![
            (0u64, vec!["vox_res_1".to_string()]), // 風禾屯 → sid 1
            (1u64, vec!["vox_res_1".to_string()]), // 草浪屯 → sid 2（同一位拓荒者）
        ];
        // 起初在主村：兩座殖民地都把她列為待遷（migration_kickoff 一次只遷一位，取最先掃到的）。
        let pending = pending_migrations(&store, &founders);
        assert_eq!(pending, vec![("vox_res_1".to_string(), 1), ("vox_res_1".to_string(), 2)]);
        // 她定居最先掃到的風禾屯（sid 1）後：不再被列入待遷——**絕不**被草浪屯（sid 2）拉走。
        store.assign("vox_res_1", 1);
        assert!(
            pending_migrations(&store, &founders).is_empty(),
            "身兼兩村拓荒者、已定居其一者不該再被另一村列為待遷（否則兩村間無限來回搬家）"
        );
        // 重掃冪等（kickoff 每 30 秒掃一輪）：結果穩定為空，永不再啟動遷居。
        assert!(pending_migrations(&store, &founders).is_empty(), "住定即止，重掃冪等");
        // 即便（歷史震盪資料 last-wins 後）她被劃在草浪屯（sid 2），風禾屯也不再拉回——
        // 住**任何**殖民地都算安身，兩端都收斂。
        store.assign("vox_res_1", 2);
        assert!(pending_migrations(&store, &founders).is_empty());
    }

    #[test]
    fn main_settlement_names_excludes_colony_residents() {
        // 源頭端（#1210 修②）：拓荒者候選池只留主村居民——已定居殖民地者不再入選新村拓荒隊。
        let mut store = SettlementStore::new();
        let residents: Vec<(String, String)> = [
            ("露娜", "vox_res_0"),
            ("諾娃", "vox_res_1"),
            ("賽勒", "vox_res_2"),
        ]
        .iter()
        .map(|(n, r)| (n.to_string(), r.to_string()))
        .collect();
        // 全員在主村：全數入選、保序（不打亂 pick_founders 依 seq 錯開起點的確定性）。
        assert_eq!(
            main_settlement_names(&store, &residents),
            vec!["露娜".to_string(), "諾娃".to_string(), "賽勒".to_string()]
        );
        // 諾娃已定居草浪屯（sid 2）：她不再是新殖民地的拓荒者候選——不會再有
        // 「立村故事寫她離開主村、人卻永遠不搬過去」的幽靈拓荒者。
        store.assign("vox_res_1", 2);
        assert_eq!(
            main_settlement_names(&store, &residents),
            vec!["露娜".to_string(), "賽勒".to_string()]
        );
        // 全員都遷走（理論極端）：候選池空——呼叫端該跳過本輪奠基，而不是奠一座沒人的村。
        store.assign("vox_res_0", 1);
        store.assign("vox_res_2", 1);
        assert!(main_settlement_names(&store, &residents).is_empty());
    }

    #[test]
    fn house_missing_near_detects_homeless_colonists() {
        let cheb = Plot::HOMESTEAD_HALF;
        // 遠古資料座標失傳（house_of None）＝缺家 → 該補蓋（prod 四位的實況）。
        assert!(house_missing_near(None, 469, 173, cheb));
        // 小屋就在地塊家園佔地內（含邊界）＝有家 → 不補（冪等：補蓋完成後永久跳過）。
        assert!(!house_missing_near(Some((469, 8, 173)), 469, 173, cheb));
        assert!(!house_missing_near(Some((469 + cheb, 8, 173 - cheb)), 469, 173, cheb));
        // 小屋遠在主村（座標在、但不在殖民地地塊上）＝這裡沒有家 → 該補蓋。
        assert!(house_missing_near(Some((-14, 8, 14)), 469, 173, cheb));
        // 剛超出家園佔地一格 → 也算缺（Chebyshev 邊界明確）。
        assert!(house_missing_near(Some((469 + cheb + 1, 8, 173)), 469, 173, cheb));
    }

    #[test]
    fn repair_lines_mention_name_and_colony() {
        let line = repair_build_feed_line("露娜", "風禾屯");
        assert!(line.contains("露娜") && line.contains("風禾屯"), "實得：{line}");
        assert!(repair_build_say_line("風禾屯").contains("風禾屯"));
    }

    #[test]
    fn feed_lines_mention_name_and_colony() {
        for line in [
            migrate_start_feed_line("露娜", "風禾屯"),
            migrate_done_feed_line("露娜", "風禾屯"),
            village_miss_feed_line("露娜", "風禾屯"),
        ] {
            assert!(line.contains("露娜") && line.contains("風禾屯"), "句子該同時提到人與村：{line}");
        }
        assert!(migrate_start_say_line("風禾屯").contains("風禾屯"));
        assert!(migrate_done_say_line("風禾屯").contains("風禾屯"));
        assert!(migrate_memory_summary("風禾屯").contains("風禾屯"));
        assert!(colony_population_line(2).contains('2'));
    }

    #[test]
    fn settlement_record_serde_roundtrip() {
        let rec = SettlementRecord { resident: "vox_res_0".into(), settlement: 1, seq: 5 };
        let line = serde_json::to_string(&rec).unwrap();
        let back: SettlementRecord = serde_json::from_str(&line).unwrap();
        assert_eq!(rec, back);
    }

    #[test]
    fn nearest_settlement_center_picks_main_when_no_colonies() {
        let got = nearest_settlement_center(3.0, 4.0, Some((0, 0)), &[]);
        assert_eq!(got, Some((0, 0)));
    }

    #[test]
    fn nearest_settlement_center_picks_closer_colony_over_far_main() {
        // 玩家/居民站在風禾屯附近，主村遠在天邊——該回風禾屯中心，不是主村。
        let main = Some((0, 0));
        let colonies = [(500, 500)];
        let got = nearest_settlement_center(501.0, 502.0, main, &colonies);
        assert_eq!(got, Some((500, 500)), "離殖民地近就該保護殖民地，不是死認主村");
    }

    #[test]
    fn nearest_settlement_center_picks_nearest_among_multiple_colonies() {
        let main = Some((0, 0));
        let colonies = [(500, 500), (-500, -500), (10, 10)];
        let got = nearest_settlement_center(12.0, 8.0, main, &colonies);
        assert_eq!(got, Some((10, 10)), "多座殖民地要挑真的最近的一座");
    }

    #[test]
    fn nearest_settlement_center_no_main_falls_back_to_colony() {
        // 極舊/乾淨世界尚未釘死主村中心（None）——仍要能保護已存在的殖民地。
        let got = nearest_settlement_center(500.0, 500.0, None, &[(500, 500)]);
        assert_eq!(got, Some((500, 500)));
    }

    #[test]
    fn nearest_settlement_center_none_when_nothing_known() {
        let got = nearest_settlement_center(0.0, 0.0, None, &[]);
        assert_eq!(got, None, "沒有任何聚落中心時該誠實回 None，不瞎編一個");
    }

    #[test]
    fn nearest_settlement_center_ties_break_deterministically() {
        // 等距時（罕見但非不可能）min_by 取第一個遇到的候選（main 排最前）——確定性、可重現。
        let got = nearest_settlement_center(0.0, 0.0, Some((10, 0)), &[(-10, 0)]);
        assert_eq!(got, Some((10, 0)));
    }

    #[test]
    fn fence_rectangle_and_south_gate_face_door() {
        let fp = HouseFootprint { min_x: -1, max_x: 1, min_z: -1, max_z: 1 };
        let cells = fence_cells(fp, DoorSide::South, |_, _| 9, |_, _, _| false);
        assert_eq!(cells.len(), 22, "7×7 外環 24 格扣院門兩格");
        assert!(!cells.iter().any(|c| c.z == 3 && (c.x == 0 || c.x == 1)));
        assert!(cells.iter().all(|c| c.x == -3 || c.x == 3 || c.z == -3 || c.z == 3));
    }

    #[test]
    fn fence_gate_follows_each_door_side() {
        let fp = HouseFootprint { min_x: 10, max_x: 12, min_z: 20, max_z: 22 };
        for side in [DoorSide::North, DoorSide::South, DoorSide::West, DoorSide::East] {
            let cells = fence_cells(fp, side, |_, _| 7, |_, _, _| false);
            assert_eq!(cells.len(), 22, "每個朝向都只留兩格缺口：{side:?}");
        }
    }

    #[test]
    fn fence_follows_slope_and_skips_occupied_cells() {
        let fp = HouseFootprint { min_x: 0, max_x: 2, min_z: 0, max_z: 2 };
        let cells = fence_cells(fp, DoorSide::South, |x, z| 8 + x - z, |x, _, z| x == -2 && z == 0);
        assert!(!cells.iter().any(|c| c.x == -2 && c.z == 0), "路／建物占格必須保留");
        assert!(cells.iter().all(|c| c.y == 8 + c.x - c.z), "每根籬笆各取自己的坡地高度");
    }

    #[test]
    fn fence_presence_is_idempotent() {
        assert!(!fence_ring_present(3, 20));
        assert!(!fence_ring_present(9, 20));
        assert!(fence_ring_present(10, 20));
        assert!(!fence_ring_present(0, 0));
    }

    #[test]
    fn yard_clutter_cells_sit_inside_fence_ring_on_back_side() {
        let fp = HouseFootprint { min_x: -1, max_x: 1, min_z: -1, max_z: 1 };
        let corners = yard_clutter_cells(fp, DoorSide::South);
        // 南門對側＝北側（z 較小那面）；兩角都落在 footprint 外一格、圍籬環（±3）內一格。
        assert!(corners.iter().all(|&(_, z)| z == fp.min_z - 1));
        assert!(corners.iter().all(|&(x, _)| x == fp.min_x - 1 || x == fp.max_x + 1));
        assert!(corners.iter().all(|&(x, z)| x > -3 && x < 3 && z > -3 && z < 3), "必須落在圍籬環內側，不與籬笆本身重疊");
        assert_ne!(corners[0], corners[1], "兩角不重複");
    }

    #[test]
    fn yard_clutter_cells_follow_each_door_side_to_the_back() {
        let fp = HouseFootprint { min_x: 10, max_x: 12, min_z: 20, max_z: 22 };
        for side in [DoorSide::North, DoorSide::South, DoorSide::West, DoorSide::East] {
            let corners = yard_clutter_cells(fp, side);
            assert_ne!(corners[0], corners[1], "每個朝向兩角都不重複：{side:?}");
            let on_back_side = match side {
                DoorSide::South => corners.iter().all(|&(_, z)| z == fp.min_z - 1),
                DoorSide::North => corners.iter().all(|&(_, z)| z == fp.max_z + 1),
                DoorSide::East => corners.iter().all(|&(x, _)| x == fp.min_x - 1),
                DoorSide::West => corners.iter().all(|&(x, _)| x == fp.max_x + 1),
            };
            assert!(on_back_side, "背側角落必須在院門對側：{side:?}");
        }
    }
}
