//! 乙太方界·居民見賢思齊 v1（voxel-envy，自主提案切片，ROADMAP 858）。
//!
//! **真缺口**：居民的渴望（`voxel_desires`）至今只從三個來源萌生——玩家對話
//! （`extract_desire`）、居民自己的禱告（`SELF_SPARK`）、好奇心自主學習
//! （`CURIOSITY_SPARK`）；世界裡真實存在的事物——例如 773/854 讓玩家蓋起、被居民
//! 命名記住的一座座地標——從沒有觸發過任何居民的心願。一位居民路過「風車丘」，
//! 看著它，卻從沒有因為「親眼看見這麼美的東西」而心生嚮往。本刀補上這第四個心願
//! 來源：**居民路過一座已命名的地標，偶爾會心生嚮往，也想擁有一座屬於自己的類似
//! 建物**——這是世界第一次讓「環境本身」而非「對話」直接驅動居民的心願，呼應
//! PLAN_ETHERVOX「記憶要驅動行為」的又一塊拼圖：這次驅動的不是記憶，而是**親眼所見**。
//!
//! **與既有的定位區隔**：
//! - `voxel_admire`/`voxel_structure_name`（773/854）是「居民對**玩家**的反應」
//!   （讚賞、命名、喚名）；本刀是「居民對**地標本身**的反應」——地標不必是玩家親自
//!   造訪時才觸發，任何居民閒晃路過都可能心生嚮往，與玩家在不在場無關。
//! - `voxel_desires` 其他心願來源都是「有人告訴她 / 她自己想」；本刀是「她親眼看見
//!   才知道自己想要」，是唯一由**環境**（而非對話或內省）驅動的心願來源。
//!
//! **純邏輯層**：是否觸發（[`should_envy`]）、羨慕哪種建物（[`pick_envy_kind`]）、
//! 心願文字（[`envy_desire_text`]）、當下的讚嘆台詞（[`envy_say_line`]）、動態牆播報
//! （[`envy_feed_line`]）全是確定性純函式，零 LLM、零鎖、零 IO。冷卻計時 / 地標查詢 /
//! 心願寫入全在 `voxel_ws.rs`，沿用既有「回饋糧倉」（857）那條已驗證的鎖序與冷卻節流
//! 慣例。
//!
//! **成本 / 濫用防護**：心願文字全走固定模板（含既有 `BuildKind::display_name`），
//! 永不夾帶玩家原話；長冷卻（[`ENVY_COOLDOWN_SECS`]）配合每 tick 低機率
//! （[`ENVY_CHANCE_PER_TICK`]），稀有有份量、天然防洗版。零 migration（心願沿用既有
//! `voxel_desires` append-only 持久化）、零新協議欄位、零新美術、FPS 零影響（僅每
//! 100ms 一次 O(1) 分格查表）。

use crate::voxel_building::BuildKind;

/// 觸發「見賢思齊」心願的每 tick 機率（冷卻期滿後才開始骰，比照 857 回饋糧倉同量級）。
pub const ENVY_CHANCE_PER_TICK: f32 = 0.02;

/// 同一位居民兩次「見賢思齊」心願之間的冷卻秒數：長冷卻讓「因為看到美的東西而心生
/// 嚮往」這件事稀有有份量，不會每次路過同一座地標就反覆嚮往。
pub const ENVY_COOLDOWN_SECS: f32 = 600.0;

/// 見賢思齊泡泡的字元上限（依字元非位元組，繁中安全，永不破泡泡框）。
pub const ENVY_SAY_MAX_CHARS: usize = 40;

/// 居民可能嚮往的建物種類池：刻意只挑 `voxel_building::classify_desire` 認得的關鍵詞
/// 對應種類，讓這份心願之後真能被既有建造系統接手蓋成真。
const KINDS: &[BuildKind] = &[
    BuildKind::House,
    BuildKind::Well,
    BuildKind::Tower,
    BuildKind::Garden,
    BuildKind::Pavilion,
];

/// 依 `pick` 從常見建物種類中選一種居民嚮往的建物（純函式、確定性、越界安全取模）。
pub fn pick_envy_kind(pick: usize) -> BuildKind {
    KINDS[pick % KINDS.len()]
}

/// 是否該觸發見賢思齊心願（純函式）：正站在一座已命名地標旁（冷卻由呼叫端的計時器
/// 分支把關）+ 過機率門檻才觸發。
pub fn should_envy(near_named_structure: bool, roll: f32) -> bool {
    near_named_structure && roll < ENVY_CHANCE_PER_TICK
}

/// 截斷輔助：保留至多 [`ENVY_SAY_MAX_CHARS`] 個字元。
fn truncate_chars(s: &str) -> String {
    s.chars().take(ENVY_SAY_MAX_CHARS).collect()
}

