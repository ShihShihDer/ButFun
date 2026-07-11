//! 乙太方界·時令豐收 v1（ROADMAP 812）——收在時令的作物，多收一份。
//!
//! 時令作物（811）給季節長出了第一顆玩法牙齒：**種**在時令季節的作物一種下就先抽長一截、
//! 比平時更快成熟。可那顆牙齒只長在「種植端」——收成那一刻，季節又回到了背景：不論你在
//! 哪個季節收割，麥子永遠只掉 1 顆、馬鈴薯永遠只挖 2 顆。這一刀把時令的牙齒補到「收成端」：
//! **當你在某作物的時令季節收割它成熟的植株，會額外多得一份果實**——與 811 對成乾淨的一對：
//! **種在時令長得快／收在時令收得多**。挑對節氣播種、又趕在當季收割，你會實打實地多攢一把
//! 糧食；季節從此不只讓作物長得快，也讓豐收更豐。
//!
//! **療癒優先、只獎不罰**（守資料安全鐵律）：非時令**照常收成、不減產、不損任何玩家資料**，
//! 只是沒有那份額外的當季紅利。冬天萬物歇息（四種作物皆非時令），冬收照常、只是無人得寵。
//!
//! **時令對應沿用 811**（單一真相來源）：🥕 胡蘿蔔→春、🌾 小麥→夏、🥔 馬鈴薯→秋、❄️ 冬無時令。
//! 本模組不重複一份「作物→季節」對照，直接複用 [`crate::voxel_timely::is_in_season`]。
//!
//! **純邏輯層**：確定性、零 LLM、零持久化、零 migration、可窮舉測試；季節取得、額外果實入袋、
//! 回饋廣播全在 `voxel_ws.rs` 的收成（挖成熟作物）路徑接線（比照 738 砍葉附加掉樹苗的既有慣例）。

use crate::voxel::Block;
use crate::voxel_farm::{CropKind, CARROT_ID, POTATO_ID, PUMPKIN_ID, WHEAT_ID};
use crate::voxel_season::Season;
use crate::voxel_timely::{crop_name_zh, is_in_season};

/// 收在時令時額外多得的果實份數。取 +1（穩定、可測、不破壞平衡）：
/// 小麥/胡蘿蔔基礎收 1 → 時令收 2；馬鈴薯基礎收 2（量大是特色）→ 時令收 3。
pub const BOUNTY_EXTRA: u32 = 1;

/// 把「成熟作物方塊」對回它的作物種類；非成熟作物方塊回 `None`。
///
/// 只認三種**成熟**狀態方塊（收割那一刻才觸發豐收判定）——幼苗/未熟狀態不在此列，
/// 挖幼苗只是取消種植（退還種子），不該給豐收紅利。
pub fn crop_kind_of_mature_block(block: Block) -> Option<CropKind> {
    match block {
        Block::WheatMature => Some(CropKind::Wheat),
        Block::CarrotMature => Some(CropKind::Carrot),
        Block::PotatoMature => Some(CropKind::Potato),
        Block::PumpkinMature => Some(CropKind::Pumpkin),
        _ => None,
    }
}

/// 某作物收成後入袋的果實 item id（沿用 `voxel_farm` 既有常數，單一真相來源）。
pub fn crop_item_id(kind: CropKind) -> u8 {
    match kind {
        CropKind::Wheat => WHEAT_ID,     // 18
        CropKind::Carrot => CARROT_ID,   // 49
        CropKind::Potato => POTATO_ID,   // 53
        CropKind::Pumpkin => PUMPKIN_ID, // 110
    }
}

/// 這次收成該給的「時令豐收」額外份數：在時令季節 → [`BOUNTY_EXTRA`]，否則 0。
///
/// 只獎不罰：非時令回 0（照常收成、不減產）；時令回正數（額外多得）。
pub fn harvest_bonus(kind: CropKind, season: Season) -> u32 {
    if is_in_season(kind, season) {
        BOUNTY_EXTRA
    } else {
        0
    }
}

