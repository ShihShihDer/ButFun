//! Voxel 世界核心（AI 生態世界 voxel 基底·切片①）。
//!
//! 這裡是「方塊世界的真相」——與渲染無關的確定性世界邏輯，延續 ButFun
//! 「後端權威、前端只畫」的骨架（對齊 `world-core` 的設計哲學，但 voxel 的東西
//! 刻意**全部隔離在本模組**，不碰 game/state/ws/protocol）。
//!
//! 切片① 只做「可走的 voxel 世界」：
//! - 方塊型別 `Block`、16³ 的 `Chunk`、`ChunkCoord` 索引。
//! - 自寫 hash value noise → 確定性程序化地形（高度圖 + 分層填方塊）。
//! - `pack_chunk`：把一個 chunk 壓成精簡 base64，供 WS 串給前端；全空氣回 `None`。
//!
//! 設計取捨：本輪伺服器其實**無狀態程序生成**——不在記憶體存 chunk，收到請求就地用噪聲
//! 算出來。省記憶體、天然無限延伸；之後切片②（放/挖方塊）再加「方塊改動 overlay」即可。
//! 一切函式皆確定性純函式，好測也讓多人之間天然一致（同座標永遠同方塊）。

use base64::Engine;
use std::collections::HashMap;

/// 一個 chunk 的邊長（方塊數）。16³ = 4096 方塊／chunk。
pub const CHUNK: i32 = 16;
/// 一個 chunk 的方塊總數（4096）。
pub const CHUNK_VOL: usize = (CHUNK * CHUNK * CHUNK) as usize;

/// 地形基準高度（世界方塊 Y）。噪聲在此之上下起伏。
pub const BASE_HEIGHT: i32 = 8;
/// 海平面（世界方塊 Y）。高度低於此、且在地表之上的空格填水。
pub const SEA_LEVEL: i32 = 5;
/// 地形噪聲種子（固定 → 全世界、前後端、多人之間地貌一致）。
pub const SEED: u32 = 0x_B0_07_Fu32; // "BOOTF"un · voxel

// ── 程序生成樹（補上合成鏈缺的木頭來源·確定性、可測）────────────────────────────
//
// 樹是「地形的一部分」：走 generate/`block_at` 路徑、**不靠 delta**——同 chunk／同座標永遠
// 長一樣（可測、多人/前後端天然一致）。設計把世界切成 `TREE_CELL`×`TREE_CELL` 的格，每格
// 至多一棵、樹幹落在格內側（留 1 格邊），使「半徑 1 的樹冠」**永不跨格** → `block_at` 只需
// 查「自己這格」的樹，O(1) 無鄰格掃描。樹只長在草地表面、避開水邊與出生保護圈。

/// 種樹的格邊長（每格至多一棵樹）。控制密度、且讓半徑 1 樹冠不跨格。
pub const TREE_CELL: i32 = 7;
/// 一格長出樹的機率門檻（per-cell hash < 門檻才長）。約 0.5 → 自然疏密，不擋路也找得到。
const TREE_CHANCE: f32 = 0.5;
/// 樹幹高度下限（含）。
const TREE_MIN_TRUNK: i32 = 3;
/// 樹幹高度上限（含）。
const TREE_MAX_TRUNK: i32 = 5;
/// 出生保護圈半徑（世界方塊）：保護錨點周圍不長樹，免樹幹卡住出生／歸巢點。
pub const SPAWN_PROTECT_RADIUS: i32 = 8;
/// 出生保護錨點（世界座標）：玩家出生原點 + 各居民家域中心
/// （對齊 `voxel_residents::resident_home_base`，刻意以小而穩定的常數複述，維持模組分層）。
const SPAWN_ANCHORS: [(i32, i32); 4] = [(0, 0), (0, 75), (-75, 0), (75, 0)];
/// 種樹噪聲種子（與地形種子分流，避免樹的分布與高度圖相關）。
const TREE_SEED: u32 = SEED ^ 0x_5EED_7EE5;

/// 一棵樹的描述：樹幹底座所在柱 (tx,tz)、底座草地表高度 base_h、樹幹高度 trunk（格數）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tree {
    pub tx: i32,
    pub tz: i32,
    pub base_h: i32,
    pub trunk: i32,
}

/// 方塊型別。`#[repr(u8)]` → 直接當 1 byte 串流（pack_chunk 用）。
/// ID 0–7：自然生成方塊；8–10：合成台 v1（ROADMAP 658）玩家合成方塊。
/// ID 11–13：種田 v1（ROADMAP 659）農地狀態方塊。
/// ID 14：種子（純背包物品，無實體方塊，voxel_farm::SEEDS_ID）。
/// ID 15：工作台（ROADMAP 665）玩家合成+放置，互動開 3×3 合成格。
/// ID 18–19：小麥/麵包（純背包物品，voxel_farm）。
/// ID 20–21：深層礦石（ROADMAP 682）煤礦/鐵礦，生成於地底石層，可採集+放置。
/// ID 22：鐵錠（ROADMAP 683）熔爐冶煉鐵礦+煤礦所得；精緻金屬建材，可放置可送禮。
/// ID 23：鐵磚（ROADMAP 684）工作台 4 鐵錠→1 鐵磚；壓縮金屬建材，比鐵錠更光滑。
/// ID 24–30：流動水（水流動模擬）——來源水 Water=7 是 level 0/無限，24..=30 是流動 level 1..=7
/// （遞減、離源太遠乾涸）；非實心、碰撞/挖放規則同 Water；id 定義集中在 voxel_water。
/// ID 31：火把（ROADMAP 685）背包 2×2：1 木+1 煤礦→4 火把；橘黃燈柱，礦坑標記/裝飾用。
/// ID 54：仙人掌（生物群系第一刀）沙漠群系程序生成，2格高柱狀，可採集+放置。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Block {
    Air = 0,
    Grass = 1,
    Dirt = 2,
    Stone = 3,
    Sand = 4,
    Wood = 5,
    Leaves = 6,
    Water = 7,
    /// 木板（2 木 → 4 木板）。比原木更工整，可當牆面/地板建材。
    Plank = 8,
    /// 石磚（2 石 → 2 石磚）。精緻石材，適合蓋正式建築。
    StoneBrick = 9,
    /// 玻璃（2 沙 → 1 玻璃）。半透明質感，窗戶或裝飾用。
    Glass = 10,
    /// 農田土（2 泥土 → 2 農田土，合成台 till 配方）。可種植種子。
    FarmSoil = 11,
    /// 幼苗（農田土種下種子後的生長狀態，~90 秒後自動長成成熟小麥）。
    FarmSoilSeeded = 12,
    /// 成熟小麥（可收割；破壞後掉落種子×2 + 農田土×1，自我延續的種田循環）。
    WheatMature = 13,
    /// 工作台（4 木板 → 1 工作台；放置後右鍵互動開啟 3×3 合成格）。
    Workbench = 15,
    /// 熔爐（8 石頭在工作台圍成一圈 → 1 熔爐；放置後右鍵互動開冶煉面板）。
    Furnace = 16,
    /// 拋光石（3 石頭在熔爐冶煉 → 3 拋光石；精緻灰石建材，比原石光滑）。
    SmoothStone = 17,
    /// 煤礦（ROADMAP 682）——深層石頭中有機率生成（y ≤ COAL_ORE_DEPTH）；
    /// 採集後回收礦方塊本身（可放置），送給居民有特別驚喜反應。
    CoalOre = 20,
    /// 鐵礦（ROADMAP 682）——比煤礦更深、更稀少（y ≤ IRON_ORE_DEPTH）；
    /// 採集後同樣可放置，居民收到時會表現出更強烈的好奇心。
    IronOre = 21,
    /// 鐵錠（ROADMAP 683）——熔爐冶煉：1 鐵礦 + 1 煤礦 → 2 鐵錠；
    /// 閃亮銀灰金屬建材，採礦→冶煉→建造循環的第一個「精煉」產物。
    IronIngot = 22,
    /// 鐵磚（ROADMAP 684）——工作台合成：4 鐵錠 → 1 鐵磚；
    /// 壓縮金屬建材，比鐵錠更光滑整齊，適合精緻建築立面或裝飾性鋼柱。
    IronBlock = 23,
    // ── 流動水（水流動模擬）：level 1..=7，離源遞減 ──────────────────────────────
    // 來源水沿用 Water=7（level 0、無限）；以下 24..=30 是「向外擴散的流動水」。
    // 非實心、碰撞/挖放同 Water；玩家不可手動放置（伺服器模擬維護的狀態方塊）。
    /// 流動水 level 1（最強、緊鄰來源/下灌）。
    WaterFlow1 = 24,
    /// 流動水 level 2。
    WaterFlow2 = 25,
    /// 流動水 level 3。
    WaterFlow3 = 26,
    /// 流動水 level 4。
    WaterFlow4 = 27,
    /// 流動水 level 5。
    WaterFlow5 = 28,
    /// 流動水 level 6。
    WaterFlow6 = 29,
    /// 流動水 level 7（最弱、離源最遠；再遠一格就乾涸）。
    WaterFlow7 = 30,
    /// 火把（ROADMAP 685）——背包 2×2 合成：1 木頭 + 1 煤礦 → 4 火把；
    /// 橘黃燈柱，點亮礦坑黑暗隧道、標記探索路線，也是裝飾性光源方塊。
    Torch = 31,
    /// 梯子（ROADMAP 688）——背包 2×2 合成：3 木板 → 3 梯子；
    /// **非實心**——玩家可穿入；站在梯子方格中可垂直攀爬，讓深礦探索能上下自如。
    /// ID 35（跳過 32/33/34 = 純物品鎬具 ID）。
    Ladder = 35,
    /// 箱子（ROADMAP 692）——工作台合成：8 木板 → 1 箱子；
    /// 放置後右鍵互動開啟共用儲存面板，可存入/取出任意材料——
    /// 讓「採集→合成→建造基地+儲存」閉環第一次真正完整。
    /// ID 42（跳過 36~41 = 斧頭/鏟子純物品 ID）。
    Chest = 42,
    /// 木門（關）（ROADMAP 693）——背包 2×2 合成：4 木板 → 2 門；
    /// 實心，右鍵切換成開啟狀態（DoorOpen），讓建造封閉房間成為可能。
    DoorClosed = 43,
    /// 木門（開）（ROADMAP 693）——DoorClosed 右鍵後的狀態：
    /// **非實心**——玩家可穿入；DDA 仍命中此格（r>0），右鍵再關。
    /// 伺服器維護狀態，玩家不可直接放置。
    DoorOpen = 44,
    /// 床（床 v1）——背包 2×2 合成：3 木板 + 3 葉片（當被褥）→ 1 床；
    /// 放置後右鍵互動：夜晚（深夜/入夜）時睡覺跳過黑夜直達隔天黎明，
    /// 白天睡不著（無效果）。實心方塊，破壞可回收。
    Bed = 45,
    /// 胡蘿蔔幼苗（第二種作物 v1）——農田土種下胡蘿蔔種子後的生長狀態，
    /// ~60 秒（水耕 30 秒）後自動長成成熟胡蘿蔔。伺服器維護狀態，玩家不可手動放置。
    CarrotSeeded = 46,
    /// 成熟胡蘿蔔（第二種作物 v1）——可收割；破壞後掉落農田土×1 + 胡蘿蔔種子×1 + 胡蘿蔔×1，
    /// 種田系統第一次有兩種作物可選（小麥慢而多用途／胡蘿蔔快而輕巧）。
    CarrotMature = 47,
    /// 馬鈴薯幼苗（第三種作物 v1）——農田土種下馬鈴薯種子後的生長狀態，
    /// ~120 秒（水耕 60 秒）後自動長成成熟馬鈴薯。伺服器維護狀態，玩家不可手動放置。
    PotatoSeeded = 50,
    /// 成熟馬鈴薯（第三種作物 v1）——可收割；破壞後掉落農田土×1 + 馬鈴薯種子×1 + 馬鈴薯×2，
    /// 種田系統第一次有「慢但收成多」的第三種節奏（小麥居中／胡蘿蔔快而輕巧／馬鈴薯慢而量大）。
    PotatoMature = 51,
    /// 仙人掌（生物群系第一刀）——沙漠群系偶有，程序生成，確定性；直徑 1 格、高 2 格柱。
    /// 採集後可放置；沙漠地表的視覺標誌物，與沙地+無樹合力讓沙漠有地方感。
    Cactus = 54,
    /// 雪（生物群系第二刀）——雪原群系地表覆蓋（取代草），程序生成，確定性。
    /// 純白覆雪地表，一眼認出的寒冷地帶；採集後可放置，可當白色建材。
    Snow = 55,
    /// 冰晶簇（雪原冰晶採集 v1）——雪原群系地表偶有、程序生成、確定性；
    /// 直徑 1 格、高 1 格的閃亮結晶，是寒冷雪原獨有的珍稀寶物。
    /// 採集後可放置（冰藍裝飾方塊）；送給居民會換來格外驚喜的珍愛反應。
    IceCrystal = 56,
    /// 冰晶燈（冰晶合成 v1）——背包 2×2 合成：1 冰晶 + 2 玻璃 → 1 冰晶燈。
    /// 把雪原專程採回的珍稀冰晶封進玻璃，成為一盞泛著冷藍幽光的裝飾燈；
    /// 純色亮藍方塊（無動態光照，比照火把作法），是雪原遠征最有感的建造回報，
    /// 蓋冰屋時擺一盞、寒夜裡也有光。可放置、可再破壞回收。
    IceLantern = 57,
    /// 乙太礦（乙太礦脈 v1）——世界最深層（y ≤ AETHER_ORE_DEPTH，緊貼地心基岩）極稀有生成、
    /// 程序生成、確定性。這片方塊天地以「乙太」為名，而乙太礦正是它埋得最深、最難尋的招牌珍寶：
    /// 一路挖到底才可能遇上一脈泛著青藍幽光的礦。採集後可放置（發光裝飾方塊），
    /// 也是精工合成乙太燈的核心材料。
    AetherOre = 58,
    /// 乙太燈（乙太礦脈 v1）——工作台 3×3 合成：1 乙太礦 + 4 玻璃 → 1 乙太燈。
    /// 把世界最深處採回的乙太礦封進玻璃燈罩，成為一盞散發清冷青藍光的明燈——
    /// 比火把更亮更冷的高階光源，是深掘地心後最有感的建造回報。可放置、可再破壞回收。
    AetherLamp = 59,
}

