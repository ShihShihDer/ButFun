//! 老友的餐贈（ROADMAP 332）：相熟度第一次「兌現成東西」。
//!
//! 329 讓玩家能在午餐桌上舉杯、330 把舉杯累積成相熟度、331 讓混熟的 NPC 整日認得你——
//! 可這條城鎮社交弧連走五片，玩家攢到的交情至今全只活在「話語」裡：回敬語氣更熱絡、走過
//! 被點名招呼。reviewer 在 #496／#498／#499 一再點出：這條弧已漸飽和，下一步該讓它
//! 「對玩家有看得見的獲得感」。本切片正面回應：**把交情第一次兌現成實打實的禮物**——
//! 當你向某位 NPC 舉杯、剛好跨進更高一層交情（升點頭之交／升餐桌熟客）的那一刻，這位
//! NPC 會順手把自家行當的一份心意塞進你背包：商人遞來剛溫的城鎮特釀、農展塞給你一袋自家瓜菜、
//! 懸賞官給你跑野地的傷藥……五片社交的累積，頭一回讓玩家真的「拿到東西」。
//!
//! 設計鐵律：
//! - **恰好搭在「跨層」那一刻**（複用 330 `ToastRecord.crossed`）：不需任何新帳本／冷卻，
//!   每對(玩家,NPC)每層只跨一次，故一份禮物每層至多送一次，天然不可farm。
//! - **零 LLM、純查表**：餐贈只查「哪位 NPC、跨哪層 → 送什麼」，數量刻意壓小（低階素材／
//!   食物／少量乙太），對經濟近乎零擾動；不送武器／護甲，免動戰力平衡。
//! - **只屬故鄉七大村落 NPC**：非村落 NPC（旅人／居民／他星商人）一律無餐贈（回 `None`），
//!   與 `lunch_regular::toast_response`／`npc_recognition::recognize_line` 的範圍一致。
//! - **跨進 `Stranger` 不存在**（相熟度只增不減、`Stranger` 是起點非「升入」），故一律 `None`。
//! - 純邏輯可獨立測試；送什麼／送多少集中於本檔，未來調平衡或在地化只動這一處。

use crate::inventory::ItemKind;
use crate::lunch_regular::Familiarity;

/// 一份餐贈：送哪種物品、送幾個。數量刻意壓小以免擾動經濟。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LunchGift {
    pub item: ItemKind,
    pub qty: u32,
}

impl LunchGift {
    const fn of(item: ItemKind, qty: u32) -> Self {
        Self { item, qty }
    }
}

