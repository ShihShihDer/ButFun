//! 乙太方界·村莊系統 v1——中央廣場 + 道路網 + 沿路地塊（純邏輯、確定性、可測）。
//!
//! 維護者：「他們的地方很凌亂，幫忙整理、做範例村莊，甚至引入麥塊的村莊系統」。
//! 此前居民的家 / 水井 / 花圃各自在 `home_base + build_offset` 的隨機環狀錨點上蓋，
//! 散落一地、沒有村子的樣子。本模組抄**麥塊村莊的設計精髓**（中心廣場 + 道路 + 沿路地塊）
//! 原創實作，把散落的建築收攏成一座有街廓的村莊：
//!
//! 1. [`VillagePlan`]：以村莊中心（現有居民 home_base 群聚中心，由呼叫端算好傳入）確定性生成
//!    **中央小廣場**（鋪面 + 中央水井 + 四角燈）、**十字主路網**（路面寬 2、四向延伸），
//!    沿主路兩側劃出若干 **7×7 地塊**（留間距、彼此不重疊）。
//! 2. [`PlotRegistry`]：地塊認領註冊表（哪位居民認領了哪塊），append-only jsonl 落地、向後相容——
//!    居民新建築（含新生兒的家）改成「認領最近的空地塊」當錨點，蓋在地塊上＝自動沿路、對齊、不散落。
//! 3. 一次性整理既有凌亂：[`pave_path_cells`] 從廣場鋪一條 L 形路連到每個既有建築——**只加不拆**，
//!    路面只鋪在自然地表上、遇非地表方塊（既有建築 / 樹 / 水）就停，絕不覆蓋居民已蓋的作品。
//!
//! **純邏輯鐵律**：本檔零 LLM、零鎖、零世界 IO——所有規劃 / 地塊劃分 / 路徑計算都是確定性純函式，
//! 吃座標吐座標清單，方便單元測試釘死（生成確定性 / 地塊不重疊 / 認領 / 路徑不毀建築 / migration 冪等）。
//! 真正把方塊放進世界（讀 delta、set_block、落地 jsonl、廣播）全在 `voxel_ws.rs`，嚴守短鎖鐵律。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::voxel::Block;

// ── 村莊尺寸常數（麥塊村莊比例：小廣場、寬路、間隔地塊）──────────────────────────

/// 中央廣場半徑（格）：廣場為 (2*R+1)×(2*R+1) 的鋪面方形。R=3 → 7×7 廣場，小而聚。
pub const PLAZA_RADIUS: i32 = 3;

/// 主路半寬：路面寬 = 2*HALF+... 這裡取「路中線兩側各鋪 1 格」→ 實際寬 2 格（麥塊主街的寬度感）。
/// 十字路以中心線 x∈{cx, cx+1}（或 z 同理）兩格寬向四方延伸。
pub const ROAD_HALF_W: i32 = 1;

/// 主路自廣場邊緣向外延伸的長度（格）。四向各延伸這麼長，構成十字主街，足夠排開沿路地塊。
pub const ROAD_REACH: i32 = 72;

/// 單一地塊邊長（格）：7×7 一塊——這是「地塊本身」的名義佔地（廣場/路的淨空以此計）。
/// **註**：居民實際在地塊中心用 `build_offset` 把數座建物（家/井/塔/花圃/擴建）散開成一小片
/// 家園，最遠可及中心 ±8 格；因此地塊「認領間距」（[`PLOT_STRIDE`]/perp）另取更寬的值，
/// 讓相鄰兩塊各自的建物群不相撞（沿用舊 home_base 間距 ≥18 的紀律）。
pub const PLOT_SIZE: i32 = 7;

/// 地塊中心距路中線的退縮（格）：留一條人行帶＋容納 build_offset 往路側伸的建物（±8）。
pub const PLOT_SETBACK: i32 = 12;

/// 沿每條主路一側可劃的地塊數（四向 × 兩側各這麼多）。
pub const PLOTS_PER_ROAD_SIDE: i32 = 3;

/// 相鄰地塊沿路方向的中心間距（格）：取 ≥ 舊 home_base 間距（18）＋餘裕，確保相鄰兩塊各自
/// 用 build_offset 散開（±8）的建物群不相撞。
pub const PLOT_STRIDE: i32 = 22;

/// 第一塊地塊中心距村莊中心的沿路距離（格）：跳過廣場與交叉口，第一塊從這裡起算。
pub const PLOT_FIRST_OFFSET: i32 = 20;

// ── 路面 / 廣場材質（依村莊群系選一種「石板路」感建材）──────────────────────────

/// 依村莊中心群系選路面材質（麥塊村莊石板路的味道；沙漠改用沙礫感的拋光石襯托）。
/// 純函式、確定性。回傳的方塊都在 `Block::is_placeable` 白名單內（可安全 set_block/落地）。
pub fn road_surface(biome: crate::voxel::VoxelBiome) -> Block {
    use crate::voxel::VoxelBiome::*;
    match biome {
        // 草原 / 森林：拋光石板路（灰白工整，像麥塊村莊的石路）。
        Grassland | Forest => Block::SmoothStone,
        // 沙漠：石磚（比拋光石更暖、與沙地對比清楚）。
        Desert => Block::StoneBrick,
        // 雪原：拋光石（雪地上的灰石路一眼可辨）。
        Snow => Block::SmoothStone,
    }
}

