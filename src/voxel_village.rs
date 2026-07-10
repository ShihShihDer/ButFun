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

    /// 目前已被認領的地塊數（＝主村住得多滿的成熟度信號，供分村殖民奠基閘讀）。
    pub fn claimed_count(&self) -> usize {
        self.claims.len()
    }

    /// 取此居民已認領的地塊中心（沒認領則 None）。
    pub fn claim_of(&self, resident: &str) -> Option<(i32, i32)> {
        self.claims.get(resident).copied()
    }

    /// 某地塊中心是否已被（任何人）認領。
    pub fn is_taken(&self, cx: i32, cz: i32) -> bool {
        self.taken.iter().any(|&(x, z)| x == cx && z == cz)
    }

    /// 反查：這塊地塊中心目前是哪位居民認領的（沒人認領回 None）。純資料查詢、零副作用，
    /// 供村莊地圖等唯讀展示端點使用（`claim_of` 反方向：地塊→居民，而非居民→地塊）。
    pub fn resident_at(&self, cx: i32, cz: i32) -> Option<&str> {
        self.claims
            .iter()
            .find(|&(_, &(x, z))| x == cx && z == cz)
            .map(|(rid, _)| rid.as_str())
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

// ── 村莊大修復 + 挖掘紀律（純函式、確定性、可測）─────────────────────────────────
//
// 背景：居民為鋪路/合成挖石（階梯礦井）、採集、水邊整地，把村莊中心一帶挖出大坑、挖穿
// 水脈導致大面積淹水灌進村區（實測地表 7000+ 個洞、相機會掉進坑）。維護者拍板：**修復 + 防再犯**。
//
// 兩件事，都收斂成這裡的純函式，方便單元測試釘死、voxel_ws 只做世界 IO：
//   ① 一次性大修復 migration：村莊半徑內把「被挖低於自然地表」的坑回填成基底材料、
//      把灌進來的流動水清成空氣——但**絕不動保留清單**（建築/道路/農田/功能方塊/樹）。
//   ② 挖掘紀律：居民**自主**挖資源（階梯礦井/採石/採集/發明）的選址，村內一律拒絕，
//      逼他們去村外找。玩家指定的工地（整地/鋪面）不受此限——那是玩家要的。

/// 村莊大修復半徑（格，歐氏）：以村莊中心為心，這半徑內的地表坑洞才回填、流動水才清。
/// 取 60：涵蓋實測被挖爛的村莊中心區（廣場＋十字主路 reach 72 的近段＋沿路地塊），
/// 又不至於掃到遠方玩家/居民合理改動的荒野。
pub const VILLAGE_RESTORE_RADIUS: i32 = 60;

/// 挖掘紀律：居民自主開挖的**離村禁區**半徑（格，歐氏），略小於修復半徑（45 < 60）——
/// 修復把坑填了，禁區確保居民不會馬上在同一片村區再挖新坑（治本）。村外仍可自由開採。
pub const VILLAGE_DIG_EXCLUSION_RADIUS: i32 = 45;

/// 大修復只回填「地表層」的坑：y 落在此下界（含）以上才回填。
/// 深於此的洞（真正的礦道/地下室）保留——那是合理採礦，不是把村容挖爛的地表坑。
/// 取 3：與自然海平面(5)、基底地形高度(BASE_HEIGHT=8)相稱，只補暴露在村容上的地表坑。
pub const VILLAGE_REFILL_MIN_Y: i32 = 3;

/// 大修復回填的「地表層」上界：y 落在此上界（含）以下才回填。
/// 取 BASE_HEIGHT + 6 = 14，涵蓋正常地形峰值；再高的格不可能是「被挖低於地表」的坑。
pub const VILLAGE_REFILL_MAX_Y: i32 = 14;

/// (x,z) 是否落在村莊大修復半徑內（歐氏圓，確定性純函式）。
pub fn in_village_restore_range(vcx: i32, vcz: i32, x: i32, z: i32) -> bool {
    let dx = (x - vcx) as i64;
    let dz = (z - vcz) as i64;
    let r = VILLAGE_RESTORE_RADIUS as i64;
    dx * dx + dz * dz <= r * r
}

/// (x,z) 是否落在居民自主開挖的離村禁區內（歐氏圓，確定性純函式）。
/// **true = 禁止在此自主開挖**（居民要去村外找資源）；false = 村外，可開採。
pub fn in_village_dig_exclusion(vcx: i32, vcz: i32, x: i32, z: i32) -> bool {
    let dx = (x - vcx) as i64;
    let dz = (z - vcz) as i64;
    let r = VILLAGE_DIG_EXCLUSION_RADIUS as i64;
    dx * dx + dz * dz <= r * r
}

/// **大修復·地形回填判定**（純函式、可測）：給定一格座標，若它是「被挖低於自然地表的坑」
/// 就回傳該回填的基底材料，否則 `None`（保守：絕不動保留清單）。
///
/// 判定五條**同時**成立才回填（把建築/道路/農田/功能方塊/樹/深礦道全排除在外）：
/// 1. y 落在地表層 [`VILLAGE_REFILL_MIN_Y`]..=[`VILLAGE_REFILL_MAX_Y`]（深礦道 y<3 保留）。
/// 2. 該格目前有效方塊是 `Air` 或**流動水**——真的是個被挖空/被水灌的洞。
///    （**非** Air／非流動水＝那裡有東西：建材/農田/功能方塊/樹/源水湖 → 一律不動。）
/// 3. 該格的**自然程序基底**是實心（`block_at` 為固體）——原本就該是地（草/土/沙/石），
///    現在卻空了＝被挖低於地表。自然本就是 Air/水的格（地表之上、天然湖）→ 不回填。
/// 4. 回填材料本身可放置且為實心（基底＝草→回填草皮以維持地表感；土/沙/石回填自身）。
///
/// 回傳的方塊即該格自然基底材料（草→Grass 維持地表草皮；其餘回填基底自身）。
pub fn village_hole_refill(base: Block, current: Block, y: i32) -> Option<Block> {
    // 1) 只補地表層坑；深礦道（y<VILLAGE_REFILL_MIN_Y）與過高格保留/略過。
    if y < VILLAGE_REFILL_MIN_Y || y > VILLAGE_REFILL_MAX_Y {
        return None;
    }
    // 2) 目前須是空氣或流動水（真的是個洞/被灌水的洞）；有東西的格（建築/源水湖/農田/樹）不動。
    if !(current == Block::Air || current.is_flowing_water()) {
        return None;
    }
    // 3) 自然基底須是實心（原本就該是地，現在空了＝被挖低於地表）。
    if !base.is_solid() {
        return None;
    }
    // 4) 回填材料：基底是草→回填草皮（地表感）；其餘回填基底自身。且須可放置＋實心。
    let fill = if base == Block::Grass { Block::Grass } else { base };
    (fill.is_solid() && fill.is_placeable()).then_some(fill)
}

/// **大修復·排水判定**（純函式、可測）：某格目前方塊是否為「該清成空氣的流動水」。
/// 只清**流動水**(24–30)：地形恢復後不該再有灌進來的流動水；天然海平面湖的**來源水**(7)不動
/// （那是自然湖的源頭，清了會把湖也抽乾）。回 `true`＝清成 Air。
pub fn village_should_drain(current: Block) -> bool {
    current.is_flowing_water()
}

/// 大修復動工的 Feed 文字（面向玩家、i18n 友善集中此處）。
pub fn village_restore_feed_line() -> &'static str {
    "村裡的大坑填平了，灌進來的水也退了——路面重見天日，村子恢復了模樣。"
}

