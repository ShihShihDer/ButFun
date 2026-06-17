//! NPC 社交平衡漣漪（ROADMAP 365：湧現派系第四塊——讓「個體之間真正彼此牽動」）。
//!
//! 在 70（人際關係網）、71（派系湧現）、355（派系面板）、356（黃昏串門）、364（玩家居中和解）之上，
//! 補上至今缺的一塊：**關係網會「自己」演化**。
//!
//! 在此之前，NPC↔NPC 的好惡值只被三股外力牽動——世界事件（`apply_world_event`）、
//! 時間衰減（`tick_decay_all`）、玩家親自和解（364 `nudge_pair`）；NPC 彼此之間的「結構」
//! 從不互相影響。本模組讓社交網依**社會平衡理論**（structural balance：朋友的朋友會更親、
//! 朋友的敵人會漸疏）緩慢自我演化：每隔一段時間，引擎掃過關係網，對每一對 NPC 計算其
//! 「共同熟人」施加的社交壓力，把這對 NPC 往平衡方向輕推一步。於是玩家在 364 替兩位鎮民
//! 重修舊好的善意，會**自己漾開**——他們共同的朋友也漸生暖意，城鎮的人情結構真的活了起來。
//!
//! 設計鐵律：
//! - 純邏輯、零 LLM（漂移由查表＋集合運算算出，廣播文字由引擎純文字組）。
//! - 純記憶體模式、零 migration，重啟清零（漂移從當下關係值重新湧現）。
//! - **不製造極端**：漂移有軟上下界（暖不過 `DRIFT_WARM_CEIL`、冷不過 `DRIFT_COLD_FLOOR`），
//!   只把人帶到結盟/敵對門檻「之前」——真正越線成盟或結怨，仍要靠世界事件或玩家親自促成，
//!   避免社交傳染自己把全鎮推成同一塊、灌爆派系面板（療癒向、低風險）。
//! - 廣播帶每對冷卻，不洗頻；只報「回暖」的漣漪（療癒基調），冷卻由引擎時鐘管理。

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::npc_factions::{mutual_avg, npc_display_name};
use crate::npc_relations::NpcRelationsState;

const NEUTRAL: i32 = 50;
/// 一對 NPC 的雙向平均好惡值偏離中性超過此值，才算「明確的」朋友或敵人，
/// 足以對第三人施加社交壓力（避免中性帶的雜訊牽動全網）。
const BOND_DEADBAND: i32 = 10;
/// 每次社交 tick 對一對 NPC 施加的漂移步長（小而慢，遠小於 364 的 `RECONCILE_BUMP`=18，
/// 也足以蓋過每 5 分鐘 1 點的關係衰減，讓平衡方向真的會慢慢成形）。
const DRIFT_STEP: i32 = 2;
/// 漂移回暖的軟上界：低於派系結盟門檻（80）。漂移只把人帶到結盟「前」，
/// 越線成盟仍須世界事件或玩家親自促成。
const DRIFT_WARM_CEIL: i32 = 78;
/// 漂移降溫的軟下界：高於派系敵對門檻（22）。漂移不會自己把人推成公開敵對。
const DRIFT_COLD_FLOOR: i32 = 24;
/// 同一對 NPC 廣播「人情漸染」漣漪的最短間隔（秒），防洗頻。
pub const RIPPLE_ANNOUNCE_COOLDOWN_SECS: u64 = 240; // 4 分鐘

/// 七大 NPC ID（與 npc_relations.rs / npc_factions.rs 一致）。
const ALL_NPCS: &[&str] = &[
    "merchant",
    "workshop_npc",
    "bounty_npc",
    "expedition_npc",
    "procurement_npc",
    "farm_fair_npc",
    "village_chief",
];

/// 一筆社交漂移：平衡壓力要把這對 NPC 往某方向輕推一步。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SocialDrift {
    pub a: &'static str,
    pub b: &'static str,
    /// 雙向各加的好惡值增量（正＝回暖、負＝降溫）。已套用軟上下界，必為非零。
    pub delta: i32,
    /// 是否為回暖（`delta > 0`）。
    pub warming: bool,
    /// 套用漂移後這對的雙向平均好惡值（= 套用前平均 + delta，已 clamp）。供廣播文案顯示回暖到幾分。
    pub warmth_after: i32,
    /// 牽動這對的「共同熟人」——壓力最強的那位第三人 id（朋友的朋友/敵人）。供廣播文案點名來源。
    pub via: &'static str,
}

