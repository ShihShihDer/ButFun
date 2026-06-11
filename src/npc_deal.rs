//! ROADMAP 101: AI 議價交易——商人能執行她在對話裡談成的交易。
//!
//! 設計鐵律：
//! - **腦子自由、手有界**：LLM 只決定要不要議價、議多少，引擎做最終驗證與執行。
//! - **不無中生有**：只允許收購 NPC 收購清單（NPC_BUY_LIST）上的物品；金庫扣真錢。
//! - **引擎定價**：物品價格必須在合理範圍（基準價～MAX_DEAL_PRICE_MULTIPLIER 倍基準價）。
//! - **不信 LLM 數字**：引擎自己查 NPC_BUY_LIST 的基準價，不採用 LLM 報的當依據。
//! - **注入防護**：`item_from_str` 只接受白名單內的字串，完全阻斷任意字串注入。
//! - 玩家背包確認在 ConfirmDeal handler（而非提議時），防止時間差攻擊。

use std::time::{Duration, Instant};
use crate::inventory::ItemKind;

/// LLM 議價暗號前綴。完整格式：`[DEAL item qty price]`。
/// 例如 `[DEAL wood 10 3]` = 商人想以 3 乙太/個收購 10 個木材。
/// 引擎偵測到後：解析 → 驗證 → 存入 PendingDeal → 送 DealOffer 給玩家確認。
pub const DEAL_TOKEN_PREFIX: &str = "[DEAL";

/// 單筆議價最大數量上限。防 LLM 亂報超大數。
pub const MAX_DEAL_QTY: u32 = 50;

/// 議價價格上限倍率：提議價格不得超過 NPC 基準收購價的此倍數。
/// 防 LLM 亂報天文數字或不合理的高價。
pub const MAX_DEAL_PRICE_MULTIPLIER: u32 = 3;

/// 待用議價有效期（秒）。超過後引擎自動視為過期。3 分鐘夠玩家看到並決定。
pub const DEAL_EXPIRE_SECS: u64 = 180;

/// 一筆待玩家確認的議價交易（商人提議、引擎已驗證合理性）。
#[derive(Clone)]
pub struct PendingDeal {
    /// 物品種類（引擎驗證過的 ItemKind，不使用 LLM 原始字串）。
    pub item: ItemKind,
    /// 數量。
    pub qty: u32,
    /// 每個物品的乙太價格（≥ 基準價、≤ 3x 基準價）。
    pub price_per: u32,
    /// 到期時刻。
    pub expires: Instant,
}

impl PendingDeal {
    pub fn is_expired(&self) -> bool {
        Instant::now() >= self.expires
    }
    pub fn total(&self) -> u32 {
        self.price_per.saturating_mul(self.qty)
    }
}

/// 白名單：物品代碼字串 → ItemKind（只允許故鄉 NPC 收購清單上的物品）。
/// 完全阻斷注入：任何不在此清單的字串一律回 None。
pub fn item_from_str(s: &str) -> Option<ItemKind> {
    match s.trim() {
        "wood"               => Some(ItemKind::Wood),
        "stone"              => Some(ItemKind::Stone),
        "ether"              => Some(ItemKind::Ether),
        "dirt"               => Some(ItemKind::Dirt),
        "crystal_shard"      => Some(ItemKind::CrystalShard),
        "mushroom_spore"     => Some(ItemKind::MushroomSpore),
        "ancient_fragment"   => Some(ItemKind::AncientFragment),
        "deep_sea_pearl"     => Some(ItemKind::DeepSeaPearl),
        "wildflower_seed"    => Some(ItemKind::WildflowerSeed),
        "rift_shard"         => Some(ItemKind::RiftShard),
        "fish_small"         => Some(ItemKind::FishSmall),
        "fish_star"          => Some(ItemKind::FishStar),
        "fish_deep"          => Some(ItemKind::FishDeep),
        "egg"                => Some(ItemKind::Egg),
        "wheat_grain"        => Some(ItemKind::WheatGrain),
        "carrot"             => Some(ItemKind::Carrot),
        "potato"             => Some(ItemKind::Potato),
        "star_crystal_shard" => Some(ItemKind::StarCrystalShard),
        _                    => None,
    }
}

