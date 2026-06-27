//! 放流養塘（ROADMAP 561）——釣魚第一次有了「收下 vs 放流」的取捨與養成曲線。
//!
//! 釣魚（47／346 上鉤小遊戲／434 釣竿／449 體長／363 季節魚）這條維度長久以來，
//! 上鉤後**只有一條路：收下入袋**。魚進背包能吃能賣，但「何時收、收不收」對玩家
//! 毫無能動性——上鉤就是收，收完就完。本切片給它第一個**真選擇**：切到「🕊️ 放流模式」，
//! 上鉤的魚便**放牠回水裡**——這趟不入袋（犧牲眼前漁獲），換來**更高的漁夫熟練度**，
//! 並一點一滴累積「**養塘度**」（放流過的尾數）。養塘度越高、**你與這片水越結緣**，
//! 釣魚的收放竿冷卻越短（有地板）——犧牲眼前一尾魚，換你長遠的釣技越來越俐落。
//!
//! 全純記憶體（重啟歸零、零持久化、零 migration）、純函式抽成可測（本檔）、零 LLM。
//!
//! **換骨架（明確別於近期切片）**：560 是「種田·成熟時機窗」、552 是「打坐·定境時長」、
//! 553~559 是「居民關係弧」；本切片切到**「釣魚·收下 vs 放流的價值取捨＋養成曲線」**——
//! 不是時機窗、不是居民社交，而是一個**持續的策略選擇**（今天釣魚換食物／賣錢，還是放流
//! 養塘、投資長遠釣技），對齊療癒向「生態守護者」基調（放生而非濫捕）。
//!
//! **平衡（誠實交代）**：放流**天生對經濟為負**——你主動放棄了這一尾魚（不入袋、不可賣、
//! 不可煮），唯一回報是熟練度與「更短的釣魚冷卻」；而冷卻有**地板**（`COOLDOWN_FLOOR_SECS`），
//! 且魚本就不進核心結算（只食用回血／小額賣 NPC），故「養塘讓你釣更快」對整體經濟近乎零擾動，
//! 只是把「放生守塘」這個利他選擇接上一條看得見的個人成長曲線（鏡像 449 漁夫驕傲的純個人向）。

use crate::fishing::{FISH_COOLDOWN_SECS, FISH_FARMER_XP};

/// 放流（而非收下）一尾魚額外給的漁夫熟練度 XP。
/// 放流總得 = `FISH_FARMER_XP + RELEASE_BONUS_XP`——比收下略高，補償你放棄的那尾魚。
pub const RELEASE_BONUS_XP: u32 = 6;

/// 養塘度（累積放流尾數）達各階「結緣等級」的門檻。
/// `bond_tier` 回「已達到的最高階索引」（0＝未結緣，最高 = 此陣列長度）。
/// 刻意前疏後密遞增，讓初期幾尾就有感、後期細水長流。
pub const BOND_THRESHOLDS: &[u32] = &[3, 8, 18, 35, 60];

/// 釣魚冷卻的地板（秒）：結緣再深也不會比這更快，守住釣魚節奏、不開水龍頭。
pub const COOLDOWN_FLOOR_SECS: f32 = 3.0;

/// 每一階結緣替釣魚冷卻縮短的秒數。基礎冷卻 `FISH_COOLDOWN_SECS`(5.0) 減去
/// `tier × 此值`、夾在地板以上——滿階（5）時 5.0 − 5×0.5 = 2.5 → 夾到地板 3.0。
pub const COOLDOWN_PER_TIER_SECS: f32 = 0.5;

/// 放流一尾魚應給的漁夫熟練度 XP（純查表、可測）。
pub fn release_farmer_xp() -> u32 {
    FISH_FARMER_XP + RELEASE_BONUS_XP
}

/// 由累積放流尾數回「結緣等級」（0＝未結緣，最高 = `BOND_THRESHOLDS.len()`）。
/// 純函式、單調不減：放流越多階級越高（封頂於最後一階）。
pub fn bond_tier(released: u32) -> u8 {
    let mut tier = 0u8;
    for &threshold in BOND_THRESHOLDS {
        if released >= threshold {
            tier += 1;
        } else {
            break;
        }
    }
    tier
}

/// 結緣等級最高階（封頂值），供前端對照與測試。
pub fn max_tier() -> u8 {
    BOND_THRESHOLDS.len() as u8
}

/// 由累積放流尾數回「此玩家當下的釣魚冷卻秒數」（含結緣縮短、夾在地板以上）。
/// 純函式、單調不增（放流越多冷卻越短）、有地板（永不刷水龍頭）。
/// 壞值安全：released 為 0 → 回基礎冷卻；極大值 → 夾到地板。
pub fn cooldown_secs(released: u32) -> f32 {
    let tier = bond_tier(released) as f32;
    (FISH_COOLDOWN_SECS - tier * COOLDOWN_PER_TIER_SECS).max(COOLDOWN_FLOOR_SECS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 放流給的熟練度高於收下() {
        assert!(release_farmer_xp() > FISH_FARMER_XP);
        assert_eq!(release_farmer_xp(), FISH_FARMER_XP + RELEASE_BONUS_XP);
    }

    #[test]
    fn 未放流時未結緣() {
        assert_eq!(bond_tier(0), 0);
        assert_eq!(bond_tier(BOND_THRESHOLDS[0] - 1), 0);
    }

    #[test]
    fn 結緣等級在各門檻遞增且封頂() {
        // 剛好踩到第一階門檻 → 1 階。
        assert_eq!(bond_tier(BOND_THRESHOLDS[0]), 1);
        // 逐階遞增。
        for (i, &threshold) in BOND_THRESHOLDS.iter().enumerate() {
            assert_eq!(bond_tier(threshold), (i + 1) as u8);
        }
        // 超過最後一階門檻仍封頂於 max_tier。
        let last = *BOND_THRESHOLDS.last().unwrap();
        assert_eq!(bond_tier(last + 1000), max_tier());
        assert_eq!(bond_tier(u32::MAX), max_tier());
    }

    #[test]
    fn 結緣等級單調不減() {
        let mut prev = 0u8;
        for released in 0..200u32 {
            let t = bond_tier(released);
            assert!(t >= prev, "released={released} tier={t} < prev={prev}");
            prev = t;
        }
    }

    #[test]
    fn 冷卻隨結緣縮短但不破地板() {
        // 未結緣 = 基礎冷卻。
        assert!((cooldown_secs(0) - FISH_COOLDOWN_SECS).abs() < 1e-6);
        // 每多一階冷卻更短或相等（單調不增）。
        let mut prev = cooldown_secs(0);
        for released in 0..200u32 {
            let c = cooldown_secs(released);
            assert!(c <= prev + 1e-6, "released={released} cd={c} > prev={prev}");
            assert!(c >= COOLDOWN_FLOOR_SECS - 1e-6, "冷卻 {c} 破地板");
            prev = c;
        }
        // 滿階夾到地板。
        let last = *BOND_THRESHOLDS.last().unwrap();
        assert!((cooldown_secs(last) - COOLDOWN_FLOOR_SECS).abs() < 1e-6);
    }

    #[test]
    fn 冷卻地板不高於基礎() {
        assert!(COOLDOWN_FLOOR_SECS < FISH_COOLDOWN_SECS);
    }
}
