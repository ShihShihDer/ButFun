//! 乙太方界·指令→任務 + 整地技能 v1（「居民真的照玩家的話做事」的地基）。
//!
//! **架構分層（同 voxel_skills 的鐵律）**：LLM 腦＝高層「做什麼／為什麼」；
//! 本模組＝低層「怎麼做」——全是零 LLM、零鎖、零 async 的純邏輯：
//! 偵測玩家的整地指令、整地任務資料模型、整地技能（逐格把地表帶到同一高度）。
//! 鎖／廣播／世界寫入／持久化觸發全留在 `voxel_ws.rs`。
//!
//! 這是「指令→可執行任務」的第一刀：玩家對居民說「幫我把這裡整平」，
//! 居民**真的走過去、把一塊地剷平/填平到同一高度**，而不再只是誠實地說做不到——
//! 因為合理大小的整地她現在真的做得到（答應是誠實的，不是空頭支票）。

use crate::voxel::{self, Block, WorldDelta, BASE_HEIGHT};
use crate::voxel_craft as vcraft;
use crate::voxel_skills::{self as vskill, GatherResource};
use std::collections::HashMap;

// ── 整地任務參數（v1 刻意保守：小而完整，別搞太大拖垮 tick）─────────────────────

/// 整地半徑（格）：以中心為原點，向四方各延伸這麼多格 → (2r+1)² 柱。
/// v1 固定 4 → 9×9 = 81 柱，是「一小塊地」的合理大小，居民一個人做得到。
pub const LEVEL_RADIUS: i32 = 4;

/// 每個 tick 處理幾柱（分批整地，別一次全改、免卡頓；比照採集/建造的節奏）。
/// 81 柱 ÷ 5 ≈ 17 個 tick（10Hz ≈ 1.7 秒）整完一塊——看得到在做事、又不炸 tick。
pub const LEVEL_COLUMNS_PER_STEP: usize = 5;

// ── 協調整地參數（B 階段：居民↔居民協調，露娜號召大家分工整一大片地）───────────────
//
// 大範圍整地一個人做不到 → 露娜號召其他不在忙的居民，把大區切成數塊子區各自認領，
// 多居民並行整地。每位參與者其實就是被指派一個**普通 DirectedTask**（子區），
// 因此走到工地→分批整地→完成的整套 tick 引擎、就地挪位、持久化全部原封複用；
// 協調層只多做三件事：切分子區、號召挑人、全部子區整完後宣告「大家一起整平了」。

/// 每塊子區半徑（格）：13×13 = 169 柱，一位居民做得完的一塊。
/// 比單人任務的 [`LEVEL_RADIUS`]（4）大，但仍在「看得到在做事、不炸 tick」的節奏內。
pub const COORD_CELL_RADIUS: i32 = 6;

/// 協調整地最多號召幾位居民（含發起者領隊）。v1 上限 4＝目前全部居民都能上陣，
/// 4 人 2×2 → 26×26＝676 柱的大片地（比單人 81 柱大 8 倍），做得完又看得到多人一起整。
pub const COORD_MAX_WORKERS: usize = 4;

/// **全域**每 tick 所有整地居民「合計」最多處理幾柱（守 FPS/伺服器 tick 鐵律）。
/// ＝最多人數 × 每人每 tick 柱數，是所有整地（含多位協調居民）並行時的總寫入上界：
/// 20 柱 × 單柱最多 (LEVEL_MAX_UP+LEVEL_MAX_DOWN) 方塊 → 每 tick 方塊寫入有硬上限、不會爆。
pub const MAX_LEVEL_COLUMNS_PER_TICK: usize = COORD_MAX_WORKERS * LEVEL_COLUMNS_PER_STEP;

// ── 鋪面任務參數（C 階段：把地表「鋪成」指定材料——說到就要做到）──────────────────
//
// 鋪面＝整地的加值版：範圍內逐柱整到同一高度，但**頂面用玩家指定的材料**（石磚/木板…）
// 而非草皮。材料消耗**誠實**：每鋪一柱就從執行居民的採集背包扣一份材料；不夠就先備料
// （地表採集原料→就地合成），連原料都採不到（石頭埋在地下）→ 挖「階梯礦井」取石
// （重用 voxel_skills::staircase_well 範本——邊挖邊留階、永遠走得回地面、不自困）。

/// 單人鋪面半徑（格）：7×7＝49 柱。比整地（9×9）小一號——鋪面每柱都要消耗材料、
/// 備料工序長，範圍收一點才「小而完整、做得完」。
pub const PAVE_RADIUS: i32 = 3;

/// 協調鋪面每塊子區半徑（格）：9×9＝81 柱/人。比協調整地（13×13）小——同上，
/// 每柱都吃材料，81 柱/人已是「備料＋鋪」做得完的上限。4 人 2×2 → 18×18 的大片鋪面。
pub const PAVE_COORD_CELL_RADIUS: i32 = 4;

/// 鋪面任務逾時（秒）。與整地不同：**有進展就續期**（挖到料/合成/鋪了一批都算進展），
/// 所以這是「連續這麼久毫無進展」才放棄的門檻，而非整件工程的總時限——
/// 鋪面含備料是大工程，總時限意義不大，「卡死偵測」才是逾時真正要防的事。
pub const PAVE_DEADLINE_SECS: f32 = 240.0;

/// 階梯礦井深度（階）：沿 +x 一階降 1 格。挖到第 6 階起清出的格子已進石頭層
/// （地表下 ≥4 格），一口井約可收 12–14 塊石頭＋幾塊泥土。
pub const QUARRY_DEPTH: i32 = 12;

/// 每 tick 最多清幾格礦井格（一格＝一塊方塊寫入，遠小於整一柱的寫入量；
/// 記帳上仍向全域每 tick 柱數上限扣 1 柱，見 voxel_ws 的鋪面 tick）。
pub const QUARRY_CELLS_PER_STEP: usize = 4;

/// 一件鋪面任務最多開挖幾口礦井（硬上限守成本；再多＝原料真的補不上→誠實停工）。
/// 協調鋪面每人 81 柱 ≈ 162 石 ≈ 13 口井，20 給足餘裕又有界。
pub const QUARRY_MAX_WELLS: u32 = 20;

/// 削平時往「目標高度之上」最多掃幾格（把高地/樹幹削掉的上界）。
pub const LEVEL_MAX_UP: i32 = 20;

/// 填平時往「目標高度之下」最多填幾格（把窪地/坑填起的下界）。
pub const LEVEL_MAX_DOWN: i32 = 20;

/// 視為「已抵達工地、可開始整地」的水平距離＝半徑 + 這個餘裕（格）。
/// 站在工地中心附近即可作業（居民在已整平處/邊緣動手，沿用可逃精神不自困）。
pub const LEVEL_ARRIVE_MARGIN: f32 = 2.0;

/// 任務逾時（秒）：走不到工地/整不完就放棄（避免卡死任務永不釋放）。給得寬鬆，
/// 因為玩家通常站在居民附近下令，正常情況遠在此之前就整完了。
pub const LEVEL_DEADLINE_SECS: f32 = 180.0;

/// 朝工地走卻連續這麼多秒「沒更接近」（被地形/深水/坑/牆卡死、貪心尋路繞不過）→
/// 就近把居民挪到工地邊緣可站處，讓她能真正開始整地。沿用本專案既有「卡住就脫困/挪位」
/// 精神（見 voxel_residents::rescue_resident 也會把卡死居民挪回家域）——差別是：整地是
/// **玩家交代的任務**，不能挪回家域放棄，而是挪到工地就地完成，確保「說到做到」。
pub const LEVEL_WALK_STALL_SECS: f32 = 8.0;

/// 判定「有更接近工地」的最小平方距離改善量（濾掉浮點抖動 / 沿牆滑行的原地微動，
/// 避免把「其實正在慢慢繞過去」誤判成卡住）。
pub const LEVEL_PROGRESS_EPS: f32 = 0.05;

// ── 玩家指令偵測（純函式、確定性、可測、零 LLM）──────────────────────────────

/// 「整地意圖」關鍵詞：玩家這句話像在叫居民把一塊地弄平就命中。
/// 刻意收斂——一般閒聊不含這些詞，不會誤觸發。
const LEVEL_TOKENS: &[&str] = &[
    "整平", "整地", "推平", "剷平", "鏟平", "夷平", "弄平", "挖平",
    "填平", "壓平", "弄成平地", "推成平地", "清出一塊地", "清一塊地",
    "清出塊地", "整出一塊平地", "鏟一塊地", "剷一塊地",
];

/// 「大範圍」暗示詞：出現這些＝玩家想要的整地超出居民一個人的能力（該誠實婉拒）。
/// 與 `voxel_ws::detect_over_scope` 的 SCALE_HINTS 同一組語意（此處另存一份，
/// 讓本模組保持純粹自足、可獨立測試）。
const OVERSIZE_HINTS: &[&str] = &[
    "100", "百格", "大片", "大範圍", "整片", "整塊", "一大片",
    "一整片", "全部的地", "所有的地", "這一帶", "附近全", "整座", "整個世界",
];

/// 偵測：這句玩家的話是否在叫居民「整平一塊地」。命中任一整地意圖詞即算。
/// 純函式、確定性、可測——不誤觸發一般聊天。
pub fn detect_level_command(text: &str) -> bool {
    LEVEL_TOKENS.iter().any(|t| text.contains(t))
}

/// 偵測：這句整地請求是否「大範圍」（超出居民一個人能力）。命中任一大範圍暗示詞即算。
/// B 階段起：大範圍不再一律婉拒，而是導向「號召大家協調整地」（見 `select_coord_workers`）；
/// 只有 [`is_absurd_level`] 那種連協調都不合理的才續走婉拒。純函式、可測。
pub fn is_oversized_level(text: &str) -> bool {
    OVERSIZE_HINTS.iter().any(|t| text.contains(t))
}

/// 「離譜到連號召大家一起也做不到」的暗示詞：整顆星球 / 整個世界 / 所有的地這類，
/// 就算全體居民出動也不可能完成 → 仍走誠實婉拒（協調也救不了）。純函式、可測。
/// 與 [`is_oversized_level`] 的差別：oversized 是「一個人不行、但一群人可以」→ 協調；
/// absurd 是「一群人也不行」→ 婉拒。門檻刻意收斂在「世界級」字眼。
const ABSURD_HINTS: &[&str] = &[
    "整個世界", "全世界", "整顆星球", "整片大陸", "所有的地", "全部的地", "整座星球",
];

/// 偵測：這句整地請求是否「離譜到連協調都不合理」（世界級）。命中任一即算。純函式、可測。
pub fn is_absurd_level(text: &str) -> bool {
    ABSURD_HINTS.iter().any(|t| text.contains(t))
}

/// 居民「答應整地」的回覆（誠實而願意——她現在真的做得到合理大小）。
/// 依 `pick` 選句增加變化；口吻溫暖、坦白會花點時間。純函式、可測、零 LLM。
pub fn accept_line(name: &str, pick: usize) -> String {
    const POOL: [&str; 4] = [
        "好，我這就過去把那塊地整平，會花點時間喔～",
        "交給我吧！我去把那塊地弄平，稍等我一下下～",
        "沒問題，我這就動身去整那塊地，整完再跟你說！",
        "好呀，我走過去把它剷平、填平到一樣高，做起來囉～",
    ];
    let _ = name; // 名字保留給未來想帶入口吻用；目前選句不依名字。
    POOL[pick % POOL.len()].to_string()
}

