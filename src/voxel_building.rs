//! 乙太方界居民蓋家系統——渴望化為方塊 v1（ROADMAP 652）。
//!
//! 居民持有心願後（由 `voxel_desires` 管理），本模組把心願分類為「建物類型」，
//! 生成一份依序放置的方塊清單（`BuildPlan`），tick 每 8 秒放一塊——
//! 讓玩家親眼看到 AI 居民把夢想一磚一瓦蓋成真。
//!
//! **純邏輯層**：零 LLM、零鎖、零 IO 外包；`classify_desire` 與 `generate_blocks` 皆純函式。
//! 鎖 / 廣播 / 持久化觸發全在 `voxel_ws.rs`。

use std::collections::{HashMap, VecDeque};

use serde::{Deserialize, Serialize};

use crate::voxel::{block_at, Block, BASE_HEIGHT};

// ── 建物類型 ──────────────────────────────────────────────────────────────────

/// 居民可蓋的建物種類（規則分類，零 LLM）。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum BuildKind {
    /// 小木屋：Wood 牆 + Stone 頂，3×3×3。
    House,
    /// 水井：Stone 框 + Water 中心，3×3×2。
    Well,
    /// 瞭望台：Stone 柱身 + 頂台，3×3×6。
    Tower,
    /// 花圃：Grass 地 + Stone 邊框 + Leaves 中心，3×3×2。
    Garden,
}

impl BuildKind {
    /// 顯示名（繁中，玩家看到的）。
    pub fn display_name(self) -> &'static str {
        match self {
            BuildKind::House => "小木屋",
            BuildKind::Well => "水井",
            BuildKind::Tower => "瞭望台",
            BuildKind::Garden => "花圃",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            BuildKind::House => "house",
            BuildKind::Well => "well",
            BuildKind::Tower => "tower",
            BuildKind::Garden => "garden",
        }
    }
}

/// 依心願文字規則分類建物種類（零 LLM、確定性、可測）。
/// 先比對「長/更具體」關鍵詞，避免短詞提早截斷。
pub fn classify_desire(desire: &str) -> Option<BuildKind> {
    // 先比對長詞
    if desire.contains("瞭望")
        || desire.contains("觀星")
        || desire.contains("高台")
    {
        return Some(BuildKind::Tower);
    }
    if desire.contains("水井") || desire.contains("水池") || desire.contains("水源") {
        return Some(BuildKind::Well);
    }
    if desire.contains("花圃")
        || desire.contains("花園")
        || desire.contains("種花")
        || desire.contains("植物")
    {
        return Some(BuildKind::Garden);
    }
    if desire.contains("小屋")
        || desire.contains("家")
        || desire.contains("房子")
        || desire.contains("房屋")
        || desire.contains("住")
    {
        return Some(BuildKind::House);
    }
    // 單字再比對
    if desire.contains("塔") {
        return Some(BuildKind::Tower);
    }
    if desire.contains("泉") || desire.contains("井") {
        return Some(BuildKind::Well);
    }
    if desire.contains("花") || desire.contains("草") || desire.contains("種") {
        return Some(BuildKind::Garden);
    }
    // 通用建造意圖 → House
    if desire.contains("蓋") || desire.contains("建") || desire.contains("造") {
        return Some(BuildKind::House);
    }
    None
}

// ── 建造計畫 ──────────────────────────────────────────────────────────────────

/// 單一待放方塊（世界絕對座標 + 型別）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BuildBlock {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    /// `Block as u8`（serde 直接存數字，向後相容）。
    pub b: u8,
}

/// 一位居民的建造計畫（jsonl 落地單位）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BuildPlan {
    pub resident: String,
    /// BuildKind::as_str()，字串供 serde 向後相容。
    pub kind: String,
    /// 建物顯示名（玩家可讀）。
    pub kind_name: String,
    /// 建物中心世界座標（生成計畫時釘死，不隨居民移動）。
    pub cx: i32,
    pub cy: i32,
    pub cz: i32,
    /// 剩餘待放方塊（依序；每放一塊 pop_front）。
    pub remaining: VecDeque<BuildBlock>,
    /// 總方塊數（供進度計算；remaining.len() ≤ total）。
    pub total: u32,
    /// 單調遞增序號（越大越新；from_entries 用來取最新一份計畫）。
    pub seq: u64,
}

