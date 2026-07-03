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

use crate::voxel::{biome_at_voxel, block_at, Block, VoxelBiome, BASE_HEIGHT};

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

    /// 由 `as_str()` 字串反查 BuildKind（從持久化的目標記錄還原）；未知回 None。
    pub fn from_str(s: &str) -> Option<BuildKind> {
        match s {
            "house" => Some(BuildKind::House),
            "well" => Some(BuildKind::Well),
            "tower" => Some(BuildKind::Tower),
            "garden" => Some(BuildKind::Garden),
            _ => None,
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

/// 一句自主禱告（`npc_pray` 產出）要不要提升成持久渴望（純函式、可測）。
///
/// ROADMAP 6「禱告驅動蓋家」：居民每隔一段時間會自主許願（不像玩家聊天的心願，
/// 每次禱告幾乎都會成功產出一句），若照玩家心願路徑無條件覆蓋，會讓多數只是抒發
/// 心情的禱告（如「好想有個慶典熱鬧一下」）頻繁蓋掉真正具體的建造心願——因此**只有
/// 這句禱告本身能分類出具體建物種類時**才值得提升成持久渴望，其餘仍只是浮現又消失
/// 的一句心願泡泡（不覆蓋、不落地）。
pub fn prayer_promotable(prayer: &str) -> bool {
    classify_desire(prayer).is_some()
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
    /// 這是否為「擴建」（基礎四種都蓋完後再蓋的第 2 座）而非首次建造。
    /// 完工時決定要呼叫 `GoalStore::mark_done` 還是 `mark_expansion`。
    /// `#[serde(default)]` 供舊 jsonl 向後相容（舊行沒有這欄，一律視為 `false`）。
    #[serde(default)]
    pub expansion: bool,
    /// 心願真的成真 v1（ROADMAP 720）：若這座建物是某位玩家的話種下的心願所驅動
    /// （非居民自我啟發、非擴建），記下啟發者的玩家顯示名，完工時指名感謝。
    /// `#[serde(default)]` 供舊 jsonl 向後相容（舊行沒有這欄，一律視為 `None`，維持原通用完工行為）。
    #[serde(default)]
    pub inspired_by: Option<String>,
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

    /// ROADMAP 699：玩家協助居民蓋家。若玩家剛放的方塊（世界座標 + 類型）正好等於
    /// 某位居民建造計畫「下一塊待放」，判定為玩家幫了忙——彈掉該塊（居民之後 tick
    /// 不會重放這塊，接著蓋下一塊），回傳 `(resident_id, kind_name)` 供呼叫端道謝。
    /// 找不到符合的計畫回 `None`（多數放置與任何居民無關，屬正常情形）。
    pub fn try_player_help(&mut self, x: i32, y: i32, z: i32, b: u8) -> Option<(String, String)> {
        for (rid, plan) in self.plans.iter_mut() {
            let is_match = plan
                .remaining
                .front()
                .map_or(false, |front| front.x == x && front.y == y && front.z == z && front.b == b);
            if is_match {
                plan.pop_next();
                return Some((rid.clone(), plan.kind_name.clone()));
            }
        }
        None
    }

    /// 新建並插入計畫；回傳 clone 供呼叫端落地 jsonl。
    /// `expansion`：是否為擴建（基礎四種都蓋完後再蓋的第 2 座），影響完工時記錄方式。
    /// `inspired_by`：若這座建物是某位玩家的話種下的心願所驅動，記下玩家名（完工時指名感謝）。
    pub fn new_plan(
        &mut self,
        resident: &str,
        kind: BuildKind,
        cx: i32,
        cy: i32,
        cz: i32,
        expansion: bool,
        inspired_by: Option<String>,
    ) -> BuildPlan {
        // 建築風格依「居民 + 群系（由錨點座標查）+ 錨點座標」確定性決定，讓每間都不同。
        let biome = biome_at_voxel(cx, cz);
        let style = BuildStyle::for_resident(resident, biome, cx, cz);
        let blocks = generate_blocks(kind, cx, cy, cz, &style);
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
            expansion,
            inspired_by,
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

// ── 建築風格：讓同種建物「依居民/群系」各有不同（純函式，確定性，可測）─────────
//
// 建築創作第一刀：此前 `generate_blocks` 對同一 `BuildKind` 永遠吐出一模一樣的方盒——
// 誰蓋幾次都是複製貼上。本層把「牆材質／屋頂形狀材質／尺寸／裝飾」抽成一份由
// 「**誰蓋的（居民 id）＋在哪個群系＋錨點座標**」確定性決定的 [`BuildStyle`]，
// 讓露娜的木屋、諾娃的石頂屋、沙漠居民的沙屋一眼看得出是不同人蓋的不同房子。
//
// **確定性鐵律**：同居民同錨點永遠算出同一份風格（可測、重啟一致、不會這次木下次石）；
// 風格在 `new_plan` 建計畫時算一次、烘進 `remaining` 方塊清單（jsonl 落地的是實際方塊，
// 之後重啟直接 replay，不重算風格）→ 與 `try_player_help` 的逐塊比對完全相容。

/// 門口點綴（每家不同的小細節；皆放在正面外側一格，不動地基/不與牆體重疊）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Decor {
    /// 無點綴（素樸）。
    None,
    /// 門口一盞火把（暖光）。
    Torch,
    /// 門口一畦花圃（草地 + 葉片）。
    Flowerbed,
    /// 門口一根柱（兩格高，牆材質）。
    Pillar,
}

/// 一座建物的樣式（由居民/群系/座標確定性決定）。純資料、無 IO。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BuildStyle {
    /// 牆體主建材（依群系：森林木、沙漠沙、雪原拋石/雪…）。
    pub wall: Block,
    /// 屋頂建材（石/木/葉/拋石）。
    pub roof: Block,
    /// 地板建材（由牆材質衍生）。
    pub floor: Block,
    /// 尖頂：平頂上再疊一層縮小的方塊，成斜頂感。
    pub peaked: bool,
    /// 側牆是否開玻璃窗。
    pub windows: bool,
    /// 牆高（層數，2 或 3）。
    pub wall_h: i32,
    /// 佔地範圍（相對中心，含界）；x_min/z_min 固定 -1，x_max/z_max 由尺寸決定。
    pub x_max: i32,
    pub z_max: i32,
    /// 門口點綴。
    pub decor: Decor,
}

impl BuildStyle {
    pub const X_MIN: i32 = -1;
    pub const Z_MIN: i32 = -1;

    /// 依「居民 id + 群系 + 錨點座標」確定性推導一份樣式（同輸入永遠同輸出）。
    pub fn for_resident(resident: &str, biome: VoxelBiome, cx: i32, cz: i32) -> BuildStyle {
        let h = style_hash(resident, cx, cz);
        let wall = wall_palette(biome)[(h & 1) as usize];
        let roof = ROOF_PALETTE[((h >> 1) & 0b11) as usize];
        let peaked = (h >> 3) & 1 == 1;
        let windows = (h >> 4) & 1 == 1;
        let wall_h = 2 + ((h >> 5) & 1) as i32; // 2 或 3 層
        // 佔地：3×3 / 4×3 / 3×4 / 4×4（小變化，別太大顆拖效能）。
        let (x_max, z_max) = match (h >> 6) & 0b11 {
            0 => (1, 1),
            1 => (2, 1),
            2 => (1, 2),
            _ => (2, 2),
        };
        let decor = match (h >> 8) & 0b11 {
            0 => Decor::None,
            1 => Decor::Torch,
            2 => Decor::Flowerbed,
            _ => Decor::Pillar,
        };
        // 地板由牆材質衍生（木系→木板、沙→沙、其餘→拋石），保持質感一致。
        let floor = match wall {
            Block::Wood | Block::Plank => Block::Plank,
            Block::Sand => Block::Sand,
            _ => Block::SmoothStone,
        };
        BuildStyle { wall, roof, floor, peaked, windows, wall_h, x_max, z_max, decor }
    }
}

/// 依「居民 id + 錨點座標」算出的穩定 64-bit 雜湊（FNV-1a）。
/// 純函式：居民 id 與座標都穩定 → 同居民同錨點永遠同雜湊（重啟一致）。
fn style_hash(resident: &str, cx: i32, cz: i32) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325; // FNV offset basis
    let mut mix = |b: u8| {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3); // FNV prime
    };
    for b in resident.bytes() {
        mix(b);
    }
    for v in [cx, cz] {
        for b in v.to_le_bytes() {
            mix(b);
        }
    }
    // 雪崩混合（splitmix64 finalizer）：讓各 bit 去相關——否則相近輸入（如 vox_res_0/1/2
    // 只差最後一個位元組）的低位 bit 容易撞在一起，害「牆材質」等只吃 1 bit 的維度失去變化。
    h ^= h >> 30;
    h = h.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h ^= h >> 27;
    h = h.wrapping_mul(0x94d0_49bb_1331_11eb);
    h ^= h >> 31;
    h
}

