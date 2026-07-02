//! 乙太方界·技能發明（真進化第一刀＋第二刀）——居民自己把**基礎動作原語**組合成解法，
//! 成功後存成「自己的技能」，之後同處境**直接重用（零 LLM）**。
//!
//! **第二刀（工作台配方鏈＋放置原語）**：第一刀只開 gather/craft 兩原語、只支援 2×2
//! 背包配方——工作台 3×3 的配方鏈她發明不了。第二刀新增 `place`（把背包裡的站點方塊
//! 放到自己旁邊）與 `craft_wb`（3×3 工作台合成，前提：附近有已放置的工作台），
//! 可行性模擬懂「這配方需要工作台」＋「工作台本身可由 4 木板合成」——她第一次能自己
//! 想出**多階段鏈**：採木→合木板→合工作台→放置→3×3 合成目標物。
//!
//! 北極星（維護者原話）：「我們沒說可以挖可以放，他就自己組合出來了」——
//! 居民自己從基礎動作組合發明、存成自己的技能。這是 Voyager（MineDojo）式 skill library
//! 的精神（吸收概念、原創實作，不抄任何外部碼），長在既有 agency 架構上。
//!
//! **架構分層（同 voxel_skills 的鐵律）**：
//! - 本模組＝純邏輯側：原語白名單、LLM 計畫解析（grounded、壞輸出一律拒絕）、
//!   處境偵測、步驟推進決策、後置條件驗證、技能庫存取——零鎖、零 async、可單元測試。
//! - 鎖／廣播／世界寫入／LLM spawn 全在 `voxel_ws.rs`。
//!
//! **全鏈（第一刀打通的一條）**：
//! 處境偵測（心願提到可合成材料、背包卻沒有）
//!   → 先查**自己的**技能庫（會 → 直接執行序列，零 LLM——這就是「進化」）
//!   → 不會 → 低頻請便宜腦（think 路由）提出「原語序列計畫」（結構化 JSON）
//!   → 確定性引擎逐步執行（採集走既有 GatherSkill 安全機制、合成走真配方表）
//!   → 後置條件驗證（背包真的多了目標材料）
//!   → 成功 → 存成該居民的**具名技能**（她自己取名），append-only 持久化
//!   → 失敗 → 不存，記一次「教訓」進記憶。
//!
//! **成本紀律**：發明是低頻事件（有渴望且卡住才想、per-居民冷卻、防重入）；
//! 執行與重用**零 LLM**。LLM 輸出只能用原語白名單，解析失敗就放棄本次發明（絕不 panic）。

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::voxel::{self, Block, WorldDelta};
use crate::voxel_craft as vcraft;
use crate::voxel_skills::{column_top, GatherResource};

// ── 參數（刻意保守：小而完整）─────────────────────────────────────────────────

/// 一個計畫最多幾步（防 LLM 給出超長序列拖垮執行）。
/// 第二刀由 6 放寬到 8：工作台鏈（採木→合板×3→合工作台→放置→3×3）最長要 8 步。
pub const MAX_STEPS: usize = 8;
/// 單一採集步驟的數量上限（防「採 999 個沙」這種不合理計畫）。
pub const MAX_GATHER_COUNT: u32 = 8;
/// 技能名長度上限（進 Feed／泡泡，別讓 LLM 取出一篇小作文）。
pub const SKILL_NAME_MAX_CHARS: usize = 12;
/// 一次計畫執行的總逾時（秒）：走路採集是真實時間，給寬鬆一點；逾時放棄（記教訓）。
/// 第二刀由 300 放寬到 480：多階段鏈要跑好幾趟採集（如箱子鏈要 6 趟砍木），仍有界。
pub const RUN_TIMEOUT_SECS: f32 = 480.0;
/// 發明冷卻（秒）：一次發明嘗試後（無論成敗）至少隔這麼久才再想——別每個 tick 打 LLM。
pub const INVENT_COOLDOWN_SECS: f32 = 300.0;
/// 發明用採集的搜尋半徑（格）：比日常採集（16）大——她在「為了目標特地找材料」，
/// 值得走遠一點；仍有界，找不到就誠實失敗（記教訓），不會無限漫遊。
pub const INVENT_GATHER_RADIUS: i32 = 28;

// ── 原語（primitives）：居民已有的原子能力，正名為可組合的白名單 ────────────────
//
// v1 白名單只開兩個原語（採集／合成 2×2）。第二刀擴為 gather / craft / place
// （craft 再分隨身 2×2 與工作台 3×3 兩形），打通「多階段鏈」：採木→合木板→合工作台
// →放置到世界→3×3 合成目標物。之後（搭橋、繞路、熔爐冶煉…）再逐步開放，擴充點在這裡。

/// LLM 計畫裡的一步（serde 落地格式；載回或解析時再過 [`check_step`] 白名單驗證）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum PrimStep {
    /// 採集：把某資源採到背包裡至少 `count` 個（**後置條件語意**：背包夠了就算完成，
    /// 可自癒可重入——中途被打斷、重啟後重跑都安全）。
    Gather { resource: String, count: u32 },
    /// 合成：照 2×2 背包配方表合成一次（消耗背包配料、產物入包；grounded 在真配方表）。
    Craft { recipe: String },
    /// 工作台合成（第二刀）：照 3×3 工作台配方表合成一次。**世界前提**：附近
    /// （[`STATION_NEAR_RADIUS`] 格內）要有已放置的工作台——可行性模擬與執行都會驗。
    CraftWb { recipe: String },
    /// 放置（第二刀）：把背包裡的一個站點方塊（工作台／熔爐）放到自己旁邊的合理位置。
    /// **後置條件語意**：該型站點已在附近就跳過（不重複放、不白耗背包存量）。
    Place { block: String },
}

/// 通過白名單驗證後的一步（執行引擎吃這個；配方指標指回 `voxel_craft` 靜態表）。
#[derive(Clone, Debug, PartialEq)]
pub enum CheckedStep {
    Gather { resource: GatherResource, count: u32 },
    Craft { recipe_id: &'static str },
    /// 工作台 3×3 合成（需附近有已放置的工作台）。
    CraftWb { recipe_id: &'static str },
    /// 放置站點方塊（工作台=15／熔爐=16；白名單見 [`place_block_from_token`]）。
    Place { block_id: u8 },
}

/// 工作台方塊 id（`Block::Workbench as u8`；放置＋3×3 前提檢查的單一常數源）。
pub const WORKBENCH_BLOCK_ID: u8 = 15;
/// 熔爐方塊 id（`Block::Furnace as u8`；place 白名單第二位，冶煉鏈留給下一刀）。
pub const FURNACE_BLOCK_ID: u8 = 16;

/// place 原語的方塊 token → id（**白名單**：只准放「站點」方塊——放這些有功能意義，
/// 也是配方鏈的必要環節；裝飾性亂放不在發明的範圍，之後要開放再擴充這裡）。
pub fn place_block_from_token(s: &str) -> Option<u8> {
    match s.trim() {
        "workbench" | "工作台" => Some(WORKBENCH_BLOCK_ID),
        "furnace" | "熔爐" => Some(FURNACE_BLOCK_ID),
        _ => None,
    }
}

/// 站點方塊 id → 英文 token（存檔/白名單的正規形；與 [`place_block_from_token`] 互逆）。
pub fn place_token_of(block_id: u8) -> &'static str {
    match block_id {
        WORKBENCH_BLOCK_ID => "workbench",
        FURNACE_BLOCK_ID => "furnace",
        _ => "workbench", // 白名單外到不了這裡（check_step 已擋）；防禦性退回工作台
    }
}

/// 資源 token → 型別（白名單）。同時接受英文 token 與繁中名——小模型兩種都可能給。
pub fn resource_from_token(s: &str) -> Option<GatherResource> {
    match s.trim() {
        "grass" | "草" | "草皮" => Some(GatherResource::Grass),
        "sand" | "沙" | "沙子" | "細沙" => Some(GatherResource::Sand),
        "dirt" | "土" | "泥土" => Some(GatherResource::Dirt),
        "stone" | "石" | "石頭" => Some(GatherResource::Stone),
        "wood" | "木" | "木頭" => Some(GatherResource::Wood),
        _ => None,
    }
}

/// 「居民自己弄得到」的材料 id 集合（**不動點閉包**，第二刀核心）：
/// 從可自採原料（草/沙/土/石/木）出發，反覆納入「配料全可取得」配方的產物——
/// 木板、工作台、玻璃…乃至熔爐、箱子這些 3×3 產物全在鏈上。
/// **誠實邊界**：熔爐冶煉配方不算（用熔爐還要放置熔爐＋冶煉原語，留給下一刀），
/// 故鐵錠/鐵鎬這些要冶煉的東西仍不可發明。靜態表不變 → 惰性算一次即可。
pub fn obtainable_ids() -> &'static HashSet<u8> {
    static SET: OnceLock<HashSet<u8>> = OnceLock::new();
    SET.get_or_init(|| {
        // 用 GatherResource 的 block_id 動態組種子集（不重複硬編 id，單一事實源在 voxel_skills）。
        let mut set: HashSet<u8> = [
            GatherResource::Grass,
            GatherResource::Sand,
            GatherResource::Dirt,
            GatherResource::Stone,
            GatherResource::Wood,
        ]
        .iter()
        .map(|r| r.block_id())
        .collect();
        loop {
            let mut grew = false;
            for r in vcraft::RECIPES.iter().chain(vcraft::WORKBENCH_RECIPES.iter()) {
                if !set.contains(&r.output_block)
                    && r.inputs.iter().all(|(bid, _)| set.contains(bid))
                {
                    set.insert(r.output_block);
                    grew = true;
                }
            }
            if !grew {
                break;
            }
        }
        set
    })
}

/// 配料全部「她自己弄得到」（可自採原料或鏈上加工品）的配方，才是可發明的合成步驟。
/// （火把要煤礦、床要葉片、鐵鎬要鐵錠——她備不了料，誠實不開放。）
pub fn recipe_inventable(r: &vcraft::Recipe) -> bool {
    r.inputs.iter().all(|(bid, _)| obtainable_ids().contains(bid))
}

/// 可發明的 2×2 隨身配方清單（木板/石磚/玻璃/農田土/工作台/梯子/木石工具…）。
pub fn inventable_recipes() -> impl Iterator<Item = &'static vcraft::Recipe> {
    vcraft::RECIPES.iter().filter(|r| recipe_inventable(r))
}

/// 可發明的 3×3 工作台配方清單（大量木板/玻璃/熔爐/箱子…；鐵系要冶煉、不在鏈上）。
pub fn inventable_wb_recipes() -> impl Iterator<Item = &'static vcraft::Recipe> {
    vcraft::WORKBENCH_RECIPES.iter().filter(|r| recipe_inventable(r))
}

/// 白名單驗證一步：資源在白名單、數量 1..=上限、配方存在且可發明。壞的一律 `None`。
pub fn check_step(s: &PrimStep) -> Option<CheckedStep> {
    match s {
        PrimStep::Gather { resource, count } => {
            let res = resource_from_token(resource)?;
            if *count == 0 || *count > MAX_GATHER_COUNT {
                return None;
            }
            Some(CheckedStep::Gather { resource: res, count: *count })
        }
        PrimStep::Craft { recipe } => {
            let r = vcraft::find_recipe(recipe)?;
            if !recipe_inventable(r) {
                return None;
            }
            Some(CheckedStep::Craft { recipe_id: r.id })
        }
        PrimStep::CraftWb { recipe } => {
            // 只認 3×3 工作台表、且配料全在鏈上（鐵系要冶煉 → 拒絕）。
            let r = vcraft::find_workbench_recipe(recipe)?;
            if !recipe_inventable(r) {
                return None;
            }
            Some(CheckedStep::CraftWb { recipe_id: r.id })
        }
        PrimStep::Place { block } => {
            let bid = place_block_from_token(block)?;
            Some(CheckedStep::Place { block_id: bid })
        }
    }
}

