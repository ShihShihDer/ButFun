//! 乙太方界·居民互相以物易物 v1（ROADMAP 723）。
//!
//! **玩家有感**：以物易物（670）讓玩家能跟居民用特長物品交換，但至今這套系統只有
//! 「玩家→居民」單一方向；老朋友互訪（672）一路疊了問候（672）/八卦（694）/
//! 互助蓋家（696）/拌嘴（715）/傳授技能（717），卻從沒讓居民之間用**同一套交易
//! 特長**做過生意——本切片把 670 的特長分類系統第一次接到居民與居民之間，讓小社會
//! 第一次有「內部經濟」在流動，不再只有對玩家單向的生意。
//!
//! **換維度**：672~717 這串到訪戲碼疊的是問候語氣／見聞流通／勞力互助／情緒衝突／
//! 技能傳承，本切片是**物資流通**——到目前為止唯一還沒被到訪劇本用過的角度。
//!
//! 只在到訪沒有觸發互助蓋家（696）/拌嘴（715）/傳授技能（717）時才可能發生
//! （同一次到訪只演一齣戲，鏡像既有優先序）；只在友人層級（Friend）發生。
//!
//! 純邏輯、零 LLM、零新持久化格式（沿用既有 Feed + memory 路徑）、確定性、可測。

use crate::voxel_bonds::BondTier;
use crate::voxel_trade::resident_trade_slot;

/// 居民互相易物觸發機率（比照 teach(0.15)/quarrel(0.12) 同量級，保持稀有感）。
pub const RESIDENT_TRADE_CHANCE: f32 = 0.13;

/// Feed 動態牆種類（分類顯示用）。
pub const FEED_KIND: &str = "居民易物";

/// 是否觸發居民互相易物：僅友人層級、這次到訪未演過其他戲碼、機率骰過。
pub fn should_resident_trade(
    tier: BondTier,
    help_happened: bool,
    quarrel_happened: bool,
    teach_happened: bool,
    roll: f32,
) -> bool {
    tier == BondTier::Friend
        && !help_happened
        && !quarrel_happened
        && !teach_happened
        && roll < RESIDENT_TRADE_CHANCE
}

/// 依居民 id 取得其交易特長物品（沿用 670 `resident_trade_slot` 的 slot 分類，
/// 與 `voxel_trade::make_offer` 的物品對表一致）。
fn specialty_item(resident_id: &str) -> u8 {
    match resident_trade_slot(resident_id) {
        0 => 14, // 種子
        1 => 3,  // 石頭
        2 => 5,  // 木頭
        _ => 10, // 玻璃
    }
}

/// 決定這次互相易物交換的物品對：`(訪客給出的物品, 主人給出的物品)`。
/// 兩人特長剛好同 slot（同款商品）就沒得換——回 `None`（避免「換一樣的東西」的尷尬場面）。
pub fn trade_pair(visitor_id: &str, host_id: &str) -> Option<(u8, u8)> {
    let v_item = specialty_item(visitor_id);
    let h_item = specialty_item(host_id);
    if v_item == h_item {
        None
    } else {
        Some((v_item, h_item))
    }
}

/// Feed 動態牆文案（確定性、面向玩家、留 i18n 空間）。
pub fn trade_feed_line(visitor: &str, host: &str, v_item_name: &str, h_item_name: &str) -> String {
    format!("{visitor} 和 {host} 互相交換了東西：{v_item_name} 換 {h_item_name}")
}

/// 交換完成後，訪客頭頂冒出的台詞（依 pick 取模選句池）。
pub fn trade_say_line(other_name: &str, got_item_name: &str, pick: usize) -> String {
    const LINES: [&str; 4] = [
        "跟{other}換了{item}，划算！",
        "{other}帶了{item}來，正好互相需要。",
        "我們交換了一下，跟{other}拿到了{item}。",
        "跟老朋友做生意最放心，謝謝{other}的{item}。",
    ];
    LINES[pick % LINES.len()]
        .replace("{other}", other_name)
        .replace("{item}", got_item_name)
}