/// 居民「號召大家一起整大片地」的回覆（B 階段：誠實＋現在有辦法了）。
/// 口吻＝坦白「我一個人不行」＋「我去把大家找來一起動手」，不是婉拒、也不是逞強一個人扛。
/// 依 `pick` 選句增加變化。純函式、可測、零 LLM。
pub fn rally_line(name: &str, pick: usize) -> String {
    const POOL: [&str; 4] = [
        "這麼大一片，我一個人可不行——我去把大家找來，一起動手！",
        "哇，這範圍好大！我一個人整不完，我這就去號召大家一起來～",
        "這片地太大啦，光靠我不夠……等我一下，我去叫夥伴們一起整！",
        "一個人可搞不定這麼大一片！我去揪大家，一起把它整平～",
    ];
    let _ = name;
    POOL[pick % POOL.len()].to_string()
}

// ── 鋪面指令偵測（純函式、確定性、可測、零 LLM）────────────────────────────────

/// 鋪面材料表：（口語關鍵詞, 對應方塊）。**先長詞後短詞**（「石磚」要先於「石頭」比對
/// 才不會把「石磚」誤認成石頭類）。只收「可放置實心＋居民備得出來」的建材——
/// 原料可地表採集（沙/泥土/石頭）或由可採原料合成（石磚/木板/玻璃，見 [`pave_provision`]）。
const PAVE_MATERIALS: &[(&str, Block)] = &[
    ("石磚", Block::StoneBrick),
    ("石板", Block::StoneBrick), // 口語「鋪石板路」→ 對齊石磚
    ("木地板", Block::Plank),
    ("木板", Block::Plank),
    ("玻璃", Block::Glass),
    ("石頭", Block::Stone),
    ("石塊", Block::Stone),
    ("沙子", Block::Sand),
    ("細沙", Block::Sand),
    ("泥土", Block::Dirt),
];

/// 「鋪面動作」詞組：刻意收斂成「鋪＋接續字」的明確動作詞，
/// 不收單獨「鋪」（會誤觸發「床鋪」「店鋪」這類名詞）。
const PAVE_VERB_TOKENS: &[&str] = &[
    "鋪成", "鋪上", "鋪個", "鋪一", "鋪滿", "鋪好", "鋪出", "鋪點",
    "鋪地", "鋪路", "幫我鋪", "舖成", "舖上", "舖滿",
];

/// 「材料+地」複合詞（如「石磚地」「木板地」）搭配的動作暗示詞：光說「石磚地」可能只是
/// 聊天（「我喜歡石磚地」），要有「做/蓋/弄/要…」這類動作意圖才算指令。
const PAVE_COMPOUND_HINTS: &[&str] =
    &["做", "蓋", "弄", "幫", "來", "整", "建", "要", "給我", "如何", "怎麼樣", "好不好"];

/// 在句中找出鋪面材料（最長詞優先，表已按長詞在前排序）。純函式、可測。
fn pave_material_in(text: &str) -> Option<Block> {
    PAVE_MATERIALS.iter().find(|(kw, _)| text.contains(kw)).map(|(_, b)| *b)
}

/// 鋪面材料的顯示名（繁中；集中一處供台詞/Feed 替換，留 i18n 空間）。純函式、可測。
pub fn pave_material_name(mat: Block) -> &'static str {
    match mat {
        Block::StoneBrick => "石磚",
        Block::Plank => "木板",
        Block::Glass => "玻璃",
        Block::Stone => "石頭",
        Block::Sand => "沙子",
        Block::Dirt => "泥土",
        _ => "材料",
    }
}

/// 偵測：這句玩家的話是否在叫居民「把地鋪成某材料」，是則回傳目標材料方塊。
/// 兩種句型命中（都要求句中有材料名，材料名見 [`PAVE_MATERIALS`]）：
/// 1. 明確鋪面動詞（「鋪成/鋪上/鋪滿…」+ 材料）——「幫我把這裡鋪成石磚地」。
/// 2. 「材料+地」複合詞 + 動作暗示——「100×100石磚地 找大家做的如何」（維護者原句）。
/// 純函式、確定性、可測——不誤觸發「床鋪」「我喜歡石磚地」這類聊天。
pub fn detect_pave_command(text: &str) -> Option<Block> {
    let mat = pave_material_in(text)?;
    if PAVE_VERB_TOKENS.iter().any(|t| text.contains(t)) {
        return Some(mat);
    }
    // 「材料+地」複合詞（如「石磚地」）＋動作暗示才算（純聊天不觸發）。
    let compound = PAVE_MATERIALS
        .iter()
        .filter(|(_, b)| *b == mat)
        .any(|(kw, _)| text.contains(&format!("{kw}地")));
    if compound && PAVE_COMPOUND_HINTS.iter().any(|t| text.contains(t)) {
        return Some(mat);
    }
    None
}

// ── 鋪面備料規格（誠實消耗：材料從哪來、怎麼合成，全查得到、算得出）────────────────

/// 一種鋪面材料的備料方式：要採什麼原料、（若需合成）用哪條 2×2 基本配方。
#[derive(Clone, Copy, Debug)]
pub struct PaveProvision {
    /// 要採集的原料（居民既有採集技能採得到的資源型別）。
    pub raw: GatherResource,
    /// 合成配方：`Some((每次合成消耗原料數, 每次產出材料數))`；`None`＝原料即材料、免合成。
    pub craft: Option<(u32, u32)>,
    /// 原料是否埋在地下、可用「階梯礦井」挖到（石頭/泥土在土石層；木頭/沙只在地表）。
    pub quarryable: bool,
}

/// 查某鋪面材料的備料規格。合成比例**直接查 vcraft 配方表**（單一原料的 2×2 基本配方），
/// 不另抄一份數字——配方表改了這裡自動跟上，材料消耗永遠與玩家合成同一套帳。
/// 回 `None`＝這材料備不出來（不在支援表上；detect 端已擋，防禦性保留）。純函式、可測。
pub fn pave_provision(mat: Block) -> Option<PaveProvision> {
    // 原料即材料：地表直接採得到。
    if let Some(raw) = GatherResource::from_block(mat) {
        let quarryable = matches!(mat, Block::Stone | Block::Dirt);
        return Some(PaveProvision { raw, craft: None, quarryable });
    }
    // 需合成：找「產出正是此材料、輸入只有一種原料、且該原料採得到」的基本配方。
    let recipe = vcraft::RECIPES
        .iter()
        .find(|r| r.output_block == mat as u8 && r.inputs.len() == 1)?;
    let (in_id, in_qty) = recipe.inputs[0];
    let raw = GatherResource::from_block(Block::from_u8(in_id)?)?;
    let quarryable = matches!(raw, GatherResource::Stone | GatherResource::Dirt);
    Some(PaveProvision { raw, craft: Some((in_qty, recipe.output_count)), quarryable })
}

/// 用背包裡的原料把材料**合成到夠 `need` 份**（或原料用光為止）。
/// 回傳合成出的材料份數（0＝免合成材料 / 已夠 / 原料不足一輪）。
/// 純背包數學（吃 `&mut HashMap`）、不碰鎖/世界，可測。
pub fn craft_toward(bag: &mut HashMap<u8, u32>, mat: Block, need: u32) -> u32 {
    let Some(PaveProvision { raw, craft: Some((in_qty, out_qty)), .. }) = pave_provision(mat)
    else {
        return 0;
    };
    let mid = mat as u8;
    let rid = raw.block_id();
    let mut made = 0u32;
    while bag.get(&mid).copied().unwrap_or(0) < need {
        let have_raw = bag.get(&rid).copied().unwrap_or(0);
        if have_raw < in_qty {
            break; // 原料不夠再合一輪 → 交給呼叫端去採/挖
        }
        *bag.entry(rid).or_insert(0) -= in_qty;
        *bag.entry(mid).or_insert(0) += out_qty;
        made += out_qty;
    }
    made
}

/// 居民「答應鋪面」的回覆（誠實而願意＋坦白要先備料）。依 `pick` 選句。純函式、可測。
pub fn pave_accept_line(mat_name: &str, pick: usize) -> String {
    let pool: [String; 3] = [
        format!("好！我先去備{mat_name}，備好就把這塊地鋪起來——要花點時間喔～"),
        format!("交給我！我先採料合{mat_name}，然後把這裡鋪好，稍等我～"),
        format!("沒問題，我這就備料，把這塊地鋪成{mat_name}的！"),
    ];
    pool[pick % pool.len()].clone()
}

/// 居民「號召大家一起鋪一大片」的回覆（誠實：太大就**先從一塊開始鋪、一塊一塊來**，
/// 不拒絕也不吹牛——答應的是真做得到的那一塊）。依 `pick` 選句。純函式、可測。
pub fn pave_rally_line(mat_name: &str, pick: usize) -> String {
    let pool: [String; 3] = [
        format!("這麼大一片一口氣鋪不完——我去找大家，先從一塊開始鋪{mat_name}，一塊一塊來！"),
        format!("哇，這範圍好大！我去號召大家各自備料，先合力鋪好一塊{mat_name}地，再慢慢擴～"),
        format!("一個人可鋪不了這麼大！我去揪夥伴，大家先從一塊{mat_name}地鋪起，一塊一塊來～"),
    ];
    pool[pick % pool.len()].clone()
}

// ── 跟隨指令（指令→任務 v1 第二刀：她真的能跟你走，不只整一塊地）────────────────

/// 跟隨最長維持秒數：逾時自動結束跟隨、回家域，避免玩家忘了說「留下」就一路跟一輩子。
pub const FOLLOW_DURATION_SECS: f32 = 240.0;

/// 跟在玩家身側的固定水平偏移（格）：別完全疊在玩家腳下，站得出一個人的樣子。
pub const FOLLOW_OFFSET: f32 = 1.2;

/// 「跟我」意圖詞：命中即開始跟隨。刻意收斂——只收「跟我來/走/著」這類明確動作詞組，
/// 不收單獨「跟我」（會誤觸發「跟我說」「跟我聊聊」這類純聊天請求）。
const FOLLOW_TOKENS: &[&str] =
    &["跟我來", "跟我走", "跟著我", "跟着我", "跟上我", "陪我去", "陪我走"];

/// 「別跟了」意圖詞：命中即結束跟隨、回到平常閒晃。
const FOLLOW_STOP_TOKENS: &[&str] =
    &["別跟了", "别跟了", "不用跟了", "不用跟著我", "不用跟着我", "留在這裡", "留在这里", "你留下"];

/// 偵測：這句話是否在叫居民「跟我走」。純函式、確定性、可測。
pub fn detect_follow_command(text: &str) -> bool {
    FOLLOW_TOKENS.iter().any(|t| text.contains(t))
}

/// 偵測：這句話是否在叫居民「別跟了」。純函式、確定性、可測。
pub fn detect_follow_stop(text: &str) -> bool {
    FOLLOW_STOP_TOKENS.iter().any(|t| text.contains(t))
}

