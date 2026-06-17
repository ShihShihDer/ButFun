//! 鎮民認得你的夥伴（ROADMAP 359）：寵物剛在 358 長出「個性」，這個切片讓那份個性
//! 第一次走出寵物自己這條維度、被 NPC 的社交世界看見。
//!
//! 358「寵物有了脾氣」給同種寵物按主人分出黏人／活潑／好奇／慵懶四種脾氣，可那份脾氣至今
//! 只在「歇腳離主人多近」這個身體細節裡——除了主人自己留心，沒有任何人事物會回應它。本切片把
//! 它接進城鎮：帶著寵物走近在崗的村落 NPC 時，NPC 會抬頭瞧一眼你身邊的小傢伙，順著牠的脾氣
//! 搭上一句就地閒話（黏人的得意一句、慵懶的逗趣一句…）。城鎮第一次「看得見你帶的是什麼脾氣的夥伴」。
//!
//! 設計鐵律（刻意與 `npc_recognition`〔331 街坊相認〕分工——那條認的是「玩家是誰」、依午餐相熟度
//! 點名招呼；這條認的是「你帶的是什麼脾氣的夥伴」、依寵物個性搭話、不需任何相熟度，人人帶寵物
//! 路過都可能被搭上一句）：
//! - 純記憶體模式，重啟清零（零 DB／零 migration／碰不到任何玩家遊戲狀態）。
//! - 零 LLM、純查表；只挑「說哪一句」，不送物品／乙太／戰力，零平衡風險。
//! - 純邏輯可獨立測試。面向玩家字串集中於本檔，作為 i18n 替換點。
//! - 沿用既有 `ServerMsg::NpcSpeech` 就地泡泡（前端 ROADMAP 92 已能渲染），前端零改動。

use std::collections::HashMap;
use crate::pet_personality::PetPersonality;

/// 搭話搆得著的半徑（像素）：玩家（寵物跟在腳邊）要走到 NPC 跟前這麼近，NPC 才瞧得見那隻寵物、開口。
/// 取得比 331 街坊相認的 `RECOGNIZE_RADIUS`(110) 略收一點——是「瞧見你腳邊的小傢伙」、不是遠遠喊人。
pub const GREET_RADIUS: f32 = 96.0;

/// 同一對「玩家×NPC」兩次評論之間的冷卻（秒）：玩家牽著寵物在攤前逗留時不會被同一人連珠炮搭話。
/// 取得比 331 街坊相認冷卻(50) 略長，讓兩條相認線即使同時觸發也錯開、不會每次都同步冒兩個泡泡。
pub const GREET_COOLDOWN_SECS: f32 = 60.0;

/// 玩家是否走進某 NPC 的搭話範圍。純函式、可測。
/// 用距離平方比較，免去開平方；非有限座標保守回 `false`。
pub fn within_reach(npc_x: f32, npc_y: f32, px: f32, py: f32) -> bool {
    let dx = npc_x - px;
    let dy = npc_y - py;
    let d2 = dx * dx + dy * dy;
    d2.is_finite() && d2 <= GREET_RADIUS * GREET_RADIUS
}

/// 「活潑」寵物的評論池：精力旺盛、蹦蹦跳跳。各 NPC 一句、語氣貼自身人設。
static PLAYFUL_GREETS: &[(&str, &[&str])] = &[
    ("merchant", &["這小傢伙活蹦亂跳的，跟你一樣有精神嘛！"]),
    ("workshop_npc", &["你這夥伴一刻也閒不住啊，倒像爐邊的火星子。"]),
    ("bounty_npc", &["瞧你身邊這隻，渾身是勁——帶去野地準是把好手！"]),
    ("expedition_npc", &["這麼活潑的小傢伙，跟你跑遠路也不嫌累吧？"]),
    ("procurement_npc", &["哎喲，蹦得真歡，別撞翻我清點的料才好。"]),
    ("farm_fair_npc", &["你的小寶貝精神頭真足，菜畦邊跑跑跳跳也熱鬧。"]),
    ("village_chief", &["這夥伴生龍活虎的，給鎮上添了不少朝氣呢。"]),
];

/// 「慵懶」寵物的評論池：愛在後頭悠悠地晃、最放鬆。
static LAZY_GREETS: &[(&str, &[&str])] = &[
    ("merchant", &["你這小傢伙懶洋洋的，倒是會享受日子啊。"]),
    ("workshop_npc", &["你的夥伴慢悠悠的，跟我這把老骨頭一個調調。"]),
    ("bounty_npc", &["這麼懶散的小東西，野地的險可別指望牠出力咯。"]),
    ("expedition_npc", &["牠這副愛打盹的樣子，怕是寧可在家睡也不想跑遠路吧？"]),
    ("procurement_npc", &["你家小傢伙趴著就不動了，倒比我清點還沉得住氣。"]),
    ("farm_fair_npc", &["這小懶蟲往菜畦邊一躺，曬太陽倒是內行。"]),
    ("village_chief", &["你的夥伴這般悠閒，看著就讓人跟著鬆快了。"]),
];

