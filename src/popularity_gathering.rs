//! 人氣聚會（ROADMAP 342）——當一名高人氣玩家（341 攢到「受歡迎」以上）身邊聚起好幾位
//! 玩家，伺服器偵測到一場「聚會」在他周圍湧現：他腳下亮起一圈發光的人氣聚會圈、世界頻道
//! 宣告「在 XX 周圍聚起了一場熱鬧的聚會」，全世界看得見人潮往受歡迎的人身邊靠攏。人潮散去，
//! 聚會自然落幕。這是多人互動弧第五拍——341 把人氣沉澱成名牌身份，342 讓那份人氣**長出後果**：
//! 受歡迎的人會真的吸引人潮、聚成一個看得見的社交節點。
//!
//! 多人互動弧層層遞進：338 單向 → 339 雙人 → 340 群體同步 → 341 留下印記（人氣身份）→
//! 342 人氣湧現後果（聚會節點）。
//!
//! 設計取捨：
//! - **純偵測 ＋ 純生命週期、零持久化**：`detect` 純函式吃一串玩家快照、吐當下成立的聚會；
//!   `GatheringState::reconcile` 純狀態機把「當下聚會集合」對映成 Started/Ended 事件（含散場
//!   緩衝防閃爍），兩者皆可獨立測。狀態純記憶體、重啟清零、零 migration。
//! - **人氣解鎖聚會**：只有人氣 ≥ `HOST_MIN_CHEERS`（＝ 341「受歡迎」階門檻 10）的玩家才當得起
//!   主人——把 341 攢到的人氣變成「能吸引聚會」的實際後果。
//! - **同區、室外、夠近**：只有同一星球、同在室外、距主人 ≤ `GATHER_RANGE` 的玩家算賓客
//!   （室內外空間不同、不同星球不互湊；接線層用 `zone` 字串隔開）。
//! - **確定可重現**：依「人氣高→低、同人氣 id 小→大」挑主人、貪婪聚團、每人至多屬一場聚會，
//!   結果不依賴 HashMap 迭代順序，可測。
//! - **零平衡風險**：純社交節點，不送任何物品／乙太／戰力——「獲得感」來自人潮往你身邊靠攏、
//!   被全世界看見這件事本身。
//! - **i18n**：對外字串（聚會宣告）集中前端，本模組只吐主人 id／賓客數。

use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// 主人最低人氣門檻（＝ `player_cheer` 的「受歡迎」階門檻 10）——人氣攢到「受歡迎」才吸引得起聚會。
/// 刻意對齊 341 的最低階：聚會是「受歡迎」這枚徽記長出的第一個看得見的後果。
pub const HOST_MIN_CHEERS: u64 = 10;

/// 賓客需落在主人這個半徑內（像素，世界座標）才算「聚到他身邊」。比共鳴（340，110px）寬些——
/// 一場聚會是更鬆散的一群人圍在受歡迎的人附近，不必擠成一團。
pub const GATHER_RANGE: f32 = 200.0;

/// 構成一場聚會、主人身邊需要的最少賓客數（**不含**主人本人）。三人以上圍著才算「聚會」。
pub const GATHER_MIN_GUESTS: usize = 3;

/// 聚會「散場緩衝」幀數（≈4s，TICK_HZ=30）——人潮短暫離開、或主人走動讓某幀剛好不成局時，
/// 不立刻判散場；連續 `GRACE_TICKS` 幀都不再成局才正式落幕，避免聚會圈在邊界一閃一閃。
/// 隨 TICK_HZ 調整：30Hz × 4s = 120 幀；若再調 TICK_HZ，此常數需同步更新為 TICK_HZ * 4。
pub const GRACE_TICKS: u16 = 120;

/// 一名可能參與聚會的玩家快照。`zone` ＝ 同區判定鍵（如星球名）；`cheers` ＝ 累積人氣
/// （決定能否當主人）。座標為世界像素座標。
#[derive(Debug, Clone)]
pub struct Attendee {
    pub id: Uuid,
    pub zone: String,
    pub x: f32,
    pub y: f32,
    pub cheers: u64,
}

/// 一場當下成立的聚會。`host` ＝ 聚會主人（人潮中心的受歡迎玩家）；`guests` ＝ 圍在他身邊的賓客數。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Party {
    pub host: Uuid,
    pub guests: u32,
}

