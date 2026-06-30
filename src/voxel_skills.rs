//! 乙太方界·居民技能庫 v1——目標＋記憶驅動的 agency（ROADMAP·居民 agency v1）。
//!
//! **架構分層（必守）**：LLM 腦＝高層「做什麼／為什麼」（慢、便宜、偶爾）；
//! 技能腳本＝低層「怎麼做」（快、免費、確定性、即時）。本模組是**技能腳本側**：
//! 全是零 LLM、零鎖、零 async 的純邏輯——挑目標、找資源、技能狀態機，皆可單元測試。
//! 鎖／廣播／世界寫入／持久化觸發全在 `voxel_ws.rs`。
//!
//! 解決舊版痛點：居民「一直重蓋同一個花圃」——沒記憶「我蓋過了」、沒進展、delta 只在記憶體。
//! v1 給三件事：
//! 1. **目標不重複**：每居民記住「蓋過哪些建物」（持久化），蓋過的種類不再蓋。
//! 2. **目標有進展**：照固定進展序（花圃→小屋→水井→瞭望台）一個個長，蓋完生下一個。
//! 3. **採集技能（技能調用範本）**：找最近資源→走過去→挖→進居民小背包，
//!    把「找目標→走過去→動作」抽成可重用骨架，之後 hunt/trade 照樣長。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::voxel::{self, Block, WorldDelta, BASE_HEIGHT};
use crate::voxel_building::BuildKind;

// ── 蓋造目標進展（不重複 + 有進展）────────────────────────────────────────────

/// 蓋造目標的固定進展順序：花圃 → 小屋 → 水井 → 瞭望台。
/// 居民照此一個個蓋；蓋過就不再蓋同一種（非重複），蓋完一個自然生出下一個（進展感）。
pub const BUILD_PROGRESSION: [BuildKind; 4] = [
    BuildKind::Garden,
    BuildKind::House,
    BuildKind::Well,
    BuildKind::Tower,
];

/// 依「已完成清單」+「玩家種下的心願（可選對應建物）」挑下一個蓋造目標。
/// - 心願對應的建物若尚未蓋過 → 優先蓋它（玩家的話真的有後果）。
/// - 否則照 [`BUILD_PROGRESSION`] 取第一個還沒蓋過的（自主進展，沒玩家也會長）。
/// - 全部蓋過 → `None`（不再重蓋，改去採集／閒晃）。
///
/// 純函式、確定性、零 LLM——這是「不鬼打牆重蓋」的核心保證。
pub fn next_build_goal(done: &[BuildKind], desired: Option<BuildKind>) -> Option<BuildKind> {
    if let Some(d) = desired {
        if !done.contains(&d) {
            return Some(d);
        }
    }
    BUILD_PROGRESSION.iter().copied().find(|k| !done.contains(k))
}

/// 每個建物錨點相對「家域中心」的偏移（依「已蓋幾個」散開成環，避免新建物疊在舊建物上）。
/// 蓋過的數量當序號 → 第 N 個建物落在第 N 個格位 → 家域慢慢長成一片小聚落。純函式、可測。
pub fn build_offset(seq: usize) -> (i32, i32) {
    // 六個彼此相距 ≥ 6 格的格位，足夠 4 種建物 + 餘裕不重疊。
    const SPOTS: [(i32, i32); 6] = [
        (7, 0),
        (0, 7),
        (-7, 0),
        (0, -7),
        (8, 8),
        (-8, -8),
    ];
    SPOTS[seq % SPOTS.len()]
}

// ── 採集技能：資源型別 ─────────────────────────────────────────────────────────

/// 居民可採集的自然資源。
/// **註**：本世界程序地形不長樹，木頭只在有人放置時才有；故 v1 採集鎖定**地表**真實存在的
/// 草皮／沙／泥，居民走到最近的地表方塊挖一塊放進小背包——「她真的在做事」最有感的一刀。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GatherResource {
    Grass,
    Sand,
    Dirt,
    Stone,
}