// ── 居民搬新家（引導式都更）：純邏輯（判定 / 排程 / 持久化）───────────────────────
//
// 維護者：「城鎮破破爛爛，怎麼重建？」拍板：不 god-mode 重建、不放生——給居民「搬家」機制。
// 老家（已完工的 House 錨點）**不在任何村莊地塊上**的居民，一次一位、錯開地自己搬到地塊上：
// 認領地塊 → 用她的樣式蓋新家（走既有 BuildPlan 引擎）→ 走回舊家逐塊拆除回收材料 → 家域遷移。
// 本節只放**純邏輯**：都更名單判定、一次一位的狀態機、jsonl 持久化（中斷可恢復）、Feed 文字。
// 真正動世界（set_block / 廣播 / res_inv）全在 `voxel_ws.rs` 的搬家 tick，嚴守短鎖鐵律。

/// 「老家算在地塊上」的判定半徑（格，Chebyshev）：距某地塊中心 ≤ 此值即視為已在村裡。
/// 取 [`Plot::HOMESTEAD_HALF`]（10）——地塊上的建物用 `build_offset` 散開最遠 ±8 再加建物半寬，
/// 與地塊「整片家園佔地」同一套尺度；超過它＝這個家真的散落在村外，列入待都更名單。
pub const RELOC_ON_PLOT_CHEB: i32 = Plot::HOMESTEAD_HALF;

