//! 乙太方界·居民也會肚子餓 v1——居民第一個「生理需求」（自主提案切片）。
//!
//! **缺口 / 為誰做**：乙太方界的居民至今有一整套**情緒**內心（心情 677、孤獨尋伴 678、
//! 掏心 confide、渴望 desires…），會記得你、形成關係、蓋家、種田——但牠們從沒有過一個
//! **身體上的需求**。牠們不會餓、不會累到得吃點東西，「活著」少了最基本的一拍：肚子餓。
//! 這正對著 PLAN_ETHERVOX 的核心信念——**記憶／狀態要驅動行為，讓居民真的活著**。本刀給
//! 居民第一個生理需求：**餓意**。它隨時間默默累積，餓了居民會冒一句「肚子有點餓了…」的
//! 心聲、放下閒晃走回家找點存糧吃，吃飽了滿足地舒一口氣——一個由**內在需求驅動**的自理
//! 行為，玩家第一次看見居民為了照顧自己的身體而行動。
//!
//! **交織點（你的善意踩在對的時間點上）**：而如果就在牠正餓的時候，你剛好餵了牠一口吃的
//! （沿用既有送食物→細細享用管線 765），牠會**記得格外深**——不是普通的一句道謝，而是
//! 「你在我正餓的時候餵了我」這樣一筆掛在你名下的暖記憶，你的餽贈第一次不只被收下，還**正好
//! 落在牠最需要的時刻**。需求驅動行為，你的互動因此更有後果。
//!
//! **與既有系統的分界（換維度·非同軸重複）**：這不是 678「孤獨尋伴」（**情緒**缺口→走向
//! **玩家**求陪）——餓是**生理**缺口、居民走向**自己的家**吃存糧、自己就能滿足，不黏玩家；
//! 也不是 664「拜託你幫個小忙」（`open_request`：討**建材**、一次性的人情）——餓是會隨時間
//! **反覆累積**的持續狀態、由居民**自理**（回家吃）為主、玩家餵食只是錦上添花的加深記憶。
//! 這是居民的第一個**需求 (need)**，開「需求驅動行為」這條至今空白的維度。
//!
//! **這裡只放確定性純邏輯**（餓意累積、門檻判定、台詞／記憶／Feed 文案），零 LLM、零鎖、
//! 零 IO、零 async，可單元測試。連線 / 鎖 / 走動 / 廣播全留在 `voxel_ws.rs`（沿用尋伴／
//! 致意的短鎖循序 + 逐 tick 重設目標慣例，守 prod 死鎖鐵律）。
//!
//! **成本 / 濫用防護**：不觸發 LLM、不開對外端點、不收玩家自由文字——餓／飽台詞與記憶全為
//! 確定性模板、只嵌玩家**顯示名**（本就出現在道謝／記憶），永不回放記憶原文或玩家原話（無
//! 注入 / NSFW 面）；餓意純伺服器 tick 累積、玩家無法自報；純記憶體、重啟歸零（餓意是數
//! 分鐘的過場狀態，重啟大不了少餓一次、零資料風險、零 migration），不碰玩家資料 / 帳號權限。

use std::collections::HashMap;

use crate::voxel::Block;
use crate::voxel_berry as vberry;
use crate::voxel_farm as vfarm;

/// 飽足時餓意為 0.0、餓到極點為 [`HUNGER_MAX`]。
pub const HUNGER_MAX: f32 = 1.0;

/// 餓意累積速率（每秒）：約 15 分鐘從全飽累到餓極。刻意慢——餓是偶爾一次的生活節拍、
/// 不是每分鐘的騷擾，稀少才有份量。
pub const HUNGER_RATE_PER_SEC: f32 = 1.0 / 900.0;

/// 「餓了」門檻：餓意越過這條線（約 10.5 分鐘沒進食）居民才開始想找點吃的。
pub const HUNGRY_THRESHOLD: f32 = 0.70;