/// 居民「答應跟隨」的回覆（誠實而願意——這是她真的做得到的小事）。純函式、可測、零 LLM。
pub fn follow_accept_line(pick: usize) -> String {
    const POOL: [&str; 4] = [
        "好呀，我跟著你走～",
        "沒問題，走吧，我跟上！",
        "嗯！我跟在你身邊。",
        "好，我陪你去看看～",
    ];
    POOL[pick % POOL.len()].to_string()
}

/// 居民「應要求停止跟隨」的回覆。純函式、可測、零 LLM。
pub fn follow_stop_line(pick: usize) -> String {
    const POOL: [&str; 3] = ["好，我留在這裡～", "了解，我先待著！", "好的，你去忙吧，我在這裡等。"];
    POOL[pick % POOL.len()].to_string()
}

/// 跟隨逾時（超過 `FOLLOW_DURATION_SECS` 沒被要求停下）時，居民自己收手的台詞。
/// 純函式、可測、零 LLM。
pub fn follow_timeout_line(pick: usize) -> String {
    const POOL: [&str; 3] = ["我先回去忙自己的事囉，下次再陪你走～", "跟了你好一段路了，我先回家一趟！", "先這樣吧，有需要再叫我～"];
    POOL[pick % POOL.len()].to_string()
}

// ── 整地任務資料模型（純資料 + 純方法；hub 只存它、tick 推進它）─────────────────

/// 一個指向某居民的整地任務。中心 (cx,cz)、半徑、目標高度 target_y（該柱最高實心方塊 y），
/// cursor＝下一個要處理的柱索引（0..總柱數），deadline＝剩餘逾時秒數。
/// v1 純記憶體（重啟後任務消失可接受）；**地形改動本身走既有 world delta 持久化**。
#[derive(Clone, Debug, PartialEq)]
pub struct DirectedTask {
    /// 被指派的居民系統 id（"vox_res_0"…）。
    pub assignee: String,
    /// 下令的玩家身份鍵（供 Feed / 記憶記錄「是誰請她整的」）。
    pub requester: String,
    /// 整地中心世界座標（水平）。
    pub cx: i32,
    pub cz: i32,
    /// 半徑（格）：範圍是以中心為原點、向四方各延伸 radius 的正方形。
    pub radius: i32,
    /// 目標地表高度：整完後每柱最高實心方塊都落在這個 y。
    pub target_y: i32,
    /// 下一個要處理的柱索引（0..total_columns）。整完＝cursor 到達總柱數。
    pub cursor: usize,
    /// 剩餘逾時（秒）：每 tick 遞減，歸零仍未整完就放棄任務。
    /// 鋪面任務（pave=Some）**有進展就續期**（見 [`PAVE_DEADLINE_SECS`]）。
    pub deadline: f32,
    /// 鋪面材料：`Some(材料)`＝鋪面任務（頂面用材料、誠實扣背包）；`None`＝整地（頂面草皮）。
    pub pave: Option<Block>,
    /// 進行中的階梯礦井（鋪面備料：地表採不到原料時挖井取石/土）。整地任務恆 `None`。
    pub quarry: Option<QuarryDig>,
    /// 本任務已開挖的礦井數（守 [`QUARRY_MAX_WELLS`] 上限；也用來錯開每口井的位置）。
    pub wells_dug: u32,
    /// 上次觀察到的「背包內材料+原料總數」——變動＝備料有進展 → 逾時續期（防誤殺慢工）。
    pub prov_seen: u32,
}

impl DirectedTask {
    /// 建一個全新**整地**任務（cursor 從 0、deadline 滿格、無鋪面材料）。
    pub fn new(assignee: String, requester: String, cx: i32, cz: i32, radius: i32, target_y: i32) -> Self {
        Self {
            assignee,
            requester,
            cx,
            cz,
            radius,
            target_y,
            cursor: 0,
            deadline: LEVEL_DEADLINE_SECS,
            pave: None,
            quarry: None,
            wells_dug: 0,
            prov_seen: 0,
        }
    }

    /// 建一個全新**鋪面**任務：同整地幾何，但頂面用 `mat`、材料誠實扣背包、
    /// 逾時採「有進展就續期」制（見 [`PAVE_DEADLINE_SECS`]）。
    pub fn new_pave(
        assignee: String,
        requester: String,
        cx: i32,
        cz: i32,
        radius: i32,
        target_y: i32,
        mat: Block,
    ) -> Self {
        let mut t = Self::new(assignee, requester, cx, cz, radius, target_y);
        t.pave = Some(mat);
        t.deadline = PAVE_DEADLINE_SECS;
        t
    }

    /// 範圍邊長（柱）：2r+1。
    fn side(&self) -> usize {
        (self.radius * 2 + 1).max(1) as usize
    }

    /// 總柱數＝邊長²。
    pub fn total_columns(&self) -> usize {
        let s = self.side();
        s * s
    }

    /// 任務是否已整完（cursor 掃過全部柱）。
    pub fn is_complete(&self) -> bool {
        self.cursor >= self.total_columns()
    }

    /// 進度百分比（0..100）。
    pub fn progress_pct(&self) -> u8 {
        let total = self.total_columns().max(1);
        ((self.cursor.min(total) * 100) / total) as u8
    }

    /// 第 idx 個柱的世界座標 (x,z)（列優先展開；idx 應 < total_columns）。
    pub fn column_at(&self, idx: usize) -> (i32, i32) {
        let s = self.side();
        let dx = (idx / s) as i32;
        let dz = (idx % s) as i32;
        (self.cx - self.radius + dx, self.cz - self.radius + dz)
    }
}

// ── 整地技能核心（確定性、零 LLM、可測）──────────────────────────────────────────

/// 找某 (x,z) 柱的「最高實心方塊」y（套 delta overlay；全空回 None）。用來定 target_y。
/// 由高往低掃（涵蓋正常地形峰值 + 建物餘裕）。純函式、可測。
pub fn ground_top(world: &WorldDelta, x: i32, z: i32) -> Option<i32> {
    let top = BASE_HEIGHT + LEVEL_MAX_UP; // 涵蓋地形峰值 + 上方餘裕
    (0..=top)
        .rev()
        .find(|&y| voxel::effective_block_at(world, x, y, z).is_solid())
}

/// **整地技能·單柱**：把 (x,z) 柱的地表帶到 target_y，回傳「要改的方塊」清單（不套用）。
///
/// 規則（確定性）：
/// - 高於 target_y 的實心方塊 → 挖掉（設 Air）：削平高地、砍掉擋路的樹幹/樹冠。
/// - 低於 target_y 的空缺（非實心：空氣/水）→ 用土填：填平窪地/坑。
///   從 target_y 往下填，遇到既有實心地基就停（不無限往下挖填）。
///   最頂那格（target_y）用草皮（Grass）收面，其下用泥土（Dirt）。
/// - 已在 target_y 且其上為空 → 無改動（回空清單）。
///
/// 掃描以 [`LEVEL_MAX_UP`] / [`LEVEL_MAX_DOWN`] 為上下界，成本有界。純函式、可測。
pub fn level_column(world: &WorldDelta, x: i32, z: i32, target_y: i32) -> Vec<(i32, i32, i32, Block)> {
    let mut out = Vec::new();

    // ① 削平：target_y 之上的實心方塊全挖成空氣。
    for y in (target_y + 1)..=(target_y + LEVEL_MAX_UP) {
        if voxel::effective_block_at(world, x, y, z).is_solid() {
            out.push((x, y, z, Block::Air));
        }
    }

    // ② 填平：從 target_y 往下，遇到非實心（空氣/水）就填土；碰到既有實心地基就停。
    let bottom = (target_y - LEVEL_MAX_DOWN).max(0);
    for y in (bottom..=target_y).rev() {
        if voxel::effective_block_at(world, x, y, z).is_solid() {
            break; // 到達地基，下面不用再填
        }
        let fill = if y == target_y { Block::Grass } else { Block::Dirt };
        out.push((x, y, z, fill));
    }

    out
}

/// **整地技能·一批（可指定上限）**：從 task.cursor 起處理至多 `max_cols`（但不超過
/// 每人單 tick 的 [`LEVEL_COLUMNS_PER_STEP`]）柱，回傳（要改的方塊清單, 下一個 cursor）。
/// `max_cols` 供協調整地在鎖外套用「全域每 tick 總柱數上限」（多居民合計不爆）時剪裁最後一位
/// 參與者的批量。純函式、可測。
pub fn level_step_capped(
    world: &WorldDelta,
    task: &DirectedTask,
    max_cols: usize,
) -> (Vec<(i32, i32, i32, Block)>, usize) {
    let total = task.total_columns();
    let cap = max_cols.min(LEVEL_COLUMNS_PER_STEP);
    let mut changes = Vec::new();
    let mut cursor = task.cursor;
    let mut processed = 0usize;
    while cursor < total && processed < cap {
        let (x, z) = task.column_at(cursor);
        changes.extend(level_column(world, x, z, task.target_y));
        cursor += 1;
        processed += 1;
    }
    (changes, cursor)
}

/// **整地技能·一批**：從 task.cursor 起處理至多 [`LEVEL_COLUMNS_PER_STEP`] 柱，
/// 回傳（要改的方塊清單, 下一個 cursor）。呼叫端套用方塊、寫回 cursor（見 voxel_ws）。
/// 不碰鎖/IO——世界寫入與持久化在呼叫端。純函式、可測。
pub fn level_step(world: &WorldDelta, task: &DirectedTask) -> (Vec<(i32, i32, i32, Block)>, usize) {
    level_step_capped(world, task, LEVEL_COLUMNS_PER_STEP)
}

// ── 鋪面技能核心（確定性、零 LLM、可測）─────────────────────────────────────────

/// **鋪面技能·單柱**：把 (x,z) 柱整到 target_y、且頂面換成材料 `mat`。
/// 回傳「要改的方塊」清單（不套用）。規則（確定性）：
/// - 高於 target_y 的實心方塊 → 挖掉（同整地削平）。
/// - 頂格 (target_y)：已是 `mat` → 不動（不重鋪、不浪費材料）；否則換成 `mat`
///   （草/土/沙/石通通替換——這就是「鋪」，**每柱消耗 1 份材料**）。
/// - 頂格原本非實心（窪地/水面）→ 頂格鋪 `mat`，其下用泥土填到既有地基（同整地填平）。
/// 掃描上下界同整地（[`LEVEL_MAX_UP`]/[`LEVEL_MAX_DOWN`]），成本有界。純函式、可測。
pub fn pave_column(
    world: &WorldDelta,
    x: i32,
    z: i32,
    target_y: i32,
    mat: Block,
) -> Vec<(i32, i32, i32, Block)> {
    let mut out = Vec::new();

    // ① 削平：target_y 之上的實心方塊全挖成空氣（同整地）。
    for y in (target_y + 1)..=(target_y + LEVEL_MAX_UP) {
        if voxel::effective_block_at(world, x, y, z).is_solid() {
            out.push((x, y, z, Block::Air));
        }
    }

    // ② 頂面鋪材料：已是目標材料就跳過（省料），否則替換（消耗 1 份材料）。
    let top_now = voxel::effective_block_at(world, x, target_y, z);
    if top_now != mat {
        out.push((x, target_y, z, mat));
    }

    // ③ 填平：頂格原本非實心（窪地/水）才需要往下用泥土補地基；碰到既有實心就停。
    if !top_now.is_solid() {
        let bottom = (target_y - LEVEL_MAX_DOWN).max(0);
        for y in (bottom..target_y).rev() {
            if voxel::effective_block_at(world, x, y, z).is_solid() {
                break; // 到達地基
            }
            out.push((x, y, z, Block::Dirt));
        }
    }

    out
}

