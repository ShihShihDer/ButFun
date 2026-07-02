//! 乙太方界·技能發明 v1（真進化第一刀）——居民自己把**基礎動作原語**組合成解法，
//! 成功後存成「自己的技能」，之後同處境**直接重用（零 LLM）**。
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

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::voxel_craft as vcraft;
use crate::voxel_skills::GatherResource;

// ── 參數（刻意保守：小而完整）─────────────────────────────────────────────────

/// 一個計畫最多幾步（防 LLM 給出超長序列拖垮執行）。
pub const MAX_STEPS: usize = 6;
/// 單一採集步驟的數量上限（防「採 999 個沙」這種不合理計畫）。
pub const MAX_GATHER_COUNT: u32 = 8;
/// 技能名長度上限（進 Feed／泡泡，別讓 LLM 取出一篇小作文）。
pub const SKILL_NAME_MAX_CHARS: usize = 12;
/// 一次計畫執行的總逾時（秒）：走路採集是真實時間，給寬鬆一點；逾時放棄（記教訓）。
pub const RUN_TIMEOUT_SECS: f32 = 300.0;
/// 發明冷卻（秒）：一次發明嘗試後（無論成敗）至少隔這麼久才再想——別每個 tick 打 LLM。
pub const INVENT_COOLDOWN_SECS: f32 = 300.0;
/// 發明用採集的搜尋半徑（格）：比日常採集（16）大——她在「為了目標特地找材料」，
/// 值得走遠一點；仍有界，找不到就誠實失敗（記教訓），不會無限漫遊。
pub const INVENT_GATHER_RADIUS: i32 = 28;

// ── 原語（primitives）：居民已有的原子能力，正名為可組合的白名單 ────────────────
//
// v1 白名單刻意只開兩個原語（採集／合成）——這已足夠打通「想要材料 → 自己想出
// 採料+合成的路 → 學會」全鏈。走路/挖/放其實內含在採集裡（走到資源旁、挖下來）；
// 之後（搭橋、繞路…）再逐步開放更多原語，白名單擴充點都在這裡。

/// LLM 計畫裡的一步（serde 落地格式；載回或解析時再過 [`check_step`] 白名單驗證）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum PrimStep {
    /// 採集：把某資源採到背包裡至少 `count` 個（**後置條件語意**：背包夠了就算完成，
    /// 可自癒可重入——中途被打斷、重啟後重跑都安全）。
    Gather { resource: String, count: u32 },
    /// 合成：照 2×2 背包配方表合成一次（消耗背包配料、產物入包；grounded 在真配方表）。
    Craft { recipe: String },
}