impl GatherResource {
    /// 對應的方塊型別。
    pub fn block(self) -> Block {
        match self {
            GatherResource::Grass => Block::Grass,
            GatherResource::Sand => Block::Sand,
            GatherResource::Dirt => Block::Dirt,
            GatherResource::Stone => Block::Stone,
        }
    }

    /// 方塊 id（進居民背包用）。
    pub fn block_id(self) -> u8 {
        self.block() as u8
    }

    /// 顯示名（繁中，玩家在 feed/泡泡看到的）。
    pub fn display_name(self) -> &'static str {
        match self {
            GatherResource::Grass => "草皮",
            GatherResource::Sand => "細沙",
            GatherResource::Dirt => "泥土",
            GatherResource::Stone => "石頭",
        }
    }

    /// 由方塊型別反查資源（找到地表方塊後得知採到的是什麼）。地表不可採的回 None。
    pub fn from_block(b: Block) -> Option<GatherResource> {
        match b {
            Block::Grass => Some(GatherResource::Grass),
            Block::Sand => Some(GatherResource::Sand),
            Block::Dirt => Some(GatherResource::Dirt),
            Block::Stone => Some(GatherResource::Stone),
            _ => None,
        }
    }
}

// ── 技能調用骨架（找目標 → 走過去 → 動作）────────────────────────────────────
//
// 這是「技能調用範本」：之後 hunt（找獵物→追→獵）、trade（找對象→走近→交易）都照同一個
// 「鎖定一個世界座標目標 → 朝它走 → 抵達就執行動作 → 收尾」骨架長。voxel_ws 的物理迴圈
// 負責「走」（每 tick step_toward），本結構只存「目標 + 逾時」這些純資料。

/// 找資源的螺旋搜尋最小半徑（從這格起找，讓居民至少走幾步才採到，動作看得見）。
pub const GATHER_MIN_RADIUS: i32 = 4;
/// 找資源的螺旋搜尋最大半徑（找不到就放棄這次採集，不卡死）。
pub const GATHER_MAX_RADIUS: i32 = 16;
/// 視為「走到資源旁、可動手挖」的水平距離（方塊）。
pub const GATHER_REACH: f32 = 2.2;
/// 採集逾時（秒）：走不到目標（地形擋路等）就放棄，避免居民永遠卡在路上。
pub const GATHER_TIMEOUT_SECS: f32 = 25.0;

/// 一次採集任務的狀態（技能實例）。居民身上至多一個 active 採集任務。
#[derive(Clone, Debug, PartialEq)]
pub struct GatherSkill {
    /// 採到的資源型別（鎖定目標時由地表方塊讀出）。
    pub resource: GatherResource,
    /// 目標方塊世界座標。
    pub tx: i32,
    pub ty: i32,
    pub tz: i32,
    /// 剩餘逾時（秒），降到 0 還沒走到就放棄。
    pub timeout: f32,
}

/// 居民閒置時要開始的下一個活動（純規則、零 LLM、確定性、可測）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NextActivity {
    /// 去採集（蓋之前先備料，像在做事）。
    Gather,
    /// 去蓋指定建物（下一個還沒蓋過的目標）。
    Build(BuildKind),
    /// 沒有可蓋目標（全蓋完）——交回呼叫端：偶爾採集，否則閒晃。
    Wander,
}

/// 依「已蓋清單、心願、本輪已採集次數、採集配額」挑下一步活動：
/// - 還有沒蓋過的建物時：先採集 `gather_quota` 次（備料、有在做事的感覺），再蓋。
/// - 沒有可蓋目標（全蓋完）：回 `Wander`，由呼叫端決定偶爾採集或閒晃。
///
/// 這把「目標＋記憶」收斂成一個可測的決策點：永遠不會選到已蓋過的建物。
pub fn choose_activity(
    done_builds: &[BuildKind],
    desired: Option<BuildKind>,
    gathered_since_build: u32,
    gather_quota: u32,
) -> NextActivity {
    match next_build_goal(done_builds, desired) {
        Some(kind) => {
            if gathered_since_build < gather_quota {
                NextActivity::Gather
            } else {
                NextActivity::Build(kind)
            }
        }
        None => NextActivity::Wander,
    }
}

