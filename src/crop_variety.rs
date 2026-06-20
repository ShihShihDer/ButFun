//! 作物品種（ROADMAP 452 深度弧 A 第一塊「種田第一次要選種什麼」）。
//!
//! 在此之前，核心農地（`field.rs`／`crops.rs`，玩家點田格一鍵照顧的那套）只有**一種**
//! 無差別作物——播種、澆水、收成都長一個樣、收一樣多乙太。本切片給作物加上「品種」維度：
//! 第一批 3 個品種，各有不同的**成長速度**與**收成乙太**，讓種田第一次要做「選種什麼」的取捨。
//!
//! 這層只管「某品種長多快、收多少」，是純資料表 + 純函式、無 IO、不碰 WebSocket / 遊戲迴圈，
//! 便於自動測試。被 `crops.rs::Crop` 持有（`#[serde(default)]` → 既有田＝預設品種 Staple，
//! 向後相容、零 migration、不破壞玩家存檔）。
//!
//! 平衡（接既有乙太經濟、刻意不失衡）：**高報酬配長成長為代價**——
//!   - 速生菜 Sprout：長得快、收得少（隨手就有、回報低）。
//!   - 主食穀 Staple：均衡，＝既有單一作物的數值（預設品種，舊存檔無縫沿用）。
//!   - 乙太瓜 Etherbloom：長得慢、收得多（耐心經營、回報高）。
//! 品質（ROADMAP 406）與沃土（438）加成與品種正交、各自獨立疊加，不在本層處理。

use serde::{Deserialize, Serialize};

use crate::crops::ETHER_PER_HARVEST;

/// 速生菜的成長速度倍率（>1＝比基準長得快）。
pub const SPROUT_GROW_RATE: f32 = 1.6;
/// 乙太瓜的成長速度倍率（<1＝比基準長得慢）。
pub const ETHERBLOOM_GROW_RATE: f32 = 0.6;
/// 速生菜收成的基礎乙太（少於主食穀＝產出低的代價）。
pub const SPROUT_ETHER: u32 = 2;
/// 乙太瓜收成的基礎乙太（高於主食穀＝長成長換來的高報酬）。
pub const ETHERBLOOM_ETHER: u32 = 5;

/// 一株作物的品種。決定它的成長速度與收成乙太；其餘照顧迴圈（澆水、品質、沃土）共用同一套。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CropVariety {
    /// 速生菜：成長快、產出少。隨手種、隨手收，回報低但不費心。
    Sprout,
    /// 主食穀：均衡，＝既有單一作物的數值。預設品種（舊存檔／無品種一律落在這）。
    Staple,
    /// 乙太瓜：成長慢、產出多。耐心經營換高報酬。
    Etherbloom,
}

impl Default for CropVariety {
    /// 預設＝主食穀：既有玩家的田、舊存檔反序列化缺欄時都落在這，行為與改動前完全一致。
    fn default() -> Self {
        CropVariety::Staple
    }
}

impl CropVariety {
    /// 本批所有品種（前端列選單、測試窮舉共用同一份順序）。
    pub const ALL: [CropVariety; 3] = [
        CropVariety::Sprout,
        CropVariety::Staple,
        CropVariety::Etherbloom,
    ];