impl BuildPlan {
    pub fn is_done(&self) -> bool {
        self.remaining.is_empty()
    }

    /// 完成百分比 0..=100。
    pub fn progress_pct(&self) -> u32 {
        if self.total == 0 {
            return 100;
        }
        let placed = self.total.saturating_sub(self.remaining.len() as u32);
        placed * 100 / self.total
    }

    /// 取出下一個待放方塊（就地修改 remaining）。
    pub fn pop_next(&mut self) -> Option<BuildBlock> {
        self.remaining.pop_front()
    }
}

/// 所有居民的建造計畫（每人至多一份 active plan）。
#[derive(Default)]
pub struct BuildStore {
    pub plans: HashMap<String, BuildPlan>,
    next_seq: u64,
}

impl BuildStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn has_plan(&self, resident: &str) -> bool {
        self.plans.contains_key(resident)
    }

    pub fn get_plan_mut(&mut self, resident: &str) -> Option<&mut BuildPlan> {
        self.plans.get_mut(resident)
    }

    /// 新建並插入計畫；回傳 clone 供呼叫端落地 jsonl。
    pub fn new_plan(
        &mut self,
        resident: &str,
        kind: BuildKind,
        cx: i32,
        cy: i32,
        cz: i32,
    ) -> BuildPlan {
        let blocks = generate_blocks(kind, cx, cy, cz);
        let total = blocks.len() as u32;
        let plan = BuildPlan {
            resident: resident.to_string(),
            kind: kind.as_str().to_string(),
            kind_name: kind.display_name().to_string(),
            cx,
            cy,
            cz,
            remaining: blocks.into(),
            total,
            seq: self.next_seq,
        };
        self.next_seq += 1;
        self.plans.insert(resident.to_string(), plan.clone());
        plan
    }

    /// 若某居民計畫已完成（remaining 空），移除之。
    pub fn remove_if_done(&mut self, resident: &str) {
        if self.plans.get(resident).map_or(false, |p| p.is_done()) {
            self.plans.remove(resident);
        }
    }

    /// 從 jsonl 記錄還原（重啟後繼續未完成的建造）。同居民多筆取 seq 最大（最新）。
    pub fn from_entries(entries: Vec<BuildPlan>) -> Self {
        let mut s = Self::default();
        for e in entries {
            if e.seq >= s.next_seq {
                s.next_seq = e.seq.wrapping_add(1);
            }
            if e.is_done() {
                continue; // 已完成不需載回
            }
            let keep = s
                .plans
                .get(&e.resident)
                .map_or(true, |existing| e.seq > existing.seq);
            if keep {
                s.plans.insert(e.resident.clone(), e);
            }
        }
        s
    }
}

// ── 地表 y 計算 ────────────────────────────────────────────────────────────────

/// 在 (wx, wz) 找程序生成地形的表面 y（最高固體方塊的正上方一格）。
/// 掃描範圍 BASE_HEIGHT ± 12，足以涵蓋正常起伏地形；找不到回安全保底值。
pub fn surface_y(wx: i32, wz: i32) -> i32 {
    let hi = BASE_HEIGHT + 8;
    let lo = BASE_HEIGHT - 8;
    for y in (lo..=hi).rev() {
        if block_at(wx, y, wz).is_solid() {
            return y + 1; // 地面正上方（站立位置）
        }
    }
    BASE_HEIGHT + 1 // 保底（不應觸及）
}