/// **鋪面技能·一批（可指定上限）**：從 task.cursor 起處理至多 `max_cols`（且不超過每人
/// 單 tick 的 [`LEVEL_COLUMNS_PER_STEP`]）柱，回傳（要改的方塊清單, 下一個 cursor,
/// **這批要消耗的材料份數**）。材料份數＝這批中「頂格需換成材料」的柱數——
/// 呼叫端先確認背包夠這個數再套用（誠實消耗，不夠就先備料）。純函式、可測。
pub fn pave_step_capped(
    world: &WorldDelta,
    task: &DirectedTask,
    max_cols: usize,
) -> (Vec<(i32, i32, i32, Block)>, usize, u32) {
    let mat = task.pave.unwrap_or(Block::Grass); // 防禦性：非鋪面任務不該呼叫本函式
    let total = task.total_columns();
    let cap = max_cols.min(LEVEL_COLUMNS_PER_STEP);
    let mut changes = Vec::new();
    let mut cursor = task.cursor;
    let mut processed = 0usize;
    let mut mat_needed = 0u32;
    while cursor < total && processed < cap {
        let (x, z) = task.column_at(cursor);
        let col = pave_column(world, x, z, task.target_y, mat);
        // 這柱是否要鋪一塊材料（頂格被換成 mat）＝消耗 1 份。
        if col.iter().any(|&(_, y, _, b)| y == task.target_y && b == mat) {
            mat_needed += 1;
        }
        changes.extend(col);
        cursor += 1;
        processed += 1;
    }
    (changes, cursor, mat_needed)
}

// ── 階梯礦井（鋪面備料：地表採不到石頭時，挖一道走得回地面的礦井取石）──────────────
//
// 重用 `voxel_skills::staircase_well` 範本（它的文件本來就寫「未來深處資源接這個範本」）：
// 沿 +x 一階降 1 格、每階清 2 格頭頂淨空 → 形成一條可踏階進出的下行坑道，**永不自困**
// （這是範本的幾何保證）。挖到第 6 階起清出的格子已在石頭層 → 挖出的實心方塊誠實入袋。

/// 一口進行中的階梯礦井：要清的格子清單（由 staircase_well 算出、絕對座標）+ 進度。
#[derive(Clone, Debug, PartialEq)]
pub struct QuarryDig {
    /// 要清成空氣的格子（依序：淺→深；每格挖到實心就入袋）。
    pub cells: Vec<(i32, i32, i32)>,
    /// 下一個要處理的格子索引。
    pub idx: usize,
}

impl QuarryDig {
    /// 這口井是否已挖完（所有格子處理過）。
    pub fn is_done(&self) -> bool {
        self.idx >= self.cells.len()
    }
}

/// 規劃一口新礦井：井口開在居民站位東側一格（絕不動她自己站的柱），
/// 第 `well_seq` 口再往 +z 錯開 2 格（平行坑道、互不打架）。
/// 井口踏面＝該柱現有地表頂。純函式（吃 &WorldDelta）、可測。
pub fn plan_quarry(world: &WorldDelta, rx: i32, rz: i32, well_seq: u32) -> QuarryDig {
    let sx = rx + 1;
    let sz = rz + (well_seq as i32) * 2;
    let sy = ground_top(world, sx, sz).unwrap_or(BASE_HEIGHT);
    QuarryDig { cells: vskill::staircase_well(sx, sy, sz, QUARRY_DEPTH), idx: 0 }
}

/// **礦井·一批**：從 q.idx 起處理至多 `max_cells` 格，回傳（要清的實心格與其原方塊, 新 idx）。
/// 只回傳**實心**格（呼叫端：設為空氣＋原方塊入袋＝誠實收料）；空氣/水格直接跳過
/// （不收料、也不動水——挖穿水脈讓水流進來是誠實的物理，交給水流模擬）。純函式、可測。
pub fn quarry_step(
    world: &WorldDelta,
    q: &QuarryDig,
    max_cells: usize,
) -> (Vec<(i32, i32, i32, Block)>, usize) {
    let mut out = Vec::new();
    let mut idx = q.idx;
    let end = (q.idx + max_cells).min(q.cells.len());
    while idx < end {
        let (x, y, z) = q.cells[idx];
        let b = voxel::effective_block_at(world, x, y, z);
        if b.is_solid() {
            out.push((x, y, z, b));
        }
        idx += 1;
    }
    (out, idx)
}

// ── 協調整地：切分子區 + 號召挑人 + 整體完成追蹤（居民↔居民協調）─────────────────────

/// 依號召到的居民數決定子區網格維度 (cols, rows)。刻意小而剛好鋪滿：
/// 2 人→2×1、3 人→3×1、4 人→2×2。1（退化，理論上協調至少 2 人）→1×1。純函式、可測。
pub fn grid_dims(worker_count: usize) -> (i32, i32) {
    match worker_count {
        0 | 1 => (1, 1),
        2 => (2, 1),
        3 => (3, 1),
        _ => (2, 2),
    }
}

/// 把以 (cx,cz) 為中心的大片地切成 `worker_count` 塊**不重疊、剛好鋪滿**的正方形子區，
/// 回傳各子區中心 (scx, scz)。每塊子區邊長 = 2·[`COORD_CELL_RADIUS`]+1，
/// 相鄰中心恰好相距一個邊長 → 子區邊界貼齊、無縫隙、無重疊。純函式、確定性、可測。
///
/// 呼叫端據此為每位居民建一個中心在 (scx,scz)、半徑 [`COORD_CELL_RADIUS`] 的普通 `DirectedTask`，
/// 於是「不重疊／覆蓋完整」的區域切分＝各子區柱集合互斥、聯集鋪滿整片大地。
pub fn partition_sub_cells(cx: i32, cz: i32, worker_count: usize) -> Vec<(i32, i32)> {
    partition_sub_cells_r(cx, cz, worker_count, COORD_CELL_RADIUS)
}

/// [`partition_sub_cells`] 的通用版：子區半徑由參數指定（協調**鋪面**用
/// [`PAVE_COORD_CELL_RADIUS`]，比整地小一號——每柱都吃材料，見該常數說明）。
/// 幾何保證同上：各子區互斥、聯集剛好鋪滿。純函式、確定性、可測。
pub fn partition_sub_cells_r(
    cx: i32,
    cz: i32,
    worker_count: usize,
    cell_radius: i32,
) -> Vec<(i32, i32)> {
    let (cols, rows) = grid_dims(worker_count);
    let cell = cell_radius * 2 + 1; // 子區邊長（柱）
    // 大片地左下角：讓整體以 (cx,cz) 置中。
    let min_x = cx - (cols * cell) / 2;
    let min_z = cz - (rows * cell) / 2;
    let mut out = Vec::new();
    for i in 0..cols {
        for j in 0..rows {
            let scx = min_x + i * cell + cell_radius;
            let scz = min_z + j * cell + cell_radius;
            out.push((scx, scz));
        }
    }
    // 網格恰好等於 worker_count（見 grid_dims）；保險 truncate 避免多給中心。
    out.truncate(worker_count.max(1));
    out
}

/// 從候選居民中挑出協調整地的參與者：**發起者（leader，被指名的露娜）永遠排第一位**，
/// 其餘依「離大片地中心近」補齊，**跳過正忙（已被指派其他整地任務）**的居民，
/// 最多 [`COORD_MAX_WORKERS`] 位。`candidates`＝(居民 id, x, z)；`busy`＝已在跑任務的居民 id。
/// 純函式、確定性（距離相同時以 id 字典序決定，避免抖動）、可測。
pub fn select_coord_workers(
    leader_id: &str,
    cx: i32,
    cz: i32,
    candidates: &[(String, f32, f32)],
    busy: &std::collections::HashSet<String>,
) -> Vec<String> {
    let mut chosen = vec![leader_id.to_string()];
    let mut rest: Vec<(f32, String)> = candidates
        .iter()
        .filter(|(id, _, _)| id != leader_id && !busy.contains(id))
        .map(|(id, x, z)| {
            let dx = *x - cx as f32;
            let dz = *z - cz as f32;
            (dx * dx + dz * dz, id.clone())
        })
        .collect();
    rest.sort_by(|a, b| {
        a.0.partial_cmp(&b.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.1.cmp(&b.1))
    });
    for (_, id) in rest {
        if chosen.len() >= COORD_MAX_WORKERS {
            break;
        }
        chosen.push(id);
    }
    chosen
}

/// 一件協調整地任務（居民↔居民協調）：記下是誰請的、以及參與的居民 id 們。
/// 每位參與者各自對應一個普通 `DirectedTask`（子區）存在 hub 的 `directed_tasks` 裡；
/// 本結構只負責「整體完成偵測」——當所有成員的子任務都消失（整完或逾時釋放）→ 這片大地完工。
/// 純資料 + 純方法。
#[derive(Clone, Debug, PartialEq)]
pub struct CoordinatedLevelTask {
    /// 下令的玩家身份鍵（供 Feed／記憶記錄「是誰請大家整的」）。
    pub requester: String,
    /// 參與整地的居民 id 們（第一位＝領隊露娜）。
    pub members: Vec<String>,
    /// 鋪面材料：`Some(材料)`＝協調**鋪面**（完工台詞/Feed 帶材料名）；`None`＝協調整地。
    pub pave: Option<Block>,
}

impl CoordinatedLevelTask {
    pub fn new(requester: String, members: Vec<String>) -> Self {
        Self { requester, members, pave: None }
    }

    /// 建一件協調**鋪面**任務（成員各自的子區任務仍是普通 `DirectedTask::new_pave`）。
    pub fn new_pave(requester: String, members: Vec<String>, mat: Block) -> Self {
        Self { requester, members, pave: Some(mat) }
    }

    /// 是否整體完成：有成員、且所有成員的子任務都已不在 `active`（仍在跑的整地任務 id 集合）中。
    /// 純函式、可測。
    pub fn all_done(&self, active: &std::collections::HashSet<String>) -> bool {
        !self.members.is_empty() && self.members.iter().all(|m| !active.contains(m))
    }
}

// ── 安全：整地時別把居民自己埋了 ─────────────────────────────────────────────────

/// 居民 AABB 半寬（與 voxel_residents::RES_HALF_W 一致；此處另存一份保持模組自足）。
const BODY_HALF_W: f32 = 0.3;
/// 居民身高（與 voxel_residents::RES_HEIGHT 一致）。
const BODY_HEIGHT: f32 = 1.7;