/// ItemKind → 中文顯示名（前端議價確認對話框用）。
pub fn item_display_zh(item: ItemKind) -> &'static str {
    match item {
        ItemKind::Wood             => "木材",
        ItemKind::Stone            => "石頭",
        ItemKind::Ether            => "乙太礦",
        ItemKind::Dirt             => "泥土",
        ItemKind::CrystalShard     => "晶石碎片",
        ItemKind::MushroomSpore    => "蕈菇孢子",
        ItemKind::AncientFragment  => "古代碎片",
        ItemKind::DeepSeaPearl     => "深海珍珠",
        ItemKind::WildflowerSeed   => "野花種子",
        ItemKind::RiftShard        => "裂縫碎片",
        ItemKind::FishSmall        => "小魚",
        ItemKind::FishStar         => "星星魚",
        ItemKind::FishDeep         => "深海魚",
        ItemKind::Egg              => "雞蛋",
        ItemKind::WheatGrain       => "小麥穗",
        ItemKind::Carrot           => "胡蘿蔔",
        ItemKind::Potato           => "馬鈴薯",
        ItemKind::StarCrystalShard => "星晶碎片",
        _                          => "物品",
    }
}

/// NPC 基準收購價（查 NPC_BUY_LIST；引擎自己查，不信 LLM 提供的數字）。
pub fn base_buy_price(item: ItemKind) -> Option<u32> {
    crate::npc::NPC_BUY_LIST
        .iter()
        .find(|e| e.item == item)
        .map(|e| e.price_per)
}

/// 從 LLM 原始回話中解析議價暗號 `[DEAL item qty price]`。
/// 回傳 `(Some((item_str, qty, price_per)), 抽掉暗號後的乾淨回話)`。
/// 若不含暗號或格式錯誤，回傳 `(None, 原始回話)`；無論如何，暗號本身都會從回話中抽掉。
pub fn extract_deal(raw: &str) -> (Option<(String, u32, u32)>, String) {
    let start = match raw.find(DEAL_TOKEN_PREFIX) {
        Some(i) => i,
        None => return (None, raw.to_string()),
    };
    let end = match raw[start..].find(']') {
        Some(i) => start + i,
        None => return (None, raw.to_string()),
    };
    // 暗號括號內容（去掉 "[DEAL" 前綴）：" wood 10 3"
    let token_content = &raw[start + DEAL_TOKEN_PREFIX.len()..end];
    let clean = format!("{}{}", &raw[..start], &raw[end + 1..]).trim().to_string();
    let parts: Vec<&str> = token_content.split_whitespace().collect();
    if parts.len() < 3 {
        return (None, clean);
    }
    let qty: u32 = match parts[1].parse::<u32>() {
        Ok(v) if v > 0 => v,
        _ => return (None, clean),
    };
    let price_per: u32 = match parts[2].parse::<u32>() {
        Ok(v) if v > 0 => v,
        _ => return (None, clean),
    };
    (Some((parts[0].to_string(), qty, price_per)), clean)
}

/// 驗證 LLM 提議的議價是否合理，全部通過則回傳 `Ok(PendingDeal)`。
///
/// 護欄（詳見 ROADMAP 101）：
/// 1. item 必須在白名單（`item_from_str`），阻斷注入
/// 2. qty 在 1..=MAX_DEAL_QTY
/// 3. price_per ≥ base_price（不低於正常收購價，保護玩家）
/// 4. price_per ≤ base_price × MAX_DEAL_PRICE_MULTIPLIER（防天文數字）
/// 5. treasury_balance ≥ 總額（金庫真實扣減，不無中生有）
///
/// 注意：玩家背包數量的最終確認在 ConfirmDeal handler 執行，防止時間差攻擊。
pub fn validate_deal(
    item_str: &str,
    qty: u32,
    price_per: u32,
    treasury_balance: u32,
) -> Result<PendingDeal, &'static str> {
    let item = item_from_str(item_str).ok_or("物品不在議價白名單")?;
    if qty == 0 || qty > MAX_DEAL_QTY {
        return Err("數量不在合理範圍（1–50）");
    }
    let base = base_buy_price(item).ok_or("找不到基準收購價")?;
    if price_per < base {
        return Err("提議價格低於基準收購價");
    }
    let max_price = base.saturating_mul(MAX_DEAL_PRICE_MULTIPLIER);
    if price_per > max_price {
        return Err("提議價格超過基準 3 倍上限");
    }
    let total = price_per.saturating_mul(qty);
    if treasury_balance < total {
        return Err("金庫餘額不足");
    }
    Ok(PendingDeal {
        item,
        qty,
        price_per,
        expires: Instant::now() + Duration::from_secs(DEAL_EXPIRE_SECS),
    })
}