/// 「好奇」寵物的評論池：總愛離主人遠一點、東張西望。
static CURIOUS_GREETS: &[(&str, &[&str])] = &[
    ("merchant", &["你家小傢伙一直盯著我攤上看，是看上哪樣貨啦？"]),
    ("workshop_npc", &["這夥伴探頭探腦的，當心爐火燙著鼻子哦。"]),
    ("bounty_npc", &["瞧牠四處張望，這份好奇心帶去探野地正合適。"]),
    ("expedition_npc", &["這麼愛東看西看的小傢伙，跟我是同道中人啊！"]),
    ("procurement_npc", &["你的小寶貝把我的貨架翻個遍，可別偷拿嘗鮮咯。"]),
    ("farm_fair_npc", &["牠在菜畦間鑽來鑽去，是聞著哪株瓜香了吧？"]),
    ("village_chief", &["這夥伴對什麼都新鮮，鎮上的角落怕是都讓牠逛遍了。"]),
];

/// 「黏人」寵物的評論池：寸步不離、貼著主人。
static CLINGY_GREETS: &[(&str, &[&str])] = &[
    ("merchant", &["哎，這小傢伙黏著你寸步不離，真是離不開主人呀。"]),
    ("workshop_npc", &["你的夥伴緊跟著你不撒手，這份親是打不出來的好東西。"]),
    ("bounty_npc", &["牠這麼黏你，野地裡反倒不怕走散，也算一樁好。"]),
    ("expedition_npc", &["走到哪跟到哪，這小傢伙認準你了，旅途上有伴嘍。"]),
    ("procurement_npc", &["你家小寶貝貼得這麼緊，連我遞貨都插不進手呢。"]),
    ("farm_fair_npc", &["牠步步跟著你，菜也不偷吃、就守著主人，乖得很。"]),
    ("village_chief", &["這夥伴對你這般依戀，看著就知是被你疼著長大的。"]),
];

/// 取某 NPC 對某「寵物個性」的評論（第 `slot` 句，在對應池內循環）。純函式、可測。
///
/// 非村落七大 NPC（旅人／居民／其他星球商人，不在池內）一律回 `None`——只有故鄉的鎮民會認你的夥伴。
pub fn greet_line(npc_id: &str, personality: PetPersonality, slot: usize) -> Option<&'static str> {
    let pool = match personality {
        PetPersonality::Playful => PLAYFUL_GREETS,
        PetPersonality::Lazy => LAZY_GREETS,
        PetPersonality::Curious => CURIOUS_GREETS,
        PetPersonality::Clingy => CLINGY_GREETS,
    };
    pool.iter()
        .find(|(id, _)| *id == npc_id)
        .map(|(_, lines)| lines[slot % lines.len()])
}

/// 寵物搭話冷卻帳本（純記憶體，重啟清零）。
///
/// 鍵 =「玩家鍵 × NPC id」；值 = 距下次可再搭話的剩餘秒數。
/// 玩家鍵即玩家在 `players` map 裡的鍵（登入玩家＝帳號 uid，訪客＝連線 id），與 331 街坊相認一致。
/// `slot` 是全鎮共用的輪替序號：每搭一句就推進，讓同一隻寵物連著路過不同 NPC 時、評論不至於一個調調。
#[derive(Debug, Default)]
pub struct GreetBook {
    cooldowns: HashMap<(String, String), f32>,
    slot: usize,
}

impl GreetBook {
    pub fn new() -> Self {
        Self {
            cooldowns: HashMap::new(),
            slot: 0,
        }
    }

    /// 推進冷卻倒數（每 tick 呼叫，`dt` 秒）：遞減各對的剩餘秒數，歸零者剔除以免帳本無限長大。
    pub fn tick(&mut self, dt: f32) {
        if self.cooldowns.is_empty() {
            return;
        }
        self.cooldowns.retain(|_, remaining| {
            *remaining -= dt;
            *remaining > 0.0
        });
    }

    /// 這一對玩家×NPC 此刻是否可再搭話（無紀錄或冷卻已過）。純查詢。
    pub fn ready(&self, player_key: &str, npc_id: &str) -> bool {
        self.cooldowns
            .get(&(player_key.to_string(), npc_id.to_string()))
            .map(|&remaining| remaining <= 0.0)
            .unwrap_or(true)
    }

    /// 記下「剛搭話過」：把這一對的冷卻設滿，`GREET_COOLDOWN_SECS` 秒內不再搭話。
    pub fn mark(&mut self, player_key: &str, npc_id: &str) {
        self.cooldowns.insert(
            (player_key.to_string(), npc_id.to_string()),
            GREET_COOLDOWN_SECS,
        );
    }