/// 存檔技能的步數上限：正規化（每個合成步前補採集步）會讓步數長於 LLM 提案上限，
/// 給存檔版寬一點的界（仍有界，防壞檔無限長）。
pub const MAX_STORED_STEPS: usize = MAX_STEPS * 3;

/// 驗證整串步驟（共用核心）：步數 1..=cap、每步過白名單。
fn check_steps_with_cap(steps: &[PrimStep], cap: usize) -> Option<Vec<CheckedStep>> {
    if steps.is_empty() || steps.len() > cap {
        return None;
    }
    steps.iter().map(check_step).collect()
}

/// 驗證 **LLM 提案**的步驟（緊的步數上限 [`MAX_STEPS`]）。
pub fn check_steps(steps: &[PrimStep]) -> Option<Vec<CheckedStep>> {
    check_steps_with_cap(steps, MAX_STEPS)
}

/// 驗證**存檔技能**的步驟（載回重用/正規化後存檔用；上限 [`MAX_STORED_STEPS`]）。
pub fn check_stored_steps(steps: &[PrimStep]) -> Option<Vec<CheckedStep>> {
    check_steps_with_cap(steps, MAX_STORED_STEPS)
}

// ── LLM 計畫解析（grounded：只能用原語白名單，壞輸出一律拒絕、絕不 panic）────────

/// LLM 回傳的原始計畫（serde 直接對應要求的 JSON 格式）。
#[derive(Debug, Deserialize)]
struct RawPlan {
    name: String,
    steps: Vec<PrimStep>,
}

/// 解析+驗證通過的計畫（發明提案）。
#[derive(Clone, Debug, PartialEq)]
pub struct InventedPlan {
    /// 居民自己給這個技能取的名字（已清洗、截長）。
    pub name: String,
    /// 原始步驟（落地存檔用，保 serde 格式）。
    pub raw_steps: Vec<PrimStep>,
    /// 驗證後步驟（執行引擎用）。
    pub steps: Vec<CheckedStep>,
}

/// 從 LLM 輸出解析計畫：抽出第一個 `{`..最後一個 `}` 的 JSON、serde 解析、白名單驗證。
/// 任何一步失敗都回 `None`（本次發明放棄、記冷卻），絕不 panic。
pub fn parse_plan(raw: &str) -> Option<InventedPlan> {
    let start = raw.find('{')?;
    let end = raw.rfind('}')?;
    if end <= start {
        return None;
    }
    let json = &raw[start..=end];
    let plan: RawPlan = serde_json::from_str(json).ok()?;
    let name: String = plan.name.trim().chars().take(SKILL_NAME_MAX_CHARS).collect();
    if name.is_empty() {
        return None;
    }
    let steps = check_steps(&plan.steps)?;
    Some(InventedPlan { name, raw_steps: plan.steps, steps })
}

// ── 處境偵測（純函式、確定性、零 LLM）────────────────────────────────────────────

/// 一個「想要卻沒有的材料」目標。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialGoal {
    /// 目標材料的方塊 id（後置條件驗證用）。
    pub block_id: u8,
    /// 面向玩家的繁中名（Feed／泡泡／prompt 用）。
    pub name_zh: &'static str,
}

/// 心願文字裡的「可合成材料」關鍵詞 → 目標。只收可發明配方的產物
/// （她真的有路子自己做出來的東西），別的詞不誤觸發。
/// 第二刀新增 3×3 工作台鏈產物：熔爐（8 石）、箱子（8 木板）——配料全可自採，
/// 完整鏈閉環（火把要煤礦、鐵鎬要鐵錠，備不了料，誠實不收）。
const MATERIAL_KEYWORDS: [(&str, u8); 6] = [
    ("玻璃", 10),
    ("木板", 8),
    ("石磚", 9),
    ("農田土", 11),
    ("熔爐", FURNACE_BLOCK_ID),
    ("箱子", 42),
];

/// 偵測：這句心願是否提到某個「可合成材料」。命中回目標，否則 `None`。
/// 純函式、確定性、可測——一般心願（想要小屋/水井）不會誤觸發。
pub fn detect_missing_material(desire: &str) -> Option<MaterialGoal> {
    for (kw, bid) in MATERIAL_KEYWORDS {
        if desire.contains(kw) {
            return Some(MaterialGoal { block_id: bid, name_zh: material_name(bid) });
        }
    }
    None
}

/// 材料 id → 繁中名（覆蓋目標材料＋站點方塊；單一事實源在 voxel_craft 的 name_zh，
/// 此處為靜態便利查表——與 MATERIAL_KEYWORDS / place 白名單同步維護）。
pub fn material_name(block_id: u8) -> &'static str {
    match block_id {
        10 => "玻璃",
        8 => "木板",
        9 => "石磚",
        11 => "農田土",
        WORKBENCH_BLOCK_ID => "工作台",
        FURNACE_BLOCK_ID => "熔爐",
        42 => "箱子",
        _ => "材料",
    }
}

// ── 執行引擎的純決策（步驟推進 + 合成套用 + 後置條件）────────────────────────────

/// 一次「發明/重用技能」的執行狀態（掛在居民身上；每 tick 扣 deadline）。
#[derive(Clone, Debug)]
pub struct InventRun {
    /// 觸發處境：想要這個材料。
    pub goal_block: u8,
    /// 材料繁中名（台詞用）。
    pub goal_name: String,
    /// 技能名（發明中＝LLM 取的名；重用＝既有技能名）。
    pub skill_name: String,
    /// 原始步驟（成功存檔時直接用，不必回轉）。
    pub raw_steps: Vec<PrimStep>,
    /// 驗證後步驟（執行用）。
    pub steps: Vec<CheckedStep>,
    /// 目前執行到第幾步。
    pub step_idx: usize,
    /// `true`＝重用自己已存的技能（零 LLM）；`false`＝首次發明的驗證執行。
    pub reuse: bool,
    /// 剩餘秒數（tick 遞減；逾時放棄、記教訓）。
    pub deadline: f32,
}

impl InventRun {
    /// 由發明提案建執行狀態。
    pub fn from_plan(goal_block: u8, goal_name: &str, plan: &InventedPlan, reuse: bool) -> Self {
        Self {
            goal_block,
            goal_name: goal_name.to_string(),
            skill_name: plan.name.clone(),
            raw_steps: plan.raw_steps.clone(),
            steps: plan.steps.clone(),
            step_idx: 0,
            reuse,
            deadline: RUN_TIMEOUT_SECS,
        }
    }

    /// 是否已逾時（放棄、記教訓）。
    pub fn is_expired(&self) -> bool {
        self.deadline <= 0.0
    }
}

