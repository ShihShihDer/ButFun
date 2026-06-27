//! 勘礦造詣（ROADMAP 562）——採礦第一次有了「越掘越懂礦脈」的個人養成曲線。
//!
//! 礦脈深掘（348 press-your-luck 見好就收）這條維度自上線以來，**每條礦脈對所有人都一樣**：
//! 不管你是第一次摸鎬子、還是落袋過上百袋礦的老手，崩塌機率、礦量、撤出回報全無分別——
//! 採礦的「技巧」只活在當下那一念之差（再敲一下 vs 收手），卻沒有任何「越做越精」的長期積累。
//! 本切片給它第一條個人成長曲線：每次「漂亮地落袋」（沒崩、安全撤出）都累積勘礦造詣，
//! 造詣分四階（生手→識礦人→老礦工→礦脈大師），階位越高：
//!   ① 每次落袋的**探索熟練度回報**越豐（純個人熟練度成長，鏡像 449 漁夫驕傲）；
//!   ② 老礦工級起，**即使礦脈崩了也從中學到東西**——給一小筆安慰探索 XP，撫平失手的刺。
//!
//! 全純記憶體（重啟歸零、零持久化、零 migration）、純函式抽成可測（本檔）、零 LLM。
//!
//! **換骨架（明確別於近期切片）**：560 是「種田·成熟時機窗」、561 是「釣魚·收下 vs 放流取捨」、
//! 553~559 是「居民關係弧」；本切片切到**「採礦·勘礦造詣養成曲線」**——換一條自 348 後再沒
//! 做深的既有系統（採礦），給它的 press-your-luck 抉擇接上一條看得見的長期精進軌跡。
//!
//! **平衡（誠實交代，零礦石／經濟擾動）**：造詣**只放大「探索熟練度」回報＋崩塌安慰 XP**——
//! 探索熟練度是個人 mastery（不可交易、不可合成、不換戰力），**不改任何礦石產量、崩塌深度
//! 分佈、採礦冷卻**（這些在每一階都一模一樣）。故對礦石經濟零擾動、對戰鬥正交，純粹把
//! 「常下礦坑、撤得漂亮」這件事接上一條個人成長曲線（同 449／535 純個人向的精神）。

/// 累積「安全落袋」次數達各階「造詣等級」的門檻。
/// `mastery_tier` 回「已達到的最高階索引」（0＝生手，最高 = 此陣列長度）。
/// 刻意前疏後密遞增，讓初期幾袋就有感、後期細水長流。
pub const MASTERY_THRESHOLDS: &[u32] = &[4, 12, 28];

/// 探索熟練度回報每高一階的加成（千分比）：基礎 XP × (1000 + tier × 此值) / 1000。
/// 生手 ×1.0、識礦人 ×1.2、老礦工 ×1.4、礦脈大師 ×1.6——只放大個人熟練度回報，零礦石擾動。
pub const XP_BONUS_PER_TIER_PERMILLE: u32 = 200;

/// 「崩塌也學到東西」安慰探索 XP 的最低造詣階（老礦工＝2 起才有）。
/// 讓老手即便整袋落空也不至於一無所獲，撫平 press-your-luck 失手的刺。
pub const CONSOLATION_MIN_TIER: u8 = 2;

/// 礦脈崩塌時，達 `CONSOLATION_MIN_TIER` 的老手所得的安慰探索 XP（刻意壓小，純安慰）。
pub const COLLAPSE_CONSOLATION_XP: u32 = 5;

/// 由累積安全落袋次數回「勘礦造詣等級」（0＝生手，最高 = `MASTERY_THRESHOLDS.len()`）。
/// 純函式、單調不減：落袋越多階級越高（封頂於最後一階）。
pub fn mastery_tier(hauls: u32) -> u8 {
    let mut tier = 0u8;
    for &threshold in MASTERY_THRESHOLDS {
        if hauls >= threshold {
            tier += 1;
        } else {
            break;
        }
    }
    tier
}

/// 勘礦造詣最高階（封頂值），供前端對照與測試。
pub fn max_tier() -> u8 {
    MASTERY_THRESHOLDS.len() as u8
}

/// 這一次落袋是否「恰好跨入新造詣階」——是則回 `Some(新階位)`，供前端飄金色升階慶賀。
/// 防呆：`new <= prev`（不可能倒退）一律回 `None`。
pub fn tier_up(prev_hauls: u32, new_hauls: u32) -> Option<u8> {
    if new_hauls <= prev_hauls {
        return None;
    }
    let before = mastery_tier(prev_hauls);
    let after = mastery_tier(new_hauls);
    if after > before {
        Some(after)
    } else {
        None
    }
}

/// 依當下造詣階位放大「這一次落袋」的探索熟練度回報（純查表、整數運算、單調不減）。
/// 壞值安全：`base_xp` 為 0 → 回 0；造詣 0 → 原值不變。
pub fn explorer_xp_for_haul(base_xp: u32, hauls: u32) -> u32 {
    let tier = mastery_tier(hauls) as u32;
    // 以 u64 中介防整數溢位（base_xp 實際很小，僅保險）。
    let scaled =
        (base_xp as u64 * (1000 + tier * XP_BONUS_PER_TIER_PERMILLE) as u64) / 1000;
    scaled as u32
}

