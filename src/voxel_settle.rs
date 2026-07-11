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
//! **純邏輯層鐵律**：本檔零 LLM、零鎖、零 async、零世界 IO——聚落歸屬/遷居名單/殖民地
//! 小地塊佈局/句式池全是確定性純函式，方便單元測試釘死。真正動世界（認領、鋪路、搬家 tick）
//! 全在 `voxel_ws.rs`，嚴守短鎖鐵律。
//! **資料安全**：append-only 落地、`#[serde(default)]` 向後相容、絕不破壞既有居民資料。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::voxel_village::Plot;

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

/// 殖民地中心周圍的小地塊環（8 塊：四正向＋四對角，全距中心 [`COLONY_PLOT_DIST`]）。
/// 純函式、確定性（同中心同佈局，重啟一致）。v1 上限 8 戶——殖民地是小村，有界成長。
pub fn colony_plots(cx: i32, cz: i32) -> Vec<Plot> {
    let d = COLONY_PLOT_DIST;
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

// ── 句式池（面向玩家、i18n 友善集中此處；零 LLM）────────────────────────────────────

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
        assert_eq!(a.len(), 8, "v1 每座殖民地 8 塊小地塊");
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
}
