//! 作物品種·市集行情（ROADMAP 455 深度弧 A 第四塊「本季搶手品種」）。
//!
//! 前三塊（452 選品種、453 季節成長、454 輪作）都作用在**成長那一側**——三條各自獨立的
//! 成長倍率（快慢／當季旺淡／換種旺長）。本切片開一條全新、正交的**產出經濟軸**：每一季，
//! 市集對某一個品種的需求高漲（「本季搶手品種」），收成它就多得一筆乙太溢價。
//!
//! 與 453 季節成長剛好成「**張力**」而非重複：453 讓某品種在某季長得快，本層讓某品種在某季
//! 賣得俏——而搶手季刻意挑在該品種的**淡季**（盛夏的速生菜、秋天的乙太瓜都不是它們的旺季），
//! 於是玩家第一次要在「順著季節種得快」與「逆著季節賣得貴」之間取捨。種田第一次有了「行情」。
//!
//! 純資料表 + 純函式、無 IO、不碰 WebSocket / 遊戲迴圈，便於自動測試。本季由世界權威
//! `app.season` 決定（確定性、零隨機、零 LLM、零持久化）；溢價在收成結算（`ws.rs`）那一下
//! 依「剛收的品種 × 當前季節」算出，**純正向**（非搶手品種＝0，永不倒扣）。

use crate::crop_variety::CropVariety;
use crate::season::Season;

/// 搶手品種收成的乙太溢價比例（占該品種基礎收成乙太 `harvest_ether` 的比例）。
/// 0.5 ＝多得半份基礎乙太：速生菜 2→+1、主食穀 3→+2、乙太瓜 5→+3（四捨五入後皆 ≥1，看得見）。
pub const DEMAND_PREMIUM_FRAC: f32 = 0.5;

/// 本季「搶手品種」：每季市集對某一品種需求高漲，收成它多得乙太。純函式、確定性。
///
/// 排程刻意讓三個品種各有當令、且把高低報酬品種的搶手季排在它們的**淡季**，製造取捨：
///   - 春：主食穀——四季皆宜的穩當主食，開年穩穩起手。
///   - 夏：速生菜——盛夏是它的淡季（季節成長 ×0.9），長得吃力卻賣得俏。
///   - 秋：乙太瓜——秋天是它的淡季（季節成長 ×0.8），逆季栽培換高溢價。
///   - 冬：主食穀——隆冬萬物皆慢，穩當的主食再次成為市集寵兒。
/// 一輪四季裡三個品種都至少當令一次（見單元測試 `every_variety_demanded_across_year`）。
pub fn demand_variety(season: Season) -> CropVariety {
    match season {
        Season::Spring => CropVariety::Staple,
        Season::Summer => CropVariety::Sprout,
        Season::Autumn => CropVariety::Etherbloom,
        Season::Winter => CropVariety::Staple,
    }
}

/// 收成某品種一株、在某季的市集溢價乙太（純正向；非當季搶手品種＝0）。
/// ＝搶手品種基礎收成乙太的 `DEMAND_PREMIUM_FRAC` 倍（四捨五入），下限 1（搶手必看得見）。純函式。
pub fn demand_bonus_ether(kind: CropVariety, season: Season) -> u32 {
    if kind == demand_variety(season) {
        let raw = (kind.harvest_ether() as f32 * DEMAND_PREMIUM_FRAC).round() as u32;
        raw.max(1)
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEASONS: [Season; 4] = [Season::Spring, Season::Summer, Season::Autumn, Season::Winter];

    #[test]
    fn every_season_has_a_demand_variety() {
        // 每一季都有明確的搶手品種（前端 HUD 標示、收成溢價才有依據）。確定性、永不 panic。
        for s in SEASONS {
            let v = demand_variety(s);
            // 落在已知品種集合內。
            assert!(CropVariety::ALL.contains(&v), "{:?} 的搶手品種須是已知品種", s);
        }
    }

    #[test]
    fn every_variety_demanded_across_year() {
        // 一輪四季裡，三個品種各至少當令一次——沒有哪個品種一年到頭被市集冷落。
        for want in CropVariety::ALL {
            assert!(
                SEASONS.iter().any(|&s| demand_variety(s) == want),
                "{} 一年內應至少當令一次",
                want.as_str()
            );
        }
    }

    #[test]
    fn bonus_is_positive_only_for_the_demanded_variety() {
        // 溢價只給「當季搶手品種」，其餘品種一律 0（純正向、永不倒扣）。
        for s in SEASONS {
            let hot = demand_variety(s);
            for v in CropVariety::ALL {
                let bonus = demand_bonus_ether(v, s);
                if v == hot {
                    assert!(bonus >= 1, "{} 在其搶手季 {:?} 溢價須 ≥1（看得見）", v.as_str(), s);
                } else {
                    assert_eq!(bonus, 0, "{} 非 {:?} 搶手品種，溢價須為 0", v.as_str(), s);
                }
            }
        }
    }

    #[test]
    fn bonus_is_half_of_base_rounded() {
        // 溢價＝基礎收成乙太的半份（四捨五入）：速生菜 2→1、主食穀 3→2、乙太瓜 5→3。
        // 直接以各品種的搶手季驗算，鎖死「半份」的平衡承諾。
        assert_eq!(demand_bonus_ether(CropVariety::Sprout, Season::Summer), 1);
        assert_eq!(demand_bonus_ether(CropVariety::Staple, Season::Spring), 2);
        assert_eq!(demand_bonus_ether(CropVariety::Etherbloom, Season::Autumn), 3);
    }

    #[test]
    fn demand_season_is_off_peak_for_picky_varieties() {
        // 設計不變式：挑季節的品種（有旺季者），其搶手季不是它的旺季——
        // 「賣得俏的季節」與「長得快的季節」刻意錯開，才有「種得快 vs 賣得貴」的取捨。
        for v in [CropVariety::Sprout, CropVariety::Etherbloom] {
            let peak = v.peak_season().expect("速生菜／乙太瓜有旺季");
            // 找出它的搶手季（這批排程裡兩者各只有一個搶手季）。
            let hot_season = SEASONS.iter().find(|&&s| demand_variety(s) == v);
            if let Some(&hs) = hot_season {
                assert_ne!(hs, peak, "{} 的搶手季不應正是它的旺季（要有逆季取捨）", v.as_str());
            }
        }
    }
}
