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
/// - 全部蓋過 → `None`（不再重蓋，改去採集／閒晃；見 [`MAX_EXPANSIONS`] 擴建額度）。
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

/// 基礎四種建物全蓋完後，居民一輩子最多再擴建幾座（每座任一已蓋過的種類皆可）。
/// 對應 [`build_offset`] 原本就多留的 2 個格位（SPOTS 共 6 個、基礎只用 0..4）。
pub const MAX_EXPANSIONS: u32 = 2;

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
/// **註**：程序地形現在會長樹（見 `voxel::tree_in_cell`），故 v1 採集涵蓋**地表**真實存在的
/// 草皮／沙／泥／石，外加**樹幹（木頭）**——居民走到最近的地表/樹旁，砍一塊放進小背包。
/// 木頭是合成鏈（木頭→木板→工作台→3×3）的第一步原料，居民也得採得到才接得上。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GatherResource {
    Grass,
    Sand,
    Dirt,
    Stone,
    /// 木頭（砍樹幹）：合成鏈的起點原料。
    Wood,
}

impl GatherResource {
    /// 對應的方塊型別。
    pub fn block(self) -> Block {
        match self {
            GatherResource::Grass => Block::Grass,
            GatherResource::Sand => Block::Sand,
            GatherResource::Dirt => Block::Dirt,
            GatherResource::Stone => Block::Stone,
            GatherResource::Wood => Block::Wood,
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
            GatherResource::Wood => "木頭",
        }
    }