/// 步驟推進決策的結果（呼叫端據此動手；本函式不碰世界/鎖）。
#[derive(Clone, Debug, PartialEq)]
pub enum StepAction {
    /// 這一步的採集還沒滿足 → 去採該資源（呼叫端設 GatherSkill）。
    StartGather { resource: GatherResource },
    /// 這一步是合成 → 執行 [`craft_apply`]（成功則推進，失敗＝計畫失敗）。
    DoCraft { recipe_id: &'static str },
    /// 這一步是工作台 3×3 合成 → 呼叫端先驗「附近真的有工作台」再套配方（沒有＝失敗）。
    DoCraftWb { recipe_id: &'static str },
    /// 這一步是放置站點 → 呼叫端找放置點、扣背包、寫世界（找不到點/沒貨＝失敗）。
    DoPlace { block_id: u8 },
    /// 這一步的後置條件已滿足 → step_idx+1 再看下一步。
    Advance,
    /// 全部步驟跑完 → 呼叫端做最終後置條件驗證（[`goal_met`]）。
    Done,
}

/// 看目前這一步該做什麼（**後置條件語意**：採集步看「背包夠不夠」而非「採了幾次」，
/// 已有存量直接跳過、中斷重跑也安全；放置步看「該型站點是否已在附近」，已有就跳過）。
/// `station_near(block_id)`：由呼叫端提供的世界查詢（純函式側不碰鎖）。可測。
pub fn next_action(
    run: &InventRun,
    bag: &HashMap<u8, u32>,
    station_near: impl Fn(u8) -> bool,
) -> StepAction {
    match run.steps.get(run.step_idx) {
        None => StepAction::Done,
        Some(CheckedStep::Gather { resource, count }) => {
            let have = bag.get(&resource.block_id()).copied().unwrap_or(0);
            if have >= *count {
                StepAction::Advance
            } else {
                StepAction::StartGather { resource: *resource }
            }
        }
        Some(CheckedStep::Craft { recipe_id }) => StepAction::DoCraft { recipe_id },
        Some(CheckedStep::CraftWb { recipe_id }) => StepAction::DoCraftWb { recipe_id },
        Some(CheckedStep::Place { block_id }) => {
            // 後置條件語意：這型站點已在附近（她之前放的、或世界本來就有）→ 跳過，
            // 不重複放、背包那份留著（可自癒可重入，重用技能時尤其省）。
            if station_near(*block_id) {
                StepAction::Advance
            } else {
                StepAction::DoPlace { block_id: *block_id }
            }
        }
    }
}

/// 對背包套用一次合成：配料夠 → 扣配料、加產物、回 `true`；不夠 → 不動、回 `false`。
/// 純函式（吃 &mut HashMap）、可測；呼叫端在 res_inv 寫鎖內用它。
pub fn craft_apply(bag: &mut HashMap<u8, u32>, recipe: &vcraft::Recipe) -> bool {
    // 先驗證全部配料都夠（不部分扣、不留爛帳）。
    for (bid, need) in recipe.inputs {
        if bag.get(bid).copied().unwrap_or(0) < *need {
            return false;
        }
    }
    for (bid, need) in recipe.inputs {
        if let Some(c) = bag.get_mut(bid) {
            *c -= *need;
        }
    }
    *bag.entry(recipe.output_block).or_insert(0) += recipe.output_count;
    true
}

/// 從背包扣 1 個指定方塊：有 → 扣、回 `true`；沒有 → 不動、回 `false`。
/// 純函式（吃 &mut HashMap）、可測；放置步的執行端在 res_inv 寫鎖內用它。
pub fn take_one(bag: &mut HashMap<u8, u32>, block_id: u8) -> bool {
    match bag.get_mut(&block_id) {
        Some(c) if *c >= 1 => {
            *c -= 1;
            true
        }
        _ => false,
    }
}

/// 最終後置條件：背包裡真的有目標材料（≥1）——「她真的做出來了」的可驗證證據。
pub fn goal_met(bag: &HashMap<u8, u32>, goal_block: u8) -> bool {
    bag.get(&goal_block).copied().unwrap_or(0) >= 1
}

/// 合成步在模擬中缺料的錯誤訊息（隨身/工作台合成共用；回饋給便宜腦修正用）。
fn craft_shortage_err(r: &vcraft::Recipe) -> String {
    let need: Vec<String> = r
        .inputs
        .iter()
        .map(|(bid, n)| format!("{}×{}", input_name(*bid), n))
        .collect();
    format!(
        "合成「{}」需要 {}，但照這個計畫走到這一步時背包裡的材料不夠——\
        採集步驟的數量必須足以支付後續所有合成的配料",
        r.name_zh,
        need.join("+")
    )
}

/// **計畫可行性模擬**（純資料、零世界、零 LLM）：在提案階段就用後置條件語意把整串
/// 步驟走一遍——採集步把背包補到至少 `count`、合成步照真配方扣料/產出、
/// 放置步扣站點方塊並記「工作台已就位」、工作台合成步驗「先有工作台才行」——
/// 抓出「採 1 個木頭卻要合成需 2 木的木板」「還沒放工作台就 3×3」這類不通的計畫。
/// `start_wb_nearby`：她附近本來就有已放置的工作台（世界快照，由呼叫端查好傳入）。
/// 回 `Err(繁中原因)` 時，呼叫端可把原因回饋給便宜腦**重試一次**（Voyager 式迭代精煉，
/// 成本有界），省得她白走一趟才發現計畫行不通。純函式、可測。
pub fn simulate_plan(
    steps: &[CheckedStep],
    start_bag: &HashMap<u8, u32>,
    goal_block: u8,
    start_wb_nearby: bool,
) -> Result<(), String> {
    let mut bag = start_bag.clone();
    let mut wb_nearby = start_wb_nearby;
    for s in steps {
        match s {
            CheckedStep::Gather { resource, count } => {
                // 後置條件語意：這一步結束時背包至少有 count 個（已夠就不多採）。
                let e = bag.entry(resource.block_id()).or_insert(0);
                if *e < *count {
                    *e = *count;
                }
            }
            CheckedStep::Craft { recipe_id } => {
                let r = vcraft::find_recipe(recipe_id)
                    .ok_or_else(|| format!("配方 {recipe_id} 不存在"))?;
                if !craft_apply(&mut bag, r) {
                    return Err(craft_shortage_err(r));
                }
            }
            CheckedStep::CraftWb { recipe_id } => {
                let r = vcraft::find_workbench_recipe(recipe_id)
                    .ok_or_else(|| format!("工作台配方 {recipe_id} 不存在"))?;
                // 依賴順序：3×3 合成前必須已有工作台在旁（本來就有、或計畫前段放好）。
                if !wb_nearby {
                    return Err(format!(
                        "「{}」是工作台配方，必須先有工作台在你旁邊——工作台可用配方 id\
                        「workbench」（木板×4）合成，再用 place 放置，然後才能 craft_wb",
                        r.name_zh
                    ));
                }
                if !craft_apply(&mut bag, r) {
                    return Err(craft_shortage_err(r));
                }
            }
            CheckedStep::Place { block_id } => {
                // 後置條件語意（與 next_action 一致）：工作台已在附近就跳過、不白耗。
                if *block_id == WORKBENCH_BLOCK_ID && wb_nearby {
                    continue;
                }
                if !take_one(&mut bag, *block_id) {
                    return Err(format!(
                        "要放置「{}」，但照這個計畫走到這一步時背包裡沒有它——\
                        必須先把它合成出來才能放置",
                        material_name(*block_id)
                    ));
                }
                if *block_id == WORKBENCH_BLOCK_ID {
                    wb_nearby = true;
                }
            }
        }
    }
    if !goal_met(&bag, goal_block) {
        return Err(format!(
            "整個計畫跑完，背包裡仍然不會有目標材料「{}」",
            material_name(goal_block)
        ));
    }
    Ok(())
}

/// 資源型別 → 英文 token（存檔/白名單的正規形）。
pub fn token_of(res: GatherResource) -> &'static str {
    match res {
        GatherResource::Grass => "grass",
        GatherResource::Sand => "sand",
        GatherResource::Dirt => "dirt",
        GatherResource::Stone => "stone",
        GatherResource::Wood => "wood",
    }
}

/// 存檔前把計畫**正規化成自足技能**：邊「從空背包模擬」邊補上缺料的備料步。
///
/// 為什麼需要：便宜腦看得到「當下背包／附近的工作台」——若她手上剛好有料或旁邊剛好
/// 有工作台，提出的計畫可能省略備料/放置步（實測真的發生：背包已有木頭×2 → 計畫＝
/// `[craft plank]`）。這種計畫**當下**可行，但存成技能後換個時空就跑不動——
/// 技能該是帶著走的本事，不依賴運氣。
///
/// **第二刀做法（模擬式正規化）**：帶著一個「從空開始」的模擬背包逐步走原計畫，
/// 遇到缺料才插入備料步——可採原料補採集步（後置條件語意，有料時零成本 no-op）；
/// 加工品（木板等）遞迴用 2×2 配方鏈補「採料＋合成」；`craft_wb` 前若模擬中還沒有
/// 工作台，插入「備妥工作台＋放置」整組。模擬讓插入**最少且不重複**（不會為每個
/// 合成步盲目重備料）→ 正規化後的技能**從空背包、身邊沒工作台也保證可行**。
/// 純函式、確定性、可測。
pub fn canonicalize_steps(steps: &[CheckedStep]) -> Vec<PrimStep> {
    let mut out = Vec::new();
    let mut bag: HashMap<u8, u32> = HashMap::new(); // 模擬背包：從空開始
    let mut wb_placed = false; // 模擬世界：從「身邊沒工作台」開始
    for s in steps {
        match s {
            CheckedStep::Gather { resource, count } => {
                out.push(PrimStep::Gather {
                    resource: token_of(*resource).to_string(),
                    count: *count,
                });
                let e = bag.entry(resource.block_id()).or_insert(0);
                if *e < *count {
                    *e = *count;
                }
            }
            CheckedStep::Craft { recipe_id } => {
                if let Some(r) = vcraft::find_recipe(recipe_id) {
                    ensure_inputs(r, &mut bag, &mut out);
                    let _ = craft_apply(&mut bag, r); // 備料理應足夠；防禦性忽略失敗
                }
                out.push(PrimStep::Craft { recipe: recipe_id.to_string() });
            }
            CheckedStep::CraftWb { recipe_id } => {
                if let Some(r) = vcraft::find_workbench_recipe(recipe_id) {
                    // 依賴順序：3×3 前必須有工作台——模擬中還沒有就補「備妥＋放置」整組。
                    if !wb_placed {
                        ensure_have(WORKBENCH_BLOCK_ID, 1, &mut bag, &mut out, ENSURE_MAX_DEPTH);
                        out.push(PrimStep::Place {
                            block: place_token_of(WORKBENCH_BLOCK_ID).to_string(),
                        });
                        let _ = take_one(&mut bag, WORKBENCH_BLOCK_ID);
                        wb_placed = true;
                    }
                    ensure_inputs(r, &mut bag, &mut out);
                    let _ = craft_apply(&mut bag, r);
                }
                out.push(PrimStep::CraftWb { recipe: recipe_id.to_string() });
            }
            CheckedStep::Place { block_id } => {
                ensure_have(*block_id, 1, &mut bag, &mut out, ENSURE_MAX_DEPTH);
                out.push(PrimStep::Place { block: place_token_of(*block_id).to_string() });
                let _ = take_one(&mut bag, *block_id);
                if *block_id == WORKBENCH_BLOCK_ID {
                    wb_placed = true;
                }
            }
        }
    }
    out
}

/// [`ensure_have`] 的遞迴深度上限（鏈：木→木板→工作台，深度 3 就夠；留餘裕仍有界）。
const ENSURE_MAX_DEPTH: u8 = 4;

/// 正規化輔助：確保「這個配方的所有配料」在模擬背包裡足夠（缺的補備料步）。
fn ensure_inputs(r: &vcraft::Recipe, bag: &mut HashMap<u8, u32>, out: &mut Vec<PrimStep>) {
    for (bid, need) in r.inputs {
        ensure_have(*bid, *need, bag, out, ENSURE_MAX_DEPTH);
    }
}

/// 正規化輔助：確保模擬背包裡至少有 `need` 個 `bid`——
/// 可採原料補一個採集步（後置條件語意）；加工品用 2×2 隨身配方遞迴補「採料＋合成」
/// （鏈上的中間材料——木板/工作台——全是 2×2 產物，夠用；3×3 不當中間備料路徑，
/// 免得遞迴又引入工作台依賴）。弄不到的（理論到不了：白名單已擋）就不動，
/// 交給存檔驗證把關。純函式、有界（深度/圈數雙上限）、可測。
fn ensure_have(bid: u8, need: u32, bag: &mut HashMap<u8, u32>, out: &mut Vec<PrimStep>, depth: u8) {
    if bag.get(&bid).copied().unwrap_or(0) >= need {
        return; // 模擬背包已夠（前段步驟備過）→ 不重複插入
    }
    // 可採原料：補「確保背包至少 need 個」的採集步（有料時零成本 no-op）。
    if let Some(res) = Block::from_u8(bid).and_then(GatherResource::from_block) {
        // 防禦性截頂：現有配方單一原料最大需求 = 8（熔爐的石頭），不會真的截斷。
        let want = need.min(MAX_GATHER_COUNT);
        out.push(PrimStep::Gather { resource: token_of(res).to_string(), count: want });
        let e = bag.entry(bid).or_insert(0);
        if *e < want {
            *e = want;
        }
        return;
    }
    if depth == 0 {
        return; // 防禦：閉包內鏈深有限，理論到不了
    }
    if let Some(r) = vcraft::RECIPES
        .iter()
        .find(|r| r.output_block == bid && recipe_inventable(r))
    {
        let mut guard = 0;
        while bag.get(&bid).copied().unwrap_or(0) < need && guard < 8 {
            guard += 1;
            for (ibid, n) in r.inputs {
                ensure_have(*ibid, *n, bag, out, depth - 1);
            }
            if !craft_apply(bag, r) {
                break; // 防禦：備料失敗（不該發生）就停，別無限迴圈
            }
            out.push(PrimStep::Craft { recipe: r.id.to_string() });
        }
    }
}

/// 重試 prompt（Voyager 式迭代精煉的第二輪）：附上上一次的計畫與失敗原因，請腦修正。
/// 只重試一次（成本有界）。純函式、可測。
pub fn retry_user_prompt(base_user: &str, prev_raw: &str, reason: &str) -> String {
    let prev_head: String = prev_raw.chars().take(300).collect();
    format!(
        "{base_user}\n\n你上一次提出的計畫是：{prev_head}\n\
        但它行不通：{reason}。請修正後重新輸出，仍然只輸出一個 JSON 物件、只用允許的原語。"
    )
}

// ── 世界查詢純函式（吃 &WorldDelta、零鎖；鎖的取放在 voxel_ws 呼叫端）────────────

/// 站點（工作台/熔爐）視為「在旁邊、可以用」的水平半徑（格）。
/// 與 [`find_place_spot`] 的放置環（≤2 格）配套：她剛放好的站點一定查得到。
pub const STATION_NEAR_RADIUS: i32 = 4;
/// 站點搜尋的垂直範圍（±格）：站點放在腳邊地表，高低差不會超過這個數。
pub const STATION_NEAR_YSPAN: i32 = 3;
/// 放置點搜尋環的最大半徑（格）：緊鄰腳邊 1~2 格，放好就在「旁邊」。
pub const PLACE_RING_MAX: i32 = 2;
/// 放置點與腳底的最大垂直落差（格）：太高搆不到、太低是坑，都不放。
pub const PLACE_MAX_DY: i32 = 2;

/// 附近是否有某型站點方塊（工作台/熔爐）已放置在世界裡（她自己放的、玩家放的都算）。
/// 掃居民腳邊一個小立方範圍的有效方塊（含 delta overlay）。純函式、可測。
pub fn station_nearby(world: &WorldDelta, fx: i32, fy: i32, fz: i32, block_id: u8) -> bool {
    let Some(want) = Block::from_u8(block_id) else {
        return false;
    };
    for dx in -STATION_NEAR_RADIUS..=STATION_NEAR_RADIUS {
        for dz in -STATION_NEAR_RADIUS..=STATION_NEAR_RADIUS {
            for dy in -STATION_NEAR_YSPAN..=STATION_NEAR_YSPAN {
                if voxel::effective_block_at(world, fx + dx, fy + dy, fz + dz) == want {
                    return true;
                }
            }
        }
    }
    false
}

/// 找「把方塊放到自己旁邊」的合理位置（放置原語的安全核心，比照居民建造的放置語意：
/// 寫進 delta、廣播、持久化都由呼叫端做，本函式只挑格子）。規則（確定性、可測）：
/// 1. **絕不放在自己身體格**：只掃半徑 1~2 的「環」，自己站的那一柱（dx=dz=0）天生不在環上。
/// 2. 放在鄰柱**地表頂上**（column_top+1）：站點要坐在實地上，不懸空、不埋進地裡。
/// 3. 目標格必須是**空氣**（不是水、不是別人的建物/作物——放得到才算，放不到誠實失敗）。
/// 4. 與腳底垂直落差 ≤ [`PLACE_MAX_DY`]：伸手可及，不會把東西放到搆不到的高台或深坑。
/// 由內圈往外找，找不到回 `None`（呼叫端把這一步當失敗收尾，不硬塞）。
pub fn find_place_spot(world: &WorldDelta, fx: i32, fy: i32, fz: i32) -> Option<(i32, i32, i32)> {
    for r in 1..=PLACE_RING_MAX {
        for dx in -r..=r {
            for dz in -r..=r {
                if dx.abs().max(dz.abs()) != r {
                    continue; // 只走環邊界，不重掃內圈（也永遠掃不到 0,0 自己）
                }
                let (x, z) = (fx + dx, fz + dz);
                let Some(top) = column_top(world, x, z) else {
                    continue;
                };
                let y = top + 1;
                if voxel::effective_block_at(world, x, y, z) != Block::Air {
                    continue; // 頂上有水/植物/建物 → 不搶格子
                }
                if (y - fy).abs() > PLACE_MAX_DY {
                    continue; // 太高搆不到 / 太低是坑
                }
                return Some((x, y, z));
            }
        }
    }
    None
}

// ── 技能庫（個體的、持久化的：露娜會的諾娃不一定會）──────────────────────────────

/// 一筆「居民自己發明的技能」（jsonl 落地單位，append-only、向後相容）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InventedSkillRecord {
    /// 居民系統 id（"vox_res_0"…）——技能是**個體的**。
    pub resident: String,
    /// 她自己取的技能名（如「燒玻璃」）。
    pub name: String,
    /// 觸發處境：想要這個材料但背包沒有。
    pub goal_block: u8,
    /// 原語序列（serde 格式；載回時再過白名單驗證，配方表若變動壞技能自然失效不執行）。
    pub steps: Vec<PrimStep>,
    /// 單調遞增序號。
    pub seq: u64,
}