/// 判斷世界格 (bx,by,bz) 是否落在「腳底在 (px,py,pz) 的居民身體」佔用的方塊格內。
/// 用來在套用填塊時過濾掉「會把居民埋起來」的實心方塊（沿用可逃精神）。純函式、可測。
pub fn cell_in_body(bx: i32, by: i32, bz: i32, px: f32, py: f32, pz: f32) -> bool {
    let x0 = (px - BODY_HALF_W).floor() as i32;
    let x1 = (px + BODY_HALF_W).floor() as i32;
    let y0 = py.floor() as i32;
    let y1 = (py + BODY_HEIGHT - 0.01).floor() as i32;
    let z0 = (pz - BODY_HALF_W).floor() as i32;
    let z1 = (pz + BODY_HALF_W).floor() as i32;
    (x0..=x1).contains(&bx) && (y0..=y1).contains(&by) && (z0..=z1).contains(&bz)
}

// ── 走到工地：卡住偵測 + 就近挪位（保證「說到做到」）────────────────────────────────

/// 依「本 tick 到工地中心的平方水平距離」更新走路卡住狀態。
/// 回傳（新的最佳平方距離, 新的卡住秒數, 是否該就近挪位到工地）。
///
/// 規則（確定性）：本 tick 若比歷來最佳「更接近工地」（改善 ≥ [`LEVEL_PROGRESS_EPS`]）
/// → 記下新最佳、卡住歸零（她正在往工地走，沒卡）；否則累加卡住秒數，一旦達
/// [`LEVEL_WALK_STALL_SECS`] 就回報「該挪位」（並把卡住歸零、最佳更新為當前，讓挪位後重新計）。
/// 純函式、可測——呼叫端（voxel_ws）據 `should_relocate` 決定是否 [`nearest_site_stand`] 挪位。
pub fn walk_stall_update(best_d2: f32, stall: f32, cur_d2: f32, dt: f32) -> (f32, f32, bool) {
    if cur_d2 + LEVEL_PROGRESS_EPS < best_d2 {
        // 有更接近工地 → 沒卡，重置計時。
        (cur_d2, 0.0, false)
    } else {
        let s = stall + dt;
        if s >= LEVEL_WALK_STALL_SECS {
            // 卡太久 → 該就近挪位；重置狀態（挪位後從新位置重新計）。
            (cur_d2, 0.0, true)
        } else {
            (best_d2, s, false)
        }
    }
}

/// 工地內「最靠近 (px,pz) 且可站立」的落腳點（世界座標，腳底 y）。
/// 用途：居民朝工地走卻被卡死太久（見 [`walk_stall_update`]）時，就近把她挪到工地邊緣，
/// 讓她能開始整地——最小視覺跳動（挪到離她最近的工地柱、而非中心）、且保證任務不放棄。
///
/// 作法：把 (px,pz) 夾到工地方形範圍 [cx±r, cz±r] 得目標柱，取該柱地表頂 + 1 當腳底
///（站在方塊之上一格，重力會落穩）。柱全空則退回 `BASE_HEIGHT + 1`。純函式、可測。
pub fn nearest_site_stand(world: &WorldDelta, px: f32, pz: f32, cx: i32, cz: i32, radius: i32) -> (f32, f32, f32) {
    let tx = (px.round() as i32).clamp(cx - radius, cx + radius);
    let tz = (pz.round() as i32).clamp(cz - radius, cz + radius);
    let top = ground_top(world, tx, tz).unwrap_or(BASE_HEIGHT);
    (tx as f32 + 0.5, (top + 1) as f32, tz as f32 + 0.5)
}

