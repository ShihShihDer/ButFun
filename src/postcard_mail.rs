//! 旅途明信片·寄給同行旅人（ROADMAP 480）——把你框下的「此刻世界」明信片，
//! 親手寄進身旁一位旅人的信箱。
//!
//! 417 的旅途明信片是一張只屬於自己的留念卡（單播回自己、下載收藏），它從不曾在
//! 玩家之間流動。本切片把這份 keepsake 接上社交：你按下「✉️ 寄給附近旅人」，伺服器
//! 以你當下的世界狀態組一張明信片，**送進身旁最近那位旅人的信箱**——一張帶著你署名
//! 與一句手寫話的風景卡，從一個玩家手裡，遞到另一個玩家手裡。「明信片（417）」這個
//! 自我表達物件，第一次能「被他人收藏」（呼應 479 雪人讚賞開的雙向社交互連，再深一層：
//! 不只按個讚，而是把一件親手框下的作品，當禮物送出去）。
//!
//! 設計鐵律：
//! - **純判定、零狀態、可測**：本模組只放純函式——挑收件旅人（`pick_recipient`）、
//!   清理手寫留言（`sanitize_note`）。投遞、限流、鎖序都在 ws 接線層，純邏輯不碰 IO。
//! - **挑最近、防隔空**：寄件者一律用自己的權威座標，挑「範圍內、最近、不在室內、
//!   非自己」的另一名在場旅人；同距取 id 較小求確定、結果可測。
//! - **零經濟、零平衡**：明信片只是一張風景卡，不送物品／乙太／戰力，純社交暖意。
//! - 面向玩家字串（手寫留言佔位）集中前端；本模組只做機械清理，留 i18n 空間。

use uuid::Uuid;

/// 寄送範圍（像素）：要寄明信片得走到對方身旁。比擦肩可及的喝采範圍寬一些，讓「同框旅人」
/// 更容易湊到（明信片是溫和善意、不像喝采怕洗榜，範圍寬無妨）。
pub const DELIVERY_RANGE: f32 = 160.0;

/// 手寫留言上限（字元）：明信片只容得下短短一行，過長截掉。
pub const NOTE_MAX_CHARS: usize = 60;

/// 在場可作為收件人的旅人候選（由 ws 層從權威 `players` 快照組出）。
#[derive(Debug, Clone)]
pub struct Recipient {
    pub id: Uuid,
    pub name: String,
    pub x: f32,
    pub y: f32,
    /// 是否在室內：室內旅人在另一套座標空間、收不到「身旁」的明信片，排除。
    pub indoor: bool,
}

/// 從候選裡挑一名收件旅人：範圍內、不在室內、非自己、最近者；同距離取 id 較小者求確定。
/// 寄件者座標 `(sx, sy)` 一律是寄件者的權威座標（防隔空寄信）。挑不到回 `None`。
pub fn pick_recipient(
    sender_id: Uuid,
    sx: f32,
    sy: f32,
    candidates: &[Recipient],
) -> Option<(Uuid, String)> {
    let mut best: Option<(Uuid, f32, String)> = None;
    for c in candidates {
        if c.id == sender_id || c.indoor {
            continue;
        }
        let dx = c.x - sx;
        let dy = c.y - sy;
        let d2 = dx * dx + dy * dy;
        if !d2.is_finite() || d2 > DELIVERY_RANGE * DELIVERY_RANGE {
            continue;
        }
        let better = match &best {
            None => true,
            Some((bid, bd2, _)) => d2 < *bd2 || (d2 == *bd2 && c.id < *bid),
        };
        if better {
            best = Some((c.id, d2, c.name.clone()));
        }
    }
    best.map(|(id, _, name)| (id, name))
}

/// 清理玩家手寫留言：把控制字元（含換行／tab）一律折成空白、收斂連續空白、去首尾留白，
/// 再截到 `NOTE_MAX_CHARS`。清完無內容回空字串（呼叫端視為「沒寫話」，仍可寄一張純風景卡）。
/// 注意：截長以「字元（Unicode 純量）」計，CJK 一字一格，不會把多位元組字切半。
pub fn sanitize_note(raw: &str) -> String {
    let spaced: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    spaced
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(NOTE_MAX_CHARS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rcp(id: u128, name: &str, x: f32, y: f32, indoor: bool) -> Recipient {
        Recipient {
            id: Uuid::from_u128(id),
            name: name.to_string(),
            x,
            y,
            indoor,
        }
    }

    #[test]
    fn picks_nearest_in_range() {
        let me = Uuid::from_u128(1);
        let cands = vec![
            rcp(2, "遠", 100.0, 0.0, false),
            rcp(3, "近", 30.0, 0.0, false),
        ];
        let got = pick_recipient(me, 0.0, 0.0, &cands);
        assert_eq!(got, Some((Uuid::from_u128(3), "近".to_string())));
    }

    #[test]
    fn none_when_all_out_of_range() {
        let me = Uuid::from_u128(1);
        let cands = vec![rcp(2, "太遠", DELIVERY_RANGE + 0.1, 0.0, false)];
        assert_eq!(pick_recipient(me, 0.0, 0.0, &cands), None);
    }

    #[test]
    fn just_inside_range_is_pickable() {
        let me = Uuid::from_u128(1);
        let cands = vec![rcp(2, "邊緣", DELIVERY_RANGE - 0.1, 0.0, false)];
        assert!(pick_recipient(me, 0.0, 0.0, &cands).is_some());
    }

    #[test]
    fn excludes_self_and_indoor() {
        let me = Uuid::from_u128(1);
        let cands = vec![
            rcp(1, "我自己", 5.0, 0.0, false), // 自己：排除
            rcp(2, "室內", 6.0, 0.0, true),    // 室內：排除
        ];
        assert_eq!(pick_recipient(me, 0.0, 0.0, &cands), None);
    }

    #[test]
    fn tie_breaks_on_smaller_id() {
        let me = Uuid::from_u128(99);
        // 兩人同距離 → 取 id 較小者（=2）求確定。
        let cands = vec![
            rcp(5, "五", 10.0, 0.0, false),
            rcp(2, "二", 10.0, 0.0, false),
        ];
        let got = pick_recipient(me, 0.0, 0.0, &cands);
        assert_eq!(got, Some((Uuid::from_u128(2), "二".to_string())));
    }

    #[test]
    fn nonfinite_coords_are_safe() {
        let me = Uuid::from_u128(1);
        let cands = vec![rcp(2, "壞座標", f32::NAN, f32::INFINITY, false)];
        assert_eq!(pick_recipient(me, 0.0, 0.0, &cands), None);
    }

    #[test]
    fn sanitize_strips_control_and_collapses_whitespace() {
        assert_eq!(sanitize_note("  你好\n\t  旅人  "), "你好 旅人");
        assert_eq!(sanitize_note("a\u{0007}b"), "a b"); // 響鈴控制字元折成空白
    }

    #[test]
    fn sanitize_empty_stays_empty() {
        assert_eq!(sanitize_note("   \n\t  "), "");
        assert_eq!(sanitize_note(""), "");
    }

    #[test]
    fn sanitize_caps_length_by_chars() {
        let long: String = "字".repeat(NOTE_MAX_CHARS + 20);
        let out = sanitize_note(&long);
        assert_eq!(out.chars().count(), NOTE_MAX_CHARS);
    }
}