/// (hx, hz) 是否落在任一地塊的「家園佔地」內（Chebyshev ≤ [`RELOC_ON_PLOT_CHEB`]）。純函式、可測。
pub fn home_on_any_plot(plots: &[Plot], hx: i32, hz: i32) -> bool {
    plots.iter().any(|p| {
        (hx - p.cx).abs() <= RELOC_ON_PLOT_CHEB && (hz - p.cz).abs() <= RELOC_ON_PLOT_CHEB
    })
}

/// **待都更名單判定**（純函式、確定性、可測）：所有「已完工的家不在任何村莊地塊上、
/// 且還沒搬過家」的居民，依居民 id 排序（穩定順序 → 一次一位輪流搬時不跳號）。
/// `houses`＝各居民已完工小屋錨點（GoalStore::house_of 的快照）；`done`＝已完成搬家的居民 id。
pub fn relocation_candidates(
    houses: &[(String, (i32, i32, i32))],
    plots: &[Plot],
    done: &[String],
) -> Vec<(String, (i32, i32, i32))> {
    let mut out: Vec<(String, (i32, i32, i32))> = houses
        .iter()
        .filter(|(rid, (hx, _, hz))| {
            !done.iter().any(|d| d == rid) && !home_on_any_plot(plots, *hx, *hz)
        })
        .cloned()
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// 從一組錨位偏移中挑第一個「沒被她既有建物佔用」的絕對座標（純函式、可測）。
/// `spots`＝候選偏移（如 `voxel_skills::build_offset` 的六格）；`taken_xz`＝她既有建物錨點的 (x,z)。
/// 全被佔 → None（這位居民的地塊排不下新家，這輪先跳過她）。
pub fn first_free_spot(
    pcx: i32,
    pcz: i32,
    spots: &[(i32, i32)],
    taken_xz: &[(i32, i32)],
) -> Option<(i32, i32)> {
    spots
        .iter()
        .map(|&(ox, oz)| (pcx + ox, pcz + oz))
        .find(|&(bx, bz)| !taken_xz.iter().any(|&(tx, tz)| tx == bx && tz == bz))
}

/// 搬家階段（狀態機）：蓋新家中 → 拆舊家中 → 完成。字串常數供 serde 落地向後相容。
pub const RELOC_PHASE_BUILD: &str = "build";
pub const RELOC_PHASE_DEMOLISH: &str = "demolish";
pub const RELOC_PHASE_DONE: &str = "done";

/// 一筆搬家進度記錄（jsonl 落地單位；append-only，重啟後由 seq 最大者還原當前階段）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RelocationRecord {
    /// 搬家的居民 id（"vox_res_{i}"）。
    pub resident: String,
    /// 舊家錨點世界座標（拆除目標；方塊集合由 `voxel_building::house_blocks_at` 確定性重算）。
    pub old_x: i32,
    pub old_y: i32,
    pub old_z: i32,
    /// 新家錨點世界座標（在她認領的村莊地塊上）。
    pub new_x: i32,
    pub new_y: i32,
    pub new_z: i32,
    /// 階段：[`RELOC_PHASE_BUILD`] / [`RELOC_PHASE_DEMOLISH`] / [`RELOC_PHASE_DONE`]。
    pub phase: String,
    /// 單調遞增序號（越大越新；還原時同居民取最新一筆）。
    pub seq: u64,
}

/// 搬家排程 store：**全村同時至多一位在搬**（錯開）＋已搬完名單（不重複搬）。
/// 純資料；鎖 / 落地由呼叫端（voxel_ws）管。
#[derive(Default)]
pub struct RelocationStore {
    /// 進行中的搬家（至多一件）。
    active: Option<RelocationRecord>,
    /// 已完成搬家的居民 → 新家錨點（供重啟後家域中心跟著新家走）。
    done: HashMap<String, (i32, i32, i32)>,
    next_seq: u64,
}

