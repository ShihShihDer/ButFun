//! 乙太方界·居民長程自主專案 v1（居民懷抱一個跨多天、記得回來繼續做的個人夢想，自主提案切片）。
//!
//! **真缺口**：居民已會即時發明技能（真進化）、蓋家（agency）、採集備料、萌生**當下的心願**
//! （`voxel_desires`），但這些全是**短程**：一件事做完就翻篇。世界至今沒有一位居民懷著
//! 「一個**較大的個人夢想**、跨好幾天、分階段慢慢完成、期間記得它、有進展、完成時圓夢」的
//! **長程自主**。這正是北極星「居民自己活出人生」最深的一種——不是被 push、不是當下衝動，而是
//! 心裡一直放著一件想親手做成的事，日復一日一點一點把它做出來。
//!
//! **本刀**：每位居民依**性格（persona）**加一點**個人差異（名字雜湊）**，立定一個屬於自己的
//! 大夢（把家旁空地種成小花園／沿路點一排燈把家連到村／為四方群系立一座紀念展台），拆成
//! **數個階段**、每個階段是一小疊要親手放下的方塊。居民閒暇時偶爾回來添上**一塊**——夢想
//! 於是在世界裡**跨天慢慢成形**：玩家這幾天路過，會看見同一位居民一直在忙同一件事，一座
//! 小花園、一排燈、一方展台從無到有慢慢長出來。放到一個階段的最後一塊、圓滿全夢時，各有
//! 一則溫暖的圓夢時刻（冒泡＋城鎮動態＋內心記憶）。
//!
//! **與既有系統 razor-sharp 區隔**：
//! - **心願（`voxel_desires`）**＝「**當下**」單一即時渴望（想要一塊麵包），一送到就了結、隨時被
//!   新的覆蓋；本刀＝**跨天**的**同一個**大夢，持久化進度、重啟接續、不被日常心願打斷。
//! - **發明（`voxel_invent`）**＝把基礎動作組合出**新技能配方**（how-to，秒～分鐘就跑完）；本刀＝
//!   一個**多階段的建造專案**（what，跨天），不學任何新技能、只是把心裡的家園一塊一塊蓋出來。
//! - **村碑（`voxel_monument`）**＝**全村**集體里程碑的實體；本刀＝**單一居民**的**個人**夢想。
//!
//! **純邏輯層**：本檔全是零 IO／零鎖／零 LLM／零 async 的確定性純函式——夢想挑選、方塊佈局、
//! 階段切分、面向玩家的句式全在這裡（i18n 友善、好測）。放置／廣播／記憶／Feed／持久化都在
//! `voxel_ws.rs`（沿用村碑 885 的「golden safe pattern」：`surface_y` 鎖外算 → `deltas` 寫鎖
//! 批次**只在空氣格**落子（絕不覆蓋任何既有方塊、冪等）→ 鎖外廣播＋append-only 落地，守死鎖鐵律）。
//! 記憶為第一人稱內心句、不含任何玩家名／私密渴望（比照 `voxel_monument::monument_memory_line`）。
//!
//! ## v1.1：圓夢角落成為世界地標（自主提案切片，接續 v1）
//! **真缺口**：v1 讓夢想在世界裡跨天長出來，但圓滿之後**什麼都沒留下**——跟遺跡（838）／溫泉
//! （839）／邊陲營地（881）／世界奇觀（940）／地底遺跡神殿（975）比起來，「居民親手圓的夢」
//! 是探索紀事系統裡唯一沒被世界記住的一種地標；玩家路過同一位居民已完工的花園／燈路／展台，
//! 世界毫無反應。本刀把已圓滿的夢接進既有 `voxel_discovery::LandmarkKind`（新增 `Dream` 一種）：
//! 玩家走近任一位已圓夢居民的錨點（[`LifeDream::anchor_cell`]）→ 記一筆探索紀事、解鎖里程碑、
//! 順手看看先前旅人在這座圓夢角落留下的話（沿用旅人留言簿 862 既有管道）。純加法：不改夢想的
//! 挑選／佈局／進度邏輯，只讓「已完工」這件事被世界看見。接線（proximity 掃描、milestone、
//! discovery/landmark_notes 呼叫）全在 `voxel_ws.rs`，本模組只加兩個確定性純函式
//! （[`LifeDream::anchor_cell`]／[`near_dream_landmark`]）。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::voxel::Block;

/// 持久化路徑（`data/` 已 gitignore）。每推進一塊 append 一行，`from_entries` 取每人最高進度重建。
const LIFEPROJECT_PATH: &str = "data/voxel_lifeprojects.jsonl";