impl Block {
    /// 是否為「實心、可站立／會擋路」的方塊（碰撞與面剔除用）。空氣、來源水、流動水、梯子、開門皆非實心。
    pub fn is_solid(self) -> bool {
        !matches!(self, Block::Air | Block::Water | Block::Ladder | Block::DoorOpen)
            && !self.is_flowing_water()
    }

    /// 是否為「可攀爬」方塊——玩家 AABB 重疊時取消重力、可垂直移動（目前只有梯子）。
    pub fn is_climbable(self) -> bool {
        matches!(self, Block::Ladder)
    }

    /// 是否為「流動水」（level 1..=7，id 24..=30）。來源水 Water 不算流動水（它是無限來源）。
    /// 碰撞、面剔除、破壞/放置判定都把流動水當「可穿越、不可覆蓋成建材」的水看待。
    pub fn is_flowing_water(self) -> bool {
        matches!(
            self,
            Block::WaterFlow1 | Block::WaterFlow2 | Block::WaterFlow3 | Block::WaterFlow4
                | Block::WaterFlow5 | Block::WaterFlow6 | Block::WaterFlow7
        )
    }

    /// 是否為「任何水」（來源或流動）——放置驗證用（水格可被覆蓋）。
    pub fn is_any_water(self) -> bool {
        matches!(self, Block::Water) || self.is_flowing_water()
    }

    /// 由 u8 還原方塊型別（解析客戶端 place 的方塊 id）；越界回 None。
    pub fn from_u8(v: u8) -> Option<Block> {
        match v {
            0 => Some(Block::Air),
            1 => Some(Block::Grass),
            2 => Some(Block::Dirt),
            3 => Some(Block::Stone),
            4 => Some(Block::Sand),
            5 => Some(Block::Wood),
            6 => Some(Block::Leaves),
            7 => Some(Block::Water),
            8 => Some(Block::Plank),
            9 => Some(Block::StoneBrick),
            10 => Some(Block::Glass),
            11 => Some(Block::FarmSoil),
            12 => Some(Block::FarmSoilSeeded),
            13 => Some(Block::WheatMature),
            15 => Some(Block::Workbench),
            16 => Some(Block::Furnace),
            17 => Some(Block::SmoothStone),
            20 => Some(Block::CoalOre),
            21 => Some(Block::IronOre),
            22 => Some(Block::IronIngot),
            23 => Some(Block::IronBlock),
            24 => Some(Block::WaterFlow1),
            25 => Some(Block::WaterFlow2),
            26 => Some(Block::WaterFlow3),
            27 => Some(Block::WaterFlow4),
            28 => Some(Block::WaterFlow5),
            29 => Some(Block::WaterFlow6),
            30 => Some(Block::WaterFlow7),
            31 => Some(Block::Torch),
            35 => Some(Block::Ladder),
            42 => Some(Block::Chest),
            43 => Some(Block::DoorClosed),
            44 => Some(Block::DoorOpen),
            45 => Some(Block::Bed),
            46 => Some(Block::CarrotSeeded),
            47 => Some(Block::CarrotMature),
            50 => Some(Block::PotatoSeeded),
            51 => Some(Block::PotatoMature),
            54 => Some(Block::Cactus),
            55 => Some(Block::Snow),
            56 => Some(Block::IceCrystal),
            57 => Some(Block::IceLantern),
            58 => Some(Block::AetherOre),
            59 => Some(Block::AetherLamp),
            _ => None,
        }
    }

    /// 玩家是否可「放置」此方塊（只准放 FarmSoil 和原本的實心建材；
    /// FarmSoilSeeded / WheatMature 是伺服器維護的狀態方塊，玩家不能手動放置）。
    pub fn is_placeable(self) -> bool {
        matches!(
            self,
            Block::Dirt | Block::Stone | Block::Sand | Block::Wood | Block::Grass |
            Block::Plank | Block::StoneBrick | Block::Glass | Block::FarmSoil |
            Block::Workbench | Block::Furnace | Block::SmoothStone |
            Block::CoalOre | Block::IronOre | Block::IronIngot | Block::IronBlock |
            Block::Torch | Block::Ladder | Block::Chest | Block::DoorClosed | Block::Bed |
            Block::Cactus | Block::Snow | Block::IceCrystal | Block::IceLantern |
            Block::AetherOre | Block::AetherLamp
        )
    }
}

/// chunk 在世界中的座標（以 chunk 為單位，每軸 ×CHUNK 才是世界方塊座標）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ChunkCoord {
    pub cx: i32,
    pub cy: i32,
    pub cz: i32,
}

/// 一個 chunk 的方塊資料（行主序 `x + z*CHUNK + y*CHUNK*CHUNK`）。
#[derive(Clone, Debug)]
pub struct Chunk {
    pub blocks: Vec<u8>,
}

/// chunk 內 (lx,ly,lz) → 一維索引。座標必須在 0..CHUNK。抽出來好測、好對齊前端解碼。
#[inline]
pub fn local_index(lx: i32, ly: i32, lz: i32) -> usize {
    debug_assert!((0..CHUNK).contains(&lx) && (0..CHUNK).contains(&ly) && (0..CHUNK).contains(&lz));
    (lx + lz * CHUNK + ly * CHUNK * CHUNK) as usize
}

/// 世界方塊座標 → 所屬 chunk 座標（對負數做 floor 除法，避免 -1 落到 chunk 0）。
#[inline]
pub fn chunk_of(wx: i32, wy: i32, wz: i32) -> ChunkCoord {
    ChunkCoord {
        cx: wx.div_euclid(CHUNK),
        cy: wy.div_euclid(CHUNK),
        cz: wz.div_euclid(CHUNK),
    }
}

/// 世界方塊座標 → 其所屬 chunk 內的一維局部索引（對負數做 euclid，與 chunk_of 對齊）。
#[inline]
pub fn world_local_index(wx: i32, wy: i32, wz: i32) -> usize {
    local_index(wx.rem_euclid(CHUNK), wy.rem_euclid(CHUNK), wz.rem_euclid(CHUNK))
}

// ── 方塊改動 overlay（delta 持久化層·切片②）──────────────────────────────────
//
// 地形本身是「無狀態程序生成」（block_at）。玩家／AI 改動的方塊不重寫地形，而是疊一層
// **delta 覆蓋**：per-chunk 的「被改方塊」表（局部索引 → 覆蓋方塊）。生成 chunk＝程序生成
// 後套用 delta。這層之後 AI 蓋家也會共用。本輪先記憶體存（session 內），純邏輯可測。

/// 單一 chunk 的方塊改動表：局部索引 → 覆蓋方塊。
pub type ChunkDelta = HashMap<usize, Block>;
/// 全世界的方塊改動表：chunk 座標 → 該 chunk 的改動。
pub type WorldDelta = HashMap<ChunkCoord, ChunkDelta>;

/// 觸及範圍（世界方塊單位）：玩家眼睛到方塊中心的最遠可破壞／放置距離。對齊 MCPE 手感。
pub const REACH: f32 = 6.0;
/// 玩家眼睛相對腳底（move 回報的 y）的高度，估算 reach 用。
pub const EYE_HEIGHT: f32 = 1.5;

/// 套用一個 chunk 的 delta 到已生成的方塊陣列（就地覆寫；越界索引略過保險）。
pub fn apply_delta(blocks: &mut [u8], delta: &ChunkDelta) {
    for (&li, &b) in delta {
        if li < blocks.len() {
            blocks[li] = b as u8;
        }
    }
}

/// 生成 chunk 並套用 delta（程序生成 + overlay）。
pub fn generate_chunk_with_delta(coord: ChunkCoord, delta: Option<&ChunkDelta>) -> Chunk {
    let mut chunk = generate_chunk(coord);
    if let Some(d) = delta {
        apply_delta(&mut chunk.blocks, d);
    }
    chunk
}

/// 任一世界座標的「有效方塊」：先看 delta 覆蓋層，沒有才回程序生成值。
pub fn effective_block_at(world: &WorldDelta, wx: i32, wy: i32, wz: i32) -> Block {
    let coord = chunk_of(wx, wy, wz);
    if let Some(d) = world.get(&coord) {
        if let Some(&b) = d.get(&world_local_index(wx, wy, wz)) {
            return b;
        }
    }
    block_at(wx, wy, wz)
}

/// 在世界 delta 寫入一個方塊改動（取代既有覆蓋；地心 y<0 不給動，避免破基岩掉出世界）。
pub fn set_block(world: &mut WorldDelta, wx: i32, wy: i32, wz: i32, b: Block) {
    let coord = chunk_of(wx, wy, wz);
    let li = world_local_index(wx, wy, wz);
    world.entry(coord).or_default().insert(li, b);
}

