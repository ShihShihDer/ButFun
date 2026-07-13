//! 乙太方界·居民私人嗜好與意外驚喜 v1（自主提案切片）。
//!
//! **緣起**：這個世界已經把居民的「能力」寫得很深——手藝專精（888）、公認名匠（889）、
//! 路人因聲望而佩服（956）、發明技能傳承（716~955）——但這一整條軸線談的都是
//! 「她多厲害、多會這件事」。居民從沒有一項**與能力/聲望無關、純粹是她這個人喜歡什麼**
//! 的私人品味。本刀補上：每位居民有一項固定的私人小嗜好（撿野花押花／收藏發光結晶），
//! 閒暇時會自己沉浸其中攢一點私藏，攢夠了、又跟你聊得來時，會把這份私藏當驚喜送你——
//! 不是因為你多會她的手藝、也不是因為你倆是戀人，只是「她這個人就是想跟你分享這個」。
//!
//! **與既有系統 razor-sharp 區隔（非同軸換皮）**：
//! - 與 888/889/956（手藝專精／名匠聲望）＝「能力」軸：越練越強、有高下之分、驅動教學。
//!   本刀＝「品味」軸：固定不變、無高下、與任何技能樹無關，兩者因果完全獨立。
//! - 與 660 贈禮（玩家→居民，需玩家主動採材料送出）＝方向相反：本刀是**居民主動**送給玩家。
//! - 與 732 心意留痕（玩家送的禮物被居民擺成世界裡的永久裝飾方塊）＝完全不放置任何方塊、
//!   不動世界地形，純粹是一次對話裡的驚喜餽贈，物品進玩家背包而非留在世界。
//! - 與 723 居民互相以物易物（居民↔居民、到訪劇本輪替的一種）＝本刀是居民↔玩家、
//!   掛在「與這位居民對話」這個天然節點觸發，不佔到訪劇本的任何一個輪替位置。
//! - 與 927~930 戀愛/家庭弧＝關係里程碑（一生一次的高潮）；本刀＝日常反覆的小驚喜，
//!   不需要戀人身分，任何交情夠深的玩家都會遇到。
//!
//! **純邏輯層**：嗜好指派、物品挑選、觸發判定、台詞全是確定性純函式，零 LLM、零鎖、零 IO；
//! 連線／鎖／持久化細節全在 `voxel_ws.rs`。**零新美術**：沿用既有紅花/黃花/藍花(94/95/96)、
//! 發光結晶(106) 這些早已存在的方塊/物品素材。**零新持久化格式**：私藏數量是純記憶體計數
//! （比照既有 `open_request`／`gathered_since_build` 同款慣例，重啟歸零、不影響任何存檔）。

/// 私人嗜好種類（依居民索引決定性指派，見 [`hobby_for`]）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hobby {
    /// 押花：喜歡撿路邊的野花，壓成押花收藏。
    Flowers,
    /// 收藏發光結晶：喜歡撿拾閃著微光的乙太結晶，攢起來把玩。
    Crystals,
}

/// 依居民索引決定性指派嗜好（`i % 2`，比照 `voxel_residents::home_direction` 同款
/// index 取模寫法）——人口成長（出生）新居民自然循環指派，不需另外維護名冊。
pub fn hobby_for(i: usize) -> Hobby {
    if i % 2 == 0 {
        Hobby::Flowers
    } else {
        Hobby::Crystals
    }
}

impl Hobby {
    /// 嗜好名稱（面向玩家、繁中，i18n 友善集中於此）。
    pub fn label(&self) -> &'static str {
        match self {
            Hobby::Flowers => "押花",
            Hobby::Crystals => "收藏發光結晶",
        }
    }

    /// 送給玩家的實際物品（對齊 `voxel::Block` + `voxel_gift::item_name_zh` 既有命名表，
    /// 零新美術）。押花依 `pick` 在紅/黃/藍三色間確定性輪替，增添收到時的驚喜多樣性；
    /// 結晶種類單一（發光結晶），本就是這位居民唯一鍾情的那一種。
    pub fn item_block_id(&self, pick: usize) -> u8 {
        match self {
            Hobby::Flowers => match pick % 3 {
                0 => 94, // 紅花
                1 => 95, // 黃花
                _ => 96, // 藍花
            },
            Hobby::Crystals => 106, // 發光結晶
        }
    }
}

/// 私藏累積門檻：攢到這麼多份才夠「拿得出手」當驚喜送出（避免才撿一朵就急著送、太輕率）。
pub const STASH_THRESHOLD: u32 = 3;

/// 每次驚喜餽贈消耗（也是玩家實際收到的數量）：只送一部分，留一點在手邊繼續攢，
/// 讓「這是她持續在做的事」而非一次清空。
pub const GIFT_ITEM_COUNT: u32 = 2;