/// 群系決定牆材質的兩個候選（再由雜湊選一，讓同群系兩位居民仍可能不同）。
fn wall_palette(biome: VoxelBiome) -> [Block; 2] {
    match biome {
        VoxelBiome::Forest => [Block::Wood, Block::Plank],
        VoxelBiome::Grassland => [Block::Wood, Block::StoneBrick],
        VoxelBiome::Desert => [Block::Sand, Block::SmoothStone],
        VoxelBiome::Snow => [Block::SmoothStone, Block::Snow],
    }
}

/// 屋頂材質候選（4 選 1）。
const ROOF_PALETTE: [Block; 4] = [Block::Stone, Block::Wood, Block::Leaves, Block::SmoothStone];

// ── 建物方塊生成（純函式，可測）────────────────────────────────────────────────

/// 生成建物的方塊清單（從底層往上，讓 tick 逐塊放置時玩家看到「由下往上長出」）。
/// `style`：由 [`BuildStyle::for_resident`] 依居民/群系確定性決定，讓每間都不同。
fn generate_blocks(kind: BuildKind, cx: i32, cy: i32, cz: i32, style: &BuildStyle) -> Vec<BuildBlock> {
    let mut out = Vec::new();

    let add = |out: &mut Vec<BuildBlock>, x: i32, y: i32, z: i32, b: Block| {
        out.push(BuildBlock { x, y, z, b: b as u8 });
    };

    match kind {
        BuildKind::House => {
            let s = style;
            let (x0, x1) = (BuildStyle::X_MIN, s.x_max);
            let (z0, z1) = (BuildStyle::Z_MIN, s.z_max);
            // 地板（cy-1 層，實心填滿佔地，style.floor）——替換地表讓地基清晰。
            for x in x0..=x1 {
                for z in z0..=z1 {
                    add(&mut out, cx + x, cy - 1, cz + z, s.floor);
                }
            }
            // 牆壁 wall_h 層（只邊框，中心空，style.wall）；正面中央（x=0, z=z_max）下兩層
            // 疊放木門讓家「能被打開」（ROADMAP·門洞 v1）；side 牆中點依 windows 開玻璃窗。
            let front_z = z1;
            for layer in 0..s.wall_h {
                let y = cy + layer;
                for x in x0..=x1 {
                    for z in z0..=z1 {
                        let border = x == x0 || x == x1 || z == z0 || z == z1;
                        if !border {
                            continue;
                        }
                        // 門：正面中央下兩層。
                        if x == 0 && z == front_z && layer < 2 {
                            add(&mut out, cx + x, y, cz + z, Block::DoorClosed);
                            continue;
                        }
                        // 窗：側牆中點（z=0、x 在左右牆），第 1 層，且開窗。
                        if s.windows && layer == 1 && z == 0 && (x == x0 || x == x1) {
                            add(&mut out, cx + x, y, cz + z, Block::Glass);
                            continue;
                        }
                        add(&mut out, cx + x, y, cz + z, s.wall);
                    }
                }
            }
            // 屋頂（cy+wall_h 層，實心填滿，style.roof）。
            let roof_y = cy + s.wall_h;
            for x in x0..=x1 {
                for z in z0..=z1 {
                    add(&mut out, cx + x, roof_y, cz + z, s.roof);
                }
            }
            // 尖頂：再疊一層縮小的方塊（斜頂感）。3×3 → 單塊小尖；更大 → 一小條脊。
            if s.peaked {
                for x in (x0 + 1)..=(x1 - 1) {
                    for z in (z0 + 1)..=(z1 - 1) {
                        add(&mut out, cx + x, roof_y + 1, cz + z, s.roof);
                    }
                }
            }
            // 門口點綴（正面外一格，不動地基/不與牆重疊；每家不同的小細節）。
            let dz = front_z + 1;
            match s.decor {
                Decor::None => {}
                Decor::Torch => add(&mut out, cx - 1, cy, cz + dz, Block::Torch),
                Decor::Flowerbed => {
                    add(&mut out, cx, cy - 1, cz + dz, Block::Grass);
                    add(&mut out, cx, cy, cz + dz, Block::Leaves);
                }
                Decor::Pillar => {
                    for layer in 0..2 {
                        add(&mut out, cx + 1, cy + layer, cz + dz, s.wall);
                    }
                }
            }
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
            // 四角頂柱（井架感）——材質依居民/群系（style.wall），讓各家水井的井架不同。
            for &(dx, dz) in &[(-1i32, -1i32), (-1, 1), (1, -1), (1, 1)] {
                add(&mut out, cx + dx, cy + 1, cz + dz, style.wall);
            }
            // 共 8 + 1 + 8 + 4 = 21 塊
        }

        BuildKind::Tower => {
            // 塔身材質依居民/群系（style.wall），塔頂依 style.roof；地基維持 Stone（穩固）。
            // 塔身高 4 或 5 層（style.peaked 再拔高一層），讓各家瞭望台高矮不同。
            let body_h = if style.peaked { 5 } else { 4 };
            // 地基（cy-1 層，3×3 Stone 實心）
            for dx in -1i32..=1 {
                for dz in -1i32..=1 {
                    add(&mut out, cx + dx, cy - 1, cz + dz, Block::Stone);
                }
            }
            // 塔身（只邊框，style.wall，中心可穿行）
            for layer in 0..body_h {
                let y = cy + layer;
                for dx in -1i32..=1 {
                    for dz in -1i32..=1 {
                        if dx.abs() == 1 || dz.abs() == 1 {
                            add(&mut out, cx + dx, y, cz + dz, style.wall);
                        }
                    }
                }
            }
            // 瞭望台頂（cy+body_h 層，3×3 style.roof 實心）
            for dx in -1i32..=1 {
                for dz in -1i32..=1 {
                    add(&mut out, cx + dx, cy + body_h, cz + dz, style.roof);
                }
            }
            // 共 9 + 8×body_h + 9 塊
        }

        BuildKind::Garden => {
            // 花木中心：多數為 Leaves，開窗風格（style.windows）者改種樹苗，增添變化。
            let center = if style.windows { Block::Sapling } else { Block::Leaves };
            // 草地底（cy-1 層，3×3 Grass）
            for dx in -1i32..=1 {
                for dz in -1i32..=1 {
                    add(&mut out, cx + dx, cy - 1, cz + dz, Block::Grass);
                }
            }
            // 花壇邊框（cy 層，3×3 外框，材質依居民/群系 style.wall）
            for dx in -1i32..=1 {
                for dz in -1i32..=1 {
                    if dx.abs() == 1 || dz.abs() == 1 {
                        add(&mut out, cx + dx, cy, cz + dz, style.wall);
                    }
                }
            }
            // 中心裝飾（象徵花木）
            add(&mut out, cx, cy, cz, center);
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

// ── 居民互助蓋家（純函式，零 LLM）──────────────────────────────────────────────
// ROADMAP 696：老朋友到訪時，若主人正在蓋家，順手幫忙推進一塊——讓情誼（672）不再只停在
// 問候與八卦（694），第一次外溢成「真的動手互相幫忙」的協力行為，小社會湧現再深一層。

/// 每次老朋友到訪，觸發一次幫忙推進的機率。
pub const HELP_CHANCE: f32 = 0.4;

/// 判斷這次到訪是否要伸出援手推進主人的建造計畫一塊。
///
/// `remaining_before_pop`：主人建造計畫目前還剩幾塊待放（0 = 沒有計畫或已完成）。
/// 要求至少剩 2 塊才幫忙，確保幫忙後計畫仍未完工——完工收尾（記錄已蓋種類/完工 Feed/
/// 廣播）只交給 `tick_residents` 第 6 節統一處理一次，本函式的呼叫端不重複那段邏輯。
pub fn should_help_build(remaining_before_pop: usize, roll: f32) -> bool {
    remaining_before_pop >= 2 && roll < HELP_CHANCE
}

/// 幫忙放了一塊後，幫忙者冒出的台詞。
pub fn help_say_line(helper: &str, kind_name: &str) -> String {
    format!("看到在蓋{kind_name}，{helper}順手也幫忙放了一塊！")
}

// ── 玩家協助居民蓋家（純函式，零 LLM）──────────────────────────────────────────
// ROADMAP 699：居民互助蓋家（696）讓居民彼此的情誼外溢成動手幫忙，但玩家——那個一路
// 採礦砍樹合成工具的人——從沒能真正伸手參與居民蓋家。本節接上這個真缺口：玩家在正確的
// 座標放對方塊，就算幫了居民一把。

/// 居民收到玩家幫忙放塊後，冒出的道謝台詞。
pub fn player_help_say_line(player_name: &str, kind_name: &str) -> String {
    format!("謝謝{player_name}幫忙放的這塊，{kind_name}又更接近完工了！")
}

/// 玩家協助居民建造後，居民心中留下的**感激記憶**摘要（純函式、確定性、零 LLM）。
///
/// 「互動有後果」閉環的第一塊拼圖：此前玩家幫居民蓋家（ROADMAP 699）只化成一句道謝
/// 泡泡＋心情補助＋Feed，**從沒寫進記憶**——重啟即遺忘、不累積好感、也不驅動任何後續
/// 回報。這條摘要讓那份出力**真的被記住**：
/// - 含「幫我蓋」關鍵詞 → 供 [`crate::voxel_fond_greeting::detect_context`] 認出
///   「協助建造」情境，居民日後見你會說出提及這份情的專屬老友問候。
/// - **刻意避開** [`crate::voxel_memory::classify_importance`] 的目標/偏好/承諾關鍵詞
///   （不含「要蓋」「想要」「答應」等）→ 停在 episodic 層：計入好感（記憶筆數），
///   卻不佔用長期精華 12 條上限。
///
/// 面向玩家字串集中一處、留 i18n 空間。
pub fn player_help_memory_line(player_name: &str, kind_name: &str) -> String {
    format!("{player_name}幫我蓋{kind_name}，出了一份力，我記著這份情。")
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

// ── 居民改動世界的方塊持久化（重啟後蓋的東西/挖的洞還在）──────────────────────
//
// hub 的 world delta 是記憶體層；居民蓋造放的每一塊、採集挖掉的每一格，都 append 到這份
// jsonl，啟動時 replay 套回 delta → **重啟後居民蓋的東西還在、挖的洞還在**（持久化）。
// append-only、向後相容（檔缺=空）。玩家自己 break/place 不走這裡（沿用原 session 內行為）。

/// 居民改動方塊落地路徑（`data/` 已 gitignore）。
pub const VOXEL_RES_BLOCKS_PATH: &str = "data/voxel_resident_blocks.jsonl";

/// Append 一筆「居民改了某方塊」記錄（放置或挖空都走這裡，b=0 即 Air）。
/// **鐵律**：只在不持任何鎖時呼叫（小檔同步寫，不 await）。
pub fn append_world_block(x: i32, y: i32, z: i32, b: u8) {
    let bb = BuildBlock { x, y, z, b };
    if let Ok(line) = serde_json::to_string(&bb) {
        write_world_line(VOXEL_RES_BLOCKS_PATH, &line);
    }
}

/// 載回所有居民方塊改動（伺服器啟動時呼叫一次，依序套回 delta）。檔不存在 / 壞行皆容忍。
pub fn load_world_blocks() -> Vec<BuildBlock> {
    let content = match std::fs::read_to_string(VOXEL_RES_BLOCKS_PATH) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    content
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.is_empty() { None } else { serde_json::from_str::<BuildBlock>(l).ok() }
        })
        .collect()
}

fn write_world_line(path: &str, line: &str) {
    use std::io::Write;
    if let Some(parent) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::OpenOptions::new().create(true).append(true).open(path) {
        Ok(mut f) => {
            let _ = writeln!(f, "{line}");
        }
        Err(e) => tracing::warn!("無法寫入居民方塊改動 {path}: {e}"),
    }
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

    // ── prayer_promotable（禱告→持久渴望的提升閘）────────────────────────────

    #[test]
    fn prayer_promotable_true_for_concrete_build_wish() {
        // 建造 prompt 範例句本身即可分類，理當提升。
        assert!(prayer_promotable("願農田旁能有水源"));
        assert!(prayer_promotable("好想蓋一座能眺望遠方的瞭望台"));
    }

    #[test]
    fn prayer_promotable_false_for_vague_mood_wish() {
        // 純抒發心情、沒有具體建物種類，不該提升（避免頻繁蓋掉真正的建造心願）。
        assert!(!prayer_promotable("好想有個慶典熱鬧一下"));
        assert!(!prayer_promotable("這一帶夜裡不安全，盼有人守望"));
        assert!(!prayer_promotable(""));
    }

    // ── generate_blocks 方塊數（用「最小樣式」＝退化回原本 3×3 方盒，維持穩定基準）──

    /// 最小樣式：3×3、牆高 2、平頂、無窗、無點綴 → 等同建築創作前的原始小木屋，
    /// 讓既有方塊數/門洞的穩定基準測試仍成立（其餘測試才去驗「變化」）。
    fn style_small() -> BuildStyle {
        BuildStyle {
            wall: Block::Wood,
            roof: Block::Stone,
            floor: Block::Plank,
            peaked: false,
            windows: false,
            wall_h: 2,
            x_max: 1,
            z_max: 1,
            decor: Decor::None,
        }
    }

    #[test]
    fn house_block_count_small_style() {
        let blocks = generate_blocks(BuildKind::House, 0, 5, 0, &style_small());
        // 地板 9 + 牆 8+8 + 屋頂 9 = 34（門洞取代 2 塊木牆，總數不變）
        assert_eq!(blocks.len(), 34);
    }

    #[test]
    fn house_has_two_layer_door_at_front() {
        let blocks = generate_blocks(BuildKind::House, 10, 5, 20, &style_small());
        // 正面（dx=0, z=z_max=+1）兩層都應是門，讓居民蓋的家真的能被打開走進去。
        let door_layer0 = blocks.iter().find(|b| b.x == 10 && b.y == 5 && b.z == 21);
        let door_layer1 = blocks.iter().find(|b| b.x == 10 && b.y == 6 && b.z == 21);
        assert_eq!(door_layer0.map(|b| b.b), Some(Block::DoorClosed as u8));
        assert_eq!(door_layer1.map(|b| b.b), Some(Block::DoorClosed as u8));
        // 其餘牆體邊框仍是 Wood，沒有被誤改。
        let corner = blocks.iter().find(|b| b.x == 9 && b.y == 5 && b.z == 19).unwrap();
        assert_eq!(corner.b, Block::Wood as u8);
    }

    #[test]
    fn well_block_count_small_style() {
        let blocks = generate_blocks(BuildKind::Well, 0, 5, 0, &style_small());
        // 底圈 8 + 水 1 + 井壁 8 + 角柱 4 = 21
        assert_eq!(blocks.len(), 21);
    }

    #[test]
    fn tower_block_count_small_style() {
        let blocks = generate_blocks(BuildKind::Tower, 0, 5, 0, &style_small());
        // 地基 9 + 塔身 8×4 + 頂台 9 = 50（peaked=false → body_h=4）
        assert_eq!(blocks.len(), 50);
    }

    #[test]
    fn garden_block_count_small_style() {
        let blocks = generate_blocks(BuildKind::Garden, 0, 5, 0, &style_small());
        // 草地 9 + 邊框 8 + 中心 1 = 18
        assert_eq!(blocks.len(), 18);
    }

    #[test]
    fn all_blocks_have_valid_block_type_across_residents() {
        // 掃過多位居民 × 四群系 × 四種建物，任何組合生出的方塊都必須是合法方塊 id。
        for rid in ["vox_res_0", "vox_res_1", "vox_res_2", "vox_res_3", "vox_res_7"] {
            for biome in [
                VoxelBiome::Grassland,
                VoxelBiome::Forest,
                VoxelBiome::Desert,
                VoxelBiome::Snow,
            ] {
                let style = BuildStyle::for_resident(rid, biome, 7, 0);
                for kind in [BuildKind::House, BuildKind::Well, BuildKind::Tower, BuildKind::Garden] {
                    let blocks = generate_blocks(kind, 0, 5, 0, &style);
                    for bb in &blocks {
                        assert!(
                            Block::from_u8(bb.b).is_some(),
                            "無效方塊 id={} 在 {:?}/{:?}/{rid}",
                            bb.b, kind, biome
                        );
                    }
                }
            }
        }
    }

    // ── 建築風格：確定性 + 變化 + 合法性（建築創作第一刀）──────────────────────

    #[test]
    fn style_is_deterministic_for_same_resident_and_anchor() {
        // 同居民、同群系、同錨點 → 永遠同一份風格（重啟一致、不會這次木下次石）。
        let a = BuildStyle::for_resident("vox_res_0", VoxelBiome::Forest, 7, 0);
        let b = BuildStyle::for_resident("vox_res_0", VoxelBiome::Forest, 7, 0);
        assert_eq!(a, b);
        // 連方塊清單也逐塊一致。
        let ba = generate_blocks(BuildKind::House, 100, 5, 100, &a);
        let bb = generate_blocks(BuildKind::House, 100, 5, 100, &b);
        assert_eq!(ba, bb, "同輸入應生出逐塊相同的藍圖");
    }

    #[test]
    fn wall_material_follows_biome() {
        // 牆材質限定在該群系的候選內（森林木系、沙漠沙/拋石、雪原拋石/雪）。
        for rid in ["vox_res_0", "vox_res_1", "vox_res_2", "vox_res_3"] {
            let forest = BuildStyle::for_resident(rid, VoxelBiome::Forest, 7, 0).wall;
            assert!(
                matches!(forest, Block::Wood | Block::Plank),
                "森林牆材應為木系：{forest:?}"
            );
            let desert = BuildStyle::for_resident(rid, VoxelBiome::Desert, 7, 0).wall;
            assert!(
                matches!(desert, Block::Sand | Block::SmoothStone),
                "沙漠牆材應為沙/拋石：{desert:?}"
            );
            let snow = BuildStyle::for_resident(rid, VoxelBiome::Snow, 7, 0).wall;
            assert!(
                matches!(snow, Block::SmoothStone | Block::Snow),
                "雪原牆材應為拋石/雪：{snow:?}"
            );
        }
    }

    #[test]
    fn different_residents_get_varied_houses() {
        // 走進村子：不同居民（同群系同座標）蓋出的房子藍圖應當彼此不同，不再是複製貼上。
        // 收集多位居民的房子「特徵指紋」（材質/尺寸/屋頂/裝飾），要求有足夠多樣。
        let biome = VoxelBiome::Grassland;
        let mut fingerprints = std::collections::HashSet::new();
        let mut blueprints = std::collections::HashSet::new();
        for i in 0..8 {
            let rid = format!("vox_res_{i}");
            let s = BuildStyle::for_resident(&rid, biome, 7, 0);
            fingerprints.insert((
                s.wall as u8,
                s.roof as u8,
                s.peaked,
                s.windows,
                s.wall_h,
                s.x_max,
                s.z_max,
                s.decor,
            ));
            let blocks = generate_blocks(BuildKind::House, 0, 5, 0, &s);
            // 以「排序後的方塊清單」當整棟房子的指紋。
            let mut v: Vec<(i32, i32, i32, u8)> =
                blocks.iter().map(|b| (b.x, b.y, b.z, b.b)).collect();
            v.sort();
            blueprints.insert(v);
        }
        // 8 位居民至少要有 4 種不同的樣式指紋（實務上遠超過），證明「各有不同」。
        assert!(
            fingerprints.len() >= 4,
            "8 位居民的房子樣式太雷同（只有 {} 種）",
            fingerprints.len()
        );
        assert!(
            blueprints.len() >= 4,
            "8 位居民的房子藍圖太雷同（只有 {} 種）",
            blueprints.len()
        );
    }

    #[test]
    fn house_never_has_overlapping_blocks() {
        // 合法性：同一棟房子不得有兩塊落在同一格（重疊＝壞掉的藍圖）。掃過所有變化維度。
        for rid in ["vox_res_0", "vox_res_1", "vox_res_2", "vox_res_3", "vox_res_5"] {
            for biome in [
                VoxelBiome::Grassland,
                VoxelBiome::Forest,
                VoxelBiome::Desert,
                VoxelBiome::Snow,
            ] {
                let s = BuildStyle::for_resident(rid, biome, 7, 0);
                let blocks = generate_blocks(BuildKind::House, 0, 5, 0, &s);
                let mut seen = std::collections::HashSet::new();
                for b in &blocks {
                    assert!(
                        seen.insert((b.x, b.y, b.z)),
                        "方塊重疊於 ({},{},{})：{rid}/{biome:?}",
                        b.x, b.y, b.z
                    );
                }
            }
        }
    }

    #[test]
    fn house_foundation_floor_is_always_solid_fill() {
        // 防破地基：無論尺寸怎麼變，cy-1 地板層都必須把整個佔地填滿實心（無空洞）。
        for i in 0..8 {
            let rid = format!("vox_res_{i}");
            let s = BuildStyle::for_resident(&rid, VoxelBiome::Grassland, 7, 0);
            let (cx, cy, cz) = (0, 5, 0);
            let blocks = generate_blocks(BuildKind::House, cx, cy, cz, &s);
            for x in BuildStyle::X_MIN..=s.x_max {
                for z in BuildStyle::Z_MIN..=s.z_max {
                    let cell = blocks
                        .iter()
                        .find(|b| b.x == cx + x && b.y == cy - 1 && b.z == cz + z);
                    let filled = cell.and_then(|b| Block::from_u8(b.b)).map_or(false, |bl| bl.is_solid());
                    assert!(filled, "地板 ({x},{z}) 應為實心地基：{rid}");
                }
            }
        }
    }

    #[test]
    fn house_always_has_two_layer_front_door() {
        // 防回歸：無論尺寸/材質/裝飾怎麼變，正面中央（x=0, z=z_max）下兩層永遠是門，
        // 讓每間家都「打得開走得進」（門洞 v1 + 完工錨點 #967 不受影響）。
        for i in 0..8 {
            let rid = format!("vox_res_{i}");
            let s = BuildStyle::for_resident(&rid, VoxelBiome::Grassland, 7, 0);
            let (cx, cy, cz) = (10, 5, 20);
            let blocks = generate_blocks(BuildKind::House, cx, cy, cz, &s);
            let door_z = cz + s.z_max;
            for layer in 0..2 {
                let d = blocks
                    .iter()
                    .find(|b| b.x == cx && b.y == cy + layer && b.z == door_z);
                assert_eq!(
                    d.map(|b| b.b),
                    Some(Block::DoorClosed as u8),
                    "正面第 {layer} 層應為門：{rid}"
                );
            }
        }
    }

    #[test]
    fn house_block_count_stays_bounded() {
        // 效能：房子方塊數不得暴增（尺寸變化有上限）。掃過所有變化維度，皆 ≤ 80。
        for rid in ["vox_res_0", "vox_res_1", "vox_res_2", "vox_res_3", "vox_res_9"] {
            for biome in [
                VoxelBiome::Grassland,
                VoxelBiome::Forest,
                VoxelBiome::Desert,
                VoxelBiome::Snow,
            ] {
                let s = BuildStyle::for_resident(rid, biome, 7, 0);
                let n = generate_blocks(BuildKind::House, 0, 5, 0, &s).len();
                assert!(n <= 80, "房子方塊數暴增（{n}）：{rid}/{biome:?}");
                assert!(n >= 30, "房子方塊數異常過少（{n}）：{rid}/{biome:?}");
            }
        }
    }

    // ── BuildStore 純函式 ─────────────────────────────────────────────────────

    #[test]
    fn store_has_plan_after_new() {
        let mut s = BuildStore::new();
        assert!(!s.has_plan("vox_res_0"));
        s.new_plan("vox_res_0", BuildKind::House, 10, 5, 20, false, None);
        assert!(s.has_plan("vox_res_0"));
        assert!(!s.has_plan("vox_res_1"));
    }

    #[test]
    fn store_pop_next_reduces_remaining() {
        let mut s = BuildStore::new();
        s.new_plan("vox_res_0", BuildKind::Well, 0, 5, 0, false, None);
        let before = s.plans["vox_res_0"].remaining.len();
        let b = s.get_plan_mut("vox_res_0").unwrap().pop_next();
        assert!(b.is_some());
        assert_eq!(s.plans["vox_res_0"].remaining.len(), before - 1);
    }

    #[test]
    fn store_remove_if_done_works() {
        let mut s = BuildStore::new();
        s.new_plan("vox_res_0", BuildKind::Well, 0, 5, 0, false, None);
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
            expansion: false,
            inspired_by: None,
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
            expansion: false,
            inspired_by: None,
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
            expansion: false,
            inspired_by: None,
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
            expansion: false,
            inspired_by: None,
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

    // ── should_help_build / help_say_line 純函式 ─────────────────────────────

    #[test]
    fn help_needs_at_least_two_remaining() {
        // 只剩 1 塊或 0 塊（沒計畫/已完成）：就算擲骰中獎也不該幫忙，避免搶在
        // tick_residents 完工收尾之前把計畫清空。
        assert!(!should_help_build(0, 0.0));
        assert!(!should_help_build(1, 0.0));
        // 剩 2 塊以上、擲骰命中 → 幫忙。
        assert!(should_help_build(2, 0.0));
        assert!(should_help_build(10, 0.39));
    }

    #[test]
    fn help_respects_chance_roll() {
        // 擲骰值 >= HELP_CHANCE 不幫忙（就算剩很多塊）。
        assert!(!should_help_build(10, HELP_CHANCE));
        assert!(!should_help_build(10, 0.99));
    }

    #[test]
    fn help_say_line_mentions_helper_and_kind() {
        let line = help_say_line("諾娃", "小木屋");
        assert!(line.contains("諾娃"), "應提到幫忙者：{line}");
        assert!(line.contains("小木屋"), "應提到建物種類：{line}");
    }

    // ── try_player_help 純邏輯（ROADMAP 699）────────────────────────────────────

    fn store_with_plan(resident: &str, kind: BuildKind) -> BuildStore {
        let mut s = BuildStore::new();
        s.new_plan(resident, kind, 0, 64, 0, false, None);
        s
    }

    #[test]
    fn player_help_pops_matching_front_block() {
        let mut s = store_with_plan("vox_res_0", BuildKind::House);
        let front = s.plans["vox_res_0"].remaining.front().cloned().unwrap();
        let before_len = s.plans["vox_res_0"].remaining.len();

        let result = s.try_player_help(front.x, front.y, front.z, front.b);

        assert_eq!(result, Some(("vox_res_0".to_string(), "小木屋".to_string())));
        assert_eq!(s.plans["vox_res_0"].remaining.len(), before_len - 1, "應彈掉一塊");
    }

    #[test]
    fn player_help_ignores_wrong_position() {
        let mut s = store_with_plan("vox_res_0", BuildKind::House);
        let before_len = s.plans["vox_res_0"].remaining.len();

        let result = s.try_player_help(9999, 9999, 9999, Block::Wood as u8);

        assert_eq!(result, None, "座標不符不算幫忙");
        assert_eq!(s.plans["vox_res_0"].remaining.len(), before_len, "沒有計畫被更動");
    }

    #[test]
    fn player_help_ignores_wrong_block_type() {
        let mut s = store_with_plan("vox_res_0", BuildKind::House);
        let front = s.plans["vox_res_0"].remaining.front().cloned().unwrap();
        let before_len = s.plans["vox_res_0"].remaining.len();
        let wrong_block = if front.b == Block::Stone as u8 { Block::Wood as u8 } else { Block::Stone as u8 };

        let result = s.try_player_help(front.x, front.y, front.z, wrong_block);

        assert_eq!(result, None, "座標對但方塊類型不符不算幫忙");
        assert_eq!(s.plans["vox_res_0"].remaining.len(), before_len);
    }

    #[test]
    fn player_help_no_plans_returns_none() {
        let mut s = BuildStore::new();
        assert_eq!(s.try_player_help(0, 64, 0, Block::Wood as u8), None);
    }

    #[test]
    fn player_help_picks_correct_resident_among_many() {
        let mut s = store_with_plan("vox_res_0", BuildKind::House);
        s.new_plan("vox_res_1", BuildKind::Well, 20, 64, 20, false, None);
        let front1 = s.plans["vox_res_1"].remaining.front().cloned().unwrap();

        let result = s.try_player_help(front1.x, front1.y, front1.z, front1.b);

        assert_eq!(result, Some(("vox_res_1".to_string(), "水井".to_string())));
        // 另一位居民的計畫不受影響。
        assert!(!s.plans["vox_res_0"].remaining.is_empty());
    }

    #[test]
    fn player_help_say_line_mentions_player_and_kind() {
        let line = player_help_say_line("小明", "小木屋");
        assert!(line.contains("小明"), "應提到玩家名：{line}");
        assert!(line.contains("小木屋"), "應提到建物種類：{line}");
    }

    #[test]
    fn player_help_memory_line_mentions_player_kind_and_has_helpbuild_keyword() {
        let line = player_help_memory_line("小明", "小木屋");
        assert!(line.contains("小明"), "應提到玩家名：{line}");
        assert!(line.contains("小木屋"), "應提到建物種類：{line}");
        // 含「幫我蓋」→ 供 fond_greeting 認出「協助建造」情境。
        assert!(line.contains("幫我蓋"), "應含 HelpedBuild 關鍵詞「幫我蓋」：{line}");
    }

    #[test]
    fn player_help_memory_line_avoids_semantic_keywords() {
        // 感激記憶必須停在 episodic 層（計入好感、不佔長期精華）——
        // 不可含 classify_importance 的目標/偏好/承諾關鍵詞，否則會被誤升成語意事實。
        for kind in ["小木屋", "水井", "瞭望台", "花圃"] {
            let line = player_help_memory_line("旅人", kind);
            for kw in ["要蓋", "要建", "想要", "打算", "要把", "我要", "最喜歡", "喜歡", "答應", "承諾"] {
                assert!(!line.contains(kw), "感激記憶不該含語意關鍵詞「{kw}」：{line}");
            }
        }
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