/// 通過白名單驗證後的一步（執行引擎吃這個；配方指標指回 `voxel_craft` 靜態表）。
#[derive(Clone, Debug, PartialEq)]
pub enum CheckedStep {
    Gather { resource: GatherResource, count: u32 },
    Craft { recipe_id: &'static str },
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

/// 配料全是「居民採得到的原料」的 2×2 配方，才是 v1 可發明的合成步驟。
/// （火把要煤礦、床要葉片、工作台要木板加工品——她備不了料，v1 誠實不開放。）
pub fn recipe_inventable(r: &vcraft::Recipe) -> bool {
    // 用 GatherResource 的 block_id 動態組集合（不重複硬編 id，單一事實源在 voxel_skills）。
    let gatherable = [
        GatherResource::Grass.block_id(),
        GatherResource::Sand.block_id(),
        GatherResource::Dirt.block_id(),
        GatherResource::Stone.block_id(),
        GatherResource::Wood.block_id(),
    ];
    r.inputs.iter().all(|(bid, _)| gatherable.contains(bid))
}

/// v1 可發明的配方清單（2×2 背包配方中、配料全可自採的那些：木板/石磚/玻璃/農田土）。
pub fn inventable_recipes() -> impl Iterator<Item = &'static vcraft::Recipe> {
    vcraft::RECIPES.iter().filter(|r| recipe_inventable(r))
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

/// 心願文字裡的「可合成材料」關鍵詞 → 目標。只收 v1 可發明配方的產物
/// （玻璃/木板/石磚/農田土——她真的有路子自己做出來的東西），別的詞不誤觸發。
const MATERIAL_KEYWORDS: [(&str, u8); 4] = [
    ("玻璃", 10),
    ("木板", 8),
    ("石磚", 9),
    ("農田土", 11),
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

/// 材料 id → 繁中名（只覆蓋 v1 目標材料；單一事實源在 voxel_craft 的 name_zh，
/// 此處為靜態便利查表——與 MATERIAL_KEYWORDS 同步維護）。
pub fn material_name(block_id: u8) -> &'static str {
    match block_id {
        10 => "玻璃",
        8 => "木板",
        9 => "石磚",
        11 => "農田土",
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
    /// 這一步的後置條件已滿足 → step_idx+1 再看下一步。
    Advance,
    /// 全部步驟跑完 → 呼叫端做最終後置條件驗證（[`goal_met`]）。
    Done,
}

/// 看目前這一步該做什麼（**後置條件語意**：採集步看「背包夠不夠」而非「採了幾次」，
/// 已有存量直接跳過、中斷重跑也安全）。純函式、可測。
pub fn next_action(run: &InventRun, bag: &HashMap<u8, u32>) -> StepAction {
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

/// 最終後置條件：背包裡真的有目標材料（≥1）——「她真的做出來了」的可驗證證據。
pub fn goal_met(bag: &HashMap<u8, u32>, goal_block: u8) -> bool {
    bag.get(&goal_block).copied().unwrap_or(0) >= 1
}

/// **計畫可行性模擬**（純資料、零世界、零 LLM）：在提案階段就用後置條件語意把整串
/// 步驟走一遍——採集步把背包補到至少 `count`、合成步照真配方扣料/產出——
/// 抓出「採 1 個木頭卻要合成需 2 木的木板」這類算術不通的計畫。
/// 回 `Err(繁中原因)` 時，呼叫端可把原因回饋給便宜腦**重試一次**（Voyager 式迭代精煉，
/// 成本有界），省得她白走一趟才發現計畫行不通。純函式、可測。
pub fn simulate_plan(
    steps: &[CheckedStep],
    start_bag: &HashMap<u8, u32>,
    goal_block: u8,
) -> Result<(), String> {
    let mut bag = start_bag.clone();
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
                    let need: Vec<String> = r
                        .inputs
                        .iter()
                        .map(|(bid, n)| format!("{}×{}", input_name(*bid), n))
                        .collect();
                    return Err(format!(
                        "合成「{}」需要 {}，但照這個計畫走到這一步時背包裡的材料不夠——\
                        採集步驟的數量必須足以支付後續所有合成的配料",
                        r.name_zh,
                        need.join("+")
                    ));
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

/// 存檔前把計畫**正規化成自足技能**：每個合成步前補上「確保配料足夠」的採集步。
///
/// 為什麼需要：便宜腦看得到「當下背包」——若她手上剛好有料，提出的計畫可能只有合成步
/// （實測真的發生：背包已有木頭×2 → 計畫＝`[craft plank]`）。這種計畫**當下**可行，
/// 但存成技能後換個時空（背包空了）就跑不動——技能該是帶著走的本事，不依賴運氣。
/// 採集步是後置條件語意（背包夠了就不多採），補上的步在有料時是零成本 no-op，
/// 沒料時正好把料補齊 → 正規化後的技能**從空背包也保證可行**。純函式、確定性、可測。
pub fn canonicalize_steps(steps: &[CheckedStep]) -> Vec<PrimStep> {
    let mut out = Vec::new();
    for s in steps {
        match s {
            CheckedStep::Gather { resource, count } => out.push(PrimStep::Gather {
                resource: token_of(*resource).to_string(),
                count: *count,
            }),
            CheckedStep::Craft { recipe_id } => {
                if let Some(r) = vcraft::find_recipe(recipe_id) {
                    for (bid, need) in r.inputs {
                        // 可發明配方的配料一定是可採原料（recipe_inventable 已保證）。
                        if let Some(res) = crate::voxel::Block::from_u8(*bid)
                            .and_then(GatherResource::from_block)
                        {
                            out.push(PrimStep::Gather {
                                resource: token_of(res).to_string(),
                                count: *need,
                            });
                        }
                    }
                }
                out.push(PrimStep::Craft { recipe: recipe_id.to_string() });
            }
        }
    }
    out
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

/// 組發明用的 (system, user) prompt。事實全部 grounded：
/// 可採資源清單來自 GatherResource、配方節錄來自 voxel_craft 真表——她想出來的路
/// 一定踩在真實世界規則上，執行器才驗證得了。
pub fn invention_prompt(
    resident_name: &str,
    goal: &MaterialGoal,
    desire: &str,
    bag_note: &str,
) -> (String, String) {
    // 配方節錄：只列可發明配方（配料她採得到的），每條含 id 與配料事實。
    let recipe_lines: Vec<String> = inventable_recipes()
        .map(|r| {
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
        })
        .collect();
    let system = format!(
        "你是{resident_name}，乙太方界的居民。你要自己想辦法解決一個處境：把你會的基礎動作\
        組合成一個新技能。你只會兩種基礎動作（原語）：\n\
        1. 採集：{{\"op\":\"gather\",\"resource\":\"<資源>\",\"count\":<數量1~{max_c}>}}，\
        resource 只能是 grass / sand / dirt / stone / wood。\n\
        2. 合成：{{\"op\":\"craft\",\"recipe\":\"<配方id>\"}}。\n\
        你知道的合成配方（事實，不可捏造別的）：\n{recipes}\n\
        注意：合成會**消耗**配料——採集步驟的數量必須足以支付後續所有合成所需的配料\
        （例如要合成需要木頭×2的配方，前面就得先採集至少 2 個木頭）。\n\
        請只輸出一個 JSON 物件（不要任何其他文字或說明）：\n\
        {{\"name\":\"<你給這個技能取的名字，繁體中文，最多{max_n}字>\",\"steps\":[<原語序列，最多{max_s}步>]}}",
        max_c = MAX_GATHER_COUNT,
        max_n = SKILL_NAME_MAX_CHARS,
        max_s = MAX_STEPS,
        recipes = recipe_lines.join("\n"),
    );
    let user = format!(
        "處境：你心裡想著「{desire}」，想要「{goal}」這種材料，但你的背包裡沒有。\
        你的背包現況：{bag}。請用你的原語組合出一個能讓背包裡出現「{goal}」的步驟計畫。",
        goal = goal.name_zh,
        bag = if bag_note.is_empty() { "空的" } else { bag_note },
    );
    (system, user)
}

/// 配料 id → 繁中名（prompt 事實行用；只需覆蓋可發明配方會用到的原料）。
fn input_name(bid: u8) -> &'static str {
    match bid {
        2 => "泥土",
        3 => "石頭",
        4 => "沙子",
        5 => "木頭",
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
        })
        .collect::<Vec<_>>()
        .join("→")
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
        // 工作台配方（3×3 的 glass_wb 不在 2×2 白名單）→ 拒絕。
        let raw = r#"{"name":"大量玻璃","steps":[{"op":"craft","recipe":"glass_wb"}]}"#;
        assert!(parse_plan(raw).is_none());
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
            next_action(&run, &bag),
            StepAction::StartGather { resource: GatherResource::Sand }
        );
    }

    #[test]
    fn next_action_skips_gather_when_bag_already_has() {
        // 背包已有 2 沙 → 採集步的後置條件已滿足 → Advance（自癒/可重入）。
        let run = glass_run();
        let bag = HashMap::from([(4u8, 2u32)]);
        assert_eq!(next_action(&run, &bag), StepAction::Advance);
    }

    #[test]
    fn next_action_craft_then_done() {
        let mut run = glass_run();
        run.step_idx = 1;
        let bag = HashMap::from([(4u8, 2u32)]);
        assert_eq!(next_action(&run, &bag), StepAction::DoCraft { recipe_id: "glass" });
        run.step_idx = 2;
        assert_eq!(next_action(&run, &bag), StepAction::Done);
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
            match next_action(&run, &bag) {
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
        let err = simulate_plan(&p.steps, &HashMap::new(), 8).unwrap_err();
        assert!(err.contains("木板"), "錯誤原因應點名不夠料的配方：{err}");
    }

    #[test]
    fn simulate_accepts_correct_plan() {
        let raw = r#"{"name":"備木成板","steps":[{"op":"gather","resource":"wood","count":2},{"op":"craft","recipe":"plank"}]}"#;
        let p = parse_plan(raw).unwrap();
        assert!(simulate_plan(&p.steps, &HashMap::new(), 8).is_ok());
    }

    #[test]
    fn simulate_rejects_plan_missing_goal() {
        // 計畫做得出玻璃、但目標是木板 → 跑完背包沒有目標材料，擋下。
        let p = parse_plan(glass_plan_json()).unwrap();
        let err = simulate_plan(&p.steps, &HashMap::new(), 8).unwrap_err();
        assert!(err.contains("木板"));
    }

    #[test]
    fn simulate_uses_existing_bag_stock() {
        // 背包已有 2 木 → 「直接合成木板」的計畫也可行（後置條件語意、少採不浪費）。
        let raw = r#"{"name":"就地取材","steps":[{"op":"craft","recipe":"plank"}]}"#;
        let p = parse_plan(raw).unwrap();
        let bag = HashMap::from([(5u8, 2u32)]);
        assert!(simulate_plan(&p.steps, &bag, 8).is_ok());
        // 空背包則不可行。
        assert!(simulate_plan(&p.steps, &HashMap::new(), 8).is_err());
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
        assert!(simulate_plan(&p.steps, &HashMap::new(), 8).is_err());
        // 正規化後：補了採集步 → 空背包可行。
        let canon = canonicalize_steps(&p.steps);
        let checked = check_stored_steps(&canon).expect("正規化版應過存檔白名單");
        assert!(simulate_plan(&checked, &HashMap::new(), 8).is_ok(), "正規化技能應自足");
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
        assert!(simulate_plan(&checked, &HashMap::new(), 10).is_ok());
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
    fn inventable_recipes_are_only_gatherable_input_ones() {
        let ids: Vec<&str> = inventable_recipes().map(|r| r.id).collect();
        // 玻璃/木板/石磚/農田土：配料全是可自採原料 → 可發明。
        for want in ["glass", "plank", "stone_brick", "till"] {
            assert!(ids.contains(&want), "{want} 應可發明");
        }
        // 火把要煤礦、床要葉片、工作台要木板（加工品）→ 不可發明（v1 誠實邊界）。
        for no in ["torch", "bed", "workbench"] {
            assert!(!ids.contains(&no), "{no} 不應可發明");
        }
    }

    // ── prompt / 台詞 ─────────────────────────────────────────────────────────

    #[test]
    fn invention_prompt_is_grounded_and_strict() {
        let goal = MaterialGoal { block_id: 10, name_zh: "玻璃" };
        let (sys, user) = invention_prompt("露娜", &goal, "好想要一塊玻璃", "木頭×1");
        // 原語白名單與嚴格輸出格式都在 system。
        assert!(sys.contains("gather") && sys.contains("craft"));
        assert!(sys.contains("JSON"));
        // grounded 配方事實：玻璃那條一定在（2 沙 → 玻璃）。
        assert!(sys.contains("glass") && sys.contains("沙子"));
        // 處境與背包現況在 user。
        assert!(user.contains("玻璃") && user.contains("木頭×1"));
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
    }
}
