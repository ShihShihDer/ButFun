//! 合成儀式（ROADMAP 388）。
//!
//! 玩家在工作台首次鍛造高階物品時，向全服廣播喜訊，讓工匠的成就成為世界事件。
//! 追蹤「世界首次」紀錄：全服史上第一位合成某件稀有物品的玩家享有特殊廣播。
//!
//! 純邏輯、零 IO、零持久化、零 migration、零 LLM。

use std::collections::HashSet;

/// 會觸發合成儀式的配方 id → 繁中物品名。
///
/// 只含高階物品（工具鏈尾端 / 稀有材料製品）；
/// 入門配方（鎬子、基礎武器）沉默合成，不佔世界頻道。
pub const CEREMONIAL: &[(&str, &str)] = &[
    ("reinforced_pickaxe", "強化鎬"),
    ("crystal_blade", "晶石之刃"),
    ("coral_lance", "珊瑚矛"),
    ("meadow_amulet", "草原護符"),
    ("crystal_shield", "晶石護盾"),
    ("pearl_potion", "珍珠復原藥"),
    ("star_crystal_blade", "星晶之刃"),
];

/// 若 `recipe_id` 屬儀式配方，回傳繁中物品名；否則回 None。
pub fn is_ceremonial(recipe_id: &str) -> Option<&'static str> {
    CEREMONIAL
        .iter()
        .find(|(id, _)| *id == recipe_id)
        .map(|(_, name)| *name)
}

/// 合成儀式狀態：追蹤哪些配方已有「世界首次」合成紀錄。
///
/// 記憶體模式，重啟清零（全服從新開始積累工藝記憶）。
pub struct CraftCeremonyState {
    /// 已有世界首次的配方 id 集合（static str 省分配）。
    world_firsts: HashSet<&'static str>,
}

impl CraftCeremonyState {
    pub fn new() -> Self {
        Self {
            world_firsts: HashSet::new(),
        }
    }

    /// 記錄此次合成，回傳 `true` 若為世界首次（歷史上第一位合成此物）。
    ///
    /// `recipe_id` 不在 CEREMONIAL 列表時仍靜默回 false（呼叫端通常先 `is_ceremonial` 過濾）。
    pub fn record(&mut self, recipe_id: &str) -> bool {
        // 找到 static str 鍵（才能存進 HashSet<&'static str>）
        if let Some((id, _)) = CEREMONIAL.iter().find(|(id, _)| *id == recipe_id) {
            self.world_firsts.insert(id)  // insert 回 true ＝ 此前不存在 ＝ 世界首次
        } else {
            false
        }
    }
}

impl Default for CraftCeremonyState {
    fn default() -> Self {
        Self::new()
    }
}

/// 合成儀式的世界頻道文字（純函式，i18n 集中替換點）。
pub fn ceremony_text(player_name: &str, item_name: &str, world_first: bool) -> String {
    if world_first {
        format!(
            "⚒️ 🌟 世界首次！{} 在工作台鍛造了【{}】，這一刻永遠留在世界的工藝記憶中！",
            player_name, item_name
        )
    } else {
        format!(
            "⚒️ {} 在工作台鍛造了【{}】！",
            player_name, item_name
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ceremonial_recipes_recognized() {
        assert_eq!(is_ceremonial("crystal_blade"), Some("晶石之刃"));
        assert_eq!(is_ceremonial("coral_lance"), Some("珊瑚矛"));
        assert_eq!(is_ceremonial("reinforced_pickaxe"), Some("強化鎬"));
        assert_eq!(is_ceremonial("pearl_potion"), Some("珍珠復原藥"));
        assert_eq!(is_ceremonial("star_crystal_blade"), Some("星晶之刃"));
    }

    #[test]
    fn common_recipes_not_ceremonial() {
        assert_eq!(is_ceremonial("weapon"), None);
        assert_eq!(is_ceremonial("pickaxe"), None);
        assert_eq!(is_ceremonial("healing_potion"), None);
        assert_eq!(is_ceremonial("unknown_recipe"), None);
    }

    #[test]
    fn first_craft_is_world_first() {
        let mut state = CraftCeremonyState::new();
        assert!(state.record("crystal_blade"), "首次合成應回傳 true");
    }

    #[test]
    fn second_craft_not_world_first() {
        let mut state = CraftCeremonyState::new();
        state.record("crystal_blade");
        assert!(!state.record("crystal_blade"), "重複合成應回傳 false");
    }

    #[test]
    fn different_recipes_independent() {
        let mut state = CraftCeremonyState::new();
        assert!(state.record("crystal_blade"));
        assert!(state.record("coral_lance"), "不同配方各自獨立");
        assert!(!state.record("crystal_blade"));
    }

    #[test]
    fn non_ceremonial_record_is_false() {
        let mut state = CraftCeremonyState::new();
        assert!(!state.record("pickaxe"), "非儀式配方不計入世界首次");
        assert!(!state.record("weapon"));
    }

    #[test]
    fn ceremony_text_world_first_format() {
        let text = ceremony_text("勇者小明", "晶石之刃", true);
        assert!(text.contains("世界首次"), "世界首次應含關鍵字");
        assert!(text.contains("勇者小明"));
        assert!(text.contains("晶石之刃"));
    }

    #[test]
    fn ceremony_text_normal_format() {
        let text = ceremony_text("小花", "珊瑚矛", false);
        assert!(!text.contains("世界首次"), "一般儀式不含世界首次");
        assert!(text.contains("小花"));
        assert!(text.contains("珊瑚矛"));
    }
}