/// 每居民技能庫。重啟後載回——「她已經會了」跨重啟仍然會。
#[derive(Default)]
pub struct InventedSkillStore {
    skills: Vec<InventedSkillRecord>,
    next_seq: u64,
}

impl InventedSkillStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 由 jsonl 記錄還原。同一 (居民, 目標材料) 多筆只留 seq 最大（最新學會的版本）。
    pub fn from_entries(entries: Vec<InventedSkillRecord>) -> Self {
        let mut s = Self::default();
        for e in entries {
            if e.seq >= s.next_seq {
                s.next_seq = e.seq.wrapping_add(1);
            }
            s.skills.retain(|k| !(k.resident == e.resident && k.goal_block == e.goal_block));
            s.skills.push(e);
        }
        s
    }

    /// 這位居民面對「想要材料 X」的處境，是否已有自己的技能。**個體查詢**：
    /// 只查她自己的，別人會的不算（教別人是之後的事）。
    pub fn find_for(&self, resident: &str, goal_block: u8) -> Option<&InventedSkillRecord> {
        self.skills
            .iter()
            .find(|k| k.resident == resident && k.goal_block == goal_block)
    }

    /// 存下一個新技能；同處境已有技能則不重複存（回 `None`）。回傳 record 供 append 落地。
    pub fn add(
        &mut self,
        resident: &str,
        name: &str,
        goal_block: u8,
        steps: Vec<PrimStep>,
    ) -> Option<InventedSkillRecord> {
        if self.find_for(resident, goal_block).is_some() {
            return None;
        }
        let rec = InventedSkillRecord {
            resident: resident.to_string(),
            name: name.to_string(),
            goal_block,
            steps,
            seq: self.next_seq,
        };
        self.next_seq = self.next_seq.wrapping_add(1);
        self.skills.push(rec.clone());
        Some(rec)
    }

    /// 這位居民已學會的技能名清單（對話時可以自豪地講出來）。
    pub fn names_for(&self, resident: &str) -> Vec<String> {
        self.skills
            .iter()
            .filter(|k| k.resident == resident)
            .map(|k| k.name.clone())
            .collect()
    }

    /// `teacher` 是否有一個 `student` 還不會的技能——可教（ROADMAP 717）。
    /// 依技能庫既有順序找第一筆符合的（決定性、非隨機）；教哪一筆、教誰由此決定，
    /// 呼叫端只負責機率門檻與台詞。
    pub fn teachable(&self, teacher: &str, student: &str) -> Option<&InventedSkillRecord> {
        self.skills
            .iter()
            .filter(|k| k.resident == teacher)
            .find(|k| self.find_for(student, k.goal_block).is_none())
    }
}

// ── jsonl 持久化（append-only，比照 voxel_goals/voxel_memory 慣例）────────────────

/// 發明技能落地路徑（`data/` 已 gitignore）。
const INVENTED_SKILLS_PATH: &str = "data/voxel_invented_skills.jsonl";

/// Append 一筆技能。**鐵律**：只在不持任何鎖時呼叫（同步小檔寫，不 await）。
pub fn append_invented_skill(rec: &InventedSkillRecord) {
    if let Ok(line) = serde_json::to_string(rec) {
        write_line(INVENTED_SKILLS_PATH, &line);
    }
}

/// 載回所有技能（啟動時呼叫一次）。檔不存在／壞行皆容忍。
pub fn load_invented_skills() -> Vec<InventedSkillRecord> {
    let content = match std::fs::read_to_string(INVENTED_SKILLS_PATH) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    content
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.is_empty() {
                None
            } else {
                serde_json::from_str::<InventedSkillRecord>(l).ok()
            }
        })
        .collect()
}

fn write_line(path: &str, line: &str) {
    use std::io::Write;
    if let Some(parent) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::OpenOptions::new().create(true).append(true).open(path) {
        Ok(mut f) => {
            let _ = writeln!(f, "{line}");
        }
        Err(e) => tracing::warn!("無法寫入發明技能記錄 {path}: {e}"),
    }
}

// ── 發明 prompt（grounded：原語白名單 + 真配方表節錄；要求嚴格 JSON 輸出）──────────

/// 一條配方的事實行（prompt 節錄用）：「- 配方 id「x」：配料 → 產物×n」。
fn recipe_fact(r: &vcraft::Recipe) -> String {
    let inputs: Vec<String> = r
        .inputs
        .iter()
        .map(|(bid, n)| format!("{}×{}", input_name(*bid), n))
        .collect();
    format!(
        "- 配方 id「{}」：{} → {}×{}",
        r.id,
        inputs.join(" + "),
        r.name_zh,
        r.output_count
    )
}

/// 組發明用的 (system, user) prompt。事實全部 grounded：
/// 可採資源清單來自 GatherResource、配方節錄來自 voxel_craft 真表（2×2 隨身＋3×3 工作台
/// 兩套都列，工作台規則講清楚）——她想出來的路一定踩在真實世界規則上，執行器才驗證得了。
/// `wb_nearby`：她附近是否已有放置好的工作台（world 事實，由呼叫端查好傳入）。
pub fn invention_prompt(
    resident_name: &str,
    goal: &MaterialGoal,
    desire: &str,
    bag_note: &str,
    wb_nearby: bool,
) -> (String, String) {
    // 配方節錄：只列可發明配方（配料她弄得到的），每條含 id 與配料事實。
    let recipe_lines: Vec<String> = inventable_recipes().map(recipe_fact).collect();
    let wb_recipe_lines: Vec<String> = inventable_wb_recipes().map(recipe_fact).collect();
    let system = format!(
        "你是{resident_name}，乙太方界的居民。你要自己想辦法解決一個處境：把你會的基礎動作\
        組合成一個新技能。你只會這幾種基礎動作（原語）：\n\
        1. 採集：{{\"op\":\"gather\",\"resource\":\"<資源>\",\"count\":<數量1~{max_c}>}}，\
        resource 只能是 grass / sand / dirt / stone / wood。\n\
        2. 隨身合成（2×2）：{{\"op\":\"craft\",\"recipe\":\"<配方id>\"}}。\n\
        3. 工作台合成（3×3）：{{\"op\":\"craft_wb\",\"recipe\":\"<配方id>\"}}——\
        **必須先有工作台放在你旁邊**才能執行。\n\
        4. 放置：{{\"op\":\"place\",\"block\":\"workbench\"}}——把背包裡的工作台放到腳邊\
        （會消耗背包裡那一個）。\n\
        你知道的隨身合成配方（事實，不可捏造別的）：\n{recipes}\n\
        你知道的工作台配方（要先有工作台在旁邊，才能用 craft_wb 做這些）：\n{wb_recipes}\n\
        注意：\n\
        - 合成會**消耗**配料——採集步驟的數量必須足以支付後續所有合成所需的配料\
        （例如要合成需要木頭×2的配方，前面就得先採集至少 2 個木頭）。\n\
        - 工作台本身可用配方 id「workbench」（木板×4）隨身合成，再用 place 放置到腳邊；\
        若你附近已經有工作台，就不必再做一個、直接 craft_wb。\n\
        請只輸出一個 JSON 物件（不要任何其他文字或說明）：\n\
        {{\"name\":\"<你給這個技能取的名字，繁體中文，最多{max_n}字>\",\"steps\":[<原語序列，最多{max_s}步>]}}",
        max_c = MAX_GATHER_COUNT,
        max_n = SKILL_NAME_MAX_CHARS,
        max_s = MAX_STEPS,
        recipes = recipe_lines.join("\n"),
        wb_recipes = wb_recipe_lines.join("\n"),
    );
    let user = format!(
        "處境：你心裡想著「{desire}」，想要「{goal}」這種材料，但你的背包裡沒有。\
        你的背包現況：{bag}。你附近{wb}。\
        請用你的原語組合出一個能讓背包裡出現「{goal}」的步驟計畫。",
        goal = goal.name_zh,
        bag = if bag_note.is_empty() { "空的" } else { bag_note },
        wb = if wb_nearby { "已經有一座放置好的工作台" } else { "沒有工作台" },
    );
    (system, user)
}

/// 配料 id → 繁中名（prompt 事實行用；覆蓋可發明配方會用到的原料與鏈上加工品）。
fn input_name(bid: u8) -> &'static str {
    match bid {
        2 => "泥土",
        3 => "石頭",
        4 => "沙子",
        5 => "木頭",
        6 => "葉片",
        8 => "木板",
        _ => "材料",
    }
}

