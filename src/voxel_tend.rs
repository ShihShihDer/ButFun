//! 乙太方界·居民順手替你照料菜園 v1（ROADMAP 753）。
//!
//! **設計依據**：`docs/PLAN_ETHERVOX.md`——「人類的樂趣（種田/採集）與 AI 的生活
//! （記憶/關係）在同一片方塊天地交織，這正是它比純 AI sim 或純建造遊戲都更迷人的地方」。
//! 種田（659~）一路是純粹單機的樂趣：你撒種、你等待、你收割，居民從不參與；好感系統
//! （675 老友情境問候等）則只在「打招呼／說話」時浮現。本切片把這兩條線第一次接起來——
//!
//! **對你有好感（affinity ≥ [`crate::voxel_fond_greeting::FOND_AFFINITY`]）的居民，路過你
//! 種下、還沒成熟的作物旁時，偶爾停下腳步順手幫忙照料**：把作物的生長往前推進一小段
//! （[`TEND_NUDGE_SECS`] 秒），冒一句話、記進記憶、Feed 播報。「喜歡你」第一次不只是嘴上
//! 說說，而是化成一件對你有實際好處的舉手之勞——你回頭一看，菜長高了一截。
//!
//! **不揠苗助長**：照料**永不**把作物推到瞬間成熟——[`nudge_amount`] 永遠留最後
//! [`TEND_MIN_REMAINING_SECS`] 秒讓玩家親眼看它長好，快熟的作物乾脆不碰（留給你收成的
//! 那份儀式感）。單純是「幫你順手澆點水、快一點點」，不是替你把田種完。
//!
//! **成本紀律**：零 LLM（判定＋台詞皆確定性純函式）、零 migration（借既有農地/記憶/Feed
//! 管線，不落新持久化表）、零協議破壞（沿用既有 `say`/記憶/Feed 廣播路徑，舊前端安全忽略）。
//!
//! 純邏輯層：零 IO、零鎖、零 LLM、零 async；確定性純函式。
//! 鎖與副作用在 `voxel_ws.rs`（`tick_residents` 2b 段，短鎖即釋、不巢狀，守死鎖鐵律）。

use crate::voxel_farm::CropKind;

/// Feed 播報種類名稱。
pub const FEED_KIND: &str = "照料菜園";

/// 每 tick（10Hz）觸發照料判定的機率（低頻，讓照料是偶爾出現的溫柔、不洗版）。
/// 實際還要層層過閘（冷卻未到、有好感玩家在旁、附近真有未熟作物），故有感頻率遠低於此。
pub const TEND_CHANCE_PER_TICK: f32 = 0.03;

/// 照料冷卻（秒）：一位居民照料過後，這段時間內不再照料，稀少才有感、也不刷成長。
pub const TEND_COOLDOWN: f32 = 180.0;

/// 居民與作物的最大照料距離（方塊）：居民要真的走到作物旁才幫得上忙。
pub const CARE_DIST: f32 = 2.5;

/// 一次照料把生長往前推進的秒數（一小段——是「幫你快一點」不是「替你種完」）。
pub const TEND_NUDGE_SECS: u64 = 25;

/// 照料**永遠**留給作物的最少剩餘生長秒數：確保不會揠苗助長到瞬間成熟，
/// 玩家總能親眼看到作物走完最後一小段長成——那份儀式感留給你。
pub const TEND_MIN_REMAINING_SECS: u64 = 8;

/// 判斷這一 tick 是否要照料附近的作物。
///
/// 三個條件皆成立才觸發：`is_fond`（附近有對你有好感的玩家、居民才願意幫）、
/// `has_immature_crop`（附近真有一塊還沒成熟、且值得照料的作物）、擲骰命中低機率。
pub fn should_tend(is_fond: bool, has_immature_crop: bool, roll: f32) -> bool {
    is_fond && has_immature_crop && roll < TEND_CHANCE_PER_TICK
}

/// 依作物剩餘生長秒數，算出這次照料實際往前推進多少秒。
///
/// 永遠留最後 [`TEND_MIN_REMAINING_SECS`] 秒不碰（不揠苗助長、不瞬間成熟）：
/// - 作物已剩不到門檻（快熟了）→ 回 0（乾脆不照料，留給玩家親手收成）。
/// - 否則 → 推進 [`TEND_NUDGE_SECS`]，但夾在「不越過門檻」的上限內。
pub fn nudge_amount(remaining_secs: u64) -> u64 {
    if remaining_secs <= TEND_MIN_REMAINING_SECS {
        0
    } else {
        TEND_NUDGE_SECS.min(remaining_secs - TEND_MIN_REMAINING_SECS)
    }
}

/// 作物顯示名（照料台詞/記憶/Feed 共用）。
pub fn crop_name(kind: CropKind) -> &'static str {
    match kind {
        CropKind::Wheat => "小麥",
        CropKind::Carrot => "胡蘿蔔",
        CropKind::Potato => "馬鈴薯",
        CropKind::Pumpkin => "南瓜",
    }
}

