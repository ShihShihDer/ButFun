//! ROADMAP 483 打水漂撈寶——打水漂第一次「玩有所得」：把純休閒的水漂接進經濟。
//!
//! 475 給了水域「打水漂」這個純表現動作——甩石、看跳數、漣漪散開，但跳得再漂亮也只是好看，
//! 不回饋玩家半點攢累（當時誠實定位成純景物，鏡像放風箏 470）。本切片把那道「甜蜜點時機」
//! 技巧接進**經濟／風險**循環：水漂貼水彈跳時漣漪會驚動水底，越漂亮的水漂（跳越多＝時機抓越準）
//! 越可能震上一點水底沉著的乙太碎屑；偶爾——只有甩出滿跳的完美水漂——還會震起一顆深海珍珠。
//! 沒撈到不是懲罰（療癒向），只是這趟石頭安靜沉下、漣漪散去而已。
//!
//! ## 設計鐵律
//! - **純邏輯可測**：`skip_find`／`ether_amount` 皆純函式、`Copy`、確定可重現、無 IO、無副作用——
//!   回饋的「源頭數值」（機率、乙太量、珍珠門檻）全定在此，呼叫端只負責產 seed 與發放。
//! - **技巧→風險→經濟**：回饋隨跳數遞增（技巧），但只是「機率震上」（風險：常常什麼都沒撈到），
//!   撈到的乙太進核心貨幣、珍珠進背包可賣 NPC（經濟）。把 475 的純表現接成有張力的循環，
//!   正面回應 reviewer「把薄動詞接進經濟／成長／風險、回饋玩家攢累」的方向。
//! - **療癒向、量級克制**：乙太量小且封頂、珍珠極罕，搭配既有 1.2s 甩石冷卻，避免變成乙太水龍頭；
//!   撈不到只是「沉了」不扣任何東西（永不懲罰，鏡像 438 輪休／454 輪作的純正向基調）。
//! - **複用既有物品零新增**：珍珠沿用 `ItemKind::DeepSeaPearl`（給它第二條來源），不新增 enum／
//!   商表／前端條目，surface 最小。零持久化、零 migration、零 LLM。

use crate::inventory::ItemKind;
use crate::skipstone::{MAX_SKIPS, MIN_SKIPS};

/// 撈到乙太的最大量（滿跳完美水漂）。刻意小、封頂——打水漂是療癒小活動，不是刷錢主力。
pub const MAX_ETHER_FIND: u32 = 3;

/// 每多一跳，「撈到東西」的機率多幾個百分點（技巧→回饋的斜率）。
/// 1 跳 ≈ 13%、5 跳 ≈ 45%（含珍珠段）；大半時候石頭只是安靜沉下。
pub const FIND_PCT_PER_SKIP: u64 = 8;

/// 撈到深海珍珠的機率（百分點）——只在滿跳完美水漂時才有機會，極罕、是追求完美的甜頭。
pub const PEARL_PCT: u64 = 5;

/// 這趟水漂震上什麼（純值、`Copy`、確定可重現）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipFind {
    /// 石頭安靜沉下，這趟沒撈到（不是懲罰，療癒向）。
    Nothing,
    /// 震上一點水底乙太碎屑（進核心貨幣餘額）。
    Ether(u32),
    /// 罕見：震起一顆深海珍珠（進背包、可賣 NPC）。
    Pearl,
}

impl SkipFind {
    /// 這趟撈到的乙太量（沒有則 0）——呼叫端據此加餘額、廣播飄字。
    pub fn ether(self) -> u32 {
        match self {
            SkipFind::Ether(n) => n,
            _ => 0,
        }
    }

    /// 這趟是否震起一顆珍珠。
    pub fn pearl(self) -> bool {
        matches!(self, SkipFind::Pearl)
    }

    /// 撈到的背包物品（目前只有珍珠走背包；乙太走餘額不進背包）。
    pub fn item(self) -> Option<ItemKind> {
        if self.pearl() {
            Some(ItemKind::DeepSeaPearl)
        } else {
            None
        }
    }
}

/// 撈到乙太時的量：隨跳數線性遞增、封頂 `MAX_ETHER_FIND`。1..=5 跳 → 1,1,2,2,3。
/// 壞跳數（0 或 > 上限）先夾在合理區間，永不回 0（撈到了就至少 1）。
pub fn ether_amount(skips: u32) -> u32 {
    let s = skips.clamp(MIN_SKIPS, MAX_SKIPS);
    (((s + 1) / 2).min(MAX_ETHER_FIND)).max(1)
}