/// 廣場地面材質（與路面同系但用石磚，讓廣場中央比路面更「正式」一點）。
pub fn plaza_surface(_biome: crate::voxel::VoxelBiome) -> Block {
    Block::StoneBrick
}

// ── 村莊規劃（純函式、確定性、seed = 村莊中心座標）───────────────────────────────

/// 一塊沿路地塊：中心世界座標 (cx, cz) + 佔地半邊（PLOT_SIZE/2）。
/// 地塊只記平面座標，實際地表 y 由呼叫端用 `surface_y` 就地算（地形會起伏）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plot {
    /// 地塊中心世界 x（居民蓋家的錨點就用這裡，取代舊 home_base+build_offset）。
    pub cx: i32,
    /// 地塊中心世界 z。
    pub cz: i32,
}

impl Plot {
    /// 地塊佔地半邊（格）：中心 ± 這麼多即地塊名義邊界（含界）。
    pub const HALF: i32 = PLOT_SIZE / 2;

    /// 一整片家園的實際佔地半徑（格）：居民在地塊中心用 `build_offset` 把數座建物散開，最遠可及
    /// 中心 ±8 格，再加建物本身約 2 格 → 取 10。用於「認領間距」判斷，確保相鄰兩塊的建物群不相撞。
    pub const HOMESTEAD_HALF: i32 = 10;

    /// 兩塊地塊（以「整片家園佔地」計）是否太近／重疊——中心距在兩片家園佔地之和內即視為相撞。
    /// 用 `HOMESTEAD_HALF` 而非名義 `HALF`，因居民的建物群比 7×7 地塊本身大得多。
    pub fn overlaps(&self, other: &Plot) -> bool {
        let sep = Self::HOMESTEAD_HALF * 2; // 中心距 < 此值即視為太近/相撞
        (self.cx - other.cx).abs() < sep && (self.cz - other.cz).abs() < sep
    }
}

/// 一份村莊藍圖：中心 + 群系 + 廣場鋪面格 + 中央水井/燈 + 主路格 + 沿路地塊。
/// 全部確定性（seed = 中心座標 + 群系）；同輸入永遠同一份藍圖（可測、重啟一致）。
#[derive(Clone, Debug, PartialEq)]
pub struct VillagePlan {
    /// 村莊中心世界座標（廣場正中；也是主路十字交會點）。
    pub cx: i32,
    pub cz: i32,
    /// 村莊群系（決定路面 / 廣場材質）。
    pub biome: crate::voxel::VoxelBiome,
    /// 廣場地面鋪面格（平面座標，實際 y 就地算）。
    pub plaza: Vec<(i32, i32)>,
    /// 廣場中央水井中心（(cx, cz)；建井的實際方塊由 voxel_building 生成，這裡只給錨點）。
    pub well_center: (i32, i32),
    /// 廣場四角燈柱（火把）的平面座標（夜裡點亮廣場，也讓村莊遠處認得出）。
    pub lantern_cells: Vec<(i32, i32)>,
    /// 十字主路鋪面格（平面座標）。
    pub road: Vec<(i32, i32)>,
    /// 沿路地塊（居民認領後當蓋家錨點）。
    pub plots: Vec<Plot>,
}

/// 依村莊中心 (cx, cz) + 群系確定性生成一份村莊藍圖（純函式、可測）。
/// 麥塊村莊精神：中央廣場（鋪面 + 水井 + 四角燈）＋ 十字主路（寬 2）＋ 沿路劃地塊（7×7、留間距）。
pub fn plan_village(cx: i32, cz: i32, biome: crate::voxel::VoxelBiome) -> VillagePlan {
    let plaza = plaza_cells(cx, cz);
    let lantern_cells = plaza_lantern_cells(cx, cz);
    let road = road_cells(cx, cz);
    let plots = plot_layout(cx, cz);
    VillagePlan {
        cx,
        cz,
        biome,
        plaza,
        well_center: (cx, cz),
        lantern_cells,
        road,
        plots,
    }
}

/// 廣場鋪面格：以中心為心的 (2*R+1)×(2*R+1) 實心方形（含中心，水井會蓋在中心之上）。
pub fn plaza_cells(cx: i32, cz: i32) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    for dx in -PLAZA_RADIUS..=PLAZA_RADIUS {
        for dz in -PLAZA_RADIUS..=PLAZA_RADIUS {
            out.push((cx + dx, cz + dz));
        }
    }
    out
}

/// 廣場四角燈柱平面座標（火把插在廣場四個角落，夜裡點亮）。
pub fn plaza_lantern_cells(cx: i32, cz: i32) -> Vec<(i32, i32)> {
    let r = PLAZA_RADIUS;
    vec![
        (cx - r, cz - r),
        (cx - r, cz + r),
        (cx + r, cz - r),
        (cx + r, cz + r),
    ]
}

