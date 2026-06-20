//! 漁夫的驕傲・魚的尺寸與個人最大尾紀錄（ROADMAP 449）。
//!
//! 釣魚（47／346／363／434）此前釣到的魚只有「魚種＋品質」兩個維度，
//! 一旦背包夠用，這條活動就沒有任何「再釣一竿」的長尾動機。本模組給釣魚補上
//! 第一個**續釣目標**：每尾魚現在都有一個**尺寸**（公分），釣到比自己這趟最大的
//! 同種魚更大的一尾＝刷新「個人最大尾」，響一聲獎盃慶賀。釣客從此會為了「破自己的
//! 紀錄」一竿一竿地下——這是釣魚迴圈第一個自我較勁的進程。
//!
//! 純邏輯、純函式、確定性（尺寸由種子推導，與魚種擲骰同一套確定性模型），無 IO、
//! 不碰 WebSocket／遊戲迴圈，便於自動測試。尺寸**不進戰鬥／經濟核心結算**
//! （售價、料理、家具加成全部只看魚種與品質，與尺寸無關），故零平衡風險。
//! 個人紀錄是**記憶體前置、不持久化、零 migration**（鏡像 wayfaring／dish_mastery／
//! traced_constellations 等記憶體切片）：重啟清空、重新攢——壓低重登 farming 的誘因，
//! 也讓「這趟連線釣到的最大尾」成為一個輕巧、無包袱的當下成就。

use std::collections::BTreeMap;

use crate::fishing_bite::FishQuality;
use crate::inventory::ItemKind;

/// 魚種的典型體長區間（公釐，min..=max）。越稀有的魚越長：
/// 小魚最短、星星魚居中、深海魚最長。回 `None` 表示這個 `ItemKind` 不是會紀錄尺寸的魚。
///
/// 用公釐（整數）當內部單位：確定性、可排序、序列化乾淨；前端顯示時再除 10 成公分（含一位小數）。
fn size_band_mm(kind: ItemKind) -> Option<(u32, u32)> {
    match kind {
        ItemKind::FishSmall => Some((80, 280)),
        ItemKind::FishStar => Some((150, 450)),
        ItemKind::FishDeep => Some((300, 900)),
        _ => None,
    }
}

/// 品質帶來的「尺寸下限抬升比例」：收竿反應越漂亮，這尾越偏向體長區間的上半段
/// （完美的一收更容易拉上大魚）。回 `[0,1)` 的下限分數——0＝可落在整個區間、
/// 0.5＝至少落在區間上半。刻意溫和，讓任何品質都仍有機會碰到大尾、也仍有機會是小尾。
fn quality_floor_frac(quality: FishQuality) -> f32 {
    match quality {
        FishQuality::Ok => 0.0,
        FishQuality::Good => 0.25,
        FishQuality::Perfect => 0.5,
    }
}

/// 依魚種、收竿品質與確定性種子，算出這一尾的體長（公釐）。
///
/// 結果保證落在該魚種體長區間 `[lo, hi]` 內；非紀錄魚種回 0（呼叫端不應對其紀錄）。
/// 種子建議沿用釣魚既有的 `player_id_low64 ^ fish_attempt_count`（與魚種擲骰同源、
/// 但這裡再混入一個固定鹽值，避免尺寸與魚種完全同相）。
pub fn roll_size_mm(kind: ItemKind, quality: FishQuality, seed: u64) -> u32 {
    let (lo, hi) = match size_band_mm(kind) {
        Some(band) => band,
        None => return 0,
    };
    let floor = quality_floor_frac(quality);
    // 混入固定鹽值再取模，讓「同一尾魚的尺寸」與「魚種擲骰」用同一顆種子卻不同相。
    let r = seed.wrapping_mul(2_654_435_761).rotate_left(13);
    let base = (r % 1000) as f32 / 1000.0; // [0,1)
    let frac = floor + (1.0 - floor) * base; // [floor,1)
    let span = (hi - lo) as f32;
    let mm = lo as f32 + frac * span;
    // 夾在區間內（防浮點誤差越界），四捨五入成整數公釐。
    (mm.round() as u32).clamp(lo, hi)
}

/// 一次收竿對個人紀錄的影響。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatchRecord {
    /// 刷新了這個魚種的個人最大尾。`prev_mm` 為先前紀錄（`None`＝這趟第一次釣到此種）。
    NewBest { size_mm: u32, prev_mm: Option<u32> },
    /// 沒破紀錄。`best_mm` 為目前保持的個人最大尾。
    NotBest { size_mm: u32, best_mm: u32 },
}

/// 玩家這趟連線各魚種的個人最大尾紀錄（記憶體前置、不持久化、零 migration、重啟清空）。
#[derive(Debug, Clone, Default)]
pub struct FishRecords {
    /// 魚種 → 該種目前保持的最大體長（公釐）。`BTreeMap` 故順序確定。
    best: BTreeMap<ItemKind, u32>,
}