/// 走到家域中心多近，算「到家、吃得上存糧」（世界方塊）。
pub const EAT_ARRIVE_DIST: f32 = 3.0;

/// 冒餓／吃飽後的靜默冷卻（秒）：一位居民喊過餓或剛吃飽後，這段時間內不再喊餓，
/// 避免反覆碎念、讓「餓了」這件事稀少而有感。
pub const HUNGER_SAY_COOLDOWN: f32 = 120.0;

/// 餓意隨時間累積 `dt` 秒（clamp 到 `[0, HUNGER_MAX]`）。純函式、確定性、可測。
pub fn tick_hunger(cur: f32, dt: f32) -> f32 {
    (cur + HUNGER_RATE_PER_SEC * dt).clamp(0.0, HUNGER_MAX)
}

/// 餓意是否已越過門檻、居民會想找吃的。
pub fn is_hungry(h: f32) -> bool {
    h >= HUNGRY_THRESHOLD
}

/// 入場錯開初始靜默冷卻（秒），避免啟動後短時間內全員一起喊餓。
pub fn hunger_cd_offset(i: usize) -> f32 {
    60.0 + i as f32 * 45.0
}

/// 居民冒「肚子餓了」心聲的台詞（起身回家找吃的那一刻，四句輪替）。
pub fn hunger_say_line(pick: usize) -> &'static str {
    const LINES: [&str; 4] = [
        "肚子有點餓了…回家找點吃的吧",
        "唔，該吃點東西了",
        "肚子在叫了，回去墊墊肚子",
        "有點餓，去翻翻存糧",
    ];
    LINES[pick % LINES.len()]
}

/// 居民回到家、吃上存糧後滿足的暖泡泡（四句輪替）。
pub fn sated_say_line(pick: usize) -> &'static str {
    const LINES: [&str; 4] = [
        "吃飽了，舒服～",
        "嗯，這下有力氣了",
        "肚子暖暖的，滿足",
        "填飽了肚子，真好",
    ];
    LINES[pick % LINES.len()]
}

/// 「你在我正餓的時候餵了我」——玩家在居民餓時餵食，居民掛在玩家名下的深記憶。
/// `player` 空（訪客無顯示名）→ 退成不點名的泛稱，仍不回放任何原話。
pub fn fed_memory_line(player: &str) -> String {
    if player.is_empty() {
        "有人在我正餓的時候餵了我一口，這份好，我記得特別牢。".to_string()
    } else {
        format!("{player}在我正餓的時候餵了我一口，這份好，我記得特別牢。")
    }
}

/// 玩家餓時餵食的城鎮動態牆一行。`player` 空 → 泛稱「有人」。
pub fn fed_feed_line(rname: &str, player: &str) -> String {
    let who = if player.is_empty() { "有人" } else { player };
    format!("{rname}正餓著，{who}剛好餵了一口——這份暖，記得格外深。")
}

// ── 飢餓接農田／倉庫 v2：吃自己種的／存的，沒有就真的餓著 ──────────────────────────
//
// **缺口**：v1 的「回家吃存糧」是**憑空**的——到家即餓意歸零，跟居民自己種的田、採的莓、
// 存進小背包（res_inv）的東西完全脫節，「存糧」只是句台詞。v2 把它接上真實食物庫存：餓了回家
// **檢查小背包真的有吃的**（小麥／麵包／胡蘿蔔／馬鈴薯／烤地薯／莓果／果醬），有就**吃掉一份**、
// 真的扣量；**沒有就真的餓著**——誠實心聲「肚子好餓，可是家裡什麼吃的都沒有…」，並放下閒晃
// **去把附近熟了的作物收成進背包**（餓 → 為了吃而去收成 → 收成進存糧 → 下次餓了吃它）。需求
// 第一次真正驅動出「為了吃而種田收成」的行為，建築（田／倉）第一次有真正的功能。
//
// 這裡只放**確定性純邏輯**（哪些是食物、挑哪份吃、扣一份、熟作物→食物產出映射、台詞），零鎖零 IO
// 零 async，全可單元測試；連線／鎖／走動／收成寫世界全留在 `voxel_ws.rs`。