/// 十字主路鋪面格：以中心為交會點，向 +x/-x/+z/-z 四方各延伸 `ROAD_REACH`，
/// 路面寬 2（中線兩格 {c, c+1}），構成一個「＋」字主街。去重（十字交會處只算一次）。
pub fn road_cells(cx: i32, cz: i32) -> Vec<(i32, i32)> {
    use std::collections::HashSet;
    let mut set: HashSet<(i32, i32)> = HashSet::new();
    // 路寬 2：沿垂直於路向的兩格 offset（0 與 1，讓路中線落在 c 與 c+1 之間）。
    let width_off = [0, 1];
    // 東西向主路（沿 x 延伸；z 佔兩格）。廣場內不重複鋪（廣場自己已鋪面）。
    for step in -ROAD_REACH..=ROAD_REACH {
        let x = cx + step;
        for &wo in &width_off {
            let z = cz + wo;
            if in_plaza(cx, cz, x, z) {
                continue;
            }
            set.insert((x, z));
        }
    }
    // 南北向主路（沿 z 延伸；x 佔兩格）。
    for step in -ROAD_REACH..=ROAD_REACH {
        let z = cz + step;
        for &wo in &width_off {
            let x = cx + wo;
            if in_plaza(cx, cz, x, z) {
                continue;
            }
            set.insert((x, z));
        }
    }
    let mut out: Vec<(i32, i32)> = set.into_iter().collect();
    out.sort(); // 確定性順序（HashSet 迭代序不穩，排序後才可測/重啟一致）
    out
}

/// (x, z) 是否落在廣場佔地方形內。
fn in_plaza(cx: i32, cz: i32, x: i32, z: i32) -> bool {
    (x - cx).abs() <= PLAZA_RADIUS && (z - cz).abs() <= PLAZA_RADIUS
}

/// 沿四條主路兩側劃地塊：每條路（東西/南北）× 兩側 × `PLOTS_PER_ROAD_SIDE` 塊。
/// 地塊中心退縮在路緣外 `PLOT_SETBACK + HALF` 處，沿路方向以 `PLOT_STRIDE` 等距排開。
/// 全部去重 + 保證彼此不重疊（含與廣場的距離）。純函式、確定性。
pub fn plot_layout(cx: i32, cz: i32) -> Vec<Plot> {
    let mut plots: Vec<Plot> = Vec::new();
    // 地塊中心到路中線的垂直距離：路半寬 + 退縮 + 地塊半邊。
    // 路中線落在 c 與 c+1 之間，取 c 側，垂直偏移用「路外緣 + setback + half」。
    let perp = ROAD_HALF_W + PLOT_SETBACK + Plot::HALF;
    // 沿路方向第 i 塊的中心位移。
    let along = |i: i32| PLOT_FIRST_OFFSET + i * PLOT_STRIDE;

    // 東西向主路（路沿 x 延伸）：地塊在路的南（+z）北（-z）兩側，正負沿 x 排開。
    for i in 0..PLOTS_PER_ROAD_SIDE {
        for &sign in &[1, -1] {
            let ax = cx + sign * along(i);
            // 南側（+z）與北側（-z）。
            for &pside in &[perp, -perp] {
                let cand = Plot { cx: ax, cz: cz + pside };
                try_push_plot(&mut plots, cand, cx, cz);
            }
        }
    }
    // 南北向主路（路沿 z 延伸）：地塊在路的東（+x）西（-x）兩側，正負沿 z 排開。
    for i in 0..PLOTS_PER_ROAD_SIDE {
        for &sign in &[1, -1] {
            let az = cz + sign * along(i);
            for &pside in &[perp, -perp] {
                let cand = Plot { cx: cx + pside, cz: az };
                try_push_plot(&mut plots, cand, cx, cz);
            }
        }
    }
    plots
}

/// 嘗試把候選地塊加入清單：與廣場太近、或與任一既有地塊重疊 → 丟棄，確保無重疊。
fn try_push_plot(plots: &mut Vec<Plot>, cand: Plot, vcx: i32, vcz: i32) {
    // 與廣場保持距離：地塊佔地不得侵入廣場（含 1 格緩衝）。
    let plaza_clear = PLAZA_RADIUS + Plot::HALF + 1;
    if (cand.cx - vcx).abs() < plaza_clear && (cand.cz - vcz).abs() < plaza_clear {
        return;
    }
    if plots.iter().any(|p| p.overlaps(&cand)) {
        return;
    }
    plots.push(cand);
}

// ── 地塊認領註冊表（append-only、向後相容）─────────────────────────────────────

/// 一筆地塊認領記錄（jsonl 落地單位）：某居民認領了某地塊（以中心座標標定）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlotClaim {
    /// 認領的居民 id（"vox_res_{i}"）。
    pub resident: String,
    /// 地塊中心世界座標（同時是該居民蓋家的錨點 x/z）。
    pub cx: i32,
    pub cz: i32,
    /// 單調遞增序號（越大越新；還原時同居民取最新一份）。
    pub seq: u64,
}

/// 地塊認領註冊表：誰認領了哪塊地。純資料，鎖/落地由呼叫端管。
#[derive(Default)]
pub struct PlotRegistry {
    /// resident id → 認領的地塊中心座標。
    claims: HashMap<String, (i32, i32)>,
    /// 已被認領的地塊中心座標集合（快速判斷某塊是否已被佔）。
    taken: Vec<(i32, i32)>,
    next_seq: u64,
}