/// 觸及範圍檢查：玩家眼睛（腳底 + EYE_HEIGHT）到方塊中心的距離是否在 REACH 內（留少量餘裕）。
pub fn in_reach(px: f32, py: f32, pz: f32, bx: i32, by: i32, bz: i32) -> bool {
    let dx = (bx as f32 + 0.5) - px;
    let dy = (by as f32 + 0.5) - (py + EYE_HEIGHT);
    let dz = (bz as f32 + 0.5) - pz;
    let max = REACH + 1.0; // 餘裕：客戶端準心 raycast 與伺服器估算眼高的小誤差。
    dx * dx + dy * dy + dz * dz <= max * max
}

/// 破壞驗證：目標必須在觸及範圍內、且目前是實心方塊（空氣/水不給挖）。回傳是否允許。
pub fn can_break(world: &WorldDelta, px: f32, py: f32, pz: f32, bx: i32, by: i32, bz: i32) -> bool {
    if by < 0 {
        return false; // 地心基岩不給挖。
    }
    if !in_reach(px, py, pz, bx, by, bz) {
        return false;
    }
    let b = effective_block_at(world, bx, by, bz);
    // 一般：實心方塊可挖；特例：木門（開）雖非實心，仍視為可破壞（破後退木門關）。
    b.is_solid() || matches!(b, Block::DoorOpen)
}

/// 放置驗證：方塊型別可放、在觸及範圍內、且目標目前是空氣或水（不覆蓋既有實心方塊）。
pub fn can_place(
    world: &WorldDelta,
    px: f32,
    py: f32,
    pz: f32,
    bx: i32,
    by: i32,
    bz: i32,
    b: Block,
) -> bool {
    if !b.is_placeable() {
        return false;
    }
    if by < 0 {
        return false;
    }
    if !in_reach(px, py, pz, bx, by, bz) {
        return false;
    }
    // 目標須為空氣或任何水（來源/流動）——不覆蓋既有實心方塊。
    let t = effective_block_at(world, bx, by, bz);
    matches!(t, Block::Air) || t.is_any_water()
}

// ── 自寫 hash value noise（零外部相依、確定性、可測；不抄外部碼）─────────────────

/// 煤礦最高出現深度（y ≤ 此值且屬石頭層才生成；BASE_HEIGHT=8 下通常距地表 ≥5 格）。
pub const COAL_ORE_DEPTH: i32 = 3;
/// 鐵礦最高出現深度（y ≤ 此值且屬石頭層才生成；比煤礦更稀少且更深）。
pub const IRON_ORE_DEPTH: i32 = 1;
/// 煤礦在合格石層中的每格生成機率（2%，掃 40×4×40 區塊約有 128 格）。
pub const COAL_ORE_DENSITY: f32 = 0.020;
/// 鐵礦在合格石層中的每格生成機率（1%，比煤礦稀少）。
pub const IRON_ORE_DENSITY: f32 = 0.010;
/// 乙太礦最高出現深度（y ≤ 此值且屬石頭層才生成）——只在最深的地心層（緊貼 y<0 基岩），
/// 比鐵礦（y≤1）更深、必須一路挖到世界底才可能遇上。
pub const AETHER_ORE_DEPTH: i32 = 0;
/// 乙太礦在合格石層中的每格生成機率（0.6%，比鐵礦（1%）更稀少——全世界最難尋的礦脈）。
pub const AETHER_ORE_DENSITY: f32 = 0.006;

/// 三維確定性雜湊→[0,1)，用於礦石生成。獨立 seed 讓煤礦/鐵礦分佈不重疊。
#[inline]
fn ore_hash3(wx: i32, wy: i32, wz: i32, seed: u32) -> f32 {
    let mut h = (wx as u32)
        .wrapping_mul(0x_27d4_eb2d)
        .wrapping_add((wy as u32).wrapping_mul(0x_6c62_272e))
        .wrapping_add((wz as u32).wrapping_mul(0x_9e37_79b1))
        .wrapping_add(seed);
    h ^= h >> 15;
    h = h.wrapping_mul(0x_85eb_ca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0x_c2b2_ae35);
    h ^= h >> 16;
    (h as f32) / (u32::MAX as f32)
}

/// 整數座標 → [0,1) 的確定性雜湊（用幾個質數攪和 + xorshift finalize）。
#[inline]
fn hash2(x: i32, z: i32, seed: u32) -> f32 {
    let mut h = (x as u32)
        .wrapping_mul(0x_27d4_eb2d)
        .wrapping_add((z as u32).wrapping_mul(0x_9e37_79b1))
        .wrapping_add(seed);
    h ^= h >> 15;
    h = h.wrapping_mul(0x_85eb_ca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0x_c2b2_ae35);
    h ^= h >> 16;
    (h as f32) / (u32::MAX as f32)
}

/// 二維 value noise：格點雜湊 + smoothstep 雙線性內插 → 平滑 [0,1)。
fn value_noise(x: f32, z: f32, seed: u32) -> f32 {
    let x0 = x.floor() as i32;
    let z0 = z.floor() as i32;
    let fx = x - x0 as f32;
    let fz = z - z0 as f32;
    // smoothstep，讓格點之間平滑（不然會看到方塊狀梯田）。
    let sx = fx * fx * (3.0 - 2.0 * fx);
    let sz = fz * fz * (3.0 - 2.0 * fz);
    let n00 = hash2(x0, z0, seed);
    let n10 = hash2(x0 + 1, z0, seed);
    let n01 = hash2(x0, z0 + 1, seed);
    let n11 = hash2(x0 + 1, z0 + 1, seed);
    let nx0 = n00 + (n10 - n00) * sx;
    let nx1 = n01 + (n11 - n01) * sx;
    nx0 + (nx1 - nx0) * sz
}

// ── 生物群系（生物群系第一刀）────────────────────────────────────────────────

/// 乙太方界生物群系。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoxelBiome {
    /// 草原（現狀）：草地+疏樹，出生地預設。
    Grassland,
    /// 森林：草地+密樹（樹機率 1.5×）。
    Forest,
    /// 沙漠：地表沙、無樹、偶有仙人掌柱。
    Desert,
    /// 雪原（生物群系第二刀）：地表覆雪（Snow 取代草）、疏落的針葉樹（樹密度同草原）。
    /// 白雪地表是一眼認出的寒冷地帶，與草原/森林/沙漠並列四大群系。
    Snow,
}

/// 生物群系場噪聲種子（大尺度、低頻 → 世界有大塊感，與高度 noise 獨立）。
const BIOME_SEED: u32 = SEED ^ 0x_B104_EB14;
/// 生物群系噪聲頻率：比高度 noise 低 4× → 群系邊界平緩（~200 格一個群系）。
const BIOME_SCALE: f32 = 192.0;
/// 出生錨點附近強制草原的半徑（格）。讓出生點永遠有草有樹。
pub const BIOME_SPAWN_RADIUS: i32 = 40;
/// 森林第二棵樹的種子（與第一棵 TREE_SEED 分流，兩棵樹分佈互不相關）。
const TREE2_SEED: u32 = TREE_SEED ^ 0x_4444_4444;
/// 森林格長「第二棵樹」的機率門檻（只有森林群系擲這顆骰；密度 2–3 倍的來源）。
const TREE2_FOREST_CHANCE: f32 = 0.85;
/// 仙人掌噪聲種子（沙漠群系隨機柱，與樹/地形獨立）。
const CACTUS_SEED: u32 = SEED ^ 0x_CA_C7_AC_7E;
/// 仙人掌格邊長（比照樹的格設計：每格至多一株、株位落格內側 →
/// 相鄰兩株至少隔 3 格，永遠是孤立小柱，不會連成綠牆）。
const CACTUS_CELL: i32 = 5;
/// 一格長出仙人掌的機率門檻（per-cell hash < 此值才長）。
/// 約 30% 的沙漠格有一株 → 視覺上「偶有」、稀疏點綴。
const CACTUS_CHANCE: f32 = 0.30;

/// 冰晶噪聲種子（雪原群系隨機結晶，與樹/地形/仙人掌獨立）。
const ICE_SEED: u32 = SEED ^ 0x_1CE_C_A57;
/// 冰晶格邊長（比照仙人掌的格設計：每格至多一株、株位落格內側 →
/// 相鄰兩株至少隔 3 格，永遠是孤立小結晶，不會連成冰牆）。
/// 用比仙人掌更大的格（7）讓冰晶更稀有——它是雪原的珍寶，不該遍地都是。
const ICE_CELL: i32 = 7;
/// 一格長出冰晶的機率門檻（per-cell hash < 此值才長）。
/// 約 18% 的雪原格有一株 → 比仙人掌更稀疏，符合「珍稀寶物」定位。
const ICE_CHANCE: f32 = 0.18;

/// 世界座標 → 生物群系。確定性純函式，同座標永遠同群系。
/// 出生保護圈（BIOME_SPAWN_RADIUS 內）強制草原，確保玩家出生有樹有草。
pub fn biome_at_voxel(wx: i32, wz: i32) -> VoxelBiome {
    // 出生錨點附近強制草原（平方比較，無 sqrt）。
    for (ax, az) in SPAWN_ANCHORS {
        let ddx = wx - ax;
        let ddz = wz - az;
        if ddx * ddx + ddz * ddz <= BIOME_SPAWN_RADIUS * BIOME_SPAWN_RADIUS {
            return VoxelBiome::Grassland;
        }
    }
    let n = value_noise(wx as f32 / BIOME_SCALE, wz as f32 / BIOME_SCALE, BIOME_SEED);
    // 生物群系第二刀：從森林高端切出雪原帶（n > 0.78），沙漠/草原邊界不動、
    // 森林剩 0.65 < n ≤ 0.78（雪原與森林相鄰＝寒林漸變，自然）。
    if n < 0.35 {
        VoxelBiome::Desert
    } else if n > 0.78 {
        VoxelBiome::Snow
    } else if n > 0.65 {
        VoxelBiome::Forest
    } else {
        VoxelBiome::Grassland
    }
}

/// 地表高度（世界方塊 Y）：多 octave value noise 疊加。確定性 → 同 (wx,wz) 永遠同高度。
pub fn height_at(wx: i32, wz: i32) -> i32 {
    let x = wx as f32;
    let z = wz as f32;
    // 大尺度起伏（丘陵/盆地）+ 中尺度細節。把 value_noise(0..1) 平移成「以 0 為中心」
    // (n-0.5) → 高度能高於也能低於 BASE_HEIGHT，才會生出低於海平面的窪地 → 湖泊/海。
    // 頻率/振幅手調到「平緩可走、偶有小丘與水塘」，且峰值不超出本輪垂直 chunk 範圍(y<32)。
    let mut h = 0.0_f32;
    h += (value_noise(x / 48.0, z / 48.0, SEED) - 0.5) * 16.0;
    h += (value_noise(x / 18.0, z / 18.0, SEED ^ 0x_9e37_79b9) - 0.5) * 5.0;
    h += (value_noise(x / 7.0, z / 7.0, SEED ^ 0x_1234_5678) - 0.5) * 2.0;
    BASE_HEIGHT + h.round() as i32
}