/// 商人議價提示：注入 LLM system prompt，讓商人知道她有議價能力。
/// 以「建議」而非「指令」的口吻撰寫，符合「腦子自由、手有界」的設計：
/// 商人自己判斷要不要議價，引擎只負責驗證與執行。
pub const MERCHANT_DEAL_HINT: &str = "\
\n\n如果玩家表達想以更高價賣出某樣物品，你可以**自己決定**是否提議議價——\
只需在回話句末加上暗號 [DEAL 物品代碼 數量 每個價格]，例如 [DEAL wood 10 3] \
表示你想以 3 乙太/個收購 10 個木材。引擎會驗證後讓玩家確認；你若不感興趣就別加暗號。\
可議價物品（代碼=中文名・基準收購價）：\
wood=木材・1, stone=石頭・1, ether=乙太礦・2, dirt=泥土・1, \
crystal_shard=晶石碎片・3, mushroom_spore=蕈菇孢子・2, ancient_fragment=古代碎片・4, \
deep_sea_pearl=深海珍珠・5, wildflower_seed=野花種子・2, rift_shard=裂縫碎片・12, \
fish_small=小魚・2, fish_star=星星魚・5, fish_deep=深海魚・10, \
egg=雞蛋・2, wheat_grain=小麥穗・2, carrot=胡蘿蔔・3, potato=馬鈴薯・4, \
star_crystal_shard=星晶碎片・5。\
注意：提議價格只能在基準價到基準價 3 倍之間（超出引擎靜默拒絕）；每人同時只能有一筆待確認議價。";