impl FishRecords {
    /// 記下一尾魚，回報是否刷新個人最大尾。嚴格大於才算破紀錄（並列不算，避免每尾都慶賀）。
    /// 冪等：同樣大小再記一次不會再回 `NewBest`。
    pub fn record(&mut self, kind: ItemKind, size_mm: u32) -> CatchRecord {
        match self.best.get(&kind).copied() {
            Some(prev) if size_mm <= prev => CatchRecord::NotBest {
                size_mm,
                best_mm: prev,
            },
            prev => {
                self.best.insert(kind, size_mm);
                CatchRecord::NewBest {
                    size_mm,
                    prev_mm: prev,
                }
            }
        }
    }

    /// 查某魚種目前的個人最大尾（公釐），沒釣過回 `None`。
    pub fn best_of(&self, kind: ItemKind) -> Option<u32> {
        self.best.get(&kind).copied()
    }

    /// 已立下紀錄的魚種數（目前最多 3 種）。
    pub fn species_count(&self) -> usize {
        self.best.len()
    }
}

// ─── 單元測試 ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// 各魚種尺寸恆落在其體長區間內（掃一萬顆種子、三種品質）。
    #[test]
    fn size_always_within_band() {
        for kind in [ItemKind::FishSmall, ItemKind::FishStar, ItemKind::FishDeep] {
            let (lo, hi) = size_band_mm(kind).unwrap();
            for quality in [FishQuality::Ok, FishQuality::Good, FishQuality::Perfect] {
                for seed in 0..10_000u64 {
                    let mm = roll_size_mm(kind, quality, seed);
                    assert!(
                        mm >= lo && mm <= hi,
                        "{kind:?} {quality:?} seed={seed} → {mm} 越界 [{lo},{hi}]"
                    );
                }
            }
        }
    }

    /// 非紀錄魚種（例：木材）回 0。
    #[test]
    fn non_fish_rolls_zero() {
        assert_eq!(roll_size_mm(ItemKind::Wood, FishQuality::Perfect, 42), 0);
        assert_eq!(roll_size_mm(ItemKind::Stone, FishQuality::Ok, 7), 0);
    }

    /// 確定性：同樣輸入永遠同樣輸出。
    #[test]
    fn roll_is_deterministic() {
        let a = roll_size_mm(ItemKind::FishStar, FishQuality::Good, 12345);
        let b = roll_size_mm(ItemKind::FishStar, FishQuality::Good, 12345);
        assert_eq!(a, b);
    }

    /// 完美收竿的平均尺寸明顯大於普通收竿（品質抬升下限的統計效果）。
    #[test]
    fn perfect_quality_skews_larger() {
        let kind = ItemKind::FishDeep;
        let avg = |q: FishQuality| -> f64 {
            let n = 5_000u64;
            let sum: u64 = (0..n).map(|s| roll_size_mm(kind, q, s) as u64).sum();
            sum as f64 / n as f64
        };
        let ok = avg(FishQuality::Ok);
        let perfect = avg(FishQuality::Perfect);
        assert!(
            perfect > ok + 50.0,
            "完美({perfect:.1}) 應明顯大於普通({ok:.1})"
        );
    }

    /// 第一次釣到某種＝NewBest(prev=None)；更大才再破紀錄、較小／並列回 NotBest。
    #[test]
    fn record_tracks_personal_best() {
        let mut r = FishRecords::default();
        assert_eq!(
            r.record(ItemKind::FishSmall, 120),
            CatchRecord::NewBest {
                size_mm: 120,
                prev_mm: None
            }
        );
        // 更小：不破紀錄。
        assert_eq!(
            r.record(ItemKind::FishSmall, 100),
            CatchRecord::NotBest {
                size_mm: 100,
                best_mm: 120
            }
        );
        // 並列：不破紀錄（嚴格大於才算）。
        assert_eq!(
            r.record(ItemKind::FishSmall, 120),
            CatchRecord::NotBest {
                size_mm: 120,
                best_mm: 120
            }
        );
        // 更大：破紀錄，帶出舊紀錄。
        assert_eq!(
            r.record(ItemKind::FishSmall, 200),
            CatchRecord::NewBest {
                size_mm: 200,
                prev_mm: Some(120)
            }
        );
        assert_eq!(r.best_of(ItemKind::FishSmall), Some(200));
    }

    /// 不同魚種各自獨立計紀錄。
    #[test]
    fn records_are_per_species() {
        let mut r = FishRecords::default();
        r.record(ItemKind::FishSmall, 150);
        r.record(ItemKind::FishStar, 300);
        assert_eq!(r.best_of(ItemKind::FishSmall), Some(150));
        assert_eq!(r.best_of(ItemKind::FishStar), Some(300));
        assert_eq!(r.best_of(ItemKind::FishDeep), None);
        assert_eq!(r.species_count(), 2);
    }
}