/// 居民背包裡算「能吃的食物」的物品 id 清單（查 `voxel_farm` / `voxel_berry`）。
/// 只列真正入口的食物，不含種子（種子是拿來種、不是拿來吃的）。
pub const FOOD_IDS: [u8; 8] = [
    vfarm::WHEAT_ID,        // 18 小麥顆粒
    vfarm::BREAD_ID,        // 19 麵包
    vfarm::CARROT_ID,       // 49 胡蘿蔔
    vfarm::POTATO_ID,       // 53 馬鈴薯
    vfarm::BAKED_POTATO_ID, // 64 烤地薯
    vberry::BERRY_ID,       // 77 莓果
    vberry::JAM_ID,         // 78 果醬
    vfarm::PUMPKIN_ID,      // 110 南瓜（季限作物·秋南瓜 v1）
];

/// 挑食物吃時的**偏好順序**（越前面越優先吃）：加工／飽足感高的先吃（麵包、果醬、烤地薯），
/// 生食料（莓果、胡蘿蔔、馬鈴薯、小麥顆粒）殿後——讓「吃」有點層次，也讓珍貴的加工品先派上用場。
const EAT_PREFERENCE: [u8; 8] = [
    vfarm::BREAD_ID,        // 麵包（療癒農業循環終點，最頂）
    vberry::JAM_ID,         // 果醬
    vfarm::BAKED_POTATO_ID, // 烤地薯
    vfarm::PUMPKIN_ID,      // 南瓜（沉甸甸一顆，飽足感高，排在生食料之前）
    vberry::BERRY_ID,       // 莓果
    vfarm::CARROT_ID,       // 胡蘿蔔
    vfarm::POTATO_ID,       // 生馬鈴薯
    vfarm::WHEAT_ID,        // 小麥顆粒（最不像一餐，墊底）
];

/// 這個物品 id 是不是「能吃的食物」。
pub fn is_food(id: u8) -> bool {
    FOOD_IDS.contains(&id)
}

/// 背包裡有沒有任何一份食物（>0）。
pub fn has_food(bag: &HashMap<u8, u32>) -> bool {
    FOOD_IDS.iter().any(|id| bag.get(id).copied().unwrap_or(0) > 0)
}

/// 依偏好順序挑一份食物來吃，回傳該食物 id；背包沒任何食物 → `None`。純判定、不改背包。
pub fn pick_food(bag: &HashMap<u8, u32>) -> Option<u8> {
    EAT_PREFERENCE
        .iter()
        .copied()
        .find(|id| bag.get(id).copied().unwrap_or(0) > 0)
}

/// 從背包吃掉一份指定食物（數量 -1，歸零則移除該鍵）。回傳是否真的吃到了（有存量才吃得到）。
/// 純函式（吃 `&mut HashMap`）、可測；呼叫端在 `res_inv` 寫鎖內用它。
pub fn consume_one(bag: &mut HashMap<u8, u32>, id: u8) -> bool {
    match bag.get_mut(&id) {
        Some(n) if *n > 0 => {
            *n -= 1;
            if *n == 0 {
                bag.remove(&id);
            }
            true
        }
        _ => false,
    }
}

/// 食物 id → 顯示名（吃飽泡泡點名「吃了顆胡蘿蔔」用）。非食物回 `None`。
pub fn food_name_zh(id: u8) -> Option<&'static str> {
    Some(match id {
        x if x == vfarm::WHEAT_ID => "小麥",
        x if x == vfarm::BREAD_ID => "麵包",
        x if x == vfarm::CARROT_ID => "胡蘿蔔",
        x if x == vfarm::POTATO_ID => "馬鈴薯",
        x if x == vfarm::BAKED_POTATO_ID => "烤地薯",
        x if x == vberry::BERRY_ID => "莓果",
        x if x == vberry::JAM_ID => "果醬",
        x if x == vfarm::PUMPKIN_ID => "南瓜",
        _ => return None,
    })
}