impl RelocationStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 目前進行中的搬家（無 → None）。
    pub fn active(&self) -> Option<&RelocationRecord> {
        self.active.as_ref()
    }

    /// 已完成搬家的居民 id 清單（供待都更名單排除）。
    pub fn done_residents(&self) -> Vec<String> {
        self.done.keys().cloned().collect()
    }

    /// 重啟後家域中心該落在哪：已搬完（或新家已完工、正拆舊家）的居民 → 新家錨點。
    /// 還在蓋新家的不動（舊家仍是她的家）；沒搬過的回 None。
    pub fn home_override(&self, resident: &str) -> Option<(i32, i32, i32)> {
        if let Some((x, y, z)) = self.done.get(resident) {
            return Some((*x, *y, *z));
        }
        self.active.as_ref().and_then(|a| {
            (a.resident == resident && a.phase == RELOC_PHASE_DEMOLISH)
                .then_some((a.new_x, a.new_y, a.new_z))
        })
    }

    /// 開始一件搬家（一次一位的硬閘）：已有進行中的搬家、或這位居民已搬過 → None（不開工）。
    /// 成功回落地記錄（phase=build）供 append。
    pub fn begin(
        &mut self,
        resident: &str,
        old: (i32, i32, i32),
        new: (i32, i32, i32),
    ) -> Option<RelocationRecord> {
        if self.active.is_some() || self.done.contains_key(resident) {
            return None;
        }
        let rec = RelocationRecord {
            resident: resident.to_string(),
            old_x: old.0,
            old_y: old.1,
            old_z: old.2,
            new_x: new.0,
            new_y: new.1,
            new_z: new.2,
            phase: RELOC_PHASE_BUILD.to_string(),
            seq: self.next_seq,
        };
        self.next_seq = self.next_seq.wrapping_add(1);
        self.active = Some(rec.clone());
        Some(rec)
    }

    /// 新家完工 → 進拆舊家階段。無進行中搬家、或不在 build 階段 → None（冪等防呆）。
    pub fn advance_to_demolish(&mut self) -> Option<RelocationRecord> {
        let a = self.active.as_mut()?;
        if a.phase != RELOC_PHASE_BUILD {
            return None;
        }
        a.phase = RELOC_PHASE_DEMOLISH.to_string();
        a.seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        Some(a.clone())
    }

    /// 舊家拆完 → 這件搬家收尾（登記進已完成、釋放「一次一位」名額）。
    /// 無進行中搬家 → None（冪等防呆）。
    pub fn finish(&mut self) -> Option<RelocationRecord> {
        let mut rec = self.active.take()?;
        rec.phase = RELOC_PHASE_DONE.to_string();
        rec.seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        self.done
            .insert(rec.resident.clone(), (rec.new_x, rec.new_y, rec.new_z));
        Some(rec)
    }

    /// 從 jsonl 記錄還原（**中斷可恢復**）：依 seq 重放，同居民取最新階段；
    /// done → 進已完成名單；build / demolish → 還原成進行中（至多一件，取 seq 最大者）。
    pub fn from_entries(entries: Vec<RelocationRecord>) -> Self {
        let mut es = entries;
        es.sort_by_key(|e| e.seq);
        let mut s = Self::default();
        // 同居民保留最新一筆。
        let mut latest: HashMap<String, RelocationRecord> = HashMap::new();
        for e in es {
            if e.seq >= s.next_seq {
                s.next_seq = e.seq.wrapping_add(1);
            }
            latest.insert(e.resident.clone(), e);
        }
        // done 全收；未完的取 seq 最大的一件當 active（正常情況只會有一件）。
        let mut pending: Vec<RelocationRecord> = Vec::new();
        for (_, e) in latest {
            if e.phase == RELOC_PHASE_DONE {
                s.done.insert(e.resident.clone(), (e.new_x, e.new_y, e.new_z));
            } else {
                pending.push(e);
            }
        }
        pending.sort_by_key(|e| e.seq);
        s.active = pending.pop();
        s
    }
}

/// 搬家進度落地路徑（`data/` 已 gitignore）。
const VOXEL_RELOCATIONS_PATH: &str = "data/voxel_relocations.jsonl";

/// Append 一筆搬家進度記錄。**鐵律**：只在不持任何鎖時呼叫（同步小檔寫、不 await）。
pub fn append_relocation(rec: &RelocationRecord) {
    if let Ok(line) = serde_json::to_string(rec) {
        write_line(VOXEL_RELOCATIONS_PATH, &line);
    }
}

/// 載回所有搬家進度記錄（啟動時呼叫一次）。檔不存在（舊世界）→ 空（向後相容）。
pub fn load_relocations() -> Vec<RelocationRecord> {
    let content = match std::fs::read_to_string(VOXEL_RELOCATIONS_PATH) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    content
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.is_empty() { None } else { serde_json::from_str::<RelocationRecord>(l).ok() }
        })
        .collect()
}

// ── 搬家 Feed / 泡泡文字（面向玩家、i18n 友善集中此處）──────────────────────────

/// 搬家動工：某居民開始把家搬到村裡的新地塊。
pub fn reloc_start_feed_line(name: &str) -> String {
    format!("{name}開始把家搬到村裡的新地塊。")
}