/// 依居民 index（0..4）決定建造錨點偏移方向，讓 4 位居民朝不同方位蓋。
/// 偏移距離固定 6 方塊，確保不與出生點重疊。
pub fn build_anchor_offset(resident_idx: usize) -> (i32, i32) {
    match resident_idx % 4 {
        0 => (6, 0),
        1 => (0, 6),
        2 => (-6, 0),
        _ => (0, -6),
    }
}

// ── 建物方塊生成（純函式，可測）────────────────────────────────────────────────

/// 生成建物的方塊清單（從底層往上，讓 tick 逐塊放置時玩家看到「由下往上長出」）。
fn generate_blocks(kind: BuildKind, cx: i32, cy: i32, cz: i32) -> Vec<BuildBlock> {
    let mut out = Vec::new();

    let add = |out: &mut Vec<BuildBlock>, x: i32, y: i32, z: i32, b: Block| {
        out.push(BuildBlock { x, y, z, b: b as u8 });
    };

    match kind {
        BuildKind::House => {
            // 地板（cy-1 層，3×3 Wood）——替換地表方塊讓地基清晰
            for dx in -1i32..=1 {
                for dz in -1i32..=1 {
                    add(&mut out, cx + dx, cy - 1, cz + dz, Block::Wood);
                }
            }
            // 牆壁 2 層（只邊框，中心空，Wood）
            for layer in 0..2 {
                let y = cy + layer;
                for dx in -1i32..=1 {
                    for dz in -1i32..=1 {
                        if dx.abs() == 1 || dz.abs() == 1 {
                            add(&mut out, cx + dx, y, cz + dz, Block::Wood);
                        }
                    }
                }
            }
            // 屋頂（cy+2 層，3×3 Stone 實心）
            for dx in -1i32..=1 {
                for dz in -1i32..=1 {
                    add(&mut out, cx + dx, cy + 2, cz + dz, Block::Stone);
                }
            }
            // 共 9 + 8 + 8 + 9 = 34 塊
        }

        BuildKind::Well => {
            // 底圈（cy-1 層，3×3 Stone 外框，中心空）
            for dx in -1i32..=1 {
                for dz in -1i32..=1 {
                    if dx.abs() == 1 || dz.abs() == 1 {
                        add(&mut out, cx + dx, cy - 1, cz + dz, Block::Stone);
                    }
                }
            }
            // 水（中心 cy-1）
            add(&mut out, cx, cy - 1, cz, Block::Water);
            // 井壁（cy 層，同外框）
            for dx in -1i32..=1 {
                for dz in -1i32..=1 {
                    if dx.abs() == 1 || dz.abs() == 1 {
                        add(&mut out, cx + dx, cy, cz + dz, Block::Stone);
                    }
                }
            }
            // 四角頂柱（Wood，作為井架感）
            for &(dx, dz) in &[(-1i32, -1i32), (-1, 1), (1, -1), (1, 1)] {
                add(&mut out, cx + dx, cy + 1, cz + dz, Block::Wood);
            }
            // 共 8 + 1 + 8 + 4 = 21 塊
        }

        BuildKind::Tower => {
            // 地基（cy-1 層，3×3 Stone 實心）
            for dx in -1i32..=1 {
                for dz in -1i32..=1 {
                    add(&mut out, cx + dx, cy - 1, cz + dz, Block::Stone);
                }
            }
            // 塔身 4 層（只邊框，Stone，中心可穿行）
            for layer in 0..4 {
                let y = cy + layer;
                for dx in -1i32..=1 {
                    for dz in -1i32..=1 {
                        if dx.abs() == 1 || dz.abs() == 1 {
                            add(&mut out, cx + dx, y, cz + dz, Block::Stone);
                        }
                    }
                }
            }
            // 瞭望台頂（cy+4 層，3×3 Stone 實心）
            for dx in -1i32..=1 {
                for dz in -1i32..=1 {
                    add(&mut out, cx + dx, cy + 4, cz + dz, Block::Stone);
                }
            }
            // 共 9 + 8×4 + 9 = 50 塊
        }

        BuildKind::Garden => {
            // 草地底（cy-1 層，3×3 Grass）
            for dx in -1i32..=1 {
                for dz in -1i32..=1 {
                    add(&mut out, cx + dx, cy - 1, cz + dz, Block::Grass);
                }
            }
            // 花壇邊框（cy 層，3×3 外框，Stone）
            for dx in -1i32..=1 {
                for dz in -1i32..=1 {
                    if dx.abs() == 1 || dz.abs() == 1 {
                        add(&mut out, cx + dx, cy, cz + dz, Block::Stone);
                    }
                }
            }
            // 中心裝飾（Leaves，象徵花木）
            add(&mut out, cx, cy, cz, Block::Leaves);
            // 共 9 + 8 + 1 = 18 塊
        }
    }

    out
}