/// 某格 (cellx,cellz) 是否長樹；長的話回傳該樹（已驗證地表為草、在保護圈外）。
/// 純函式、確定性（同格永遠同結果）、可測。是「樹是地形一部分」的單一真相來源。
/// 樹機率依群系：森林 0.95 + 第二棵樹 0.85（每格可長兩棵→實測密度約草原 2.5–3 倍）、
/// 草原 0.50（疏，現狀，單棵）、沙漠 0.0（無）。
pub fn tree_in_cell(cellx: i32, cellz: i32) -> Option<Tree> {
    // 以格中心世界座標查群系，決定本格的樹機率門檻。
    let tree_chance = match biome_of_cell(cellx, cellz) {
        VoxelBiome::Forest => 0.95_f32,
        VoxelBiome::Grassland => TREE_CHANCE, // 0.50，現狀不變
        VoxelBiome::Snow => TREE_CHANCE,      // 0.50，雪原疏落針葉（密度同草原、單棵）
        VoxelBiome::Desert => return None,    // 沙漠無樹
    };
    tree_in_cell_seeded(cellx, cellz, TREE_SEED, tree_chance)
}

/// 森林群系的「第二棵樹」：森林每格可長兩棵（密度 2–3 倍的來源），
/// 草原/沙漠無第二棵。與第一棵同柱時略過（避免同柱雙樹幹疊在一起）。
pub fn tree2_in_cell(cellx: i32, cellz: i32) -> Option<Tree> {
    if biome_of_cell(cellx, cellz) != VoxelBiome::Forest {
        return None;
    }
    let t2 = tree_in_cell_seeded(cellx, cellz, TREE2_SEED, TREE2_FOREST_CHANCE)?;
    if let Some(t1) = tree_in_cell(cellx, cellz) {
        if t1.tx == t2.tx && t1.tz == t2.tz {
            return None;
        }
    }
    Some(t2)
}

/// 樹格 (cellx,cellz) 的群系（以格中心世界座標查）。樹的密度/有無由此決定。
#[inline]
fn biome_of_cell(cellx: i32, cellz: i32) -> VoxelBiome {
    biome_at_voxel(cellx * TREE_CELL + TREE_CELL / 2, cellz * TREE_CELL + TREE_CELL / 2)
}

/// 以指定 seed 在某格擲一棵樹（共用邏輯：擲骰、落點、保護圈、草地檢查、樹幹高度）。
/// 純函式、確定性；第一棵樹（TREE_SEED）與森林第二棵（TREE2_SEED）都走這裡。
fn tree_in_cell_seeded(cellx: i32, cellz: i32, seed: u32, chance: f32) -> Option<Tree> {
    // 以格座標 hash 擲骰：是否長樹。
    if hash2(cellx, cellz, seed) >= chance {
        return None;
    }
    // 樹幹落在格內側（offset 1..=5 of 本格 0..=6），確保半徑 1 樹冠不跨格。
    let ox = (1 + (hash2(cellx, cellz, seed ^ 0x_1111_1111) * 5.0) as i32).clamp(1, 5);
    let oz = (1 + (hash2(cellx, cellz, seed ^ 0x_2222_2222) * 5.0) as i32).clamp(1, 5);
    let tx = cellx * TREE_CELL + ox;
    let tz = cellz * TREE_CELL + oz;
    // 出生保護圈：任一錨點半徑內不長樹（免擋出生／歸巢點）。
    for (ax, az) in SPAWN_ANCHORS {
        let (ddx, ddz) = (tx - ax, tz - az);
        if ddx * ddx + ddz * ddz <= SPAWN_PROTECT_RADIUS * SPAWN_PROTECT_RADIUS {
            return None;
        }
    }
    // 只在草地表長樹：地表為草（高於海平面+1 才是草，否則沙）、避開水邊。
    let base_h = height_at(tx, tz);
    if base_h <= SEA_LEVEL + 1 {
        return None;
    }
    // 樹幹高度 TREE_MIN_TRUNK..=TREE_MAX_TRUNK。
    let span = (TREE_MAX_TRUNK - TREE_MIN_TRUNK + 1) as f32;
    let trunk = (TREE_MIN_TRUNK + (hash2(cellx, cellz, seed ^ 0x_3333_3333) * span) as i32)
        .clamp(TREE_MIN_TRUNK, TREE_MAX_TRUNK);
    Some(Tree { tx, tz, base_h, trunk })
}

/// 樹的方塊查詢：某世界座標若落在「所屬格的樹」的樹幹或樹冠上，回 `Wood`/`Leaves`，否則 `None`。
/// 樹冠半徑 1 不跨格 → 只需查 (wx,wz) 自己這格的樹（森林至多兩棵），O(1)。純函式、可測。
fn tree_block_at(wx: i32, wy: i32, wz: i32) -> Option<Block> {
    let (cellx, cellz) = (wx.div_euclid(TREE_CELL), wz.div_euclid(TREE_CELL));
    if let Some(t) = tree_in_cell(cellx, cellz) {
        if let Some(b) = tree_block_hit(&t, wx, wy, wz) {
            return Some(b);
        }
    }
    if let Some(t) = tree2_in_cell(cellx, cellz) {
        if let Some(b) = tree_block_hit(&t, wx, wy, wz) {
            return Some(b);
        }
    }
    None
}

/// 單棵樹的方塊命中判定（樹幹柱 + 兩層 3×3 樹冠環 + 頂蓋十字）。
fn tree_block_hit(tree: &Tree, wx: i32, wy: i32, wz: i32) -> Option<Block> {
    let top = tree.base_h + tree.trunk; // 樹幹最高一格的 y
    let (dx, dz) = (wx - tree.tx, wz - tree.tz);
    // 樹幹：本柱、地表之上到 top。
    if dx == 0 && dz == 0 && wy > tree.base_h && wy <= top {
        return Some(Block::Wood);
    }
    // 樹冠：top-1、top 兩層 3×3 環（不含樹幹柱本身，那兩格是樹幹頂）。
    if (wy == top - 1 || wy == top) && !(dx == 0 && dz == 0) && dx.abs() <= 1 && dz.abs() <= 1 {
        return Some(Block::Leaves);
    }
    // 樹冠頂蓋：top+1 的十字（樹幹正上方 + 四鄰）。
    if wy == top + 1 && (dx == 0 && dz == 0 || dx.abs() + dz.abs() == 1) {
        return Some(Block::Leaves);
    }
    None
}

/// 仙人掌方塊查詢：沙漠群系地表上方 1–2 格偶有「孤立」仙人掌柱，確定性。
/// 比照樹的格設計：世界切成 CACTUS_CELL×CACTUS_CELL 的格、每格至多一株、
/// 株位落在格內側（offset 1..=3 of 0..=4）→ 相鄰兩株必隔 ≥3 格，永不連成綠牆。
fn cactus_block_at(wx: i32, wy: i32, wz: i32) -> Option<Block> {
    let cellx = wx.div_euclid(CACTUS_CELL);
    let cellz = wz.div_euclid(CACTUS_CELL);
    // 以格座標 hash 擲骰：本格是否有一株。
    if hash2(cellx, cellz, CACTUS_SEED) >= CACTUS_CHANCE {
        return None;
    }
    // 株位落在格內側，確保跨格株距 ≥3、本柱查詢 O(1)（只查自己這格）。
    let ox = (1 + (hash2(cellx, cellz, CACTUS_SEED ^ 0x_5555_5555) * 3.0) as i32).clamp(1, 3);
    let oz = (1 + (hash2(cellx, cellz, CACTUS_SEED ^ 0x_6666_6666) * 3.0) as i32).clamp(1, 3);
    let tx = cellx * CACTUS_CELL + ox;
    let tz = cellz * CACTUS_CELL + oz;
    if wx != tx || wz != tz {
        return None;
    }
    // 只在沙漠群系（以株位查，同株永遠同判定）。
    if biome_at_voxel(tx, tz) != VoxelBiome::Desert {
        return None;
    }
    // 非水邊的沙地才長仙人掌（水邊本就是沙，但仙人掌視覺上不搭）。
    let h = height_at(tx, tz);
    if h <= SEA_LEVEL + 1 {
        return None;
    }
    // 仙人掌高 2 格（地表上方第 1、2 格）。
    if wy == h + 1 || wy == h + 2 {
        Some(Block::Cactus)
    } else {
        None
    }
}

/// 雪原群系地表偶有的冰晶簇（雪原冰晶採集 v1）。
/// 比照仙人掌的格設計：世界切成 ICE_CELL×ICE_CELL 的格、每格至多一株、
/// 株位落格內側（offset 1..=3 of 0..=6）→ 相鄰兩株必隔 ≥3 格，永不連成冰牆。
/// 只在雪原、非水邊；高 1 格（地表上方第 1 格）。
fn ice_crystal_block_at(wx: i32, wy: i32, wz: i32) -> Option<Block> {
    let cellx = wx.div_euclid(ICE_CELL);
    let cellz = wz.div_euclid(ICE_CELL);
    // 以格座標 hash 擲骰：本格是否有一株。
    if hash2(cellx, cellz, ICE_SEED) >= ICE_CHANCE {
        return None;
    }
    // 株位落在格內側，確保跨格株距 ≥3、本柱查詢 O(1)（只查自己這格）。
    let ox = (1 + (hash2(cellx, cellz, ICE_SEED ^ 0x_7777_7777) * 3.0) as i32).clamp(1, 3);
    let oz = (1 + (hash2(cellx, cellz, ICE_SEED ^ 0x_8888_8888) * 3.0) as i32).clamp(1, 3);
    let tx = cellx * ICE_CELL + ox;
    let tz = cellz * ICE_CELL + oz;
    if wx != tx || wz != tz {
        return None;
    }
    // 只在雪原群系（以株位查，同株永遠同判定）。
    if biome_at_voxel(tx, tz) != VoxelBiome::Snow {
        return None;
    }
    // 非水邊的雪地才長冰晶（近海平面是沙、視覺上不搭）。
    let h = height_at(tx, tz);
    if h <= SEA_LEVEL + 1 {
        return None;
    }
    // 冰晶高 1 格（地表上方第 1 格）——小巧珍稀，不像仙人掌那樣成柱。
    if wy == h + 1 {
        Some(Block::IceCrystal)
    } else {
        None
    }
}

/// 任一世界座標的方塊（確定性程序生成）。這是「無狀態世界」的核心查詢。
pub fn block_at(wx: i32, wy: i32, wz: i32) -> Block {
    // 地心一律基岩石頭（避免從世界底掉出去；本輪只生成 y>=0 的 chunk）。
    if wy < 0 {
        return Block::Stone;
    }
    let h = height_at(wx, wz);
    if wy > h {
        // 地表之上：海平面（含）以下補水。
        if wy <= SEA_LEVEL {
            return Block::Water;
        }
        // 再看是否為樹（樹幹/樹冠，只填到原本是空氣的格；樹只長在草地、樹塊恆高於海平面）。
        if let Some(tb) = tree_block_at(wx, wy, wz) {
            return tb;
        }
        // 沙漠群系：偶有仙人掌柱（地表上方 1–2 格）。
        if let Some(cb) = cactus_block_at(wx, wy, wz) {
            return cb;
        }
        // 雪原群系：偶有冰晶簇（地表上方第 1 格）。
        if let Some(ib) = ice_crystal_block_at(wx, wy, wz) {
            return ib;
        }
        return Block::Air;
    }
    if wy == h {
        // 地表層：近海平面用沙，否則依群系（沙漠=沙、雪原=雪、其餘=草）。
        if h <= SEA_LEVEL + 1 {
            return Block::Sand;
        }
        match biome_at_voxel(wx, wz) {
            VoxelBiome::Desert => return Block::Sand,
            VoxelBiome::Snow => return Block::Snow,
            _ => return Block::Grass,
        }
    }
    // 地表以下：依群系決定上層材質（沙漠=沙取代泥土；其餘=泥土）。
    if wy >= h - 3 {
        if biome_at_voxel(wx, wz) == VoxelBiome::Desert {
            return Block::Sand;
        }
        return Block::Dirt;
    }
    // 深層礦石（ROADMAP 682 / 乙太礦脈 v1）：距地表足夠深的石頭層有機率含礦。
    // 三種礦石使用不同 seed，分佈互不重疊；查詢順序 煤→鐵→乙太，愈深愈稀有。
    if wy <= COAL_ORE_DEPTH && ore_hash3(wx, wy, wz, 0xdead_beef) < COAL_ORE_DENSITY {
        return Block::CoalOre;
    }
    if wy <= IRON_ORE_DEPTH && ore_hash3(wx, wy, wz, 0xcafe_1234) < IRON_ORE_DENSITY {
        return Block::IronOre;
    }
    // 乙太礦：只在最深的地心層（y ≤ 0）、最稀有；一路挖到世界底才可能遇上這脈青藍寶礦。
    if wy <= AETHER_ORE_DEPTH && ore_hash3(wx, wy, wz, 0xae74_0e55) < AETHER_ORE_DENSITY {
        return Block::AetherOre;
    }
    Block::Stone
}

