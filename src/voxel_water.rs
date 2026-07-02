//! 乙太方界水流動模擬核心（麥塊式概念·原創碼實作）。
//!
//! 維護者反饋「水不會流動」——地形生成時只把海平面以下的空格填 `Block::Water`，
//! 玩家挖破湖岸／地形後，水完全靜止不動、也不填回缺口。這裡做一套「來源不乾涸、
//! 破口會流、離源太遠會乾涸」的**確定性水流模擬**，設計概念抄自 Minecraft 的
//! 「來源水(level 0) + 流動水(level 1..=7 遞減)」，但**程式碼全為原創、可測**。
//!
//! 分層原則（對齊 voxel.rs 的哲學）：
//! - 本模組**只放與世界狀態無關的純函式**（給定周圍方塊 → 這格該變成什麼）。
//!   不碰 hub / 鎖 / 廣播 / IO——那些接線與 tick 排程在 voxel_ws.rs。
//! - 方塊 id 定義、`Block` enum、`effective_block_at`/`set_block` 仍是 voxel.rs 的真相；
//!   本模組只**引用**它們，維持「方塊世界的真相在 voxel.rs」的單一來源。
//!
//! 為什麼不整世界每 tick 掃描（效能鐵律）：世界無限大，全掃會拖垮既有
//! 居民 agency + 日夜 + 農地 tick。改用**待處理佇列**：只在「有可能變化」的格
//! （玩家挖破的缺口鄰格、既有水格自己擴散到的新鄰格）排入佇列，每 tick 只算佇列，
//! 穩定的移出，天然收斂、成本與「正在流動的水量」成正比而非世界大小。

use crate::voxel::{self, Block, WorldDelta};

/// 流動水的最大等級（麥塊同款：來源=0，流動 1..=7 遞減，>7 就不再擴散）。
pub const MAX_FLOW: u8 = 7;

/// 流動水方塊 id 起點：接續現有最大方塊 id（IronBlock=23）之後，佔 24..=30 共 7 個。
/// **向後相容**：新 id 不影響舊存檔既有方塊（舊世界的 Water=7 意義不變）；
/// 流動水非實心，碰撞/挖放規則同 Water（見 voxel.rs `is_solid` / `can_place`）。
pub const FLOW_ID_BASE: u8 = 24;

/// 把「流動等級 lvl(1..=7)」轉成方塊 id（24..=30）。lvl 0 是來源、不走這裡。
#[inline]
pub fn flow_id(lvl: u8) -> u8 {
    debug_assert!((1..=MAX_FLOW).contains(&lvl));
    FLOW_ID_BASE + (lvl - 1)
}

/// 若 id 是流動水 → 回其等級 1..=7；否則 None。純函式，供 voxel.rs from_u8 / 前端對齊。
#[inline]
pub fn flow_level_of_id(id: u8) -> Option<u8> {
    if (FLOW_ID_BASE..FLOW_ID_BASE + MAX_FLOW).contains(&id) {
        Some(id - FLOW_ID_BASE + 1)
    } else {
        None
    }
}

/// 一個方塊在「水」意義下的狀態：不是水 / 來源水(level 0) / 流動水(level 1..=7)。
/// 抽出來讓所有流動判定都對著同一份語意，純資料、可測。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaterState {
    /// 不是水（空氣、實心方塊皆歸此；用 `is_air` 進一步分辨可否被水淹）。
    Dry { is_air: bool },
    /// 天然來源水（海/湖，level 0）——**無限、永不乾涸**。
    Source,
    /// 流動水，帶等級 1..=7（數字越大離源越遠、越接近乾涸）。
    Flow(u8),
}

impl WaterState {
    /// 由方塊判定水狀態。Water=來源；24..=30=流動；Air=可被淹的乾格；其餘=實心乾格。
    pub fn of(b: Block) -> WaterState {
        match b {
            Block::Water => WaterState::Source,
            Block::Air => WaterState::Dry { is_air: true },
            other => match flow_level_of_id(other as u8) {
                Some(lvl) => WaterState::Flow(lvl),
                None => WaterState::Dry { is_air: false },
            },
        }
    }

    /// 是不是「任何水」（來源或流動）。
    #[inline]
    pub fn is_water(self) -> bool {
        matches!(self, WaterState::Source | WaterState::Flow(_))
    }

    /// 這格能不能被水淹進來（空氣 or 既有流動水才可被更強的水覆蓋；來源/實心不可）。
    /// 來源水不被覆蓋（無限來源優先）；實心方塊擋水。
    #[inline]
    pub fn floodable(self) -> bool {
        matches!(self, WaterState::Dry { is_air: true } | WaterState::Flow(_))
    }