// ─── 純邏輯單元測試 ────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_from_str_whitelist_accepts_known_items() {
        assert_eq!(item_from_str("wood"), Some(ItemKind::Wood));
        assert_eq!(item_from_str("crystal_shard"), Some(ItemKind::CrystalShard));
        assert_eq!(item_from_str("fish_deep"), Some(ItemKind::FishDeep));
        assert_eq!(item_from_str("star_crystal_shard"), Some(ItemKind::StarCrystalShard));
    }

    #[test]
    fn item_from_str_rejects_unknown() {
        assert!(item_from_str("hack").is_none());
        // 星球碎片不在故鄉收購清單
        assert!(item_from_str("jade_shard").is_none());
        assert!(item_from_str("lava_crystal").is_none());
        assert!(item_from_str("").is_none());
    }

    #[test]
    fn item_from_str_rejects_injection_attempts() {
        assert!(item_from_str("; DROP TABLE players; --").is_none());
        assert!(item_from_str("[GIFT]").is_none());
        assert!(item_from_str("[DISCOUNT]").is_none());
    }

    #[test]
    fn base_buy_price_returns_correct_values() {
        assert_eq!(base_buy_price(ItemKind::Wood), Some(1));
        assert_eq!(base_buy_price(ItemKind::Ether), Some(2));
        assert_eq!(base_buy_price(ItemKind::RiftShard), Some(12));
        assert_eq!(base_buy_price(ItemKind::DeepSeaPearl), Some(5));
    }

    #[test]
    fn base_buy_price_absent_for_non_buyable() {
        // 武器不在收購清單
        assert!(base_buy_price(ItemKind::Weapon).is_none());
    }

    #[test]
    fn extract_deal_parses_valid_token() {
        let raw = "好吧，我以 3 乙太/個收你的木材！[DEAL wood 10 3]";
        let (parsed, clean) = extract_deal(raw);
        let (item_str, qty, price) = parsed.unwrap();
        assert_eq!(item_str, "wood");
        assert_eq!(qty, 10);
        assert_eq!(price, 3);
        assert!(!clean.contains("[DEAL"), "乾淨回話不應含暗號");
        assert!(clean.contains("好吧"));
    }

    #[test]
    fn extract_deal_no_token_returns_none() {
        let raw = "今天生意還不錯，謝謝光臨！";
        let (parsed, clean) = extract_deal(raw);
        assert!(parsed.is_none());
        assert_eq!(clean, raw);
    }

    #[test]
    fn extract_deal_malformed_token_stripped_but_no_deal() {
        // 格式不足時：暗號被抽掉，但不觸發議價。
        let raw = "我想提議[DEAL wood]這樣。";
        let (parsed, clean) = extract_deal(raw);
        assert!(parsed.is_none());
        assert!(!clean.contains("[DEAL"), "格式錯的暗號也應被抽掉");
    }

    #[test]
    fn extract_deal_zero_qty_returns_none() {
        let raw = "試試[DEAL wood 0 5]看。";
        let (parsed, _) = extract_deal(raw);
        assert!(parsed.is_none());
    }

    #[test]
    fn validate_deal_succeeds_for_valid_input() {
        let result = validate_deal("wood", 5, 2, 100);
        assert!(result.is_ok());
        let deal = result.unwrap();
        assert_eq!(deal.item, ItemKind::Wood);
        assert_eq!(deal.qty, 5);
        assert_eq!(deal.price_per, 2);
        assert_eq!(deal.total(), 10);
        assert!(!deal.is_expired());
    }

    #[test]
    fn validate_deal_accepts_at_base_price() {
        // 等於基準價也合法（不硬要溢價）
        let result = validate_deal("wood", 3, 1, 100);
        assert!(result.is_ok());
    }

    #[test]
    fn validate_deal_rejects_below_base_price() {
        // wood 基準 = 1，price_per=0 應被拒絕
        let result = validate_deal("wood", 5, 0, 100);
        assert!(result.is_err(), "低於基準價應被拒");
    }

    #[test]
    fn validate_deal_rejects_above_max_price() {
        // wood 基準 1, 3 倍 = 3；price_per=4 應被拒
        let result = validate_deal("wood", 5, 4, 100);
        assert!(result.is_err(), "超過 3 倍基準應被拒");
    }

    #[test]
    fn validate_deal_rejects_zero_qty() {
        let result = validate_deal("wood", 0, 2, 100);
        assert!(result.is_err());
    }

    #[test]
    fn validate_deal_rejects_excessive_qty() {
        let result = validate_deal("wood", MAX_DEAL_QTY + 1, 1, 10_000);
        assert!(result.is_err(), "超過 MAX_DEAL_QTY 應被拒");
    }

    #[test]
    fn validate_deal_rejects_insufficient_treasury() {
        // 5 × 2 = 10，金庫只有 5 → 拒絕
        let result = validate_deal("wood", 5, 2, 5);
        assert!(result.is_err());
    }

    #[test]
    fn validate_deal_rejects_unknown_item() {
        let result = validate_deal("jade_shard", 3, 8, 1_000);
        assert!(result.is_err());
    }

    #[test]
    fn pending_deal_total_is_correct() {
        let deal = validate_deal("ether", 10, 4, 1_000).unwrap();
        assert_eq!(deal.total(), 40); // 10 × 4 = 40
    }

    #[test]
    fn merchant_deal_hint_contains_example_token() {
        assert!(MERCHANT_DEAL_HINT.contains("[DEAL"), "提示應包含暗號範例讓 LLM 知道格式");
        assert!(!MERCHANT_DEAL_HINT.is_empty());
    }

    #[test]
    fn constants_are_reasonable() {
        assert!(MAX_DEAL_QTY >= 1, "最大數量至少 1");
        assert!(MAX_DEAL_PRICE_MULTIPLIER >= 2, "至少 2 倍才有議價意義");
        assert!(DEAL_EXPIRE_SECS >= 60, "有效期至少 1 分鐘");
    }
}