    /// 線格式字串（snake_case，對齊既有事件約定；前端據此送播種品種、查表顯示名稱）。
    pub fn as_str(self) -> &'static str {
        match self {
            CropVariety::Sprout => "sprout",
            CropVariety::Staple => "staple",
            CropVariety::Etherbloom => "etherbloom",
        }
    }

    /// 由線格式字串解析品種；未知 / 空 / 缺一律退回預設（Staple），永不失敗——
    /// 讓舊前端或被竄改的客戶端送怪字串時，保守當成最普通的主食穀，不破壞耕作。
    pub fn from_wire(s: &str) -> CropVariety {
        match s {
            "sprout" => CropVariety::Sprout,
            "etherbloom" => CropVariety::Etherbloom,
            // "staple" 與其餘未知值都落在預設。
            _ => CropVariety::Staple,
        }
    }

    /// 線格式碼（隨田格快照下傳，給前端畫品種微異外觀：0=主食穀 1=速生菜 2=乙太瓜）。
    /// 主食穀＝0 對齊「預設品種」，舊前端讀不到欄位時的 `0` 自然落在主食穀。
    pub fn code(self) -> u8 {
        match self {
            CropVariety::Staple => 0,
            CropVariety::Sprout => 1,
            CropVariety::Etherbloom => 2,
        }
    }

    /// 這個品種的成長速度倍率（作用在每 tick 的有效成長量上；澆水耗水仍按真實時間，不受影響）。
    /// 主食穀＝1.0（基準、與改動前一致）。純函式。
    pub fn grow_rate(self) -> f32 {
        match self {
            CropVariety::Sprout => SPROUT_GROW_RATE,
            CropVariety::Staple => 1.0,
            CropVariety::Etherbloom => ETHERBLOOM_GROW_RATE,
        }
    }

    /// 這個品種收成一株的基礎乙太（品質加成與沃土加成另計、與品種正交）。
    /// 主食穀＝既有 `ETHER_PER_HARVEST`（與改動前一致）。純函式。
    pub fn harvest_ether(self) -> u32 {
        match self {
            CropVariety::Sprout => SPROUT_ETHER,
            CropVariety::Staple => ETHER_PER_HARVEST,
            CropVariety::Etherbloom => ETHERBLOOM_ETHER,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_staple_matching_legacy() {
        // 預設＝主食穀，數值＝改動前的單一作物（向後相容的關鍵）。
        assert_eq!(CropVariety::default(), CropVariety::Staple);
        assert_eq!(CropVariety::Staple.grow_rate(), 1.0);
        assert_eq!(CropVariety::Staple.harvest_ether(), ETHER_PER_HARVEST);
    }

    #[test]
    fn wire_round_trips_and_unknown_falls_back() {
        for v in CropVariety::ALL {
            assert_eq!(CropVariety::from_wire(v.as_str()), v, "{} 應往返", v.as_str());
        }
        // 未知 / 空字串保守退預設，不破壞耕作。
        assert_eq!(CropVariety::from_wire(""), CropVariety::Staple);
        assert_eq!(CropVariety::from_wire("turnip"), CropVariety::Staple);
        assert_eq!(CropVariety::from_wire("STAPLE"), CropVariety::Staple);
    }

    #[test]
    fn codes_are_stable_and_default_is_zero() {
        // 主食穀＝0：舊前端讀不到 kind 欄位的預設 0 自然落在主食穀。
        assert_eq!(CropVariety::Staple.code(), 0);
        assert_eq!(CropVariety::Sprout.code(), 1);
        assert_eq!(CropVariety::Etherbloom.code(), 2);
    }

    #[test]
    fn higher_reward_costs_slower_growth() {
        // 平衡不變式：產出越高，成長越慢（高報酬配長成長為代價）。
        // 速生菜：最快、最少。
        assert!(CropVariety::Sprout.grow_rate() > CropVariety::Staple.grow_rate());
        assert!(CropVariety::Sprout.harvest_ether() < CropVariety::Staple.harvest_ether());
        // 乙太瓜：最慢、最多。
        assert!(CropVariety::Etherbloom.grow_rate() < CropVariety::Staple.grow_rate());
        assert!(CropVariety::Etherbloom.harvest_ether() > CropVariety::Staple.harvest_ether());
        // 成長速度與收成乙太單調反向（rate 越大、ether 越小）。
        let mut by_rate = CropVariety::ALL;
        by_rate.sort_by(|a, b| a.grow_rate().partial_cmp(&b.grow_rate()).unwrap());
        // 由慢到快：乙太瓜 → 主食穀 → 速生菜；對應乙太由多到少。
        assert_eq!(by_rate, [CropVariety::Etherbloom, CropVariety::Staple, CropVariety::Sprout]);
        assert!(by_rate[0].harvest_ether() > by_rate[1].harvest_ether());
        assert!(by_rate[1].harvest_ether() > by_rate[2].harvest_ether());
    }

    #[test]
    fn grow_rates_are_finite_positive() {
        // 成長倍率必為有限正值（grow_boosted 才不會被汙染或停滯）。
        for v in CropVariety::ALL {
            assert!(v.grow_rate().is_finite() && v.grow_rate() > 0.0, "{} 倍率須正", v.as_str());
        }
    }
}
