//! 旅人手帳（ROADMAP 415）——把玩家「永久累積」的成長維度統整成一頁，
//! 並算出「差一點就達成」的下一個里程碑，給回訪一個明確的「下一步往哪走」。
//!
//! 設計鐵律：
//! - **純邏輯、確定性、可測**：零 IO、零鎖、零 LLM、零持久化（只讀既有持久化狀態算出視圖）。
//! - 只統整「**永久累積**」的成長（等級、生態／探索／天象圖鑑、人氣）——回訪看得到長進才有意義；
//!   不納入會隨重啟／單趟歸零的暫態計數（如本趟遠遊、記憶體擊殺數），免得進度忽高忽低。
//! - 面向玩家字串走 wire key，在前端對照顯示（留 i18n 空間）。

/// 各成長軌跡的 wire key（前端據此對照標籤／圖示，留 i18n 空間）。
pub const TRACK_LEVEL: &str = "level";
pub const TRACK_ECO: &str = "eco";
pub const TRACK_TERRAIN: &str = "terrain";
pub const TRACK_SKY: &str = "sky";
pub const TRACK_CHEERS: &str = "cheers";

/// 各軌跡的里程碑階梯（嚴格遞增）。達成最後一階即「圓滿」（無下一個目標）。
/// 圖鑑類上限對齊各 CATALOG 的 TOTAL（生態 19／探索 11／天象 7），湊滿即圓滿。
const LADDER_LEVEL: &[u32] = &[5, 10, 20, 30, 50];
const LADDER_ECO: &[u32] = &[5, 10, 15, 19];
const LADDER_TERRAIN: &[u32] = &[3, 6, 9, 11];
const LADDER_SKY: &[u32] = &[2, 4, 6, 7];
const LADDER_CHEERS: &[u32] = &[1, 10, 25, 50, 100];

/// 軌跡定義表。**次序即面板顯示次序，也是頭條平手時的 tie-break 次序**（確定性）。
const TRACKS: &[(&str, &[u32])] = &[
    (TRACK_LEVEL, LADDER_LEVEL),
    (TRACK_ECO, LADDER_ECO),
    (TRACK_TERRAIN, LADDER_TERRAIN),
    (TRACK_SKY, LADDER_SKY),
    (TRACK_CHEERS, LADDER_CHEERS),
];

/// 玩家當前的成長數據（呼叫端從 Player 既有持久化欄位填入）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct JourneyStats {
    /// 目前等級（exp / 100）。
    pub level: u32,
    /// 生態圖鑑已發現物種數（`field_guide::count`）。
    pub eco_seen: u32,
    /// 探索圖鑑已踏足奇景數（`terrain_atlas::count`）。
    pub terrain_seen: u32,
    /// 天象圖鑑已目睹天象數（`sky_codex::count`）。
    pub sky_seen: u32,
    /// 累積人氣（其他玩家的喝采數）。
    pub cheers: u32,
}

/// 一條成長軌跡的計算結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JourneyTrack {
    pub key: &'static str,
    /// 目前數值。
    pub current: u32,
    /// 下一個里程碑目標值；`None` = 已圓滿（達成最後一階）。
    pub next_goal: Option<u32>,
    /// 已達成的里程碑數（階位）。
    pub tier: u32,
}

/// 「差一點就達成」的頭條提示——所有未圓滿軌跡中，當前階段完成度最高的那條。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Headline {
    pub key: &'static str,
    /// 距離下一個里程碑還差多少。
    pub remaining: u32,
    /// 下一個里程碑目標值。
    pub goal: u32,
}

/// 旅人手帳整份報告。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JourneyReport {
    pub tracks: Vec<JourneyTrack>,
    pub headline: Option<Headline>,
}

/// 在階梯中找出 `current` 對應的（已達成階數, 下一目標）。
/// 階梯嚴格遞增；全階達成回 `(len, None)`（圓滿）。
fn evaluate(current: u32, ladder: &[u32]) -> (u32, Option<u32>) {
    let mut tier = 0u32;
    for &goal in ladder {
        if current >= goal {
            tier += 1;
        } else {
            return (tier, Some(goal));
        }
    }
    (tier, None)
}