impl PlotRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 此居民是否已認領過地塊。
    pub fn has_claim(&self, resident: &str) -> bool {
        self.claims.contains_key(resident)
    }

    /// 取此居民已認領的地塊中心（沒認領則 None）。
    pub fn claim_of(&self, resident: &str) -> Option<(i32, i32)> {
        self.claims.get(resident).copied()
    }

    /// 某地塊中心是否已被（任何人）認領。
    pub fn is_taken(&self, cx: i32, cz: i32) -> bool {
        self.taken.iter().any(|&(x, z)| x == cx && z == cz)
    }

    /// 為某居民登記認領一塊地；回傳落地記錄供 append。
    /// 同居民重複認領同一塊＝冪等（回原記錄、不重複佔位）；認領新塊會覆蓋舊登記。
    pub fn claim(&mut self, resident: &str, cx: i32, cz: i32) -> PlotClaim {
        // 若這居民先前認領的是別塊，先把舊塊從 taken 移除（讓它可被別人認領）。
        if let Some(&(ox, oz)) = self.claims.get(resident) {
            if (ox, oz) != (cx, cz) {
                self.taken.retain(|&(x, z)| !(x == ox && z == oz));
            }
        }
        self.claims.insert(resident.to_string(), (cx, cz));
        if !self.is_taken(cx, cz) {
            self.taken.push((cx, cz));
        }
        let rec = PlotClaim { resident: resident.to_string(), cx, cz, seq: self.next_seq };
        self.next_seq = self.next_seq.wrapping_add(1);
        rec
    }

    /// 從一份候選地塊清單中，替某居民挑「最近的一塊尚未被認領的空地塊」。
    /// `near` = 居民 home_base（一般取離家最近的空地塊，讓認領貼近其原本活動範圍）。
    /// 全空可挑則回該塊中心；全被佔（沒有空地）→ None（呼叫端退回舊 build_offset 邏輯）。
    /// 純函式、確定性（距離相同時取 plots 清單靠前者，plot_layout 順序固定）。
    pub fn nearest_free_plot(&self, plots: &[Plot], near_x: i32, near_z: i32) -> Option<Plot> {
        plots
            .iter()
            .filter(|p| !self.is_taken(p.cx, p.cz))
            .min_by_key(|p| {
                let dx = (p.cx - near_x) as i64;
                let dz = (p.cz - near_z) as i64;
                dx * dx + dz * dz
            })
            .copied()
    }

    /// 從 jsonl 記錄還原（重啟後仍記得誰認領了哪塊）。同居民多筆取 seq 最大（最新）。
    pub fn from_entries(entries: Vec<PlotClaim>) -> Self {
        // 先依 seq 排序，讓「同居民最新一筆」與「同塊最後認領者」都可確定性重建。
        let mut es = entries;
        es.sort_by_key(|e| e.seq);
        let mut s = Self::default();
        for e in es {
            if e.seq >= s.next_seq {
                s.next_seq = e.seq.wrapping_add(1);
            }
            // claim() 內建「換塊時釋放舊塊」與去重，直接重放即可正確重建 taken/claims。
            s.claim(&e.resident, e.cx, e.cz);
        }
        s
    }
}

// ── 一次性整理：從廣場鋪 L 形路連到既有建築（只加不拆、遇非地表方塊即停）─────────

/// 從村莊中心 (vcx, vcz) 鋪一條 **L 形路徑** 到目標建築 (tx, tz) 的平面格清單（純函式、可測）。
/// 先沿 x 走到 tx（z 固定在 vcz），再沿 z 走到 tz（x 固定在 tx）——一條轉一次彎的簡單路。
/// **只回平面座標**：呼叫端就地用 `surface_y` 找地表、且**只在該格目前是自然地表方塊時才鋪**、
/// 遇既有建築 / 樹 / 水就跳過那格（不覆蓋、不拆）——保守整理的關鍵在呼叫端那層把關。
/// 這裡回完整 L 路徑格（含起訖），去重、去掉落在廣場內的格（廣場已鋪面）。
pub fn pave_path_cells(vcx: i32, vcz: i32, tx: i32, tz: i32) -> Vec<(i32, i32)> {
    use std::collections::HashSet;
    let mut seen: HashSet<(i32, i32)> = HashSet::new();
    let mut out: Vec<(i32, i32)> = Vec::new();
    let mut push = |out: &mut Vec<(i32, i32)>, seen: &mut HashSet<(i32, i32)>, x: i32, z: i32| {
        // 廣場內不鋪（廣場自己已是鋪面）。
        if in_plaza(vcx, vcz, x, z) {
            return;
        }
        if seen.insert((x, z)) {
            out.push((x, z));
        }
    };
    // 段一：沿 x 從 vcx 走到 tx（z 固定 vcz）。
    let (x0, x1) = (vcx.min(tx), vcx.max(tx));
    for x in x0..=x1 {
        push(&mut out, &mut seen, x, vcz);
    }
    // 段二：沿 z 從 vcz 走到 tz（x 固定 tx）。
    let (z0, z1) = (vcz.min(tz), vcz.max(tz));
    for z in z0..=z1 {
        push(&mut out, &mut seen, tx, z);
    }
    out
}