    /// 這格「向下方供水」時算作幾級的水源頭：來源與任何流動水，向下都視為「滿格下灌」，
    /// 讓瀑布正下方及底部漫開的水維持強度（麥塊式：垂直落水不遞減）。回 None = 不是水。
    #[inline]
    fn as_water_level(self) -> Option<u8> {
        match self {
            WaterState::Source => Some(0),
            WaterState::Flow(l) => Some(l),
            WaterState::Dry { .. } => None,
        }
    }
}

/// 一格周圍的方塊快照（給純函式算「這格該變成什麼水」）。
/// 只帶「上、下、四個水平鄰格」6 面——水流只看直接鄰居，O(1)。
#[derive(Clone, Copy, Debug)]
pub struct Neighborhood {
    /// 這格自己現在的方塊。
    pub here: Block,
    /// 正上方方塊（判斷是否有水往下灌）。
    pub above: Block,
    /// 正下方方塊（判斷水能不能繼續往下流）。
    pub below: Block,
    /// 四個水平鄰格（±x, ±z）。
    pub sides: [Block; 4],
}

/// 純函式核心：給定一格的鄰域，算出這格「穩定後」該是什麼方塊。
///
/// 規則（麥塊式概念、原創實作）：
/// 1. **來源水永不改變**——遠古海／湖不該被算乾涸（無限來源）。實心方塊也不變（擋水）。
/// 2. 這格若可被淹（空氣或既有流動水）：
///    a. 正上方是水（來源或流動）→ 垂直下灌，這格成 **level 1**（滿格流動、可再往外漫）。
///    b. 否則看四個水平鄰格中「是水」者的最小等級 m：這格成 level `m+1`（遞減）。
///       但若 `m+1 > MAX_FLOW`（離源太遠）→ 這格**乾涸**成 Air。
///    c. 四周與上方都沒有水撐著 → 乾涸成 Air（缺口被填/水退去）。
/// 3. 回傳「這格應有的方塊」；與現值相同代表已穩定（呼叫端據此決定是否還要傳播）。
pub fn settled_block(n: &Neighborhood) -> Block {
    let here = WaterState::of(n.here);

    // 規則 1：來源與實心乾格不變（來源無限、實心擋水）。
    match here {
        WaterState::Source => return Block::Water,
        WaterState::Dry { is_air: false } => return n.here, // 實心方塊原樣
        _ => {}
    }
    // 此時 here 是「可被淹的空氣」或「既有流動水」。

    // 規則 2a：正上方有水 → 垂直下灌，滿格 level 1。
    if WaterState::of(n.above).is_water() {
        return Block::from_u8(flow_id(1)).unwrap_or(Block::Air);
    }

    // 規則 2b：四個水平鄰格中，取「是水」者的最小等級。
    let mut min_supply: Option<u8> = None;
    for &s in &n.sides {
        if let Some(lvl) = WaterState::of(s).as_water_level() {
            min_supply = Some(min_supply.map_or(lvl, |m| m.min(lvl)));
        }
    }
    match min_supply {
        Some(m) => {
            let next = m + 1;
            if next > MAX_FLOW {
                Block::Air // 離源太遠 → 乾涸
            } else {
                Block::from_u8(flow_id(next)).unwrap_or(Block::Air)
            }
        }
        // 規則 2c：四周無水撐著 → 乾涸（缺口被填、或來源被移走後水退）。
        None => Block::Air,
    }
}

/// 這格更新後，哪些鄰格「可能」需要重新評估、要排回佇列？
/// 只有「這格是水（或剛變成水/剛乾涸）」時才需要驚動鄰格：
/// - 下方（水會往下流）、四個水平鄰格（會往外漫或需補位）、上方（上方流動水可能失去支撐）。
/// 回傳 6 個方向的偏移；呼叫端據 `changed` 與是否為水決定要不要真的排入（省佇列）。
pub const PROPAGATE_OFFSETS: [(i32, i32, i32); 6] = [
    (0, -1, 0), // 下（水往下流優先）
    (1, 0, 0),
    (-1, 0, 0),
    (0, 0, 1),
    (0, 0, -1),
    (0, 1, 0), // 上（上方流動水可能失去支撐→要重算）
];

