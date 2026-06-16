//! 表情共鳴（ROADMAP 340）——當一群靠近的玩家在短時間內比出**同一個表情**，伺服器偵測到
//! 「共鳴」，在他們的中心迸出一道放大發光的共鳴特效。這是多人互動弧的第三拍。
//!
//! 多人互動弧的層層遞進：
//! - 338 表情：**單人單向**——一個人比、全服看得到（各比各的）。
//! - 339 擊掌：**雙人同步**——兩個真人各自出手、又站得夠近才「啪」地成對。
//! - 340 共鳴：**群體同步**——三人以上恰好同時比了同一個表情，天雷勾動地火、共鳴爆發。
//!   它完全長在 338 之上：沒有新的玩家指令，只是把大家本就在比的表情「湊在一起」放大成
//!   一個眾人共享的大場面——一群人同時揮手、同時歡呼、同時比愛心，世界替他們「轟」地放大這一刻。
//!
//! 設計取捨：
//! - **純偵測、零持久化**：玩家比表情時 `ws.rs` 在他身上點亮一個短暫的「最近表情」倒數
//!   （`recent_emote`，記憶體前置、不入快照、不持久化、零 migration）。偵測與廣播都在
//!   `game.rs` 每幀做：把當下「最近還在比同個表情、又靠得夠近」的玩家聚成一團，≥ `RESONANCE_MIN`
//!   人就迸共鳴。
//! - **純函式可測**：核心 `detect` 是純函式（吃一串「最近表情」回響、吐確定的共鳴），
//!   以「種子玩家＋半徑內同表情者」貪婪聚團、依 id 排序求確定可重現的結果。
//! - **同區才共鳴**：只有**同一星球、且同在室外**的玩家才湊得起來（接線層用 `zone` 字串隔開
//!   不同星球、室內外）。
//! - **零平衡風險**：共鳴純社交場面，不送任何物品／乙太／戰力——「獲得感」來自眾人同框的大場面本身。
//! - **i18n**：對外字串（共鳴播報）集中前端，本模組只吐索引／座標／人數，glyph 由 `player_emote`
//!   依索引查出、emoji 跨語通用。

use uuid::Uuid;

/// 「最近表情」倒數維持的幀數（`game.rs` 每幀遞減）。約 2 秒（TICK_HZ≈20）——一群人不必分毫
/// 不差同時按，在這個時窗內各比一次同個表情就湊得上；過了就淡掉、不和不相干的後來者誤湊。
pub const RESONANCE_WINDOW: u16 = 40;

/// 構成一次共鳴所需的最少人數。三人（含）以上同表情同框才算「眾人共鳴」，兩人是擊掌（339）的領域。
pub const RESONANCE_MIN: usize = 3;

/// 共鳴聚團的最大半徑（像素，世界座標）。同一團裡每個成員都得落在種子玩家這個半徑內——
/// 比擊掌（64px）寬些，讓三五個人圍在一塊也算同框，但仍要「看得到彼此」的近。
pub const RESONANCE_RANGE: f32 = 110.0;

/// 共鳴特效在前端顯示的秒數（放大發光迸發＋淡出的總時長）。比單枚表情久一點、更有份量。
pub const RESONANCE_DISPLAY_SECS: u32 = 5;

/// 一筆「最近比了表情」的玩家回響快照。`kind` = `player_emote::EMOTES` 索引（同索引＝同表情）；
/// `zone` = 同區判定鍵（如星球名）；座標為世界像素座標。
#[derive(Debug, Clone)]
pub struct Echo {
    pub id: Uuid,
    pub kind: u8,
    pub zone: String,
    pub x: f32,
    pub y: f32,
}

/// 一次偵測到的共鳴。`kind` = 共鳴的表情索引；`mx`/`my` = 參與者重心（特效中心）；
/// `size` = 參與人數；`members` = 參與者 id（接線層據此清掉這些人的「最近表情」、避免下幀重複迸）。
#[derive(Debug, Clone, PartialEq)]
pub struct Resonance {
    pub kind: u8,
    pub mx: f32,
    pub my: f32,
    pub size: u32,
    pub members: Vec<Uuid>,
}