/// 當季鮮採時給玩家的暖回饋句（確定性、嵌作物名與季節名）。
///
/// 僅在 [`harvest_bonus`] > 0（即時令）時呼叫；非時令不冒此句（只獎不罰、不嘮叨）。
pub fn bounty_line(kind: CropKind, season: Season) -> String {
    format!(
        "🌾 當季鮮採！這{}正逢{}的時令，多收了一份沉甸甸的收成。",
        crop_name_zh(kind),
        season.display_name(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 三種成熟作物方塊各自對回正確的作物種類。
    #[test]
    fn mature_block_maps_to_kind() {
        assert_eq!(
            crop_kind_of_mature_block(Block::WheatMature),
            Some(CropKind::Wheat)
        );
        assert_eq!(
            crop_kind_of_mature_block(Block::CarrotMature),
            Some(CropKind::Carrot)
        );
        assert_eq!(
            crop_kind_of_mature_block(Block::PotatoMature),
            Some(CropKind::Potato)
        );
    }

    /// 非成熟作物方塊一律回 None（不觸發豐收）——幼苗、農田土、草、石頭等皆是。
    #[test]
    fn non_mature_blocks_map_to_none() {
        for b in [
            Block::FarmSoilSeeded,
            Block::CarrotSeeded,
            Block::PotatoSeeded,
            Block::FarmSoil,
            Block::Grass,
            Block::Dirt,
            Block::Stone,
            Block::BerryBushRipe,
        ] {
            assert_eq!(crop_kind_of_mature_block(b), None, "{:?} 不該觸發豐收", b);
        }
    }

    /// 作物 item id 沿用 voxel_farm 常數。
    #[test]
    fn crop_item_ids() {
        assert_eq!(crop_item_id(CropKind::Wheat), WHEAT_ID);
        assert_eq!(crop_item_id(CropKind::Carrot), CARROT_ID);
        assert_eq!(crop_item_id(CropKind::Potato), POTATO_ID);
        assert_eq!(crop_item_id(CropKind::Pumpkin), PUMPKIN_ID);
    }

    /// 季限作物·秋南瓜 v1（933）：成熟南瓜方塊對回南瓜；南瓜的時令＝秋，故秋收有豐收紅利、其餘三季 0。
    #[test]
    fn pumpkin_mature_and_autumn_bounty() {
        assert_eq!(crop_kind_of_mature_block(Block::PumpkinMature), Some(CropKind::Pumpkin));
        assert_eq!(harvest_bonus(CropKind::Pumpkin, Season::Autumn), BOUNTY_EXTRA);
        for s in [Season::Spring, Season::Summer, Season::Winter] {
            assert_eq!(harvest_bonus(CropKind::Pumpkin, s), 0, "{s:?} 南瓜不該有豐收紅利");
        }
    }

    /// 時令季節收成 → 額外 BOUNTY_EXTRA；非時令 → 0。
    #[test]
    fn bonus_only_in_season() {
        // 每種作物恰好在自己的時令季節有紅利、其餘三季為 0。
        let all = [Season::Spring, Season::Summer, Season::Autumn, Season::Winter];
        for kind in [CropKind::Wheat, CropKind::Carrot, CropKind::Potato] {
            let rewarded: Vec<Season> = all
                .iter()
                .copied()
                .filter(|&s| harvest_bonus(kind, s) > 0)
                .collect();
            assert_eq!(rewarded.len(), 1, "{:?} 應恰好只有一季有豐收紅利", kind);
            assert_eq!(harvest_bonus(kind, rewarded[0]), BOUNTY_EXTRA);
        }
    }

    /// 冬天四種作物皆無豐收紅利（冬藏）。
    #[test]
    fn winter_no_bounty() {
        for kind in [CropKind::Wheat, CropKind::Carrot, CropKind::Potato] {
            assert_eq!(harvest_bonus(kind, Season::Winter), 0);
        }
    }

    /// 豐收回饋句：非空、嵌得到作物名與季節名。
    #[test]
    fn bounty_line_content() {
        let line = bounty_line(CropKind::Wheat, Season::Summer);
        assert!(line.contains("小麥"));
        assert!(line.contains("夏天"));
        assert!(!line.is_empty());
        assert!(bounty_line(CropKind::Carrot, Season::Spring).contains("胡蘿蔔"));
        assert!(bounty_line(CropKind::Potato, Season::Autumn).contains("馬鈴薯"));
    }
}