// ── 單元測試 ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voxel::height_at;

    // ── detect_level_command：該中 / 不誤觸發 ────────────────────────────────────

    #[test]
    fn detect_level_command_catches_leveling_intent() {
        assert!(detect_level_command("幫我把這裡整平"));
        assert!(detect_level_command("露娜，幫我把這裡整平"));
        assert!(detect_level_command("把這塊地推平"));
        assert!(detect_level_command("可以幫我剷平這塊地嗎"));
        assert!(detect_level_command("這裡夷平一下"));
        assert!(detect_level_command("幫我整地"));
        assert!(detect_level_command("清出一塊地給我"));
        assert!(detect_level_command("把這弄平"));
        assert!(detect_level_command("填平這個坑"));
    }

    #[test]
    fn detect_level_command_ignores_chitchat() {
        assert!(!detect_level_command("你好呀，今天天氣真好"));
        assert!(!detect_level_command("你在做什麼呀"));
        assert!(!detect_level_command("這片天地好漂亮"));
        assert!(!detect_level_command("玻璃怎麼合成"));
        assert!(!detect_level_command("你叫什麼名字"));
        assert!(!detect_level_command(""));
    }

    #[test]
    fn is_oversized_level_flags_big_requests() {
        // 大範圍 → 太大（該婉拒）。
        assert!(is_oversized_level("幫我把這附近100×100的地整平"));
        assert!(is_oversized_level("把這一大片整平"));
        assert!(is_oversized_level("夷平這整片土地"));
        assert!(is_oversized_level("把百格的地推平"));
        // 小範圍「這裡/這塊」→ 不算太大（居民做得到）。
        assert!(!is_oversized_level("幫我把這裡整平"));
        assert!(!is_oversized_level("把這塊地推平"));
        assert!(!is_oversized_level("整地"));
    }

    #[test]
    fn accept_line_is_warm_and_varied() {
        let a = accept_line("露娜", 0);
        let b = accept_line("露娜", 1);
        assert!(!a.is_empty());
        assert_ne!(a, b, "不同 pick 應可選到不同句");
    }

    // ── 跟隨指令：該中 / 不誤觸發 ─────────────────────────────────────────────────

    #[test]
    fn detect_follow_command_catches_follow_intent() {
        assert!(detect_follow_command("跟我來"));
        assert!(detect_follow_command("露娜，跟我走"));
        assert!(detect_follow_command("你可以跟著我嗎"));
        assert!(detect_follow_command("跟着我"));
        assert!(detect_follow_command("陪我去那邊看看"));
    }

    #[test]
    fn detect_follow_command_ignores_chitchat_and_talk_requests() {
        // 「跟我說/聊」是要她說話，不是要她跟著走——不該誤觸發。
        assert!(!detect_follow_command("跟我說說你今天做了什麼"));
        assert!(!detect_follow_command("可以跟我聊聊天嗎"));
        assert!(!detect_follow_command("你好呀，今天天氣真好"));
        assert!(!detect_follow_command(""));
    }

    #[test]
    fn detect_follow_stop_catches_stop_intent() {
        assert!(detect_follow_stop("別跟了"));
        assert!(detect_follow_stop("好了，不用跟了"));
        assert!(detect_follow_stop("你留在這裡"));
    }

    #[test]
    fn detect_follow_stop_ignores_unrelated_chat() {
        assert!(!detect_follow_stop("跟我來"));
        assert!(!detect_follow_stop("你好呀"));
        assert!(!detect_follow_stop(""));
    }

    #[test]
    fn follow_lines_are_nonempty_and_varied() {
        let a = follow_accept_line(0);
        let b = follow_accept_line(1);
        assert!(!a.is_empty());
        assert_ne!(a, b);
        assert!(!follow_stop_line(0).is_empty());
        assert!(!follow_timeout_line(0).is_empty());
    }

    // ── DirectedTask 模型 ────────────────────────────────────────────────────────

    #[test]
    fn directed_task_geometry_and_progress() {
        let t = DirectedTask::new("vox_res_0".into(), "濕濕的".into(), 10, 20, 4, 8);
        // 9×9 = 81 柱。
        assert_eq!(t.total_columns(), 81);
        assert!(!t.is_complete());
        assert_eq!(t.progress_pct(), 0);
        // 第 0 柱＝左下角 (cx-r, cz-r)。
        assert_eq!(t.column_at(0), (6, 16));
        // 最後一柱＝右上角 (cx+r, cz+r)。
        assert_eq!(t.column_at(80), (14, 24));
        // 每柱座標都在範圍內、且互不重複。
        let mut seen = std::collections::HashSet::new();
        for i in 0..t.total_columns() {
            let (x, z) = t.column_at(i);
            assert!((6..=14).contains(&x) && (16..=24).contains(&z));
            assert!(seen.insert((x, z)), "柱座標不應重複");
        }
        assert_eq!(seen.len(), 81);
    }

    #[test]
    fn directed_task_completes_when_cursor_reaches_end() {
        let mut t = DirectedTask::new("r".into(), "p".into(), 0, 0, 1, 5); // 3×3=9
        assert_eq!(t.total_columns(), 9);
        t.cursor = 9;
        assert!(t.is_complete());
        assert_eq!(t.progress_pct(), 100);
    }

    // ── level_column：削高 / 填低 / 已平不動 ─────────────────────────────────────

    /// 造一個「乾淨的單柱」：把 (x,z) 從 y=0..=top 全設實心 Stone（模擬一根實心柱到 top）。
    fn make_solid_column(world: &mut WorldDelta, x: i32, z: i32, top: i32) {
        // 清掉 top 之上一段（保守），再把 0..=top 設實心。
        for y in (top + 1)..(top + LEVEL_MAX_UP + 2) {
            voxel::set_block(world, x, y, z, Block::Air);
        }
        for y in 0..=top {
            voxel::set_block(world, x, y, z, Block::Stone);
        }
    }

    #[test]
    fn level_column_shaves_high_ground() {
        let mut world = WorldDelta::new();
        // 一根高柱（頂在 20），target 8 → 應把 9..=20 挖成空氣。
        make_solid_column(&mut world, 100, 100, 20);
        let changes = level_column(&world, 100, 100, 8);
        assert!(!changes.is_empty());
        // 全部改動都是「挖成空氣」且在 target 之上。
        for (_, y, _, b) in &changes {
            assert_eq!(*b, Block::Air);
            assert!(*y > 8);
        }
        // 套用後最高實心＝target_y。
        let mut w2 = world.clone();
        for (x, y, z, b) in changes {
            voxel::set_block(&mut w2, x, y, z, b);
        }
        assert_eq!(ground_top(&w2, 100, 100), Some(8));
    }

    #[test]
    fn level_column_fills_low_pit() {
        let mut world = WorldDelta::new();
        // 一根矮柱（頂在 3），target 8 → 應把 4..=8 填土（頂草、下泥）。
        make_solid_column(&mut world, 200, 200, 3);
        let changes = level_column(&world, 200, 200, 8);
        assert!(!changes.is_empty());
        for (_, y, _, b) in &changes {
            assert!(*y >= 4 && *y <= 8);
            assert!(b.is_solid());
        }
        let mut w2 = world.clone();
        for (x, y, z, b) in changes {
            voxel::set_block(&mut w2, x, y, z, b);
        }
        assert_eq!(ground_top(&w2, 200, 200), Some(8));
        // 頂面是草皮。
        assert_eq!(voxel::effective_block_at(&w2, 200, 8, 200), Block::Grass);
    }

    #[test]
    fn level_column_flat_is_noop() {
        let mut world = WorldDelta::new();
        make_solid_column(&mut world, 300, 300, 8);
        let changes = level_column(&world, 300, 300, 8);
        assert!(changes.is_empty(), "已在目標高度且上方為空 → 不需改動");
    }

    // ── level_step + 迴圈：凹凸地形 → 全平（核心「她真的整平了」證據）──────────────

    #[test]
    fn level_step_flattens_bumpy_region_to_target() {
        let mut world = WorldDelta::new();
        // 造一片 radius=3（7×7）凹凸地：每柱高度依座標波動（3..=15）。
        let (cx, cz, r): (i32, i32, i32) = (500, 500, 3);
        for dx in -r..=r {
            for dz in -r..=r {
                let x = cx + dx;
                let z = cz + dz;
                // 用簡單確定性公式造高低起伏。
                let h = 3 + ((dx.abs() * 2 + dz.abs() * 3) % 12);
                make_solid_column(&mut world, x, z, h);
            }
        }
        let target_y = 8;
        let mut task = DirectedTask::new("vox_res_0".into(), "濕濕的".into(), cx, cz, r, target_y);

        // 反覆 level_step、套用改動，直到任務完成——鏡像 production 的分批整地。
        let mut guard = 0;
        while !task.is_complete() {
            let (changes, next) = level_step(&world, &task);
            for (x, y, z, b) in changes {
                voxel::set_block(&mut world, x, y, z, b);
            }
            task.cursor = next;
            guard += 1;
            assert!(guard < 1000, "整地應在有限步內完成（cursor 每步前進）");
        }

        // 驗證：範圍內每一柱地表頂都恰好在 target_y，且其上為空氣（真的變平了）。
        for dx in -r..=r {
            for dz in -r..=r {
                let x = cx + dx;
                let z = cz + dz;
                assert_eq!(
                    ground_top(&world, x, z),
                    Some(target_y),
                    "柱 ({x},{z}) 應被整平到 {target_y}"
                );
                assert_eq!(
                    voxel::effective_block_at(&world, x, target_y + 1, z),
                    Block::Air,
                    "柱 ({x},{z}) 目標高度之上應為空氣"
                );
            }
        }
    }

    #[test]
    fn level_step_advances_cursor_in_bounded_batches() {
        let world = WorldDelta::new();
        let task = DirectedTask::new("r".into(), "p".into(), 0, 0, 4, 8);
        let (_changes, next) = level_step(&world, &task);
        // 一步至多前進 LEVEL_COLUMNS_PER_STEP 柱。
        assert_eq!(next, LEVEL_COLUMNS_PER_STEP.min(task.total_columns()));
    }

    // ── ground_top：吃 delta ────────────────────────────────────────────────────

    #[test]
    fn ground_top_reads_delta_overlay() {
        let mut world = WorldDelta::new();
        // 找一個陸地點，疊一塊 delta 石頭抬高地表頂。
        let (x, z) = (0, 0);
        let base = height_at(x, z);
        voxel::set_block(&mut world, x, base + 3, z, Block::Stone);
        assert_eq!(ground_top(&world, x, z), Some(base + 3));
    }

    // ── walk_stall_update：卡住偵測（有進步歸零 / 卡太久回報挪位）────────────────────

    #[test]
    fn walk_stall_resets_when_getting_closer() {
        // 每 tick 都更接近工地 → 永遠不卡、卡住秒數保持 0。
        let (mut best, mut stall) = (f32::MAX, 0.0);
        for d2 in [100.0_f32, 80.0, 60.0, 40.0, 20.0] {
            let (nb, ns, reloc) = walk_stall_update(best, stall, d2, 0.1);
            assert!(!reloc, "正在接近工地不該挪位");
            assert_eq!(ns, 0.0, "有進步 → 卡住歸零");
            best = nb; stall = ns;
        }
        assert_eq!(best, 20.0);
    }

    #[test]
    fn walk_stall_triggers_relocate_after_stuck() {
        // 距離卡在同一值不再改善 → 累加到 LEVEL_WALK_STALL_SECS 時回報該挪位。
        let (mut best, mut stall) = (50.0_f32, 0.0);
        let mut relocated = false;
        let mut elapsed = 0.0;
        // 卡在 50.0（沒有更接近）。
        for _ in 0..(LEVEL_WALK_STALL_SECS / 0.1) as i32 + 2 {
            let (nb, ns, reloc) = walk_stall_update(best, stall, 50.0, 0.1);
            best = nb; stall = ns; elapsed += 0.1;
            if reloc { relocated = true; break; }
        }
        assert!(relocated, "卡住超過門檻應回報挪位");
        assert!(
            elapsed >= LEVEL_WALK_STALL_SECS - 0.05,
            "應在累積達門檻後才挪位：elapsed={elapsed}"
        );
    }

    #[test]
    fn walk_stall_ignores_tiny_jitter() {
        // 只有 < EPS 的微小改善（沿牆滑行抖動）不算進步 → 仍會累加卡住。
        let (nb, ns, _reloc) = walk_stall_update(50.0, 1.0, 50.0 - LEVEL_PROGRESS_EPS / 2.0, 0.1);
        assert_eq!(nb, 50.0, "微小抖動不更新最佳");
        assert!(ns > 1.0, "微小抖動不算進步 → 卡住繼續累加");
    }

    // ── nearest_site_stand：就近挪到工地可站處 ──────────────────────────────────────

    #[test]
    fn nearest_site_stand_clamps_into_site_and_stands_on_top() {
        let mut world = WorldDelta::new();
        // 工地中心 (10,10) 半徑 4 → 範圍 x,z ∈ [6,14]。造一柱地表頂在 8。
        make_solid_column(&mut world, 14, 10, 8);
        // 居民遠在 (100,10)（工地外）→ 夾到最靠近的邊緣柱 (14,10)，站在頂 (y=8) 之上一格。
        let (sx, sy, sz) = nearest_site_stand(&world, 100.0, 10.0, 10, 10, 4);
        assert_eq!((sx, sz), (14.5, 10.5), "應夾到最近的工地柱中心");
        assert_eq!(sy, 9.0, "應站在地表頂(8)之上一格");
        // 已在工地內的點 → 不外推，留在原柱。
        let (ax, _ay, az) = nearest_site_stand(&world, 11.4, 9.6, 10, 10, 4);
        assert_eq!((ax, az), (11.5, 10.5));
    }

    // ── cell_in_body：安全過濾（別把居民埋了）────────────────────────────────────

    #[test]
    fn cell_in_body_detects_occupied_cells() {
        // 居民腳底在 (10.5, 8.0, 10.5)，身高 1.7 → 佔 y=8,9 兩層、x/z=10 一格。
        assert!(cell_in_body(10, 8, 10, 10.5, 8.0, 10.5));
        assert!(cell_in_body(10, 9, 10, 10.5, 8.0, 10.5));
        // 腳下那格（y=7）不在身體內。
        assert!(!cell_in_body(10, 7, 10, 10.5, 8.0, 10.5));
        // 頭頂上方（y=10）不在身體內。
        assert!(!cell_in_body(10, 10, 10, 10.5, 8.0, 10.5));
        // 隔壁柱不在身體內。
        assert!(!cell_in_body(11, 8, 10, 10.5, 8.0, 10.5));
    }

    // ── 協調整地：偵測 / 切分 / 挑人 / 完成 / 上限 ─────────────────────────────────

    #[test]
    fn is_absurd_level_only_flags_world_scale() {
        // 世界級 → 連協調也做不到 → 婉拒。
        assert!(is_absurd_level("幫我把整個世界都整平"));
        assert!(is_absurd_level("把所有的地都推平"));
        assert!(is_absurd_level("夷平整顆星球"));
        // 「大片／整片／100×100」是大範圍但不離譜 → 該號召協調、不算 absurd。
        assert!(!is_absurd_level("幫我把這一大片地整平"));
        assert!(!is_absurd_level("把這附近100×100的地整平"));
        assert!(!is_absurd_level("整平這整片土地"));
        assert!(!is_absurd_level("幫我整地"));
    }

    #[test]
    fn oversized_but_not_absurd_is_coordinatable() {
        // 協調整地的觸發條件（voxel_ws 用）：是整地指令 + 大範圍 + 不離譜。
        let coordinatable = |t: &str| {
            detect_level_command(t) && is_oversized_level(t) && !is_absurd_level(t)
        };
        assert!(coordinatable("露娜，幫我把這一大片地整平"));
        assert!(coordinatable("把這附近100×100的地全部整平"));
        assert!(coordinatable("幫我夷平這整片土地"));
        // 世界級 → 大範圍但離譜 → 不導向協調（續走婉拒）。
        assert!(!coordinatable("把整個世界都整平"));
        // 小範圍 → 不是大範圍 → 走單人任務、不協調。
        assert!(!coordinatable("幫我把這裡整平"));
    }

    #[test]
    fn rally_line_is_a_call_not_a_refusal() {
        let a = rally_line("露娜", 0);
        let b = rally_line("露娜", 1);
        assert!(!a.is_empty());
        assert_ne!(a, b, "不同 pick 應可選到不同句");
        // 是「號召大家一起」的口吻（含關鍵字），不是「做不到」的婉拒。
        assert!(rally_line("露娜", 0).contains("大家") || rally_line("露娜", 0).contains("夥伴"));
    }

    #[test]
    fn grid_dims_pick_compact_grids() {
        assert_eq!(grid_dims(2), (2, 1));
        assert_eq!(grid_dims(3), (3, 1));
        assert_eq!(grid_dims(4), (2, 2));
        assert_eq!(grid_dims(1), (1, 1));
    }

    /// 協調整地的核心保證：把大片地切給 N 位居民，各子區**互不重疊**且**聯集鋪滿**整片。
    #[test]
    fn partition_sub_cells_are_disjoint_and_cover_fully() {
        for n in [2usize, 3, 4] {
            let (cx, cz) = (40, 60);
            let centers = partition_sub_cells(cx, cz, n);
            assert_eq!(centers.len(), n, "{n} 位居民應切出 {n} 塊子區");
            let (cols, rows) = grid_dims(n);
            let cell = (COORD_CELL_RADIUS * 2 + 1) as usize;
            let expect_cols_per_cell = cell * cell; // 每塊子區柱數
            // 收集所有子區的柱座標，驗證互斥（不重疊）。
            let mut all = std::collections::HashSet::new();
            for (scx, scz) in &centers {
                let sub = DirectedTask::new(
                    "r".into(), "p".into(), *scx, *scz, COORD_CELL_RADIUS, 8,
                );
                assert_eq!(sub.total_columns(), expect_cols_per_cell);
                for i in 0..sub.total_columns() {
                    let (x, z) = sub.column_at(i);
                    assert!(all.insert((x, z)), "子區之間不應有重疊柱 ({x},{z})");
                }
            }
            // 覆蓋完整：總柱數＝每塊 × 塊數，且恰好鋪滿 cols*cell × rows*cell 的矩形。
            assert_eq!(all.len(), expect_cols_per_cell * n);
            let span_x = (cols as usize) * cell;
            let span_z = (rows as usize) * cell;
            assert_eq!(all.len(), span_x * span_z, "聯集應剛好鋪滿整片矩形（無縫隙）");
        }
    }

    #[test]
    fn select_coord_workers_leads_with_leader_then_nearest_free() {
        let cands = vec![
            ("vox_res_0".to_string(), 100.0, 0.0), // 領隊（遠，但永遠第一）
            ("vox_res_1".to_string(), 2.0, 0.0),   // 最近
            ("vox_res_2".to_string(), 5.0, 0.0),
            ("vox_res_3".to_string(), 30.0, 0.0),  // 較遠
        ];
        let busy = std::collections::HashSet::new();
        let chosen = select_coord_workers("vox_res_0", 0, 0, &cands, &busy);
        assert_eq!(chosen.len(), COORD_MAX_WORKERS);
        assert_eq!(chosen[0], "vox_res_0", "領隊永遠第一");
        // 其餘依距中心 (0,0) 由近到遠：1(2) → 2(5) → 3(30)。
        assert_eq!(chosen[1], "vox_res_1");
        assert_eq!(chosen[2], "vox_res_2");
        assert_eq!(chosen[3], "vox_res_3");
    }

    #[test]
    fn select_coord_workers_skips_busy() {
        let cands = vec![
            ("vox_res_0".to_string(), 0.0, 0.0),
            ("vox_res_1".to_string(), 1.0, 0.0),
            ("vox_res_2".to_string(), 2.0, 0.0),
        ];
        let mut busy = std::collections::HashSet::new();
        busy.insert("vox_res_1".to_string()); // 賽勒正忙 → 不號召
        let chosen = select_coord_workers("vox_res_0", 0, 0, &cands, &busy);
        assert!(chosen.contains(&"vox_res_0".to_string()));
        assert!(chosen.contains(&"vox_res_2".to_string()));
        assert!(!chosen.contains(&"vox_res_1".to_string()), "正忙的不該被號召");
    }

    #[test]
    fn coordinated_task_completes_when_all_members_gone() {
        let task = CoordinatedLevelTask::new(
            "濕濕的".into(),
            vec!["vox_res_0".into(), "vox_res_1".into()],
        );
        // 兩位都還在跑 → 未完成。
        let mut active: std::collections::HashSet<String> =
            ["vox_res_0", "vox_res_1"].iter().map(|s| s.to_string()).collect();
        assert!(!task.all_done(&active));
        // 只剩一位 → 仍未完成。
        active.remove("vox_res_0");
        assert!(!task.all_done(&active));
        // 全部子任務都消失 → 整體完成。
        active.remove("vox_res_1");
        assert!(task.all_done(&active));
    }

    #[test]
    fn level_step_capped_respects_limit_and_bounds() {
        let world = WorldDelta::new();
        let task = DirectedTask::new("r".into(), "p".into(), 0, 0, COORD_CELL_RADIUS, 8);
        // max_cols=2 → 只前進 2 柱。
        let (_c, next) = level_step_capped(&world, &task, 2);
        assert_eq!(next, 2);
        // max_cols 超過每人上限 → 夾在 LEVEL_COLUMNS_PER_STEP。
        let (_c2, next2) = level_step_capped(&world, &task, 999);
        assert_eq!(next2, LEVEL_COLUMNS_PER_STEP.min(task.total_columns()));
        // max_cols=0 → 不前進（供全域上限用光時剪裁）。
        let (c3, next3) = level_step_capped(&world, &task, 0);
        assert!(c3.is_empty());
        assert_eq!(next3, 0);
    }

    // ── 鋪面：指令偵測 / 材料抽取 ─────────────────────────────────────────────────

    #[test]
    fn detect_pave_command_catches_paving_intent_with_material() {
        // 明確鋪面動詞 + 材料。
        assert_eq!(detect_pave_command("幫我把這裡鋪成石磚地"), Some(Block::StoneBrick));
        assert_eq!(detect_pave_command("露娜，把這塊鋪上木板好嗎"), Some(Block::Plank));
        assert_eq!(detect_pave_command("這裡鋪滿玻璃"), Some(Block::Glass));
        assert_eq!(detect_pave_command("幫我鋪一條石板路"), Some(Block::StoneBrick));
        assert_eq!(detect_pave_command("把這裡鋪成石頭"), Some(Block::Stone));
        assert_eq!(detect_pave_command("鋪點沙子在這裡"), Some(Block::Sand));
        // 「材料+地」複合詞 + 動作暗示——維護者的原句必須命中石磚。
        assert_eq!(
            detect_pave_command("是說100×100石磚地 找大家做的如何"),
            Some(Block::StoneBrick)
        );
        assert_eq!(detect_pave_command("幫我弄個木板地"), Some(Block::Plank));
    }

    #[test]
    fn detect_pave_command_prefers_longest_material_word() {
        // 「石磚」要贏過「石頭」：句中只有石磚時絕不能誤判成石頭。
        assert_eq!(detect_pave_command("把這裡鋪成石磚"), Some(Block::StoneBrick));
        // 兩者都在時，長詞（表前位）優先。
        assert_eq!(detect_pave_command("用石頭合的石磚鋪上這裡"), Some(Block::StoneBrick));
    }

    #[test]
    fn detect_pave_command_ignores_chitchat_and_levelling() {
        // 純聊天：有材料沒動作 / 有「鋪」字名詞沒材料 → 都不觸發。
        assert_eq!(detect_pave_command("我喜歡石磚地"), None);
        assert_eq!(detect_pave_command("我的床鋪旁邊有塊石頭"), None);
        assert_eq!(detect_pave_command("石磚怎麼合成呀"), None);
        assert_eq!(detect_pave_command("你好呀，今天天氣真好"), None);
        // 整地指令沒有材料 → 不是鋪面（交給整地分支）。
        assert_eq!(detect_pave_command("幫我把這裡整平"), None);
        assert_eq!(detect_pave_command(""), None);
    }

    #[test]
    fn pave_material_name_covers_supported_blocks() {
        for (_, b) in PAVE_MATERIALS {
            assert_ne!(pave_material_name(*b), "材料", "支援材料都該有專屬名字");
        }
    }

    // ── 鋪面：備料規格與合成（誠實消耗、與 vcraft 配方同一套帳）──────────────────────

    #[test]
    fn pave_provision_matches_craft_recipes() {
        // 石磚：2 石 → 2 石磚，原料石頭、可挖井。
        let p = pave_provision(Block::StoneBrick).unwrap();
        assert_eq!(p.raw, GatherResource::Stone);
        assert_eq!(p.craft, Some((2, 2)));
        assert!(p.quarryable);
        // 木板：2 木 → 4 木板，原料木頭、不可挖井（樹只在地表）。
        let p = pave_provision(Block::Plank).unwrap();
        assert_eq!(p.raw, GatherResource::Wood);
        assert_eq!(p.craft, Some((2, 4)));
        assert!(!p.quarryable);
        // 玻璃：2 沙 → 1 玻璃。
        let p = pave_provision(Block::Glass).unwrap();
        assert_eq!(p.raw, GatherResource::Sand);
        assert_eq!(p.craft, Some((2, 1)));
        assert!(!p.quarryable);
        // 原料即材料：石頭免合成、可挖井；沙免合成、不可挖井。
        let p = pave_provision(Block::Stone).unwrap();
        assert_eq!(p.raw, GatherResource::Stone);
        assert_eq!(p.craft, None);
        assert!(p.quarryable);
        let p = pave_provision(Block::Sand).unwrap();
        assert_eq!(p.craft, None);
        assert!(!p.quarryable);
        // 支援表上的每種材料都備得出來（detect 與 provision 永遠一致）。
        for (_, b) in PAVE_MATERIALS {
            assert!(pave_provision(*b).is_some(), "{b:?} 該有備料規格");
        }
    }

    #[test]
    fn craft_toward_crafts_until_enough_or_raw_runs_out() {
        // 6 石在袋，要 5 石磚：2 石→2 磚 × 3 輪 → 6 磚（≥5 停）、石 0。
        let mut bag: HashMap<u8, u32> = HashMap::from([(Block::Stone as u8, 6)]);
        let made = craft_toward(&mut bag, Block::StoneBrick, 5);
        assert_eq!(made, 6);
        assert_eq!(bag.get(&(Block::StoneBrick as u8)), Some(&6));
        assert_eq!(bag.get(&(Block::Stone as u8)), Some(&0));
        // 已夠 → 一輪都不合（不浪費原料）。
        let made = craft_toward(&mut bag, Block::StoneBrick, 5);
        assert_eq!(made, 0);
        // 原料不足一輪 → 合不出來。
        let mut bag: HashMap<u8, u32> = HashMap::from([(Block::Stone as u8, 1)]);
        assert_eq!(craft_toward(&mut bag, Block::StoneBrick, 1), 0);
        assert_eq!(bag.get(&(Block::Stone as u8)), Some(&1), "不夠合就不動原料");
        // 免合成材料（石頭直採）→ 恆 0。
        let mut bag: HashMap<u8, u32> = HashMap::new();
        assert_eq!(craft_toward(&mut bag, Block::Stone, 5), 0);
    }

    // ── 鋪面：單柱替換 / 跳過 / 整平 ─────────────────────────────────────────────

    #[test]
    fn pave_column_replaces_surface_top_with_material() {
        let mut world = WorldDelta::new();
        // 平地（頂在 8、頂面石頭）→ 鋪石磚：只換頂那一格。
        make_solid_column(&mut world, 400, 400, 8);
        let changes = pave_column(&world, 400, 400, 8, Block::StoneBrick);
        assert_eq!(changes, vec![(400, 8, 400, Block::StoneBrick)]);
    }

    #[test]
    fn pave_column_skips_already_paved() {
        let mut world = WorldDelta::new();
        make_solid_column(&mut world, 410, 410, 8);
        voxel::set_block(&mut world, 410, 8, 410, Block::StoneBrick);
        let changes = pave_column(&world, 410, 410, 8, Block::StoneBrick);
        assert!(changes.is_empty(), "頂面已是目標材料 → 不重鋪、不浪費材料");
    }

    #[test]
    fn pave_column_shaves_high_and_fills_low_with_material_top() {
        let mut world = WorldDelta::new();
        // 高柱（頂 14）→ 削到 8 且頂面換石磚。
        make_solid_column(&mut world, 420, 420, 14);
        let changes = pave_column(&world, 420, 420, 8, Block::StoneBrick);
        let mut w2 = world.clone();
        for (x, y, z, b) in &changes {
            voxel::set_block(&mut w2, *x, *y, *z, *b);
        }
        assert_eq!(ground_top(&w2, 420, 420), Some(8));
        assert_eq!(voxel::effective_block_at(&w2, 420, 8, 420), Block::StoneBrick);
        // 矮柱（頂 4）→ 填到 8：頂面石磚、其下泥土。
        let mut world = WorldDelta::new();
        make_solid_column(&mut world, 430, 430, 4);
        let changes = pave_column(&world, 430, 430, 8, Block::StoneBrick);
        let mut w2 = world.clone();
        for (x, y, z, b) in &changes {
            voxel::set_block(&mut w2, *x, *y, *z, *b);
        }
        assert_eq!(ground_top(&w2, 430, 430), Some(8));
        assert_eq!(voxel::effective_block_at(&w2, 430, 8, 430), Block::StoneBrick);
        assert_eq!(voxel::effective_block_at(&w2, 430, 7, 430), Block::Dirt);
    }

    #[test]
    fn pave_step_counts_material_need_and_respects_cap() {
        let mut world = WorldDelta::new();
        let (cx, cz): (i32, i32) = (500, 700);
        // 3×3 全平地頂面石頭；其中一柱已鋪好石磚 → 這批 9 柱只需 8 份材料。
        for dx in -1..=1 {
            for dz in -1..=1 {
                make_solid_column(&mut world, cx + dx, cz + dz, 8);
            }
        }
        voxel::set_block(&mut world, cx, 8, cz, Block::StoneBrick);
        let mut task =
            DirectedTask::new_pave("r".into(), "p".into(), cx, cz, 1, 8, Block::StoneBrick);
        // 批量上限仍受 LEVEL_COLUMNS_PER_STEP 夾住。
        let (_c, next, _need) = pave_step_capped(&world, &task, 999);
        assert_eq!(next, LEVEL_COLUMNS_PER_STEP.min(task.total_columns()));
        // 全掃一遍：材料總需求 = 9 - 1（已鋪那柱免費）。
        let mut total_need = 0u32;
        while !task.is_complete() {
            let (changes, next, need) = pave_step_capped(&world, &task, LEVEL_COLUMNS_PER_STEP);
            for (x, y, z, b) in changes {
                voxel::set_block(&mut world, x, y, z, b);
            }
            task.cursor = next;
            total_need += need;
        }
        assert_eq!(total_need, 8);
        // 完鋪驗證：每柱頂都在 8、都是石磚。
        for dx in -1..=1 {
            for dz in -1..=1 {
                assert_eq!(ground_top(&world, cx + dx, cz + dz), Some(8));
                assert_eq!(
                    voxel::effective_block_at(&world, cx + dx, 8, cz + dz),
                    Block::StoneBrick
                );
            }
        }
    }

    #[test]
    fn pave_step_flattens_bumpy_region_with_material_top() {
        // 核心「她真的鋪好了」證據：凹凸地形 → 鋪面後每柱頂＝target_y 且是材料。
        let mut world = WorldDelta::new();
        let (cx, cz, r): (i32, i32, i32) = (600, 600, PAVE_RADIUS);
        for dx in -r..=r {
            for dz in -r..=r {
                let h = 3 + ((dx.abs() * 2 + dz.abs() * 3) % 12);
                make_solid_column(&mut world, cx + dx, cz + dz, h);
            }
        }
        let target_y = 8;
        let mut task =
            DirectedTask::new_pave("vox_res_0".into(), "濕濕的".into(), cx, cz, r, target_y, Block::Plank);
        let mut guard = 0;
        while !task.is_complete() {
            let (changes, next, _need) = pave_step_capped(&world, &task, LEVEL_COLUMNS_PER_STEP);
            for (x, y, z, b) in changes {
                voxel::set_block(&mut world, x, y, z, b);
            }
            task.cursor = next;
            guard += 1;
            assert!(guard < 1000, "鋪面應在有限步內完成");
        }
        for dx in -r..=r {
            for dz in -r..=r {
                let (x, z) = (cx + dx, cz + dz);
                assert_eq!(ground_top(&world, x, z), Some(target_y), "柱 ({x},{z}) 應整到 {target_y}");
                assert_eq!(
                    voxel::effective_block_at(&world, x, target_y, z),
                    Block::Plank,
                    "柱 ({x},{z}) 頂面應是木板"
                );
                assert_eq!(voxel::effective_block_at(&world, x, target_y + 1, z), Block::Air);
            }
        }
    }

    // ── 階梯礦井：走得回地面的取石坑道 ─────────────────────────────────────────────

    #[test]
    fn plan_quarry_starts_beside_and_offsets_wells() {
        let world = WorldDelta::new();
        let q0 = plan_quarry(&world, 100, 100, 0);
        let q1 = plan_quarry(&world, 100, 100, 1);
        // 每階 2 格頭頂淨空 → 共 QUARRY_DEPTH*2 格。
        assert_eq!(q0.cells.len(), (QUARRY_DEPTH * 2) as usize);
        assert!(!q0.is_done());
        // 井口在站位東側（x+1），絕不含她自己站的柱 (100,*,100)。
        assert!(q0.cells.iter().all(|&(x, _, z)| x >= 101 && z == 100));
        // 第二口井往 +z 錯開 2 格（平行坑道）。
        assert!(q1.cells.iter().all(|&(_, _, z)| z == 102));
    }

    #[test]
    fn quarry_step_collects_solid_cells_and_reaches_stone() {
        let mut world = WorldDelta::new();
        // 造一片平地：站位 (200,300) 周邊、含坑道沿線柱全高 10（實心石柱，
        // make_solid_column 造的柱通體 Stone——夠驗「深階挖到石頭」）。
        for x in 199..(201 + QUARRY_DEPTH + 2) {
            for z in 298..306 {
                make_solid_column(&mut world, x, z, 10);
            }
        }
        let mut q = plan_quarry(&world, 200, 300, 0);
        // 一批一批挖完整口井，收集所有入袋方塊。
        let mut collected: Vec<Block> = Vec::new();
        let mut guard = 0;
        while !q.is_done() {
            let (cells, nidx) = quarry_step(&world, &q, QUARRY_CELLS_PER_STEP);
            assert!(cells.len() <= QUARRY_CELLS_PER_STEP);
            for (x, y, z, b) in cells {
                assert!(b.is_solid(), "只該回傳實心格");
                collected.push(b);
                voxel::set_block(&mut world, x, y, z, Block::Air);
            }
            q.idx = nidx;
            guard += 1;
            assert!(guard < 100, "礦井應在有限批內挖完");
        }
        // 真的收到石頭（深階格在石頭層）。
        assert!(
            collected.iter().filter(|b| **b == Block::Stone).count() >= 4,
            "一口井該收到至少幾塊石頭：{collected:?}"
        );
        // 坑道可逃性（範本幾何保證）：每階踏面仍是實心、相鄰階踏面高差 1 → 踏階走得回地面。
        let sx = 201; // 井口 x（站位+1）
        let sy = 10; // 井口踏面
        for step in 0..QUARRY_DEPTH {
            let tread = voxel::effective_block_at(&world, sx + step, sy - step, 300);
            assert!(tread.is_solid(), "第 {step} 階踏面應保留（走得回地面）");
        }
    }

    // ── 鋪面：任務模型 / 協調 ─────────────────────────────────────────────────────

    #[test]
    fn new_pave_task_carries_material_and_progress_deadline() {
        let t = DirectedTask::new_pave("r".into(), "p".into(), 0, 0, PAVE_RADIUS, 8, Block::StoneBrick);
        assert_eq!(t.pave, Some(Block::StoneBrick));
        assert_eq!(t.deadline, PAVE_DEADLINE_SECS);
        assert_eq!(t.total_columns(), 49); // 7×7
        assert!(t.quarry.is_none());
        assert_eq!(t.wells_dug, 0);
        // 整地建構子不帶鋪面欄位（既有行為零回歸）。
        let l = DirectedTask::new("r".into(), "p".into(), 0, 0, 4, 8);
        assert_eq!(l.pave, None);
        assert_eq!(l.deadline, LEVEL_DEADLINE_SECS);
    }

    #[test]
    fn partition_sub_cells_r_pave_radius_disjoint_and_cover() {
        for n in [2usize, 3, 4] {
            let centers = partition_sub_cells_r(40, 60, n, PAVE_COORD_CELL_RADIUS);
            assert_eq!(centers.len(), n);
            let cell = (PAVE_COORD_CELL_RADIUS * 2 + 1) as usize;
            let mut all = std::collections::HashSet::new();
            for (scx, scz) in &centers {
                let sub = DirectedTask::new_pave(
                    "r".into(), "p".into(), *scx, *scz, PAVE_COORD_CELL_RADIUS, 8, Block::StoneBrick,
                );
                for i in 0..sub.total_columns() {
                    let (x, z) = sub.column_at(i);
                    assert!(all.insert((x, z)), "鋪面子區之間不應重疊 ({x},{z})");
                }
            }
            let (cols, rows) = grid_dims(n);
            assert_eq!(all.len(), (cols as usize) * cell * (rows as usize) * cell, "聯集剛好鋪滿");
        }
    }

    #[test]
    fn oversized_pave_is_coordinatable_and_absurd_is_not() {
        // 觸發條件鏡像整地：鋪面指令 + 大範圍 + 不離譜 → 號召協調（先鋪上限的一塊）。
        let coordinatable = |t: &str| {
            detect_pave_command(t).is_some() && is_oversized_level(t) && !is_absurd_level(t)
        };
        assert!(coordinatable("是說100×100石磚地 找大家做的如何"));
        assert!(coordinatable("把這一大片鋪成木板"));
        // 世界級 → 連協調也做不到 → 不導向協調（續走婉拒）。
        assert!(!coordinatable("把整個世界鋪成石磚"));
        assert!(is_absurd_level("把整個世界鋪成石磚"));
        // 小範圍 → 單人鋪面、不協調。
        assert!(!coordinatable("幫我把這裡鋪成石磚地"));
    }

    #[test]
    fn pave_lines_are_honest_and_varied() {
        let a = pave_accept_line("石磚", 0);
        let b = pave_accept_line("石磚", 1);
        assert!(a.contains("石磚"));
        assert_ne!(a, b);
        // 號召句要傳達「先從一塊開始、一塊一塊來」的誠實態度（不拒絕也不吹牛）。
        for pick in 0..3 {
            let r = pave_rally_line("石磚", pick);
            assert!(r.contains("石磚"));
            assert!(r.contains("一塊"), "號召句應提到先從一塊開始：{r}");
        }
    }

    #[test]
    fn coordinated_pave_task_tracks_material() {
        let t = CoordinatedLevelTask::new_pave(
            "濕濕的".into(),
            vec!["vox_res_0".into()],
            Block::StoneBrick,
        );
        assert_eq!(t.pave, Some(Block::StoneBrick));
        // 既有整地建構子不帶材料（零回歸）。
        assert_eq!(CoordinatedLevelTask::new("p".into(), vec![]).pave, None);
    }

    #[test]
    fn per_tick_column_cap_bounds_total_writes() {
        // 全域上限＝最多人數 × 每人每 tick 柱數；模擬多位居民合計不超過此上限。
        let world = WorldDelta::new();
        let mut budget = MAX_LEVEL_COLUMNS_PER_TICK;
        let mut total_processed = 0usize;
        // 造 COORD_MAX_WORKERS + 2 位「都想整」的居民（超額者應被上限擋下）。
        for _ in 0..(COORD_MAX_WORKERS + 2) {
            if budget == 0 {
                break;
            }
            let task = DirectedTask::new("r".into(), "p".into(), 0, 0, COORD_CELL_RADIUS, 8);
            let (_c, next) = level_step_capped(&world, &task, budget);
            let processed = next - task.cursor;
            budget -= processed;
            total_processed += processed;
        }
        assert!(
            total_processed <= MAX_LEVEL_COLUMNS_PER_TICK,
            "每 tick 合計柱數不得超過全域上限"
        );
    }
}