/// 照料時冒在居民頭頂的話（依 `pick` 確定性輪替，四句）。
pub fn tend_say_line(crop: &str, pick: usize) -> String {
    const LINES: [&str; 4] = [
        "你的{crop}我順手幫你澆點水～",
        "這株{crop}看起來渴了，我幫你照料一下！",
        "讓我幫你的{crop}鬆鬆土，長快一點～",
        "看到你種的{crop}，忍不住想幫你顧一顧！",
    ];
    LINES[pick % LINES.len()].replace("{crop}", crop)
}

/// 照料寫進居民記憶的摘要（掛在該玩家名下，好感自然累積——你的善意有了回報）。
pub fn tend_memory_line(player: &str, crop: &str) -> String {
    format!("路過{player}種的{crop}，順手幫忙照料了一下")
}

/// 照料的 Feed 播報文字（第三人稱，讓不在場的訪客回來也讀得到這份溫柔）。
pub fn tend_feed_line(resident: &str, player: &str, crop: &str) -> String {
    format!("{resident}路過{player}種的{crop}，順手幫忙照料了一下～")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_tend_requires_all_three_conditions() {
        assert!(should_tend(true, true, 0.0));
        assert!(!should_tend(false, true, 0.0)); // 沒好感玩家在旁 → 不照料
        assert!(!should_tend(true, false, 0.0)); // 附近沒未熟作物 → 不照料
        assert!(!should_tend(true, true, TEND_CHANCE_PER_TICK)); // 擲骰未命中
    }

    #[test]
    fn should_tend_respects_chance_threshold() {
        assert!(should_tend(true, true, TEND_CHANCE_PER_TICK - 0.001));
        assert!(!should_tend(true, true, TEND_CHANCE_PER_TICK));
        assert!(!should_tend(true, true, 0.99));
    }

    #[test]
    fn nudge_amount_zero_for_nearly_mature_crop() {
        // 剩餘 ≤ 門檻的作物快熟了，乾脆不照料（留給玩家親手收成）。
        assert_eq!(nudge_amount(0), 0);
        assert_eq!(nudge_amount(TEND_MIN_REMAINING_SECS), 0);
    }

    #[test]
    fn nudge_amount_never_crosses_maturity() {
        // 照料後的剩餘秒數永遠 ≥ 門檻（不揠苗助長到瞬間成熟）。
        for remaining in 0..500u64 {
            let n = nudge_amount(remaining);
            assert!(n <= remaining, "推進量不可超過剩餘量");
            if n > 0 {
                assert!(remaining - n >= TEND_MIN_REMAINING_SECS, "照料後必留最後一小段");
            }
        }
    }

    #[test]
    fn nudge_amount_full_nudge_when_plenty_remaining() {
        // 剩餘充足時推進滿額 TEND_NUDGE_SECS。
        assert_eq!(
            nudge_amount(TEND_NUDGE_SECS + TEND_MIN_REMAINING_SECS + 10),
            TEND_NUDGE_SECS
        );
    }

    #[test]
    fn nudge_amount_clamped_near_threshold() {
        // 剛好越過門檻一點點時，只推進到門檻為止、不越界。
        let remaining = TEND_MIN_REMAINING_SECS + 5;
        assert_eq!(nudge_amount(remaining), 5);
    }

    #[test]
    fn crop_name_covers_all_kinds() {
        assert_eq!(crop_name(CropKind::Wheat), "小麥");
        assert_eq!(crop_name(CropKind::Carrot), "胡蘿蔔");
        assert_eq!(crop_name(CropKind::Potato), "馬鈴薯");
    }

    #[test]
    fn say_line_embeds_crop_and_varies_with_pick() {
        let a = tend_say_line("小麥", 0);
        let b = tend_say_line("小麥", 1);
        assert!(a.contains("小麥"));
        assert!(!a.contains("{crop}")); // 佔位符已替換
        assert_ne!(a, b);
    }

    #[test]
    fn say_line_wraps_around_deterministically() {
        assert_eq!(tend_say_line("胡蘿蔔", 0), tend_say_line("胡蘿蔔", 4));
        assert_eq!(tend_say_line("胡蘿蔔", 1), tend_say_line("胡蘿蔔", 5));
    }

    #[test]
    fn memory_line_mentions_player_and_crop() {
        let line = tend_memory_line("旅人", "馬鈴薯");
        assert!(line.contains("旅人"));
        assert!(line.contains("馬鈴薯"));
    }

    #[test]
    fn feed_line_mentions_resident_player_and_crop() {
        let line = tend_feed_line("露娜", "旅人", "胡蘿蔔");
        assert!(line.contains("露娜"));
        assert!(line.contains("旅人"));
        assert!(line.contains("胡蘿蔔"));
    }
}