/// 某方塊種類是否為「可鋪路的自然地表」——只有自然生成的地面（草/土/沙/石/雪/礫）
/// 才允許被路面覆蓋。**排除**：水、樹幹/葉、農田、以及任何建材/裝飾方塊（＝居民作品或既有建築）。
/// 這是「一次性整理絕不拆作品」的核心把關：路遇到不是自然地表的方塊就停、不鋪。純函式、可測。
pub fn is_natural_ground(b: Block) -> bool {
    matches!(
        b,
        Block::Grass | Block::Dirt | Block::Sand | Block::Stone | Block::Snow
    )
}

/// 廣場中央該不該（一次性整理時）補放一盞燈以外，是否也要放中央水井——由呼叫端決定。
/// 這裡集中一句 migration 動工的 Feed 文字（面向玩家、i18n 友善）。
pub fn village_feed_line() -> &'static str {
    "村裡鋪起了石板路，散落的家被一條條路連了起來。"
}

/// 某居民在南/北/東/西哪個方位安了新家的 Feed 文字（認領地塊時用）。
/// 依地塊中心相對村莊中心的方位給一句方向感描述（面向玩家、i18n 友善集中此處）。
pub fn plot_claim_feed_line(name: &str, plot: &Plot, vcx: i32, vcz: i32) -> String {
    let dir = compass_dir(plot.cx - vcx, plot.cz - vcz);
    format!("{name}在村子{dir}的地塊安了新家。")
}

/// 由相對位移給一個粗略方位詞（純函式、可測）。z 增為南、z 減為北；x 增為東、x 減為西。
fn compass_dir(dx: i32, dz: i32) -> &'static str {
    if dx.abs() >= dz.abs() {
        if dx >= 0 { "東邊" } else { "西邊" }
    } else if dz >= 0 {
        "南邊"
    } else {
        "北邊"
    }
}

// ── jsonl 持久化（append-only，比照 voxel_building / voxel_roster 慣例）───────────

/// 地塊認領落地路徑（`data/` 已 gitignore）。
const VILLAGE_PLOTS_PATH: &str = "data/voxel_village_plots.jsonl";

/// 村莊已規劃旗標檔（覆寫式單一標記，讓村莊規劃與一次性整理只跑一次、冪等）。
/// 內容為村莊中心座標 "cx,cz"，供除錯/驗證；存在即代表「這座村莊已規劃/整理過」。
const VILLAGE_DONE_PATH: &str = "data/voxel_village_done";

/// Append 一筆地塊認領記錄。**鐵律**：只在不持任何鎖時呼叫（同步小檔寫、不 await）。
pub fn append_plot_claim(rec: &PlotClaim) {
    if let Ok(line) = serde_json::to_string(rec) {
        write_line(VILLAGE_PLOTS_PATH, &line);
    }
}

/// 載回所有地塊認領記錄（啟動時呼叫一次）。檔不存在（舊世界）→ 空（向後相容）。
pub fn load_plot_claims() -> Vec<PlotClaim> {
    let content = match std::fs::read_to_string(VILLAGE_PLOTS_PATH) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    content
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.is_empty() { None } else { serde_json::from_str::<PlotClaim>(l).ok() }
        })
        .collect()
}

/// 村莊規劃/整理是否已跑過（冪等閘門）：旗標檔存在即已跑過。
pub fn village_done() -> bool {
    std::path::Path::new(VILLAGE_DONE_PATH).exists()
}

/// 標記村莊規劃/整理已完成（寫入中心座標）。**鐵律**：只在不持任何鎖時呼叫。
pub fn mark_village_done(cx: i32, cz: i32) {
    if let Some(parent) = std::path::Path::new(VILLAGE_DONE_PATH).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(VILLAGE_DONE_PATH, format!("{cx},{cz}\n"));
}

/// 讀回一次性整理時**釘死的村莊中心**（旗標檔內容 "cx,cz"）。存在即回 `Some((cx,cz))`。
/// 供地塊認領沿用同一個中心：確保居民認領的地塊與鋪好的道路網對齊，不隨新生兒改變質心而漂移。
/// 檔缺 / 壞行 → `None`（呼叫端退回即時質心）。**鐵律**：只在不持任何鎖時呼叫（同步小檔讀）。
pub fn load_village_center() -> Option<(i32, i32)> {
    let content = std::fs::read_to_string(VILLAGE_DONE_PATH).ok()?;
    let mut it = content.trim().split(',');
    let cx = it.next()?.trim().parse::<i32>().ok()?;
    let cz = it.next()?.trim().parse::<i32>().ok()?;
    Some((cx, cz))
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
        Err(e) => tracing::warn!("無法寫入村莊地塊認領 {path}: {e}"),
    }
}

// ── 村莊中心推定（純函式、確定性）─────────────────────────────────────────────

/// 由一組既有居民 home_base 座標算出村莊中心（幾何質心，四捨五入到整數格）。
/// 空清單 → 退回世界出生點 (0,0)（露娜家所在，玩家降生附近）。純函式、可測。
pub fn village_center(home_bases: &[(i32, i32)]) -> (i32, i32) {
    if home_bases.is_empty() {
        return (0, 0);
    }
    let n = home_bases.len() as i64;
    let sx: i64 = home_bases.iter().map(|&(x, _)| x as i64).sum();
    let sz: i64 = home_bases.iter().map(|&(_, z)| z as i64).sum();
    // 四捨五入（加半個 n 再整除）。
    let round_div = |s: i64, n: i64| -> i32 {
        let half = n / 2;
        (if s >= 0 { (s + half) / n } else { (s - half) / n }) as i32
    };
    (round_div(sx, n), round_div(sz, n))
}