/// 由水漂跳數與一顆 seed 算這趟震上什麼。確定可重現、純函式、無副作用。
/// - 跳越多（時機抓越準）→「撈到東西」機率越高、乙太量越大（技巧→回饋）。
/// - 只有滿跳完美水漂（`skips == MAX_SKIPS`）才有極小機率震起深海珍珠。
/// - 其餘＝`Nothing`（石頭沉了，不給也不扣——風險只是「白甩一趟」，永不懲罰）。
///
/// seed 建議帶 `player_id_low64 ^ skip_attempt_count`（每趟結果不同、可重現、好測），
/// 與釣魚 `roll_fish` 同一套確定性擲骰範式。
pub fn skip_find(skips: u32, seed: u64) -> SkipFind {
    let skips = skips.clamp(MIN_SKIPS, MAX_SKIPS);
    let pct = seed % 100; // 0..=99 的骰面

    // 1) 珍珠機會：只有完美滿跳才開，從骰面最前段切出（與乙太段不重疊）。
    if skips >= MAX_SKIPS && pct < PEARL_PCT {
        return SkipFind::Pearl;
    }

    // 2) 撈到乙太：窗口隨跳數加寬。珍珠段已佔最前 `PEARL_PCT`，乙太段接其後，
    //    故門檻 = PEARL_PCT + skips×斜率（非滿跳沒有珍珠段，但門檻同式、由 0 起判，
    //    等效於把最前段也讓給乙太，數學上一致、無縫）。
    let find_threshold = PEARL_PCT + (skips as u64) * FIND_PCT_PER_SKIP;
    if pct < find_threshold {
        return SkipFind::Ether(ether_amount(skips));
    }

    // 3) 其餘：石頭安靜沉下。
    SkipFind::Nothing
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 撈到乙太量：隨跳數不減、恆 >=1、封頂 MAX_ETHER_FIND。
    #[test]
    fn 乙太量隨跳數不減且封頂() {
        let mut prev = 0;
        for skips in MIN_SKIPS..=MAX_SKIPS {
            let amt = ether_amount(skips);
            assert!(amt >= 1, "撈到了就至少 1 乙太：skips={skips} amt={amt}");
            assert!(amt <= MAX_ETHER_FIND, "乙太量不得超過封頂：skips={skips} amt={amt}");
            assert!(amt >= prev, "乙太量應隨跳數不減：{prev} → {amt} @ skips={skips}");
            prev = amt;
        }
        // 壞跳數夾在界內、不 panic、仍 >=1。
        assert!(ether_amount(0) >= 1);
        assert!(ether_amount(9999) <= MAX_ETHER_FIND);
    }

    /// 完美滿跳水漂在骰面最前段震起珍珠。
    #[test]
    fn 滿跳前段骰面震起珍珠() {
        for pct in 0..PEARL_PCT {
            assert_eq!(skip_find(MAX_SKIPS, pct), SkipFind::Pearl, "滿跳 pct={pct} 應為珍珠");
        }
        // 珍珠段之後就不是珍珠了。
        assert_ne!(skip_find(MAX_SKIPS, PEARL_PCT), SkipFind::Pearl);
    }

    /// 非滿跳水漂永遠震不出珍珠（珍珠是完美的專屬甜頭）。
    #[test]
    fn 非滿跳永不出珍珠() {
        for skips in MIN_SKIPS..MAX_SKIPS {
            for seed in 0..200u64 {
                assert!(
                    !skip_find(skips, seed).pearl(),
                    "非滿跳不該出珍珠：skips={skips} seed={seed}"
                );
            }
        }
    }

    /// 「撈到東西」（珍珠或乙太）的骰面數隨跳數單調不減——技巧越好回饋機率越高。
    #[test]
    fn 撈到機率隨跳數遞增() {
        let mut prev_hits = 0;
        for skips in MIN_SKIPS..=MAX_SKIPS {
            let hits = (0..100u64).filter(|&pct| skip_find(skips, pct) != SkipFind::Nothing).count();
            assert!(hits >= prev_hits, "撈到骰面數應隨跳數不減：{prev_hits} → {hits} @ skips={skips}");
            prev_hits = hits;
        }
        // 最差跳數仍有「撈到」的可能（療癒向，給點盼頭），但不是必中。
        let worst = (0..100u64).filter(|&pct| skip_find(MIN_SKIPS, pct) != SkipFind::Nothing).count();
        assert!(worst > 0 && worst < 100, "最差跳數應該偶爾撈到、但非必中：{worst}/100");
    }

    /// 高骰面（接近 99）一律沒撈到——大半時候石頭只是安靜沉下。
    #[test]
    fn 高骰面皆沉底() {
        for skips in MIN_SKIPS..=MAX_SKIPS {
            assert_eq!(skip_find(skips, 99), SkipFind::Nothing, "骰面 99 應沉底：skips={skips}");
            assert_eq!(skip_find(skips, 90), SkipFind::Nothing, "骰面 90 應沉底：skips={skips}");
        }
    }

    /// 確定可重現：同跳數同 seed 永遠同結果。
    #[test]
    fn 同輸入同輸出可重現() {
        for skips in MIN_SKIPS..=MAX_SKIPS {
            for seed in [0u64, 3, 17, 42, 99, 100, 12345, u64::MAX] {
                assert_eq!(skip_find(skips, seed), skip_find(skips, seed));
            }
        }
    }

    /// 壞跳數（0 或遠超上限）夾在界內、不 panic、結果合法。
    #[test]
    fn 壞跳數保守夾界() {
        for seed in 0..100u64 {
            // 0 跳等效 MIN_SKIPS、超大跳等效 MAX_SKIPS，皆走正常分支。
            let lo = skip_find(0, seed);
            let hi = skip_find(9999, seed);
            assert!(matches!(lo, SkipFind::Nothing | SkipFind::Ether(_)));
            assert!(matches!(hi, SkipFind::Nothing | SkipFind::Ether(_) | SkipFind::Pearl));
            // 0 跳不該震出珍珠（珍珠是滿跳專屬）。
            assert!(!lo.pearl());
        }
    }

    /// SkipFind 存取器自洽：ether()／pearl()／item() 互相一致。
    #[test]
    fn 存取器自洽() {
        assert_eq!(SkipFind::Nothing.ether(), 0);
        assert!(!SkipFind::Nothing.pearl());
        assert_eq!(SkipFind::Nothing.item(), None);

        assert_eq!(SkipFind::Ether(2).ether(), 2);
        assert!(!SkipFind::Ether(2).pearl());
        assert_eq!(SkipFind::Ether(2).item(), None);

        assert_eq!(SkipFind::Pearl.ether(), 0);
        assert!(SkipFind::Pearl.pearl());
        assert_eq!(SkipFind::Pearl.item(), Some(ItemKind::DeepSeaPearl));
    }
}
