//! 鎮民派系成形——盟友連成「陣營」、湧現出帶頭的核心人物（ROADMAP 366：湧現派系第五塊）。
//!
//! 在 70（人際關係網）、71（派系湧現）、355（派系面板）、364（玩家居中和解）、365（社交平衡漣漪）
//! 之上，補上至今缺的一塊：**派系從「兩兩配對」長成「成群結派」的群體結構**。
//!
//! 在此之前，玩家只能看到 NPC↔NPC 一對一的結盟／敵對（355）。本模組讓引擎在「此刻」的關係網上，
//! 把彼此牽起一張連通友誼網的三位以上居民，認作一個**陣營**，並從中推舉出眾望所歸的**核心人物**
//! （figurehead）——這正是 365 讓關係網「自己」演化後，自然長出的形狀：派系第一次從點對點的線，
//! 聚成有成員、有頭面人物的塊。玩家在派系面板裡，第一次看見村落自發組織成幾個圈子、各有帶頭的人。
//!
//! 設計鐵律：
//! - 純邏輯、零 LLM（陣營由連通分量算出，廣播文字由引擎純文字組）。
//! - 純記憶體模式、零 migration，重啟清零（陣營從當下關係值重新湧現）。
//! - **不製造一黨獨大**：陣營只從真正越過結盟門檻（80）的關係長出；365 的社交漂移軟上界鎖在
//!   78 < 80，社交傳染自己推不成盟——真正越線成盟仍須世界事件或玩家親自促成，不會全鎮一夜成黨。
//! - 廣播帶冷卻、只在新陣營成形或成員增長時報一次，不洗頻。

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::npc_factions::{mutual_avg, npc_display_name};
use crate::npc_relations::NpcRelationsState;

/// 兩個 NPC 的雙向平均好惡值 ≥ 此閾值 → 視為一條「結盟邊」。
/// 鏡像 `npc_factions::ALLIANCE_THRESHOLD`（80），確保面板上「兩兩結盟」與「陣營」門檻一致。
const BLOC_ALLIANCE_THRESHOLD: i32 = 80;
/// 連通分量達此人數才算一個「陣營」。兩人的盟友 355 已逐對顯示，三人以上才是新的群體湧現。
const MIN_BLOC_SIZE: usize = 3;
/// 同一陣營（成員集合）廣播成形的最短間隔（秒），防洗頻。
pub const BLOC_ANNOUNCE_COOLDOWN_SECS: u64 = 900; // 15 分鐘

/// 七大 NPC ID（與 npc_relations.rs / npc_factions.rs / social_dynamics.rs 一致）。
const ALL_NPCS: &[&str] = &[
    "merchant",
    "workshop_npc",
    "bounty_npc",
    "expedition_npc",
    "procurement_npc",
    "farm_fair_npc",
    "village_chief",
];

/// 一個「此刻」湧現的陣營：一群彼此牽起連通結盟網的居民，附帶推舉出的核心人物。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bloc {
    /// 陣營成員 id（依 `ALL_NPCS` 次序排列，確定性）。
    pub members: Vec<&'static str>,
    /// 核心人物 id：對陣營內其他成員好惡總和最高者（最得人心、最居中）。
    pub figurehead: &'static str,
    /// 凝聚度：陣營內所有成員對之間雙向平均好惡值的平均（越高越鐵）。
    pub cohesion: i32,
}