/// 從一串玩家快照中偵測當下成立的聚會。
///
/// 規則：
/// - 候選主人 ＝ 人氣 ≥ `HOST_MIN_CHEERS` 者；依「人氣高→低、同人氣取 id 小者」挑主人，使
///   結果**確定可重現**（不依賴輸入順序／HashMap 迭代順序）。
/// - 主人身邊蒐集「同 `zone`、距主人 ≤ `GATHER_RANGE`、尚未屬於任何聚會」的賓客；賓客數
///   ≥ `GATHER_MIN_GUESTS` 才成局。成局後主人與這些賓客都標記為已入局（**每人至多屬一場聚會**），
///   人潮被人氣更高的主人先「拿走」。
///
/// 多數時刻沒人攢到人氣門檻→第一輪就跳光、近乎零成本。
pub fn detect(attendees: &[Attendee]) -> Vec<Party> {
    // 依人氣高→低、同人氣 id 小→大排序候選主人順位。
    let mut order: Vec<usize> = (0..attendees.len()).collect();
    order.sort_by(|&a, &b| {
        attendees[b]
            .cheers
            .cmp(&attendees[a].cheers)
            .then(attendees[a].id.cmp(&attendees[b].id))
    });

    let mut taken = vec![false; attendees.len()];
    let mut out: Vec<Party> = Vec::new();

    for &hi in &order {
        let h = &attendees[hi];
        // order 由人氣高→低排列：一旦掃到人氣不足門檻者，後面只會更低，直接收工。
        if h.cheers < HOST_MIN_CHEERS {
            break;
        }
        if taken[hi] {
            continue;
        }
        // 蒐集同區、半徑內、未入局的賓客。
        let mut guests: Vec<usize> = Vec::new();
        for (gi, g) in attendees.iter().enumerate() {
            if gi == hi || taken[gi] || g.zone != h.zone {
                continue;
            }
            let dx = g.x - h.x;
            let dy = g.y - h.y;
            if dx * dx + dy * dy <= GATHER_RANGE * GATHER_RANGE {
                guests.push(gi);
            }
        }
        if guests.len() >= GATHER_MIN_GUESTS {
            taken[hi] = true;
            for &gi in &guests {
                taken[gi] = true;
            }
            out.push(Party { host: h.id, guests: guests.len() as u32 });
        }
    }

    out
}

/// 由 `reconcile` 回傳、交給 `game.rs` 廣播的聚會生命週期事件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GatheringEvent {
    /// 一場聚會剛湧現。`host` ＝ 主人 id；`guests` ＝ 成局當下的賓客數。
    Started { host: Uuid, guests: u32 },
    /// 一場聚會落幕（人潮散去、緩衝耗盡）。`host` ＝ 原主人 id。
    Ended { host: Uuid },
}

/// 聚會生命週期狀態機（純記憶體、零持久化）。把「每幀偵測到的聚會集合」對映成 Started/Ended
/// 事件，並以散場緩衝避免邊界閃爍。
#[derive(Debug, Default)]
pub struct GatheringState {
    /// 進行中的聚會：主人 id → 散場緩衝剩餘幀數。當幀重新成局則回滿 `GRACE_TICKS`；
    /// 不成局則每幀遞減，歸零才正式落幕。
    active: HashMap<Uuid, u16>,
}

impl GatheringState {
    pub fn new() -> Self {
        Self { active: HashMap::new() }
    }

    /// 目前進行中（含散場緩衝期）的聚會場數——供測試／觀測用。
    pub fn active_count(&self) -> usize {
        self.active.len()
    }