/// 居民是否已走到資源旁、可以動手挖（只看水平距離；垂直由重力處理）。純函式、可測。
pub fn within_gather_reach(rx: f32, rz: f32, tx: i32, tz: i32) -> bool {
    let dx = (tx as f32 + 0.5) - rx;
    let dz = (tz as f32 + 0.5) - rz;
    dx * dx + dz * dz <= GATHER_REACH * GATHER_REACH
}

/// 判斷「挖這塊會不會把自己困住」（修：居民採集別把自己挖坑卡住）。
/// 給定居民腳底所在格 (fx,fy,fz) 與想挖的目標方塊 (tx,ty,tz)，回傳 `true`＝安全可挖。
///
/// 兩條保守規則（確定性、可測）：
/// 1. **別挖自己站的那一柱**（tx==fx && tz==fz）：會抽掉腳下的地，居民直接掉進洞裡。
/// 2. **別挖明顯低於腳底的方塊**（ty < fy-1）：挖低處＝把要走過去的地方掏成坑，
///    一刀刀連起來就成了爬不出的大洞。只採「與腳同層或更高」的地表方塊
///    （草皮／沙在表層最自然）——挖完該柱頂多留一個 1 格深的小坑，居民踏階即可爬出。
pub fn safe_to_dig(fx: i32, fy: i32, fz: i32, tx: i32, ty: i32, tz: i32) -> bool {
    if tx == fx && tz == fz {
        return false;
    }
    if ty < fy - 1 {
        return false;
    }
    true
}

/// 找某 (x,z) 柱的「地表方塊」：由高往低掃 effective_block，回最高一個實心方塊 (y, 型別)。
/// 套 delta overlay → 別人挖過/蓋過也算數。全空（不該發生）回 None。
fn surface_block(world: &WorldDelta, x: i32, z: i32) -> Option<(i32, Block)> {
    let top = BASE_HEIGHT + 14; // 涵蓋正常地形峰值；不必掃到建物高層
    for y in (0..=top).rev() {
        let b = voxel::effective_block_at(world, x, y, z);
        if b.is_solid() {
            return Some((y, b));
        }
    }
    None
}

/// 從 (ox,oz) 螺旋向外找最近一個「可採地表方塊」（草／沙／泥／石，非水），
/// 回傳 (x, 地表y, z, 資源)。從 [`GATHER_MIN_RADIUS`] 起找（讓居民走幾步），
/// 找到 [`GATHER_MAX_RADIUS`] 仍無 → None。純函式（吃 &WorldDelta）、可測。
pub fn find_nearest_resource(
    world: &WorldDelta,
    ox: i32,
    oz: i32,
    max_radius: i32,
) -> Option<(i32, i32, i32, GatherResource)> {
    // 居民站立柱的地表頂 → 推估腳底層（fy），用來剔除「挖了會把自己困住」的目標
    // （腳下那柱、明顯低於腳底的坑底）。找不到站立柱（不該發生）就不過濾、退回原行為。
    let foot_fy = surface_block(world, ox, oz).map(|(y, _)| y + 1);
    for r in GATHER_MIN_RADIUS..=max_radius {
        for dx in -r..=r {
            for dz in -r..=r {
                // 只走當前半徑的「環」邊界，避免重複掃內圈。
                if dx.abs().max(dz.abs()) != r {
                    continue;
                }
                let (x, z) = (ox + dx, oz + dz);
                if let Some((y, b)) = surface_block(world, x, z) {
                    if let Some(res) = GatherResource::from_block(b) {
                        // 只挑「挖了不會把自己困住」的地表（防採集挖坑卡死）。
                        if foot_fy.map_or(true, |fy| safe_to_dig(ox, fy, oz, x, y, z)) {
                            return Some((x, y, z, res));
                        }
                    }
                }
            }
        }
    }
    None
}

// ── 已完成目標 store（持久化：不重複的記憶土壤）──────────────────────────────