/// 私藏數量上限：閒暇沉浸攢到這裡就不再累加（避免長期掛機無限膨脹，療癒但有界）。
pub const STASH_CAP: u32 = 6;

/// 沉浸嗜好、私藏 +1 的低頻閒置節拍：入場後每位居民各自錯開。
pub const COLLECT_COOLDOWN_SECS: f32 = 50.0;
/// 每個合格 tick 觸發「沉浸嗜好」的機率（10Hz 下與其他閒置社交同量級，稀有不洗版）。
pub const COLLECT_CHANCE_PER_TICK: f32 = 0.02;

/// 驚喜餽贈觸發後的冷卻：讓「送你私藏」稀有有份量，不會每次聊天都送。
pub const GIFT_COOLDOWN_SECS: f32 = 240.0;
/// 驚喜餽贈所需的最低好感（記憶筆數）：比老友門檻（[`crate::voxel_fond_greeting::FOND_AFFINITY`]=5）
/// 略寬鬆——這是「聊得來就願意分享」的門檻，不需要深交多年的老友才夠格。
pub const GIFT_AFFINITY: usize = 3;
/// 驚喜餽贈的機率（每次符合條件的對話各擲一次骰）：稀有，不是每次都送。
pub const GIFT_CHANCE: f32 = 0.3;

/// 是否該在這次閒置 tick 沉浸嗜好、私藏 +1（純判定，呼叫端負責讀 `stash < STASH_CAP` 再加）。
pub fn should_collect(idle: bool, stash: u32, roll: f32) -> bool {
    idle && stash < STASH_CAP && roll < COLLECT_CHANCE_PER_TICK
}

/// 是否該在這次對話裡順手送出驚喜（純判定）：私藏夠了、好感夠了、擲骰命中。
pub fn should_surprise_gift(stash: u32, affinity: usize, roll: f32) -> bool {
    stash >= STASH_THRESHOLD && affinity >= GIFT_AFFINITY && roll < GIFT_CHANCE
}

/// 沉浸嗜好時的閒置泡泡台詞（確定性、≤ 40 字）。
pub fn collect_say_line(hobby: Hobby, pick: usize) -> String {
    let lines: &[&str] = match hobby {
        Hobby::Flowers => &[
            "蹲下身，仔細挑揀著路邊的野花～",
            "把撿來的花瓣夾進手邊的小本子裡。",
            "又找到一朵喜歡的顏色，開心地收了起來。",
        ],
        Hobby::Crystals => &[
            "撿起一顆微微發亮的結晶，對著光看了又看。",
            "把新撿的發光結晶收進口袋，滿足地笑了。",
            "蹲在地上，仔細端詳手裡那顆閃著微光的結晶。",
        ],
    };
    lines[pick % lines.len()].to_string()
}

/// 驚喜餽贈當下的對話泡泡台詞（嵌玩家名，≤ 40 字，確定性）。
pub fn surprise_gift_say_line(hobby: Hobby, player_name: &str, pick: usize) -> String {
    let name: String = if player_name.is_empty() {
        "你".to_string()
    } else {
        player_name.chars().take(6).collect()
    };
    let lines: [String; 2] = match hobby {
        Hobby::Flowers => [
            format!("對了{name}，這是我一直在攢的押花，送你一些！"),
            format!("{name}，這幾朵是我挑最喜歡的顏色留的，給你！"),
        ],
        Hobby::Crystals => [
            format!("{name}，這是我私藏的發光結晶，分你一點～"),
            format!("對了{name}，這顆我一直很喜歡，也想讓你看看！"),
        ],
    };
    lines[pick % lines.len()].chars().take(40).collect()
}

/// 驚喜餽贈記進居民自己心裡的一筆記憶（不含玩家原話，確定性模板）。
pub fn surprise_gift_memory_line(hobby: Hobby) -> String {
    match hobby {
        Hobby::Flowers => "今天，我把私藏的押花分給了對方，看到對方開心，我也很高興。".to_string(),
        Hobby::Crystals => "今天，我把私藏的發光結晶分給了對方，這是我私底下珍視的東西。".to_string(),
    }
}

/// 城鎮動態牆播報（確定性模板，截斷防破框）。
pub fn surprise_gift_feed_line(resident_name: &str, hobby: Hobby, player_name: &str) -> String {
    let rname: String = resident_name.chars().take(12).collect();
    let pname: String = if player_name.is_empty() {
        "一位旅人".to_string()
    } else {
        player_name.chars().take(12).collect()
    };
    format!("{rname}把自己私藏的{}分給了{pname}。", hobby.label())
}