/// 泡泡／Feed 片段字元上限（比照其他泡泡台詞）。
pub const BUBBLE_MAX: usize = 40;

/// 圓夢地標偵測半徑（世界方塊，比照 `voxel::DUNGEON_DISCOVER_RADIUS` 量級）：玩家走到這麼近
/// 圓滿夢想的錨點，視為「發現這座地標」——記一筆探索紀事、可留言。純函式、確定性、零狀態。
pub const DISCOVER_RADIUS: f32 = 4.0;

/// 距離平方判定「玩家走到某位居民已圓滿的夢想錨點附近」。純函式（不含 sqrt）。
pub fn near_dream_landmark(px: f32, pz: f32, ax: i32, az: i32) -> bool {
    let dx = px - ax as f32;
    let dz = pz - az as f32;
    dx * dx + dz * dz <= DISCOVER_RADIUS * DISCOVER_RADIUS
}

// ── 夢想種類 ─────────────────────────────────────────────────────────────────

/// 一位居民立定的大夢種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifeDreamKind {
    /// 把家旁的空地種成一座小花園（圍籬→翻土→播下野花）。
    FlowerGarden,
    /// 沿路點一排乙太燈，把自己的家一路連到村子中心。
    LanternPath,
    /// 為四方群系立一座小小紀念展台，擺上各地帶回的紀念品。
    BiomeShrine,
}

impl LifeDreamKind {
    /// 穩定 id（持久化鍵，別更動既有值）。
    pub fn id(self) -> &'static str {
        match self {
            LifeDreamKind::FlowerGarden => "flower_garden",
            LifeDreamKind::LanternPath => "lantern_path",
            LifeDreamKind::BiomeShrine => "biome_shrine",
        }
    }

    /// 由持久化 id 還原（未知 id → None，容忍舊壞資料）。
    pub fn from_id(s: &str) -> Option<Self> {
        match s {
            "flower_garden" => Some(LifeDreamKind::FlowerGarden),
            "lantern_path" => Some(LifeDreamKind::LanternPath),
            "biome_shrine" => Some(LifeDreamKind::BiomeShrine),
            _ => None,
        }
    }

    /// 這個夢的白話標題（面向玩家）。
    pub fn title_zh(self) -> &'static str {
        match self {
            LifeDreamKind::FlowerGarden => "把家旁的空地種成一座小花園",
            LifeDreamKind::LanternPath => "沿路點一排燈，把家一路連到村子",
            LifeDreamKind::BiomeShrine => "立一座小展台，擺上四方帶回的紀念品",
        }
    }
}

/// 依居民 **persona**（性格，0=市集/1=農務/2=廣場/3=漫遊）＋**名字雜湊**（個人差異）挑一個大夢。
///
/// 主軸由性格決定（農務者想種花園、漫遊者想點燈連路、市集人想蒐藏紀念品）；少數人（雜湊落在
/// 特定餘數）會偏離主軸挑鄰近的夢，讓同性格的人也不完全一樣（個人化）。確定性、重啟穩定。
pub fn dream_kind_for(persona_code: u8, name_hash: u64) -> LifeDreamKind {
    // 三種夢的固定次序，供「偏移一格」取變化。
    const KINDS: [LifeDreamKind; 3] = [
        LifeDreamKind::FlowerGarden,
        LifeDreamKind::LanternPath,
        LifeDreamKind::BiomeShrine,
    ];
    // persona → 主軸夢的索引。
    let base = match persona_code {
        1 => 0, // FarmWorker → 花園
        3 => 1, // Wanderer → 點燈連路
        0 => 2, // MarketBrowser → 紀念展台
        _ => 1, // TownSquare（2）與其他 → 點燈連路（把廣場與家連起來）
    };
    // 少數人（雜湊 % 5 == 0）向後偏一格，製造個人差異但仍以性格為主。
    let idx = if name_hash % 5 == 0 { (base + 1) % 3 } else { base };
    KINDS[idx]
}

/// 面向玩家名字的確定性雜湊（FNV-1a，純函式、跨平台穩定）。供 `dream_kind_for` 與句式輪替種子。
pub fn name_hash(name: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in name.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

// ── 夢的方塊佈局 ─────────────────────────────────────────────────────────────

/// 夢裡要親手放下的一塊：相對「錨點（居民家）」的水平位移 `(dx, dz)` ＋在該格**地面正上方**再往上
/// `dy` 格（0＝直接落在地面上）＋方塊種類。呼叫端各格自算 `surface_y(anchor_x+dx, anchor_z+dz)`
/// 再加 `dy`，故不同地表高度各自貼地（守「絕不覆蓋既有方塊」的前提）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DreamCell {
    pub dx: i32,
    pub dy: i32,
    pub dz: i32,
    pub block: Block,
}