    /// 把「當下這幀成立的聚會」對映成事件，並維護散場緩衝：
    /// - 當下成局、之前不在表 → `Started`、加入表（緩衝回滿）。
    /// - 當下成局、之前已在表 → 沿用、緩衝回滿（**不重複** `Started`）。
    /// - 之前在表、當下不成局 → 緩衝遞減；歸零才移除＋`Ended`（避免人潮一進一出就閃爍）。
    ///
    /// `Ended` 事件依主人 id 排序回傳，使輸出確定可重現（不依賴 HashMap 迭代順序）。
    pub fn reconcile(&mut self, current: &[Party]) -> Vec<GatheringEvent> {
        let mut events: Vec<GatheringEvent> = Vec::new();
        let mut current_hosts: HashSet<Uuid> = HashSet::new();

        // 當下成局者：新的發 Started，舊的把緩衝回滿。
        for p in current {
            current_hosts.insert(p.host);
            if self.active.insert(p.host, GRACE_TICKS).is_none() {
                events.push(GatheringEvent::Started { host: p.host, guests: p.guests });
            }
        }

        // 進行中但這幀沒成局者：緩衝遞減，歸零落幕。
        let mut ended: Vec<Uuid> = Vec::new();
        for (host, grace) in self.active.iter_mut() {
            if current_hosts.contains(host) {
                continue;
            }
            *grace = grace.saturating_sub(1);
            if *grace == 0 {
                ended.push(*host);
            }
        }
        ended.sort();
        for host in ended {
            self.active.remove(&host);
            events.push(GatheringEvent::Ended { host });
        }

        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn att(id: u128, zone: &str, x: f32, y: f32, cheers: u64) -> Attendee {
        Attendee { id: Uuid::from_u128(id), zone: zone.to_string(), x, y, cheers }
    }

    // ── detect ──────────────────────────────────────────────────────────

    #[test]
    fn empty_world_no_party() {
        assert!(detect(&[]).is_empty());
    }

    #[test]
    fn popular_host_with_enough_guests_forms_party() {
        // 主人(id1, 人氣 12) ＋ 三名近處賓客 → 成局，賓客數 3。
        let a = vec![
            att(1, "home", 0.0, 0.0, 12),
            att(2, "home", 10.0, 0.0, 0),
            att(3, "home", -10.0, 5.0, 0),
            att(4, "home", 0.0, 20.0, 0),
        ];
        let parties = detect(&a);
        assert_eq!(parties.len(), 1);
        assert_eq!(parties[0].host, Uuid::from_u128(1));
        assert_eq!(parties[0].guests, 3);
    }

    #[test]
    fn too_few_guests_no_party() {
        // 主人 ＋ 僅兩名賓客（未達 GATHER_MIN_GUESTS=3）→ 不成局。
        let a = vec![
            att(1, "home", 0.0, 0.0, 50),
            att(2, "home", 10.0, 0.0, 0),
            att(3, "home", -10.0, 0.0, 0),
        ];
        assert!(detect(&a).is_empty());
    }

    #[test]
    fn unpopular_host_no_party_even_with_crowd() {
        // 中心玩家人氣不足門檻(9 < 10)，旁邊三人也都不夠 → 沒人當得起主人、不成局。
        let a = vec![
            att(1, "home", 0.0, 0.0, 9),
            att(2, "home", 10.0, 0.0, 5),
            att(3, "home", -10.0, 0.0, 0),
            att(4, "home", 0.0, 10.0, 3),
        ];
        assert!(detect(&a).is_empty());
    }

    #[test]
    fn guests_outside_range_do_not_count() {
        // 主人夠人氣，但三名玩家都散在半徑外 → 湊不到賓客、不成局。
        let far = GATHER_RANGE + 1.0;
        let a = vec![
            att(1, "home", 0.0, 0.0, 30),
            att(2, "home", far, 0.0, 0),
            att(3, "home", -far, 0.0, 0),
            att(4, "home", 0.0, far, 0),
        ];
        assert!(detect(&a).is_empty());
    }

    #[test]
    fn just_inside_range_counts_just_outside_does_not() {
        let inside = vec![
            att(1, "home", 0.0, 0.0, 20),
            att(2, "home", GATHER_RANGE - 0.1, 0.0, 0),
            att(3, "home", 0.0, GATHER_RANGE - 0.1, 0),
            att(4, "home", -(GATHER_RANGE - 0.1), 0.0, 0),
        ];
        assert_eq!(detect(&inside).len(), 1, "三名賓客都剛好在半徑內 → 成局");
        let outside = vec![
            att(1, "home", 0.0, 0.0, 20),
            att(2, "home", GATHER_RANGE + 0.1, 0.0, 0),
            att(3, "home", 0.0, GATHER_RANGE + 0.1, 0),
            att(4, "home", -(GATHER_RANGE + 0.1), 0.0, 0),
        ];
        assert!(detect(&outside).is_empty(), "三名賓客都剛好超出半徑 → 不成局");
    }

    #[test]
    fn different_zones_do_not_gather() {
        // 主人在 home、三名玩家在 verdant（同座標但不同星球）→ 湊不起來。
        let a = vec![
            att(1, "home", 0.0, 0.0, 40),
            att(2, "verdant", 5.0, 0.0, 0),
            att(3, "verdant", -5.0, 0.0, 0),
            att(4, "verdant", 0.0, 5.0, 0),
        ];
        assert!(detect(&a).is_empty());
    }

    #[test]
    fn highest_popularity_hosts_the_crowd() {
        // 兩名夠人氣者(id1=12、id5=99)都在同一群人裡：人氣高的 id5 先當主人、拿走人潮，
        // id1 反而成了賓客 → 只有一場聚會、主人是 id5。
        let a = vec![
            att(1, "home", 0.0, 0.0, 12),
            att(5, "home", 20.0, 0.0, 99),
            att(2, "home", 10.0, 0.0, 0),
            att(3, "home", 30.0, 0.0, 0),
            att(4, "home", 15.0, 10.0, 0),
        ];
        let parties = detect(&a);
        assert_eq!(parties.len(), 1);
        assert_eq!(parties[0].host, Uuid::from_u128(5), "人氣最高者當主人");
    }

    #[test]
    fn each_attendee_in_at_most_one_party() {
        // 兩團相距很遠、各有一名主人＋三名賓客 → 兩場聚會、每人至多屬一場。
        let a = vec![
            att(1, "home", 0.0, 0.0, 20),
            att(2, "home", 10.0, 0.0, 0),
            att(3, "home", 20.0, 0.0, 0),
            att(4, "home", 5.0, 10.0, 0),
            att(5, "home", 5000.0, 0.0, 30),
            att(6, "home", 5010.0, 0.0, 0),
            att(7, "home", 5020.0, 0.0, 0),
            att(8, "home", 5005.0, 10.0, 0),
        ];
        let parties = detect(&a);
        assert_eq!(parties.len(), 2);
        let mut seen = HashSet::new();
        // 主人不重複、且兩場各自獨立。
        for p in &parties {
            assert!(seen.insert(p.host));
            assert_eq!(p.guests, 3);
        }
    }

    #[test]
    fn detect_is_deterministic_regardless_of_order() {
        let a = vec![
            att(3, "home", 20.0, 0.0, 0),
            att(1, "home", 0.0, 0.0, 25),
            att(4, "home", 5.0, 10.0, 0),
            att(2, "home", 10.0, 0.0, 0),
        ];
        let mut b = a.clone();
        b.reverse();
        assert_eq!(detect(&a), detect(&b), "聚會偵測不該因輸入順序而變");
    }

    // ── reconcile 生命週期 ─────────────────────────────────────────────

    #[test]
    fn reconcile_emits_started_once() {
        let mut st = GatheringState::new();
        let party = vec![Party { host: Uuid::from_u128(1), guests: 3 }];
        let ev1 = st.reconcile(&party);
        assert_eq!(ev1, vec![GatheringEvent::Started { host: Uuid::from_u128(1), guests: 3 }]);
        // 同一場聚會持續成局：不該再發 Started。
        let ev2 = st.reconcile(&party);
        assert!(ev2.is_empty(), "聚會持續中不重複宣告");
        assert_eq!(st.active_count(), 1);
    }

    #[test]
    fn reconcile_ends_after_grace_expires() {
        let mut st = GatheringState::new();
        let party = vec![Party { host: Uuid::from_u128(1), guests: 3 }];
        st.reconcile(&party); // Started
        // 之後都不成局：GRACE_TICKS-1 幀內不該落幕（緩衝撐著）。
        for _ in 0..(GRACE_TICKS - 1) {
            assert!(st.reconcile(&[]).is_empty(), "緩衝期內聚會仍在");
        }
        // 緩衝耗盡的那幀落幕。
        let ev = st.reconcile(&[]);
        assert_eq!(ev, vec![GatheringEvent::Ended { host: Uuid::from_u128(1) }]);
        assert_eq!(st.active_count(), 0);
    }

    #[test]
    fn reconcile_refreshes_grace_on_reappear() {
        let mut st = GatheringState::new();
        let party = vec![Party { host: Uuid::from_u128(1), guests: 3 }];
        st.reconcile(&party); // Started
        // 緩衝耗到剩一格前夕，人潮又回來 → 緩衝回滿、不落幕、不重複 Started。
        for _ in 0..(GRACE_TICKS - 1) {
            st.reconcile(&[]);
        }
        let ev = st.reconcile(&party);
        assert!(ev.is_empty(), "重新成局：不落幕、不重複 Started");
        // 緩衝已回滿：再撐 GRACE_TICKS-1 幀仍不落幕。
        for _ in 0..(GRACE_TICKS - 1) {
            assert!(st.reconcile(&[]).is_empty());
        }
        assert_eq!(st.active_count(), 1);
    }

    #[test]
    fn reconcile_multiple_ended_are_sorted() {
        let mut st = GatheringState::new();
        let parties = vec![
            Party { host: Uuid::from_u128(9), guests: 3 },
            Party { host: Uuid::from_u128(2), guests: 4 },
        ];
        st.reconcile(&parties); // 兩場 Started
        // 兩場同時散：緩衝同步耗盡，Ended 依 id 排序回傳（2 在 9 前）。
        let mut last = Vec::new();
        for _ in 0..GRACE_TICKS {
            last = st.reconcile(&[]);
        }
        assert_eq!(
            last,
            vec![
                GatheringEvent::Ended { host: Uuid::from_u128(2) },
                GatheringEvent::Ended { host: Uuid::from_u128(9) },
            ],
        );
    }
}
