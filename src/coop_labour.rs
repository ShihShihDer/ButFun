//! 並肩協作·結伴勞動的默契加成（ROADMAP 414）。
//!
//! ButFun 的北極星是「**多人**療癒世界」，但長久以來幾乎所有生產活動都是**各做各的**：
//! 採集、種田、採礦的回報只看你一個人，旁邊有沒有真人玩家一起忙活，毫無差別。多人互動
//! 這條線此前長出的全是**情緒信號**（338 表情／339 擊掌／340 共鳴／341 喝采／342 人氣聚會）
//! ——迸完即散、與「世界的核心勞動」脫鉤。本切片給多人維度第一個**綁在核心生產活動上的
//! 真實協作回報**：當你和其他真人玩家**並肩**做同一件勞動（先從採集接起），彼此湧起一份
//! 「勞動默契」，採集量與經驗都會多一點——一起忙活，比單幹更有收穫、也更有伴。
//!
//! 設計取捨（與既有社交弧刻意換骨架）：
//! - **換骨架**：338～342 是「比個表情、拍個手」的即時情緒特效，本切片是「**並肩做事就有
//!   實質產出加成**」的協作機制——不是新的玩家指令、不是新的特效煙火，而是把多人「在一起
//!   勞動」這件事第一次接進核心採集迴圈的回報。
//! - **純函式可測**：`count_partners`／`coop_yield_bonus`／`coop_exp_pct` 皆純函式，
//!   只吃座標與人數、無副作用，方便單元測試；接線在 `ws.rs` 採集路徑。
//! - **零持久化、零 migration、零動架構**：只在採集當下讀一次在線玩家座標算同伴數，
//!   不新增任何欄位、不碰移動物理（wasm）、不碰玩家資料。
//! - **療癒分寸＋防刷**：同伴數**封頂** `MAX_PARTNERS`，加成很小（每位同伴 +1 採集量、
//!   +5% 採集經驗，最多 +3／+15%）；它是「謝你陪我一起忙」的暖意，不是刷產出的水龍頭，
//!   也不削弱獨自遊玩（沒同伴就照舊、零懲罰）。

/// 並肩協作半徑（px）：兩名玩家相距在此之內，才算「一起勞動」。
/// 取 160px——比擊掌／共鳴的貼身距離寬一些（勞動是各忙各的、不必擠在一起），
/// 但仍需「看得見彼此在同一片地上忙活」的近。
pub const COOP_RADIUS: f32 = 160.0;

/// 計入默契的最大同伴數（封頂防刷）：再多人圍著也只算到這麼多。
pub const MAX_PARTNERS: usize = 3;

/// 每位並肩同伴帶來的額外採集量。
pub const BONUS_QTY_PER_PARTNER: u32 = 1;

/// 每位並肩同伴帶來的額外採集經驗百分比。
pub const EXP_PCT_PER_PARTNER: u32 = 5;

/// 數出「我」身旁在協作半徑內的同伴數，封頂於 `MAX_PARTNERS`。
///
/// `others` 應是**其他**在線玩家的座標（呼叫端需先排除自己與不該計入者，
/// 例如倒地玩家）。以距離平方比較、免開根號。半徑採**含界**（恰好在半徑上也算）。
pub fn count_partners(my: (f32, f32), others: &[(f32, f32)]) -> usize {
    let r2 = COOP_RADIUS * COOP_RADIUS;
    let n = others
        .iter()
        .filter(|(ox, oy)| {
            let dx = ox - my.0;
            let dy = oy - my.1;
            dx * dx + dy * dy <= r2
        })
        .count();
    n.min(MAX_PARTNERS)
}

/// 依並肩同伴數算額外採集量（已對 `MAX_PARTNERS` 防禦性封頂）。
/// 0 名同伴 → 0（獨自遊玩照舊、零加成、零懲罰）。
pub fn coop_yield_bonus(partners: usize) -> u32 {
    partners.min(MAX_PARTNERS) as u32 * BONUS_QTY_PER_PARTNER
}

/// 依並肩同伴數算額外採集經驗百分比（已對 `MAX_PARTNERS` 防禦性封頂）。
pub fn coop_exp_pct(partners: usize) -> u32 {
    partners.min(MAX_PARTNERS) as u32 * EXP_PCT_PER_PARTNER
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_others_means_no_partners_no_bonus() {
        assert_eq!(count_partners((0.0, 0.0), &[]), 0);
        assert_eq!(coop_yield_bonus(0), 0);
        assert_eq!(coop_exp_pct(0), 0);
    }

    #[test]
    fn one_nearby_counts_as_one_partner() {
        // 距離 100px < 160px 半徑：算一名同伴。
        let p = count_partners((0.0, 0.0), &[(100.0, 0.0)]);
        assert_eq!(p, 1);
        assert_eq!(coop_yield_bonus(p), BONUS_QTY_PER_PARTNER);
        assert_eq!(coop_exp_pct(p), EXP_PCT_PER_PARTNER);
    }

    #[test]
    fn partner_exactly_on_radius_is_inclusive() {
        // 恰好在半徑上（含界）→ 算。
        assert_eq!(count_partners((0.0, 0.0), &[(COOP_RADIUS, 0.0)]), 1);
    }

    #[test]
    fn partner_just_beyond_radius_excluded() {
        // 略超出半徑 → 不算。
        assert_eq!(count_partners((0.0, 0.0), &[(COOP_RADIUS + 0.5, 0.0)]), 0);
    }

    #[test]
    fn counts_only_those_within_radius() {
        // 一近一遠：只算近的那位。
        let others = [(50.0, 50.0), (1000.0, 1000.0)];
        assert_eq!(count_partners((0.0, 0.0), &others), 1);
    }

    #[test]
    fn partner_count_caps_at_max() {
        // 五名都在半徑內 → 封頂於 MAX_PARTNERS。
        let others: Vec<(f32, f32)> = (0..5).map(|i| (10.0 * i as f32, 0.0)).collect();
        let p = count_partners((0.0, 0.0), &others);
        assert_eq!(p, MAX_PARTNERS);
        assert_eq!(coop_yield_bonus(p), MAX_PARTNERS as u32 * BONUS_QTY_PER_PARTNER);
        assert_eq!(coop_exp_pct(p), MAX_PARTNERS as u32 * EXP_PCT_PER_PARTNER);
    }

    #[test]
    fn bonus_is_monotonic_and_capped() {
        let mut last_qty = 0;
        let mut last_pct = 0;
        for n in 0..8 {
            let q = coop_yield_bonus(n);
            let pct = coop_exp_pct(n);
            assert!(q >= last_qty, "採集量加成不應隨同伴變少");
            assert!(pct >= last_pct, "經驗加成不應隨同伴變少");
            assert!(q <= MAX_PARTNERS as u32 * BONUS_QTY_PER_PARTNER, "採集量加成超過上限");
            assert!(pct <= MAX_PARTNERS as u32 * EXP_PCT_PER_PARTNER, "經驗加成超過上限");
            last_qty = q;
            last_pct = pct;
        }
    }
}