/// 一個階段：一個好記的名字＋這階要放下的方塊們。
pub struct DreamStage {
    pub name_zh: &'static str,
    pub cells: Vec<DreamCell>,
}

/// 一個完整的大夢：種類＋依序的數個階段。
pub struct LifeDream {
    pub kind: LifeDreamKind,
    pub stages: Vec<DreamStage>,
}

impl LifeDream {
    /// 全夢總共要放下幾塊。
    pub fn total_cells(&self) -> usize {
        self.stages.iter().map(|s| s.cells.len()).sum()
    }

    /// 攤平後第 `idx` 塊（跨階段連續編號）。越界 → None。
    pub fn cell_at(&self, idx: usize) -> Option<DreamCell> {
        let mut n = idx;
        for s in &self.stages {
            if n < s.cells.len() {
                return Some(s.cells[n]);
            }
            n -= s.cells.len();
        }
        None
    }

    /// 各階段的「攤平累計結束索引」（如 [4, 8, 10] 表三階分別到第 4/8/10 塊）。
    pub fn stage_boundaries(&self) -> Vec<usize> {
        let mut acc = 0;
        self.stages
            .iter()
            .map(|s| {
                acc += s.cells.len();
                acc
            })
            .collect()
    }

    /// 攤平索引 `idx`（0-based，指「第 idx 塊」）屬於第幾階。越界 → None。
    pub fn stage_of(&self, idx: usize) -> Option<usize> {
        let mut acc = 0;
        for (si, s) in self.stages.iter().enumerate() {
            acc += s.cells.len();
            if idx < acc {
                return Some(si);
            }
        }
        None
    }

    /// 放到「已放 `new_placed` 塊」時，是否**剛好放完某一階的最後一塊**；是則回該階索引。
    /// （`new_placed` 為放完這一塊後的累計數，1-based 數量。）用來在階段收尾那一刻冒里程碑泡泡。
    pub fn stage_completed_by(&self, new_placed: usize) -> Option<usize> {
        self.stage_boundaries()
            .iter()
            .position(|&b| b == new_placed)
    }

    /// 全夢是否已完成（已放數量 ≥ 總數）。
    pub fn is_complete(&self, placed: usize) -> bool {
        placed >= self.total_cells()
    }

    /// 圓夢地標 v1（自主提案切片，接續本模組 v1）：這座夢的「代表點」——取第一塊的相對位移，
    /// 供玩家路過偵測與旅人留言簿定位。夢至少有一塊（見 `build_dream` 各分支），故必回傳。
    pub fn anchor_cell(&self) -> DreamCell {
        self.cell_at(0).expect("每個夢至少有一塊")
    }
}

/// 依種類＋「朝村子的方向」建出完整的夢。`toward_x`/`toward_z` 為指向村莊中心的單位方向
/// （各 ∈ {-1,0,1}）：只有「點燈連路」用得到（燈往村子延伸）；其餘種類忽略。若兩者皆 0
/// （沒有村莊中心可參照），點燈連路預設朝 +x 延伸（永不退化成原地疊燈）。
pub fn build_dream(kind: LifeDreamKind, toward_x: i32, toward_z: i32) -> LifeDream {
    match kind {
        LifeDreamKind::FlowerGarden => flower_garden(),
        LifeDreamKind::LanternPath => lantern_path(toward_x, toward_z),
        LifeDreamKind::BiomeShrine => biome_shrine(),
    }
}

