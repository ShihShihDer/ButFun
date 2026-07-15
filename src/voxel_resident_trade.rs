//! 乙太方界·居民互相以物易物 v1（ROADMAP 723）＋生計互相依賴 v1（自主提案切片）。
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
//! **生計互相依賴 v1（真缺口）**：989（生計決定交易 v4）把「玩家↔居民」這條交易接上了
//! 居民的生計身分，但自己的 PR 文字誠實留白——「居民之間互易…刻意維持原本的 slot 分類
//! 不變」；於是居民彼此互易時拿出來的東西，至今仍是與生計毫無關係的 id 雜湊（鐵匠可能
//! 換給漁夫一把種子）。回應 reviewer 對 988/989 的方向提醒「生計之間產生互相依賴／交換」
//! ——本刀正是那句話裡「互相依賴」的那一半（989 做的是「物資鏈」那一半）：居民之間互易的
//! 東西改成各自真正的生計產物（[`vocation_specialty_item`]，與 989 共用同一份
//! `voxel_trade::vocation_trade_pair` 對照表），且五種有生計偏好的職業構成一個**環狀依存**
//! （[`wants_from`]）——農夫想要鐵匠的鐵磚修農具、鐵匠想要漁夫的烤魚補力氣、漁夫想要獵人的
//! 火把夜釣照明、獵人想要工匠的木板修獵具、工匠想要農夫的麵包填肚子——五者環環相扣，玩家
//! 撞見兩位生計環上相鄰的居民互易時，兩人會用格外合拍的台詞道謝（[`is_complementary`]），
//! 第一次讓「小社會的內部經濟」看得出誰的生計真的需要誰。**（更新，ROADMAP 1007）**
//! 當初這裡誠實留白「跨村商隊（950）沿用的是舊版 `specialty_item`（依 id 雜湊），本刀
//! 不動它」——1007 已補上這塊：商隊帶出去的貨改用 [`crate::voxel_trade::vocation_trade_pair`]，
//! 舊版 `specialty_item`（依 id 雜湊、與生計無關）自此無人呼叫，予以移除。
//!
//! 純邏輯、零 LLM、零新持久化格式（沿用既有 Feed + memory 路徑）、確定性、可測。

use crate::voxel_bonds::BondTier;
use crate::voxel_trade::vocation_trade_pair;
use crate::voxel_vocation::Vocation;

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

/// 依居民**生計身分**取得其交易特長物品（生計互相依賴 v1）——與 989 玩家↔居民版本
/// 共用同一份 `voxel_trade::vocation_trade_pair` 對照表的「居民提供」那一半，讓居民
/// 之間互易的東西也真的是各自的生計產物（鐵匠一定是鐵磚、漁夫一定是烤魚），不再是
/// 與身分無關的 id 雜湊。商人仍走 `vocation_trade_pair` 既有的雜貨輪替分支。
fn vocation_specialty_item(resident_id: &str, vocation: Vocation) -> u8 {
    vocation_trade_pair(resident_id, vocation).0
}

/// 依生計決定她「特別想要」哪個生計的產物——五種有生計偏好的職業構成一個環狀依存：
/// 農夫想要鐵匠的鐵磚（修農具）→鐵匠想要漁夫的烤魚（補力氣）→漁夫想要獵人的火把
/// （夜釣照明）→獵人想要工匠的木板（修獵具）→工匠想要農夫的麵包（填肚子），五者
/// 環環相扣、首尾相連。商人不進環（什麼都收，無所謂特別想要誰，回 `None`）。
pub fn wants_from(vocation: Vocation) -> Option<Vocation> {
    match vocation {
        Vocation::Farmer => Some(Vocation::Smith),
        Vocation::Smith => Some(Vocation::Fisher),
        Vocation::Fisher => Some(Vocation::Hunter),
        Vocation::Hunter => Some(Vocation::Artisan),
        Vocation::Artisan => Some(Vocation::Farmer),
        Vocation::Merchant => None,
    }
}

/// 這對居民互易是否恰好合乎彼此生計的環狀依存（[`wants_from`] 任一方向命中即算合拍）。
pub fn is_complementary(a: Vocation, b: Vocation) -> bool {
    wants_from(a) == Some(b) || wants_from(b) == Some(a)
}