/// 一畦成熟作物「被居民收成」時的產出：`Some((食物 id, 份數, 收成後回退成的方塊))`。
/// 比照玩家破壞成熟作物的掉落（`voxel_ws` 收割規則）＋收成回贈（Mature→Seeded 可再長）——
/// 收成後作物退回「已播種」狀態，田還在、能再長一輪（療癒的可持續農業）。莓果叢退回結果前的苗。
/// 非成熟作物（含空氣／地形）→ `None`。純映射、可測。
pub fn harvest_food_of(b: Block) -> Option<(u8, u32, Block)> {
    Some(match b {
        Block::WheatMature => (vfarm::WHEAT_ID, 1, Block::FarmSoilSeeded),
        Block::CarrotMature => (vfarm::CARROT_ID, 1, Block::CarrotSeeded),
        Block::PotatoMature => (vfarm::POTATO_ID, 2, Block::PotatoSeeded), // 馬鈴薯量大是特色
        Block::PumpkinMature => (vfarm::PUMPKIN_ID, 3, Block::PumpkinSeeded), // 南瓜全作物最大收量
        Block::BerryBushRipe => (vberry::BERRY_ID, vberry::BERRY_YIELD, Block::BerryBush),
        _ => return None,
    })
}

/// 這個方塊是不是「熟了、可被居民為了吃而去收成」的食物作物。
pub fn is_harvestable_food_block(b: Block) -> bool {
    harvest_food_of(b).is_some()
}

/// 到家、從背包吃掉一份食物後的滿足暖泡泡（點名吃了什麼，四句輪替）。
pub fn ate_say_line(food: &str, pick: usize) -> String {
    const TAILS: [&str; 4] = [
        "吃了顆{food}，舒服～",
        "嗯，這份{food}下肚，有力氣了",
        "吃了點存下的{food}，肚子暖暖的",
        "把存的{food}墊了墊，真好",
    ];
    TAILS[pick % TAILS.len()].replace("{food}", food)
}

/// 餓了回到家、卻發現家裡什麼吃的都沒有——誠實的心聲（四句輪替）。不粉飾：她是真的餓著。
pub fn no_food_say_line(pick: usize) -> &'static str {
    const LINES: [&str; 4] = [
        "肚子好餓，可是家裡什麼吃的都沒有…",
        "翻遍了存糧，一點吃的也沒有…",
        "餓著肚子，家裡空空的，得去弄點吃的",
        "沒東西吃…去田裡看看有沒有熟的吧",
    ];
    LINES[pick % LINES.len()]
}

/// 沒糧的餓意驅動她去收成／覓食時，起身那一刻的心聲（四句輪替）。
pub fn forage_say_line(pick: usize) -> &'static str {
    const LINES: [&str; 4] = [
        "去田裡收點熟的來吃",
        "肚子餓，去看看哪畦作物熟了",
        "得去採點吃的，不然要餓壞了",
        "去把熟了的收一收，好填肚子",
    ];
    LINES[pick % LINES.len()]
}

/// 餓著去把一畦作物收進背包後的心聲（收成即入存糧，四句輪替）。
pub fn foraged_say_line(food: &str, pick: usize) -> String {
    const TAILS: [&str; 4] = [
        "收了些{food}，這下有得吃了",
        "採到{food}啦，先存著待會吃",
        "{food}熟了，收進袋子墊肚子",
        "把{food}收好，餓的時候就有著落了",
    ];
    TAILS[pick % TAILS.len()].replace("{food}", food)
}