/// 從一串「最近表情」回響中偵測共鳴。
///
/// 規則：
/// - 以**未入團且 id 最小**的玩家為種子，蒐集與他**同 `kind`、同 `zone`、距離 ≤ `RESONANCE_RANGE`**
///   的所有未入團者（含種子自己）成一團。
/// - 一團人數 ≥ `RESONANCE_MIN` 才算共鳴：取重心為特效中心、人數為 `size`、成員 id 排序後回傳，
///   並把這些人標記為已入團（每人至多入一團）。不足人數的種子不成團、其成員留待後續種子或下幀。
/// - 依 id 排序挑種子，結果確定可重現（不依賴 HashMap 迭代順序）。
///
/// 同一表情、同一團人，永遠得到同一個共鳴結果，可測。
pub fn detect(echoes: &[Echo]) -> Vec<Resonance> {
    // 依 id 排序，讓「誰當種子」與聚團結果都確定。
    let mut idx: Vec<usize> = (0..echoes.len()).collect();
    idx.sort_by_key(|&i| echoes[i].id);

    let mut taken = vec![false; echoes.len()];
    let mut out: Vec<Resonance> = Vec::new();

    for a_pos in 0..idx.len() {
        let ai = idx[a_pos];
        if taken[ai] {
            continue;
        }
        let a = &echoes[ai];
        // 蒐集同表情、同區、落在種子半徑內的未入團者（種子本人先入團）。
        let mut group: Vec<usize> = vec![ai];
        for &bi in idx.iter() {
            if bi == ai || taken[bi] {
                continue;
            }
            let b = &echoes[bi];
            if b.kind != a.kind || b.zone != a.zone {
                continue;
            }
            let dx = a.x - b.x;
            let dy = a.y - b.y;
            if dx * dx + dy * dy <= RESONANCE_RANGE * RESONANCE_RANGE {
                group.push(bi);
            }
        }
        if group.len() >= RESONANCE_MIN {
            let n = group.len() as f32;
            let mx = group.iter().map(|&i| echoes[i].x).sum::<f32>() / n;
            let my = group.iter().map(|&i| echoes[i].y).sum::<f32>() / n;
            let mut members: Vec<Uuid> = group.iter().map(|&i| echoes[i].id).collect();
            members.sort();
            for &i in &group {
                taken[i] = true;
            }
            out.push(Resonance { kind: a.kind, mx, my, size: group.len() as u32, members });
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn echo(id: u128, kind: u8, zone: &str, x: f32, y: f32) -> Echo {
        Echo { id: Uuid::from_u128(id), kind, zone: zone.to_string(), x, y }
    }

    #[test]
    fn no_echoes_no_resonance() {
        assert!(detect(&[]).is_empty());
    }

    #[test]
    fn two_same_emote_is_not_enough() {
        // 兩人同表情同框：未達 RESONANCE_MIN(3)，不成共鳴（那是擊掌的領域）。
        let echoes = vec![echo(1, 0, "home", 0.0, 0.0), echo(2, 0, "home", 10.0, 0.0)];
        assert!(detect(&echoes).is_empty());
    }

    #[test]
    fn three_same_emote_close_resonates() {
        let echoes = vec![
            echo(1, 0, "home", 0.0, 0.0),
            echo(2, 0, "home", 10.0, 0.0),
            echo(3, 0, "home", -10.0, 5.0),
        ];
        let res = detect(&echoes);
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].kind, 0);
        assert_eq!(res[0].size, 3);
        assert_eq!(res[0].members, vec![Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3)]);
        // 重心 = 三點平均。
        assert!((res[0].mx - 0.0).abs() < 1e-3);
        assert!((res[0].my - (5.0 / 3.0)).abs() < 1e-3);
    }

    #[test]
    fn different_emotes_do_not_resonate() {
        // 三人同框但各比不同表情：湊不成同一團（每個 kind 各自只有一人）。
        let echoes = vec![
            echo(1, 0, "home", 0.0, 0.0),
            echo(2, 1, "home", 10.0, 0.0),
            echo(3, 2, "home", -10.0, 0.0),
        ];
        assert!(detect(&echoes).is_empty());
    }

    #[test]
    fn far_apart_same_emote_does_not_resonate() {
        // 三人同表情但散得太開（彼此 >RANGE）：種子半徑內湊不到 3 人。
        let echoes = vec![
            echo(1, 0, "home", 0.0, 0.0),
            echo(2, 0, "home", 500.0, 0.0),
            echo(3, 0, "home", 1000.0, 0.0),
        ];
        assert!(detect(&echoes).is_empty());
    }

    #[test]
    fn different_zones_do_not_merge() {
        // 同座標、同表情，但分屬不同星球：不同 zone 不湊團。
        let echoes = vec![
            echo(1, 0, "home", 5.0, 5.0),
            echo(2, 0, "verdant", 5.0, 5.0),
            echo(3, 0, "home", 6.0, 6.0),
        ];
        // home 只有 1、3 兩人 → 不足 3；verdant 只有 2 一人。皆不成共鳴。
        assert!(detect(&echoes).is_empty());
    }

    #[test]
    fn just_inside_range_resonates_just_outside_does_not() {
        let inside = vec![
            echo(1, 0, "home", 0.0, 0.0),
            echo(2, 0, "home", RESONANCE_RANGE - 0.1, 0.0),
            echo(3, 0, "home", 0.0, RESONANCE_RANGE - 0.1),
        ];
        assert_eq!(detect(&inside).len(), 1, "剛好都在種子半徑內 → 共鳴");
        let outside = vec![
            echo(1, 0, "home", 0.0, 0.0),
            echo(2, 0, "home", RESONANCE_RANGE + 0.1, 0.0),
            echo(3, 0, "home", 0.0, RESONANCE_RANGE + 0.1),
        ];
        assert!(detect(&outside).is_empty(), "都剛好超出種子半徑 → 不共鳴");
    }

    #[test]
    fn each_player_in_at_most_one_resonance() {
        // 六人同表情擠成兩團（每團三人、團間遠）：應得兩個共鳴、每人至多入一團。
        let echoes = vec![
            echo(1, 0, "home", 0.0, 0.0),
            echo(2, 0, "home", 10.0, 0.0),
            echo(3, 0, "home", 20.0, 0.0),
            echo(4, 0, "home", 1000.0, 0.0),
            echo(5, 0, "home", 1010.0, 0.0),
            echo(6, 0, "home", 1020.0, 0.0),
        ];
        let res = detect(&echoes);
        assert_eq!(res.len(), 2, "兩團各自共鳴");
        let mut seen = std::collections::HashSet::new();
        for r in &res {
            for m in &r.members {
                assert!(seen.insert(*m), "同一人不可入兩個共鳴");
            }
        }
    }

    #[test]
    fn two_emote_kinds_each_with_three_resonate_separately() {
        // 同框六人：三人比 wave(0)、三人比 cheer(1)，各自共鳴、互不相混。
        let echoes = vec![
            echo(1, 0, "home", 0.0, 0.0),
            echo(2, 0, "home", 8.0, 0.0),
            echo(3, 0, "home", 16.0, 0.0),
            echo(4, 1, "home", 4.0, 8.0),
            echo(5, 1, "home", 12.0, 8.0),
            echo(6, 1, "home", 20.0, 8.0),
        ];
        let mut res = detect(&echoes);
        res.sort_by_key(|r| r.kind);
        assert_eq!(res.len(), 2);
        assert_eq!(res[0].kind, 0);
        assert_eq!(res[0].size, 3);
        assert_eq!(res[1].kind, 1);
        assert_eq!(res[1].size, 3);
    }

    #[test]
    fn larger_group_reports_full_size() {
        // 五人同表情同框：一個共鳴、size=5。
        let echoes = (1..=5)
            .map(|i| echo(i, 0, "home", (i as f32) * 8.0, 0.0))
            .collect::<Vec<_>>();
        let res = detect(&echoes);
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].size, 5);
        assert_eq!(res[0].members.len(), 5);
    }

    #[test]
    fn result_is_deterministic_regardless_of_input_order() {
        let a = vec![
            echo(3, 0, "home", 16.0, 0.0),
            echo(1, 0, "home", 0.0, 0.0),
            echo(2, 0, "home", 8.0, 0.0),
        ];
        let mut b = a.clone();
        b.reverse();
        assert_eq!(detect(&a), detect(&b), "共鳴結果不該因輸入順序而變");
    }
}