/// 計算「此刻」關係網上所有湧現的陣營（ROADMAP 366）。
///
/// 純函式、只讀關係、確定性可測：
/// - 以雙向平均好惡值 ≥ `BLOC_ALLIANCE_THRESHOLD` 為無向邊，求連通分量（DFS）。
/// - 只收 `size >= MIN_BLOC_SIZE` 的分量為陣營（兩人盟友 355 已顯示）。
/// - 每個陣營推舉核心人物＝對陣營內其他成員好惡總和最高者，平手取 `ALL_NPCS` 最前者。
/// - 陣營排序：成員多者在前、凝聚高者次之、核心 `ALL_NPCS` 次序末（穩定可讀、確定性）。
pub fn compute_blocs(relations: &NpcRelationsState) -> Vec<Bloc> {
    let n = ALL_NPCS.len();

    // 鄰接：alliance[i][j] = 第 i、j 位 NPC 雙向平均好惡值是否達結盟門檻。
    let mut allied = vec![vec![false; n]; n];
    for i in 0..n {
        for j in (i + 1)..n {
            if mutual_avg(relations, ALL_NPCS[i], ALL_NPCS[j]) >= BLOC_ALLIANCE_THRESHOLD {
                allied[i][j] = true;
                allied[j][i] = true;
            }
        }
    }

    // 連通分量（DFS）。visited 以 ALL_NPCS 次序掃，確定性。
    let mut visited = vec![false; n];
    let mut blocs: Vec<Bloc> = Vec::new();
    for start in 0..n {
        if visited[start] {
            continue;
        }
        // 收集這個分量的成員索引。
        let mut comp: Vec<usize> = Vec::new();
        let mut stack = vec![start];
        visited[start] = true;
        while let Some(u) = stack.pop() {
            comp.push(u);
            for v in 0..n {
                if allied[u][v] && !visited[v] {
                    visited[v] = true;
                    stack.push(v);
                }
            }
        }
        if comp.len() < MIN_BLOC_SIZE {
            continue;
        }
        comp.sort_unstable(); // 依 ALL_NPCS 次序，確定性

        // 核心人物：對其他成員好惡總和最高者；平手取索引最前（ALL_NPCS 最前）。
        let mut figurehead_idx = comp[0];
        let mut best_sum = i32::MIN;
        for &m in &comp {
            let sum: i32 = comp
                .iter()
                .filter(|&&o| o != m)
                .map(|&o| mutual_avg(relations, ALL_NPCS[m], ALL_NPCS[o]))
                .sum();
            if sum > best_sum {
                best_sum = sum;
                figurehead_idx = m;
            }
        }

        // 凝聚度：成員兩兩雙向平均好惡的平均。
        let mut total = 0i32;
        let mut pairs = 0i32;
        for a in 0..comp.len() {
            for b in (a + 1)..comp.len() {
                total += mutual_avg(relations, ALL_NPCS[comp[a]], ALL_NPCS[comp[b]]);
                pairs += 1;
            }
        }
        let cohesion = if pairs > 0 { total / pairs } else { 0 };

        blocs.push(Bloc {
            members: comp.iter().map(|&i| ALL_NPCS[i]).collect(),
            figurehead: ALL_NPCS[figurehead_idx],
            cohesion,
        });
    }

    // 排序：成員多者在前、凝聚高者次之、核心次序末。
    blocs.sort_by(|x, y| {
        y.members
            .len()
            .cmp(&x.members.len())
            .then_with(|| y.cohesion.cmp(&x.cohesion))
            .then_with(|| npc_order(x.figurehead).cmp(&npc_order(y.figurehead)))
    });
    blocs
}

/// NPC id → `ALL_NPCS` 次序（找不到回大值，排在最後）。供確定性 tie-break。
fn npc_order(id: &str) -> usize {
    ALL_NPCS.iter().position(|&x| x == id).unwrap_or(usize::MAX)
}

/// 一次陣營成形事件（引擎偵測到後廣播到聊天頻道）。
#[derive(Debug, Clone)]
pub struct BlocEvent {
    pub members: Vec<&'static str>,
    pub figurehead: &'static str,
}

impl BlocEvent {
    /// 生成聊天廣播文字：由核心人物以「同進退」的口吻點名成員（療癒、正向）。
    pub fn announce_text(&self) -> String {
        let lead = npc_display_name(self.figurehead);
        let names: Vec<&str> = self
            .members
            .iter()
            .map(|&id| npc_display_name(id))
            .collect();
        format!(
            "🏛️ [村落陣營] 以 {} 為核心，{} 漸成一個圈子，往後同進退。",
            lead,
            names.join("、")
        )
    }
}

/// 追蹤已宣告的陣營，防止冷卻期內重複廣播（記憶體模式，重啟清零）。
#[derive(Default)]
pub struct TownBlocState {
    /// 成員集合鍵（依次序串接的 id）→ 上次宣告時間。成員增長會改變鍵，自然當作新陣營重報。
    last_announced: HashMap<String, Instant>,
}