// ── 面向玩家的台詞／Feed／記憶文字（i18n：集中在此、可替換）────────────────────────

/// 步驟序列的人話摘要（Feed 用）：「採集細沙×2→合成玻璃」。
pub fn steps_summary(steps: &[CheckedStep]) -> String {
    steps
        .iter()
        .map(|s| match s {
            CheckedStep::Gather { resource, count } => {
                format!("採集{}×{}", resource.display_name(), count)
            }
            CheckedStep::Craft { recipe_id } => {
                let name = vcraft::find_recipe(recipe_id).map(|r| r.name_zh).unwrap_or("？");
                format!("合成{name}")
            }
            CheckedStep::CraftWb { recipe_id } => {
                let name = vcraft::find_workbench_recipe(recipe_id)
                    .map(|r| r.name_zh)
                    .unwrap_or("？");
                format!("在工作台合成{name}")
            }
            CheckedStep::Place { block_id } => format!("放置{}", material_name(*block_id)),
        })
        .collect::<Vec<_>>()
        .join("→")
}

/// 放好站點方塊的冒泡（放置原語完成那一刻——玩家看得到「她把工作台擺出來了」）。
pub fn placed_line(block_name: &str) -> String {
    format!("我把{block_name}放好了！")
}

/// 學會技能的冒泡（發明成功那一刻——維護者要看得到「進化」）。
pub fn learned_line(skill_name: &str) -> String {
    format!("我學會「{skill_name}」了！")
}

/// 學會技能的 Feed 詳情。
pub fn learned_feed(skill_name: &str, goal_name: &str, steps: &[CheckedStep]) -> String {
    format!(
        "自己想出了辦法、發明了「{skill_name}」（{}），做出{goal_name}了！",
        steps_summary(steps)
    )
}

/// 學會技能寫進記憶（日記走既有事件管道，會自然反映）。
pub fn learned_memory(skill_name: &str, goal_name: &str) -> String {
    format!("我自己發明了「{skill_name}」這個技能，靠它做出了{goal_name}——我學會的、誰也拿不走")
}

/// 重用既有技能的開工冒泡（零 LLM——「這我會！」）。
pub fn reuse_line(skill_name: &str) -> String {
    format!("這我會！用我的「{skill_name}」～")
}

/// 重用技能完成的 Feed 詳情。
pub fn reuse_feed(skill_name: &str, goal_name: &str) -> String {
    format!("用自己發明的「{skill_name}」，又做出{goal_name}了（熟練，一次到位）")
}

/// 發明失敗的教訓（進記憶，不存技能）。
pub fn fail_lesson(goal_name: &str) -> String {
    format!("我試著自己想辦法做出{goal_name}，這次沒成功——下次再想想別的路子")
}

/// 對話 system prompt 的「我會的技能」注入段（玩家問她會什麼時講得出來）。
pub fn skills_talk_note(names: &[String]) -> Option<String> {
    if names.is_empty() {
        return None;
    }
    let list = names
        .iter()
        .map(|n| format!("「{n}」"))
        .collect::<Vec<_>>()
        .join("、");
    Some(format!(
        "你還有自己發明的技能：{list}——這是你自己從基礎動作組合出來、親手驗證過的本事，\
        旅人問起你會什麼時，可以自豪地提到它。"
    ))
}

/// 測試注入口（**僅供隔離實測**）：設 `BUTFUN_INVENT_FIXED_PLAN` 時，發明流程改用
/// 這串固定 JSON 當作「LLM 的輸出」——當測試環境打不到思考腦時，用來驗證
/// 「執行→驗證→存→重用」的確定性鏈。prod 不設此變數，永遠走真便宜腦。
pub fn fixed_plan_env() -> Option<String> {
    std::env::var("BUTFUN_INVENT_FIXED_PLAN").ok().filter(|s| !s.trim().is_empty())
}