/// 共用糧倉 v1（村莊互助）：附近找不到熟作物，改去一個有人存了食物的箱子借一份後的心聲
/// （四句輪替）。措辭刻意與 [`foraged_say_line`]（自己種的、自己收）區隔——這份是**借**來的、
/// 靠的是村裡有人存了糧，凸顯「共用糧倉」與「自己田裡收成」是兩件事。
pub fn borrowed_say_line(food: &str, pick: usize) -> String {
    const TAILS: [&str; 4] = [
        "田裡沒熟的，翻了村裡的箱子，借了點{food}",
        "找不到熟作物，幸好箱子裡還有{food}",
        "跟大家的糧倉借了份{food}，先墊墊肚子",
        "自己田裡沒得收，村裡的箱子有{food}，先拿了",
    ];
    TAILS[pick % TAILS.len()].replace("{food}", food)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hunger_accumulates_and_clamps() {
        // 從全飽開始累積：正向增加。
        let h1 = tick_hunger(0.0, 60.0);
        assert!(h1 > 0.0 && h1 < HUNGER_MAX);
        // 大 dt 也不會超過上限。
        assert_eq!(tick_hunger(0.9, 100_000.0), HUNGER_MAX);
        // 已在上限：維持上限、不溢位。
        assert_eq!(tick_hunger(HUNGER_MAX, 10.0), HUNGER_MAX);
        // 負餓意（理論上不會發生）也夾回 0 以上。
        assert_eq!(tick_hunger(-5.0, 0.0), 0.0);
    }

    #[test]
    fn rate_reaches_threshold_in_expected_time() {
        // 約 10.5 分鐘（630 秒）應恰好越過門檻，不會太快也不會太慢。
        let h = tick_hunger(0.0, HUNGRY_THRESHOLD / HUNGER_RATE_PER_SEC);
        assert!((h - HUNGRY_THRESHOLD).abs() < 1e-4);
        // 略早於此則尚未餓。
        assert!(!is_hungry(tick_hunger(0.0, 600.0)));
    }

    #[test]
    fn is_hungry_threshold_boundary() {
        assert!(!is_hungry(HUNGRY_THRESHOLD - 0.01));
        assert!(is_hungry(HUNGRY_THRESHOLD)); // 恰好等於門檻算餓
        assert!(is_hungry(HUNGER_MAX));
        assert!(!is_hungry(0.0));
    }

    #[test]
    fn cd_offsets_are_staggered() {
        // 四位居民初始冷卻互不相同、遞增，錯開喊餓時機。
        let offs: Vec<f32> = (0..4).map(hunger_cd_offset).collect();
        for w in offs.windows(2) {
            assert!(w[1] > w[0], "初始冷卻應遞增錯開");
        }
        assert!(offs[0] > 0.0);
    }

    #[test]
    fn say_lines_rotate_and_bounded() {
        // 台詞輪替、非空、長度合理（前端泡泡 ≤ 50 字上限內）。
        for pick in 0..8 {
            let hl = hunger_say_line(pick);
            let sl = sated_say_line(pick);
            assert!(!hl.is_empty() && hl.chars().count() <= 40);
            assert!(!sl.is_empty() && sl.chars().count() <= 40);
        }
        // pick 溢出用取模包回，不 panic。
        assert_eq!(hunger_say_line(0), hunger_say_line(4));
        assert_eq!(sated_say_line(1), sated_say_line(5));
    }

    #[test]
    fn fed_memory_embeds_player_or_falls_back() {
        let m = fed_memory_line("諾娃");
        assert!(m.contains("諾娃"));
        assert!(m.contains("餓"));
        // 空名（訪客）退泛稱、不留空洞、不含原話。
        let g = fed_memory_line("");
        assert!(!g.contains("諾娃"));
        assert!(g.contains("餓"));
    }

    #[test]
    fn fed_feed_embeds_names() {
        let f = fed_feed_line("露娜", "旅人阿爾");
        assert!(f.contains("露娜") && f.contains("旅人阿爾"));
        // 空玩家名 → 泛稱「有人」。
        let g = fed_feed_line("露娜", "");
        assert!(g.contains("露娜") && g.contains("有人"));
    }

    // ── 飢餓接農田／倉庫 v2 ────────────────────────────────────────────────────

    #[test]
    fn food_ids_are_real_foods_not_seeds() {
        // 每種入口食物都被認得是食物（含季限作物·秋南瓜 v1 的南瓜）。
        for id in FOOD_IDS {
            assert!(is_food(id), "{id} 應被認作食物");
            assert!(food_name_zh(id).is_some(), "{id} 應有顯示名");
        }
        assert!(is_food(vfarm::PUMPKIN_ID), "南瓜應是食物");
        // 種子不是食物（種是拿來種、不是拿來吃）。
        assert!(!is_food(vfarm::SEEDS_ID));
        assert!(!is_food(vfarm::CARROT_SEEDS_ID));
        assert!(!is_food(vfarm::POTATO_SEEDS_ID));
        assert!(!is_food(vfarm::PUMPKIN_SEEDS_ID)); // 南瓜種子是拿來種、不是吃
        // 建材（石頭 id 之類）也不是食物。
        assert!(!is_food(0));
        assert!(!is_food(11)); // 農田土
        assert!(food_name_zh(11).is_none());
    }

    #[test]
    fn has_food_detects_any_food_stock() {
        let mut bag: HashMap<u8, u32> = HashMap::new();
        assert!(!has_food(&bag), "空背包沒食物");
        // 只有建材、種子 → 仍沒食物。
        bag.insert(4, 5); // 假設是石頭之類
        bag.insert(vfarm::SEEDS_ID, 3);
        assert!(!has_food(&bag), "只有建材/種子不算有食物");
        // 放一份麵包 → 有食物。
        bag.insert(vfarm::BREAD_ID, 1);
        assert!(has_food(&bag));
        // 存量歸零的食物鍵不算有食物。
        bag.insert(vfarm::BREAD_ID, 0);
        assert!(!has_food(&bag), "存量 0 的食物不算有");
    }

    #[test]
    fn pick_food_follows_preference_bread_first() {
        let mut bag: HashMap<u8, u32> = HashMap::new();
        // 同時有小麥顆粒與麵包 → 先吃麵包（偏好順序）。
        bag.insert(vfarm::WHEAT_ID, 3);
        bag.insert(vfarm::BREAD_ID, 1);
        assert_eq!(pick_food(&bag), Some(vfarm::BREAD_ID));
        // 只剩小麥 → 吃小麥。
        bag.remove(&vfarm::BREAD_ID);
        assert_eq!(pick_food(&bag), Some(vfarm::WHEAT_ID));
        // 沒任何食物 → None。
        bag.clear();
        assert_eq!(pick_food(&bag), None);
        // 只有種子/建材 → None。
        bag.insert(vfarm::CARROT_SEEDS_ID, 2);
        assert_eq!(pick_food(&bag), None);
    }

    #[test]
    fn consume_one_decrements_and_removes_at_zero() {
        let mut bag: HashMap<u8, u32> = HashMap::new();
        bag.insert(vfarm::CARROT_ID, 2);
        // 吃一份：2 → 1，鍵還在。
        assert!(consume_one(&mut bag, vfarm::CARROT_ID));
        assert_eq!(bag.get(&vfarm::CARROT_ID), Some(&1));
        // 再吃一份：1 → 0，鍵被移除（乾淨）。
        assert!(consume_one(&mut bag, vfarm::CARROT_ID));
        assert_eq!(bag.get(&vfarm::CARROT_ID), None);
        // 沒了還吃 → false，不 panic、不改別的鍵。
        assert!(!consume_one(&mut bag, vfarm::CARROT_ID));
        assert!(!consume_one(&mut bag, vfarm::BREAD_ID));
    }

    #[test]
    fn eat_full_cycle_pick_then_consume() {
        // 端到端：有食物 → 挑 → 吃 → 扣量後仍一致。
        let mut bag: HashMap<u8, u32> = HashMap::new();
        bag.insert(vberry::BERRY_ID, 1);
        bag.insert(vfarm::WHEAT_ID, 2);
        assert!(has_food(&bag));
        let food = pick_food(&bag).expect("該挑得到食物");
        assert!(is_food(food));
        assert!(consume_one(&mut bag, food));
        // 吃了一份莓果後莓果沒了，但還有小麥 → 仍有食物。
        assert!(has_food(&bag));
    }

    #[test]
    fn harvest_food_maps_mature_crops_to_food_and_regrow() {
        // 小麥：熟 → 小麥×1、退回已播種（田還在能再長）。
        let (id, q, back) = harvest_food_of(Block::WheatMature).unwrap();
        assert_eq!(id, vfarm::WHEAT_ID);
        assert_eq!(q, 1);
        assert_eq!(back, Block::FarmSoilSeeded);
        assert!(is_food(id));
        // 馬鈴薯量大是特色：一次收 2。
        let (pid, pq, _) = harvest_food_of(Block::PotatoMature).unwrap();
        assert_eq!(pid, vfarm::POTATO_ID);
        assert_eq!(pq, 2);
        // 胡蘿蔔。
        let (cid, _, cback) = harvest_food_of(Block::CarrotMature).unwrap();
        assert_eq!(cid, vfarm::CARROT_ID);
        assert_eq!(cback, Block::CarrotSeeded);
        // 莓果叢熟了：產莓果、退回結果前的苗（多年生）。
        let (bid, bq, bback) = harvest_food_of(Block::BerryBushRipe).unwrap();
        assert_eq!(bid, vberry::BERRY_ID);
        assert_eq!(bq, vberry::BERRY_YIELD);
        assert_eq!(bback, Block::BerryBush);
        // 沒熟／非作物 → None。
        assert!(harvest_food_of(Block::FarmSoilSeeded).is_none());
        assert!(harvest_food_of(Block::Air).is_none());
        assert!(harvest_food_of(Block::Stone).is_none());
        // is_harvestable_food_block 與 harvest_food_of 一致。
        assert!(is_harvestable_food_block(Block::WheatMature));
        assert!(!is_harvestable_food_block(Block::FarmSoil));
    }

    #[test]
    fn v2_say_lines_rotate_bounded_and_honest() {
        for pick in 0..8 {
            let ate = ate_say_line("麵包", pick);
            assert!(ate.contains("麵包") && ate.chars().count() <= 40);
            let nf = no_food_say_line(pick);
            assert!(!nf.is_empty() && nf.chars().count() <= 40);
            let fg = forage_say_line(pick);
            assert!(!fg.is_empty() && fg.chars().count() <= 40);
            let fd = foraged_say_line("莓果", pick);
            assert!(fd.contains("莓果") && fd.chars().count() <= 40);
        }
        // 沒糧誠實心聲：至少一句點出「餓」與「沒有」的窘境。
        assert!(no_food_say_line(0).contains("餓") && no_food_say_line(0).contains("沒有"));
        // 溢位取模包回、不 panic。
        assert_eq!(no_food_say_line(0), no_food_say_line(4));
        assert_eq!(forage_say_line(1), forage_say_line(5));
    }

    #[test]
    fn borrowed_say_line_rotates_bounded_and_names_food() {
        for pick in 0..8 {
            let line = borrowed_say_line("麵包", pick);
            assert!(line.contains("麵包") && line.chars().count() <= 40);
        }
        assert_eq!(borrowed_say_line("麵包", 0), borrowed_say_line("麵包", 4)); // 溢位取模包回
        // 措辭與「自己田裡收成」的 foraged_say_line 不同（凸顯「借的」與「自己收的」有別）。
        assert_ne!(borrowed_say_line("莓果", 0), foraged_say_line("莓果", 0));
    }
}