/// 小花園：家旁偏一點的一方 3×3 園圃。①圍四角木樁②中間翻五格農土③農土上播下五朵野花。
fn flower_garden() -> LifeDream {
    // 園圃中心偏離家門 3 格（+x），免得蓋在自家門口。
    let ox = 3;
    let oz = 0;
    let corners = [(-1, -1), (1, -1), (-1, 1), (1, 1)];
    let plus = [(0, 0), (-1, 0), (1, 0), (0, -1), (0, 1)];
    let flowers = [
        Block::WildflowerRed,
        Block::WildflowerYellow,
        Block::WildflowerBlue,
        Block::WildflowerYellow,
        Block::WildflowerRed,
    ];
    LifeDream {
        kind: LifeDreamKind::FlowerGarden,
        stages: vec![
            DreamStage {
                name_zh: "圍起小籬笆",
                cells: corners
                    .iter()
                    .map(|&(dx, dz)| DreamCell { dx: ox + dx, dy: 0, dz: oz + dz, block: Block::Wood })
                    .collect(),
            },
            DreamStage {
                name_zh: "翻好一畦土",
                cells: plus
                    .iter()
                    .map(|&(dx, dz)| DreamCell { dx: ox + dx, dy: 0, dz: oz + dz, block: Block::FarmSoil })
                    .collect(),
            },
            DreamStage {
                name_zh: "播下一片野花",
                cells: plus
                    .iter()
                    .zip(flowers.iter())
                    .map(|(&(dx, dz), &fl)| DreamCell { dx: ox + dx, dy: 1, dz: oz + dz, block: fl })
                    .collect(),
            },
        ],
    }
}

/// 點燈連路：從家往村子方向，每隔兩格點一盞乙太燈，共九盞，分三階（起點→延伸→接上村子）。
fn lantern_path(toward_x: i32, toward_z: i32) -> LifeDream {
    // 方向正規化：任一非零就用它；全零預設 +x（永不原地疊燈）。
    let (sx, sz) = if toward_x == 0 && toward_z == 0 {
        (1, 0)
    } else {
        (toward_x.signum(), toward_z.signum())
    };
    // 第 k 盞燈的位移（從家外第 2 格起、每 2 格一盞）。
    let lamp = |k: i32| DreamCell {
        dx: sx * (2 + k * 2),
        dy: 0,
        dz: sz * (2 + k * 2),
        block: Block::AetherLamp,
    };
    LifeDream {
        kind: LifeDreamKind::LanternPath,
        stages: vec![
            DreamStage { name_zh: "先點亮家門口", cells: (0..3).map(lamp).collect() },
            DreamStage { name_zh: "一盞盞往前延伸", cells: (3..6).map(lamp).collect() },
            DreamStage { name_zh: "接上村子的燈火", cells: (6..9).map(lamp).collect() },
        ],
    }
}

/// 四方紀念展台：家旁一方 2×2 石磚展台，四角各擺一件群系紀念品（沙／冰晶／樹苗／野花），
/// 最後中央立一柱點一盞燈。①鋪四格石磚台②擺上四方紀念品③石磚柱＋頂燈。
fn biome_shrine() -> LifeDream {
    let ox = 3; // 展台中心偏離家門 3 格。
    let oz = 2;
    let base = [(0, 0), (1, 0), (0, 1), (1, 1)];
    let souvenirs = [Block::Sand, Block::IceCrystal, Block::Sapling, Block::WildflowerBlue];
    LifeDream {
        kind: LifeDreamKind::BiomeShrine,
        stages: vec![
            DreamStage {
                name_zh: "鋪一方展台",
                cells: base
                    .iter()
                    .map(|&(dx, dz)| DreamCell { dx: ox + dx, dy: 0, dz: oz + dz, block: Block::StoneBrick })
                    .collect(),
            },
            DreamStage {
                name_zh: "擺上四方紀念品",
                cells: base
                    .iter()
                    .zip(souvenirs.iter())
                    .map(|(&(dx, dz), &sv)| DreamCell { dx: ox + dx, dy: 1, dz: oz + dz, block: sv })
                    .collect(),
            },
            DreamStage {
                name_zh: "立柱點一盞燈",
                cells: vec![
                    // 展台後方立一根貼地的兩格石磚柱，頂端一盞乙太燈（自地面往上疊，不懸空）。
                    DreamCell { dx: ox, dy: 0, dz: oz - 1, block: Block::StoneBrick },
                    DreamCell { dx: ox, dy: 1, dz: oz - 1, block: Block::StoneBrick },
                    DreamCell { dx: ox, dy: 2, dz: oz - 1, block: Block::AetherLamp },
                ],
            },
        ],
    }
}

// ── 面向玩家的句式（集中一處，i18n 友善；全部截到 BUBBLE_MAX）─────────────────────

fn clip(s: String) -> String {
    s.chars().take(BUBBLE_MAX).collect()
}

/// 立定大夢那一刻、動手放下第一塊時冒的泡泡。
pub fn start_bubble(kind: LifeDreamKind, seed: u64) -> String {
    let goal = kind.title_zh();
    let line = match seed % 3 {
        0 => format!("我想好了——{goal}，就從今天開始"),
        1 => format!("一直放在心上的事，該動手了：{goal}"),
        _ => format!("嗯…{goal}，慢慢來也要把它做成"),
    };
    clip(line)
}