/// 寫進雙方記憶的摘要（各自視角，確定性）。
pub fn trade_memory_line(other_name: &str, gave_name: &str, got_name: &str) -> String {
    format!("和{other_name}互相易物：我給了{gave_name}，換來了{got_name}")
}

// ── 測試 ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_friend_tier_triggers() {
        assert!(!should_resident_trade(BondTier::Stranger, false, false, false, 0.0));
        assert!(!should_resident_trade(BondTier::Acquaintance, false, false, false, 0.0));
        assert!(should_resident_trade(BondTier::Friend, false, false, false, 0.0));
    }

    #[test]
    fn other_dramas_take_priority() {
        assert!(!should_resident_trade(BondTier::Friend, true, false, false, 0.0), "已互助蓋家就不再易物");
        assert!(!should_resident_trade(BondTier::Friend, false, true, false, 0.0), "已拌嘴就不再易物");
        assert!(!should_resident_trade(BondTier::Friend, false, false, true, 0.0), "已傳授技能就不再易物");
    }

    #[test]
    fn chance_boundary_respected() {
        assert!(should_resident_trade(BondTier::Friend, false, false, false, RESIDENT_TRADE_CHANCE - 0.001));
        assert!(!should_resident_trade(BondTier::Friend, false, false, false, RESIDENT_TRADE_CHANCE));
        assert!(!should_resident_trade(BondTier::Friend, false, false, false, 0.99));
    }

    #[test]
    fn trade_pair_differs_when_slots_differ() {
        // vox_res_0..3 依既有慣例分散在不同 slot（見 voxel_trade 既有測試假設）。
        let pair = trade_pair("vox_res_0", "vox_res_1");
        if let Some((v, h)) = pair {
            assert_ne!(v, h, "不同 slot 應給出不同物品");
        }
    }

    #[test]
    fn trade_pair_none_when_same_slot() {
        // 同一個 id 對自己（同 slot）必然回 None。
        assert_eq!(trade_pair("居民甲", "居民甲"), None);
    }

    #[test]
    fn trade_pair_deterministic() {
        let a = trade_pair("vox_res_0", "vox_res_2");
        let b = trade_pair("vox_res_0", "vox_res_2");
        assert_eq!(a, b);
    }

    #[test]
    fn trade_pair_specialty_item_in_known_set() {
        for id in ["vox_res_0", "vox_res_1", "vox_res_2", "vox_res_3", "任意居民"] {
            let item = specialty_item(id);
            assert!(
                [14u8, 3, 5, 10].contains(&item),
                "specialty_item({id})={item} 應落在已知交易物品集合"
            );
        }
    }

    #[test]
    fn feed_line_contains_both_names_and_items() {
        let line = trade_feed_line("露娜", "諾娃", "種子", "石頭");
        assert!(line.contains("露娜"));
        assert!(line.contains("諾娃"));
        assert!(line.contains("種子"));
        assert!(line.contains("石頭"));
    }

    #[test]
    fn say_line_replaces_placeholders() {
        for pick in 0..4 {
            let line = trade_say_line("諾娃", "玻璃", pick);
            assert!(line.contains("諾娃"));
            assert!(line.contains("玻璃"));
            assert!(!line.contains("{other}"));
            assert!(!line.contains("{item}"));
        }
    }

    #[test]
    fn say_line_pick_wraps_safely() {
        // pick 遠大於句池長度也不能 panic。
        let line = trade_say_line("露娜", "木頭", 9999);
        assert!(line.contains("露娜"));
    }

    #[test]
    fn memory_line_mentions_both_items() {
        let line = trade_memory_line("賽勒", "玻璃", "石頭");
        assert!(line.contains("賽勒"));
        assert!(line.contains("玻璃"));
        assert!(line.contains("石頭"));
    }

    #[test]
    fn chance_constant_is_sane_probability() {
        assert!(RESIDENT_TRADE_CHANCE > 0.0 && RESIDENT_TRADE_CHANCE < 1.0);
    }
}