// ── 居民建造台詞（純函式，零 LLM）────────────────────────────────────────────

/// 生成居民在建造不同階段冒泡的台詞（進度百分比驅動）。
pub fn build_say_line(kind_name: &str, progress_pct: u32) -> String {
    if progress_pct == 0 {
        format!("我要開始蓋{}了！", kind_name)
    } else if progress_pct < 50 {
        format!("{}慢慢成形了……", kind_name)
    } else if progress_pct < 95 {
        format!("{}快蓋好了！", kind_name)
    } else {
        format!("{}蓋好了！✨", kind_name)
    }
}

// ── jsonl 持久化 ──────────────────────────────────────────────────────────────

/// 建造計畫落地路徑（`data/` 已 gitignore）。
const VOXEL_BUILDS_PATH: &str = "data/voxel_builds.jsonl";

/// Append 一筆計畫快照到 jsonl（每次放塊後更新 remaining）。
/// **鐵律**：只在不持任何鎖的情境呼叫（小檔同步寫，不 await）。
pub fn append_build(plan: &BuildPlan) {
    if let Ok(line) = serde_json::to_string(plan) {
        write_line(VOXEL_BUILDS_PATH, &line);
    }
}

/// 載回所有建造計畫記錄（伺服器啟動時呼叫一次）。檔不存在 / 壞行皆容忍。
pub fn load_builds() -> Vec<BuildPlan> {
    read_lines(VOXEL_BUILDS_PATH)
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
        Err(e) => tracing::warn!("無法寫入居民建造記錄 {path}: {e}"),
    }
}

fn read_lines(path: &str) -> Vec<BuildPlan> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    content
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.is_empty() { None } else { serde_json::from_str::<BuildPlan>(l).ok() }
        })
        .collect()
}