/// 平日回來添一塊時偶爾冒的泡泡（讓玩家看出「她這幾天都在忙同一件事」）。
pub fn work_bubble(kind: LifeDreamKind, stage_name: &str, seed: u64) -> String {
    let _ = kind;
    let line = match seed % 3 {
        0 => format!("今天再往前一點……（{stage_name}）"),
        1 => format!("一塊一塊來，{stage_name}"),
        _ => format!("有空就回來添一手，{stage_name}"),
    };
    clip(line)
}

/// 完成一個階段那一刻冒的泡泡。
pub fn stage_done_bubble(stage_name: &str, seed: u64) -> String {
    let line = match seed % 2 {
        0 => format!("「{stage_name}」總算成了，喘口氣～"),
        _ => format!("這一段做完了：{stage_name}！"),
    };
    clip(line)
}

/// 圓滿全夢那一刻冒的泡泡（一個小圓夢時刻）。
pub fn done_bubble(kind: LifeDreamKind, seed: u64) -> String {
    let line = match (kind, seed % 2) {
        (LifeDreamKind::FlowerGarden, 0) => "我的小花園……真的種成了。".to_string(),
        (LifeDreamKind::FlowerGarden, _) => "看著這一園野花，這幾天沒白忙。".to_string(),
        (LifeDreamKind::LanternPath, 0) => "從家到村子，這一路都亮起來了。".to_string(),
        (LifeDreamKind::LanternPath, _) => "最後一盞也點上了——回家的路不再黑。".to_string(),
        (LifeDreamKind::BiomeShrine, 0) => "四方的紀念品，都擺上這座展台了。".to_string(),
        (LifeDreamKind::BiomeShrine, _) => "我的小展台，總算像個樣子了。".to_string(),
    };
    clip(line)
}

/// 立定大夢時的城鎮動態（不在線上的玩家回來也讀得到某居民立了個志向）。
pub fn start_feed(kind: LifeDreamKind, name: &str) -> String {
    clip(format!("🌱 {name}給自己立了個心願——{}，開始一天天慢慢做起。", kind.title_zh()))
}

/// 完成一個階段時的城鎮動態。
pub fn stage_done_feed(name: &str, stage_name: &str) -> String {
    clip(format!("🪴 {name}把手上的長程心願又推進了一段：{stage_name}。"))
}

/// 圓滿全夢時的城鎮動態（一個小圓夢時刻）。
pub fn done_feed(kind: LifeDreamKind, name: &str) -> String {
    clip(format!("🎉 {name}花了好些天，終於把心願做成了——{}。", kind.title_zh()))
}

/// 立定大夢時寫進記憶的第一人稱內心句（不含玩家名／私密渴望）。
pub fn start_memory(kind: LifeDreamKind) -> String {
    format!("我決定要好好做一件事——{}。也許要花上好幾天，但我想一點一點把它做成。", kind.title_zh())
}

/// 完成一個階段時寫進記憶的第一人稱內心句。
pub fn stage_done_memory(stage_name: &str) -> String {
    format!("今天又往我那件一直在做的事前進了一段：{stage_name}。看著它慢慢成形，心裡踏實。")
}

/// 圓滿全夢時寫進記憶的第一人稱內心句。
pub fn done_memory(kind: LifeDreamKind) -> String {
    format!("我終於把它做成了——{}。這些天一塊一塊慢慢堆起來的，如今就在眼前。這是我親手做成的。", kind.title_zh())
}

// ── 持久化：每居民一份長程專案進度（append-only jsonl，取最高進度重建）───────────────

/// 一筆持久化記錄：某居民的某個夢、已放下幾塊、是否圓滿。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LifeProjectEntry {
    pub resident: String,
    pub kind: String,
    pub placed: u32,
    #[serde(default)]
    pub done: bool,
}

/// 居民長程專案進度帳本（純同步資料結構，由呼叫端包進 `RwLock`）。key = 居民 id。
#[derive(Default, Debug)]
pub struct LifeProjectStore {
    by_res: HashMap<String, LifeProjectEntry>,
}