/// 跨進某相熟層級時，該 NPC 餽贈的一份心意。純查表、可測。
///
/// - 升「點頭之交」（`Acquaintance`）：一份輕巧的見面禮（少量自家行當的基礎物產）。
/// - 升「餐桌熟客」（`Regular`）：一份更豐厚的老友厚禮（成品食物／補給或更大份量）。
/// - `Stranger`（起點，非「升入」）與非村落七大 NPC 一律回 `None`。
///
/// 每位 NPC 的餽贈扣合其行當性格，與 330 升層台詞（`tierup_line`）相呼應：
/// 商人「最好的那壇我給你溫上了」→ 城鎮特釀；農展「這地裡的好東西隨便拿」→ 自家瓜菜……
pub fn gift_for(npc_id: &str, tier: Familiarity) -> Option<LunchGift> {
    let gift = match (npc_id, tier) {
        // 商人：果脯雜貨起家，老主顧待遇——見面遞麵包，混熟了溫上一壇城鎮特釀。
        ("merchant", Familiarity::Acquaintance) => LunchGift::of(ItemKind::Bread, 2),
        ("merchant", Familiarity::Regular) => LunchGift::of(ItemKind::TownBrew, 1),
        // 工坊鐵匠：爐邊行當——先抓把木料給你，熟了再添一堆好礦石。
        ("workshop_npc", Familiarity::Acquaintance) => LunchGift::of(ItemKind::Wood, 3),
        ("workshop_npc", Familiarity::Regular) => LunchGift::of(ItemKind::Stone, 5),
        // 懸賞官：跑野地的痛快人——少許乙太賞，混熟了塞兩瓶傷藥讓你路上保命。
        ("bounty_npc", Familiarity::Acquaintance) => LunchGift::of(ItemKind::Ether, 5),
        ("bounty_npc", Familiarity::Regular) => LunchGift::of(ItemKind::HealingPotion, 2),
        // 探險家：走遍星海——分你一點星塵，熟了給你採來的星晶碎片。
        ("expedition_npc", Familiarity::Acquaintance) => LunchGift::of(ItemKind::StarDust, 2),
        ("expedition_npc", Familiarity::Regular) => LunchGift::of(ItemKind::StarCrystalShard, 3),
        // 採購補給：壓箱好料——先請你嚐幾顆蛋，熟了直接煎好兩份端上。
        ("procurement_npc", Familiarity::Acquaintance) => LunchGift::of(ItemKind::Egg, 3),
        ("procurement_npc", Familiarity::Regular) => LunchGift::of(ItemKind::FriedEgg, 2),
        // 農展：自家地裡的瓜菜——先給你挑根胡蘿蔔，熟了塞給你一袋馬鈴薯。
        ("farm_fair_npc", Familiarity::Acquaintance) => LunchGift::of(ItemKind::Carrot, 2),
        ("farm_fair_npc", Familiarity::Regular) => LunchGift::of(ItemKind::Potato, 3),
        // 里長：鎮上自家人——先分你些自種小麥，混熟了敬你一壇城鎮特釀。
        ("village_chief", Familiarity::Acquaintance) => LunchGift::of(ItemKind::WheatGrain, 3),
        ("village_chief", Familiarity::Regular) => LunchGift::of(ItemKind::TownBrew, 1),
        // 其餘（含 Stranger 起點、非村落 NPC）一律無餐贈。
        _ => return None,
    };
    Some(gift)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 七大村落 NPC 在「升點頭之交」「升餐桌熟客」兩個跨層時刻都得有禮可送，
    /// 免得混熟了反而空手——且送的份量恆為正（拿到的是真東西）。
    #[test]
    fn every_village_npc_has_a_gift_for_each_tier_up() {
        let npcs = [
            "merchant",
            "workshop_npc",
            "bounty_npc",
            "expedition_npc",
            "procurement_npc",
            "farm_fair_npc",
            "village_chief",
        ];
        for id in npcs {
            for tier in [Familiarity::Acquaintance, Familiarity::Regular] {
                let g = gift_for(id, tier).unwrap_or_else(|| panic!("{id} 缺 {tier:?} 餐贈"));
                assert!(g.qty > 0, "{id} 的 {tier:?} 餐贈份量得為正");
            }
        }
    }

    /// 起點 `Stranger`（並非「升入」的層級）一律無餐贈：禮只在「跨進更高一層」那刻送。
    #[test]
    fn stranger_tier_never_gifts() {
        assert_eq!(gift_for("merchant", Familiarity::Stranger), None);
        assert_eq!(gift_for("village_chief", Familiarity::Stranger), None);
    }

    /// 非村落七大 NPC 一律無餐贈（與回敬／相認的範圍一致）。
    #[test]
    fn non_village_npc_never_gifts() {
        assert_eq!(gift_for("stranger_npc", Familiarity::Acquaintance), None);
        assert_eq!(gift_for("traveler", Familiarity::Regular), None);
        assert_eq!(gift_for("", Familiarity::Acquaintance), None);
    }

    /// 餐贈刻意不含武器／護甲：純素材／食物／少量乙太，免動戰力與經濟平衡。
    #[test]
    fn gifts_are_never_combat_gear() {
        let npcs = [
            "merchant",
            "workshop_npc",
            "bounty_npc",
            "expedition_npc",
            "procurement_npc",
            "farm_fair_npc",
            "village_chief",
        ];
        // 任何可當武器裝備的物品都不該被當餐贈白送（免動戰力平衡）。
        for id in npcs {
            for tier in [Familiarity::Acquaintance, Familiarity::Regular] {
                if let Some(g) = gift_for(id, tier) {
                    assert!(
                        crate::combat::weapon_from_item(g.item).is_none(),
                        "{id} 的 {tier:?} 餐贈不該是武器（{:?}）",
                        g.item
                    );
                }
            }
        }
    }

    /// 老友厚禮（Regular）不該比見面禮（Acquaintance）寒酸：至少其一在「份量或檔次」上更厚。
    /// 這裡用一個寬鬆斷言——兩層的禮物不該完全相同（送的東西或數量總得有別），
    /// 體現「愈熟、待你愈不同」。
    #[test]
    fn regular_gift_differs_from_acquaintance() {
        let npcs = [
            "merchant",
            "workshop_npc",
            "bounty_npc",
            "expedition_npc",
            "procurement_npc",
            "farm_fair_npc",
            "village_chief",
        ];
        for id in npcs {
            let acq = gift_for(id, Familiarity::Acquaintance);
            let reg = gift_for(id, Familiarity::Regular);
            assert_ne!(acq, reg, "{id} 兩層餐贈不該一模一樣");
        }
    }
}