/// Feed 動態牆種類（分類顯示用，面向玩家、集中可 i18n）。
pub const FEED_KIND: &str = "私人嗜好";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hobby_for_alternates_and_cycles_with_population_growth() {
        assert_eq!(hobby_for(0), Hobby::Flowers);
        assert_eq!(hobby_for(1), Hobby::Crystals);
        assert_eq!(hobby_for(2), Hobby::Flowers);
        assert_eq!(hobby_for(3), Hobby::Crystals);
        // 人口成長後索引超過初始 4 位仍確定性循環（比照 home_direction 的 i % 4 精神）。
        assert_eq!(hobby_for(10), hobby_for(0));
        assert_eq!(hobby_for(11), hobby_for(1));
    }

    #[test]
    fn item_block_id_flowers_rotates_three_colors_all_valid() {
        assert_eq!(Hobby::Flowers.item_block_id(0), 94);
        assert_eq!(Hobby::Flowers.item_block_id(1), 95);
        assert_eq!(Hobby::Flowers.item_block_id(2), 96);
        // 繞回。
        assert_eq!(Hobby::Flowers.item_block_id(3), 94);
    }

    #[test]
    fn item_block_id_crystals_always_glow_crystal() {
        for pick in 0..5 {
            assert_eq!(Hobby::Crystals.item_block_id(pick), 106);
        }
    }

    #[test]
    fn should_collect_requires_idle_and_under_cap_and_roll() {
        assert!(should_collect(true, 0, 0.0));
        assert!(!should_collect(false, 0, 0.0)); // 非閒置不沉浸
        assert!(!should_collect(true, STASH_CAP, 0.0)); // 已到上限不再累加
        assert!(!should_collect(true, 0, 0.99)); // 骰子沒過門檻
    }

    #[test]
    fn should_surprise_gift_requires_all_three_gates() {
        assert!(should_surprise_gift(STASH_THRESHOLD, GIFT_AFFINITY, 0.0));
        assert!(!should_surprise_gift(STASH_THRESHOLD - 1, GIFT_AFFINITY, 0.0)); // 私藏不夠
        assert!(!should_surprise_gift(STASH_THRESHOLD, GIFT_AFFINITY - 1, 0.0)); // 好感不夠
        assert!(!should_surprise_gift(STASH_THRESHOLD, GIFT_AFFINITY, 0.99)); // 骰子沒過
    }

    #[test]
    fn should_surprise_gift_boundary_is_inclusive() {
        // 門檻剛好卡在邊界值也算數（>= 而非 >），避免差一點永遠不觸發的挫折感。
        assert!(should_surprise_gift(STASH_THRESHOLD, GIFT_AFFINITY, GIFT_CHANCE - 0.001));
        assert!(!should_surprise_gift(STASH_THRESHOLD, GIFT_AFFINITY, GIFT_CHANCE));
    }

    #[test]
    fn collect_say_line_nonempty_and_deterministic_per_hobby() {
        for hobby in [Hobby::Flowers, Hobby::Crystals] {
            let a = collect_say_line(hobby, 5);
            let b = collect_say_line(hobby, 5);
            assert_eq!(a, b);
            assert!(!a.is_empty());
        }
    }

    #[test]
    fn surprise_gift_say_line_embeds_player_name_and_truncates() {
        let line = surprise_gift_say_line(Hobby::Flowers, "阿光", 0);
        assert!(line.contains("阿光"));
        assert!(line.chars().count() <= 40);
        // 空名安全退場，不崩潰、不出現空字串主詞。
        let empty = surprise_gift_say_line(Hobby::Crystals, "", 0);
        assert!(empty.contains('你'));
    }

    #[test]
    fn surprise_gift_memory_line_differs_by_hobby_and_omits_player_text() {
        let flowers = surprise_gift_memory_line(Hobby::Flowers);
        let crystals = surprise_gift_memory_line(Hobby::Crystals);
        assert_ne!(flowers, crystals);
        // 不外洩玩家原話：純模板句不含任何玩家自訂內容標記。
        assert!(!flowers.is_empty() && !crystals.is_empty());
    }

    #[test]
    fn surprise_gift_feed_line_embeds_both_names_and_truncates_long_names() {
        let long_name = "超級無敵霹靂長的名字測試截斷用途看看夠不夠長啊真的很長";
        let line = surprise_gift_feed_line(long_name, Hobby::Flowers, "旅人甲");
        assert!(line.contains("旅人甲"));
        assert!(line.contains("押花"));
        // 兩段名字各自截斷到 12 字，總長不會無界暴衝。
        assert!(line.chars().count() < 40);
    }

    #[test]
    fn surprise_gift_feed_line_empty_player_falls_back_to_generic() {
        let line = surprise_gift_feed_line("露娜", Hobby::Crystals, "");
        assert!(line.contains("一位旅人"));
        assert!(line.contains("露娜"));
    }
}