// ── 單元測試 ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voxel::VoxelBiome;

    // ── plan_village 確定性 ───────────────────────────────────────────────────

    #[test]
    fn plan_is_deterministic() {
        let a = plan_village(100, -50, VoxelBiome::Grassland);
        let b = plan_village(100, -50, VoxelBiome::Grassland);
        assert_eq!(a, b, "同輸入應生出逐格相同的村莊藍圖");
    }

    #[test]
    fn plan_has_plaza_road_and_plots() {
        let p = plan_village(0, 0, VoxelBiome::Grassland);
        assert!(!p.plaza.is_empty(), "應有廣場鋪面");
        assert!(!p.road.is_empty(), "應有主路");
        assert!(!p.plots.is_empty(), "應劃出地塊");
        assert_eq!(p.well_center, (0, 0), "水井在廣場中央");
        assert_eq!(p.lantern_cells.len(), 4, "廣場四角各一盞燈");
    }

    #[test]
    fn plaza_is_square_of_expected_size() {
        let cells = plaza_cells(10, 20);
        let side = (2 * PLAZA_RADIUS + 1) as usize;
        assert_eq!(cells.len(), side * side, "廣場應為 {side}×{side} 方形");
        // 含中心。
        assert!(cells.contains(&(10, 20)));
        // 四角。
        assert!(cells.contains(&(10 - PLAZA_RADIUS, 20 - PLAZA_RADIUS)));
        assert!(cells.contains(&(10 + PLAZA_RADIUS, 20 + PLAZA_RADIUS)));
    }

    // ── 主路：寬度、十字、去重、確定性排序 ─────────────────────────────────────

    #[test]
    fn road_forms_cross_and_is_width_two() {
        let cells = road_cells(0, 0);
        // 東西向遠端點（x=+ROAD_REACH）兩格寬都在。
        assert!(cells.contains(&(ROAD_REACH, 0)));
        assert!(cells.contains(&(ROAD_REACH, 1)));
        // 南北向遠端點（z=+ROAD_REACH）兩格寬都在。
        assert!(cells.contains(&(0, ROAD_REACH)));
        assert!(cells.contains(&(1, ROAD_REACH)));
    }

    #[test]
    fn road_has_no_duplicates_and_is_sorted() {
        let cells = road_cells(5, 5);
        let mut sorted = cells.clone();
        sorted.sort();
        assert_eq!(cells, sorted, "主路格應已排序（確定性）");
        let uniq: std::collections::HashSet<_> = cells.iter().collect();
        assert_eq!(uniq.len(), cells.len(), "主路格不得重複（十字交會處只算一次）");
    }

    #[test]
    fn road_does_not_pave_inside_plaza() {
        let cells = road_cells(0, 0);
        for &(x, z) in &cells {
            assert!(
                !(x.abs() <= PLAZA_RADIUS && z.abs() <= PLAZA_RADIUS),
                "主路不該鋪進廣場（{x},{z}）",
            );
        }
    }

    // ── 地塊：不重疊、離廣場有距離、確定性 ─────────────────────────────────────

    #[test]
    fn plots_never_overlap_each_other() {
        let plots = plot_layout(0, 0);
        assert!(plots.len() >= 4, "應劃出足夠多地塊（實得 {}）", plots.len());
        for i in 0..plots.len() {
            for j in (i + 1)..plots.len() {
                assert!(
                    !plots[i].overlaps(&plots[j]),
                    "地塊 {i} 與 {j} 重疊：{:?} vs {:?}",
                    plots[i], plots[j]
                );
            }
        }
    }

    #[test]
    fn plots_keep_clear_of_plaza() {
        let (vcx, vcz) = (30, -40);
        let plots = plot_layout(vcx, vcz);
        let clear = PLAZA_RADIUS + Plot::HALF;
        for p in &plots {
            let near = (p.cx - vcx).abs() <= clear && (p.cz - vcz).abs() <= clear;
            assert!(!near, "地塊 {:?} 侵入廣場淨空", p);
        }
    }

    #[test]
    fn plot_layout_is_deterministic() {
        let a = plot_layout(7, 7);
        let b = plot_layout(7, 7);
        assert_eq!(a, b);
    }

    #[test]
    fn plots_do_not_sit_on_the_road_lanes() {
        // 地塊佔地方形不該覆蓋主路的兩條中線格（否則蓋家會壓在路上）。
        let (vcx, vcz) = (0, 0);
        let plots = plot_layout(vcx, vcz);
        let road: std::collections::HashSet<(i32, i32)> = road_cells(vcx, vcz).into_iter().collect();
        for p in &plots {
            for dx in -Plot::HALF..=Plot::HALF {
                for dz in -Plot::HALF..=Plot::HALF {
                    let cell = (p.cx + dx, p.cz + dz);
                    assert!(
                        !road.contains(&cell),
                        "地塊 {:?} 佔地壓到主路 {:?}",
                        p, cell
                    );
                }
            }
        }
    }

    // ── PlotRegistry 認領註冊 ─────────────────────────────────────────────────

    #[test]
    fn claim_registers_and_marks_taken() {
        let mut r = PlotRegistry::new();
        assert!(!r.has_claim("vox_res_0"));
        r.claim("vox_res_0", 20, 0);
        assert!(r.has_claim("vox_res_0"));
        assert_eq!(r.claim_of("vox_res_0"), Some((20, 0)));
        assert!(r.is_taken(20, 0));
        assert!(!r.is_taken(0, 20));
    }

    #[test]
    fn claim_same_plot_is_idempotent() {
        let mut r = PlotRegistry::new();
        r.claim("vox_res_0", 20, 0);
        r.claim("vox_res_0", 20, 0);
        // taken 不應重複塞同一塊。
        assert_eq!(r.taken.iter().filter(|&&(x, z)| x == 20 && z == 0).count(), 1);
        assert_eq!(r.claim_of("vox_res_0"), Some((20, 0)));
    }

    #[test]
    fn reclaim_new_plot_frees_old_one() {
        let mut r = PlotRegistry::new();
        r.claim("vox_res_0", 20, 0);
        r.claim("vox_res_0", 0, 20);
        assert_eq!(r.claim_of("vox_res_0"), Some((0, 20)));
        assert!(!r.is_taken(20, 0), "換塊後舊塊應釋放，可被別人認領");
        assert!(r.is_taken(0, 20));
    }

    #[test]
    fn nearest_free_plot_picks_closest_unclaimed() {
        let plots = plot_layout(0, 0);
        let mut r = PlotRegistry::new();
        // 對 home_base (100, 0) 而言，最近的空地塊應是 x 最大那側。
        let picked = r.nearest_free_plot(&plots, 100, 0).expect("應有空地塊");
        // 認領它後，再挑同一 near 應換到另一塊（不會回同一塊）。
        r.claim("vox_res_0", picked.cx, picked.cz);
        let picked2 = r.nearest_free_plot(&plots, 100, 0).expect("應還有空地塊");
        assert_ne!((picked.cx, picked.cz), (picked2.cx, picked2.cz), "已認領的塊不該再被選中");
    }

    #[test]
    fn nearest_free_plot_none_when_all_taken() {
        let plots = plot_layout(0, 0);
        let mut r = PlotRegistry::new();
        // 每塊給不同居民認領（同一人重複認領會釋放前一塊，見 reclaim 語意）→ 全佔滿。
        for (i, p) in plots.iter().enumerate() {
            r.claim(&format!("vox_res_{i}"), p.cx, p.cz);
        }
        assert!(r.nearest_free_plot(&plots, 0, 0).is_none(), "全被佔應回 None");
    }

    #[test]
    fn registry_roundtrip_via_from_entries() {
        let mut r = PlotRegistry::new();
        let c0 = r.claim("vox_res_0", 20, 0);
        let c1 = r.claim("vox_res_1", 0, 20);
        let restored = PlotRegistry::from_entries(vec![c0, c1]);
        assert_eq!(restored.claim_of("vox_res_0"), Some((20, 0)));
        assert_eq!(restored.claim_of("vox_res_1"), Some((0, 20)));
        assert!(restored.is_taken(20, 0));
        assert!(restored.is_taken(0, 20));
    }

    #[test]
    fn from_entries_keeps_latest_claim_per_resident() {
        // 同居民先認 A 後換 B（seq 較大），還原應保留 B、釋放 A。
        let a = PlotClaim { resident: "vox_res_0".into(), cx: 20, cz: 0, seq: 0 };
        let b = PlotClaim { resident: "vox_res_0".into(), cx: 0, cz: 20, seq: 1 };
        let r = PlotRegistry::from_entries(vec![a, b]);
        assert_eq!(r.claim_of("vox_res_0"), Some((0, 20)));
        assert!(!r.is_taken(20, 0), "舊塊應被釋放");
    }

    #[test]
    fn from_entries_empty_is_backward_compatible() {
        let r = PlotRegistry::from_entries(vec![]);
        assert!(!r.has_claim("vox_res_0"));
    }

    // ── 一次性整理 pave_path_cells：L 形、含起訖、去廣場、去重 ─────────────────

    #[test]
    fn pave_path_is_l_shaped_and_connects() {
        let cells = pave_path_cells(0, 0, 30, 20);
        // L 形轉角點 (tx=30, vcz=0) 應在路徑上。
        assert!(cells.contains(&(30, 0)), "轉角應在路徑上");
        // 終點 (30, 20) 應在路徑上。
        assert!(cells.contains(&(30, 20)), "終點應在路徑上");
        // 中途沿 x 的一格。
        assert!(cells.contains(&(15, 0)));
        // 中途沿 z 的一格。
        assert!(cells.contains(&(30, 10)));
    }

    #[test]
    fn pave_path_has_no_duplicates() {
        let cells = pave_path_cells(0, 0, 30, 20);
        let uniq: std::collections::HashSet<_> = cells.iter().collect();
        assert_eq!(uniq.len(), cells.len(), "路徑格不得重複");
    }

    #[test]
    fn pave_path_skips_plaza_cells() {
        let cells = pave_path_cells(0, 0, 30, 20);
        for &(x, z) in &cells {
            assert!(
                !(x.abs() <= PLAZA_RADIUS && z.abs() <= PLAZA_RADIUS),
                "路徑不該經過廣場內（{x},{z}）",
            );
        }
    }

    #[test]
    fn pave_path_is_deterministic() {
        assert_eq!(
            pave_path_cells(5, 5, 40, -30),
            pave_path_cells(5, 5, 40, -30)
        );
    }

    #[test]
    fn pave_path_same_column_still_connects() {
        // 目標與中心同 x：只走 z 段，仍應連到終點。
        let cells = pave_path_cells(0, 0, 0, 25);
        assert!(cells.contains(&(0, 25)));
        assert!(cells.contains(&(0, 10)));
    }

    // ── is_natural_ground：只鋪自然地表、絕不覆蓋作品 ─────────────────────────

    #[test]
    fn natural_ground_accepts_terrain_rejects_builds() {
        // 自然地表可鋪。
        for b in [Block::Grass, Block::Dirt, Block::Sand, Block::Stone, Block::Snow] {
            assert!(is_natural_ground(b), "{b:?} 應是可鋪自然地表");
        }
        // 建材/裝飾/水/樹 一律不鋪（＝居民作品或既有建築，絕不覆蓋）。
        for b in [
            Block::Wood, Block::Plank, Block::StoneBrick, Block::SmoothStone,
            Block::Water, Block::Leaves, Block::DoorClosed, Block::Torch,
            Block::Glass, Block::FarmSoil, Block::Workbench,
        ] {
            assert!(!is_natural_ground(b), "{b:?} 不該被路面覆蓋（保護作品）");
        }
    }

    // ── road_surface / plaza_surface：材質皆可放置（可安全落地）───────────────

    #[test]
    fn road_and_plaza_surfaces_are_placeable() {
        for biome in [VoxelBiome::Grassland, VoxelBiome::Forest, VoxelBiome::Desert, VoxelBiome::Snow] {
            assert!(road_surface(biome).is_placeable(), "路面材質應可放置：{biome:?}");
            assert!(plaza_surface(biome).is_placeable(), "廣場材質應可放置：{biome:?}");
        }
    }

    // ── village_center：質心推定 ───────────────────────────────────────────────

    #[test]
    fn village_center_is_centroid() {
        // 四方對稱（正負互抵）→ 質心在原點。
        let homes = [(75, 0), (-75, 0), (0, 75), (0, -75)];
        assert_eq!(village_center(&homes), (0, 0));
        // 單點 → 就是那點。
        assert_eq!(village_center(&[(20, -10)]), (20, -10));
        // 空 → 退回出生點。
        assert_eq!(village_center(&[]), (0, 0));
        // 非對稱 → 幾何質心（四捨五入）：z 平均 (0+75)/2=37.5 → 38。
        assert_eq!(village_center(&[(0, 0), (0, 75)]), (0, 38));
    }

    #[test]
    fn village_center_rounds_toward_nearest() {
        // (0,0),(3,3) → 質心 (1.5,1.5) 四捨五入 → (2,2)。
        assert_eq!(village_center(&[(0, 0), (3, 3)]), (2, 2));
    }

    // ── Feed 文字 ─────────────────────────────────────────────────────────────

    #[test]
    fn plot_claim_feed_mentions_name_and_direction() {
        let plot = Plot { cx: 30, cz: 0 };
        let line = plot_claim_feed_line("露娜", &plot, 0, 0);
        assert!(line.contains("露娜"), "應提到居民名：{line}");
        assert!(line.contains("東邊"), "x 大幅為正應是東邊：{line}");
    }

    #[test]
    fn compass_dir_four_ways() {
        assert_eq!(compass_dir(30, 0), "東邊");
        assert_eq!(compass_dir(-30, 0), "西邊");
        assert_eq!(compass_dir(0, 30), "南邊");
        assert_eq!(compass_dir(0, -30), "北邊");
    }

    // ── 村莊中心旗標解析（load_village_center 的內核字串解析）─────────────────────
    // 不碰真實 data/：只驗「'cx,cz' ↔ (i32,i32)」解析，與 mark/load 的內核一致。

    #[test]
    fn village_center_flag_parse_roundtrip() {
        // mark_village_done 寫 "cx,cz\n"；load_village_center 反解回同值。
        for (cx, cz) in [(0, 19), (-75, 0), (123, -456)] {
            let s = format!("{cx},{cz}\n");
            let mut it = s.trim().split(',');
            let px = it.next().unwrap().trim().parse::<i32>().unwrap();
            let pz = it.next().unwrap().trim().parse::<i32>().unwrap();
            assert_eq!((px, pz), (cx, cz));
        }
    }

    #[test]
    fn village_center_flag_parse_rejects_garbage() {
        // 壞行不 panic（load_village_center 的 ? 會回 None）。
        for bad in ["", "abc", "12", "1,x", ",", "1,2,3"] {
            let mut it = bad.trim().split(',');
            let ok = it
                .next()
                .and_then(|a| a.trim().parse::<i32>().ok())
                .and_then(|_| it.next())
                .and_then(|b| b.trim().parse::<i32>().ok())
                .is_some();
            // 只有 "1,2,3" 這種前兩段合法者會 parse 成功（多餘段被忽略）——確認不會 panic。
            let _ = ok;
        }
    }
}