/// 一筆「居民完成了某建物」記錄（jsonl 落地單位）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GoalRecord {
    /// 居民系統 id（"vox_res_0"…）。
    pub resident: String,
    /// 完成的建物種類（`BuildKind::as_str()`，字串供 serde 向後相容）。
    pub kind: String,
    /// 單調遞增序號（保留還原順序；亦供去重）。
    pub seq: u64,
}

/// 每居民「已完成建物種類」集合 store。讓 `choose_activity` 永不重選蓋過的種類。
#[derive(Default)]
pub struct GoalStore {
    /// resident id → 已完成的 BuildKind 字串集合（去重）。
    done: HashMap<String, Vec<String>>,
    next_seq: u64,
}

impl GoalStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 由 jsonl 記錄還原（重啟後仍記得蓋過什麼 → 不會重蓋）。
    pub fn from_entries(entries: Vec<GoalRecord>) -> Self {
        let mut s = Self::default();
        for e in entries {
            if e.seq >= s.next_seq {
                s.next_seq = e.seq.wrapping_add(1);
            }
            let v = s.done.entry(e.resident.clone()).or_default();
            if !v.contains(&e.kind) {
                v.push(e.kind);
            }
        }
        s
    }

    /// 此居民是否已蓋過該種建物。
    pub fn is_done(&self, resident: &str, kind: BuildKind) -> bool {
        self.done
            .get(resident)
            .map_or(false, |v| v.iter().any(|k| k == kind.as_str()))
    }

    /// 此居民已完成的建物種類（給 `choose_activity` / `next_build_goal`）。
    pub fn done_kinds(&self, resident: &str) -> Vec<BuildKind> {
        self.done
            .get(resident)
            .map(|v| v.iter().filter_map(|k| BuildKind::from_str(k)).collect())
            .unwrap_or_default()
    }

    /// 此居民已完成的建物數量（當建物錨點散開的序號用）。
    pub fn done_count(&self, resident: &str) -> usize {
        self.done.get(resident).map_or(0, |v| v.len())
    }

    /// 標記某居民完成了某建物；回傳新 record 供呼叫端 append 落地。
    /// 已存在則回 None（不重複落地）。
    pub fn mark_done(&mut self, resident: &str, kind: BuildKind) -> Option<GoalRecord> {
        let v = self.done.entry(resident.to_string()).or_default();
        if v.iter().any(|k| k == kind.as_str()) {
            return None;
        }
        v.push(kind.as_str().to_string());
        let rec = GoalRecord {
            resident: resident.to_string(),
            kind: kind.as_str().to_string(),
            seq: self.next_seq,
        };
        self.next_seq = self.next_seq.wrapping_add(1);
        Some(rec)
    }
}

// ── jsonl 持久化（append-only，比照 voxel_desires/voxel_building）─────────────

/// 已完成目標落地路徑（`data/` 已 gitignore）。
const VOXEL_GOALS_PATH: &str = "data/voxel_goals.jsonl";

/// Append 一筆完成記錄。**鐵律**：只在不持任何鎖時呼叫（同步小檔寫，不 await）。
pub fn append_goal(rec: &GoalRecord) {
    if let Ok(line) = serde_json::to_string(rec) {
        write_line(VOXEL_GOALS_PATH, &line);
    }
}

/// 載回所有完成記錄（啟動時呼叫一次）。檔不存在／壞行皆容忍。
pub fn load_goals() -> Vec<GoalRecord> {
    read_lines(VOXEL_GOALS_PATH)
}

fn write_line(path: &str, line: &str) {
    use std::io::Write;
    if let Some(parent) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::OpenOptions::new().create(true).append(true).open(path) {
        Ok(mut f) => {
            let _ = writeln!(f, "{line}");
        }
        Err(e) => tracing::warn!("無法寫入居民目標記錄 {path}: {e}"),
    }
}

fn read_lines(path: &str) -> Vec<GoalRecord> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    content
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.is_empty() {
                None
            } else {
                serde_json::from_str::<GoalRecord>(l).ok()
            }
        })
        .collect()
}