/// 生成一整個 chunk（就地用 `block_at` 填）。供需要實體 chunk 的場合（如測試）。
pub fn generate_chunk(coord: ChunkCoord) -> Chunk {
    let mut blocks = vec![0u8; CHUNK_VOL];
    let base_x = coord.cx * CHUNK;
    let base_y = coord.cy * CHUNK;
    let base_z = coord.cz * CHUNK;
    for ly in 0..CHUNK {
        for lz in 0..CHUNK {
            for lx in 0..CHUNK {
                let b = block_at(base_x + lx, base_y + ly, base_z + lz);
                blocks[local_index(lx, ly, lz)] = b as u8;
            }
        }
    }
    Chunk { blocks }
}

/// 把一個 chunk 壓成精簡 base64（4096 bytes → ~5.5KB 字串）供 WS 串流。
/// 全空氣的 chunk 回 `None`——呼叫端據此不傳（高空 chunk 幾乎都被略過，省大量頻寬）。
pub fn pack_chunk(coord: ChunkCoord) -> Option<String> {
    pack_chunk_with_delta(coord, None)
}

/// 同 `pack_chunk`，但先套用 delta overlay（玩家/AI 改過的方塊）再打包。
/// 全空氣（含 delta 後）仍回 `None`。late-join 的玩家靠這個拿到別人改過的世界。
pub fn pack_chunk_with_delta(coord: ChunkCoord, delta: Option<&ChunkDelta>) -> Option<String> {
    let chunk = generate_chunk_with_delta(coord, delta);
    if chunk.blocks.iter().all(|&b| b == Block::Air as u8) {
        return None;
    }
    Some(base64::engine::general_purpose::STANDARD.encode(&chunk.blocks))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 找一個「地表夠高且上方數格皆空氣（無樹幹/樹冠）」的乾淨陸地柱，
    /// 給需要純地表（不被程序生成的樹干擾）的舊地形/碰撞測試用。
    fn clear_land_column(z: i32, min_above_sea: i32) -> i32 {
        for cand in 0..20000 {
            let h = height_at(cand, z);
            if h > SEA_LEVEL + min_above_sea && (1..=4).all(|d| block_at(cand, h + d, z) == Block::Air)
            {
                return cand;
            }
        }
        0
    }

    #[test]
    fn local_index_roundtrip_is_unique() {
        // 每個 (lx,ly,lz) 應映到唯一索引，且覆蓋滿 0..CHUNK_VOL。
        let mut seen = vec![false; CHUNK_VOL];
        for ly in 0..CHUNK {
            for lz in 0..CHUNK {
                for lx in 0..CHUNK {
                    let i = local_index(lx, ly, lz);
                    assert!(i < CHUNK_VOL);
                    assert!(!seen[i], "索引重複 @ {lx},{ly},{lz}");
                    seen[i] = true;
                }
            }
        }
        assert!(seen.iter().all(|&s| s), "有索引沒被覆蓋到");
    }

    #[test]
    fn chunk_of_handles_negatives() {
        // 正常正座標。
        assert_eq!(chunk_of(0, 0, 0), ChunkCoord { cx: 0, cy: 0, cz: 0 });
        assert_eq!(chunk_of(15, 15, 15), ChunkCoord { cx: 0, cy: 0, cz: 0 });
        assert_eq!(chunk_of(16, 16, 16), ChunkCoord { cx: 1, cy: 1, cz: 1 });
        // 負座標必須 floor 到 -1，不能落回 0（不然會錯位）。
        assert_eq!(chunk_of(-1, -1, -1), ChunkCoord { cx: -1, cy: -1, cz: -1 });
        assert_eq!(chunk_of(-16, -16, -16), ChunkCoord { cx: -1, cy: -1, cz: -1 });
        assert_eq!(chunk_of(-17, 0, 0).cx, -2);
    }

    #[test]
    fn terrain_is_deterministic() {
        // 同座標多次查詢必須完全一致（多人/前後端一致的根本保證）。
        for &(x, z) in &[(0, 0), (100, -50), (-12345, 6789), (5, 5)] {
            let h1 = height_at(x, z);
            let h2 = height_at(x, z);
            assert_eq!(h1, h2);
            let b1 = block_at(x, h1, z);
            let b2 = block_at(x, h1, z);
            assert_eq!(b1, b2);
        }
    }

    #[test]
    fn surface_and_layers_make_sense() {
        // 取一個乾淨陸地點（高於海平面、上方無樹）。
        let z = 0;
        let x = clear_land_column(z, 2);
        let h = height_at(x, z);
        assert!(h > SEA_LEVEL + 1, "測試點應在海平面之上的陸地");
        // 地表是草、其下是土、再下是石、其上是空氣。
        assert_eq!(block_at(x, h, z), Block::Grass);
        assert_eq!(block_at(x, h - 1, z), Block::Dirt);
        // 深層石層可能含礦石（ROADMAP 682 煤/鐵；乙太礦脈 v1 乙太礦只在 y≤0 最深層）。
        assert!(matches!(
            block_at(x, h - 8, z),
            Block::Stone | Block::CoalOre | Block::IronOre | Block::AetherOre
        ));
        assert_eq!(block_at(x, h + 1, z), Block::Air);
        // 負 y 座標一律石頭（ROADMAP 682 礦石只在 y≥0 層生成）。
        assert_eq!(block_at(x, -5, z), Block::Stone);
    }

    #[test]
    fn sea_fills_with_water() {
        // 找一個地表低於海平面的點（窪地）→ 海平面以下的空格應是水。
        let mut found = false;
        for cand in 0..5000 {
            if height_at(cand, 17) < SEA_LEVEL {
                let h = height_at(cand, 17);
                // 地表之上、海平面之下 → 水。
                assert_eq!(block_at(cand, h + 1, 17), Block::Water);
                found = true;
                break;
            }
        }
        assert!(found, "5000 格內應找得到一個低於海平面的窪地");
    }

    #[test]
    fn is_solid_matrix() {
        assert!(!Block::Air.is_solid());
        assert!(!Block::Water.is_solid());
        assert!(Block::Grass.is_solid());
        assert!(Block::Stone.is_solid());
        assert!(Block::Wood.is_solid());
    }

    // ── 乙太礦脈 v1（乙太礦/乙太燈）測試 ──────────────────────────────────────────
    #[test]
    fn aether_blocks_roundtrip_and_flags() {
        // id ↔ enum 雙向對應。
        assert_eq!(Block::from_u8(58), Some(Block::AetherOre));
        assert_eq!(Block::from_u8(59), Some(Block::AetherLamp));
        assert_eq!(Block::AetherOre as u8, 58);
        assert_eq!(Block::AetherLamp as u8, 59);
        // 兩者皆實心、可放置（採集回收＋合成產物都能擺進世界）。
        assert!(Block::AetherOre.is_solid());
        assert!(Block::AetherLamp.is_solid());
        assert!(Block::AetherOre.is_placeable());
        assert!(Block::AetherLamp.is_placeable());
    }

    #[test]
    fn aether_ore_is_rarest_and_deepest() {
        // 常數關係：乙太礦比鐵礦更深（層數更低）、比鐵礦更稀（機率更小）。
        assert!(AETHER_ORE_DEPTH < IRON_ORE_DEPTH, "乙太礦應比鐵礦更深");
        assert!(IRON_ORE_DEPTH < COAL_ORE_DEPTH, "鐵礦應比煤礦更深");
        assert!(AETHER_ORE_DENSITY < IRON_ORE_DENSITY, "乙太礦應比鐵礦更稀有");
        assert!(IRON_ORE_DENSITY < COAL_ORE_DENSITY, "鐵礦應比煤礦更稀有");
    }

    #[test]
    fn aether_ore_only_in_deepest_layer() {
        // 掃描一片區域，凡是乙太礦者，其 y 必 ≤ AETHER_ORE_DEPTH（絕不出現在較淺的石層）。
        for x in 0..80 {
            for z in 0..80 {
                let h = height_at(x, z);
                for y in 1..h {
                    if block_at(x, y, z) == Block::AetherOre {
                        assert!(
                            y <= AETHER_ORE_DEPTH,
                            "乙太礦不該出現在 y={y}（> AETHER_ORE_DEPTH={AETHER_ORE_DEPTH}）"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn aether_ore_actually_generates() {
        // 掃描夠大的最深層（y=0），應找得到至少一脈乙太礦（證明生成邏輯有效、非死碼）。
        let mut found = false;
        'outer: for x in -150..150 {
            for z in -150..150 {
                // 只在 y=0 落在石層（h-3 > 0）的柱子上查，避開淺地表把 y=0 當泥土。
                if height_at(x, z) > 3 && block_at(x, 0, z) == Block::AetherOre {
                    found = true;
                    break 'outer;
                }
            }
        }
        assert!(found, "300×300 的最深層應找得到至少一格乙太礦");
    }

    #[test]
    fn flowing_water_is_non_solid_water() {
        // 流動水（24..=30）：非實心、算「任何水」、但不算「來源水」（來源仍只有 Water）。
        for b in [
            Block::WaterFlow1, Block::WaterFlow2, Block::WaterFlow3, Block::WaterFlow4,
            Block::WaterFlow5, Block::WaterFlow6, Block::WaterFlow7,
        ] {
            assert!(!b.is_solid(), "流動水應非實心：{b:?}");
            assert!(b.is_flowing_water(), "應被判為流動水：{b:?}");
            assert!(b.is_any_water(), "流動水應算任何水：{b:?}");
            assert!(!b.is_placeable(), "玩家不可手動放置流動水：{b:?}");
        }
        // 來源水：是任何水、但不是流動水。
        assert!(Block::Water.is_any_water());
        assert!(!Block::Water.is_flowing_water());
        // 實心方塊都不是水。
        assert!(!Block::Stone.is_any_water());
        assert!(!Block::Stone.is_flowing_water());
    }

    #[test]
    fn flowing_water_from_u8_roundtrips() {
        // id 24..=30 ↔ WaterFlow1..=7 往返正確，且與既有方塊 id 不衝突。
        let pairs = [
            (24u8, Block::WaterFlow1), (25, Block::WaterFlow2), (26, Block::WaterFlow3),
            (27, Block::WaterFlow4), (28, Block::WaterFlow5), (29, Block::WaterFlow6),
            (30, Block::WaterFlow7),
        ];
        for (id, b) in pairs {
            assert_eq!(Block::from_u8(id), Some(b));
            assert_eq!(b as u8, id);
        }
        // 31 = Torch（ROADMAP 685），32 以後是未使用 id。
        assert_eq!(Block::from_u8(31), Some(Block::Torch));
        assert_eq!(Block::from_u8(32), None);
    }

    #[test]
    fn can_place_on_flowing_water() {
        // 站乾淨陸地，玩家可把建材放進「流動水格」（同來源水，水可被建材覆蓋）。
        let z = 0;
        let x = clear_land_column(z, 3);
        let h = height_at(x, z);
        let mut world: WorldDelta = WorldDelta::new();
        // 在腳邊空氣格放一格流動水（模擬水流過來），驗證仍可在其上放石頭。
        set_block(&mut world, x, h + 1, z, Block::WaterFlow3);
        let (px, py, pz) = (x as f32 + 0.5, (h + 1) as f32, z as f32 + 0.5);
        assert!(
            can_place(&world, px, py, pz, x, h + 1, z, Block::Stone),
            "應可在流動水格放建材"
        );
        // 但流動水本身仍不可被破壞（非實心）。
        assert!(!can_break(&world, px, py, pz, x, h + 1, z));
    }

    #[test]
    fn pack_chunk_skips_all_air_but_packs_ground() {
        // 高空 chunk（cy 很大）應全空氣 → None。
        let high = pack_chunk(ChunkCoord { cx: 0, cy: 20, cz: 0 });
        assert!(high.is_none(), "高空 chunk 該被略過");
        // 地面 chunk（cy=0）一定有方塊 → Some，且解碼後長度正確。
        let ground = pack_chunk(ChunkCoord { cx: 0, cy: 0, cz: 0 });
        let s = ground.expect("地面 chunk 應有方塊");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(s)
            .expect("應為合法 base64");
        assert_eq!(bytes.len(), CHUNK_VOL);
    }

    #[test]
    fn world_local_index_handles_negatives() {
        // 正座標：等同直接 local_index。
        assert_eq!(world_local_index(0, 0, 0), local_index(0, 0, 0));
        assert_eq!(world_local_index(15, 15, 15), local_index(15, 15, 15));
        // chunk 邊界：wx=16 落到下一 chunk 的 lx=0。
        assert_eq!(world_local_index(16, 0, 0), local_index(0, 0, 0));
        // 負座標：-1 應在 chunk -1 的 lx=15（euclid 取餘）。
        assert_eq!(world_local_index(-1, -1, -1), local_index(15, 15, 15));
    }

    #[test]
    fn delta_overlay_overrides_terrain() {
        // 找一個地表草點。
        let (mut x, mut z) = (0, 0);
        for cand in 0..2000 {
            if height_at(cand, 0) > SEA_LEVEL + 2 {
                x = cand;
                z = 0;
                break;
            }
        }
        let h = height_at(x, z);
        assert_eq!(block_at(x, h, z), Block::Grass);

        let mut world: WorldDelta = WorldDelta::new();
        // 挖掉地表（覆蓋成空氣）。
        set_block(&mut world, x, h, z, Block::Air);
        assert_eq!(effective_block_at(&world, x, h, z), Block::Air);
        // 放一塊石頭在地表上方原本是空氣的格。
        set_block(&mut world, x, h + 1, z, Block::Stone);
        assert_eq!(effective_block_at(&world, x, h + 1, z), Block::Stone);
        // 沒被改的鄰格仍是程序生成值。
        assert_eq!(effective_block_at(&world, x + 1, h, z), block_at(x + 1, h, z));

        // pack 出來的 chunk 必須含 delta（解碼後該局部索引是被改的值）。
        let coord = chunk_of(x, h, z);
        let cd = world.get(&coord).unwrap();
        let s = pack_chunk_with_delta(coord, Some(cd)).expect("有方塊");
        let bytes = base64::engine::general_purpose::STANDARD.decode(s).unwrap();
        assert_eq!(bytes[world_local_index(x, h, z)], Block::Air as u8);
    }

    #[test]
    fn break_and_place_validation() {
        // 站在乾淨地表草點上方一點（上方無樹，免樹塊干擾放置/破壞判定）。
        let z = 0;
        let x = clear_land_column(z, 3);
        let h = height_at(x, z);
        let world: WorldDelta = WorldDelta::new();
        // 玩家腳底站在 h+1（地表方塊頂面），眼睛 h+1+EYE。
        let (px, py, pz) = (x as f32 + 0.5, (h + 1) as f32, z as f32 + 0.5);

        // 可破壞腳下的地表草方塊（近、實心）。
        assert!(can_break(&world, px, py, pz, x, h, z));
        // 不能破壞空氣（地表上方第三格）。
        assert!(!can_break(&world, px, py, pz, x, h + 3, z));
        // 太遠不能破壞。
        assert!(!can_break(&world, px, py, pz, x + 50, h, z));
        // 地心不給挖。
        assert!(!can_break(&world, px, py, pz, x, -1, z));

        // 可在腳邊空氣格放石頭。
        assert!(can_place(&world, px, py, pz, x, h + 1, z, Block::Stone));
        // 不能放在既有實心方塊上（會覆蓋）。
        assert!(!can_place(&world, px, py, pz, x, h, z, Block::Stone));
        // 不能放空氣（那是挖、不是放）。
        assert!(!can_place(&world, px, py, pz, x, h + 1, z, Block::Air));
        // 不能放水。
        assert!(!can_place(&world, px, py, pz, x, h + 1, z, Block::Water));
        // 太遠不能放。
        assert!(!can_place(&world, px, py, pz, x, h + 50, z, Block::Stone));
    }

    #[test]
    fn door_open_can_be_broken() {
        // DoorOpen 雖非實心，can_break 仍允許破壞（退還木門關）。
        let z = 0;
        let x = clear_land_column(z, 5);
        let h = height_at(x, z);
        let mut world = WorldDelta::new();
        // 玩家站在地表上方，設 DoorOpen 於腳邊。
        let (px, py, pz) = (x as f32 + 0.5, (h + 1) as f32, z as f32 + 0.5);
        set_block(&mut world, x, h + 1, z, Block::DoorOpen);
        assert!(can_break(&world, px, py, pz, x, h + 1, z), "開門可破壞");
        // DoorClosed 依舊可破壞（實心）。
        set_block(&mut world, x, h + 1, z, Block::DoorClosed);
        assert!(can_break(&world, px, py, pz, x, h + 1, z), "關門可破壞");
        // 空氣仍不可破壞。
        set_block(&mut world, x, h + 1, z, Block::Air);
        assert!(!can_break(&world, px, py, pz, x, h + 1, z), "空氣不可破壞");
    }

    #[test]
    fn bed_is_solid_placeable_and_roundtrips() {
        // 床（id 45）：實心（可站立/會擋路）、可放置、from_u8 正確還原。
        assert!(Block::Bed.is_solid(), "床應為實心方塊");
        assert!(Block::Bed.is_placeable(), "床應可放置");
        assert_eq!(Block::from_u8(45), Some(Block::Bed));
    }

    #[test]
    fn carrot_states_are_solid_but_not_player_placeable() {
        // 胡蘿蔔幼苗/成熟胡蘿蔔（id 46/47）：比照 FarmSoilSeeded/WheatMature，
        // 實心（可站立）但伺服器維護狀態，玩家不能手動放置。
        assert!(Block::CarrotSeeded.is_solid(), "胡蘿蔔幼苗應為實心");
        assert!(Block::CarrotMature.is_solid(), "成熟胡蘿蔔應為實心");
        assert!(!Block::CarrotSeeded.is_placeable(), "胡蘿蔔幼苗不可手動放置");
        assert!(!Block::CarrotMature.is_placeable(), "成熟胡蘿蔔不可手動放置");
        assert_eq!(Block::from_u8(46), Some(Block::CarrotSeeded));
        assert_eq!(Block::from_u8(47), Some(Block::CarrotMature));
    }

    #[test]
    fn block_from_u8_roundtrips() {
        for b in [
            Block::Air,
            Block::Grass,
            Block::Dirt,
            Block::Stone,
            Block::Sand,
            Block::Wood,
            Block::Leaves,
            Block::Water,
        ] {
            assert_eq!(Block::from_u8(b as u8), Some(b));
        }
        assert_eq!(Block::from_u8(99), None);
    }

    #[test]
    fn generate_chunk_matches_block_at() {
        // generate_chunk 必須與逐點 block_at 完全一致（串流與查詢同源）。
        let coord = ChunkCoord { cx: 3, cy: 0, cz: -2 };
        let chunk = generate_chunk(coord);
        for ly in 0..CHUNK {
            for lz in 0..CHUNK {
                for lx in 0..CHUNK {
                    let expect = block_at(
                        coord.cx * CHUNK + lx,
                        coord.cy * CHUNK + ly,
                        coord.cz * CHUNK + lz,
                    ) as u8;
                    assert_eq!(chunk.blocks[local_index(lx, ly, lz)], expect);
                }
            }
        }
    }

    // ── 程序生成樹 ──────────────────────────────────────────────────────────────

    #[test]
    fn trees_are_deterministic() {
        // 同格／同座標多次查詢完全一致（多人/前後端一致的根本）。
        for &(cx, cz) in &[(0, 0), (3, -2), (-17, 40), (123, -456)] {
            assert_eq!(tree_in_cell(cx, cz), tree_in_cell(cx, cz));
        }
        for &(x, y, z) in &[(20, 12, 5), (-30, 14, 7), (100, 13, -100)] {
            assert_eq!(block_at(x, y, z), block_at(x, y, z));
        }
    }

    #[test]
    fn some_trees_exist_with_wood_and_leaves() {
        // 掃一大片地表上方空間，必須真的長出 Wood（樹幹）與 Leaves（樹冠）。
        let (mut wood, mut leaves) = (0u32, 0u32);
        for x in -60..60 {
            for z in -60..60 {
                let h = height_at(x, z);
                for dy in 1..=8 {
                    match block_at(x, h + dy, z) {
                        Block::Wood => wood += 1,
                        Block::Leaves => leaves += 1,
                        _ => {}
                    }
                }
            }
        }
        assert!(wood > 0, "地表應長出樹幹 Wood：wood={wood}");
        assert!(leaves > 0, "地表應長出樹冠 Leaves：leaves={leaves}");
    }

    #[test]
    fn tree_density_is_reasonable() {
        // 密度合理：一大片地表上的樹數既不為 0（找得到）也不過密（別擋路）。
        // ── 注意：密度現在依群系而異（森林 0.95 > 草原 0.50 > 沙漠 0.0）；
        // 整體統計仍落在合理區間（掃 -20..20 格，混合群系，實際約 30..1200 棵）。
        let mut trees = 0u32;
        let (lo, hi) = (-20, 20); // 41×41 格 = 1681 柱
        for cx in lo..hi {
            for cz in lo..hi {
                if tree_in_cell(cx, cz).is_some() {
                    trees += 1;
                }
            }
        }
        // 寬鬆區間：草原 + 森林格都有樹，沙漠格一棵沒有，整體合理。
        assert!(trees > 30, "樹太稀疏（找不到木頭）：trees={trees}");
        assert!(trees < 1200, "樹太密集（擋路）：trees={trees}");
    }

    #[test]
    fn tree_grows_from_grass_trunk_then_leaves() {
        // 找一棵真實的樹，驗證：底座是草、其上是樹幹 Wood、樹冠含 Leaves。
        let tree = (0..200)
            .flat_map(|cx| (0..200).map(move |cz| (cx, cz)))
            .find_map(|(cx, cz)| tree_in_cell(cx, cz))
            .expect("應找得到一棵樹");
        // 樹底座踩在草地表上。
        assert_eq!(block_at(tree.tx, tree.base_h, tree.tz), Block::Grass);
        // 樹幹：底座之上整段都是 Wood。
        for y in (tree.base_h + 1)..=(tree.base_h + tree.trunk) {
            assert_eq!(block_at(tree.tx, y, tree.tz), Block::Wood, "樹幹應為 Wood @ y={y}");
        }
        // 樹幹高度在設定範圍內。
        assert!((TREE_MIN_TRUNK..=TREE_MAX_TRUNK).contains(&tree.trunk));
        // 樹冠：樹幹頂的 3×3 環上至少有 Leaves。
        let top = tree.base_h + tree.trunk;
        let mut canopy = 0u32;
        for dx in -1..=1 {
            for dz in -1..=1 {
                if block_at(tree.tx + dx, top, tree.tz + dz) == Block::Leaves {
                    canopy += 1;
                }
            }
        }
        assert!(canopy > 0, "樹幹頂應有樹冠 Leaves");
        // 樹幹正上方頂蓋是 Leaves。
        assert_eq!(block_at(tree.tx, top + 1, tree.tz), Block::Leaves);
    }

    #[test]
    fn tree_canopy_never_crosses_cell() {
        // 設計不變量：半徑 1 樹冠永不跨格 → block_at 只查自己這格即可（O(1)）。
        // 驗證每棵樹（含森林第二棵）的樹幹落點都在格內側（offset 1..=5 of 0..=6）。
        for cx in -30..30 {
            for cz in -30..30 {
                for t in [tree_in_cell(cx, cz), tree2_in_cell(cx, cz)].into_iter().flatten() {
                    let ox = t.tx - cx * TREE_CELL;
                    let oz = t.tz - cz * TREE_CELL;
                    assert!((1..=5).contains(&ox) && (1..=5).contains(&oz),
                        "樹幹落點應在格內側：off=({ox},{oz})");
                }
            }
        }
    }

    #[test]
    fn no_trees_in_spawn_protection_core() {
        // 出生保護圈核心（錨點半徑 R-2 內，足夠涵蓋出生點+落地餘裕）不得有任何樹塊。
        let core = SPAWN_PROTECT_RADIUS - 2;
        for (ax, az) in SPAWN_ANCHORS {
            for dx in -core..=core {
                for dz in -core..=core {
                    if dx * dx + dz * dz > core * core {
                        continue;
                    }
                    let (x, z) = (ax + dx, az + dz);
                    let h = height_at(x, z);
                    for dy in 1..=8 {
                        let b = block_at(x, h + dy, z);
                        assert!(
                            b != Block::Wood && b != Block::Leaves,
                            "保護圈核心不該有樹 @ ({x},{z},{})",
                            h + dy
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn trees_persist_through_generate_chunk() {
        // 樹是「地形的一部分」：generate_chunk（串流路徑）必含樹塊、且與逐點 block_at 一致。
        // 找一棵樹，定位其所屬地面 chunk，驗證 packed chunk 裡含 Wood。
        let tree = (0..200)
            .flat_map(|cx| (0..200).map(move |cz| (cx, cz)))
            .find_map(|(cx, cz)| tree_in_cell(cx, cz))
            .expect("應找得到一棵樹");
        let coord = chunk_of(tree.tx, tree.base_h + 1, tree.tz);
        let chunk = generate_chunk(coord);
        assert!(
            chunk.blocks.iter().any(|&b| b == Block::Wood as u8),
            "樹所屬 chunk 應含 Wood（走 generate 路徑、非 delta）"
        );
        // generate 與逐點 block_at 完全同源（含樹）。
        let li = world_local_index(tree.tx, tree.base_h + 1, tree.tz);
        assert_eq!(chunk.blocks[li], Block::Wood as u8);
    }

    // ── 深層礦石（ROADMAP 682）────────────────────────────────────────────────

    #[test]
    fn ore_hash3_is_deterministic() {
        // 同座標多次呼叫必須完全一致。
        for &(x, y, z, seed) in &[
            (0, 0, 0, 0u32),
            (100, 3, -50, 0xdead_beef),
            (-12345, 1, 6789, 0xcafe_1234),
        ] {
            let a = ore_hash3(x, y, z, seed);
            let b = ore_hash3(x, y, z, seed);
            assert_eq!(a, b, "ore_hash3 應為確定性");
        }
    }

    #[test]
    fn ore_hash3_is_in_range() {
        // 回傳值必須在 [0, 1)。
        for &(x, y, z) in &[(0, 0, 0), (100, 3, -50), (-999, 1, 42)] {
            let v = ore_hash3(x, y, z, 0xdead_beef);
            assert!(v >= 0.0 && v < 1.0, "ore_hash3 應在 [0,1)：{v}");
        }
    }

    #[test]
    fn ore_generation_exists_at_valid_depths() {
        // 掃一個足夠大的區域，確認煤礦和鐵礦在合法深度真的有生成。
        let (mut coal, mut iron) = (0u32, 0u32);
        for x in -100..100 {
            for z in -100..100 {
                for y in 0..=COAL_ORE_DEPTH {
                    let b = block_at(x, y, z);
                    if b == Block::CoalOre { coal += 1; }
                    if b == Block::IronOre { iron += 1; }
                }
            }
        }
        assert!(coal > 0, "應能在地底找到煤礦：coal={coal}");
        assert!(iron > 0, "應能在地底找到鐵礦：iron={iron}");
    }

    #[test]
    fn ore_not_generated_above_depth_limit() {
        // 超過深度限制的石層不生成礦石。
        let z = 0;
        let x = clear_land_column(z, 2);
        let h = height_at(x, z);
        // 選一個高於 COAL_ORE_DEPTH 的石層深度。
        let mid_stone = h - 5;
        if mid_stone > COAL_ORE_DEPTH {
            let b = block_at(x, mid_stone, z);
            assert!(
                !matches!(b, Block::CoalOre | Block::IronOre),
                "深度 {mid_stone} > COAL_ORE_DEPTH={COAL_ORE_DEPTH}，不應生成礦石：{b:?}"
            );
        }
    }

    #[test]
    fn ore_blocks_are_solid_and_placeable() {
        assert!(Block::CoalOre.is_solid());
        assert!(Block::IronOre.is_solid());
        assert!(Block::CoalOre.is_placeable());
        assert!(Block::IronOre.is_placeable());
    }

    #[test]
    fn ore_from_u8_roundtrips() {
        assert_eq!(Block::from_u8(20), Some(Block::CoalOre));
        assert_eq!(Block::from_u8(21), Some(Block::IronOre));
    }

    #[test]
    fn iron_block_is_solid_and_placeable() {
        // 鐵磚（ROADMAP 684）：實心、可放置、from_u8 往返正確。
        assert!(Block::IronBlock.is_solid());
        assert!(Block::IronBlock.is_placeable());
        assert_eq!(Block::from_u8(23), Some(Block::IronBlock));
        assert_eq!(Block::IronBlock as u8, 23);
    }

    #[test]
    fn negative_y_always_stone_not_ore() {
        // y < 0 的早期返回確保礦石不在世界底部生成。
        for x in &[-5, 0, 100] {
            for z in &[-5, 0, 100] {
                for y in &[-1, -5, -100] {
                    assert_eq!(
                        block_at(*x, *y, *z), Block::Stone,
                        "y={y} 應永遠是 Stone（非礦石）"
                    );
                }
            }
        }
    }

    // ── 生物群系（生物群系第一刀）────────────────────────────────────────────────

    #[test]
    fn biome_at_voxel_is_deterministic() {
        // 同座標永遠同結果。
        for &(x, z) in &[(0, 0), (100, -50), (-999, 777), (500, 500)] {
            assert_eq!(biome_at_voxel(x, z), biome_at_voxel(x, z));
        }
    }

    #[test]
    fn four_biomes_exist_in_world() {
        // 掃一片世界，四種群系都必須出現（生物群系第二刀：新增雪原）。
        let mut has_grassland = false;
        let mut has_forest = false;
        let mut has_desert = false;
        let mut has_snow = false;
        // 雪原在 noise 高端（>0.78）較稀，掃大一點的範圍確保出現。
        for x in (-1500..=1500i32).step_by(20) {
            for z in (-1500..=1500i32).step_by(20) {
                match biome_at_voxel(x, z) {
                    VoxelBiome::Grassland => has_grassland = true,
                    VoxelBiome::Forest => has_forest = true,
                    VoxelBiome::Desert => has_desert = true,
                    VoxelBiome::Snow => has_snow = true,
                }
            }
        }
        assert!(has_grassland, "應找到草原群系");
        assert!(has_forest, "應找到森林群系");
        assert!(has_desert, "應找到沙漠群系");
        assert!(has_snow, "應找到雪原群系");
    }

    #[test]
    fn spawn_area_is_always_grassland() {
        // 出生點附近（半徑 BIOME_SPAWN_RADIUS 內核心點）一律草原。
        for (ax, az) in SPAWN_ANCHORS {
            for dx in -5..=5i32 {
                for dz in -5..=5i32 {
                    assert_eq!(
                        biome_at_voxel(ax + dx * 3, az + dz * 3),
                        VoxelBiome::Grassland,
                        "出生保護圈應為草原 @ ({},{})", ax + dx * 3, az + dz * 3
                    );
                }
            }
        }
    }

    #[test]
    fn desert_surface_is_sand_no_trees() {
        // 沙漠群系地表為沙、無木塊（沙漠無樹）。
        let mut found = false;
        'outer: for x in -500..500i32 {
            for z in -500..500i32 {
                if biome_at_voxel(x, z) == VoxelBiome::Desert {
                    let h = height_at(x, z);
                    if h > SEA_LEVEL + 1 {
                        assert_eq!(
                            block_at(x, h, z), Block::Sand,
                            "沙漠地表應為沙 @ ({},{})", x, z
                        );
                        // 沙漠地表上方多格不應有木塊。
                        for dy in 1..=6 {
                            let b = block_at(x, h + dy, z);
                            assert!(
                                b != Block::Wood && b != Block::Leaves,
                                "沙漠不應有樹 @ ({},{},{})", x, h + dy, z
                            );
                        }
                        found = true;
                        break 'outer;
                    }
                }
            }
        }
        assert!(found, "應找到沙漠群系的陸地格");
    }

    #[test]
    fn forest_has_higher_tree_density_than_grassland() {
        // 森林群系的樹密度應為草原的 2 倍以上（規格：2–3 倍）。
        // 計「棵數」（含森林第二棵樹），不是「有樹的格數」——密度倍率才量得準。
        let mut forest_trees = 0u32;
        let mut forest_total = 0u32;
        let mut grassland_trees = 0u32;
        let mut grassland_total = 0u32;
        for cx in -80..80i32 {
            for cz in -80..80i32 {
                // 格中心座標查群系，與 tree_in_cell 內部一致。
                let wx = cx * TREE_CELL + TREE_CELL / 2;
                let wz = cz * TREE_CELL + TREE_CELL / 2;
                let biome = biome_at_voxel(wx, wz);
                let n_trees = tree_in_cell(cx, cz).is_some() as u32
                    + tree2_in_cell(cx, cz).is_some() as u32;
                match biome {
                    VoxelBiome::Forest => {
                        forest_total += 1;
                        forest_trees += n_trees;
                    }
                    VoxelBiome::Grassland => {
                        grassland_total += 1;
                        grassland_trees += n_trees;
                    }
                    VoxelBiome::Desert | VoxelBiome::Snow => {}
                }
            }
        }
        if forest_total > 10 && grassland_total > 10 {
            let forest_ratio = forest_trees as f32 / forest_total as f32;
            let grassland_ratio = grassland_trees as f32 / grassland_total as f32;
            assert!(
                forest_ratio >= grassland_ratio * 2.0,
                "森林樹密度({forest_ratio:.3} 棵/格)應為草原({grassland_ratio:.3} 棵/格)的 2 倍以上"
            );
        }
    }

    #[test]
    fn tree2_only_in_forest_and_deterministic() {
        // 森林第二棵樹：只在森林群系出現、確定性、且不與第一棵同柱。
        let mut found_tree2 = false;
        for cx in -80..80i32 {
            for cz in -80..80i32 {
                let t2 = tree2_in_cell(cx, cz);
                assert_eq!(t2, tree2_in_cell(cx, cz), "tree2 應為確定性 @ cell({cx},{cz})");
                if let Some(t2) = t2 {
                    let center_x = cx * TREE_CELL + TREE_CELL / 2;
                    let center_z = cz * TREE_CELL + TREE_CELL / 2;
                    assert_eq!(
                        biome_at_voxel(center_x, center_z),
                        VoxelBiome::Forest,
                        "第二棵樹只該出現在森林 @ cell({cx},{cz})"
                    );
                    if let Some(t1) = tree_in_cell(cx, cz) {
                        assert!(
                            (t1.tx, t1.tz) != (t2.tx, t2.tz),
                            "兩棵樹不得同柱 @ cell({cx},{cz})"
                        );
                    }
                    found_tree2 = true;
                }
            }
        }
        assert!(found_tree2, "掃描範圍內應找得到森林第二棵樹");
    }

    #[test]
    fn desert_has_no_tree_cells() {
        // 沙漠群系的格不長樹（tree_in_cell / tree2_in_cell 永遠 None）。
        // 用格中心座標查群系，與 tree_in_cell 內部邏輯一致（避免邊界對不齊的誤判）。
        for cx in -80..80i32 {
            for cz in -80..80i32 {
                let center_x = cx * TREE_CELL + TREE_CELL / 2;
                let center_z = cz * TREE_CELL + TREE_CELL / 2;
                if biome_at_voxel(center_x, center_z) == VoxelBiome::Desert {
                    assert!(
                        tree_in_cell(cx, cz).is_none(),
                        "沙漠格不應長樹 @ cell({},{})", cx, cz
                    );
                    assert!(
                        tree2_in_cell(cx, cz).is_none(),
                        "沙漠格不應長第二棵樹 @ cell({},{})", cx, cz
                    );
                }
            }
        }
    }

    #[test]
    fn desert_has_cacti() {
        // 沙漠群系中應能找到仙人掌方塊。
        let mut found = false;
        'outer: for x in -500..500i32 {
            for z in -500..500i32 {
                if biome_at_voxel(x, z) == VoxelBiome::Desert {
                    let h = height_at(x, z);
                    if h > SEA_LEVEL + 1 {
                        for dy in 1..=2 {
                            if block_at(x, h + dy, z) == Block::Cactus {
                                found = true;
                                break 'outer;
                            }
                        }
                    }
                }
            }
        }
        assert!(found, "沙漠群系中應能找到仙人掌");
    }

    #[test]
    fn cactus_only_in_desert() {
        // 仙人掌只生在沙漠，非沙漠群系不得出現。
        for x in -100..100i32 {
            let h = height_at(x, 0);
            for dy in 1..=3 {
                let b = block_at(x, h + dy, 0);
                if b == Block::Cactus {
                    assert_eq!(
                        biome_at_voxel(x, 0), VoxelBiome::Desert,
                        "仙人掌只能在沙漠 @ ({}, {})", x, h + dy
                    );
                }
            }
        }
    }

    #[test]
    fn cactus_is_solid_placeable_roundtrips() {
        assert!(Block::Cactus.is_solid(), "仙人掌應為實心");
        assert!(Block::Cactus.is_placeable(), "仙人掌應可放置");
        assert_eq!(Block::from_u8(54), Some(Block::Cactus));
        assert_eq!(Block::Cactus as u8, 54);
    }

    #[test]
    fn cacti_are_isolated_columns() {
        // 設計不變量：仙人掌是「孤立小柱」——任兩株水平距離（Chebyshev）≥ 2，
        // 永不相鄰連成綠牆（格內側落點保證跨格株距 ≥3；同格只一株）。
        let mut columns: Vec<(i32, i32)> = Vec::new();
        for x in -400..400i32 {
            for z in -400..400i32 {
                let h = height_at(x, z);
                if h > SEA_LEVEL + 1 && block_at(x, h + 1, z) == Block::Cactus {
                    columns.push((x, z));
                }
            }
        }
        assert!(!columns.is_empty(), "掃描範圍內應找得到仙人掌柱");
        // 兩兩檢查相鄰（排序後只需比對附近的即可，但柱數不多、直接兩兩比）。
        for (i, &(x1, z1)) in columns.iter().enumerate() {
            for &(x2, z2) in &columns[i + 1..] {
                let cheb = (x1 - x2).abs().max((z1 - z2).abs());
                assert!(cheb >= 2, "仙人掌不得相鄰 @ ({x1},{z1}) vs ({x2},{z2})");
            }
        }
    }

    #[test]
    fn snow_is_solid_placeable_roundtrips() {
        // 生物群系第二刀：雪方塊實心、可放置、u8 往返一致。
        assert!(Block::Snow.is_solid(), "雪應為實心");
        assert!(Block::Snow.is_placeable(), "雪應可放置");
        assert_eq!(Block::from_u8(55), Some(Block::Snow));
        assert_eq!(Block::Snow as u8, 55);
    }

    #[test]
    fn ice_crystal_is_solid_placeable_roundtrips() {
        // 雪原冰晶採集 v1：冰晶方塊實心、可放置、u8 往返一致。
        assert!(Block::IceCrystal.is_solid(), "冰晶應為實心");
        assert!(Block::IceCrystal.is_placeable(), "冰晶應可放置");
        assert_eq!(Block::from_u8(56), Some(Block::IceCrystal));
        assert_eq!(Block::IceCrystal as u8, 56);
    }

    #[test]
    fn snow_biome_has_ice_crystals() {
        // 雪原群系中應能找到冰晶方塊（地表上方第 1 格）。
        let mut found = false;
        'outer: for x in -800..800i32 {
            for z in -800..800i32 {
                if biome_at_voxel(x, z) == VoxelBiome::Snow {
                    let h = height_at(x, z);
                    if h > SEA_LEVEL + 1 && block_at(x, h + 1, z) == Block::IceCrystal {
                        found = true;
                        break 'outer;
                    }
                }
            }
        }
        assert!(found, "雪原群系中應能找到冰晶");
    }

    #[test]
    fn ice_crystal_only_in_snow() {
        // 冰晶只生在雪原，非雪原群系不得出現。
        for x in -600..600i32 {
            for z in (-600..600i32).step_by(37) {
                let h = height_at(x, z);
                for dy in 1..=2 {
                    if block_at(x, h + dy, z) == Block::IceCrystal {
                        assert_eq!(
                            biome_at_voxel(x, z), VoxelBiome::Snow,
                            "冰晶只能在雪原 @ ({}, {})", x, z
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn ice_crystals_are_isolated() {
        // 設計不變量：冰晶是「孤立小結晶」——任兩株水平距離（Chebyshev）≥ 2，
        // 永不相鄰連成冰牆（格內側落點保證跨格株距 ≥3；同格只一株）。
        let mut crystals: Vec<(i32, i32)> = Vec::new();
        for x in -400..400i32 {
            for z in -400..400i32 {
                let h = height_at(x, z);
                if h > SEA_LEVEL + 1 && block_at(x, h + 1, z) == Block::IceCrystal {
                    crystals.push((x, z));
                }
            }
        }
        assert!(!crystals.is_empty(), "掃描範圍內應找得到冰晶");
        for (i, &(x1, z1)) in crystals.iter().enumerate() {
            for &(x2, z2) in &crystals[i + 1..] {
                let cheb = (x1 - x2).abs().max((z1 - z2).abs());
                assert!(cheb >= 2, "冰晶不得相鄰 @ ({x1},{z1}) vs ({x2},{z2})");
            }
        }
    }

    #[test]
    fn ice_crystal_generation_is_deterministic() {
        // 同座標重複查詢永遠同結果（無狀態世界的核心不變量）。
        for x in [-333, -50, 0, 77, 512] {
            for z in [-421, -12, 5, 199, 640] {
                let h = height_at(x, z);
                let a = block_at(x, h + 1, z);
                let b = block_at(x, h + 1, z);
                assert_eq!(a, b, "冰晶生成應確定性 @ ({x},{z})");
            }
        }
    }

    #[test]
    fn snow_biome_surface_is_snow_over_dirt() {
        // 雪原群系的陸地格：地表為雪、其下為泥土（雪只覆蓋最上層，非整柱）。
        let mut found = false;
        'outer: for x in (-1500..1500i32).step_by(3) {
            for z in (-1500..1500i32).step_by(3) {
                if biome_at_voxel(x, z) == VoxelBiome::Snow {
                    let h = height_at(x, z);
                    if h > SEA_LEVEL + 1 {
                        assert_eq!(
                            block_at(x, h, z), Block::Snow,
                            "雪原地表應為雪 @ ({},{})", x, z
                        );
                        // 地表下一格為泥土（雪只在最上層）。
                        assert_eq!(
                            block_at(x, h - 1, z), Block::Dirt,
                            "雪原地表下應為泥土 @ ({},{})", x, z
                        );
                        found = true;
                        break 'outer;
                    }
                }
            }
        }
        assert!(found, "應找到雪原群系的陸地格");
    }

    #[test]
    fn snow_surface_only_in_snow_biome() {
        // 反向不變量：任何地表雪塊（非玩家放置的程序生成）必屬雪原群系。
        for x in (-1500..1500i32).step_by(7) {
            for z in (-1500..1500i32).step_by(7) {
                let h = height_at(x, z);
                if h > SEA_LEVEL + 1 && block_at(x, h, z) == Block::Snow {
                    assert_eq!(
                        biome_at_voxel(x, z), VoxelBiome::Snow,
                        "地表雪塊只該出現在雪原 @ ({},{})", x, z
                    );
                }
            }
        }
    }

    #[test]
    fn snow_biome_has_sparse_trees() {
        // 雪原是有樹群系（疏落針葉，密度同草原），非光禿——掃描範圍內應找得到雪原樹格。
        let mut found = false;
        for cx in -200..200i32 {
            for cz in -200..200i32 {
                let center_x = cx * TREE_CELL + TREE_CELL / 2;
                let center_z = cz * TREE_CELL + TREE_CELL / 2;
                if biome_at_voxel(center_x, center_z) == VoxelBiome::Snow
                    && tree_in_cell(cx, cz).is_some()
                {
                    found = true;
                    break;
                }
            }
            if found {
                break;
            }
        }
        assert!(found, "雪原群系應有疏落的樹");
    }
}