impl TownBlocState {
    pub fn new() -> Self {
        Self::default()
    }

    /// 基於當前陣營快照，偵測新成形（或成員增長）且過冷卻的陣營，回傳應廣播的事件。
    ///
    /// 不修改 blocs，只讀。呼叫端在 tick 後呼叫，回傳的每個 `BlocEvent` 都應廣播到 tx_chat。
    pub fn detect_new(&mut self, blocs: &[Bloc]) -> Vec<BlocEvent> {
        let now = Instant::now();
        let cooldown = Duration::from_secs(BLOC_ANNOUNCE_COOLDOWN_SECS);
        let mut events = Vec::new();
        for b in blocs {
            let key = b.members.join("|");
            let should = match self.last_announced.get(&key) {
                None => true,
                Some(last) => now.duration_since(*last) >= cooldown,
            };
            if should {
                self.last_announced.insert(key, now);
                events.push(BlocEvent {
                    members: b.members.clone(),
                    figurehead: b.figurehead,
                });
            }
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::npc_relations::NpcRelationsState;

    /// 把一組 NPC 兩兩設成穩固結盟（雙向 90），方便構造陣營場景。
    fn ally_all(r: &mut NpcRelationsState, ids: &[&str]) {
        for i in 0..ids.len() {
            for j in (i + 1)..ids.len() {
                r.set_pair_for_test(ids[i], ids[j], 90, 90);
            }
        }
    }

    #[test]
    fn initial_state_has_no_bloc() {
        // 初始好惡值皆 < 80，無結盟邊 → 無陣營。
        let r = NpcRelationsState::new();
        assert!(compute_blocs(&r).is_empty(), "初始村民無人成盟，不應有陣營");
    }

    #[test]
    fn triangle_forms_one_bloc() {
        let mut r = NpcRelationsState::new();
        ally_all(&mut r, &["merchant", "workshop_npc", "bounty_npc"]);
        let blocs = compute_blocs(&r);
        assert_eq!(blocs.len(), 1, "三人互盟應形成恰一個陣營");
        assert_eq!(blocs[0].members.len(), 3);
    }

    #[test]
    fn two_allies_do_not_form_bloc() {
        // 只有兩人結盟（size < 3）不成陣營——那已由 355 兩兩面板顯示。
        let mut r = NpcRelationsState::new();
        ally_all(&mut r, &["merchant", "workshop_npc"]);
        assert!(compute_blocs(&r).is_empty(), "兩人結盟不應成陣營");
    }

    #[test]
    fn chain_of_four_is_connected_bloc() {
        // A-B-C-D 鏈式結盟（非全互盟）仍連通，應算一個四人陣營。
        let mut r = NpcRelationsState::new();
        r.set_pair_for_test("merchant", "workshop_npc", 90, 90);
        r.set_pair_for_test("workshop_npc", "bounty_npc", 90, 90);
        r.set_pair_for_test("bounty_npc", "expedition_npc", 90, 90);
        let blocs = compute_blocs(&r);
        assert_eq!(blocs.len(), 1, "連通鏈應算一個陣營");
        assert_eq!(blocs[0].members.len(), 4, "四人鏈皆連通");
    }

    #[test]
    fn figurehead_is_most_central_member() {
        // workshop_npc 與兩人皆 90，merchant/bounty 彼此僅 81（仍成盟但較弱）→ 核心應是 workshop_npc。
        let mut r = NpcRelationsState::new();
        r.set_pair_for_test("merchant", "workshop_npc", 90, 90);
        r.set_pair_for_test("workshop_npc", "bounty_npc", 90, 90);
        r.set_pair_for_test("merchant", "bounty_npc", 81, 81);
        let blocs = compute_blocs(&r);
        assert_eq!(blocs.len(), 1);
        assert_eq!(blocs[0].figurehead, "workshop_npc", "好惡總和最高者應為核心");
    }

    #[test]
    fn figurehead_tie_breaks_by_npc_order() {
        // 三人全 90，好惡總和相同 → 平手取 ALL_NPCS 最前者（merchant）。
        let mut r = NpcRelationsState::new();
        ally_all(&mut r, &["merchant", "workshop_npc", "bounty_npc"]);
        let blocs = compute_blocs(&r);
        assert_eq!(blocs[0].figurehead, "merchant", "平手應取次序最前");
    }

    #[test]
    fn members_sorted_deterministically() {
        let mut r = NpcRelationsState::new();
        ally_all(&mut r, &["bounty_npc", "merchant", "workshop_npc"]);
        let blocs = compute_blocs(&r);
        // 成員應依 ALL_NPCS 次序排列：merchant(0) < workshop_npc(1) < bounty_npc(2)。
        assert_eq!(blocs[0].members, vec!["merchant", "workshop_npc", "bounty_npc"]);
    }

    #[test]
    fn compute_is_reproducible() {
        let mut r = NpcRelationsState::new();
        ally_all(&mut r, &["merchant", "workshop_npc", "bounty_npc", "expedition_npc"]);
        assert_eq!(compute_blocs(&r), compute_blocs(&r), "純函式應可重現");
    }

    #[test]
    fn two_separate_blocs_sorted_by_size() {
        // 一個四人陣營 + 一個三人陣營 → 大的排前面。
        let mut r = NpcRelationsState::new();
        ally_all(&mut r, &["merchant", "workshop_npc", "bounty_npc", "expedition_npc"]);
        ally_all(&mut r, &["procurement_npc", "farm_fair_npc", "village_chief"]);
        let blocs = compute_blocs(&r);
        assert_eq!(blocs.len(), 2, "應有兩個獨立陣營");
        assert_eq!(blocs[0].members.len(), 4, "人多的陣營應排前");
        assert_eq!(blocs[1].members.len(), 3);
    }

    #[test]
    fn cohesion_in_range() {
        let mut r = NpcRelationsState::new();
        ally_all(&mut r, &["merchant", "workshop_npc", "bounty_npc"]);
        let blocs = compute_blocs(&r);
        assert!((0..=100).contains(&blocs[0].cohesion), "凝聚度應在 0~100");
        assert!(blocs[0].cohesion >= BLOC_ALLIANCE_THRESHOLD, "全 90 互盟凝聚應 ≥ 門檻");
    }

    #[test]
    fn announce_text_contains_figurehead_and_members() {
        let ev = BlocEvent {
            members: vec!["merchant", "workshop_npc", "bounty_npc"],
            figurehead: "workshop_npc",
        };
        let text = ev.announce_text();
        assert!(text.contains("🏛️"), "應含陣營 emoji");
        assert!(text.contains("鐸恩"), "應點名核心人物工匠鐸恩");
        assert!(text.contains("薇拉") && text.contains("蘭卡"), "應列出其他成員");
    }

    #[test]
    fn detect_new_announces_once_then_cooldown() {
        let mut r = NpcRelationsState::new();
        ally_all(&mut r, &["merchant", "workshop_npc", "bounty_npc"]);
        let blocs = compute_blocs(&r);
        let mut st = TownBlocState::new();
        let first = st.detect_new(&blocs);
        assert_eq!(first.len(), 1, "首次應廣播一則陣營成形");
        let second = st.detect_new(&blocs);
        assert!(second.is_empty(), "冷卻期內同一陣營不應重複廣播");
    }

    #[test]
    fn detect_new_reannounces_when_bloc_grows() {
        // 三人陣營成形 → 廣播；再加入第四人（成員集合變）→ 視為新陣營重報。
        let mut r = NpcRelationsState::new();
        ally_all(&mut r, &["merchant", "workshop_npc", "bounty_npc"]);
        let mut st = TownBlocState::new();
        let _ = st.detect_new(&compute_blocs(&r));
        ally_all(&mut r, &["merchant", "workshop_npc", "bounty_npc", "expedition_npc"]);
        let grown = st.detect_new(&compute_blocs(&r));
        assert_eq!(grown.len(), 1, "成員增長應重報一次");
        assert_eq!(grown[0].members.len(), 4);
    }
}