// ── 單元測試 ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voxel::{height_at, SEA_LEVEL};

    // ── next_build_goal：不重複 + 進展 ────────────────────────────────────────

    #[test]
    fn next_goal_progression_order() {
        // 全新 → 取進展序第一個（花圃）。
        assert_eq!(next_build_goal(&[], None), Some(BuildKind::Garden));
        // 蓋過花圃 → 換小屋（不再蓋花圃）。
        assert_eq!(next_build_goal(&[BuildKind::Garden], None), Some(BuildKind::House));
        // 花圃+小屋 → 水井。
        assert_eq!(
            next_build_goal(&[BuildKind::Garden, BuildKind::House], None),
            Some(BuildKind::Well)
        );
    }

    #[test]
    fn next_goal_none_when_all_done() {
        let all = [BuildKind::Garden, BuildKind::House, BuildKind::Well, BuildKind::Tower];
        assert_eq!(next_build_goal(&all, None), None);
    }

    #[test]
    fn next_goal_desire_takes_priority_if_not_built() {
        // 心願是塔且還沒蓋 → 先蓋塔（玩家的話有後果），蓋過花圃也不影響。
        assert_eq!(
            next_build_goal(&[BuildKind::Garden], Some(BuildKind::Tower)),
            Some(BuildKind::Tower)
        );
    }

    #[test]
    fn next_goal_desire_ignored_if_already_built() {
        // 心願是花圃但已蓋過 → 略過心願，回進展序下一個（小屋）。
        assert_eq!(
            next_build_goal(&[BuildKind::Garden], Some(BuildKind::Garden)),
            Some(BuildKind::House)
        );
    }

    // ── build_offset：散開不重疊 ──────────────────────────────────────────────

    #[test]
    fn build_offsets_are_distinct() {
        let offs: Vec<(i32, i32)> = (0..4).map(build_offset).collect();
        let uniq: std::collections::HashSet<_> = offs.iter().collect();
        assert_eq!(uniq.len(), 4, "前 4 個建物錨點應各不相同（不疊在一起）");
        // 任兩個間距 ≥ 6 格（不重疊 3×3 建物）。
        for i in 0..offs.len() {
            for j in (i + 1)..offs.len() {
                let dx = (offs[i].0 - offs[j].0) as f32;
                let dz = (offs[i].1 - offs[j].1) as f32;
                assert!((dx * dx + dz * dz).sqrt() >= 6.0, "建物間距應 ≥ 6");
            }
        }
    }

    // ── choose_activity：採集配額 → 蓋造 ──────────────────────────────────────

    #[test]
    fn choose_gathers_before_building() {
        // 還沒採滿配額 → 先採集。
        assert_eq!(choose_activity(&[], None, 0, 2), NextActivity::Gather);
        assert_eq!(choose_activity(&[], None, 1, 2), NextActivity::Gather);
        // 採滿配額 → 蓋下一個目標（花圃）。
        assert_eq!(choose_activity(&[], None, 2, 2), NextActivity::Build(BuildKind::Garden));
    }

    #[test]
    fn choose_wander_when_all_built() {
        let all = [BuildKind::Garden, BuildKind::House, BuildKind::Well, BuildKind::Tower];
        // 全蓋完，不論採集次數都回 Wander（不再重蓋）。
        assert_eq!(choose_activity(&all, None, 0, 2), NextActivity::Wander);
        assert_eq!(choose_activity(&all, None, 5, 2), NextActivity::Wander);
    }

    #[test]
    fn choose_build_respects_desire() {
        // 心願塔、採滿配額 → 蓋塔。
        assert_eq!(
            choose_activity(&[], Some(BuildKind::Tower), 2, 2),
            NextActivity::Build(BuildKind::Tower)
        );
    }

    // ── GatherResource ────────────────────────────────────────────────────────

    #[test]
    fn gather_resource_block_roundtrip() {
        for res in [
            GatherResource::Grass,
            GatherResource::Sand,
            GatherResource::Dirt,
            GatherResource::Stone,
        ] {
            assert_eq!(GatherResource::from_block(res.block()), Some(res));
            assert!(!res.display_name().is_empty());
            assert_eq!(res.block_id(), res.block() as u8);
        }
        // 不可採的方塊 → None。
        assert_eq!(GatherResource::from_block(Block::Water), None);
        assert_eq!(GatherResource::from_block(Block::Air), None);
    }

    // ── within_gather_reach ───────────────────────────────────────────────────

    #[test]
    fn within_reach_boundary() {
        // 站在方塊中心正上方 → 在範圍內。
        assert!(within_gather_reach(5.5, 5.5, 5, 5));
        // 遠處 → 不在範圍。
        assert!(!within_gather_reach(0.0, 0.0, 20, 20));
    }

    // ── find_nearest_resource ─────────────────────────────────────────────────

    /// 找一個明顯高於海平面的陸地點。
    fn land_point() -> (i32, i32) {
        for c in 0..3000 {
            if height_at(c, 0) > SEA_LEVEL + 3 {
                return (c, 0);
            }
        }
        (0, 0)
    }

    #[test]
    fn find_nearest_resource_on_land_finds_surface() {
        let world = WorldDelta::new();
        let (ox, oz) = land_point();
        let found = find_nearest_resource(&world, ox, oz, GATHER_MAX_RADIUS);
        assert!(found.is_some(), "陸地上應找得到地表可採資源");
        let (x, y, z, res) = found.unwrap();
        // 找到的應真的是地表（型別與讀到的資源一致）。
        assert_eq!(voxel::effective_block_at(&world, x, y, z), res.block());
        // 距原點至少 GATHER_MIN_RADIUS（讓居民走幾步）。
        let d = (x - ox).abs().max((z - oz).abs());
        assert!(d >= GATHER_MIN_RADIUS, "資源應在最小半徑外：d={d}");
    }

    #[test]
    fn find_nearest_resource_respects_delta() {
        // 放一塊 delta 石頭改寫某柱地表頂 → 驗證 surface_block 走 effective（吃 delta）。
        let world = WorldDelta::new();
        let (ox, oz) = land_point();
        let mut w2 = world.clone();
        let h = height_at(ox + GATHER_MIN_RADIUS, oz);
        voxel::set_block(&mut w2, ox + GATHER_MIN_RADIUS, h + 1, oz, Block::Stone);
        let (_, _, _, res) = find_nearest_resource(&w2, ox, oz, GATHER_MAX_RADIUS).unwrap();
        // 至少能找到某個可採資源（不 panic、有結果）。
        assert!(!res.display_name().is_empty());
    }

    // ── safe_to_dig：採集別把自己挖坑卡住 ────────────────────────────────────

    #[test]
    fn safe_to_dig_rejects_own_column() {
        // 腳下那一柱（同 x,z）→ 不可挖（會抽掉腳下的地）。
        assert!(!safe_to_dig(10, 30, 10, 10, 29, 10));
        assert!(!safe_to_dig(10, 30, 10, 10, 30, 10));
    }

    #[test]
    fn safe_to_dig_rejects_blocks_below_feet() {
        // 目標明顯低於腳底（ty < fy-1）→ 挖坑，不可挖。
        assert!(!safe_to_dig(10, 30, 10, 12, 28, 10)); // 低 2 格
        assert!(!safe_to_dig(10, 30, 10, 12, 25, 10)); // 低 5 格（坑底）
    }

    #[test]
    fn safe_to_dig_allows_same_level_surface() {
        // 平地：腳底 fy，旁邊一柱地表頂在 fy-1（站立柱頂同層）→ 可挖（頂多留 1 格小坑）。
        assert!(safe_to_dig(10, 30, 10, 12, 29, 10));
        // 旁邊一柱比腳高（台階上方）→ 可挖，不會把自己困住。
        assert!(safe_to_dig(10, 30, 10, 12, 30, 10));
        assert!(safe_to_dig(10, 30, 10, 12, 31, 10));
    }

    #[test]
    fn find_nearest_resource_skips_pit_below() {
        // 在站立柱旁挖一個深坑（地表掏到很低）→ find 不該回那個坑底（safe_to_dig 擋下）。
        let world = WorldDelta::new();
        let (ox, oz) = land_point();
        // 在最小半徑環上挑一柱，把它地表往下掏 4 格成坑。
        let (px, pz) = (ox + GATHER_MIN_RADIUS, oz);
        let h = height_at(px, pz);
        let mut w2 = world.clone();
        for dy in 0..4 {
            voxel::set_block(&mut w2, px, h - dy, pz, Block::Air);
        }
        // find 回的目標（若有）必不是那個被掏成坑的柱（其地表已遠低於腳底）。
        if let Some((x, y, z, _)) = find_nearest_resource(&w2, ox, oz, GATHER_MAX_RADIUS) {
            let foot_fy = height_at(ox, oz) + 1;
            assert!(
                safe_to_dig(ox, foot_fy, oz, x, y, z),
                "find 回的目標應通過 safe_to_dig（不挖坑底）：({x},{y},{z}) foot={foot_fy}"
            );
        }
    }

    // ── GoalStore：不重複的記憶 ──────────────────────────────────────────────

    #[test]
    fn goal_store_mark_and_query() {
        let mut s = GoalStore::new();
        assert!(!s.is_done("vox_res_0", BuildKind::Garden));
        let rec = s.mark_done("vox_res_0", BuildKind::Garden);
        assert!(rec.is_some());
        assert!(s.is_done("vox_res_0", BuildKind::Garden));
        assert_eq!(s.done_count("vox_res_0"), 1);
        // 別的居民不受影響。
        assert!(!s.is_done("vox_res_1", BuildKind::Garden));
    }

    #[test]
    fn goal_store_mark_twice_is_idempotent() {
        let mut s = GoalStore::new();
        assert!(s.mark_done("r", BuildKind::Well).is_some());
        // 第二次標記同種 → None（不重複落地），數量不變。
        assert!(s.mark_done("r", BuildKind::Well).is_none());
        assert_eq!(s.done_count("r"), 1);
    }

    #[test]
    fn goal_store_drives_non_repeat_goal() {
        let mut s = GoalStore::new();
        // 蓋完花圃 → done_kinds 含花圃 → next_build_goal 換小屋。
        s.mark_done("r", BuildKind::Garden);
        let done = s.done_kinds("r");
        assert_eq!(next_build_goal(&done, None), Some(BuildKind::House));
    }

    #[test]
    fn goal_store_from_entries_restores() {
        let entries = vec![
            GoalRecord { resident: "r".into(), kind: "garden".into(), seq: 0 },
            GoalRecord { resident: "r".into(), kind: "house".into(), seq: 1 },
            // 重複行：去重。
            GoalRecord { resident: "r".into(), kind: "garden".into(), seq: 2 },
        ];
        let s = GoalStore::from_entries(entries);
        assert!(s.is_done("r", BuildKind::Garden));
        assert!(s.is_done("r", BuildKind::House));
        assert_eq!(s.done_count("r"), 2, "重複種類應去重");
        // 重啟後 next 應跳過已蓋的兩種 → 水井。
        assert_eq!(next_build_goal(&s.done_kinds("r"), None), Some(BuildKind::Well));
    }

    #[test]
    fn goal_store_jsonl_roundtrip() {
        let dir = std::env::temp_dir().join(format!("voxgoal_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("voxel_goals.jsonl");
        let _ = std::fs::remove_file(&path);
        let pstr = path.to_str().unwrap();
        let r1 = GoalRecord { resident: "vox_res_0".into(), kind: "garden".into(), seq: 0 };
        let r2 = GoalRecord { resident: "vox_res_0".into(), kind: "house".into(), seq: 1 };
        write_line(pstr, &serde_json::to_string(&r1).unwrap());
        write_line(pstr, &serde_json::to_string(&r2).unwrap());
        let loaded = read_lines(pstr);
        assert_eq!(loaded.len(), 2);
        let s = GoalStore::from_entries(loaded);
        assert_eq!(s.done_count("vox_res_0"), 2);
        let _ = std::fs::remove_file(&path);
    }
}