// ── 單元測試 ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn glass_plan_json() -> &'static str {
        r#"{"name":"燒玻璃","steps":[{"op":"gather","resource":"sand","count":2},{"op":"craft","recipe":"glass"}]}"#
    }

    /// 第二刀全鏈計畫（熔爐）：採木×2→合木板→合工作台→放置→採石×8→3×3 合熔爐。
    fn furnace_chain_json() -> &'static str {
        r#"{"name":"造爐之路","steps":[
            {"op":"gather","resource":"wood","count":2},
            {"op":"craft","recipe":"plank"},
            {"op":"craft","recipe":"workbench"},
            {"op":"place","block":"workbench"},
            {"op":"gather","resource":"stone","count":8},
            {"op":"craft_wb","recipe":"furnace_wb"}]}"#
    }

    /// 永遠回「附近沒有站點」的查詢（多數測試用；有站點的情境另建閉包）。
    fn no_station(_bid: u8) -> bool {
        false
    }

    // ── parse_plan：白名單／壞輸出拒絕 ────────────────────────────────────────

    #[test]
    fn parse_accepts_valid_plan() {
        let p = parse_plan(glass_plan_json()).expect("合法計畫應解析成功");
        assert_eq!(p.name, "燒玻璃");
        assert_eq!(p.steps.len(), 2);
        assert_eq!(
            p.steps[0],
            CheckedStep::Gather { resource: GatherResource::Sand, count: 2 }
        );
        assert_eq!(p.steps[1], CheckedStep::Craft { recipe_id: "glass" });
    }

    #[test]
    fn parse_accepts_json_wrapped_in_prose() {
        // 小模型常在 JSON 前後加話——抽出第一個 { 到最後一個 } 應仍可解析。
        let raw = format!("好的，我的計畫是：\n{}\n希望能成功！", glass_plan_json());
        assert!(parse_plan(&raw).is_some());
    }

    #[test]
    fn parse_accepts_chinese_resource_token() {
        let raw = r#"{"name":"備木料","steps":[{"op":"gather","resource":"木頭","count":2},{"op":"craft","recipe":"plank"}]}"#;
        let p = parse_plan(raw).expect("繁中資源名也應通過白名單");
        assert_eq!(
            p.steps[0],
            CheckedStep::Gather { resource: GatherResource::Wood, count: 2 }
        );
    }

    #[test]
    fn parse_rejects_unknown_op() {
        // 白名單外的原語（例如捏造 "teleport"）→ 整個計畫拒絕。
        let raw = r#"{"name":"瞬移","steps":[{"op":"teleport","resource":"sand","count":1}]}"#;
        assert!(parse_plan(raw).is_none());
    }

    #[test]
    fn parse_rejects_unknown_resource_and_recipe() {
        // 不存在的資源。
        let raw = r#"{"name":"挖鑽石","steps":[{"op":"gather","resource":"diamond","count":1}]}"#;
        assert!(parse_plan(raw).is_none());
        // 不存在的配方。
        let raw = r#"{"name":"亂合","steps":[{"op":"craft","recipe":"no_such_recipe"}]}"#;
        assert!(parse_plan(raw).is_none());
    }

    #[test]
    fn parse_rejects_non_inventable_recipe() {
        // torch 配料要煤礦（居民採不到）→ 不可發明，拒絕。
        let raw = r#"{"name":"做火把","steps":[{"op":"craft","recipe":"torch"}]}"#;
        assert!(parse_plan(raw).is_none());
        // 3×3 配方用錯原語（craft 只認 2×2 表；glass_wb 要用 craft_wb）→ 拒絕。
        let raw = r#"{"name":"大量玻璃","steps":[{"op":"craft","recipe":"glass_wb"}]}"#;
        assert!(parse_plan(raw).is_none());
        // craft_wb 也只認 3×3 表（2×2 的 glass 不在裡面）→ 拒絕。
        let raw = r#"{"name":"玻璃","steps":[{"op":"craft_wb","recipe":"glass"}]}"#;
        assert!(parse_plan(raw).is_none());
        // 鐵鎬要鐵錠（要冶煉，鏈外）→ craft_wb 也不可發明，拒絕。
        let raw = r#"{"name":"鐵鎬","steps":[{"op":"craft_wb","recipe":"iron_pickaxe"}]}"#;
        assert!(parse_plan(raw).is_none());
    }

    #[test]
    fn parse_accepts_workbench_chain_plan() {
        // 第二刀全鏈計畫：採木→合板→合工作台→放置→採石→3×3 合熔爐——白名單全過。
        let p = parse_plan(furnace_chain_json()).expect("工作台鏈計畫應解析成功");
        assert_eq!(p.steps.len(), 6);
        assert_eq!(p.steps[3], CheckedStep::Place { block_id: WORKBENCH_BLOCK_ID });
        assert_eq!(p.steps[5], CheckedStep::CraftWb { recipe_id: "furnace_wb" });
    }

    #[test]
    fn parse_place_accepts_only_station_whitelist() {
        // 放玻璃（裝飾性亂放不在白名單）→ 拒絕。
        let raw = r#"{"name":"擺玻璃","steps":[{"op":"place","block":"glass"}]}"#;
        assert!(parse_plan(raw).is_none());
        // 繁中 token 也通（小模型兩種都可能給）。
        let raw = r#"{"name":"擺台","steps":[{"op":"place","block":"工作台"}]}"#;
        let p = parse_plan(raw).unwrap();
        assert_eq!(p.steps[0], CheckedStep::Place { block_id: WORKBENCH_BLOCK_ID });
        // 熔爐也在站點白名單。
        assert_eq!(place_block_from_token("furnace"), Some(FURNACE_BLOCK_ID));
        // token 互逆。
        assert_eq!(place_block_from_token(place_token_of(WORKBENCH_BLOCK_ID)), Some(WORKBENCH_BLOCK_ID));
    }

    #[test]
    fn parse_rejects_bad_counts_and_lengths() {
        // count = 0。
        let raw = r#"{"name":"a","steps":[{"op":"gather","resource":"sand","count":0}]}"#;
        assert!(parse_plan(raw).is_none());
        // count 超上限。
        let raw = r#"{"name":"a","steps":[{"op":"gather","resource":"sand","count":999}]}"#;
        assert!(parse_plan(raw).is_none());
        // 空步驟。
        let raw = r#"{"name":"a","steps":[]}"#;
        assert!(parse_plan(raw).is_none());
        // 步數超上限（MAX_STEPS+1 步）。
        let step = r#"{"op":"gather","resource":"sand","count":1}"#;
        let steps = vec![step; MAX_STEPS + 1].join(",");
        let raw = format!(r#"{{"name":"a","steps":[{steps}]}}"#);
        assert!(parse_plan(&raw).is_none());
        // 空名字。
        let raw = r#"{"name":"  ","steps":[{"op":"gather","resource":"sand","count":1}]}"#;
        assert!(parse_plan(raw).is_none());
    }

    #[test]
    fn parse_rejects_garbage() {
        assert!(parse_plan("").is_none());
        assert!(parse_plan("我不知道怎麼辦").is_none());
        assert!(parse_plan("{not json at all]").is_none());
    }

    #[test]
    fn parse_truncates_long_name() {
        let raw = format!(
            r#"{{"name":"{}","steps":[{{"op":"gather","resource":"sand","count":1}}]}}"#,
            "超長名字".repeat(20)
        );
        let p = parse_plan(&raw).expect("超長名字截斷後仍應接受");
        assert!(p.name.chars().count() <= SKILL_NAME_MAX_CHARS);
    }

    // ── 處境偵測 ─────────────────────────────────────────────────────────────

    #[test]
    fn detect_missing_material_hits_craftables() {
        assert_eq!(
            detect_missing_material("好想要一扇玻璃窗").map(|g| g.block_id),
            Some(10)
        );
        assert_eq!(
            detect_missing_material("想用木板鋪個地板").map(|g| g.block_id),
            Some(8)
        );
        assert_eq!(detect_missing_material("想要石磚砌的爐子").map(|g| g.block_id), Some(9));
    }

    #[test]
    fn detect_missing_material_ignores_other_desires() {
        assert!(detect_missing_material("想要一座瞭望塔").is_none());
        assert!(detect_missing_material("想跟大家一起看星星").is_none());
        assert!(detect_missing_material("").is_none());
    }

    // ── craft_apply ──────────────────────────────────────────────────────────

    #[test]
    fn craft_apply_consumes_and_produces() {
        let glass = vcraft::find_recipe("glass").unwrap(); // 2 沙 → 1 玻璃
        let mut bag = HashMap::from([(4u8, 3u32)]);
        assert!(craft_apply(&mut bag, glass));
        assert_eq!(bag.get(&4).copied(), Some(1)); // 3 - 2 = 1 沙
        assert_eq!(bag.get(&10).copied(), Some(1)); // + 1 玻璃
    }

    #[test]
    fn craft_apply_rejects_insufficient_without_mutation() {
        let glass = vcraft::find_recipe("glass").unwrap();
        let mut bag = HashMap::from([(4u8, 1u32)]); // 只有 1 沙，不夠
        let before = bag.clone();
        assert!(!craft_apply(&mut bag, glass));
        assert_eq!(bag, before, "不夠料時背包應原封不動");
    }

    // ── next_action：後置條件語意 ─────────────────────────────────────────────

    fn glass_run() -> InventRun {
        let plan = parse_plan(glass_plan_json()).unwrap();
        InventRun::from_plan(10, "玻璃", &plan, false)
    }

    #[test]
    fn next_action_starts_gather_when_bag_lacks() {
        let run = glass_run();
        let bag = HashMap::new();
        assert_eq!(
            next_action(&run, &bag, no_station),
            StepAction::StartGather { resource: GatherResource::Sand }
        );
    }

    #[test]
    fn next_action_skips_gather_when_bag_already_has() {
        // 背包已有 2 沙 → 採集步的後置條件已滿足 → Advance（自癒/可重入）。
        let run = glass_run();
        let bag = HashMap::from([(4u8, 2u32)]);
        assert_eq!(next_action(&run, &bag, no_station), StepAction::Advance);
    }

    #[test]
    fn next_action_craft_then_done() {
        let mut run = glass_run();
        run.step_idx = 1;
        let bag = HashMap::from([(4u8, 2u32)]);
        assert_eq!(next_action(&run, &bag, no_station), StepAction::DoCraft { recipe_id: "glass" });
        run.step_idx = 2;
        assert_eq!(next_action(&run, &bag, no_station), StepAction::Done);
    }

    /// 全鏈模擬（純邏輯側的「發明→執行→驗證」證據）：
    /// 空背包 → 去採沙 →（模擬採到 2 沙）→ 合成玻璃 → Done → 後置條件成立。
    #[test]
    fn full_chain_simulated_execution_reaches_goal() {
        let mut run = glass_run();
        let mut bag: HashMap<u8, u32> = HashMap::new();
        let mut guard = 0;
        loop {
            guard += 1;
            assert!(guard < 20, "執行應在有限步內收斂");
            match next_action(&run, &bag, no_station) {
                StepAction::StartGather { resource } => {
                    // 模擬走過去挖到一塊（production 由 GatherSkill + 可逃性判定執行）。
                    *bag.entry(resource.block_id()).or_insert(0) += 1;
                }
                StepAction::Advance => run.step_idx += 1,
                StepAction::DoCraft { recipe_id } => {
                    let r = vcraft::find_recipe(recipe_id).unwrap();
                    assert!(craft_apply(&mut bag, r), "照計畫備好料，合成應成功");
                    run.step_idx += 1;
                }
                StepAction::Done => break,
                other => panic!("玻璃計畫不該出現放置/工作台步：{other:?}"),
            }
        }
        assert!(goal_met(&bag, 10), "後置條件：背包真的有玻璃");
    }

    #[test]
    fn goal_not_met_on_empty_bag() {
        assert!(!goal_met(&HashMap::new(), 10));
    }

    // ── simulate_plan：提案階段抓出算術不通的計畫（實測遇過的真 bug 場景）──────────

    #[test]
    fn simulate_rejects_undercounted_gather() {
        // 實測真場景：便宜腦提「採木頭×1→合成木板」，但木板配方要木頭×2 → 模擬應擋下。
        let raw = r#"{"name":"溫暖木地板","steps":[{"op":"gather","resource":"wood","count":1},{"op":"craft","recipe":"plank"}]}"#;
        let p = parse_plan(raw).unwrap();
        let err = simulate_plan(&p.steps, &HashMap::new(), 8, false).unwrap_err();
        assert!(err.contains("木板"), "錯誤原因應點名不夠料的配方：{err}");
    }

    #[test]
    fn simulate_accepts_correct_plan() {
        let raw = r#"{"name":"備木成板","steps":[{"op":"gather","resource":"wood","count":2},{"op":"craft","recipe":"plank"}]}"#;
        let p = parse_plan(raw).unwrap();
        assert!(simulate_plan(&p.steps, &HashMap::new(), 8, false).is_ok());
    }

    #[test]
    fn simulate_rejects_plan_missing_goal() {
        // 計畫做得出玻璃、但目標是木板 → 跑完背包沒有目標材料，擋下。
        let p = parse_plan(glass_plan_json()).unwrap();
        let err = simulate_plan(&p.steps, &HashMap::new(), 8, false).unwrap_err();
        assert!(err.contains("木板"));
    }

    #[test]
    fn simulate_uses_existing_bag_stock() {
        // 背包已有 2 木 → 「直接合成木板」的計畫也可行（後置條件語意、少採不浪費）。
        let raw = r#"{"name":"就地取材","steps":[{"op":"craft","recipe":"plank"}]}"#;
        let p = parse_plan(raw).unwrap();
        let bag = HashMap::from([(5u8, 2u32)]);
        assert!(simulate_plan(&p.steps, &bag, 8, false).is_ok());
        // 空背包則不可行。
        assert!(simulate_plan(&p.steps, &HashMap::new(), 8, false).is_err());
    }

    #[test]
    fn retry_prompt_carries_feedback() {
        let s = retry_user_prompt("處境……", r#"{"name":"x"}"#, "不夠料");
        assert!(s.contains("不夠料") && s.contains("行不通") && s.contains("JSON"));
    }

    // ── canonicalize_steps：存檔技能必須自足（空背包也能執行）───────────────────

    #[test]
    fn canonicalize_makes_craft_only_plan_self_contained() {
        // 實測真場景：她背包剛好有木頭 → 腦提「只合成」計畫 → 當下可行，但存檔版
        // 必須從空背包也可行（技能是帶著走的本事）。正規化應補上採集木頭×2。
        let raw = r#"{"name":"溫暖木板","steps":[{"op":"craft","recipe":"plank"}]}"#;
        let p = parse_plan(raw).unwrap();
        // 原計畫從空背包不可行。
        assert!(simulate_plan(&p.steps, &HashMap::new(), 8, false).is_err());
        // 正規化後：補了採集步 → 空背包可行。
        let canon = canonicalize_steps(&p.steps);
        let checked = check_stored_steps(&canon).expect("正規化版應過存檔白名單");
        assert!(simulate_plan(&checked, &HashMap::new(), 8, false).is_ok(), "正規化技能應自足");
        // 第一步應是「採集木頭×2」（plank 配方的配料）。
        assert_eq!(
            checked[0],
            CheckedStep::Gather { resource: GatherResource::Wood, count: 2 }
        );
    }

    #[test]
    fn canonicalize_keeps_existing_gathers_and_stays_valid() {
        // 已含採集步的完整計畫：正規化只是在合成前補「確保配料」步（有料時零成本 no-op），
        // 從空背包依然可行、語意不變。
        let p = parse_plan(glass_plan_json()).unwrap();
        let canon = canonicalize_steps(&p.steps);
        let checked = check_stored_steps(&canon).unwrap();
        assert!(simulate_plan(&checked, &HashMap::new(), 10, false).is_ok());
        // 原本的採集步仍在最前面。
        assert_eq!(
            checked[0],
            CheckedStep::Gather { resource: GatherResource::Sand, count: 2 }
        );
    }

    #[test]
    fn stored_steps_cap_is_wider_than_llm_cap() {
        // 存檔上限應寬於 LLM 提案上限（正規化會展開步驟），且仍有界。
        assert!(MAX_STORED_STEPS > MAX_STEPS);
        let step = PrimStep::Gather { resource: "sand".into(), count: 1 };
        let long: Vec<PrimStep> = vec![step.clone(); MAX_STEPS + 1];
        assert!(check_steps(&long).is_none(), "LLM 提案超步數應拒絕");
        assert!(check_stored_steps(&long).is_some(), "存檔版在較寬上限內應接受");
        let too_long: Vec<PrimStep> = vec![step; MAX_STORED_STEPS + 1];
        assert!(check_stored_steps(&too_long).is_none(), "存檔版仍有上限");
    }

    // ── InventRun 逾時 ────────────────────────────────────────────────────────

    #[test]
    fn run_expires_when_deadline_elapsed() {
        let mut run = glass_run();
        assert!(!run.is_expired());
        run.deadline = 0.0;
        assert!(run.is_expired());
    }

    // ── 技能庫：個體性 / 去重 / 還原 ─────────────────────────────────────────

    #[test]
    fn store_skills_are_personal() {
        let mut s = InventedSkillStore::new();
        let plan = parse_plan(glass_plan_json()).unwrap();
        let rec = s.add("vox_res_0", &plan.name, 10, plan.raw_steps.clone());
        assert!(rec.is_some(), "首次存技能應成功");
        // 露娜會了 → 查得到；諾娃沒學過 → 查不到（技能是個體的）。
        assert!(s.find_for("vox_res_0", 10).is_some());
        assert!(s.find_for("vox_res_1", 10).is_none());
        // 同處境不重複存。
        assert!(s.add("vox_res_0", "另一個名", 10, plan.raw_steps).is_none());
        // 技能名列表（對話注入用）。
        assert_eq!(s.names_for("vox_res_0"), vec!["燒玻璃".to_string()]);
        assert!(s.names_for("vox_res_1").is_empty());
    }

    #[test]
    fn store_roundtrip_from_entries() {
        let mut s = InventedSkillStore::new();
        let plan = parse_plan(glass_plan_json()).unwrap();
        let rec = s.add("vox_res_0", "燒玻璃", 10, plan.raw_steps).unwrap();
        // 模擬 jsonl 落地→載回（serde roundtrip）。
        let line = serde_json::to_string(&rec).unwrap();
        let back: InventedSkillRecord = serde_json::from_str(&line).unwrap();
        let restored = InventedSkillStore::from_entries(vec![back]);
        let k = restored.find_for("vox_res_0", 10).expect("重啟後她仍然會");
        assert_eq!(k.name, "燒玻璃");
        // 載回的步驟仍過白名單 → 可直接重用執行（零 LLM）。
        assert!(check_steps(&k.steps).is_some());
    }

    // ── teachable：可教技能查詢（ROADMAP 717 用）────────────────────────────────

    #[test]
    fn teachable_finds_skill_teacher_has_and_student_lacks() {
        let mut s = InventedSkillStore::new();
        let plan = parse_plan(glass_plan_json()).unwrap();
        s.add("vox_res_0", &plan.name, 10, plan.raw_steps).unwrap();
        let k = s.teachable("vox_res_0", "vox_res_1").expect("露娜會、諾娃不會 → 可教");
        assert_eq!(k.name, "燒玻璃");
    }

    #[test]
    fn teachable_none_when_student_already_knows() {
        let mut s = InventedSkillStore::new();
        let plan = parse_plan(glass_plan_json()).unwrap();
        s.add("vox_res_0", &plan.name, 10, plan.raw_steps.clone()).unwrap();
        s.add("vox_res_1", "自己的燒玻璃法", 10, plan.raw_steps).unwrap();
        assert!(s.teachable("vox_res_0", "vox_res_1").is_none(), "兩人都會就沒什麼好教的");
    }

    #[test]
    fn teachable_none_when_teacher_knows_nothing() {
        let s = InventedSkillStore::new();
        assert!(s.teachable("vox_res_0", "vox_res_1").is_none());
    }

    #[test]
    fn store_keeps_latest_for_same_situation() {
        let plan = parse_plan(glass_plan_json()).unwrap();
        let old = InventedSkillRecord {
            resident: "vox_res_0".into(),
            name: "舊玻璃法".into(),
            goal_block: 10,
            steps: plan.raw_steps.clone(),
            seq: 0,
        };
        let new = InventedSkillRecord {
            resident: "vox_res_0".into(),
            name: "新玻璃法".into(),
            goal_block: 10,
            steps: plan.raw_steps,
            seq: 5,
        };
        let s = InventedSkillStore::from_entries(vec![old, new]);
        assert_eq!(s.find_for("vox_res_0", 10).unwrap().name, "新玻璃法");
    }

    #[test]
    fn check_steps_rejects_corrupt_record() {
        // 存檔被手改壞（未知配方）→ 載回驗證失敗 → 不執行（不 panic、不亂跑）。
        let bad = vec![PrimStep::Craft { recipe: "hacked".into() }];
        assert!(check_steps(&bad).is_none());
    }

    // ── 可發明配方集合 ────────────────────────────────────────────────────────

    #[test]
    fn inventable_recipes_follow_obtainable_closure() {
        let ids: Vec<&str> = inventable_recipes().map(|r| r.id).collect();
        // 玻璃/木板/石磚/農田土：配料全是可自採原料 → 可發明。
        // 第二刀閉包：工作台（4 木板，木板在鏈上）、木門/梯子（木板）也可發明了。
        for want in ["glass", "plank", "stone_brick", "till", "workbench", "door", "ladder"] {
            assert!(ids.contains(&want), "{want} 應可發明");
        }
        // 火把要煤礦、床要葉片、麵包要小麥 → 她弄不到料，仍不可發明（誠實邊界）。
        for no in ["torch", "bed", "bread"] {
            assert!(!ids.contains(&no), "{no} 不應可發明");
        }
        // 3×3 工作台配方：熔爐（8 石）/箱子（8 木板）/大量玻璃…在鏈上 → 可發明；
        // 鐵系（要冶煉出鐵錠）不在鏈上 → 不可發明（冶煉留給下一刀）。
        let wb_ids: Vec<&str> = inventable_wb_recipes().map(|r| r.id).collect();
        for want in ["furnace_wb", "chest", "glass_wb", "plank_wb", "stone_wood_mix", "farm_kit"] {
            assert!(wb_ids.contains(&want), "{want} 應可發明（工作台鏈）");
        }
        for no in ["iron_block", "iron_pickaxe", "iron_axe", "iron_shovel"] {
            assert!(!wb_ids.contains(&no), "{no} 不應可發明（要冶煉）");
        }
        // 閉包集合本身：熔爐/箱子可取得；鐵錠/拋光石（熔爐冶煉產物）不可。
        assert!(obtainable_ids().contains(&FURNACE_BLOCK_ID));
        assert!(obtainable_ids().contains(&42u8), "箱子在鏈上");
        assert!(!obtainable_ids().contains(&22u8), "鐵錠要冶煉，不在鏈上");
        assert!(!obtainable_ids().contains(&17u8), "拋光石要冶煉，不在鏈上");
    }

    // ── 第二刀：工作台鏈的可行性模擬（依賴順序：先有工作台才能 3×3）────────────────

    #[test]
    fn simulate_rejects_craft_wb_without_workbench() {
        // 沒放工作台就 3×3 → 擋下，錯誤原因要教它工作台怎麼來（回饋給便宜腦修正）。
        let raw = r#"{"name":"直接開爐","steps":[
            {"op":"gather","resource":"stone","count":8},
            {"op":"craft_wb","recipe":"furnace_wb"}]}"#;
        let p = parse_plan(raw).unwrap();
        let err = simulate_plan(&p.steps, &HashMap::new(), FURNACE_BLOCK_ID, false).unwrap_err();
        assert!(err.contains("工作台"), "錯誤原因應點名缺工作台：{err}");
        assert!(err.contains("workbench"), "錯誤原因應附上工作台配方 id：{err}");
        // 同一計畫、但她附近本來就有工作台 → 可行（不必重做一個）。
        assert!(simulate_plan(&p.steps, &HashMap::new(), FURNACE_BLOCK_ID, true).is_ok());
    }

    #[test]
    fn simulate_accepts_full_workbench_chain() {
        // 全鏈：採木→合板→合工作台→放置→採石→3×3 合熔爐，空背包、附近沒工作台也可行。
        let p = parse_plan(furnace_chain_json()).unwrap();
        assert!(simulate_plan(&p.steps, &HashMap::new(), FURNACE_BLOCK_ID, false).is_ok());
    }

    #[test]
    fn simulate_rejects_place_without_item() {
        // 沒先合成工作台就放置 → 擋下（背包裡沒有它）。
        let raw = r#"{"name":"憑空放台","steps":[
            {"op":"place","block":"workbench"},
            {"op":"gather","resource":"stone","count":8},
            {"op":"craft_wb","recipe":"furnace_wb"}]}"#;
        let p = parse_plan(raw).unwrap();
        let err = simulate_plan(&p.steps, &HashMap::new(), FURNACE_BLOCK_ID, false).unwrap_err();
        assert!(err.contains("放置") && err.contains("工作台"), "{err}");
    }

    #[test]
    fn simulate_rejects_wrong_order_place_after_craft_wb() {
        // 順序排錯：先 3×3 再放工作台 → 依賴順序驗證應擋下。
        let raw = r#"{"name":"順序錯了","steps":[
            {"op":"gather","resource":"wood","count":2},
            {"op":"craft","recipe":"plank"},
            {"op":"craft","recipe":"workbench"},
            {"op":"gather","resource":"stone","count":8},
            {"op":"craft_wb","recipe":"furnace_wb"},
            {"op":"place","block":"workbench"}]}"#;
        let p = parse_plan(raw).unwrap();
        assert!(simulate_plan(&p.steps, &HashMap::new(), FURNACE_BLOCK_ID, false).is_err());
    }

    #[test]
    fn simulate_place_skips_when_workbench_already_nearby() {
        // 附近已有工作台時，place 步是後置條件 no-op（不需要背包裡真的有一個）。
        let raw = r#"{"name":"就地用台","steps":[
            {"op":"place","block":"workbench"},
            {"op":"gather","resource":"sand","count":6},
            {"op":"craft_wb","recipe":"glass_wb"}]}"#;
        let p = parse_plan(raw).unwrap();
        assert!(simulate_plan(&p.steps, &HashMap::new(), 10, true).is_ok());
        // 附近沒有 → 背包也沒有 → 擋下。
        assert!(simulate_plan(&p.steps, &HashMap::new(), 10, false).is_err());
    }

    #[test]
    fn simulate_chest_chain_math() {
        // 箱子鏈算術：工作台 4 木板 + 箱子 8 木板 = 12 木板 = 3 次 plank 合成 = 6 木。
        let raw = r#"{"name":"打一口箱","steps":[
            {"op":"gather","resource":"wood","count":6},
            {"op":"craft","recipe":"plank"},
            {"op":"craft","recipe":"plank"},
            {"op":"craft","recipe":"plank"},
            {"op":"craft","recipe":"workbench"},
            {"op":"place","block":"workbench"},
            {"op":"craft_wb","recipe":"chest"}]}"#;
        let p = parse_plan(raw).unwrap();
        assert!(simulate_plan(&p.steps, &HashMap::new(), 42, false).is_ok());
        // 少採一輪木（4 木只夠 2 次合成 = 8 木板，工作台用掉 4、剩 4 < 8）→ 擋下。
        let short = raw.replace(r#""count":6"#, r#""count":4"#);
        let p2 = parse_plan(&short).unwrap();
        assert!(simulate_plan(&p2.steps, &HashMap::new(), 42, false).is_err());
    }

    // ── 第二刀：next_action 的放置/工作台步決策 ─────────────────────────────────

    #[test]
    fn next_action_place_and_craft_wb_flow() {
        let plan = parse_plan(furnace_chain_json()).unwrap();
        let mut run = InventRun::from_plan(FURNACE_BLOCK_ID, "熔爐", &plan, false);
        // 走到放置步（index 3）：附近沒工作台 → DoPlace；已有 → Advance（後置條件語意）。
        run.step_idx = 3;
        let bag = HashMap::from([(WORKBENCH_BLOCK_ID, 1u32)]);
        assert_eq!(
            next_action(&run, &bag, no_station),
            StepAction::DoPlace { block_id: WORKBENCH_BLOCK_ID }
        );
        assert_eq!(
            next_action(&run, &bag, |bid| bid == WORKBENCH_BLOCK_ID),
            StepAction::Advance,
            "站點已在附近 → 跳過放置（不重複放）"
        );
        // 走到 3×3 步（index 5）→ DoCraftWb（附近有無工作台由執行端再驗）。
        run.step_idx = 5;
        assert_eq!(
            next_action(&run, &bag, no_station),
            StepAction::DoCraftWb { recipe_id: "furnace_wb" }
        );
    }

    #[test]
    fn take_one_consumes_or_refuses() {
        let mut bag = HashMap::from([(WORKBENCH_BLOCK_ID, 1u32)]);
        assert!(take_one(&mut bag, WORKBENCH_BLOCK_ID));
        assert_eq!(bag.get(&WORKBENCH_BLOCK_ID).copied(), Some(0));
        assert!(!take_one(&mut bag, WORKBENCH_BLOCK_ID), "沒貨不能再扣");
        assert!(!take_one(&mut bag, 42), "從沒有過的東西也扣不了");
    }

    /// 第二刀全鏈模擬（純邏輯側證據）：空背包 → 採木合板合工作台 → 放置 → 採石 →
    /// 3×3 合熔爐 → Done → 後置條件成立（背包真的有熔爐）。放置後 station 查詢轉真，
    /// 重演執行端「她剛放的工作台馬上可用」的時序。
    #[test]
    fn full_workbench_chain_simulated_execution_reaches_goal() {
        let plan = parse_plan(furnace_chain_json()).unwrap();
        let mut run = InventRun::from_plan(FURNACE_BLOCK_ID, "熔爐", &plan, false);
        let mut bag: HashMap<u8, u32> = HashMap::new();
        let mut wb_placed = false;
        let mut guard = 0;
        loop {
            guard += 1;
            assert!(guard < 60, "執行應在有限步內收斂");
            let near = wb_placed;
            match next_action(&run, &bag, move |bid| near && bid == WORKBENCH_BLOCK_ID) {
                StepAction::StartGather { resource } => {
                    *bag.entry(resource.block_id()).or_insert(0) += 1; // 模擬挖到一塊
                }
                StepAction::Advance => run.step_idx += 1,
                StepAction::DoCraft { recipe_id } => {
                    let r = vcraft::find_recipe(recipe_id).unwrap();
                    assert!(craft_apply(&mut bag, r), "照計畫備好料，合成應成功");
                    run.step_idx += 1;
                }
                StepAction::DoPlace { block_id } => {
                    assert!(take_one(&mut bag, block_id), "放置前背包應有工作台");
                    wb_placed = true; // 模擬 set_block 寫進世界
                    run.step_idx += 1;
                }
                StepAction::DoCraftWb { recipe_id } => {
                    assert!(wb_placed, "3×3 前必須已放好工作台");
                    let r = vcraft::find_workbench_recipe(recipe_id).unwrap();
                    assert!(craft_apply(&mut bag, r), "照計畫備好料，工作台合成應成功");
                    run.step_idx += 1;
                }
                StepAction::Done => break,
            }
        }
        assert!(goal_met(&bag, FURNACE_BLOCK_ID), "後置條件：背包真的有熔爐");
    }

    // ── 第二刀：工作台鏈的正規化（存檔技能自足）────────────────────────────────

    #[test]
    fn canonicalize_full_chain_is_minimal_and_self_sufficient() {
        // 完整鏈計畫：模擬式正規化不應重複插備料步（前段已備的料不重備）。
        let p = parse_plan(furnace_chain_json()).unwrap();
        let canon = canonicalize_steps(&p.steps);
        assert_eq!(canon.len(), p.raw_steps.len(), "完整計畫正規化後應原樣（零冗餘）");
        let checked = check_stored_steps(&canon).unwrap();
        assert!(simulate_plan(&checked, &HashMap::new(), FURNACE_BLOCK_ID, false).is_ok());
    }

    #[test]
    fn canonicalize_inserts_workbench_group_for_bare_craft_wb() {
        // 她發明時附近剛好有工作台 → 腦可能只提「採石→3×3」；存檔版必須自足：
        // 正規化應自動補上「採木→合板→合工作台→放置」整組。
        let raw = r#"{"name":"開爐","steps":[
            {"op":"gather","resource":"stone","count":8},
            {"op":"craft_wb","recipe":"furnace_wb"}]}"#;
        let p = parse_plan(raw).unwrap();
        // 原計畫只有在「附近有工作台」時可行。
        assert!(simulate_plan(&p.steps, &HashMap::new(), FURNACE_BLOCK_ID, false).is_err());
        assert!(simulate_plan(&p.steps, &HashMap::new(), FURNACE_BLOCK_ID, true).is_ok());
        // 正規化後：空背包、附近沒工作台也可行（技能是帶著走的本事）。
        let canon = canonicalize_steps(&p.steps);
        let checked = check_stored_steps(&canon).expect("正規化版應過存檔白名單");
        assert!(
            simulate_plan(&checked, &HashMap::new(), FURNACE_BLOCK_ID, false).is_ok(),
            "正規化技能應自足（含工作台備妥組）"
        );
        // 應包含放置步與工作台合成步。
        assert!(canon.iter().any(|s| matches!(s, PrimStep::Place { block } if block == "workbench")));
        assert!(canon.iter().any(|s| matches!(s, PrimStep::Craft { recipe } if recipe == "workbench")));
    }

    #[test]
    fn canonicalize_stays_within_stored_cap() {
        // 最長鏈（箱子：12 木板）正規化後也不超過存檔步數上限。
        let raw = r#"{"name":"打箱子","steps":[{"op":"craft_wb","recipe":"chest"}]}"#;
        let p = parse_plan(raw).unwrap();
        let canon = canonicalize_steps(&p.steps);
        assert!(canon.len() <= MAX_STORED_STEPS, "正規化步數 {} 應 ≤ {}", canon.len(), MAX_STORED_STEPS);
        let checked = check_stored_steps(&canon).expect("應過存檔白名單");
        assert!(simulate_plan(&checked, &HashMap::new(), 42, false).is_ok(), "箱子鏈技能應自足");
    }

    // ── 第二刀：處境偵測新目標 ───────────────────────────────────────────────

    #[test]
    fn detect_missing_material_hits_workbench_chain_goals() {
        assert_eq!(
            detect_missing_material("好想要一座熔爐煉點東西").map(|g| g.block_id),
            Some(FURNACE_BLOCK_ID)
        );
        assert_eq!(detect_missing_material("想要個箱子收納").map(|g| g.block_id), Some(42));
        assert_eq!(material_name(FURNACE_BLOCK_ID), "熔爐");
        assert_eq!(material_name(42), "箱子");
        assert_eq!(material_name(WORKBENCH_BLOCK_ID), "工作台");
    }

    // ── 第二刀：放置點與站點查詢（世界純函式）───────────────────────────────────

    #[test]
    fn find_place_spot_picks_adjacent_air_not_own_column() {
        use crate::voxel_skills::column_top;
        let world: WorldDelta = WorldDelta::new();
        let (fx, fz) = (10, 10);
        let fy = column_top(&world, fx, fz).unwrap() + 1; // 站在地表上
        let (x, y, z) = find_place_spot(&world, fx, fy, fz).expect("平地應找得到放置點");
        // 安全：絕不放自己身體格（自己那一柱）。
        assert!(!(x == fx && z == fz), "不可放在自己站的柱");
        // 放在鄰柱地表頂上、目標格是空氣、伸手可及。
        assert_eq!(y, column_top(&world, x, z).unwrap() + 1);
        assert_eq!(voxel::effective_block_at(&world, x, y, z), Block::Air);
        assert!((y - fy).abs() <= PLACE_MAX_DY);
        assert!((x - fx).abs().max((z - fz).abs()) <= PLACE_RING_MAX, "在腳邊環內");
    }

    #[test]
    fn find_place_spot_none_when_all_neighbors_unreachable() {
        use crate::voxel_skills::column_top;
        // 把腳邊 1~2 格環的所有鄰柱都疊高 4 格 → 全部超出可及高度 → 誠實回 None。
        let mut world: WorldDelta = WorldDelta::new();
        let (fx, fz) = (30, 30);
        let fy = column_top(&world, fx, fz).unwrap() + 1;
        for dx in -PLACE_RING_MAX..=PLACE_RING_MAX {
            for dz in -PLACE_RING_MAX..=PLACE_RING_MAX {
                if dx == 0 && dz == 0 {
                    continue;
                }
                let (x, z) = (fx + dx, fz + dz);
                let top = column_top(&world, x, z).unwrap();
                for h in 1..=(PLACE_MAX_DY + 3) {
                    voxel::set_block(&mut world, x, top + h, z, Block::Stone);
                }
            }
        }
        assert!(find_place_spot(&world, fx, fy, fz).is_none(), "全被疊高 → 放不了就誠實失敗");
    }

    #[test]
    fn station_nearby_sees_placed_workbench_within_radius() {
        use crate::voxel_skills::column_top;
        let mut world: WorldDelta = WorldDelta::new();
        let (fx, fz) = (50, 50);
        let fy = column_top(&world, fx, fz).unwrap() + 1;
        assert!(!station_nearby(&world, fx, fy, fz, WORKBENCH_BLOCK_ID), "還沒放 → 查無");
        // 放在腳邊 2 格處（find_place_spot 的環內）→ 查得到。
        let ty = column_top(&world, fx + 2, fz).unwrap() + 1;
        voxel::set_block(&mut world, fx + 2, ty, fz, Block::Workbench);
        assert!(station_nearby(&world, fx, fy, fz, WORKBENCH_BLOCK_ID), "剛放的工作台應可用");
        // 型別要對：查熔爐仍是查無。
        assert!(!station_nearby(&world, fx, fy, fz, FURNACE_BLOCK_ID));
        // 放遠（半徑外）→ 查無。
        let far = STATION_NEAR_RADIUS + 3;
        let mut world2: WorldDelta = WorldDelta::new();
        let ty2 = column_top(&world2, fx + far, fz).unwrap() + 1;
        voxel::set_block(&mut world2, fx + far, ty2, fz, Block::Workbench);
        assert!(!station_nearby(&world2, fx, fy, fz, WORKBENCH_BLOCK_ID), "太遠不算在旁邊");
    }

    // ── prompt / 台詞 ─────────────────────────────────────────────────────────

    #[test]
    fn invention_prompt_is_grounded_and_strict() {
        let goal = MaterialGoal { block_id: 10, name_zh: "玻璃" };
        let (sys, user) = invention_prompt("露娜", &goal, "好想要一塊玻璃", "木頭×1", false);
        // 原語白名單與嚴格輸出格式都在 system。
        assert!(sys.contains("gather") && sys.contains("craft"));
        assert!(sys.contains("JSON"));
        // grounded 配方事實：玻璃那條一定在（2 沙 → 玻璃）。
        assert!(sys.contains("glass") && sys.contains("沙子"));
        // 處境與背包現況在 user。
        assert!(user.contains("玻璃") && user.contains("木頭×1"));
    }

    #[test]
    fn invention_prompt_teaches_workbench_chain() {
        let goal = MaterialGoal { block_id: FURNACE_BLOCK_ID, name_zh: "熔爐" };
        let (sys, user) = invention_prompt("露娜", &goal, "想要一座熔爐", "", false);
        // 新原語與工作台規則都在 system。
        assert!(sys.contains("craft_wb") && sys.contains("place"));
        assert!(sys.contains("workbench"), "要教她工作台配方 id");
        // grounded 工作台配方事實：熔爐那條一定在（8 石 → 熔爐）。
        assert!(sys.contains("furnace_wb") && sys.contains("石頭"));
        // 鐵系不在鏈上 → 不該出現在可發明配方節錄裡。
        assert!(!sys.contains("iron_pickaxe"), "鐵鎬要冶煉，不該列給她");
        // user 帶「附近沒有工作台」的世界事實。
        assert!(user.contains("沒有工作台"));
        let (_, user2) = invention_prompt("露娜", &goal, "想要一座熔爐", "", true);
        assert!(user2.contains("已經有一座放置好的工作台"));
    }

    #[test]
    fn lines_and_feeds_are_nonempty_and_mention_skill() {
        let plan = parse_plan(glass_plan_json()).unwrap();
        assert!(learned_line("燒玻璃").contains("燒玻璃"));
        assert!(learned_feed("燒玻璃", "玻璃", &plan.steps).contains("採集細沙×2"));
        assert!(reuse_line("燒玻璃").contains("燒玻璃"));
        assert!(reuse_feed("燒玻璃", "玻璃").contains("熟練"));
        assert!(fail_lesson("玻璃").contains("沒成功"));
        assert!(learned_memory("燒玻璃", "玻璃").contains("發明"));
        let note = skills_talk_note(&["燒玻璃".into()]).unwrap();
        assert!(note.contains("「燒玻璃」"));
        assert!(skills_talk_note(&[]).is_none());
    }

    #[test]
    fn steps_summary_reads_naturally() {
        let plan = parse_plan(glass_plan_json()).unwrap();
        assert_eq!(steps_summary(&plan.steps), "採集細沙×2→合成玻璃");
        // 第二刀新步驟的人話摘要：放置＋工作台合成。
        let chain = parse_plan(furnace_chain_json()).unwrap();
        let s = steps_summary(&chain.steps);
        assert!(s.contains("放置工作台"), "{s}");
        assert!(s.contains("在工作台合成熔爐"), "{s}");
        // 放置冒泡台詞。
        assert!(placed_line("工作台").contains("工作台"));
    }
}