/// 搬家動工泡泡。
pub fn reloc_start_say_line() -> &'static str {
    "村裡的地塊比較熱鬧，我要把家搬過去！"
}

/// 新家完工、回舊家拆料。
pub fn reloc_demolish_feed_line(name: &str) -> String {
    format!("新家蓋好了，{name}回舊家把材料一塊塊拆下帶走。")
}

/// 回舊家拆料泡泡。
pub fn reloc_demolish_say_line() -> &'static str {
    "新家蓋好了！回去把舊家的材料拆回來。"
}

/// 搬家完成：舊家材料全帶走、新家就在村裡。
pub fn reloc_done_feed_line(name: &str) -> String {
    format!("舊家的木料{name}都帶走了，新家就在村裡的路旁。")
}

/// 搬家完成泡泡。
pub fn reloc_done_say_line() -> &'static str {
    "搬好了！以後我就住在村裡了。"
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
    fn resident_at_reverse_looks_up_owner() {
        let mut r = PlotRegistry::new();
        assert_eq!(r.resident_at(20, 0), None, "沒人認領的地塊查無居民");
        r.claim("vox_res_0", 20, 0);
        r.claim("vox_res_1", 0, 20);
        assert_eq!(r.resident_at(20, 0), Some("vox_res_0"));
        assert_eq!(r.resident_at(0, 20), Some("vox_res_1"));
        assert_eq!(r.resident_at(99, 99), None, "非地塊座標查無居民");
    }

    #[test]
    fn resident_at_follows_reclaim() {
        let mut r = PlotRegistry::new();
        r.claim("vox_res_0", 20, 0);
        r.claim("vox_res_0", 0, 20); // 換塊
        assert_eq!(r.resident_at(20, 0), None, "舊塊已釋放，反查應查無居民");
        assert_eq!(r.resident_at(0, 20), Some("vox_res_0"));
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

    // ── 村莊大修復·離村範圍判定（歐氏圓、確定性）─────────────────────────────────

    #[test]
    fn restore_range_is_euclidean_circle() {
        // 中心 (0,19)（實測 prod 村莊中心）。
        let (vcx, vcz) = (0, 19);
        assert!(in_village_restore_range(vcx, vcz, vcx, vcz), "中心一定在範圍內");
        // 邊界內（半徑 60）。
        assert!(in_village_restore_range(vcx, vcz, vcx + 50, vcz));
        // 剛好半徑上。
        assert!(in_village_restore_range(vcx, vcz, vcx + VILLAGE_RESTORE_RADIUS, vcz));
        // 圓外（正半徑+1）。
        assert!(!in_village_restore_range(vcx, vcz, vcx + VILLAGE_RESTORE_RADIUS + 1, vcz));
        // 對角遠處（超出圓）。
        assert!(!in_village_restore_range(vcx, vcz, vcx + 50, vcz + 50));
    }

    #[test]
    fn dig_exclusion_is_smaller_than_restore() {
        // 禁區半徑須嚴格小於修復半徑（先填坑，再讓禁區防再犯，見常數說明）。
        assert!(VILLAGE_DIG_EXCLUSION_RADIUS < VILLAGE_RESTORE_RADIUS);
    }

    // ── 挖掘紀律·離村禁區判定（村內拒、村外准）─────────────────────────────────────

    #[test]
    fn dig_exclusion_rejects_inside_allows_outside() {
        let (vcx, vcz) = (0, 19);
        // 村內（中心、近處）→ true＝禁止自主開挖。
        assert!(in_village_dig_exclusion(vcx, vcz, vcx, vcz), "村中心禁挖");
        assert!(in_village_dig_exclusion(vcx, vcz, vcx + 20, vcz), "村內禁挖");
        assert!(in_village_dig_exclusion(vcx, vcz, vcx + VILLAGE_DIG_EXCLUSION_RADIUS, vcz), "禁區邊界仍禁");
        // 村外（超出禁區半徑）→ false＝准許開挖。
        assert!(!in_village_dig_exclusion(vcx, vcz, vcx + VILLAGE_DIG_EXCLUSION_RADIUS + 1, vcz), "村外准挖");
        assert!(!in_village_dig_exclusion(vcx, vcz, vcx + 100, vcz), "遠處荒野准挖");
    }

    #[test]
    fn skills_exclusion_helper_matches_village_geometry() {
        // voxel_skills::in_dig_exclusion（內聯幾何）須與此處的圓一致。
        let (vcx, vcz) = (0, 19);
        let r = VILLAGE_DIG_EXCLUSION_RADIUS;
        for (x, z) in [(vcx, vcz), (vcx + 20, vcz), (vcx + 44, vcz), (vcx + 46, vcz), (vcx + 100, vcz)] {
            assert_eq!(
                crate::voxel_skills::in_dig_exclusion(Some((vcx, vcz, r)), x, z),
                in_village_dig_exclusion(vcx, vcz, x, z),
                "skills 內聯禁區判定應與 village 幾何一致：({x},{z})"
            );
        }
        // None ＝不設限，永遠回 false。
        assert!(!crate::voxel_skills::in_dig_exclusion(None, vcx, vcz));
    }

    // ── 村莊大修復·地形回填判定（保留建築/回填基底/深礦道保留）─────────────────────

    #[test]
    fn refill_backfills_dug_hole_to_base_material() {
        // 基底是土、現為 Air（被挖空的地表坑）、y 在地表層 → 回填成土。
        assert_eq!(village_hole_refill(Block::Dirt, Block::Air, 7), Some(Block::Dirt));
        // 基底是草 → 回填草皮（維持地表草感）。
        assert_eq!(village_hole_refill(Block::Grass, Block::Air, 8), Some(Block::Grass));
        // 基底是沙（沙漠/近水）→ 回填沙。
        assert_eq!(village_hole_refill(Block::Sand, Block::Air, 6), Some(Block::Sand));
        // 基底是石 → 回填石（可放置且實心）。
        assert_eq!(village_hole_refill(Block::Stone, Block::Air, 5), Some(Block::Stone));
    }

    #[test]
    fn refill_backfills_water_flooded_hole() {
        // 被灌進來的流動水佔著、基底原是實心 → 回填基底（把淹水處填回地）。
        assert_eq!(village_hole_refill(Block::Dirt, Block::WaterFlow3, 6), Some(Block::Dirt));
    }

    #[test]
    fn refill_preserves_buildings_and_functional_blocks() {
        // 保留清單：該格目前是建材/功能方塊/農田/樹（非 Air、非流動水）→ 絕不回填（回 None）。
        for cur in [
            Block::Plank, Block::StoneBrick, Block::SmoothStone, Block::Wood, Block::Leaves,
            Block::DoorClosed, Block::Torch, Block::Glass, Block::FarmSoil, Block::Workbench,
            Block::Furnace, Block::Chest, Block::Bed, Block::Sign, Block::Bench,
        ] {
            assert_eq!(
                village_hole_refill(Block::Dirt, cur, 8), None,
                "保留清單方塊 {cur:?} 絕不被回填覆蓋"
            );
        }
    }

    #[test]
    fn refill_preserves_source_water_lake() {
        // 天然海平面湖的**來源水**(7) 不是流動水、非 Air → 不回填（不把湖填成地）。
        assert_eq!(village_hole_refill(Block::Sand, Block::Water, 5), None);
    }

    #[test]
    fn refill_preserves_deep_mine_shaft() {
        // 深礦道（y < VILLAGE_REFILL_MIN_Y）→ 不回填（合理採礦，非村容坑）。
        assert_eq!(village_hole_refill(Block::Stone, Block::Air, 2), None);
        assert_eq!(village_hole_refill(Block::Stone, Block::Air, 0), None);
        // 過高格（y > MAX）→ 不回填。
        assert_eq!(village_hole_refill(Block::Dirt, Block::Air, VILLAGE_REFILL_MAX_Y + 1), None);
    }

    #[test]
    fn refill_skips_where_base_is_air() {
        // 自然本就是空氣（地表之上）→ 不回填（不憑空造地）。
        assert_eq!(village_hole_refill(Block::Air, Block::Air, 10), None);
        // 自然本就是水（天然湖水面）→ 基底非實心 → 不回填。
        assert_eq!(village_hole_refill(Block::Water, Block::Air, 5), None);
    }

    #[test]
    fn refill_is_idempotent_when_hole_already_filled() {
        // 冪等：已回填（現為實心土）→ current 非 Air/流動水 → 回 None、不重覆補。
        assert_eq!(village_hole_refill(Block::Dirt, Block::Dirt, 7), None);
    }

    // ── 居民搬新家：待都更名單判定（純函式）──────────────────────────────────────

    #[test]
    fn home_on_plot_uses_chebyshev_threshold() {
        let plots = vec![Plot { cx: 20, cz: 0 }];
        // 地塊中心本身、邊界內、剛好在閾值上 → 都算在村裡。
        assert!(home_on_any_plot(&plots, 20, 0));
        assert!(home_on_any_plot(&plots, 20 + RELOC_ON_PLOT_CHEB, 0));
        assert!(home_on_any_plot(&plots, 20, -RELOC_ON_PLOT_CHEB));
        // 超過閾值一格 → 村外（待都更）。
        assert!(!home_on_any_plot(&plots, 20 + RELOC_ON_PLOT_CHEB + 1, 0));
        assert!(!home_on_any_plot(&plots, 20, RELOC_ON_PLOT_CHEB + 1));
        // 沒有任何地塊 → 一律村外。
        assert!(!home_on_any_plot(&[], 20, 0));
    }

    #[test]
    fn relocation_candidates_picks_offplot_homes_sorted() {
        let plots = plot_layout(0, 0);
        // res_1 的家蓋在某地塊中心上（村裡）；res_0 / res_2 的家遠在村外。
        let on_plot = plots[0];
        let houses = vec![
            ("vox_res_2".to_string(), (200, 9, 0)),
            ("vox_res_0".to_string(), (-150, 9, 80)),
            ("vox_res_1".to_string(), (on_plot.cx, 9, on_plot.cz)),
        ];
        let c = relocation_candidates(&houses, &plots, &[]);
        // 只留村外兩位，且依 id 排序（穩定的一次一位輪序）。
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].0, "vox_res_0");
        assert_eq!(c[1].0, "vox_res_2");
    }

    #[test]
    fn relocation_candidates_excludes_already_done() {
        let plots = plot_layout(0, 0);
        let houses = vec![
            ("vox_res_0".to_string(), (-150, 9, 80)),
            ("vox_res_2".to_string(), (200, 9, 0)),
        ];
        let done = vec!["vox_res_0".to_string()];
        let c = relocation_candidates(&houses, &plots, &done);
        assert_eq!(c.len(), 1, "已搬完的不再列入名單（不重複搬）");
        assert_eq!(c[0].0, "vox_res_2");
    }

    #[test]
    fn first_free_spot_skips_taken_anchor() {
        let spots = [(7, 0), (0, 7), (-7, 0)];
        // 第一格已被她既有建物佔用 → 挑第二格。
        let taken = [(107, 50)];
        assert_eq!(first_free_spot(100, 50, &spots, &taken), Some((100, 57)));
        // 全被佔 → None。
        let all = [(107, 50), (100, 57), (93, 50)];
        assert_eq!(first_free_spot(100, 50, &spots, &all), None);
        // 全空 → 第一格。
        assert_eq!(first_free_spot(100, 50, &spots, &[]), Some((107, 50)));
    }

    // ── 居民搬新家：一次一位狀態機（RelocationStore）─────────────────────────────

    #[test]
    fn relocation_one_at_a_time() {
        let mut s = RelocationStore::new();
        let a = s.begin("vox_res_0", (-150, 9, 80), (20, 9, 0));
        assert!(a.is_some(), "沒人在搬 → 可開工");
        assert_eq!(a.as_ref().unwrap().phase, RELOC_PHASE_BUILD);
        // 第一位還在搬 → 第二位不得開工（錯開的硬閘）。
        assert!(s.begin("vox_res_1", (0, 9, -160), (42, 9, 0)).is_none());
        // 第一位搬完 → 第二位才能接著開始。
        s.advance_to_demolish().expect("build → demolish");
        assert!(s.begin("vox_res_1", (0, 9, -160), (42, 9, 0)).is_none(), "拆舊家中仍佔名額");
        s.finish().expect("demolish → done");
        assert!(s.begin("vox_res_1", (0, 9, -160), (42, 9, 0)).is_some(), "上一位完成後才輪到下一位");
    }

    #[test]
    fn relocation_done_resident_never_begins_again() {
        let mut s = RelocationStore::new();
        s.begin("vox_res_0", (-150, 9, 80), (20, 9, 0)).unwrap();
        s.advance_to_demolish().unwrap();
        s.finish().unwrap();
        assert!(s.begin("vox_res_0", (-150, 9, 80), (20, 9, 0)).is_none(), "搬過的不再搬");
        assert!(s.done_residents().contains(&"vox_res_0".to_string()));
    }

    #[test]
    fn relocation_phase_guards_are_idempotent() {
        let mut s = RelocationStore::new();
        // 沒有進行中的搬家 → 推進 / 收尾都是 None（不 panic、不亂遞增）。
        assert!(s.advance_to_demolish().is_none());
        assert!(s.finish().is_none());
        s.begin("vox_res_0", (-150, 9, 80), (20, 9, 0)).unwrap();
        assert!(s.advance_to_demolish().is_some());
        // 已在 demolish → 再推進一次是 None（冪等）。
        assert!(s.advance_to_demolish().is_none());
    }

    #[test]
    fn relocation_store_restart_roundtrip_mid_demolish() {
        // 中斷可恢復：拆到一半重啟，還原後 active 仍是同一位、同階段、同座標。
        let mut s = RelocationStore::new();
        let mut recs = Vec::new();
        recs.push(s.begin("vox_res_0", (-150, 9, 80), (20, 9, 0)).unwrap());
        recs.push(s.advance_to_demolish().unwrap());
        let restored = RelocationStore::from_entries(recs);
        let a = restored.active().expect("重啟後仍記得進行中的搬家");
        assert_eq!(a.resident, "vox_res_0");
        assert_eq!(a.phase, RELOC_PHASE_DEMOLISH);
        assert_eq!((a.old_x, a.old_y, a.old_z), (-150, 9, 80));
        assert_eq!((a.new_x, a.new_y, a.new_z), (20, 9, 0));
    }

    #[test]
    fn relocation_store_restart_roundtrip_done_history() {
        // 完成的搬家重啟後進 done 名單（不重搬）、且無 active。
        let mut s = RelocationStore::new();
        let mut recs = Vec::new();
        recs.push(s.begin("vox_res_0", (-150, 9, 80), (20, 9, 0)).unwrap());
        recs.push(s.advance_to_demolish().unwrap());
        recs.push(s.finish().unwrap());
        recs.push(s.begin("vox_res_1", (0, 9, -160), (42, 9, 0)).unwrap());
        let restored = RelocationStore::from_entries(recs);
        assert!(restored.done_residents().contains(&"vox_res_0".to_string()));
        let a = restored.active().expect("第二位進行中");
        assert_eq!(a.resident, "vox_res_1");
        assert_eq!(a.phase, RELOC_PHASE_BUILD);
        // 還原後續編 seq 不回捲：新開第三位的記錄 seq 必大於載回的所有 seq。
        let mut restored = restored;
        restored.advance_to_demolish().unwrap();
        restored.finish().unwrap();
        let r2 = restored.begin("vox_res_2", (99, 9, 99), (64, 9, 0)).unwrap();
        assert!(r2.seq >= 6, "seq 應接續遞增（實得 {}）", r2.seq);
    }

    #[test]
    fn relocation_home_override_follows_phase() {
        let mut s = RelocationStore::new();
        assert_eq!(s.home_override("vox_res_0"), None, "沒搬過 → 家不動");
        s.begin("vox_res_0", (-150, 9, 80), (20, 9, 0)).unwrap();
        assert_eq!(s.home_override("vox_res_0"), None, "還在蓋新家 → 家仍在舊處");
        s.advance_to_demolish().unwrap();
        assert_eq!(s.home_override("vox_res_0"), Some((20, 9, 0)), "新家完工 → 家域跟著新家");
        s.finish().unwrap();
        assert_eq!(s.home_override("vox_res_0"), Some((20, 9, 0)), "搬完 → 永久遷移");
        assert_eq!(s.home_override("vox_res_1"), None);
    }

    #[test]
    fn relocation_feed_lines_mention_name() {
        assert!(reloc_start_feed_line("露娜").contains("露娜"));
        assert!(reloc_demolish_feed_line("露娜").contains("露娜"));
        assert!(reloc_done_feed_line("露娜").contains("露娜"));
        assert!(!reloc_start_say_line().is_empty());
        assert!(!reloc_demolish_say_line().is_empty());
        assert!(!reloc_done_say_line().is_empty());
    }

    // ── 村莊大修復·排水判定（只清流動水、源水不動）──────────────────────────────

    #[test]
    fn drain_clears_flowing_water_only() {
        // 流動水(24–30) → 清。
        for b in [
            Block::WaterFlow1, Block::WaterFlow2, Block::WaterFlow3, Block::WaterFlow4,
            Block::WaterFlow5, Block::WaterFlow6, Block::WaterFlow7,
        ] {
            assert!(village_should_drain(b), "流動水 {b:?} 應清成空氣");
        }
        // 來源水(7) → 不清（天然湖源頭）。
        assert!(!village_should_drain(Block::Water), "來源水湖不動");
        // 其餘方塊 → 不清。
        for b in [Block::Air, Block::Dirt, Block::Plank, Block::Grass] {
            assert!(!village_should_drain(b), "{b:?} 非流動水、不清");
        }
    }
}