/// 決定這次互相易物交換的物品對：`(訪客給出的物品, 主人給出的物品)`——生計互相依賴 v1
/// 起，兩人拿出來的東西真的是各自的生計產物，不再是與身分無關的雜湊。兩人生計剛好相同
/// （同款產物）就沒得換——回 `None`（避免「換一樣的東西」的尷尬場面）。
pub fn trade_pair(
    visitor_id: &str,
    visitor_vocation: Vocation,
    host_id: &str,
    host_vocation: Vocation,
) -> Option<(u8, u8)> {
    let v_item = vocation_specialty_item(visitor_id, visitor_vocation);
    let h_item = vocation_specialty_item(host_id, host_vocation);
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

/// 交換完成後，訪客頭頂冒出的台詞（依 pick 取模選句池）。`complementary`＝這對生計恰好
/// 環環相扣（[`is_complementary`]）時，換一組格外合拍的台詞，讓玩家聽得出「找對人辦對事」。
pub fn trade_say_line(other_name: &str, got_item_name: &str, complementary: bool, pick: usize) -> String {
    const LINES: [&str; 4] = [
        "跟{other}換了{item}，划算！",
        "{other}帶了{item}來，正好互相需要。",
        "我們交換了一下，跟{other}拿到了{item}。",
        "跟老朋友做生意最放心，謝謝{other}的{item}。",
    ];
    const COMPLEMENTARY_LINES: [&str; 3] = [
        "正好缺{item}，{other}來得正是時候！",
        "難怪人都說找對人辦對事——跟{other}換到{item}，剛剛好。",
        "{other}的{item}，我這行當正用得上，謝了！",
    ];
    let pool: &[&str] = if complementary { &COMPLEMENTARY_LINES } else { &LINES };
    pool[pick % pool.len()]
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
    fn trade_pair_differs_when_vocations_differ() {
        // 不同生計必給出不同的生計產物。
        let pair = trade_pair("vox_res_0", Vocation::Farmer, "vox_res_1", Vocation::Smith);
        let (v, h) = pair.expect("不同生計應能互易");
        assert_ne!(v, h, "不同生計應給出不同產物");
        assert_eq!(v, 19, "農夫給出的應是麵包");
        assert_eq!(h, 23, "鐵匠給出的應是鐵磚");
    }

    #[test]
    fn trade_pair_none_when_same_vocation() {
        // 兩人剛好同生計（同款產物）就沒得換。
        assert_eq!(trade_pair("居民甲", Vocation::Fisher, "居民乙", Vocation::Fisher), None);
    }

    #[test]
    fn trade_pair_deterministic() {
        let a = trade_pair("vox_res_0", Vocation::Hunter, "vox_res_2", Vocation::Artisan);
        let b = trade_pair("vox_res_0", Vocation::Hunter, "vox_res_2", Vocation::Artisan);
        assert_eq!(a, b);
    }

    #[test]
    fn wants_from_forms_a_five_cycle_excluding_merchant() {
        use Vocation::*;
        // 從農夫出發沿環走五步應回到農夫本身，且五步走過五個不同生計（不含商人）。
        let mut cur = Farmer;
        let mut seen: Vec<Vocation> = Vec::new();
        for _ in 0..5 {
            seen.push(cur);
            cur = wants_from(cur).expect("環上每一站都該有下一站");
        }
        assert_eq!(cur, Farmer, "五步應繞回起點，形成閉環");
        for (i, a) in seen.iter().enumerate() {
            for (j, b) in seen.iter().enumerate() {
                if i != j {
                    assert_ne!(a, b, "環應涵蓋五種互不重複的職業");
                }
            }
        }
        assert!(!seen.contains(&Merchant), "商人不進環");
        assert_eq!(wants_from(Merchant), None, "商人無所謂特別想要誰");
    }

    #[test]
    fn is_complementary_symmetric_and_matches_cycle() {
        assert!(is_complementary(Vocation::Farmer, Vocation::Smith));
        assert!(is_complementary(Vocation::Smith, Vocation::Farmer), "應對稱");
        assert!(!is_complementary(Vocation::Farmer, Vocation::Fisher), "非相鄰不算合拍");
        assert!(!is_complementary(Vocation::Merchant, Vocation::Farmer), "商人不進環，不算合拍");
        assert!(!is_complementary(Vocation::Farmer, Vocation::Farmer), "同生計不算合拍");
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
        for complementary in [false, true] {
            for pick in 0..4 {
                let line = trade_say_line("諾娃", "玻璃", complementary, pick);
                assert!(line.contains("諾娃"));
                assert!(line.contains("玻璃"));
                assert!(!line.contains("{other}"));
                assert!(!line.contains("{item}"));
            }
        }
    }

    #[test]
    fn say_line_pick_wraps_safely() {
        // pick 遠大於句池長度也不能 panic（兩個句池都要驗）。
        assert!(trade_say_line("露娜", "木頭", false, 9999).contains("露娜"));
        assert!(trade_say_line("露娜", "木頭", true, 9999).contains("露娜"));
    }

    #[test]
    fn say_line_pools_differ_by_complementary_flag() {
        // 同一 pick 落在不同句池，理當不會湊出一模一樣的句子（避免兩池其實共用同一份文案）。
        let generic = trade_say_line("諾娃", "麵包", false, 0);
        let matched = trade_say_line("諾娃", "麵包", true, 0);
        assert_ne!(generic, matched);
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