/// 由玩家成長數據算出旅人手帳。確定性、純函式。
pub fn compute(stats: JourneyStats) -> JourneyReport {
    let values = [
        stats.level,
        stats.eco_seen,
        stats.terrain_seen,
        stats.sky_seen,
        stats.cheers,
    ];
    let mut tracks = Vec::with_capacity(TRACKS.len());
    // 頭條挑選：未圓滿軌跡中，**當前階段完成度（此階已前進 / 此階總跨距）最高**者；
    // 完成度相同取 TRACKS 次序最前（確定性）。以交叉相乘比較分數，不碰浮點等值。
    let mut best: Option<(u32 /*num*/, u32 /*den*/, &'static str, u32 /*remaining*/, u32 /*goal*/)> = None;
    for (i, (key, ladder)) in TRACKS.iter().enumerate() {
        let current = values[i];
        let (tier, next_goal) = evaluate(current, ladder);
        tracks.push(JourneyTrack { key, current, next_goal, tier });
        if let Some(goal) = next_goal {
            // 此階起點 = 上一個里程碑（tier 0 時為 0）。
            let prev = if tier == 0 { 0 } else { ladder[(tier - 1) as usize] };
            let num = current - prev; // 此階已前進
            let den = goal - prev; // 此階總跨距（階梯嚴格遞增 → den >= 1，不會除以零）
            let remaining = goal - current;
            let better = match best {
                None => true,
                // num/den > bn/bd ⇔ num*bd > bn*den（den、bd 皆 >= 1，安全）。
                Some((bn, bd, _, _, _)) => (num as u64) * (bd as u64) > (bn as u64) * (den as u64),
            };
            if better {
                best = Some((num, den, key, remaining, goal));
            }
        }
    }
    let headline = best.map(|(_, _, key, remaining, goal)| Headline { key, remaining, goal });
    JourneyReport { tracks, headline }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluate_basic_tiers_and_next_goal() {
        // 階梯 [5,10,20,30,50]
        assert_eq!(evaluate(0, LADDER_LEVEL), (0, Some(5)));
        assert_eq!(evaluate(5, LADDER_LEVEL), (1, Some(10)));
        assert_eq!(evaluate(7, LADDER_LEVEL), (1, Some(10)));
        assert_eq!(evaluate(20, LADDER_LEVEL), (3, Some(30)));
        // 達最後一階即圓滿、無下一目標。
        assert_eq!(evaluate(50, LADDER_LEVEL), (5, None));
        assert_eq!(evaluate(999, LADDER_LEVEL), (5, None));
    }

    #[test]
    fn report_has_one_track_per_dimension() {
        let r = compute(JourneyStats::default());
        assert_eq!(r.tracks.len(), TRACKS.len());
        let keys: Vec<_> = r.tracks.iter().map(|t| t.key).collect();
        assert_eq!(keys, vec![TRACK_LEVEL, TRACK_ECO, TRACK_TERRAIN, TRACK_SKY, TRACK_CHEERS]);
    }

    #[test]
    fn brand_new_player_headline_points_at_first_level_milestone() {
        // 全 0 的新玩家：所有軌跡完成度同為 0/den；tie-break 取次序最前＝等級。
        let r = compute(JourneyStats::default());
        let h = r.headline.expect("新玩家該有頭條");
        assert_eq!(h.key, TRACK_LEVEL);
        assert_eq!(h.goal, 5);
        assert_eq!(h.remaining, 5);
    }

    #[test]
    fn headline_picks_closest_to_completion() {
        // 等級 1（此階 1/5）、生態 9（此階 4/5，差 1 到 10）、其餘 0。
        // 生態完成度 4/5 最高 → 頭條為生態，差 1。
        let r = compute(JourneyStats { level: 1, eco_seen: 9, ..Default::default() });
        let h = r.headline.expect("該有頭條");
        assert_eq!(h.key, TRACK_ECO);
        assert_eq!(h.remaining, 1);
        assert_eq!(h.goal, 10);
    }

    #[test]
    fn fully_maxed_player_has_no_headline() {
        let r = compute(JourneyStats {
            level: 50,
            eco_seen: 19,
            terrain_seen: 11,
            sky_seen: 7,
            cheers: 100,
        });
        assert!(r.headline.is_none(), "全圓滿不該再有頭條提示");
        for t in &r.tracks {
            assert_eq!(t.next_goal, None, "{} 應圓滿", t.key);
        }
    }

    #[test]
    fn maxed_track_excluded_from_headline() {
        // 生態圓滿（19）但天象差 1 到 2 → 頭條落在未圓滿且完成度最高者。
        let r = compute(JourneyStats { eco_seen: 19, sky_seen: 1, ..Default::default() });
        let eco = r.tracks.iter().find(|t| t.key == TRACK_ECO).unwrap();
        assert_eq!(eco.next_goal, None);
        let h = r.headline.expect("仍有未圓滿軌跡");
        assert_ne!(h.key, TRACK_ECO, "圓滿軌跡不該成為頭條");
    }

    #[test]
    fn tier_counts_reached_milestones() {
        let r = compute(JourneyStats { cheers: 25, ..Default::default() });
        let cheers = r.tracks.iter().find(|t| t.key == TRACK_CHEERS).unwrap();
        // 階梯 [1,10,25,50,100]，25 達成前三階。
        assert_eq!(cheers.tier, 3);
        assert_eq!(cheers.next_goal, Some(50));
    }

    #[test]
    fn deterministic_same_input_same_output() {
        let s = JourneyStats { level: 12, eco_seen: 6, terrain_seen: 2, sky_seen: 3, cheers: 8 };
        assert_eq!(compute(s), compute(s));
    }
}