impl LifeProjectStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 從 append-only 記錄還原：同一居民取**最高 placed** 的那筆（進度只增不減），`done` 取或。
    /// 未知 kind（舊壞資料）安全略過。
    pub fn from_entries(entries: impl IntoIterator<Item = LifeProjectEntry>) -> Self {
        let mut s = Self::new();
        for e in entries {
            if LifeDreamKind::from_id(&e.kind).is_none() {
                continue;
            }
            let done_was = s.by_res.get(&e.resident).map(|p| p.done).unwrap_or(false);
            match s.by_res.get(&e.resident) {
                Some(prev) if prev.placed >= e.placed => {
                    // 既有進度較高：只把 done 併上去（防亂序 append 把已圓夢的旗標蓋回）。
                    if e.done && !prev.done {
                        s.by_res.get_mut(&e.resident).unwrap().done = true;
                    }
                }
                _ => {
                    let mut ne = e;
                    ne.done = ne.done || done_was;
                    s.by_res.insert(ne.resident.clone(), ne);
                }
            }
        }
        s
    }

    /// 查某居民目前的進度記錄（None = 還沒立定任何夢）。
    pub fn get(&self, resident: &str) -> Option<&LifeProjectEntry> {
        self.by_res.get(resident)
    }

    /// 已放下幾塊（沒立定過 → 0）。
    pub fn placed(&self, resident: &str) -> u32 {
        self.by_res.get(resident).map(|p| p.placed).unwrap_or(0)
    }

    /// 是否已圓滿此夢。
    pub fn is_done(&self, resident: &str) -> bool {
        self.by_res.get(resident).map(|p| p.done).unwrap_or(false)
    }

    /// 記下最新進度（記憶體）。回傳這筆記錄的複本，供呼叫端 append 落地。
    /// `placed` 應為單調遞增；`done` 圓滿後不再回退（併入舊值防亂序覆蓋）。
    pub fn record(&mut self, resident: &str, kind: &str, placed: u32, done: bool) -> LifeProjectEntry {
        let done = done || self.by_res.get(resident).map(|p| p.done).unwrap_or(false);
        let entry = LifeProjectEntry {
            resident: resident.to_string(),
            kind: kind.to_string(),
            placed,
            done,
        };
        self.by_res.insert(resident.to_string(), entry.clone());
        entry
    }
}

/// Append 一筆進度記錄。**鐵律**：只在不持任何鎖時呼叫（同步小檔寫、不 await）。
pub fn append_entry(entry: &LifeProjectEntry) {
    if let Ok(line) = serde_json::to_string(entry) {
        write_line(LIFEPROJECT_PATH, &line);
    }
}

/// 載回所有進度記錄（啟動時呼叫一次）。檔不存在／壞行皆容忍。
pub fn load_entries() -> Vec<LifeProjectEntry> {
    let content = match std::fs::read_to_string(LIFEPROJECT_PATH) {
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
                serde_json::from_str::<LifeProjectEntry>(l).ok()
            }
        })
        .collect()
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
        Err(e) => tracing::warn!("無法寫入長程專案進度 {path}: {e}"),
    }
}