/// 取一對 NPC 的雙向平均好惡值相對中性的偏向（正＝暖、負＝冷）。
fn lean(relations: &NpcRelationsState, a: &str, b: &str) -> i32 {
    mutual_avg(relations, a, b) - NEUTRAL
}

/// 計算「此刻」關係網依社會平衡理論應發生的所有漂移（ROADMAP 365）。
///
/// 純函式、只讀關係、確定性可測：
/// - 只掃有序對（`ALL_NPCS` 次序，`a` 在 `b` 前），每對只算一次。
/// - 對每對 (a,b)，掃所有共同熟人 c：若 a–c 與 c–b **雙雙**是「明確」關係（|lean| > deadband），
///   則該 c 對 a–b 施加 `sign(lean(a,c)) * sign(lean(c,b))` 的壓力（朋友的朋友→正、朋友的敵人→負）。
/// - 壓力淨額 > 0 → 回暖一步；< 0 → 降溫一步；= 0 → 不漂移（不入列）。
/// - 套軟上下界：已達 `DRIFT_WARM_CEIL` 不再回暖、已達 `DRIFT_COLD_FLOOR` 不再降溫（回空步＝不入列）。
/// - `via` 取貢獻同號壓力中「兩段關係強度和」最大的共同熟人（平手取 `ALL_NPCS` 最前者，確定性）。
pub fn compute_drift(relations: &NpcRelationsState) -> Vec<SocialDrift> {
    let mut out: Vec<SocialDrift> = Vec::new();
    for i in 0..ALL_NPCS.len() {
        for j in (i + 1)..ALL_NPCS.len() {
            let a = ALL_NPCS[i];
            let b = ALL_NPCS[j];

            let mut pressure = 0i32;
            // 追蹤對「主導方向」貢獻最強的共同熟人，供廣播點名。
            let mut best_via: &'static str = "";
            let mut best_via_strength = -1i32;

            for &c in ALL_NPCS {
                if c == a || c == b {
                    continue;
                }
                let lac = lean(relations, a, c);
                let lcb = lean(relations, c, b);
                if lac.abs() <= BOND_DEADBAND || lcb.abs() <= BOND_DEADBAND {
                    continue; // c 與其中一方關係不明確，不施壓
                }
                let contribution = lac.signum() * lcb.signum(); // +1 或 -1
                pressure += contribution;
            }

            if pressure == 0 {
                continue;
            }
            let warming = pressure > 0;

            // 軟上下界：已在極端帶就不再往同方向推。
            let avg_before = mutual_avg(relations, a, b);
            let raw = if warming { DRIFT_STEP } else { -DRIFT_STEP };
            let delta = if warming {
                (DRIFT_WARM_CEIL - avg_before).clamp(0, raw)
            } else {
                // raw 為負：clamp 到 [raw, 0]，但不可低於下界
                (DRIFT_COLD_FLOOR - avg_before).clamp(raw, 0)
            };
            if delta == 0 {
                continue; // 已抵軟界，本輪不漂移
            }

            // 再掃一次，挑出與主導方向同號、強度最大的共同熟人作為 via。
            let want_sign = if warming { 1 } else { -1 };
            for &c in ALL_NPCS {
                if c == a || c == b {
                    continue;
                }
                let lac = lean(relations, a, c);
                let lcb = lean(relations, c, b);
                if lac.abs() <= BOND_DEADBAND || lcb.abs() <= BOND_DEADBAND {
                    continue;
                }
                if lac.signum() * lcb.signum() != want_sign {
                    continue;
                }
                let strength = lac.abs() + lcb.abs();
                if strength > best_via_strength {
                    best_via_strength = strength;
                    best_via = c;
                }
            }

            out.push(SocialDrift {
                a,
                b,
                delta,
                warming,
                warmth_after: (avg_before + delta).clamp(0, 100),
                via: best_via,
            });
        }
    }
    out
}

/// 追蹤已廣播的漣漪，防冷卻期內重複刷頻（記憶體模式，重啟清零）。
#[derive(Default)]
pub struct SocialDynamicsState {
    /// (字母序較小者, 另一個) → 上次廣播時間。
    last_announced: HashMap<(&'static str, &'static str), Instant>,
}

impl SocialDynamicsState {
    pub fn new() -> Self {
        Self::default()
    }