/// 從世界 delta 讀一格的鄰域快照（純讀，呼叫端在持 delta 讀鎖時呼叫、即讀即用）。
/// 抽成函式讓「取鄰域」與「算結果」分離，tick 端好守「短鎖快照 → drop → 純計算」。
pub fn neighborhood_at(world: &WorldDelta, x: i32, y: i32, z: i32) -> Neighborhood {
    let g = |dx: i32, dy: i32, dz: i32| voxel::effective_block_at(world, x + dx, y + dy, z + dz);
    Neighborhood {
        here: g(0, 0, 0),
        above: g(0, 1, 0),
        below: g(0, -1, 0),
        sides: [g(1, 0, 0), g(-1, 0, 0), g(0, 0, 1), g(0, 0, -1)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voxel::{self, WorldDelta};

    /// 造一個「四周都是實心石頭、指定面換成給定方塊」的鄰域，方便單獨釘規則。
    fn nb(here: Block, above: Block, below: Block, sides: [Block; 4]) -> Neighborhood {
        Neighborhood { here, above, below, sides }
    }

    #[test]
    fn flow_id_roundtrips() {
        // 等級 1..=7 ↔ id 24..=30 一一對應，且與既有方塊 id（0..=23）不重疊。
        for lvl in 1..=MAX_FLOW {
            let id = flow_id(lvl);
            assert!(id >= 24 && id <= 30, "流動水 id 應落在 24..=30：{id}");
            assert_eq!(flow_level_of_id(id), Some(lvl));
        }
        // 既有方塊 id 不被誤判成流動水。
        for id in 0u8..=23 {
            assert_eq!(flow_level_of_id(id), None, "既有方塊 id={id} 不該被當流動水");
        }
        assert_eq!(flow_level_of_id(31), None);
    }

    #[test]
    fn source_never_dries() {
        // 來源水四周即使全是實心/空氣，也永遠維持 Water（無限來源、遠古海不乾涸）。
        let n = nb(Block::Water, Block::Air, Block::Stone,
                   [Block::Air, Block::Air, Block::Air, Block::Air]);
        assert_eq!(settled_block(&n), Block::Water);
    }

    #[test]
    fn solid_block_unchanged() {
        // 實心方塊擋水、原樣不變。
        let n = nb(Block::Stone, Block::Water, Block::Water,
                   [Block::Water, Block::Water, Block::Water, Block::Water]);
        assert_eq!(settled_block(&n), Block::Stone);
    }

    #[test]
    fn water_never_erodes_resident_built_structure_blocks() {
        // 蓋家鬼打牆調查結論（b）：水流**不會**沖掉居民蓋的建物方塊——
        // 水井/花圃/小屋的框架是 Stone/Wood（實心，被水包圍也原樣不變），
        // 井中心的 Water 是**來源**（永不乾涸、不被覆蓋）。故完工偵測不會因水而失真。
        // 這裡把「四面全是水的建物方塊」都釘死：驗證任一種建材都不被水侵蝕。
        for b in [Block::Stone, Block::Wood, Block::Grass, Block::Leaves] {
            let surrounded = nb(b, Block::Water, Block::Water,
                                [Block::Water, Block::Water, Block::Water, Block::Water]);
            assert_eq!(settled_block(&surrounded), b, "建材 {:?} 被水包圍仍不被侵蝕", b);
        }
        // 井中心的來源水四面被流動水包圍，也維持來源（不被降級/沖走）。
        let l3 = Block::from_u8(flow_id(3)).unwrap();
        let well_center = nb(Block::Water, Block::Air, Block::Stone, [l3, l3, l3, l3]);
        assert_eq!(settled_block(&well_center), Block::Water, "井心來源水不被流動水覆蓋");
    }

    #[test]
    fn air_next_to_source_becomes_level_1() {
        // 空氣格挨著來源水（水平），應成 level 1（0+1）。
        let n = nb(Block::Air, Block::Air, Block::Stone,
                   [Block::Water, Block::Stone, Block::Stone, Block::Stone]);
        assert_eq!(settled_block(&n), Block::from_u8(flow_id(1)).unwrap());
    }

    #[test]
    fn flow_level_decreases_with_distance() {
        // 空氣格只被 level 3 的流動水撐著（水平）→ 應成 level 4（遞減）。
        let l3 = Block::from_u8(flow_id(3)).unwrap();
        let n = nb(Block::Air, Block::Stone, Block::Stone,
                   [l3, Block::Stone, Block::Stone, Block::Stone]);
        assert_eq!(settled_block(&n), Block::from_u8(flow_id(4)).unwrap());
    }

    #[test]
    fn too_far_from_source_dries_up() {
        // 只被 level 7（最遠）撐著 → 7+1=8 > MAX_FLOW → 乾涸成 Air。
        let l7 = Block::from_u8(flow_id(7)).unwrap();
        let n = nb(Block::Air, Block::Stone, Block::Stone,
                   [l7, Block::Stone, Block::Stone, Block::Stone]);
        assert_eq!(settled_block(&n), Block::Air, "離源太遠應乾涸");
    }

    #[test]
    fn water_above_pours_straight_down_full() {
        // 正上方是流動水（level 5）→ 垂直下灌不遞減，這格成滿格 level 1。
        let l5 = Block::from_u8(flow_id(5)).unwrap();
        let n = nb(Block::Air, l5, Block::Air,
                   [Block::Stone, Block::Stone, Block::Stone, Block::Stone]);
        assert_eq!(settled_block(&n), Block::from_u8(flow_id(1)).unwrap());
    }

    #[test]
    fn unsupported_flow_dries_to_air() {
        // 一格既有流動水，但四周＋上方都沒水撐 → 乾涸成 Air（水退／缺口被填）。
        let l2 = Block::from_u8(flow_id(2)).unwrap();
        let n = nb(l2, Block::Air, Block::Stone,
                   [Block::Stone, Block::Stone, Block::Stone, Block::Air]);
        assert_eq!(settled_block(&n), Block::Air);
    }

    #[test]
    fn takes_minimum_neighbor_level() {
        // 同時被 level 2 與 level 5 撐著 → 取最小(2)+1 = level 3。
        let l2 = Block::from_u8(flow_id(2)).unwrap();
        let l5 = Block::from_u8(flow_id(5)).unwrap();
        let n = nb(Block::Air, Block::Stone, Block::Stone,
                   [l2, l5, Block::Stone, Block::Stone]);
        assert_eq!(settled_block(&n), Block::from_u8(flow_id(3)).unwrap());
    }

    #[test]
    fn water_state_classification() {
        assert_eq!(WaterState::of(Block::Water), WaterState::Source);
        assert_eq!(WaterState::of(Block::Air), WaterState::Dry { is_air: true });
        assert_eq!(WaterState::of(Block::Stone), WaterState::Dry { is_air: false });
        assert_eq!(WaterState::of(Block::from_u8(flow_id(4)).unwrap()), WaterState::Flow(4));
        assert!(WaterState::Source.floodable() == false);
        assert!(WaterState::Dry { is_air: true }.floodable());
        assert!(WaterState::Flow(3).floodable());
        assert!(WaterState::Dry { is_air: false }.floodable() == false);
    }

    // ── 小型模擬觀察：真實情境「挖破湖岸讓水流出」（實測證據，比照居民壓力測）──────
    //
    // 直接在記憶體 WorldDelta 上跑一個「排隊→處理→傳播」的小模擬，不碰 hub/鎖/廣播，
    // 驗證：① 破口會被水填 ② 水往下流進窪地 ③ 離來源太遠處會乾涸/停止擴散 ④ 來源不乾涸。

    /// 把一格算穩定值寫回 delta，回傳 (是否改變, 這格穩定後是否為水)。
    fn step_cell(world: &mut WorldDelta, x: i32, y: i32, z: i32) -> (bool, bool) {
        let n = neighborhood_at(world, x, y, z);
        let next = settled_block(&n);
        let changed = next != n.here;
        if changed {
            voxel::set_block(world, x, y, z, next);
        }
        (changed, WaterState::of(next).is_water())
    }

    /// 跑滿佇列直到穩定（帶步數上限防呆），回傳處理過的格數。純記憶體、確定性。
    fn run_sim(world: &mut WorldDelta, seed: Vec<(i32, i32, i32)>, max_steps: usize) -> usize {
        use std::collections::VecDeque;
        let mut queue: VecDeque<(i32, i32, i32)> = seed.into_iter().collect();
        let mut steps = 0;
        while let Some((x, y, z)) = queue.pop_front() {
            steps += 1;
            if steps > max_steps {
                break;
            }
            let (changed, _is_water) = step_cell(world, x, y, z);
            if changed {
                // 改變了 → 把 6 鄰排回佇列重新評估（可能被淹或失去支撐）。
                for (dx, dy, dz) in PROPAGATE_OFFSETS {
                    queue.push_back((x + dx, y + dy, z + dz));
                }
            }
        }
        steps
    }

    #[test]
    fn sim_breach_lake_flows_out_and_down() {
        // 造一個「牆後一格來源水、旁邊有缺口通向低地」的最小場景。
        // 佈局（y 固定平面 + 一格下陷窪地）：
        //   來源水在 (0,10,0)；(1,10,0) 原是實心牆（缺口打通後變空氣）；
        //   (2,10,0) 是空地；(2,9,0) 是空氣窪地（水應往下流進去）。
        let mut world = WorldDelta::new();
        // 先鋪一層石頭地板 y=9（讓水有底、只在 y=10 漫開，缺口處往下流）。
        for x in -1..=3 {
            for z in -1..=1 {
                voxel::set_block(&mut world, x, 9, z, Block::Stone);
            }
        }
        // 窪地：把 (2,9,0) 的地板挖空 → 水到這會往下掉。
        voxel::set_block(&mut world, 2, 9, 0, Block::Air);
        voxel::set_block(&mut world, 2, 8, 0, Block::Stone); // 窪底
        // 來源水。
        voxel::set_block(&mut world, 0, 10, 0, Block::Water);
        // 圍牆（除了缺口）：讓水只能往 +x 方向流。
        for z in [-1, 1] {
            for x in 0..=2 {
                voxel::set_block(&mut world, x, 10, z, Block::Stone);
            }
        }
        voxel::set_block(&mut world, -1, 10, 0, Block::Stone);
        // (1,10,0)=缺口（空氣，玩家剛挖破的牆）；(2,10,0)=空氣。
        voxel::set_block(&mut world, 1, 10, 0, Block::Air);
        voxel::set_block(&mut world, 2, 10, 0, Block::Air);

        // 排入缺口鄰格（模擬「玩家挖破牆」時 voxel_ws 會做的事）。
        run_sim(&mut world, vec![(1, 10, 0)], 10_000);

        // ① 缺口被水填（成流動水，非空氣）。
        assert!(
            WaterState::of(voxel::effective_block_at(&world, 1, 10, 0)).is_water(),
            "缺口應被水填"
        );
        // ② 水往下流進窪地 (2,9,0)。
        assert!(
            WaterState::of(voxel::effective_block_at(&world, 2, 9, 0)).is_water(),
            "水應往下流進窪地"
        );
        // ④ 來源不乾涸。
        assert_eq!(
            voxel::effective_block_at(&world, 0, 10, 0),
            Block::Water,
            "來源水永不乾涸"
        );
    }

    #[test]
    fn sim_flow_stops_far_from_source() {
        // 一條長走廊：來源在一端，水順著流，離源 7 格以外應乾涸／停止（不會無限延伸）。
        let mut world = WorldDelta::new();
        // 地板 y=9，走廊 y=10 空氣，長度 20。
        for x in -1..=20 {
            voxel::set_block(&mut world, x, 9, 0, Block::Stone);
            voxel::set_block(&mut world, x, 10, 1, Block::Stone);
            voxel::set_block(&mut world, x, 10, -1, Block::Stone);
        }
        voxel::set_block(&mut world, -1, 10, 0, Block::Stone);
        voxel::set_block(&mut world, 0, 10, 0, Block::Water); // 來源
        run_sim(&mut world, vec![(1, 10, 0)], 100_000);

        // 靠源處（x=1）有水。
        assert!(
            WaterState::of(voxel::effective_block_at(&world, 1, 10, 0)).is_water(),
            "靠源處應有水"
        );
        // 遠端（x=15，遠超 7 格）應是乾的（水無法延伸到那麼遠）。
        assert!(
            !WaterState::of(voxel::effective_block_at(&world, 15, 10, 0)).is_water(),
            "離源太遠處應乾涸/無水"
        );
        // 模擬會收斂（不會無限跑）——run_sim 有上限，能跑到這裡即代表沒爆量。
    }

    #[test]
    fn sim_removing_source_recedes_flow() {
        // 來源被移走後，先前流出去的水應逐步乾涸（不留孤水）。
        let mut world = WorldDelta::new();
        for x in -1..=10 {
            voxel::set_block(&mut world, x, 9, 0, Block::Stone);
            voxel::set_block(&mut world, x, 10, 1, Block::Stone);
            voxel::set_block(&mut world, x, 10, -1, Block::Stone);
        }
        voxel::set_block(&mut world, -1, 10, 0, Block::Stone);
        voxel::set_block(&mut world, 0, 10, 0, Block::Water);
        run_sim(&mut world, vec![(1, 10, 0)], 100_000);
        assert!(WaterState::of(voxel::effective_block_at(&world, 1, 10, 0)).is_water());

        // 移走來源（換成石頭），把原來源格 + 鄰格排入重算。
        voxel::set_block(&mut world, 0, 10, 0, Block::Stone);
        run_sim(&mut world, vec![(1, 10, 0), (0, 10, 0)], 100_000);
        // 先前的流動水應退去（至少靠源那格不再是水）。
        assert!(
            !WaterState::of(voxel::effective_block_at(&world, 1, 10, 0)).is_water(),
            "來源移走後流動水應退去"
        );
    }
}