/// 見賢思齊心願文字（純函式）：含地標名 + 建物種類關鍵詞，讓
/// `voxel_building::classify_desire` 認得出來、之後真能被建造系統接手蓋成真。
pub fn envy_desire_text(structure_name: &str, kind: BuildKind) -> String {
    format!("看到「{structure_name}」那麼特別，我也好想擁有一座自己的{}", kind.display_name())
}

/// 心生嚮往那一刻的泡泡台詞（繁中、面向玩家、i18n 集中於此；確定性依 `pick` 選句，
/// 永不含玩家原話，無注入 / NSFW 風險）。
pub fn envy_say_line(structure_name: &str, kind: BuildKind, pick: usize) -> String {
    const LINES: &[&str] = &[
        "「{n}」蓋得真美……我也好想有一座自己的{k}。",
        "每次經過「{n}」我都在想，要是我也有一座{k}該多好。",
        "看著「{n}」，我心裡忽然冒出一個念頭——我也想蓋一座{k}。",
        "「{n}」真的很打動我，也許有一天我也能擁有一座{k}。",
    ];
    let line = LINES[pick % LINES.len()]
        .replace("{n}", structure_name)
        .replace("{k}", kind.display_name());
    truncate_chars(&line)
}

/// 動態牆播報（純函式，不含玩家原話，只嵌居民名 / 地標名 / 建物種類）。
pub fn envy_feed_line(resident_name: &str, structure_name: &str, kind: BuildKind) -> String {
    format!(
        "{resident_name}看著「{structure_name}」，心裡冒出了想蓋一座{}的念頭",
        kind.display_name()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── pick_envy_kind ───────────────────────────────────────────────────────
    #[test]
    fn pick_envy_kind_in_bounds_for_any_pick() {
        for pick in 0..50 {
            let _ = pick_envy_kind(pick); // 不 panic 即通過（越界安全取模）
        }
    }

    #[test]
    fn pick_envy_kind_deterministic() {
        assert_eq!(pick_envy_kind(3), pick_envy_kind(3));
        assert_eq!(pick_envy_kind(3), pick_envy_kind(3 + KINDS.len()));
    }

    // ── should_envy ──────────────────────────────────────────────────────────
    #[test]
    fn should_envy_requires_named_structure() {
        assert!(!should_envy(false, 0.0));
        assert!(should_envy(true, 0.0));
    }

    #[test]
    fn should_envy_requires_roll_under_chance() {
        assert!(should_envy(true, ENVY_CHANCE_PER_TICK - 0.001));
        assert!(!should_envy(true, ENVY_CHANCE_PER_TICK));
        assert!(!should_envy(true, 1.0));
    }

    // ── envy_desire_text / classify_desire 相容性 ───────────────────────────
    #[test]
    fn envy_desire_text_classifiable_for_every_kind() {
        for &kind in KINDS {
            let text = envy_desire_text("風車丘", kind);
            assert!(text.contains("風車丘"));
            assert_eq!(
                crate::voxel_building::classify_desire(&text),
                Some(kind),
                "envy 心願文字必須能被既有建造系統分類，種類={kind:?}"
            );
        }
    }

    // ── envy_say_line ────────────────────────────────────────────────────────
    #[test]
    fn envy_say_line_contains_name_and_kind_and_fits_frame() {
        for pick in 0..4 {
            let line = envy_say_line("望遠橋", BuildKind::Tower, pick);
            assert!(line.contains("望遠橋"));
            assert!(line.contains("瞭望台"));
            assert!(line.chars().count() <= ENVY_SAY_MAX_CHARS);
        }
    }

    #[test]
    fn envy_say_line_rotates_deterministically() {
        let a = envy_say_line("月台", BuildKind::Well, 0);
        let b = envy_say_line("月台", BuildKind::Well, 1);
        assert_ne!(a, b);
        assert_eq!(a, envy_say_line("月台", BuildKind::Well, 0));
    }

    #[test]
    fn envy_say_line_never_breaks_frame_even_with_long_structure_name() {
        let long_name = "超級無敵長的地標名字測試用超級無敵長的地標名字測試用超級無敵長的地標名字測試用";
        let line = envy_say_line(long_name, BuildKind::House, 0);
        assert!(line.chars().count() <= ENVY_SAY_MAX_CHARS);
    }

    // ── envy_feed_line ───────────────────────────────────────────────────────
    #[test]
    fn envy_feed_line_mentions_resident_structure_and_kind() {
        let line = envy_feed_line("露娜", "風車丘", BuildKind::Garden);
        assert!(line.contains("露娜"));
        assert!(line.contains("風車丘"));
        assert!(line.contains("花圃"));
    }
}