    /// 從本輪漂移中挑「最值得一報」的回暖漣漪（療癒基調：只報回暖、不報降溫），
    /// 套每對冷卻後，回傳一行世界頻道廣播文字；無可報時回 `None`。
    ///
    /// 「最值得一報」＝回暖步長最大者（平手取 `compute_drift` 的確定性次序最前者）。
    /// 不修改關係，只記錄冷卻時鐘。
    pub fn pick_announcement(&mut self, drifts: &[SocialDrift]) -> Option<String> {
        let now = Instant::now();
        let cooldown = Duration::from_secs(RIPPLE_ANNOUNCE_COOLDOWN_SECS);
        let pick = drifts
            .iter()
            .filter(|d| d.warming)
            .find(|d| {
                let key = pair_key(d.a, d.b);
                match self.last_announced.get(&key) {
                    None => true,
                    Some(t) => now.duration_since(*t) >= cooldown,
                }
            })?;
        self.last_announced.insert(pair_key(pick.a, pick.b), now);
        Some(ripple_line(pick))
    }
}

/// 有序對鍵（字母序較小者在前），與冷卻表一致。
fn pair_key(a: &'static str, b: &'static str) -> (&'static str, &'static str) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

/// 回暖漣漪的世界頻道廣播文字（面向玩家字串集中於此，留 i18n 空間）。
pub fn ripple_line(d: &SocialDrift) -> String {
    let name_a = npc_display_name(d.a);
    let name_b = npc_display_name(d.b);
    if d.via.is_empty() {
        format!(
            "🌿 [村落人情] {} 與 {} 在日常相處間漸生暖意，情誼回暖到 {}/100。",
            name_a, name_b, d.warmth_after
        )
    } else {
        let via_name = npc_display_name(d.via);
        format!(
            "🌿 [村落人情] 受與 {} 共同的情誼牽動，{} 與 {} 也漸生暖意，回暖到 {}/100。",
            via_name, name_a, name_b, d.warmth_after
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 構造一個全中性的關係網，方便單獨擺布特定三角關係做測試。
    fn neutral_state() -> NpcRelationsState {
        let mut s = NpcRelationsState::default();
        for &a in ALL_NPCS {
            for &b in ALL_NPCS {
                if a != b {
                    s.set_pair_for_test(a, b, NEUTRAL, NEUTRAL);
                }
            }
        }
        s
    }

    #[test]
    fn neutral_world_yields_no_drift() {
        // 全中性 → 無人施壓 → 零漂移。
        let s = neutral_state();
        assert!(compute_drift(&s).is_empty(), "全中性世界不應有任何社交漂移");
    }

    #[test]
    fn friend_of_friend_warms() {
        // a 與 c 親、c 與 b 親 → 平衡理論：a 與 b 應回暖。
        let mut s = neutral_state();
        // merchant–village_chief 親、village_chief–workshop 親 → merchant–workshop 應暖
        s.set_pair_for_test("merchant", "village_chief", 75, 75);
        s.set_pair_for_test("village_chief", "workshop_npc", 75, 75);
        let drifts = compute_drift(&s);
        let mw = drifts
            .iter()
            .find(|d| pair_key(d.a, d.b) == pair_key("merchant", "workshop_npc"))
            .expect("商人–工匠應因共同好友里長而回暖");
        assert!(mw.warming && mw.delta > 0, "應為回暖、正增量");
        assert_eq!(mw.via, "village_chief", "牽線者應為共同好友里長");
    }

    #[test]
    fn friend_of_enemy_cools() {
        // a 與 c 親、c 與 b 仇 → a 與 b 應降溫。
        let mut s = neutral_state();
        s.set_pair_for_test("merchant", "village_chief", 80, 80); // 親
        s.set_pair_for_test("village_chief", "bounty_npc", 15, 15); // 仇
        let drifts = compute_drift(&s);
        let mb = drifts
            .iter()
            .find(|d| pair_key(d.a, d.b) == pair_key("merchant", "bounty_npc"))
            .expect("商人–獵手應因里長的好惡而降溫");
        assert!(!mb.warming && mb.delta < 0, "應為降溫、負增量");
    }

    #[test]
    fn warm_ceiling_caps_drift() {
        // 已在軟上界附近，回暖步長被夾到不越界。
        let mut s = neutral_state();
        s.set_pair_for_test("merchant", "village_chief", 75, 75);
        s.set_pair_for_test("village_chief", "workshop_npc", 75, 75);
        // 把 merchant–workshop 預先拉到剛好軟上界
        s.set_pair_for_test("merchant", "workshop_npc", DRIFT_WARM_CEIL, DRIFT_WARM_CEIL);
        let drifts = compute_drift(&s);
        let mw = drifts
            .iter()
            .find(|d| pair_key(d.a, d.b) == pair_key("merchant", "workshop_npc"));
        assert!(mw.is_none(), "已達軟上界的回暖對不應再漂移（不入列）");
    }

    #[test]
    fn drift_never_crosses_alliance_threshold_alone() {
        // 反覆套用漂移，回暖永遠停在軟上界（78）以下，不會自己造出結盟（≥80）。
        let mut s = neutral_state();
        s.set_pair_for_test("merchant", "village_chief", 78, 78);
        s.set_pair_for_test("village_chief", "workshop_npc", 78, 78);
        for _ in 0..50 {
            let drifts = compute_drift(&s);
            for d in &drifts {
                s.nudge_pair(d.a, d.b, d.delta);
            }
        }
        for i in 0..ALL_NPCS.len() {
            for j in (i + 1)..ALL_NPCS.len() {
                let avg = mutual_avg(&s, ALL_NPCS[i], ALL_NPCS[j]);
                assert!(avg <= DRIFT_WARM_CEIL, "漂移不應把任何對推過軟上界，avg={avg}");
            }
        }
    }

    #[test]
    fn deterministic_order() {
        // 同一輸入兩次計算結果完全相同（不依賴 HashMap 迭代順序）。
        let mut s = neutral_state();
        s.set_pair_for_test("merchant", "village_chief", 75, 75);
        s.set_pair_for_test("village_chief", "workshop_npc", 75, 75);
        s.set_pair_for_test("bounty_npc", "expedition_npc", 80, 80);
        s.set_pair_for_test("expedition_npc", "farm_fair_npc", 80, 80);
        let d1 = compute_drift(&s);
        let d2 = compute_drift(&s);
        assert_eq!(d1, d2, "純函式應確定可重現");
    }

    #[test]
    fn deadband_blocks_weak_bonds() {
        // c 與兩方都只是「微暖」（落在 deadband 內）→ 不施壓。
        let mut s = neutral_state();
        s.set_pair_for_test("merchant", "village_chief", 58, 58); // lean +8 < deadband 10
        s.set_pair_for_test("village_chief", "workshop_npc", 58, 58);
        let drifts = compute_drift(&s);
        assert!(
            drifts
                .iter()
                .all(|d| pair_key(d.a, d.b) != pair_key("merchant", "workshop_npc")),
            "弱關係（落在 deadband 內）不應牽動第三方"
        );
    }

    #[test]
    fn ripple_line_warming_mentions_names_and_warmth() {
        let d = SocialDrift {
            a: "merchant",
            b: "workshop_npc",
            delta: 2,
            warming: true,
            warmth_after: 64,
            via: "village_chief",
        };
        let line = ripple_line(&d);
        assert!(line.contains("🌿"), "應含人情漣漪符號");
        assert!(line.contains("薇拉") && line.contains("鐸恩"), "應點名兩位當事 NPC");
        assert!(line.contains("凱爾") || line.contains("里長"), "應點名牽線的共同好友");
        assert!(line.contains("64"), "應顯示回暖後的好惡值");
    }

    #[test]
    fn pick_announcement_only_warming_and_cooldown() {
        let mut st = SocialDynamicsState::new();
        let warming = SocialDrift {
            a: "merchant",
            b: "workshop_npc",
            delta: 2,
            warming: true,
            warmth_after: 64,
            via: "village_chief",
        };
        let cooling = SocialDrift {
            a: "bounty_npc",
            b: "farm_fair_npc",
            delta: -2,
            warming: false,
            warmth_after: 38,
            via: "village_chief",
        };
        // 第一次：回暖那筆應被選中（降溫的略過）。
        let first = st.pick_announcement(&[cooling, warming]);
        assert!(first.is_some(), "應從回暖漂移中選一則廣播");
        assert!(first.unwrap().contains("薇拉"), "選中的應是回暖那筆");
        // 冷卻期內同一對不應再報。
        let second = st.pick_announcement(&[warming]);
        assert!(second.is_none(), "冷卻期內同一對不應重複廣播");
    }

    #[test]
    fn pick_announcement_none_when_all_cooling() {
        let mut st = SocialDynamicsState::new();
        let cooling = SocialDrift {
            a: "bounty_npc",
            b: "farm_fair_npc",
            delta: -2,
            warming: false,
            warmth_after: 38,
            via: "village_chief",
        };
        assert!(st.pick_announcement(&[cooling]).is_none(), "全是降溫時不廣播（療癒基調）");
    }
}