// ── 測試 ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_id_roundtrips() {
        for k in [LifeDreamKind::FlowerGarden, LifeDreamKind::LanternPath, LifeDreamKind::BiomeShrine] {
            assert_eq!(LifeDreamKind::from_id(k.id()), Some(k));
        }
        assert_eq!(LifeDreamKind::from_id("bogus"), None);
    }

    #[test]
    fn dream_kind_is_deterministic_and_persona_led() {
        // 同輸入 → 同輸出（確定性、重啟穩定）。
        let a = dream_kind_for(1, name_hash("諾娃"));
        let b = dream_kind_for(1, name_hash("諾娃"));
        assert_eq!(a, b);
        // 農務性格（非偏移名）→ 花園；漫遊 → 點燈；市集 → 展台。挑不會落在 %5==0 偏移的名字驗證主軸。
        let farm_name = pick_name_with_residue(|h| h % 5 != 0);
        assert_eq!(dream_kind_for(1, farm_name), LifeDreamKind::FlowerGarden);
        let wander_name = pick_name_with_residue(|h| h % 5 != 0);
        assert_eq!(dream_kind_for(3, wander_name), LifeDreamKind::LanternPath);
        let market_name = pick_name_with_residue(|h| h % 5 != 0);
        assert_eq!(dream_kind_for(0, market_name), LifeDreamKind::BiomeShrine);
    }

    // 找一個名字雜湊滿足 pred（供上面測試取「非偏移」的樣本）。
    fn pick_name_with_residue(pred: impl Fn(u64) -> bool) -> u64 {
        for i in 0..1000u64 {
            let h = name_hash(&format!("測試{i}"));
            if pred(h) {
                return h;
            }
        }
        panic!("找不到樣本");
    }

    #[test]
    fn name_hash_stable_and_varies() {
        assert_eq!(name_hash("露娜"), name_hash("露娜"));
        assert_ne!(name_hash("露娜"), name_hash("諾娃"));
        assert_ne!(name_hash(""), name_hash("a"));
    }

    #[test]
    fn all_dreams_have_three_stages_and_positive_cells() {
        for k in [LifeDreamKind::FlowerGarden, LifeDreamKind::LanternPath, LifeDreamKind::BiomeShrine] {
            let d = build_dream(k, 1, 0);
            assert_eq!(d.stages.len(), 3, "{:?} 應有三階", k);
            assert!(d.total_cells() >= 8, "{:?} 塊數太少", k);
            for s in &d.stages {
                assert!(!s.cells.is_empty());
                assert!(!s.name_zh.is_empty());
            }
        }
    }

    #[test]
    fn cell_at_covers_exactly_total_then_none() {
        let d = build_dream(LifeDreamKind::FlowerGarden, 0, 0);
        let total = d.total_cells();
        for i in 0..total {
            assert!(d.cell_at(i).is_some(), "第 {i} 塊應存在");
        }
        assert!(d.cell_at(total).is_none(), "越界應 None");
    }

    #[test]
    fn stage_boundaries_and_stage_of_agree() {
        let d = build_dream(LifeDreamKind::BiomeShrine, 0, 0);
        let bounds = d.stage_boundaries();
        assert_eq!(bounds.len(), 3);
        assert_eq!(*bounds.last().unwrap(), d.total_cells());
        // 每一塊的 stage_of 落在正確區間。
        for i in 0..d.total_cells() {
            let si = d.stage_of(i).unwrap();
            let lo = if si == 0 { 0 } else { bounds[si - 1] };
            let hi = bounds[si];
            assert!(i >= lo && i < hi, "第 {i} 塊落錯階");
        }
        assert!(d.stage_of(d.total_cells()).is_none());
    }

    #[test]
    fn stage_completed_by_fires_only_on_boundaries() {
        let d = build_dream(LifeDreamKind::LanternPath, 1, 0);
        let bounds = d.stage_boundaries(); // 例如 [3, 6, 9]
        // 恰好放完某階最後一塊時回該階索引，其餘 None。
        for placed in 1..=d.total_cells() {
            let expect = bounds.iter().position(|&b| b == placed);
            assert_eq!(d.stage_completed_by(placed), expect, "placed={placed}");
        }
        assert!(d.is_complete(d.total_cells()));
        assert!(!d.is_complete(d.total_cells() - 1));
    }

    #[test]
    fn lantern_path_extends_along_direction_and_never_stacks() {
        let d = build_dream(LifeDreamKind::LanternPath, 1, 0);
        let mut seen = std::collections::HashSet::new();
        for i in 0..d.total_cells() {
            let c = d.cell_at(i).unwrap();
            assert_eq!(c.block, Block::AetherLamp);
            // 每盞燈位置互異（不原地疊）。
            assert!(seen.insert((c.dx, c.dy, c.dz)), "第 {i} 盞燈重疊");
            // 沿 +x 延伸、z 不動。
            assert_eq!(c.dz, 0);
            assert!(c.dx > 0);
        }
    }

    #[test]
    fn lantern_path_zero_direction_defaults_and_does_not_stack() {
        // 沒有村莊中心（0,0）→ 預設朝 +x，仍不原地疊燈。
        let d = build_dream(LifeDreamKind::LanternPath, 0, 0);
        let mut seen = std::collections::HashSet::new();
        for i in 0..d.total_cells() {
            let c = d.cell_at(i).unwrap();
            assert!(seen.insert((c.dx, c.dz)), "退化成原地疊燈了");
        }
    }

    #[test]
    fn all_bubbles_and_feeds_bounded_and_nonempty() {
        for k in [LifeDreamKind::FlowerGarden, LifeDreamKind::LanternPath, LifeDreamKind::BiomeShrine] {
            for seed in 0..6u64 {
                for line in [
                    start_bubble(k, seed),
                    work_bubble(k, "翻好一畦土", seed),
                    stage_done_bubble("翻好一畦土", seed),
                    done_bubble(k, seed),
                ] {
                    assert!(!line.is_empty());
                    assert!(line.chars().count() <= BUBBLE_MAX);
                    assert!(!line.contains('\n'));
                }
            }
            // Feed 也截斷、不換行。
            assert!(start_feed(k, "露娜").chars().count() <= BUBBLE_MAX);
            assert!(done_feed(k, "露娜").chars().count() <= BUBBLE_MAX);
            assert!(!stage_done_feed("露娜", "翻好一畦土").is_empty());
        }
    }

    #[test]
    fn memory_lines_are_first_person_no_leak() {
        for k in [LifeDreamKind::FlowerGarden, LifeDreamKind::LanternPath, LifeDreamKind::BiomeShrine] {
            for line in [start_memory(k), stage_done_memory("翻好一畦土"), done_memory(k)] {
                assert!(!line.is_empty());
                assert!(!line.contains('\n'));
                // 不外洩玩家名占位符之類。
                assert!(!line.contains('{'));
            }
        }
    }

    #[test]
    fn store_from_entries_takes_highest_placed() {
        let entries = vec![
            LifeProjectEntry { resident: "vox_res_0".into(), kind: "flower_garden".into(), placed: 1, done: false },
            LifeProjectEntry { resident: "vox_res_0".into(), kind: "flower_garden".into(), placed: 5, done: false },
            LifeProjectEntry { resident: "vox_res_0".into(), kind: "flower_garden".into(), placed: 3, done: false },
        ];
        let s = LifeProjectStore::from_entries(entries);
        assert_eq!(s.placed("vox_res_0"), 5, "取最高進度");
        assert!(!s.is_done("vox_res_0"));
    }

    #[test]
    fn store_done_flag_never_regresses() {
        // 亂序：先高進度+done，後低進度未done——done 不該被蓋回。
        let entries = vec![
            LifeProjectEntry { resident: "r".into(), kind: "lantern_path".into(), placed: 9, done: true },
            LifeProjectEntry { resident: "r".into(), kind: "lantern_path".into(), placed: 2, done: false },
        ];
        let s = LifeProjectStore::from_entries(entries);
        assert_eq!(s.placed("r"), 9);
        assert!(s.is_done("r"), "圓夢旗標不該被亂序 append 蓋回");
    }

    #[test]
    fn store_skips_unknown_kind() {
        let entries = vec![
            LifeProjectEntry { resident: "r".into(), kind: "not_a_dream".into(), placed: 3, done: false },
        ];
        let s = LifeProjectStore::from_entries(entries);
        assert_eq!(s.placed("r"), 0);
        assert!(s.get("r").is_none());
    }

    #[test]
    fn store_record_updates_and_keeps_done() {
        let mut s = LifeProjectStore::new();
        let e1 = s.record("r", "flower_garden", 1, false);
        assert_eq!(e1.placed, 1);
        assert!(!e1.done);
        s.record("r", "flower_garden", 14, true);
        assert!(s.is_done("r"));
        // 圓夢後再記一筆未done（不該回退）。
        let e3 = s.record("r", "flower_garden", 14, false);
        assert!(e3.done, "record 併入既有 done，不回退");
        assert!(s.is_done("r"));
    }

    #[test]
    fn empty_store_defaults() {
        let s = LifeProjectStore::new();
        assert_eq!(s.placed("nobody"), 0);
        assert!(!s.is_done("nobody"));
        assert!(s.get("nobody").is_none());
    }

    // ── 圓夢地標 v1.1 ────────────────────────────────────────────────────────

    #[test]
    fn anchor_cell_is_first_cell_for_every_dream_kind() {
        for k in [LifeDreamKind::FlowerGarden, LifeDreamKind::LanternPath, LifeDreamKind::BiomeShrine] {
            let d = build_dream(k, 1, 0);
            assert_eq!(d.anchor_cell(), d.cell_at(0).unwrap(), "{:?} 錨點應為第一塊", k);
        }
    }

    #[test]
    fn near_dream_landmark_within_radius_true_outside_false() {
        assert!(near_dream_landmark(100.0, 200.0, 100, 200), "站在錨點正上方應算抵達");
        assert!(near_dream_landmark(103.0, 200.0, 100, 200), "半徑內應算抵達");
        assert!(!near_dream_landmark(200.0, 200.0, 100, 200), "遠在半徑外不該誤判");
    }

    #[test]
    fn near_dream_landmark_boundary_is_inclusive() {
        // 剛好貼著半徑邊界（dx=DISCOVER_RADIUS, dz=0 → dx²+dz²=16=DISCOVER_RADIUS²）。
        assert!(near_dream_landmark(104.0, 0.0, 100, 0), "邊界應視為抵達（<=）");
        assert!(!near_dream_landmark(104.01, 0.0, 100, 0), "剛超出邊界不該誤判");
    }
}