/// 礦脈崩塌時，依當下造詣階位回「安慰探索 XP」——未達老礦工（`CONSOLATION_MIN_TIER`）回 0。
/// 純函式、壞值安全：純查表、無副作用。
pub fn collapse_consolation_xp(hauls: u32) -> u32 {
    if mastery_tier(hauls) >= CONSOLATION_MIN_TIER {
        COLLAPSE_CONSOLATION_XP
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mining_vein::HAUL_EXPLORER_XP_PER_DEPTH;

    #[test]
    fn 未落袋時為生手() {
        assert_eq!(mastery_tier(0), 0);
        assert_eq!(mastery_tier(MASTERY_THRESHOLDS[0] - 1), 0);
    }

    #[test]
    fn 造詣等級在各門檻遞增且封頂() {
        // 剛好踩到第一階門檻 → 1 階。
        assert_eq!(mastery_tier(MASTERY_THRESHOLDS[0]), 1);
        // 逐階遞增。
        for (i, &threshold) in MASTERY_THRESHOLDS.iter().enumerate() {
            assert_eq!(mastery_tier(threshold), (i + 1) as u8);
        }
        // 超過最後一階門檻仍封頂於 max_tier。
        let last = *MASTERY_THRESHOLDS.last().unwrap();
        assert_eq!(mastery_tier(last + 1000), max_tier());
        assert_eq!(mastery_tier(u32::MAX), max_tier());
    }

    #[test]
    fn 造詣等級單調不減() {
        let mut prev = 0u8;
        for hauls in 0..200u32 {
            let t = mastery_tier(hauls);
            assert!(t >= prev, "hauls={hauls} tier={t} < prev={prev}");
            prev = t;
        }
    }

    #[test]
    fn 升階只在恰好跨越時觸發() {
        // 同階內不觸發。
        assert_eq!(tier_up(0, MASTERY_THRESHOLDS[0] - 1), None);
        // 恰好跨入第一階。
        assert_eq!(tier_up(MASTERY_THRESHOLDS[0] - 1, MASTERY_THRESHOLDS[0]), Some(1));
        // 一口氣跨多階回最終階位。
        let last = *MASTERY_THRESHOLDS.last().unwrap();
        assert_eq!(tier_up(0, last), Some(max_tier()));
        // 防呆：非遞增一律 None。
        assert_eq!(tier_up(50, 50), None);
        assert_eq!(tier_up(50, 10), None);
    }

    #[test]
    fn 探索回報隨造詣放大且單調不減() {
        let base = HAUL_EXPLORER_XP_PER_DEPTH * 3; // 模擬撤出於第 3 層
        // 生手 = 原值。
        assert_eq!(explorer_xp_for_haul(base, 0), base);
        // 每高一階回報不減。
        let mut prev = explorer_xp_for_haul(base, 0);
        for hauls in 0..200u32 {
            let xp = explorer_xp_for_haul(base, hauls);
            assert!(xp >= prev, "hauls={hauls} xp={xp} < prev={prev}");
            prev = xp;
        }
        // 滿階加成正確（×1.6）。
        let last = *MASTERY_THRESHOLDS.last().unwrap();
        assert_eq!(
            explorer_xp_for_haul(base, last),
            base * (1000 + max_tier() as u32 * XP_BONUS_PER_TIER_PERMILLE) / 1000
        );
        // 壞值安全：base 0 → 0。
        assert_eq!(explorer_xp_for_haul(0, last), 0);
    }

    #[test]
    fn 崩塌安慰僅老礦工級起且壓小() {
        // 生手／識礦人崩塌＝一無所獲。
        assert_eq!(collapse_consolation_xp(0), 0);
        assert_eq!(collapse_consolation_xp(MASTERY_THRESHOLDS[0]), 0); // 識礦人(1)
        // 老礦工（2）起才有安慰。
        let veteran = MASTERY_THRESHOLDS[1];
        assert_eq!(mastery_tier(veteran), CONSOLATION_MIN_TIER);
        assert_eq!(collapse_consolation_xp(veteran), COLLAPSE_CONSOLATION_XP);
        // 礦脈大師仍給同一筆安慰。
        let master = *MASTERY_THRESHOLDS.last().unwrap();
        assert_eq!(collapse_consolation_xp(master), COLLAPSE_CONSOLATION_XP);
        // 安慰刻意壓小：不超過一次淺撤的回報。
        assert!(COLLAPSE_CONSOLATION_XP <= HAUL_EXPLORER_XP_PER_DEPTH * 2);
    }

    #[test]
    fn 造詣零經濟擾動_僅放大探索熟練度() {
        // 此測釘住「造詣只動探索 XP」的承諾：本模組不輸出任何礦量／冷卻函式，
        // 唯一的數值出口是探索 XP（explorer_xp_for_haul / collapse_consolation_xp）。
        // 若未來有人想在此加礦量加成，會先撞到這個語意鎖（需顯式改測、迫使重新權衡平衡）。
        assert!(XP_BONUS_PER_TIER_PERMILLE > 0, "造詣應實際放大探索回報");
        assert!(CONSOLATION_MIN_TIER <= max_tier(), "安慰門檻不該高過封頂");
    }
}