    /// 由方塊型別反查資源（找到方塊後得知採到的是什麼）。不可採的回 None。
    /// 樹冠 Leaves 不直接採（採的是樹幹 Wood，見 `find_nearest_resource` 的樹處理）。
    pub fn from_block(b: Block) -> Option<GatherResource> {
        match b {
            Block::Grass => Some(GatherResource::Grass),
            Block::Sand => Some(GatherResource::Sand),
            Block::Dirt => Some(GatherResource::Dirt),
            Block::Stone => Some(GatherResource::Stone),
            Block::Wood => Some(GatherResource::Wood),
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
    /// 基礎四種都蓋完了，但仍有具體渴望且擴建額度未滿 → 再蓋一座同種建物（擴建）。
    Expand(BuildKind),
    /// 沒有可蓋目標、也沒有可擴建的渴望——交回呼叫端：偶爾採集，否則閒晃。
    Wander,
}

/// 依「已蓋清單、心願、本輪已採集次數、採集配額、已擴建次數」挑下一步活動：
/// - 還有沒蓋過的建物時：先採集 `gather_quota` 次（備料、有在做事的感覺），再蓋。
/// - 基礎全蓋完後，若仍懷著具體渴望（哪怕對應的種類早蓋過）且擴建額度未滿
///   （見 [`MAX_EXPANSIONS`]）→ 再蓋一座——**心願不再石沉大海**：此前對已蓋過種類
///   的渴望會被 `next_build_goal` 直接忽略，永遠不會被回應。
/// - 沒有可蓋目標、也沒有可擴建的渴望 → `Wander`，由呼叫端決定偶爾採集或閒晃。
///
/// 這把「目標＋記憶」收斂成一個可測的決策點：永遠不會選到已蓋過的建物（除非是擴建）。
pub fn choose_activity(
    done_builds: &[BuildKind],
    desired: Option<BuildKind>,
    gathered_since_build: u32,
    gather_quota: u32,
    expansion_count: u32,
) -> NextActivity {
    if let Some(kind) = next_build_goal(done_builds, desired) {
        return if gathered_since_build < gather_quota {
            NextActivity::Gather
        } else {
            NextActivity::Build(kind)
        };
    }
    if let Some(kind) = desired {
        if expansion_count < MAX_EXPANSIONS {
            return if gathered_since_build < gather_quota {
                NextActivity::Gather
            } else {
                NextActivity::Expand(kind)
            };
        }
    }
    NextActivity::Wander
}

/// 居民是否已走到資源旁、可以動手挖（只看水平距離；垂直由重力處理）。純函式、可測。
pub fn within_gather_reach(rx: f32, rz: f32, tx: i32, tz: i32) -> bool {
    let dx = (tx as f32 + 0.5) - rx;
    let dz = (tz as f32 + 0.5) - rz;
    dx * dx + dz * dz <= GATHER_REACH * GATHER_REACH
}

/// 幾何閘（無世界、純整數）：給定居民腳底所在格 (fx,fy,fz) 與想挖的目標方塊 (tx,ty,tz)，
/// 回傳 `true`＝通過第一道保守檢查。**這只是地基**——真正擋住「採集挖坑自困」的可逃性
/// 判定在 [`is_escapable_after_dig`]（它會先過這道閘，再加窪地/可站回的世界級保險）。
///
/// 兩條保守規則（確定性、可測）：
/// 1. **別挖自己站的那一柱**（tx==fx && tz==fz）：會抽掉腳下的地，居民直接掉進洞裡。
/// 2. **別挖明顯低於腳底的方塊**（ty < fy-1）：挖低處＝把要走過去的地方掏成坑。
///    只考慮「腳底層（fy-1）或更高」的方塊；是否真的可挖再交給可逃性判定收尾。
pub fn safe_to_dig(fx: i32, fy: i32, fz: i32, tx: i32, ty: i32, tz: i32) -> bool {
    if tx == fx && tz == fz {
        return false;
    }
    if ty < fy - 1 {
        return false;
    }
    true
}

/// 四向水平鄰柱偏移（判窪地用）。
const NEIGHBORS_4: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];

/// 某 (x,z) 柱最高實心方塊的 y（套 delta overlay；全空回 None）。給可逃性判定算「柱頂」。
pub fn column_top(world: &WorldDelta, x: i32, z: i32) -> Option<i32> {
    surface_block(world, x, z).map(|(y, _)| y)
}

/// 居民腳下柱是否身處「窪地」：四向任一鄰柱頂**高於**腳下柱頂 → 在窪地裡。
/// 在窪地裡時禁止再往腳底層下挖（否則一刀刀越挖越深，接成爬不出的坑）。純函式、可測。
fn foot_in_depression(world: &WorldDelta, fx: i32, fz: i32, foot_top: i32) -> bool {
    NEIGHBORS_4
        .iter()
        .any(|(dx, dz)| column_top(world, fx + dx, fz + dz).map_or(false, |t| t > foot_top))
}

/// **可逃性判定（核心保證：採集永不把自己關進爬不出的坑）**。
/// 模擬「挖掉目標 (tx,ty,tz) 後」的世界，判斷站在腳底格 (fx,fy,fz) 的居民是否仍踏 1 階走得出去。
/// 回 `true` ＝這刀挖了也逃得出去，可採。全是確定性、零鎖、可單元測試的純邏輯。
///
/// 三道保守規則疊起來，數學上把「採集挖坑」的深度封在「原地表下至多 1 格」，故永不受困：
/// 1. 先過幾何閘 [`safe_to_dig`]：不挖腳下那一柱、不挖明顯低於腳的方塊。
/// 2. **窪地不再下挖**：若挖的是「腳底層」方塊（ty == fy-1，會在腳邊掏出 1 格凹陷），
///    且居民此刻已身處窪地（鄰柱比腳下高）→ 拒挖。站在平地/高處才允許掏 1 格凹陷
///    （挖完頂多缺 1 格、踏階能走回），但永遠下不去第 2 層 → 不會一路往下挖成深井。
/// 3. **挖後仍可站回**：挖掉後目標柱的新地表頂須 >= fy-2（居民踏進去頂多差 1 階，
///    一定爬得回腳底層）；挖完底下全空（無底洞）一律拒挖。
pub fn is_escapable_after_dig(
    world: &WorldDelta,
    fx: i32,
    fy: i32,
    fz: i32,
    tx: i32,
    ty: i32,
    tz: i32,
) -> bool {
    // 規則 1：幾何閘。
    if !safe_to_dig(fx, fy, fz, tx, ty, tz) {
        return false;
    }
    let foot_top = fy - 1; // 居民站立柱頂面所在方塊 y
                           // 規則 2：腳底層掏坑只允許在「非窪地」時（防越挖越深、接成深坑）。
    if ty == foot_top && foot_in_depression(world, fx, fz, foot_top) {
        return false;
    }
    // 規則 3：挖後目標柱新地表頂（ty 以下最高實心）須 >= fy-2，踏 1 階可進出。
    match (0..ty).rev().find(|&y| voxel::effective_block_at(world, tx, y, tz).is_solid()) {
        Some(new_top) => new_top >= fy - 2,
        None => false, // 挖完底下全空 → 無底洞，拒挖
    }
}

// ── 樓梯井生成（未來「往下採深處資源/礦」的範本：邊挖邊留階，永遠走得回地面）─────────
//
// v1 採集**完全不往下挖**（is_escapable_after_dig 已把深度封在地表下 1 格）。但維護者方向是
// 「真要往下挖就留樓梯/坡」，故先把「樓梯井」抽成可測純函式備用：未來深處資源接這個範本，
// 一層一層往下、每層往同一水平方向位移 1 格並清出 2 格頭頂高 → 形成可走回地面的階梯井，
// 而不是垂直深坑。本函式只算「要清成空氣的格子座標」，不碰世界/鎖。

/// 樓梯井的單階水平方向（沿 +x 一格一階往下；夠簡單可測，未來要轉向再擴充）。
pub const STAIR_DIR: (i32, i32) = (1, 0);
/// 居民身高需要的頭頂淨空格數（站立 + 頭部，至少 2 格才走得過去）。
pub const STAIR_HEADROOM: i32 = 2;

/// 由 (sx,sy,sz) 起往下挖 `depth` 階的「樓梯井」要清空的方塊座標清單。
/// `sy` ＝最上一階的**踏面 y**（站上去的地表頂方塊 y）。每往下一階：水平沿 [`STAIR_DIR`]
/// 位移 1 格、踏面降 1 格，並清出該階踏面上方 [`STAIR_HEADROOM`] 格頭頂淨空。
/// 相鄰兩階踏面「垂直差 1、水平相鄰」→ 居民踏階即可上下，永遠走得回地面。純函式、可測。
pub fn staircase_well(sx: i32, sy: i32, sz: i32, depth: i32) -> Vec<(i32, i32, i32)> {
    let mut cells = Vec::new();
    let (dx, dz) = STAIR_DIR;
    for step in 0..depth.max(0) {
        let x = sx + dx * step;
        let z = sz + dz * step;
        let tread_y = sy - step; // 這一階的踏面方塊 y（站上去那塊地表）
                                 // 清掉踏面上方 HEADROOM 格 → 站得進去、走得過去。
        for h in 1..=STAIR_HEADROOM {
            cells.push((x, tread_y + h, z));
        }
    }
    cells
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

/// 某 (x,z) 柱若是樹，回傳「樹幹底座方塊」的 y（最低的一塊 Wood）。非樹回 None。
/// 居民站在旁邊草地、側向砍這塊樹幹底（近地表、安全可逃）。套 delta overlay（長過/砍過都算）。
/// 由底往上掃，第一塊 Wood 即最低樹幹塊。純函式、可測。
pub fn trunk_base(world: &WorldDelta, x: i32, z: i32) -> Option<i32> {
    let top = BASE_HEIGHT + 14; // 與 surface_block 同上界，涵蓋正常樹高
    (0..=top).find(|&y| voxel::effective_block_at(world, x, y, z) == Block::Wood)
}

/// 螺旋掃指定半徑環，對每個 (x,z) 呼叫 `pick`；回傳第一個 `Some` 的結果。
/// 把「從 MIN_RADIUS 起一圈圈往外找最近」的骨架抽出來，木頭/地表兩種找法共用。
fn spiral_find<T>(
    ox: i32,
    oz: i32,
    max_radius: i32,
    mut pick: impl FnMut(i32, i32) -> Option<T>,
) -> Option<T> {
    for r in GATHER_MIN_RADIUS..=max_radius {
        for dx in -r..=r {
            for dz in -r..=r {
                // 只走當前半徑的「環」邊界，避免重複掃內圈。
                if dx.abs().max(dz.abs()) != r {
                    continue;
                }
                if let Some(t) = pick(ox + dx, oz + dz) {
                    return Some(t);
                }
            }
        }
    }
    None
}

/// 從 (ox,oz) 螺旋向外找最近一個「可採資源」。
///
/// **優先序**：先找**木頭（樹幹）**，沒有才找地表草／沙／泥／石。原因：草皮鋪滿整個地表、
/// 一定搶在樹之前被「最近」選中，若不優先找木頭，居民幾乎永遠採不到木頭、合成鏈第一步
/// （木頭→木板）就接不上。木頭稀少又是鏈起點，故只要採集半徑內有可砍的樹就先去砍。
/// 地表材料到處都是、隨時補得到，不會因此匱乏。
///
/// 從 [`GATHER_MIN_RADIUS`] 起找（讓居民走幾步），找到 [`GATHER_MAX_RADIUS`] 仍無 → None。
/// 純函式（吃 &WorldDelta）、可測。
pub fn find_nearest_resource(
    world: &WorldDelta,
    ox: i32,
    oz: i32,
    max_radius: i32,
) -> Option<(i32, i32, i32, GatherResource)> {
    // 居民站立柱的地表頂 → 推估腳底層（fy），用來剔除「挖了會把自己困住」的目標
    // （腳下那柱、明顯低於腳底的坑底）。找不到站立柱（不該發生）就不過濾、退回原行為。
    let foot_fy = surface_block(world, ox, oz).map(|(y, _)| y + 1);
    let escapable = |x: i32, y: i32, z: i32| {
        foot_fy.map_or(true, |fy| is_escapable_after_dig(world, ox, fy, oz, x, y, z))
    };

    // 第一優先：最近一棵「砍得到」的樹（側向砍最低樹幹塊，近地表、安全可逃）。
    // 先用 surface_block 便宜判斷該柱是不是樹（地表頂為樹冠/樹幹），是才掃樹幹底，避免空地全掃。
    if let Some(w) = spiral_find(ox, oz, max_radius, |x, z| {
        let (_, b) = surface_block(world, x, z)?;
        if !matches!(b, Block::Leaves | Block::Wood) {
            return None;
        }
        let wy = trunk_base(world, x, z)?;
        escapable(x, wy, z).then_some((x, wy, z, GatherResource::Wood))
    }) {
        return Some(w);
    }

    // 退而求其次：最近一塊可採地表方塊（草／沙／泥／石，非水；跳過樹冠擋住的樹柱）。
    spiral_find(ox, oz, max_radius, |x, z| {
        let (y, b) = surface_block(world, x, z)?;
        if matches!(b, Block::Leaves | Block::Wood) {
            return None; // 樹柱已在木頭階段處理（砍不到就不採其地表）
        }
        let res = GatherResource::from_block(b)?;
        escapable(x, y, z).then_some((x, y, z, res))
    })
}

/// 從 (ox,oz) 螺旋向外找最近一個「可採指定型別」的方塊（給「指名要採 XX」的跑腿任務用，
/// ROADMAP·指令→任務第三刀）。與 [`find_nearest_resource`] 的差別：後者是「就地取材、
/// 木頭優先」的背景採集找法；本函式是「一定要找到這個特定種類」，沒有優先序，找不到就回 `None`
/// （呼叫端據此決定放棄或帶著已採到的先回去交差）。純函式（吃 &WorldDelta）、可測。
pub fn find_nearest_resource_of(
    world: &WorldDelta,
    ox: i32,
    oz: i32,
    max_radius: i32,
    want: GatherResource,
) -> Option<(i32, i32, i32)> {
    let foot_fy = surface_block(world, ox, oz).map(|(y, _)| y + 1);
    let escapable = |x: i32, y: i32, z: i32| {
        foot_fy.map_or(true, |fy| is_escapable_after_dig(world, ox, fy, oz, x, y, z))
    };
    if want == GatherResource::Wood {
        return spiral_find(ox, oz, max_radius, |x, z| {
            let (_, b) = surface_block(world, x, z)?;
            if !matches!(b, Block::Leaves | Block::Wood) {
                return None;
            }
            let wy = trunk_base(world, x, z)?;
            escapable(x, wy, z).then_some((x, wy, z))
        });
    }
    spiral_find(ox, oz, max_radius, |x, z| {
        let (y, b) = surface_block(world, x, z)?;
        if GatherResource::from_block(b)? != want {
            return None;
        }
        escapable(x, y, z).then_some((x, y, z))
    })
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
    /// 建物錨點世界座標（供「回家遮蔽」等需要地點的行為查詢）。
    /// `Option` 供舊資料向後相容（舊行沒有這三欄，載回時視為 `None`）。
    #[serde(default)]
    pub x: Option<i32>,
    #[serde(default)]
    pub y: Option<i32>,
    #[serde(default)]
    pub z: Option<i32>,
    /// 這筆是否為「擴建」（基礎四種都蓋完後再蓋的第 2 座）而非首次完成。
    /// `#[serde(default)]` 對舊資料向後相容（舊行沒有這欄，一律視為 `false`＝首次完成）。
    #[serde(default)]
    pub expansion: bool,
}

/// 每居民「已完成建物種類」集合 store。讓 `choose_activity` 永不重選蓋過的種類。
#[derive(Default)]
pub struct GoalStore {
    /// resident id → 已完成的 BuildKind 字串集合（去重）。
    done: HashMap<String, Vec<String>>,
    /// resident id → 已蓋好的小屋世界座標（ROADMAP 夜間歸巢遮蔽用；只記 House）。
    houses: HashMap<String, (i32, i32, i32)>,
    /// resident id → 已擴建次數（見 [`MAX_EXPANSIONS`]）。擴建記錄不進 `done`（種類早就在裡面了），
    /// 只在這裡累計次數，避免無止盡擴建。
    expansions: HashMap<String, u32>,
    next_seq: u64,
}

impl GoalStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 由 jsonl 記錄還原（重啟後仍記得蓋過什麼 → 不會重蓋；也還原小屋座標／擴建次數）。
    pub fn from_entries(entries: Vec<GoalRecord>) -> Self {
        let mut s = Self::default();
        for e in entries {
            if e.seq >= s.next_seq {
                s.next_seq = e.seq.wrapping_add(1);
            }
            if e.expansion {
                // 擴建不進 done 集合（種類早已在裡面）、也不覆蓋小屋座標
                // （保留最初那間供夜間歸巢遮蔽查詢，擴建的第 2 間不取代它）。
                *s.expansions.entry(e.resident.clone()).or_insert(0) += 1;
                continue;
            }
            if e.kind == BuildKind::House.as_str() {
                if let (Some(x), Some(y), Some(z)) = (e.x, e.y, e.z) {
                    s.houses.insert(e.resident.clone(), (x, y, z));
                }
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

    /// 此居民已擴建過幾座（基礎四種蓋完後的追加建物；上限見 [`MAX_EXPANSIONS`]）。
    pub fn expansion_count(&self, resident: &str) -> u32 {
        self.expansions.get(resident).copied().unwrap_or(0)
    }

    /// 標記某居民完成了某建物（附上錨點座標）；回傳新 record 供呼叫端 append 落地。
    /// 已存在則回 None（不重複落地）。
    pub fn mark_done(&mut self, resident: &str, kind: BuildKind, loc: (i32, i32, i32)) -> Option<GoalRecord> {
        let v = self.done.entry(resident.to_string()).or_default();
        if v.iter().any(|k| k == kind.as_str()) {
            return None;
        }
        v.push(kind.as_str().to_string());
        if kind == BuildKind::House {
            self.houses.insert(resident.to_string(), loc);
        }
        let rec = GoalRecord {
            resident: resident.to_string(),
            kind: kind.as_str().to_string(),
            seq: self.next_seq,
            x: Some(loc.0),
            y: Some(loc.1),
            z: Some(loc.2),
            expansion: false,
        };
        self.next_seq = self.next_seq.wrapping_add(1);
        Some(rec)
    }

    /// 標記某居民擴建完成了一座（種類早已蓋過，這是追加的第 2 座）；回傳新 record 供
    /// 呼叫端 append 落地。擴建次數不設上限（呼叫端 [`choose_activity`] 已用
    /// [`MAX_EXPANSIONS`] 把關，這裡單純記錄，職責分離、跟 `mark_done` 同一種手法）。
    pub fn mark_expansion(
        &mut self,
        resident: &str,
        kind: BuildKind,
        loc: (i32, i32, i32),
    ) -> GoalRecord {
        *self.expansions.entry(resident.to_string()).or_insert(0) += 1;
        let rec = GoalRecord {
            resident: resident.to_string(),
            kind: kind.as_str().to_string(),
            seq: self.next_seq,
            x: Some(loc.0),
            y: Some(loc.1),
            z: Some(loc.2),
            expansion: true,
        };
        self.next_seq = self.next_seq.wrapping_add(1);
        rec
    }

    /// 此居民已蓋好的小屋世界座標（沒蓋過小屋則 `None`）。供夜間歸巢遮蔽查詢。
    pub fn house_of(&self, resident: &str) -> Option<(i32, i32, i32)> {
        self.houses.get(resident).copied()
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
        assert_eq!(choose_activity(&[], None, 0, 2, 0), NextActivity::Gather);
        assert_eq!(choose_activity(&[], None, 1, 2, 0), NextActivity::Gather);
        // 採滿配額 → 蓋下一個目標（花圃）。
        assert_eq!(choose_activity(&[], None, 2, 2, 0), NextActivity::Build(BuildKind::Garden));
    }

    #[test]
    fn choose_wander_when_all_built_and_no_desire() {
        let all = [BuildKind::Garden, BuildKind::House, BuildKind::Well, BuildKind::Tower];
        // 全蓋完、沒有渴望，不論採集次數都回 Wander（不再重蓋）。
        assert_eq!(choose_activity(&all, None, 0, 2, 0), NextActivity::Wander);
        assert_eq!(choose_activity(&all, None, 5, 2, 0), NextActivity::Wander);
    }

    #[test]
    fn choose_build_respects_desire() {
        // 心願塔、採滿配額 → 蓋塔。
        assert_eq!(
            choose_activity(&[], Some(BuildKind::Tower), 2, 2, 0),
            NextActivity::Build(BuildKind::Tower)
        );
    }

    // ── 擴建 v1：全蓋完後，具體渴望不再石沉大海 ─────────────────────────────

    #[test]
    fn choose_expand_when_all_built_but_desire_and_slots_remain() {
        let all = [BuildKind::Garden, BuildKind::House, BuildKind::Well, BuildKind::Tower];
        // 全蓋完 + 想再要一座水井（即使早蓋過）+ 擴建額度未滿 + 採滿配額 → 擴建。
        assert_eq!(
            choose_activity(&all, Some(BuildKind::Well), 2, 2, 0),
            NextActivity::Expand(BuildKind::Well)
        );
    }

    #[test]
    fn choose_gather_before_expand() {
        let all = [BuildKind::Garden, BuildKind::House, BuildKind::Well, BuildKind::Tower];
        // 擴建前也要先採滿配額（跟一般蓋造待遇一致）。
        assert_eq!(
            choose_activity(&all, Some(BuildKind::Well), 0, 2, 0),
            NextActivity::Gather
        );
    }

    #[test]
    fn choose_wander_when_expansion_cap_reached() {
        let all = [BuildKind::Garden, BuildKind::House, BuildKind::Well, BuildKind::Tower];
        // 擴建額度已滿（MAX_EXPANSIONS=2）→ 即使仍有渴望，也回 Wander，不再無止盡擴建。
        assert_eq!(
            choose_activity(&all, Some(BuildKind::Well), 2, 2, MAX_EXPANSIONS),
            NextActivity::Wander
        );
    }

    #[test]
    fn choose_wander_when_all_built_and_no_desire_even_with_slots() {
        let all = [BuildKind::Garden, BuildKind::House, BuildKind::Well, BuildKind::Tower];
        // 全蓋完但沒有渴望 → 就算擴建額度還有，也不會憑空擴建（要有具體渴望才動工）。
        assert_eq!(choose_activity(&all, None, 2, 2, 0), NextActivity::Wander);
    }

    // ── GatherResource ────────────────────────────────────────────────────────

    #[test]
    fn gather_resource_block_roundtrip() {
        for res in [
            GatherResource::Grass,
            GatherResource::Sand,
            GatherResource::Dirt,
            GatherResource::Stone,
            GatherResource::Wood,
        ] {
            assert_eq!(GatherResource::from_block(res.block()), Some(res));
            assert!(!res.display_name().is_empty());
            assert_eq!(res.block_id(), res.block() as u8);
        }
        // 不可採的方塊 → None（樹冠 Leaves 不直接採、水、空氣）。
        assert_eq!(GatherResource::from_block(Block::Leaves), None);
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

    // ── 採木頭：補上合成鏈缺的木頭來源 ────────────────────────────────────────

    /// 找一棵「旁邊有同高平地站得到、砍得到」的樹，回 (ox, oz, 樹)。
    /// 站立柱與樹底座同高 → 砍最低樹幹塊一定通過可逃性判定（不挖坑、踏地即可）。
    fn tree_with_flat_neighbor() -> (i32, i32, voxel::Tree) {
        for cx in 0..600 {
            for cz in -300..300 {
                if let Some(t) = voxel::tree_in_cell(cx, cz) {
                    // 站在樹西邊 5 格（chebyshev 5，落在 [MIN,MAX] 採集半徑內）。
                    let (ox, oz) = (t.tx - 5, t.tz);
                    if height_at(ox, oz) == t.base_h {
                        return (ox, oz, t);
                    }
                }
            }
        }
        panic!("找不到旁邊有平地的樹");
    }

    #[test]
    fn trunk_base_finds_lowest_wood() {
        let world = WorldDelta::new();
        let t = (0..200)
            .flat_map(|cx| (0..200).map(move |cz| (cx, cz)))
            .find_map(|(cx, cz)| voxel::tree_in_cell(cx, cz))
            .expect("應找得到一棵樹");
        // 最低樹幹塊就在草地表之上一格。
        assert_eq!(trunk_base(&world, t.tx, t.tz), Some(t.base_h + 1));
        assert_eq!(
            voxel::effective_block_at(&world, t.tx, t.base_h + 1, t.tz),
            Block::Wood
        );
    }

    #[test]
    fn find_nearest_resource_prefers_wood_when_tree_in_range() {
        // 採集半徑內有可砍的樹 → 應優先回木頭（否則鋪滿地表的草會搶先、居民永遠採不到木）。
        let world = WorldDelta::new();
        let (ox, oz, _t) = tree_with_flat_neighbor();
        let (x, y, z, res) =
            find_nearest_resource(&world, ox, oz, GATHER_MAX_RADIUS).expect("應找得到資源");
        assert_eq!(res, GatherResource::Wood, "半徑內有樹 → 優先採木頭");
        // 目標真的是樹幹 Wood，且砍得到（可逃性通過）。
        assert_eq!(voxel::effective_block_at(&world, x, y, z), Block::Wood);
        let fy = height_at(ox, oz) + 1;
        assert!(
            is_escapable_after_dig(&world, ox, fy, oz, x, y, z),
            "居民應砍得到這塊樹幹（不把自己困住）"
        );
        // 居民真的能拿到木頭：模擬砍掉這塊 → 變空氣（材料入背包）。
        let mut w2 = world.clone();
        voxel::set_block(&mut w2, x, y, z, Block::Air);
        assert_eq!(voxel::effective_block_at(&w2, x, y, z), Block::Air);
    }

    #[test]
    fn gather_falls_back_to_ground_without_trees() {
        // 沒有樹的世界（用 delta 把附近樹幹/樹冠抹平不切實際；改驗證：找不到樹時退回地表材料）。
        // 取一個陸地點，若其半徑內剛好沒可砍的樹，結果應是地表草/沙/泥/石之一（非木頭也合法）。
        let world = WorldDelta::new();
        let (ox, oz) = land_point();
        let (x, y, z, res) =
            find_nearest_resource(&world, ox, oz, GATHER_MAX_RADIUS).expect("陸地應有可採資源");
        // 不論回木頭或地表材料，型別都與該座標方塊一致（同源、可實際挖到）。
        assert_eq!(voxel::effective_block_at(&world, x, y, z), res.block());
    }

    // ── find_nearest_resource_of：指名要採特定型別（跑腿採集用）────────────────────

    #[test]
    fn find_nearest_resource_of_wood_finds_tree() {
        let world = WorldDelta::new();
        let (ox, oz, t) = tree_with_flat_neighbor();
        let (x, y, z) = find_nearest_resource_of(&world, ox, oz, GATHER_MAX_RADIUS, GatherResource::Wood)
            .expect("指名木頭應找得到樹");
        assert_eq!((x, z), (t.tx, t.tz));
        assert_eq!(voxel::effective_block_at(&world, x, y, z), Block::Wood);
    }

    #[test]
    fn find_nearest_resource_of_matches_requested_kind_only() {
        let world = WorldDelta::new();
        let (ox, oz) = land_point();
        for want in [GatherResource::Grass, GatherResource::Sand, GatherResource::Dirt, GatherResource::Stone] {
            if let Some((x, y, z)) = find_nearest_resource_of(&world, ox, oz, GATHER_MAX_RADIUS, want) {
                assert_eq!(
                    GatherResource::from_block(voxel::effective_block_at(&world, x, y, z)),
                    Some(want),
                    "找到的方塊型別應恰好等於指名要的型別"
                );
            }
        }
    }

    #[test]
    fn find_nearest_resource_of_none_when_kind_absent_in_range() {
        // 造一片只有石頭地表的小世界（陸地點附近全填石頭），指名要沙子理應找不到。
        let mut world = WorldDelta::new();
        let (ox, oz) = land_point();
        for dx in -(GATHER_MAX_RADIUS + 2)..=(GATHER_MAX_RADIUS + 2) {
            for dz in -(GATHER_MAX_RADIUS + 2)..=(GATHER_MAX_RADIUS + 2) {
                let (x, z) = (ox + dx, oz + dz);
                let h = height_at(x, z);
                voxel::set_block(&mut world, x, h, z, Block::Stone);
                voxel::set_block(&mut world, x, h + 1, z, Block::Air);
            }
        }
        assert_eq!(
            find_nearest_resource_of(&world, ox, oz, GATHER_MAX_RADIUS, GatherResource::Sand),
            None,
            "半徑內完全沒有沙子 → 該老實回 None，不亂猜"
        );
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

    // ── is_escapable_after_dig：可逃性判定（核心保證）─────────────────────────

    /// 在世界 (x,z) 柱「就地把地表壓到指定高度 top」：把 top 之上全設空氣、top 設實心。
    /// 方便造出窪地/台階等地形來測可逃性。
    fn set_column_top(world: &mut WorldDelta, x: i32, z: i32, top: i32, b: Block) {
        // 清掉原地表上方殘留（保守掃一段），再把 top 設成實心（其下為程序地形實心）。
        for y in top + 1..top + 8 {
            voxel::set_block(world, x, y, z, Block::Air);
        }
        voxel::set_block(world, x, top, z, b);
    }

    #[test]
    fn escapable_rejects_own_column_and_deep_pit() {
        let world = WorldDelta::new();
        // 腳下那一柱 → 永遠拒挖。
        assert!(!is_escapable_after_dig(&world, 10, 30, 10, 10, 29, 10));
        // 明顯低於腳底（坑底）→ 拒挖。
        assert!(!is_escapable_after_dig(&world, 10, 30, 10, 12, 25, 10));
    }

    #[test]
    fn escapable_allows_dimple_on_flat_ground() {
        // 平地（四周等高）站立 → 腳邊掏 1 格凹陷可逃（踏階走得回）→ 允許。
        let mut world = WorldDelta::new();
        let (ox, oz) = land_point();
        let h = height_at(ox, oz);
        // 把腳下與四鄰壓平到同高 h（造一片平地）。
        for (dx, dz) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1), (2, 0)] {
            set_column_top(&mut world, ox + dx, oz + dz, h, Block::Grass);
        }
        let fy = h + 1; // 站在平地上腳底
                        // 挖腳底層（ty == fy-1 == h）的鄰柱 → 平地非窪地 → 允許。
        assert!(is_escapable_after_dig(&world, ox, fy, oz, ox + 2, h, oz));
    }

    #[test]
    fn escapable_forbids_deepening_when_in_depression() {
        // 居民身處窪地（某鄰柱比腳下高）→ 禁止再挖腳底層（防越挖越深、接成爬不出的坑）。
        // 這正是舊版 safe_to_dig 擋不住、累計脫困 291 次的根因：它允許在窪地裡續挖 fy-1。
        let mut world = WorldDelta::new();
        let (ox, oz) = land_point();
        let h = height_at(ox, oz);
        // 腳下柱壓到 h；左鄰更高 (h+1) → 形成窪地。
        set_column_top(&mut world, ox, oz, h, Block::Grass);
        set_column_top(&mut world, ox - 1, oz, h + 1, Block::Grass);
        set_column_top(&mut world, ox + 1, oz, h, Block::Grass);
        let fy = h + 1;
        // 舊幾何閘會放行（ty==fy-1），但可逃性判定因「在窪地」而拒挖。
        assert!(safe_to_dig(ox, fy, oz, ox + 1, h, oz), "前置：幾何閘本會放行");
        assert!(
            !is_escapable_after_dig(&world, ox, fy, oz, ox + 1, h, oz),
            "窪地裡不該再往下挖（否則越挖越深）"
        );
    }

    #[test]
    fn escapable_allows_shaving_higher_neighbor_in_depression() {
        // 即使在窪地，仍可「削平更高的鄰柱」（ty >= fy）——這只會讓地變平、不會挖出坑。
        let mut world = WorldDelta::new();
        let (ox, oz) = land_point();
        let h = height_at(ox, oz);
        set_column_top(&mut world, ox, oz, h, Block::Grass);
        set_column_top(&mut world, ox - 1, oz, h + 1, Block::Grass); // 窪地
        set_column_top(&mut world, ox + 1, oz, h + 1, Block::Grass); // 較高鄰柱
        let fy = h + 1;
        // 削平 (ox+1) 的頂（ty = h+1 = fy，腳同層或更高）→ 允許（變平、可逃）。
        assert!(is_escapable_after_dig(&world, ox, fy, oz, ox + 1, h + 1, oz));
    }

    // ── staircase_well：往下採礦的樓梯井範本（相鄰階可走回地面）──────────────────

    #[test]
    fn staircase_steps_are_walkable() {
        // 每一階「踏面相鄰且垂直差 1」→ 居民踏階即可上下，永遠走得回地面。
        let depth = 5;
        let cells = staircase_well(100, 20, 100, depth);
        // 每階清出 HEADROOM 格 → 總清格數 = depth * HEADROOM。
        assert_eq!(cells.len() as i32, depth * STAIR_HEADROOM);
        // 還原每階踏面 (x,z,tread_y) 並驗證相鄰階可走。
        let (dx, dz) = STAIR_DIR;
        for step in 0..depth {
            let x = 100 + dx * step;
            let z = 100 + dz * step;
            let tread = 20 - step;
            // 該階頭頂淨空都被清出。
            for hh in 1..=STAIR_HEADROOM {
                assert!(cells.contains(&(x, tread + hh, z)), "頭頂淨空未清：階 {step}");
            }
            if step + 1 < depth {
                let nx = 100 + dx * (step + 1);
                let nz = 100 + dz * (step + 1);
                let ntread = 20 - (step + 1);
                // 水平相鄰（曼哈頓距 1）。
                assert_eq!((nx - x).abs() + (nz - z).abs(), 1, "相鄰階應水平相鄰");
                // 垂直差恰 1（踏階可上下）。
                assert_eq!((tread - ntread).abs(), 1, "相鄰階垂直差應為 1（可踏階）");
            }
        }
    }

    #[test]
    fn staircase_zero_depth_is_empty() {
        assert!(staircase_well(0, 10, 0, 0).is_empty());
        assert!(staircase_well(0, 10, 0, -3).is_empty());
    }

    // ── 實測證據：大量連續採集，居民永不把自己挖坑卡住（脫困趨近 0）────────────────
    //
    // 這是本次修復的關鍵驗證：用真實程序地形 + 居民真實物理（重力/逐軸碰撞/踏階），
    // 鏡像 production 的採集迴圈（找資源→走過去→可逃性判定→挖），壓力連跑數千 tick，
    // 證明「居民不再受困」（rescue 事件 = 0），且採集仍真的拿到材料（mined > 0）。

    #[test]
    fn simulated_mass_gathering_never_traps_residents() {
        use crate::voxel_residents as vr;

        struct Sim {
            body: vr::Body,
            gather: Option<(i32, i32, i32, GatherResource, f32)>,
            stuck: f32,
        }

        let mut world = WorldDelta::new();
        let dt = 1.0 / 30.0;

        // 多位居民散在不同陸地起點，各自連續採集（壓力測試）。
        let mut sims: Vec<Sim> = Vec::new();
        for base in [40, 350, 800, 1500, 2300] {
            let mut start = None;
            for c in base..base + 600 {
                if height_at(c, 0) > SEA_LEVEL + 3 {
                    start = Some((c, 0));
                    break;
                }
            }
            if let Some((sx, sz)) = start {
                sims.push(Sim { body: vr::dry_ground_spawn(sx, sz), gather: None, stuck: 0.0 });
            }
        }
        assert!(sims.len() >= 3, "應找得到數個陸地起點");

        // 先讓每位落穩。
        for s in sims.iter_mut() {
            for _ in 0..40 {
                vr::gravity_step(&world, &mut s.body, dt);
            }
        }

        let mut rescues = 0u32;
        let mut mined = 0u32;
        let ticks = 6000;

        for _tick in 0..ticks {
            for s in sims.iter_mut() {
                let (px, pz) = (s.body.x, s.body.z);

                if let Some((tx, ty, tz, res, ref mut timeout)) = s.gather {
                    *timeout -= dt;
                    let reached = within_gather_reach(s.body.x, s.body.z, tx, tz);
                    if reached {
                        let (fx, fy, fz) = (
                            s.body.x.floor() as i32,
                            s.body.y.floor() as i32,
                            s.body.z.floor() as i32,
                        );
                        // 可逃性判定（與 production 同一把鎖）：通過才真的挖。
                        if is_escapable_after_dig(&world, fx, fy, fz, tx, ty, tz)
                            && voxel::effective_block_at(&world, tx, ty, tz) == res.block()
                        {
                            voxel::set_block(&mut world, tx, ty, tz, Block::Air);
                            mined += 1;
                        }
                        s.gather = None;
                        vr::gravity_step(&world, &mut s.body, dt);
                    } else if *timeout <= 0.0 {
                        s.gather = None;
                        vr::gravity_step(&world, &mut s.body, dt);
                    } else {
                        vr::step_toward(
                            &world, &mut s.body,
                            tx as f32 + 0.5, tz as f32 + 0.5, dt, vr::RES_SPEED,
                        );
                    }
                } else {
                    // 沒在採集 → 立刻找下一個資源（壓力：盡量多挖、模擬居民一直採）。
                    let (ox, oz) = (s.body.x.floor() as i32, s.body.z.floor() as i32);
                    match find_nearest_resource(&world, ox, oz, GATHER_MAX_RADIUS) {
                        Some((tx, ty, tz, res)) => {
                            s.gather = Some((tx, ty, tz, res, GATHER_TIMEOUT_SECS));
                        }
                        None => {
                            // 附近沒可採資源 → 往旁邊挪一點換地方再找（不卡死）。
                            let (wx, wz) = (s.body.x + 3.0, s.body.z);
                            vr::step_toward(&world, &mut s.body, wx, wz, dt, vr::RES_SPEED);
                        }
                    }
                }

                // 卡住偵測（與 production 同邏輯）：只有「純導航（沒在採集）+ 幾何被困」才累加。
                let moved = ((s.body.x - px).powi(2) + (s.body.z - pz).powi(2)).sqrt();
                let navigating = s.gather.is_none();
                let confined = navigating && vr::is_confined(&world, &s.body);
                s.stuck = vr::update_stuck_timer(s.stuck, moved, navigating, confined, dt);
                if s.stuck >= vr::STUCK_SECS {
                    rescues += 1;
                    vr::rescue_resident(&world, &mut s.body, px, pz, vr::UNSTUCK_MAX_LIFT);
                    s.stuck = 0.0;
                    s.gather = None;
                }
            }
        }

        // 觀察 log（`cargo test -- --nocapture` 看得到）：採集量 + 脫困次數。
        println!(
            "[採集壓力測] 居民={} tick={} 採到方塊={} 脫困次數={}",
            sims.len(), ticks, mined, rescues
        );
        // 採集仍真的拿到材料。
        assert!(mined > 0, "壓力採集應挖到材料：mined={mined}");
        // 核心保證：居民永不被自己的採集坑困到觸發脫困。
        assert_eq!(rescues, 0, "採集永不該把居民困到觸發脫困：rescues={rescues}");
    }

    // ── GoalStore：不重複的記憶 ──────────────────────────────────────────────

    #[test]
    fn goal_store_mark_and_query() {
        let mut s = GoalStore::new();
        assert!(!s.is_done("vox_res_0", BuildKind::Garden));
        let rec = s.mark_done("vox_res_0", BuildKind::Garden, (1, 2, 3));
        assert!(rec.is_some());
        assert!(s.is_done("vox_res_0", BuildKind::Garden));
        assert_eq!(s.done_count("vox_res_0"), 1);
        // 別的居民不受影響。
        assert!(!s.is_done("vox_res_1", BuildKind::Garden));
    }

    #[test]
    fn goal_store_mark_twice_is_idempotent() {
        let mut s = GoalStore::new();
        assert!(s.mark_done("r", BuildKind::Well, (0, 0, 0)).is_some());
        // 第二次標記同種 → None（不重複落地），數量不變。
        assert!(s.mark_done("r", BuildKind::Well, (0, 0, 0)).is_none());
        assert_eq!(s.done_count("r"), 1);
    }

    #[test]
    fn goal_store_drives_non_repeat_goal() {
        let mut s = GoalStore::new();
        // 蓋完花圃 → done_kinds 含花圃 → next_build_goal 換小屋。
        s.mark_done("r", BuildKind::Garden, (0, 0, 0));
        let done = s.done_kinds("r");
        assert_eq!(next_build_goal(&done, None), Some(BuildKind::House));
    }

    #[test]
    fn goal_store_house_of_tracks_location_only_for_house() {
        let mut s = GoalStore::new();
        assert_eq!(s.house_of("r"), None);
        // 花圃不是小屋，不記地點。
        s.mark_done("r", BuildKind::Garden, (5, 6, 7));
        assert_eq!(s.house_of("r"), None);
        // 小屋才記地點。
        s.mark_done("r", BuildKind::House, (10, 20, 30));
        assert_eq!(s.house_of("r"), Some((10, 20, 30)));
        // 別的居民不受影響。
        assert_eq!(s.house_of("other"), None);
    }

    #[test]
    fn goal_store_from_entries_restores() {
        let entries = vec![
            GoalRecord { resident: "r".into(), kind: "garden".into(), seq: 0, x: Some(1), y: Some(2), z: Some(3), expansion: false },
            GoalRecord { resident: "r".into(), kind: "house".into(), seq: 1, x: Some(10), y: Some(20), z: Some(30), expansion: false },
            // 重複行：去重。
            GoalRecord { resident: "r".into(), kind: "garden".into(), seq: 2, x: Some(1), y: Some(2), z: Some(3), expansion: false },
        ];
        let s = GoalStore::from_entries(entries);
        assert!(s.is_done("r", BuildKind::Garden));
        assert!(s.is_done("r", BuildKind::House));
        assert_eq!(s.done_count("r"), 2, "重複種類應去重");
        // 重啟後 next 應跳過已蓋的兩種 → 水井。
        assert_eq!(next_build_goal(&s.done_kinds("r"), None), Some(BuildKind::Well));
        // 小屋座標也還原回來（供夜間歸巢遮蔽）。
        assert_eq!(s.house_of("r"), Some((10, 20, 30)));
    }

    #[test]
    fn goal_store_from_entries_tolerates_missing_location() {
        // 舊資料沒有 x/y/z 欄位（serde default → None）：不應 panic，house_of 安全回 None。
        let entries = vec![
            GoalRecord { resident: "r".into(), kind: "house".into(), seq: 0, x: None, y: None, z: None, expansion: false },
        ];
        let s = GoalStore::from_entries(entries);
        assert!(s.is_done("r", BuildKind::House));
        assert_eq!(s.house_of("r"), None);
    }

    #[test]
    fn goal_record_old_jsonl_without_location_deserializes() {
        // 模擬升級前寫入的舊行（沒有 x/y/z 欄位）：serde(default) 應安全補 None，不壞資料。
        let old_line = r#"{"resident":"vox_res_0","kind":"house","seq":0}"#;
        let rec: GoalRecord = serde_json::from_str(old_line).expect("舊格式應可解析");
        assert_eq!(rec.x, None);
        assert_eq!(rec.y, None);
        assert_eq!(rec.z, None);
        assert!(!rec.expansion, "舊行沒有 expansion 欄位，應安全補 false（視為首次完成）");
    }

    // ── 擴建 v1：GoalStore.mark_expansion ─────────────────────────────────────

    #[test]
    fn goal_store_mark_expansion_increments_and_records() {
        let mut s = GoalStore::new();
        assert_eq!(s.expansion_count("r"), 0);
        let rec = s.mark_expansion("r", BuildKind::Well, (1, 2, 3));
        assert_eq!(s.expansion_count("r"), 1);
        assert!(rec.expansion);
        assert_eq!(rec.kind, "well");
        // 擴建不影響 done_kinds（種類早在裡面，這只是次數累計，不重複去重進 done）。
        s.mark_done("r", BuildKind::Well, (1, 2, 3));
        assert_eq!(s.done_count("r"), 1, "擴建不應算進 done 集合");
        let _ = s.mark_expansion("r", BuildKind::Garden, (4, 5, 6));
        assert_eq!(s.expansion_count("r"), 2, "同居民累計次數");
        // 別的居民不受影響。
        assert_eq!(s.expansion_count("other"), 0);
    }

    #[test]
    fn goal_store_from_entries_restores_expansion_count_and_keeps_original_house() {
        let entries = vec![
            GoalRecord { resident: "r".into(), kind: "house".into(), seq: 0, x: Some(10), y: Some(20), z: Some(30), expansion: false },
            // 擴建的第 2 間小屋：不應覆蓋原本的小屋座標（夜間歸巢遮蔽要認原屋）。
            GoalRecord { resident: "r".into(), kind: "house".into(), seq: 1, x: Some(99), y: Some(99), z: Some(99), expansion: true },
        ];
        let s = GoalStore::from_entries(entries);
        assert_eq!(s.expansion_count("r"), 1);
        assert_eq!(s.house_of("r"), Some((10, 20, 30)), "擴建記錄不應覆蓋原屋座標");
        // 擴建記錄不重複算進 done_count（done_count 只看基礎種類，這裡只有 1 種 house）。
        assert_eq!(s.done_count("r"), 1);
    }

    #[test]
    fn goal_store_jsonl_roundtrip() {
        let dir = std::env::temp_dir().join(format!("voxgoal_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("voxel_goals.jsonl");
        let _ = std::fs::remove_file(&path);
        let pstr = path.to_str().unwrap();
        let r1 = GoalRecord { resident: "vox_res_0".into(), kind: "garden".into(), seq: 0, x: Some(1), y: Some(2), z: Some(3), expansion: false };
        let r2 = GoalRecord { resident: "vox_res_0".into(), kind: "house".into(), seq: 1, x: Some(10), y: Some(20), z: Some(30), expansion: false };
        write_line(pstr, &serde_json::to_string(&r1).unwrap());
        write_line(pstr, &serde_json::to_string(&r2).unwrap());
        let loaded = read_lines(pstr);
        assert_eq!(loaded.len(), 2);
        let s = GoalStore::from_entries(loaded);
        assert_eq!(s.done_count("vox_res_0"), 2);
        assert_eq!(s.house_of("vox_res_0"), Some((10, 20, 30)));
        let _ = std::fs::remove_file(&path);
    }
}