// ── 單元測試 ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── classify_desire 純函式 ────────────────────────────────────────────────

    #[test]
    fn classify_house_keywords() {
        assert_eq!(classify_desire("我想蓋一間小屋"), Some(BuildKind::House));
        assert_eq!(classify_desire("我想有一個家"), Some(BuildKind::House));
        assert_eq!(classify_desire("我想建一棟房子"), Some(BuildKind::House));
        assert_eq!(classify_desire("我想住在這裡"), Some(BuildKind::House));
    }

    #[test]
    fn classify_well_keywords() {
        assert_eq!(classify_desire("我想蓋一口水井"), Some(BuildKind::Well));
        assert_eq!(classify_desire("我想要一個水池"), Some(BuildKind::Well));
        assert_eq!(classify_desire("我夢想有個清泉"), Some(BuildKind::Well));
    }

    #[test]
    fn classify_tower_keywords() {
        assert_eq!(classify_desire("我想蓋一座塔"), Some(BuildKind::Tower));
        assert_eq!(classify_desire("我想建一個瞭望台"), Some(BuildKind::Tower));
        assert_eq!(classify_desire("我想在觀星台看星"), Some(BuildKind::Tower));
    }

    #[test]
    fn classify_garden_keywords() {
        assert_eq!(classify_desire("我想種花"), Some(BuildKind::Garden));
        assert_eq!(classify_desire("我想有個花圃"), Some(BuildKind::Garden));
        assert_eq!(classify_desire("我想在花園裡種草"), Some(BuildKind::Garden));
    }

    #[test]
    fn classify_generic_build_falls_to_house() {
        assert_eq!(classify_desire("我想蓋些什麼"), Some(BuildKind::House));
        assert_eq!(classify_desire("我想建一個東西"), Some(BuildKind::House));
    }

    #[test]
    fn classify_no_match_returns_none() {
        assert!(classify_desire("我想和旅人聊天").is_none());
        assert!(classify_desire("我想學習新事物").is_none());
        assert!(classify_desire("").is_none());
    }

    #[test]
    fn classify_longer_keyword_wins_over_shorter() {
        // 「瞭望台」應比單字「塔」更優先比對
        assert_eq!(classify_desire("我想建一個瞭望台"), Some(BuildKind::Tower));
        // 「水井」應比「井」更優先
        assert_eq!(classify_desire("我想要一口水井"), Some(BuildKind::Well));
    }

    // ── generate_blocks 方塊數 ────────────────────────────────────────────────

    #[test]
    fn house_block_count() {
        let blocks = generate_blocks(BuildKind::House, 0, 5, 0);
        // 地板 9 + 牆 8+8 + 屋頂 9 = 34
        assert_eq!(blocks.len(), 34);
    }

    #[test]
    fn well_block_count() {
        let blocks = generate_blocks(BuildKind::Well, 0, 5, 0);
        // 底圈 8 + 水 1 + 井壁 8 + 角柱 4 = 21
        assert_eq!(blocks.len(), 21);
    }

    #[test]
    fn tower_block_count() {
        let blocks = generate_blocks(BuildKind::Tower, 0, 5, 0);
        // 地基 9 + 塔身 8×4 + 頂台 9 = 50
        assert_eq!(blocks.len(), 50);
    }

    #[test]
    fn garden_block_count() {
        let blocks = generate_blocks(BuildKind::Garden, 0, 5, 0);
        // 草地 9 + 邊框 8 + 中心 1 = 18
        assert_eq!(blocks.len(), 18);
    }

    #[test]
    fn all_blocks_have_valid_block_type() {
        for kind in [BuildKind::House, BuildKind::Well, BuildKind::Tower, BuildKind::Garden] {
            let blocks = generate_blocks(kind, 0, 5, 0);
            for bb in &blocks {
                assert!(Block::from_u8(bb.b).is_some(), "無效方塊 id={} 在 {:?}", bb.b, kind);
            }
        }
    }

    // ── BuildStore 純函式 ─────────────────────────────────────────────────────

    #[test]
    fn store_has_plan_after_new() {
        let mut s = BuildStore::new();
        assert!(!s.has_plan("vox_res_0"));
        s.new_plan("vox_res_0", BuildKind::House, 10, 5, 20);
        assert!(s.has_plan("vox_res_0"));
        assert!(!s.has_plan("vox_res_1"));
    }

    #[test]
    fn store_pop_next_reduces_remaining() {
        let mut s = BuildStore::new();
        s.new_plan("vox_res_0", BuildKind::Well, 0, 5, 0);
        let before = s.plans["vox_res_0"].remaining.len();
        let b = s.get_plan_mut("vox_res_0").unwrap().pop_next();
        assert!(b.is_some());
        assert_eq!(s.plans["vox_res_0"].remaining.len(), before - 1);
    }

    #[test]
    fn store_remove_if_done_works() {
        let mut s = BuildStore::new();
        s.new_plan("vox_res_0", BuildKind::Well, 0, 5, 0);
        // drain all blocks
        while s.get_plan_mut("vox_res_0").and_then(|p| p.pop_next()).is_some() {}
        assert!(s.plans["vox_res_0"].is_done());
        s.remove_if_done("vox_res_0");
        assert!(!s.has_plan("vox_res_0"));
    }

    #[test]
    fn from_entries_restores_incomplete_plan() {
        let plan = BuildPlan {
            resident: "vox_res_0".into(),
            kind: "house".into(),
            kind_name: "小木屋".into(),
            cx: 5,
            cy: 3,
            cz: 5,
            remaining: vec![BuildBlock { x: 5, y: 3, z: 5, b: Block::Wood as u8 }].into(),
            total: 34,
            seq: 0,
        };
        let s = BuildStore::from_entries(vec![plan]);
        assert!(s.has_plan("vox_res_0"));
    }

    #[test]
    fn from_entries_skips_done_plan() {
        let plan = BuildPlan {
            resident: "vox_res_0".into(),
            kind: "house".into(),
            kind_name: "小木屋".into(),
            cx: 5,
            cy: 3,
            cz: 5,
            remaining: VecDeque::new(), // done
            total: 34,
            seq: 0,
        };
        let s = BuildStore::from_entries(vec![plan]);
        assert!(!s.has_plan("vox_res_0"), "已完成的計畫不應載回");
    }

    #[test]
    fn from_entries_keeps_latest_seq() {
        let old = BuildPlan {
            resident: "vox_res_0".into(),
            kind: "house".into(),
            kind_name: "小木屋".into(),
            cx: 0,
            cy: 0,
            cz: 0,
            remaining: vec![BuildBlock { x: 0, y: 0, z: 0, b: Block::Wood as u8 }].into(),
            total: 10,
            seq: 0,
        };
        let new = BuildPlan {
            resident: "vox_res_0".into(),
            kind: "tower".into(),
            kind_name: "瞭望台".into(),
            cx: 5,
            cy: 3,
            cz: 5,
            remaining: vec![BuildBlock { x: 5, y: 3, z: 5, b: Block::Stone as u8 }].into(),
            total: 50,
            seq: 5,
        };
        let s = BuildStore::from_entries(vec![old, new]);
        assert_eq!(s.plans["vox_res_0"].kind, "tower", "應保留 seq 較大的計畫");
    }

    // ── build_say_line 純函式 ─────────────────────────────────────────────────

    #[test]
    fn say_line_stages() {
        let line0 = build_say_line("小木屋", 0);
        assert!(line0.contains("開始蓋"), "進度 0 應包含「開始蓋」：{line0}");

        let line30 = build_say_line("小木屋", 30);
        assert!(line30.contains("成形"), "進度 30 應包含「成形」：{line30}");

        let line70 = build_say_line("水井", 70);
        assert!(line70.contains("快蓋好"), "進度 70 應包含「快蓋好」：{line70}");

        let line99 = build_say_line("瞭望台", 99);
        assert!(line99.contains("蓋好了"), "進度 99 應包含「蓋好了」：{line99}");
    }

    // ── surface_y 純函式 ──────────────────────────────────────────────────────

    #[test]
    fn surface_y_above_solid() {
        // 程序地形在 BASE_HEIGHT 附近有固體，surface_y 應回 > 0
        let sy = surface_y(0, 0);
        assert!(sy > 0, "surface_y 應在地表以上");
        // sy-1 應是固體方塊
        assert!(block_at(0, sy - 1, 0).is_solid(), "surface_y-1 應是固體");
        // sy 本身應是空氣（站立處）
        assert!(!block_at(0, sy, 0).is_solid(), "surface_y 本身應是空氣");
    }

    #[test]
    fn build_anchor_offset_covers_four_directions() {
        let offsets: Vec<(i32, i32)> = (0..4).map(build_anchor_offset).collect();
        let mut xs: Vec<i32> = offsets.iter().map(|&(x, _)| x).collect();
        let mut zs: Vec<i32> = offsets.iter().map(|&(_, z)| z).collect();
        xs.sort();
        zs.sort();
        // 四個方向應有 -6, 0, 0, 6 各一
        assert_eq!(xs, vec![-6, 0, 0, 6]);
        assert_eq!(zs, vec![-6, 0, 0, 6]);
    }
}