    /// 取下一句評論並推進全鎮輪替序號。村落 NPC 必得一句、非村落 NPC 回 `None`。
    pub fn greet(&mut self, npc_id: &str, personality: PetPersonality) -> Option<&'static str> {
        let line = greet_line(npc_id, personality, self.slot)?;
        self.slot = self.slot.wrapping_add(1);
        Some(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 村落七大 NPC（與 `npc_schedule::VILLAGE_NPCS` 同步；測試獨立列出以守住「每人每個性格都有評論」）。
    const VILLAGE_IDS: &[&str] = &[
        "merchant", "workshop_npc", "bounty_npc", "expedition_npc",
        "procurement_npc", "farm_fair_npc", "village_chief",
    ];

    #[test]
    fn within_reach_respects_radius() {
        // 同點當然搆得著。
        assert!(within_reach(100.0, 100.0, 100.0, 100.0));
        // 半徑內（差 90px < 96）搆得著。
        assert!(within_reach(100.0, 100.0, 190.0, 100.0));
        // 半徑外（差 200px > 96）搆不著。
        assert!(!within_reach(100.0, 100.0, 300.0, 100.0));
        // 非有限座標保守回 false。
        assert!(!within_reach(f32::NAN, 0.0, 0.0, 0.0));
    }

    #[test]
    fn every_village_npc_has_line_for_every_personality() {
        // 七大 NPC × 四種個性都該有一句評論（不留空缺，否則玩家帶某脾氣寵物路過會無聲）。
        for &npc in VILLAGE_IDS {
            for p in PetPersonality::ALL {
                let line = greet_line(npc, p, 0);
                assert!(line.is_some(), "{npc} 對 {:?} 個性應有評論", p);
                assert!(!line.unwrap().is_empty(), "{npc} 對 {:?} 評論不該為空", p);
            }
        }
    }

    #[test]
    fn non_village_npc_gets_nothing() {
        // 非村落 NPC（旅人／居民／其他星球商人）一律不評論寵物。
        for p in PetPersonality::ALL {
            assert_eq!(greet_line("traveler_42", p, 0), None);
            assert_eq!(greet_line("resident_7", p, 0), None);
            assert_eq!(greet_line("", p, 0), None);
        }
    }

    #[test]
    fn line_slot_cycles_within_pool() {
        // slot 在池內環繞、不越界（連取多個 slot 都能取到句子、不 panic）。
        for slot in 0..12 {
            assert!(greet_line("merchant", PetPersonality::Clingy, slot).is_some());
        }
    }

    #[test]
    fn personalities_give_distinct_lines() {
        // 同一個 NPC 對不同個性的評論應彼此不同（否則「個性」就沒被看見）。
        let merchant_lines: std::collections::HashSet<&str> = PetPersonality::ALL
            .iter()
            .map(|&p| greet_line("merchant", p, 0).unwrap())
            .collect();
        assert_eq!(merchant_lines.len(), 4, "商人對四種個性應有四句相異評論");
    }

    #[test]
    fn book_ready_then_cooldown_then_recovers() {
        let mut book = GreetBook::new();
        // 從未搭話過 → 可搭話。
        assert!(book.ready("p1", "merchant"));
        // 搭話後進入冷卻 → 不可再搭話。
        book.mark("p1", "merchant");
        assert!(!book.ready("p1", "merchant"));
        // 冷卻未過完 → 仍不可。
        book.tick(GREET_COOLDOWN_SECS - 1.0);
        assert!(!book.ready("p1", "merchant"));
        // 冷卻過完 → 條目被剔除、恢復可搭話。
        book.tick(2.0);
        assert!(book.ready("p1", "merchant"));
    }

    #[test]
    fn book_cooldown_is_per_pair() {
        let mut book = GreetBook::new();
        book.mark("p1", "merchant");
        // 同玩家對「不同 NPC」互不影響。
        assert!(book.ready("p1", "village_chief"));
        // 不同玩家對「同一 NPC」也互不影響。
        assert!(book.ready("p2", "merchant"));
        // 被標記的那一對才在冷卻。
        assert!(!book.ready("p1", "merchant"));
    }

    #[test]
    fn greet_advances_slot_for_variety() {
        // greet 每呼叫一次就推進輪替序號（即便目前每池只有一句，序號仍應前進，
        // 為日後在池內補第二句時自動帶來變化）。
        let mut book = GreetBook::new();
        assert_eq!(book.slot, 0);
        assert!(book.greet("merchant", PetPersonality::Playful).is_some());
        assert_eq!(book.slot, 1);
        // 非村落 NPC 取不到句子、序號不前進。
        assert!(book.greet("traveler_1", PetPersonality::Playful).is_none());
        assert_eq!(book.slot, 1);
    }

    #[test]
    fn empty_book_tick_is_noop() {
        // 空帳本 tick 不 panic、保持空。
        let mut book = GreetBook::new();
        book.tick(1.0);
        assert!(book.ready("anyone", "merchant"));
    }
}
