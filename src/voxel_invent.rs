//! 乙太方界·技能發明（真進化第一刀＋第二刀＋第四刀）——居民自己把**基礎動作原語**
//! 組合成解法，成功後存成「自己的技能」，之後同處境**直接重用（零 LLM）**。
//!
//! **第二刀（工作台配方鏈＋放置原語）**：第一刀只開 gather/craft 兩原語、只支援 2×2
//! 背包配方——工作台 3×3 的配方鏈她發明不了。第二刀新增 `place`（把背包裡的站點方塊
//! 放到自己旁邊）與 `craft_wb`（3×3 工作台合成，前提：附近有已放置的工作台），
//! 可行性模擬懂「這配方需要工作台」＋「工作台本身可由 4 木板合成」——她第一次能自己
//! 想出**多階段鏈**：採木→合木板→合工作台→放置→3×3 合成目標物。
//!
//! **第四刀（熔爐冶煉，接續第三刀技能組合技能）**：第二刀刻意誠實劃了邊界——
//! 「熔爐冶煉配方不算，用熔爐還要放置熔爐＋冶煉原語，留給下一刀」。本刀補上那個原語：
//! `smelt`（熔爐冶煉，前提：附近有已放置的熔爐；**冶煉需要時間**，不像 craft/craft_wb
//! 瞬間完成——`InventRun::smelt_wait` 讓她「開爐→等待→回來收成」，比照 [`voxel_smelt`]
//! 給真人玩家的煨煮手感）。這一刀讓她第一次能自己做出**拋光石**（`smelt_stone`）——
//! 一種除了熔爐冶煉之外**沒有任何其他途徑**能得到的建材，是全鏈第一個「只有走完整條
//! 採集→備料→蓋工作台→蓋熔爐→冶煉→等待」才碰得到的目標物。鐵/魚/薯/莓果醬等冶煉
//! 配方的生料（礦石/漁獲/作物）仍不在她能自採的原語閉包內，誠實維持不可發明。
//!
//! **第五刀（鑿井尋礦，接續第四刀熔爐冶煉）**：把第四刀留在閉包外的**礦石**接進來——
//! 煤礦/鐵礦進 `GatherResource`（單一事實源 `ALL`），可自採種子集隨之長大，火把、
//! **鐵錠**（smelt_iron，1 鐵礦＋1 煤礦→2 鐵錠）、鐵磚、紅陶磚、花盆…整條鐵系鏈
//! 第一次自動落進可發明閉包、也自動長進好奇心的「可能性目錄」。礦石稀疏散佈在深層
//! 石頭帶（煤 y≤3、鐵 y≤1），腳邊亂開井多半撲空 → 採集步對礦石改走**定向礦井**：
//! `voxel_skills::find_nearest_ore_excl` 先找最近一顆還沒被挖走的礦（delta overlay
//! 誠實判定），`voxel_directed_task::plan_ore_well` 反推井口讓既有階梯井恰好把那顆
//! 礦清出入袋——井挖到礦、礦入背包、走得回地面，發明引擎其餘一切照舊。漁獲/作物
//! 生料仍不可自採，誠實留在閉包外。
//!
//! **第六刀（漁獲入自採閉包，接續第五刀鑿井尋礦）**：把第四刀留在閉包外的**漁獲**
//! 接進來——新原語 `fish`（釣魚，前提：附近要有水；**跟冶煉一樣需要等待**，浮標靜候
//! 上鉤才收竿，`InventRun::fish_wait` 讓她「拋竿→等待→收竿入袋」，比照 [`voxel_fishing`]
//! 給真人玩家的垂釣手感，只是刻意簡化成固定等候時間、恆定收穫小魚，不模擬雨天/深水/
//! 稀有魚——那些是玩家垂釣才有的講究，發明引擎只要「魚生料可自採」這件事成立）。
//! 小魚（`voxel_fishing::FISH_ID`）進種子集 → **烤魚**（`smelt_fish`，唯一途徑）第一次
//! 自動落進可發明閉包——「想要烤魚」的心願第一次真能被自己實現：拋竿釣起生魚、放進
//! 熔爐烤熟。與礦石不同，水是**地表**資源、不必挖井——`water_nearby` 只在附近**找不找得
//! 到水**這一件事上把關（比照 `station_nearby` 的鄰近半徑檢查），找不到水就誠實失敗，
//! 不會替她跑去遠方找水（那是「移動去資源」的下一刀，本刀刻意只做「附近有水才釣得成」）。
//!
//! **第七刀（作物入自採閉包，接續第六刀漁獲）**：把第四刀留在閉包外的**作物生料**
//! 接進來——新原語 `harvest`（收成，前提：附近要有一畦這種作物已經熟了）。與挖礦/釣魚
//! 不同，她**種不出**新的一畦（種子至今沒有任何自採途徑，誠實留給更後面一刀）——本刀只讓
//! 她能撞見**已經存在**（多半是玩家種的）的熟作物、順手收一點，比照 `water_nearby` 的
//! 鄰近檢查精神：找不到就誠實失敗，不會替她跑去遠方找、更不會替她播種。小麥/胡蘿蔔/馬鈴薯
//! 生料進種子集 → **麵包**（3 小麥）、**烤地薯**（smelt_potato，唯一途徑）、**野菜暖湯**
//! （2 胡蘿蔔+2 馬鈴薯+1 小麥，三種作物齊聚的複雜料理）第一次自動落進可發明閉包。
//! **南瓜刻意不含**：南瓜是季限作物（933），收成閉包要不要跟著季節走是另一個設計決定，
//! 留給下一刀，本刀先打通常態三作物。
//!
//! **第八刀（移動去資源，接續第六/七刀）**：換軸——不是再接一種材料，是把已有的兩個原語
//! （`fish`／`harvest`）從「附近沒有就放棄」升級成「附近沒有、走遠一點找得到就走過去」。
//! 第六刀模組頭註早預告了這一刀：`water_nearby`／`ripe_crop_nearby` 只做鄰近檢查
//! （半徑 12），找不到就誠實失敗，即便走個二三十格外其實就有水有田。本刀在鄰近檢查失手時，
//! 改用 [`find_water`]／[`find_ripe_crop_far`] 在 `INVENT_GATHER_RADIUS`（56 格）內螺旋
//! 再找一次——找到就設 [`InventWalk`] 走過去（比照 [`crate::voxel_skills::GatherSkill`]
//! 既有的走路安全機制：沿牆滑行、逾時放棄、走不到不硬闖），到位後鄰近檢查自然通過、原語照舊
//! 執行；找不到（真的四下無水無田）才是誠實失敗，不無止盡漫遊、不隔空施法。這是發明引擎從
//! 「附近剛好有」升到「她會自己想辦法」的一步——她第一次會為了心裡的計畫主動走去更遠的地方。
//!
//! **第九刀（播種自給，接續第七刀作物入自採閉包）**：第七刀模組頭註早留了誠實邊界——
//! 「她種不出新的一畦（種子至今沒有任何自採途徑，誠實留給更後面一刀）」。本刀補上種子來源
//! ＋播種動作：日常採集草皮／泥土時，比照玩家破壞同型地表的既有掉落規則（`voxel_ws.rs`
//! 破壞分潤表），額外附贈一顆胡蘿蔔／馬鈴薯種子——她第一次能自己攢下種子。收成步鄰近／
//! 遠尋熟作物皆落空時，若手邊已有對應種子、附近又找得到可翻土的地表（草皮／泥土），就地
//! 翻土播種（[`tillable_ground_nearby`]）——這次嘗試仍誠實算失敗（種子還沒長熟，領不到
//! 材料），但世界因此多了一畦她自己種下的田：下次冷卻後再試、或田先熟了，屢敗屢種本身
//! 就是真的進展。**小麥刻意不含**：小麥種子只從葉片掉落，居民不採葉片，誠實維持只能撞見
//! 既有麥田（比照第七刀刻意排除南瓜的邊界精神）。
//!
//! **第十刀（莓果入自採閉包，接續第七刀作物入自採閉包）**：第七刀模組頭註留的邊界只點名
//! 「小麥/胡蘿蔔/馬鈴薯」三種一次性作物，`voxel_berry`（多年生莓果叢）從沒被接進來——
//! 她至今連撞見一叢已經結果的莓果叢都不會順手採，果醬（`smelt_jam`，唯一途徑）自然也
//! 沒進過閉包，996 刀走完整條發明鏈仍留了這一味沒補。本刀把 `CropResource::Berry`
//! 接上：她會找**已經結果**的莓果叢（`Block::BerryBushRipe`）順手採，比照第八刀「附近
//! 沒有就走遠一點找」的既有機制；莓果叢是多年生（採完回退成苗、重新計時，不像一次性
//! 作物採完要重播種），故**自己種下新的一叢**這件事誠實留給更後面一刀（`seed_id`／
//! `tillable_block` 皆回 `None`，與小麥同款邊界——她只能撞見**已經存在**的叢）。
//! 莓果（`BERRY_ID`）進種子集 → **莓果醬**（`smelt_jam`，3 莓果→1 果醬）第一次自動
//! 落進可發明閉包。
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
use crate::voxel_berry as vberry;
use crate::voxel_craft as vcraft;
use crate::voxel_farm as vfarm;
use crate::voxel_fishing::FISH_ID;
use crate::voxel_skills::{column_top, spiral_find, GatherResource};

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
/// 56 格：群系更新後樹可能被劃到較遠處，給她走得到的空間。
pub const INVENT_GATHER_RADIUS: i32 = 56;

/// 釣魚原語一次拋竿的等候時間（秒，第六刀）：拋竿→等待→收竿的固定節奏，簡化自
/// [`voxel_fishing::BITE_MIN_SECS`]／`BITE_MAX_SECS` 的隨機區間——發明引擎不模擬雨天
/// /深水/稀有魚，只要「等一下、收穫一條小魚」這個節奏成立即可。
pub const INVENT_FISH_WAIT_SECS: f32 = 5.0;

/// 一次發明採集最多開幾口階梯井（有界防呆）：需要地下資源（石／泥）而地表無天然源時，
/// 就地往下挖一口階梯井採料（見 [`resource_is_underground`]）。一口井 depth 12、清出的實心
/// 方塊誠實入袋，典型石料需求（×3～×8）一口即足；罕見不夠再開一口，達上限仍不夠 → 誠實失敗，
/// 不無限挖。3 口＝夠寬裕又不失控。
pub const INVENT_MAX_WELLS: u32 = 3;

/// 同一發明目標連敗幾次後進退避（防「同一釣竿試了又試」的鬼打牆迴圈）。
pub const INVENT_BACKOFF_THRESHOLD: u8 = 2;
/// 退避持續時間（秒）：2 小時內好奇心不再挑這個目標，重啟歸零可接受。
pub const INVENT_BACKOFF_SECS: f32 = 7200.0;

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
    /// 熔爐冶煉（第四刀）：照熔爐配方表冶煉一次。**世界前提**：附近（[`STATION_NEAR_RADIUS`]
    /// 格內）要有已放置的熔爐。**時間前提**：冶煉需要煨煮時間（見 [`InventRun::smelt_wait`]），
    /// 不像 craft/craft_wb 瞬間完成——開爐後這一步要等熟成才算過。
    Smelt { recipe: String },
    /// 引用自己已經學會的技能（第三刀·技能組合技能）：把她之前發明過、已經存進技能庫
    /// 的一整段步驟當一步用——「已經會的事」不用每次重新拆成一串原語。只在
    /// [`expand_step`] 展開（查她自己的技能庫換成具體原語序列），[`check_step`] 對它
    /// 一律回 `None`（單獨出現視為無效——必須先展開才是合法的執行單位）。
    UseSkill { name: String },
    /// 釣魚（第六刀）：釣起至少 `count` 條小魚到背包裡（後置條件語意，同 `Gather`）。
    /// **世界前提**：附近（[`WATER_NEAR_RADIUS`] 格內）要有水面可拋竿。**時間前提**：
    /// 跟冶煉一樣要等（見 [`InventRun::fish_wait`]），不是拋竿就立刻有魚。
    Fish { count: u32 },
    /// 收成（第七刀）：收成至少 `count` 份指定作物的生料到背包裡（後置條件語意，同 `Gather`）。
    /// **世界前提**：附近（[`CROP_NEAR_RADIUS`] 格內）要有一畦這種作物已經熟了——她不會播種，
    /// 只能撞見已經存在的熟作物。無時間前提：作物已經熟了，找到就能立刻收成。
    Harvest { crop: String, count: u32 },
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
    /// 熔爐冶煉（第四刀，需附近有已放置的熔爐；需等煨煮時間，見 [`InventRun::smelt_wait`]）。
    Smelt { recipe_id: &'static str },
    /// 釣魚（第六刀，需附近有水；需等上鉤時間，見 [`InventRun::fish_wait`]）。
    Fish { count: u32 },
    /// 收成（第七刀，需附近有一畦這種作物已經熟了；無等待，找到即收）。
    Harvest { crop: CropResource, count: u32 },
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
        // 鑿井尋礦 v1：礦石可採（發明側自動改走定向礦井）。「鐵」單字不收——
        // 與「鐵錠/鐵磚」歧義，逼便宜腦寫明確的 iron_ore／鐵礦。
        "coal_ore" | "coal" | "煤" | "煤礦" => Some(GatherResource::CoalOre),
        "iron_ore" | "iron" | "鐵礦" => Some(GatherResource::IronOre),
        _ => None,
    }
}

/// 作物種類（第七刀·作物入自採閉包）：對應成熟作物方塊與收成產出，讓她能鎖定
/// 收成「特定一種」而非隨便撿到什麼就收——麵包要小麥顆粒、烤地薯要生馬鈴薯、
/// 野菜暖湯三種都要。南瓜刻意不含（見模組頭註）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CropResource {
    Wheat,
    Carrot,
    Potato,
    /// 莓果（第十刀）——多年生莓果叢結出的果實，與一次性作物不同：只認「已結果的叢」，
    /// 自己種下新的一叢誠實留給更後面一刀（見 `seed_id`／`tillable_block`）。
    Berry,
}

impl CropResource {
    /// 收成產出的生料物品 id（`voxel_farm`/`voxel_berry` 常數，單一事實源）。
    pub fn raw_id(self) -> u8 {
        match self {
            CropResource::Wheat => vfarm::WHEAT_ID,
            CropResource::Carrot => vfarm::CARROT_ID,
            CropResource::Potato => vfarm::POTATO_ID,
            CropResource::Berry => vberry::BERRY_ID,
        }
    }

    /// 這種作物「熟了」的方塊型別（`ripe_crop_nearby` 找的目標、`harvest_food_of` 收成的對象）。
    pub fn mature_block(self) -> Block {
        match self {
            CropResource::Wheat => Block::WheatMature,
            CropResource::Carrot => Block::CarrotMature,
            CropResource::Potato => Block::PotatoMature,
            CropResource::Berry => Block::BerryBushRipe,
        }
    }

    /// 繁中名（台詞／Feed 用）。
    pub fn display_name(self) -> &'static str {
        match self {
            CropResource::Wheat => "小麥",
            CropResource::Carrot => "胡蘿蔔",
            CropResource::Potato => "馬鈴薯",
            CropResource::Berry => "莓果",
        }
    }

    /// 英文 token（存檔/白名單正規形，比照 `token_of`）。
    pub fn token(self) -> &'static str {
        match self {
            CropResource::Wheat => "wheat",
            CropResource::Carrot => "carrot",
            CropResource::Potato => "potato",
            CropResource::Berry => "berry",
        }
    }

    /// 播種自給（第九刀）需要的種子物品 id——**只有能從日常採集額外攢到種子的作物**才有：
    /// 胡蘿蔔種子來自採草皮、馬鈴薯種子來自採泥土（比照 `voxel_ws.rs` 玩家破壞分潤表）。
    /// 小麥種子只從葉片掉落、居民不採葉片，誠實回 `None`（她仍只能撞見既有麥田）。
    /// 莓果叢（第十刀）同款誠實邊界：自己種一叢新的留給更後面一刀，回 `None`。
    pub fn seed_id(self) -> Option<u8> {
        match self {
            CropResource::Wheat => None,
            CropResource::Carrot => Some(vfarm::CARROT_SEEDS_ID),
            CropResource::Potato => Some(vfarm::POTATO_SEEDS_ID),
            CropResource::Berry => None,
        }
    }

    /// 播種前要先翻土的地表方塊型別（草皮／泥土；小麥/莓果不支援自給播種，回 `None`）。
    pub fn tillable_block(self) -> Option<Block> {
        match self {
            CropResource::Wheat => None,
            CropResource::Carrot => Some(Block::Grass),
            CropResource::Potato => Some(Block::Dirt),
            CropResource::Berry => None,
        }
    }

    /// 翻土播種後的「已播種、還沒熟」方塊態（`voxel_farm` 種植流程的 Seeded 態）。
    /// 莓果從不會走到這條路（`tillable_block` 回 `None`），但填上真實對應值
    /// （莓果叢種下就是未結果的苗態）保持誠實、不留假值。
    pub fn seeded_block(self) -> Block {
        match self {
            CropResource::Wheat => Block::FarmSoilSeeded,
            CropResource::Carrot => Block::CarrotSeeded,
            CropResource::Potato => Block::PotatoSeeded,
            CropResource::Berry => Block::BerryBush,
        }
    }

    /// 對應的 `voxel_farm::CropKind`（登記進 farm store 的生長計時器用）；只有支援自給播種
    /// 的作物才有意義，`None` = 這種作物不走 `voxel_farm` 的計時（莓果叢有自己的
    /// `voxel_berry::BerryStore`，且第十刀仍不支援自己種新的一叢）。
    pub fn crop_kind(self) -> Option<vfarm::CropKind> {
        match self {
            CropResource::Wheat => Some(vfarm::CropKind::Wheat),
            CropResource::Carrot => Some(vfarm::CropKind::Carrot),
            CropResource::Potato => Some(vfarm::CropKind::Potato),
            CropResource::Berry => None,
        }
    }
}

/// 作物 token → 型別（白名單，同 `resource_from_token` 收英文/繁中兩種寫法）。
pub fn crop_from_token(s: &str) -> Option<CropResource> {
    match s.trim() {
        "wheat" | "小麥" | "麥" => Some(CropResource::Wheat),
        "carrot" | "胡蘿蔔" | "蘿蔔" => Some(CropResource::Carrot),
        "potato" | "馬鈴薯" | "地瓜" | "薯" => Some(CropResource::Potato),
        "berry" | "莓果" | "莓" => Some(CropResource::Berry),
        _ => None,
    }
}

/// 生料 block id → 作物型別（`ensure_have` 遞迴補料時反查用；查無 → `None`）。
fn crop_of_raw_id(bid: u8) -> Option<CropResource> {
    match bid {
        x if x == vfarm::WHEAT_ID => Some(CropResource::Wheat),
        x if x == vfarm::CARROT_ID => Some(CropResource::Carrot),
        x if x == vfarm::POTATO_ID => Some(CropResource::Potato),
        x if x == vberry::BERRY_ID => Some(CropResource::Berry),
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
        // 種子集：一步可得的葉節點（單一事實源見 `one_step_resource_ids`，與階梯好奇心的
        // `base_resource_ids` 共用，別讓「她弄得到」與「她搆得著」兩套事實分岔）。
        let mut set: HashSet<u8> = one_step_resource_ids().clone();
        loop {
            let mut grew = false;
            // 第四刀：熔爐冶煉配方也納入閉包——只有「配料全可自採/鏈上加工品」的冶煉
            // （如 smelt_stone：石×3→拋光石）才會真的進來；鐵/魚/薯/莓果醬的生料
            // （礦石/漁獲/作物）不在種子集也不由任何配方鏈產出，誠實留在閉包外。
            for r in vcraft::RECIPES
                .iter()
                .chain(vcraft::WORKBENCH_RECIPES.iter())
                .chain(vcraft::FURNACE_RECIPES.iter())
            {
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

/// 可發明的熔爐冶煉配方清單（第四刀）：拋光石（唯一途徑）＋玻璃/石磚的冶煉版本
/// （配料皆可自採，冶煉只是更高效的另一條路）；鐵/魚/薯/莓果醬的生料她弄不到，誠實排除。
pub fn inventable_furnace_recipes() -> impl Iterator<Item = &'static vcraft::Recipe> {
    vcraft::FURNACE_RECIPES.iter().filter(|r| recipe_inventable(r))
}

/// 白名單驗證一步：資源在白名單、數量 1..=上限、配方存在且可發明。壞的一律 `None`。
///
/// **op 標籤自動修復**（隔離實測 qwen2.5:3b 驅動）：小模型常把 craft／craft_wb 兩張
/// 清單用反（`craft_wb workbench`），即使重試回饋點名正確原語仍改不動——但這種錯
/// **意圖無歧義**（配方 id 真實存在、只是掛在另一張表上），機械可判定就直接修正，
/// 比照 [`resource_from_token`] 收繁中別名的寬容精神。修復只換查表、不放水正確性：
/// 修成 CraftWb 的步仍受可行性模擬「必須先有工作台」把關；兩張表都查無才拒絕。
pub fn check_step(s: &PrimStep) -> Option<CheckedStep> {
    match s {
        PrimStep::Gather { resource, count } => {
            let res = resource_from_token(resource)?;
            if *count == 0 || *count > MAX_GATHER_COUNT {
                return None;
            }
            Some(CheckedStep::Gather { resource: res, count: *count })
        }
        PrimStep::Craft { recipe } | PrimStep::CraftWb { recipe } => {
            // 先查「本來那張表」、查無再查另一張（op 標籤自動修復）；
            // 哪張表命中決定步型——步型決定執行臂與模擬的依賴檢查。
            let prefer_wb = matches!(s, PrimStep::CraftWb { .. });
            let (r, is_wb) = if prefer_wb {
                vcraft::find_workbench_recipe(recipe)
                    .map(|r| (r, true))
                    .or_else(|| vcraft::find_recipe(recipe).map(|r| (r, false)))?
            } else {
                vcraft::find_recipe(recipe)
                    .map(|r| (r, false))
                    .or_else(|| vcraft::find_workbench_recipe(recipe).map(|r| (r, true)))?
            };
            if !recipe_inventable(r) {
                return None; // 配料她弄不到（要冶煉/稀有掉落）→ 誠實拒絕
            }
            Some(if is_wb {
                CheckedStep::CraftWb { recipe_id: r.id }
            } else {
                CheckedStep::Craft { recipe_id: r.id }
            })
        }
        PrimStep::Place { block } => {
            let bid = place_block_from_token(block)?;
            Some(CheckedStep::Place { block_id: bid })
        }
        PrimStep::Smelt { recipe } => {
            let r = vcraft::find_furnace_recipe(recipe)?;
            if !recipe_inventable(r) {
                return None; // 生料她弄不到（礦石/漁獲/作物）→ 誠實拒絕
            }
            Some(CheckedStep::Smelt { recipe_id: r.id })
        }
        // 單獨出現一律無效——`UseSkill` 只能透過 [`expand_step`] 展開成具體原語序列，
        // 不是可執行的原子步（也保證存檔技能永遠不會殘留一顆沒展開的 `UseSkill`：
        // 一旦不慎混進 raw_steps，`check_stored_steps` 會在這裡誠實判它失效）。
        PrimStep::UseSkill { .. } => None,
        PrimStep::Fish { count } => {
            if *count == 0 || *count > MAX_GATHER_COUNT {
                return None;
            }
            Some(CheckedStep::Fish { count: *count })
        }
        PrimStep::Harvest { crop, count } => {
            let c = crop_from_token(crop)?;
            if *count == 0 || *count > MAX_GATHER_COUNT {
                return None;
            }
            Some(CheckedStep::Harvest { crop: c, count: *count })
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

// ── 技能組合技能（第三刀·真進化）：已學會的技能可當一塊積木疊進新計畫 ───────────
//
// 動機（線上真實日誌驅動）：複雜目標（水井藍圖／瞭望台藍圖…）攤開成純原語常常
// 一路超過 [`MAX_STEPS`]（8），發明從沒機會走到執行就先在解析階段被拒絕——但如果
// 她已經學過「自製木板」「蓋工作台」這類子技能，理應可以直接**引用**、不必每次
// 重新拆成一串 gather/craft。`UseSkill` 就是這塊拼圖：LLM 提案裡的一步可以是
// 「用某個我已經會的技能」，展開時查她自己的技能庫換成具體原語序列——一個 op
// 換來好幾個原語，讓深度鏈第一次有機會塞進淺淺的 raw op 上限裡。

/// 展開一步：一般原語照舊過 [`check_step`] 白名單、包成單元素 `Vec`；`UseSkill`
/// 則查 `known`（呼叫端傳入的「這位居民自己會的技能」`(名字, 原語序列)` 清單）換成
/// 該技能已存好的原語序列，並**逐步再驗證一次**白名單（配方表可能已變動，不盲信舊檔）。
///
/// **只展開一層**：引用的技能若自己內部又含 `UseSkill`（理論上到不了——存檔前一律
/// 已展平，見 [`expand_steps_with_cap`] 的呼叫端），一律拒絕整段引用，防循環引用／
/// 防深遞迴（v1 保守邊界）。名字比對去頭尾空白、精確相符（她引用的必須是自己技能庫
/// 裡一字不差的名字，不猜測相似度）。純函式、可測。
pub fn expand_step(s: &PrimStep, known: &[(String, Vec<PrimStep>)]) -> Result<Vec<CheckedStep>, String> {
    match s {
        PrimStep::UseSkill { name } => {
            let key = name.trim();
            let (_, sub_steps) = known
                .iter()
                .find(|(n, _)| n.trim() == key)
                .ok_or_else(|| format!("你還沒學會「{key}」這個技能，不能引用它"))?;
            if sub_steps.iter().any(|ss| matches!(ss, PrimStep::UseSkill { .. })) {
                return Err(format!("「{key}」本身也引用了別的技能，暫不支援疊兩層"));
            }
            sub_steps.iter().map(|ss| check_step(ss).ok_or_else(|| explain_bad_step(ss))).collect()
        }
        other => check_step(other).map(|c| vec![c]).ok_or_else(|| explain_bad_step(other)),
    }
}

/// 展開整串步驟（含 `UseSkill` 展開）→ flatten 成 `CheckedStep` 序列，並驗展開後總長
/// 落在 `[1, cap]`——**raw op 數仍卡 [`MAX_STEPS`]**（她只需列出少少幾個 op，含
/// `use_skill`）；本函式驗的是**展開後**的具體步數，用較寬的 `cap`（呼叫端傳
/// [`MAX_STORED_STEPS`]），讓組合已學技能的深度鏈有空間塞得下，同時仍然有界。
/// 純函式、可測。
pub fn expand_steps_with_cap(
    steps: &[PrimStep],
    known: &[(String, Vec<PrimStep>)],
    cap: usize,
) -> Result<Vec<CheckedStep>, String> {
    let mut out = Vec::new();
    for s in steps {
        out.extend(expand_step(s, known)?);
    }
    if out.is_empty() {
        return Err("steps 展開後不可為空".to_string());
    }
    if out.len() > cap {
        return Err(format!("展開後步數 {} 超過上限 {cap}", out.len()));
    }
    Ok(out)
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
/// 任何一步失敗都回 `None`（本次發明放棄、記冷卻），絕不 panic。不支援 `use_skill`
/// 展開（等同 [`parse_plan_detailed_with_skills`] 傳 `&[]`）——保留給沒有技能庫context
/// 的舊呼叫點，行為與第三刀之前完全一致。
pub fn parse_plan(raw: &str) -> Option<InventedPlan> {
    parse_plan_detailed(raw).ok()
}

/// [`parse_plan`] 的詳細版：失敗時回**具體的繁中原因**（哪一步、錯在哪）——
/// Voyager 式重試的關鍵回饋。實測（qwen2.5:3b 真便宜腦）看到小模型把隨身配方用
/// craft_wb 做（`craft_wb workbench`），籠統的「輸出不合法」讓它修正時瞎猜；
/// 具體指出「workbench 是隨身配方，要用 craft」才修得回來。純函式、可測。
/// 不含技能組合展開（`known` 傳 `&[]`）；要展開 `use_skill` 請用
/// [`parse_plan_detailed_with_skills`]。
pub fn parse_plan_detailed(raw: &str) -> Result<InventedPlan, String> {
    parse_plan_detailed_with_skills(raw, &[])
}

/// [`parse_plan_detailed`] 的技能組合版（第三刀）：`known` 是這位居民自己技能庫裡
/// 「(技能名, 原語序列)」清單——計畫裡的 `use_skill` 步驟會查這份清單展開成具體原語。
/// **raw op 數仍卡 [`MAX_STEPS`]**（prompt 只要求她列少少幾步）；展開後的具體步數改驗
/// 較寬的 [`MAX_STORED_STEPS`]（見 [`expand_steps_with_cap`]），讓「組合已學技能」的
/// 深度鏈有機會塞得下。純函式、可測。
pub fn parse_plan_detailed_with_skills(
    raw: &str,
    known: &[(String, Vec<PrimStep>)],
) -> Result<InventedPlan, String> {
    let start = raw.find('{').ok_or("輸出裡找不到 JSON 物件")?;
    let end = raw.rfind('}').ok_or("輸出裡找不到 JSON 物件")?;
    if end <= start {
        return Err("輸出裡找不到 JSON 物件".to_string());
    }
    let json = &raw[start..=end];
    // 先走嚴格解析（合法 JSON 零額外成本、零風險）；失敗才退而用 [`relax_json`] 修復
    // 便宜腦常見的 JSON 瑕疵（裸鍵 `count:8`、trailing comma、`//`／`/* */` 註解、單引號），
    // 再解一次。實測小模型（qwen2.5:3b）約半數計畫壞在這幾種語法瑕疵上，修復後多能救回。
    let plan: RawPlan = match serde_json::from_str::<RawPlan>(json) {
        Ok(p) => p,
        Err(_) => serde_json::from_str::<RawPlan>(&relax_json(json)).map_err(|_| {
            "JSON 解析失敗——必須是 {\"name\":\"…\",\"steps\":[…]} 且每步只用允許的原語欄位"
                .to_string()
        })?,
    };
    let name: String = plan.name.trim().chars().take(SKILL_NAME_MAX_CHARS).collect();
    if name.is_empty() {
        return Err("技能名（name）不可為空".to_string());
    }
    if plan.steps.is_empty() {
        return Err("steps 不可為空".to_string());
    }
    if plan.steps.len() > MAX_STEPS {
        return Err(format!("步數 {} 超過上限 {MAX_STEPS}", plan.steps.len()));
    }
    let steps = expand_steps_with_cap(&plan.steps, known, MAX_STORED_STEPS)?;
    Ok(InventedPlan { name, raw_steps: plan.steps, steps })
}

/// 寬容修復便宜腦常吐的「近似 JSON」——**只在嚴格解析失敗後當退路呼叫**，合法 JSON 永遠走不到這裡。
///
/// 單趟掃描、字串內容一律原樣保留（不會誤改字串裡的 `,`／`}`／`//`／看似裸鍵的字），只在
/// 結構位置修四種實測最常見的瑕疵：
/// 1. **裸物件鍵**：`count:8` → `"count":8`（鍵位置的未加引號識別字補上雙引號）。
/// 2. **trailing comma**：`[1,2,]`／`{"a":1,}` → 去掉 `}`／`]` 前多餘的逗號。
/// 3. **註解**：`// …` 行註解與 `/* … */` 區塊註解整段刪除。
/// 4. **單引號字串**：`'gather'` → `"gather"`（內含雙引號會轉義）。
///
/// 純函式、無 I/O、絕不 panic；壞到修不動就原樣吐回、留給呼叫端的 serde 再判一次失敗。
pub fn relax_json(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(input.len());
    // 容器堆疊（'{' 物件／'[' 陣列）＋「下一個 token 是否為物件鍵位置」。
    let mut stack: Vec<char> = Vec::new();
    let mut expect_key = false;
    let mut i = 0;
    while i < n {
        let c = chars[i];
        // 註解（結構位置才算；字串內容在下面的 '"' 分支整段複製，到不了這裡）。
        if c == '/' && i + 1 < n {
            if chars[i + 1] == '/' {
                i += 2;
                while i < n && chars[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            if chars[i + 1] == '*' {
                i += 2;
                while i + 1 < n && !(chars[i] == '*' && chars[i + 1] == '/') {
                    i += 1;
                }
                i = (i + 2).min(n);
                continue;
            }
        }
        // 雙引號字串：原樣複製（含跳脫），字串裡的一切都不解讀。
        if c == '"' {
            out.push('"');
            i += 1;
            while i < n {
                let d = chars[i];
                out.push(d);
                i += 1;
                if d == '\\' && i < n {
                    out.push(chars[i]);
                    i += 1;
                    continue;
                }
                if d == '"' {
                    break;
                }
            }
            expect_key = false;
            continue;
        }
        // 單引號字串 → 轉成合法雙引號字串（內含雙引號跳脫）。
        if c == '\'' {
            out.push('"');
            i += 1;
            while i < n {
                let d = chars[i];
                if d == '\\' && i + 1 < n {
                    out.push(d);
                    out.push(chars[i + 1]);
                    i += 2;
                    continue;
                }
                if d == '\'' {
                    i += 1;
                    break;
                }
                if d == '"' {
                    out.push('\\');
                }
                out.push(d);
                i += 1;
            }
            out.push('"');
            expect_key = false;
            continue;
        }
        match c {
            '{' => {
                out.push(c);
                stack.push('{');
                expect_key = true;
                i += 1;
            }
            '[' => {
                out.push(c);
                stack.push('[');
                expect_key = false;
                i += 1;
            }
            '}' | ']' => {
                trim_trailing_comma(&mut out);
                out.push(c);
                stack.pop();
                expect_key = false;
                i += 1;
            }
            ',' => {
                out.push(c);
                expect_key = matches!(stack.last(), Some('{'));
                i += 1;
            }
            ':' => {
                out.push(c);
                expect_key = false;
                i += 1;
            }
            _ if c.is_whitespace() => {
                out.push(c);
                i += 1;
            }
            _ => {
                // 物件鍵位置遇到未加引號的識別字 → 補雙引號（如 `count:` → `"count":`）。
                if expect_key
                    && matches!(stack.last(), Some('{'))
                    && (c.is_alphabetic() || c == '_')
                {
                    let mut ident = String::new();
                    while i < n && (chars[i].is_alphanumeric() || chars[i] == '_') {
                        ident.push(chars[i]);
                        i += 1;
                    }
                    out.push('"');
                    out.push_str(&ident);
                    out.push('"');
                    expect_key = false;
                } else {
                    out.push(c);
                    i += 1;
                }
            }
        }
    }
    out
}

/// 去掉 `out` 尾端「空白＊＋一個逗號」——供 `}`／`]` 前修 trailing comma。
fn trim_trailing_comma(out: &mut String) {
    let trimmed_len = out.trim_end().len();
    if out.as_bytes().get(trimmed_len.wrapping_sub(1)) == Some(&b',') {
        out.truncate(trimmed_len - 1);
    }
}

/// 白名單驗證失敗的一步 → 具體的繁中原因（回饋給便宜腦修正用）。
/// 註：craft/craft_wb 清單用反已由 [`check_step`] 自動修復、到不了這裡；
/// 會走到這裡的合成步失敗只剩「兩張表都查無此 id」與「配料弄不到」兩型。
fn explain_bad_step(s: &PrimStep) -> String {
    match s {
        PrimStep::Gather { resource, count } => {
            if resource_from_token(resource).is_none() {
                format!(
                    "採集資源「{resource}」不在白名單——只能是 grass / sand / dirt / stone / wood / coal_ore / iron_ore"
                )
            } else {
                format!("採集數量 {count} 不在 1~{MAX_GATHER_COUNT} 範圍內")
            }
        }
        PrimStep::Craft { recipe } | PrimStep::CraftWb { recipe } => {
            if vcraft::find_recipe(recipe).is_some()
                || vcraft::find_workbench_recipe(recipe).is_some()
            {
                format!("配方「{recipe}」的配料你自己弄不到（要冶煉或稀有掉落），不能用")
            } else {
                format!("「{recipe}」不在任何一張配方清單裡（隨身與工作台都查無）")
            }
        }
        PrimStep::Place { block } => {
            format!("place 只能放 workbench 或 furnace，「{block}」不在白名單")
        }
        PrimStep::Smelt { recipe } => {
            if vcraft::find_furnace_recipe(recipe).is_some() {
                format!("熔爐配方「{recipe}」的生料你自己弄不到（要挖礦/釣魚/種田），不能用")
            } else {
                format!("「{recipe}」不在熔爐配方清單裡")
            }
        }
        // 實務上到不了這裡（[`expand_step`] 攔在更前面、給出更具體的原因）；
        // 保留這支只為了讓 `explain_bad_step` 對 `PrimStep` 保持窮舉、防禦未來改動。
        PrimStep::UseSkill { name } => {
            format!("「{name}」需要展開成技能庫裡的具體步驟，不能單獨當一步")
        }
        PrimStep::Fish { count } => {
            format!("釣魚數量 {count} 不在 1~{MAX_GATHER_COUNT} 範圍內")
        }
        PrimStep::Harvest { crop, count } => {
            if crop_from_token(crop).is_none() {
                format!("收成作物「{crop}」不在白名單——只能是 wheat / carrot / potato")
            } else {
                format!("收成數量 {count} 不在 1~{MAX_GATHER_COUNT} 範圍內")
            }
        }
    }
}

/// **提案接受管線**（發明流程的單一入口）：解析＋op 修復 → **數量閉包正規化** → 模擬把關。
///
/// 為什麼正規化搬到提案階段（隔離實測 qwen2.5:3b 驅動）：便宜腦挑得對目標配方、
/// 排得對依賴順序（合工作台→放置→3×3），但**配料數量算不動**——箱子要木板×8、
/// 工作台又吃掉×4，該合 3 次木板它只合 1 次；模擬回饋點名缺料，一次有界重試仍
/// 算不對。這正是 [`canonicalize_steps`]（存檔時本來就在做的「補備料步」）機械可解
/// 的問題：把同一套正規化提前到提案階段，讓她的計畫**結構**（腦的貢獻：選對配方、
/// 排對依賴、取名字）配上引擎的**算術**（確定性補料）。模擬仍是最後防線——正規化
/// 後跑不到目標（腦選錯配方/沒做目標物）照樣拒絕、回饋、重試。純函式、可測。
pub fn accept_proposal(
    raw: &str,
    bag: &HashMap<u8, u32>,
    goal_block: u8,
    wb_nearby: bool,
    furnace_nearby: bool,
) -> Result<InventedPlan, String> {
    accept_proposal_with_skills(raw, bag, goal_block, wb_nearby, furnace_nearby, &[])
}

/// [`accept_proposal`] 的技能組合版（第三刀）：`known` 是這位居民自己技能庫裡
/// 「(技能名, 原語序列)」清單，供計畫裡的 `use_skill` 步驟展開。展開發生在
/// [`parse_plan_detailed_with_skills`]，之後 `use_skill` 已經變回具體的
/// Gather/Craft/CraftWb/Place，`canonicalize_steps`／`simulate_plan` 完全不必
/// 知道「技能組合」這回事——**存下來的技能永遠是自足的具體原語序列**，
/// 就算日後引用的那個子技能被遺忘或改變，這個新技能仍照樣能跑。純函式、可測。
pub fn accept_proposal_with_skills(
    raw: &str,
    bag: &HashMap<u8, u32>,
    goal_block: u8,
    wb_nearby: bool,
    furnace_nearby: bool,
    known: &[(String, Vec<PrimStep>)],
) -> Result<InventedPlan, String> {
    let plan = parse_plan_detailed_with_skills(raw, known)?;
    // 正規化成自足鏈（從空背包/沒工作台/沒熔爐也可行）；步數用存檔上限（正規化會變長，仍有界）。
    let canon = canonicalize_steps(&plan.steps);
    let steps = check_stored_steps(&canon)
        .ok_or_else(|| format!("補上備料步後步數超過上限 {MAX_STORED_STEPS}，計畫太迂迴"))?;
    simulate_plan(&steps, bag, goal_block, wb_nearby, furnace_nearby)?;
    Ok(InventedPlan { name: plan.name, raw_steps: canon, steps })
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
///
/// **好奇心第三刀擴充**：查手工關鍵詞表 + 掃**可發明配方的產物名**，兩邊都命中時
/// **取字數最長者**（漂流瓶 v1 修：「空玻璃瓶」若只查手工表會被「玻璃」子字串搶先
/// 誤判成玻璃(10)——產物名越長代表越具體，理當贏過較短的通用關鍵詞）。
/// 好奇心種下的心願（梯子/木鎬/釣竿…）與玩家順口提到的都接得住，
/// 單一事實源在配方表：新配方上桌自動被涵蓋，不再手動同步關鍵詞。
/// 不可發明的產物（火把要煤礦、床要葉片）仍誠實不觸發。
pub fn detect_missing_material(desire: &str) -> Option<MaterialGoal> {
    let mut best: Option<(usize, MaterialGoal)> = None;
    let mut consider = |len: usize, goal: MaterialGoal| {
        if best.as_ref().is_none_or(|(best_len, _)| len > *best_len) {
            best = Some((len, goal));
        }
    };
    for (kw, bid) in MATERIAL_KEYWORDS {
        if desire.contains(kw) {
            consider(kw.chars().count(), MaterialGoal { block_id: bid, name_zh: material_name(bid) });
        }
    }
    for r in inventable_recipes().chain(inventable_wb_recipes()).chain(inventable_furnace_recipes()) {
        if desire.contains(r.name_zh) {
            consider(r.name_zh.chars().count(), MaterialGoal { block_id: r.output_block, name_zh: r.name_zh });
        }
    }
    best.map(|(_, goal)| goal)
}

/// 材料 id → 繁中名（覆蓋目標材料＋站點方塊＋任何配方中繼加工品）。
/// 工作台／熔爐是「放置後的世界方塊」，`voxel_gift::item_name_zh` 沒有這兩個 id
/// （它只覆蓋可收進背包餽贈的物品），此處特例覆蓋；其餘一律委派給
/// `item_name_zh`——單一事實源，新物品只要在那邊補過名字，這裡與發明 prompt
/// 自動一起拿到正確名字，不必兩邊同步維護查表（此前漏同步正是「空玻璃瓶／
/// 乙太沃肥／水井藍圖」等目標材料在發明失敗訊息裡淪為泛稱「材料」，
/// 便宜腦看不懂到底缺什麼的根因）。
pub fn material_name(block_id: u8) -> &'static str {
    match block_id {
        WORKBENCH_BLOCK_ID => "工作台",
        FURNACE_BLOCK_ID => "熔爐",
        _ => crate::voxel_gift::item_name_zh(block_id),
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
    /// 熔爐冶煉煨煮倒數（第四刀）：`None`＝目前這步還沒開爐（或這步根本不是冶煉）；
    /// `Some(w)`＝已開爐，`w` 秒後熟成（tick 遞減，比照 `deadline` 同一節拍）。
    /// 熟成（`w <= 0.0`）後 [`next_action`] 才會判定該收成、推進到下一步。
    pub smelt_wait: Option<f32>,
    /// 釣魚上鉤倒數（第六刀）：`None`＝目前這步還沒拋竿（或這步根本不是釣魚）；
    /// `Some(w)`＝已拋竿，`w` 秒後上鉤（tick 遞減，比照 `smelt_wait` 同一節拍）。
    /// 上鉤（`w <= 0.0`）後 [`next_action`] 才會判定該收竿、入袋一條、再看是否還要再拋。
    pub fish_wait: Option<f32>,
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
            smelt_wait: None,
            fish_wait: None,
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
    /// 這一步是熔爐冶煉、還沒開爐 → 呼叫端驗附近有熔爐、扣生料、開始煨煮倒數。
    DoSmelt { recipe_id: &'static str },
    /// 這一步的熔爐冶煉還在煨煮中 → 這輪什麼都不做，等下個 tick 再看。
    Waiting,
    /// 這一步的熔爐冶煉已熟成 → 呼叫端把成品交付背包、推進到下一步。
    CollectSmelt { recipe_id: &'static str },
    /// 這一步的採集還沒滿足、還沒拋竿 → 呼叫端驗附近有水、開始上鉤倒數。
    StartFish,
    /// 這一步的釣魚已上鉤 → 呼叫端交付一條小魚入背包，再看是否還要再拋竿湊足數量。
    CollectFish,
    /// 這一步的收成還沒滿足 → 呼叫端驗附近有一畦這種作物熟了、就地收成入背包
    /// （無等待，找到即收；再看是否還要再收下一畦湊足數量，比照 `CollectFish` 節奏）。
    DoHarvest { crop: CropResource },
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
        Some(CheckedStep::Smelt { recipe_id }) => match run.smelt_wait {
            // 還沒開爐 → 交呼叫端驗附近有熔爐、扣生料、開始煨煮倒數。
            None => StepAction::DoSmelt { recipe_id },
            // 開了爐、還沒熟成 → 這輪什麼都不做，等下個 tick。
            Some(w) if w > 0.0 => StepAction::Waiting,
            // 熟成（<=0.0）→ 交呼叫端交付成品、推進到下一步。
            Some(_) => StepAction::CollectSmelt { recipe_id },
        },
        // 後置條件語意（同 Gather）：背包已有 count 條就跳過；不夠 → 依 fish_wait 狀態
        // 決定拋竿／等待／收竿——收竿後不直接 Advance，讓呼叫端下一輪回頭再檢查數量是否已夠
        // （一次拋竿只收一條，湊不足數量會自然再拋一輪，同 Gather 多趟採集的節奏）。
        Some(CheckedStep::Fish { count }) => {
            let have = bag.get(&FISH_ID).copied().unwrap_or(0);
            if have >= *count {
                StepAction::Advance
            } else {
                match run.fish_wait {
                    None => StepAction::StartFish,
                    Some(w) if w > 0.0 => StepAction::Waiting,
                    Some(_) => StepAction::CollectFish,
                }
            }
        }
        // 後置條件語意（同 Gather）：背包已有 count 份就跳過；不夠 → 交呼叫端找一畦附近
        // 熟了的這種作物就地收成。無等待（作物已經熟了，找到即收）——收成後不直接 Advance，
        // 讓呼叫端下一輪回頭再檢查數量是否已夠（一次只收一畦，湊不足會自然再找下一畦）。
        Some(CheckedStep::Harvest { crop, count }) => {
            let have = bag.get(&crop.raw_id()).copied().unwrap_or(0);
            if have >= *count {
                StepAction::Advance
            } else {
                StepAction::DoHarvest { crop: *crop }
            }
        }
    }
}

/// 這種採集資源是否「埋在地表底下、地表 surface-mine 永遠碰不到」——需要改走階梯礦井
/// 往下挖才採得到。**石頭／泥土**皆非草地世界的地表頂（頂層永遠是草／樹），surface 搜尋恆
/// `None` → 石器（石鎬／石斧／石鏟…）與需泥的配方一到採料步就秒失敗（實測發明成功率 0%）；
/// 本函式讓呼叫端在地表無源時，對這兩種資源改開一口 [`crate::voxel_skills::staircase_well`]
/// 階梯井（永不自困）採料。**細沙**（灘地地表）／**草皮**（地表頂）／**木頭**（樹）維持地表採集
/// ——它們本就在地表、或礦井底下根本挖不到。純函式、可測。
/// **第五刀（鑿井尋礦）**：煤礦／鐵礦也屬地下——但它們稀疏散佈在深層石頭帶，腳邊亂開井
/// 多半撲空，呼叫端對礦石（`voxel_skills::resource_is_ore`）改走**定向礦井**：先
/// `find_nearest_ore_excl` 找最近一顆礦、再 `plan_ore_well` 反推井口讓井恰好穿過它。
pub fn resource_is_underground(res: GatherResource) -> bool {
    matches!(
        res,
        GatherResource::Stone
            | GatherResource::Dirt
            | GatherResource::CoalOre
            | GatherResource::IronOre
    )
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

/// 開始一爐冶煉：扣除配方所需生料，**不產出成品**——成品要等煨煮熟成才交付
/// （見 `StepAction::CollectSmelt`／[`InventRun::smelt_wait`]）。配料夠 → 扣除、回 `true`；
/// 不夠 → 不動、回 `false`。純函式（吃 &mut HashMap）、可測；呼叫端在 res_inv 寫鎖內用它。
pub fn smelt_start_apply(bag: &mut HashMap<u8, u32>, recipe: &vcraft::Recipe) -> bool {
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
        .map(|(bid, n)| format!("{}×{}", material_name(*bid), n))
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
    start_furnace_nearby: bool,
) -> Result<(), String> {
    let mut bag = start_bag.clone();
    let mut wb_nearby = start_wb_nearby;
    let mut furnace_nearby = start_furnace_nearby;
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
                if *block_id == FURNACE_BLOCK_ID {
                    furnace_nearby = true;
                }
            }
            CheckedStep::Smelt { recipe_id } => {
                let r = vcraft::find_furnace_recipe(recipe_id)
                    .ok_or_else(|| format!("熔爐配方 {recipe_id} 不存在"))?;
                // 依賴順序：冶煉前必須已有熔爐在旁（本來就有、或計畫前段合出來放好）。
                if !furnace_nearby {
                    return Err(format!(
                        "「{}」是熔爐配方，必須先有熔爐在你旁邊——熔爐可用配方 id\
                        「furnace_wb」（石頭×8，需工作台）合成，再用 place 放置，然後才能 smelt",
                        r.name_zh
                    ));
                }
                // 冶煉需要煨煮時間（真實世界由 smelt_wait 倒數），但計畫可行性模擬只看
                // 材料流動是否走得通——時間到了終究會產出，故此處視同即時（idealized）。
                if !craft_apply(&mut bag, r) {
                    return Err(craft_shortage_err(r));
                }
            }
            CheckedStep::Fish { count } => {
                // 後置條件語意（同 Gather）：這一步結束時背包至少有 count 條小魚。
                // 水源可得性不在模擬階段判定（同礦石——執行期才真的去找，找不到才誠實失敗）。
                let e = bag.entry(FISH_ID).or_insert(0);
                if *e < *count {
                    *e = *count;
                }
            }
            CheckedStep::Harvest { crop, count } => {
                // 後置條件語意（同 Fish）：這一步結束時背包至少有 count 份該作物生料。
                // 熟作物可得性不在模擬階段判定（執行期才真的去找，找不到才誠實失敗）。
                let e = bag.entry(crop.raw_id()).or_insert(0);
                if *e < *count {
                    *e = *count;
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
        GatherResource::CoalOre => "coal_ore",
        GatherResource::IronOre => "iron_ore",
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
    let mut furnace_placed = false; // 模擬世界：從「身邊沒熔爐」開始（第四刀）
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
                    ensure_craftable(r, &mut bag, &mut out);
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
                    ensure_craftable(r, &mut bag, &mut out);
                }
                out.push(PrimStep::CraftWb { recipe: recipe_id.to_string() });
            }
            CheckedStep::Place { block_id } => {
                if *block_id == FURNACE_BLOCK_ID {
                    // 熔爐只能在工作台合成（3×3 furnace_wb），不像工作台本身可 2×2 隨手合——
                    // 專屬巢狀備料（見 ensure_furnace_ready），別走通用 ensure_have（找不到會靜默落空）。
                    ensure_furnace_ready(&mut bag, &mut out, &mut wb_placed);
                } else {
                    ensure_have(*block_id, 1, &mut bag, &mut out, ENSURE_MAX_DEPTH);
                }
                out.push(PrimStep::Place { block: place_token_of(*block_id).to_string() });
                let _ = take_one(&mut bag, *block_id);
                if *block_id == WORKBENCH_BLOCK_ID {
                    wb_placed = true;
                }
                if *block_id == FURNACE_BLOCK_ID {
                    furnace_placed = true;
                }
            }
            CheckedStep::Smelt { recipe_id } => {
                if let Some(r) = vcraft::find_furnace_recipe(recipe_id) {
                    if !furnace_placed {
                        ensure_furnace_ready(&mut bag, &mut out, &mut wb_placed);
                        out.push(PrimStep::Place {
                            block: place_token_of(FURNACE_BLOCK_ID).to_string(),
                        });
                        let _ = take_one(&mut bag, FURNACE_BLOCK_ID);
                        furnace_placed = true;
                    }
                    ensure_inputs(r, &mut bag, &mut out);
                    // 冶煉扣生料、產出記進模擬背包——現實中冶煉要等煨煮時間，但正規化階段
                    // 只算「材料流通不通」，理想化視同即時（時間到了終究會產出）。
                    let _ = craft_apply(&mut bag, r);
                }
                out.push(PrimStep::Smelt { recipe: recipe_id.to_string() });
            }
            CheckedStep::Fish { count } => {
                out.push(PrimStep::Fish { count: *count });
                let e = bag.entry(FISH_ID).or_insert(0);
                if *e < *count {
                    *e = *count;
                }
            }
            CheckedStep::Harvest { crop, count } => {
                out.push(PrimStep::Harvest { crop: crop.token().to_string(), count: *count });
                let e = bag.entry(crop.raw_id()).or_insert(0);
                if *e < *count {
                    *e = *count;
                }
            }
        }
    }
    out
}

/// 正規化輔助：確保模擬背包裡至少有 1 座熔爐——熔爐只能在工作台合成（3×3 `furnace_wb`），
/// 不像工作台本身可用 2×2 隨手合成，需要專屬的巢狀備料（先備妥＋放置工作台，
/// 再用工作台合出熔爐）。`wb_placed` 借用呼叫端的模擬世界狀態（副作用：確保後工作台
/// 視同已放置）。純函式、確定性、可測。
fn ensure_furnace_ready(bag: &mut HashMap<u8, u32>, out: &mut Vec<PrimStep>, wb_placed: &mut bool) {
    if bag.get(&FURNACE_BLOCK_ID).copied().unwrap_or(0) >= 1 {
        return; // 模擬背包已有熔爐（前段步驟備過）→ 不重複合
    }
    if !*wb_placed {
        ensure_have(WORKBENCH_BLOCK_ID, 1, bag, out, ENSURE_MAX_DEPTH);
        out.push(PrimStep::Place { block: place_token_of(WORKBENCH_BLOCK_ID).to_string() });
        let _ = take_one(bag, WORKBENCH_BLOCK_ID);
        *wb_placed = true;
    }
    if let Some(fr) = vcraft::find_workbench_recipe("furnace_wb") {
        ensure_craftable(fr, bag, out);
        out.push(PrimStep::CraftWb { recipe: fr.id.to_string() });
    }
}

/// [`ensure_have`] 的遞迴深度上限（鏈：木→木板→工作台，深度 3 就夠；留餘裕仍有界）。
const ENSURE_MAX_DEPTH: u8 = 4;

/// 正規化輔助：確保「這個配方的所有配料」在模擬背包裡足夠（缺的補備料步）。
fn ensure_inputs(r: &vcraft::Recipe, bag: &mut HashMap<u8, u32>, out: &mut Vec<PrimStep>) {
    for (bid, need) in r.inputs {
        ensure_have(*bid, *need, bag, out, ENSURE_MAX_DEPTH);
    }
}

/// 一次補料迴圈的上限（每輪至少補足一個缺口；鏈有界，8 輪綽綽有餘）。
const CRAFT_ENSURE_MAX_ROUNDS: u8 = 8;

/// 正規化輔助：確保這個配方**當下能一次合成**——反覆補齊配料直到 [`craft_apply`] 成功。
///
/// 為什麼要迴圈而非單趟 `ensure_inputs`（隔離實測 wood_pickaxe／wood_axe 驅動）：
/// [`ensure_inputs`] 依序確保每個配料，但備某個**加工配料**（木板＝木×2）會**吃掉**
/// 先前為另一個**原料配料**（木頭）備好的存量——木鎬要木×3＋木板×1，先備木×3、
/// 再備木板時削掉 2 木剩 1 木，最後 `craft_apply` 缺木失敗（原本被防禦性忽略，正規化後
/// 的計畫其實缺料、`simulate_plan` 才擋下 → 居民放棄發明）。原料被共用配料的合成吃掉
/// 是必然，補一輪後重驗、缺多少再補多少，直到能一次合成（有界防呆）。
/// 純函式、確定性、可測。
fn ensure_craftable(r: &vcraft::Recipe, bag: &mut HashMap<u8, u32>, out: &mut Vec<PrimStep>) {
    let mut round = 0;
    loop {
        ensure_inputs(r, bag, out);
        if craft_apply(bag, r) {
            return; // 配料到位、已扣料產出（模擬背包同步推進）
        }
        round += 1;
        if round >= CRAFT_ENSURE_MAX_ROUNDS {
            // 防禦：理論到不了（配料皆可自採／鏈上加工品）；補不動就停，
            // 交給 check_stored_steps／simulate_plan 把關（不硬塞、不 panic）。
            return;
        }
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
    // 小魚（第六刀）：不是 GatherResource（釣法不同軸），補「確保背包至少 need 條」的釣魚步。
    if bid == FISH_ID {
        let want = need.min(MAX_GATHER_COUNT);
        out.push(PrimStep::Fish { count: want });
        let e = bag.entry(bid).or_insert(0);
        if *e < want {
            *e = want;
        }
        return;
    }
    // 作物生料（第七刀）：不是 GatherResource（收成法不同軸），補「確保背包至少 need 份」的收成步。
    if let Some(crop) = crop_of_raw_id(bid) {
        let want = need.min(MAX_GATHER_COUNT);
        out.push(PrimStep::Harvest { crop: crop.token().to_string(), count: want });
        let e = bag.entry(bid).or_insert(0);
        if *e < want {
            *e = want;
        }
        return;
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

/// 水面視為「在附近、可拋竿」的水平搜尋半徑（格，第六刀）：比站點（4 格）寬得多——
/// 水是**天然地形**、不是她自己放的，河湖散佈各處，給更寬的鄰近範圍才貼近「附近有水
/// 就釣得成」的直覺。仍是**鄰近檢查**、不是移動去資源——找不到就誠實失敗（見模組頭註）。
pub const WATER_NEAR_RADIUS: i32 = 12;
/// 水面搜尋的垂直範圍（±格）：水面通常在地表附近，不必掃到天上或深地底。
pub const WATER_NEAR_YSPAN: i32 = 4;

/// 附近是否有水面可拋竿（第六刀）。掃居民腳邊一個小立方範圍的有效方塊（含 delta
/// overlay——填平/引水都算數），任一格是水（來源／流動／溫泉皆算，同垂釣既有判定精神）
/// 即回 `true`。純函式、可測。
pub fn water_nearby(world: &WorldDelta, fx: i32, fy: i32, fz: i32) -> bool {
    for dx in -WATER_NEAR_RADIUS..=WATER_NEAR_RADIUS {
        for dz in -WATER_NEAR_RADIUS..=WATER_NEAR_RADIUS {
            for dy in -WATER_NEAR_YSPAN..=WATER_NEAR_YSPAN {
                if voxel::effective_block_at(world, fx + dx, fy + dy, fz + dz).is_any_water() {
                    return true;
                }
            }
        }
    }
    false
}

/// 熟作物視為「在附近、可以直接收成」的水平搜尋半徑（格，第七刀）：同水面鄰近檢查
/// （12 格）同一個尺度——這些都是她**弄不出來、只能撞見**的資源（水是天然地形、
/// 熟作物多半是玩家種的），仍是鄰近檢查、不是移動去資源（見模組頭註）。
pub const CROP_NEAR_RADIUS: i32 = 12;
/// 熟作物搜尋的垂直範圍（±格）：田地在地表附近，不必掃到天上或深地底。
pub const CROP_NEAR_YSPAN: i32 = 4;

/// 附近是否有一畦指定作物已經熟了，熟在哪一格（第七刀）。掃居民腳邊一個小立方範圍的
/// 有效方塊（含 delta overlay），找到即回該格座標；沒有回 `None`（呼叫端誠實失敗，
/// 同水面鄰近檢查精神——不會替她跑去遠方找，更不會替她播種）。純函式、可測。
///
/// `blocked(x, z)`（review 修正，居民尊重玩家領地 v1 second pass）：某格是否不准挑——例如
/// 落在別人領地內。**必須推進迴圈裡**、逐格跳過繼續掃，不能讓呼叫端事後對回傳值 `.filter()`：
/// 那樣「最近一格恰好被擋」就會讓整次搜尋直接判定查無，即使旁邊還有一堆合法候選。
pub fn ripe_crop_nearby(
    world: &WorldDelta,
    fx: i32,
    fy: i32,
    fz: i32,
    want: Block,
    blocked: impl Fn(i32, i32) -> bool,
) -> Option<(i32, i32, i32)> {
    for dx in -CROP_NEAR_RADIUS..=CROP_NEAR_RADIUS {
        for dz in -CROP_NEAR_RADIUS..=CROP_NEAR_RADIUS {
            let (x, z) = (fx + dx, fz + dz);
            if blocked(x, z) {
                continue;
            }
            for dy in -CROP_NEAR_YSPAN..=CROP_NEAR_YSPAN {
                let y = fy + dy;
                if voxel::effective_block_at(world, x, y, z) == want {
                    return Some((x, y, z));
                }
            }
        }
    }
    None
}

/// 發明「移動去資源」的行走狀態（第八刀，接續第六/七刀）：`water_nearby`／`ripe_crop_nearby`
/// 只做**鄰近檢查**——附近沒有就誠實失敗，即便走遠一點其實找得到，她也不會去。第六刀模組頭註
/// 早就預告「那是移動去資源的下一刀」。本刀補上：鄰近檢查沒找到時，改用 [`find_water`]／
/// [`find_ripe_crop_far`] 在 [`INVENT_GATHER_RADIUS`] 範圍內再找一次——找到就設此狀態走過去，
/// 到位後回頭讓鄰近檢查再驗一次（這次會過）；仍找不到才是真的誠實失敗。純資料、無邏輯，
/// 走路本身由 `voxel_ws.rs` 比照 [`crate::voxel_skills::GatherSkill`] 的既有安全機制推進
/// （不挖、不種，純走路，逾時／走不到就放棄）。
#[derive(Clone, Debug, PartialEq)]
pub struct InventWalk {
    pub tx: i32,
    pub ty: i32,
    pub tz: i32,
    /// 走路逾時倒數（秒）：比照 [`crate::voxel_skills::GATHER_TIMEOUT_SECS`] 同一把尺——
    /// 找到的目標雖然可能遠達 `INVENT_GATHER_RADIUS`，但「最近一個」實務上多半近得多，
    /// 沿用既有日常採集走路的逾時預算即可，不必另開一個更寬鬆的常數。
    pub timeout: f32,
}

/// 螺旋向外找最近一格水面（第八刀·移動去資源），比 [`water_nearby`] 的鄰近檢查搜得遠——
/// 附近沒有水才值得呼叫這個，找到就走過去、找不到仍誠實失敗（不會無止盡漫遊）。純函式、可測。
pub fn find_water(
    world: &WorldDelta,
    ox: i32,
    oy: i32,
    oz: i32,
    max_radius: i32,
) -> Option<(i32, i32, i32)> {
    spiral_find(ox, oz, 0, max_radius, |x, z| {
        (-WATER_NEAR_YSPAN..=WATER_NEAR_YSPAN).find_map(|dy| {
            let y = oy + dy;
            voxel::effective_block_at(world, x, y, z)
                .is_any_water()
                .then_some((x, y, z))
        })
    })
}

/// 螺旋向外找最近一畦**指定種類**的熟作物（第八刀·移動去資源），比 [`ripe_crop_nearby`]
/// 的鄰近檢查搜得遠——鎖定「這一種」熟作物（她要湊的是特定作物的份數，不是隨便撿到什麼），
/// 找到就走過去、找不到仍誠實失敗（不替她播種，也不無止盡漫遊）。純函式、可測。
///
/// `blocked(x, z)`：見 [`ripe_crop_nearby`] 同款說明——擋在 `spiral_find` 的 pick 閉包裡，
/// 被擋的格子跟「這格沒熟」一視同仁，螺旋會自然繼續掃下一圈，不會讓整次搜尋提早放棄。
pub fn find_ripe_crop_far(
    world: &WorldDelta,
    ox: i32,
    oy: i32,
    oz: i32,
    max_radius: i32,
    want: Block,
    blocked: impl Fn(i32, i32) -> bool,
) -> Option<(i32, i32, i32)> {
    spiral_find(ox, oz, 0, max_radius, |x, z| {
        if blocked(x, z) {
            return None;
        }
        (-CROP_NEAR_YSPAN..=CROP_NEAR_YSPAN).find_map(|dy| {
            let y = oy + dy;
            (voxel::effective_block_at(world, x, y, z) == want).then_some((x, y, z))
        })
    })
}

/// 可翻土地表視為「在附近」的搜尋半徑（格，第九刀·播種自給）：同水面/熟作物鄰近檢查
/// 一樣的尺度——草皮/泥土隨處都是，找不到只代表附近異常（全被建物佔滿之類），不值得
/// 像找熟作物那樣再往遠處螺旋找，鄰近檢查落空就誠實放棄這次播種。
pub const TILL_NEAR_RADIUS: i32 = 12;
/// 翻土地表搜尋的垂直範圍（±格）：地表在腳邊附近，不必掃到天上或深地底。
pub const TILL_NEAR_YSPAN: i32 = 4;

/// 附近是否有可翻土播種的地表（草皮／泥土，依作物種類而定），在哪一格（第九刀·播種
/// 自給）。由近而遠螺旋掃（比照 [`find_water`]／[`find_ripe_crop_far`] 共用 [`spiral_find`]），
/// 且只認**真地表**——該格上方須為 `Air`（可直接種下、種下去玩家看得見），排除被草皮／
/// 岩層蓋住的深層泥土（審查點名：舊版三層迴圈固定順序掃到最遠角落 + 最深層先中，會把
/// 馬鈴薯種到地底）。找到即回該格座標；沒有回 `None`（呼叫端誠實放棄這次播種，不會替她
/// 跑去遠方找地——那不像找水找熟作物，到處都是草皮泥土，附近沒有多半代表地形異常，
/// 或單純還沒有裸露的泥土可種）。純函式、可測。
///
/// `blocked(x, z)`：見 [`ripe_crop_nearby`] 同款說明——這顆函式尤其重要，草皮／泥土到處
/// 都是，若在回傳值上事後過濾，居民只要站在別人領地邊上，螺旋第一格多半就命中領地內，
/// 播種自給整支會靜默失效，即使隔壁一格完全合法。
pub fn tillable_ground_nearby(
    world: &WorldDelta,
    fx: i32,
    fy: i32,
    fz: i32,
    want: Block,
    blocked: impl Fn(i32, i32) -> bool,
) -> Option<(i32, i32, i32)> {
    spiral_find(fx, fz, 0, TILL_NEAR_RADIUS, |x, z| {
        if blocked(x, z) {
            return None;
        }
        (-TILL_NEAR_YSPAN..=TILL_NEAR_YSPAN).find_map(|dy| {
            let y = fy + dy;
            let is_surface = voxel::effective_block_at(world, x, y, z) == want
                && voxel::effective_block_at(world, x, y + 1, z) == Block::Air;
            is_surface.then_some((x, y, z))
        })
    })
}

/// 扣一顆種子（播種消耗；第九刀）。先驗證真的有 ≥1 顆才扣，比照 [`smelt_start_apply`]
/// 「不部分扣、不留爛帳」的精神。不夠 → 不動、回 `false`。純函式、可測。
pub fn consume_seed(bag: &mut HashMap<u8, u32>, seed_id: u8) -> bool {
    match bag.get_mut(&seed_id) {
        Some(c) if *c > 0 => {
            *c -= 1;
            true
        }
        _ => false,
    }
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
    /// 技能來源（人口成長 v1·世代傳承）：`Some("諾娃")` 表示這是新生兒承繼自諾娃的技能；
    /// `None`＝自己發明的（或舊記錄，`#[serde(default)]` 向後相容：舊檔沒此欄位載回即 None）。
    #[serde(default)]
    pub source: Option<String>,
    /// 師承標記（技能互教·北極星第四刀）：`true`＝這筆是**在世居民之間口耳相傳學來的**
    /// （`source`＝老師名，顯示「師承XX」）；`false`＝沿用舊語意（`source` 有值＝出生時
    /// 承自父母「承自XX」，`None`＝自己發明）。`#[serde(default)]` 向後相容：
    /// 舊檔沒此欄位載回即 false，既有親子繼承記錄語意不變。
    #[serde(default)]
    pub taught: bool,
}

/// 一筆技能的「來歷」標籤（師承鏈可見·北極星第四刀）：三種來源攤開給人看——
/// 自己發明／出生承自父母／在世時師承老師。技能簿 API 與面板用它呈現村裡的知識系譜。
pub fn lineage_label(rec: &InventedSkillRecord) -> String {
    match (&rec.source, rec.taught) {
        (None, _) => "自己發明".to_string(),
        (Some(n), true) => format!("師承{n}"),
        (Some(n), false) => format!("承自{n}"),
    }
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
            source: None,
            taught: false,
        };
        self.next_seq = self.next_seq.wrapping_add(1);
        self.skills.push(rec.clone());
        Some(rec)
    }

    /// 某位居民可傳給下一代的技能清單（人口成長 v1·世代傳承）：她自己會的全部技能。
    /// 回傳 clone 快照供呼叫端在鎖外挑選 1~2 個給新生兒；順序即技能庫既有順序（確定性）。
    pub fn inheritable_for(&self, parent: &str) -> Vec<InventedSkillRecord> {
        self.skills
            .iter()
            .filter(|k| k.resident == parent)
            .cloned()
            .collect()
    }

    /// 讓 `child` 繼承一筆前輩技能（人口成長 v1·世代傳承）：把 `from` 的原語序列複製到
    /// 新生兒名下，`source` 標成父母名——她一出生就會做這件事（零 LLM 重用照舊）。
    /// 同處境（goal_block）child 已有技能 → 不重複繼承（回 `None`）。回傳 record 供落地。
    pub fn inherit(&mut self, child: &str, from: &InventedSkillRecord, parent_name: &str) -> Option<InventedSkillRecord> {
        if self.find_for(child, from.goal_block).is_some() {
            return None;
        }
        let rec = InventedSkillRecord {
            resident: child.to_string(),
            name: from.name.clone(),
            goal_block: from.goal_block,
            steps: from.steps.clone(),
            seq: self.next_seq,
            source: Some(parent_name.to_string()),
            taught: false,
        };
        self.next_seq = self.next_seq.wrapping_add(1);
        self.skills.push(rec.clone());
        Some(rec)
    }

    /// 讓 `student` 從老師手上**活著學會**一筆技能（技能互教·北極星第四刀）：把 `from` 的
    /// 原語序列複製到學生名下，`source` 標成老師名、`taught` 標 true——顯示成「師承XX」，
    /// 與出生時的「承自XX」（親子）並行成兩條知識傳承的路。學到之後零 LLM 重用照舊
    /// （`find_for` 查得到、`check_stored_steps` 驗過就能執行），也能再往下教（師承鏈一節一節）。
    /// 同處境（goal_block）student 已有技能 → 不重複學（回 `None`）。回傳 record 供落地。
    pub fn learn_from(
        &mut self,
        student: &str,
        from: &InventedSkillRecord,
        teacher_name: &str,
    ) -> Option<InventedSkillRecord> {
        if self.find_for(student, from.goal_block).is_some() {
            return None;
        }
        let rec = InventedSkillRecord {
            resident: student.to_string(),
            name: from.name.clone(),
            goal_block: from.goal_block,
            steps: from.steps.clone(),
            seq: self.next_seq,
            source: Some(teacher_name.to_string()),
            taught: true,
        };
        self.next_seq = self.next_seq.wrapping_add(1);
        self.skills.push(rec.clone());
        Some(rec)
    }

    /// 這位居民技能庫的完整記錄快照（師承鏈可見·北極星第四刀）：技能簿 API 用它連同
    /// 來歷（[`lineage_label`]）一起攤開。順序同技能庫既有順序（與 `names_for` 對齊）。
    pub fn records_for(&self, resident: &str) -> Vec<&InventedSkillRecord> {
        self.skills.iter().filter(|k| k.resident == resident).collect()
    }

    /// 全村技能庫的唯讀快照（名匠聲望 v1·ROADMAP 888）：聲望要橫掃所有居民的
    /// 發明與師承紀錄才算得出來，故給一支全量存取點（短讀鎖內取用即釋，不外洩內部結構）。
    pub fn all(&self) -> &[InventedSkillRecord] {
        &self.skills
    }

    /// 這位居民已會技能的**目標材料 id 集合**（好奇心第三刀：可能性目錄
    /// 「排掉她已會的」用——會了的東西不再列進「世界上還能學什麼」）。
    pub fn known_goals_for(&self, resident: &str) -> HashSet<u8> {
        self.skills
            .iter()
            .filter(|k| k.resident == resident)
            .map(|k| k.goal_block)
            .collect()
    }

    /// 這位居民已學會的技能名清單（對話時可以自豪地講出來）。
    pub fn names_for(&self, resident: &str) -> Vec<String> {
        self.skills
            .iter()
            .filter(|k| k.resident == resident)
            .map(|k| k.name.clone())
            .collect()
    }

    /// 這位居民自己會的技能，`(名字, 原語序列)` 清單（第三刀·技能組合技能用）：
    /// 供 `expand_step` 展開她計畫裡的 `use_skill` 步驟、供 [`invention_prompt`]
    /// 列出她能引用的名字。回傳 clone 快照，供呼叫端在鎖外使用。
    pub fn known_steps_for(&self, resident: &str) -> Vec<(String, Vec<PrimStep>)> {
        self.skills
            .iter()
            .filter(|k| k.resident == resident)
            .map(|k| (k.name.clone(), k.steps.clone()))
            .collect()
    }

    /// 這位居民**自己發明**的技能筆數（第一次發明立碑 v1·ROADMAP 930）：只數
    /// `source.is_none()` 的記錄——出生承自父母（`承自XX`）或在世師承老師（`師承XX`）
    /// 的技能都有 `source`，不算她「自己想出來的」。呼叫端用它判定「這是不是她生涯
    /// 第一次真的靠自己發明」，是則就地立起發明之光碑。
    pub fn self_invented_count(&self, resident: &str) -> usize {
        self.skills
            .iter()
            .filter(|k| k.resident == resident && k.source.is_none())
            .count()
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
        .map(|(bid, n)| format!("{}×{}", material_name(*bid), n))
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
    furnace_nearby: bool,
    known_skill_names: &[String],
) -> (String, String) {
    // 配方節錄：只列可發明配方（配料她弄得到的），每條含 id 與配料事實。
    let recipe_lines: Vec<String> = inventable_recipes().map(recipe_fact).collect();
    let wb_recipe_lines: Vec<String> = inventable_wb_recipes().map(recipe_fact).collect();
    let furnace_recipe_lines: Vec<String> = inventable_furnace_recipes().map(recipe_fact).collect();
    // 第三刀·技能組合技能：她自己已經會的技能可以直接引用當一步，省得每次重新拆解。
    // 沒學過任何技能（多半是新生兒或還沒發明成功過）就不提這個 op，避免她引用不存在的名字。
    let use_skill_line = if known_skill_names.is_empty() {
        String::new()
    } else {
        format!(
            "8. 使用已學會的技能：{{\"op\":\"use_skill\",\"name\":\"<你已經會的技能名>\"}}——\
            把你之前發明過、已經會的技能整段當一步用，不必重新拆解成原語。\n\
            你已經學會的技能（事實，只能引用這些名字，不可捏造）：{}\n",
            known_skill_names.join("、"),
        )
    };
    // 「優先用 use_skill 引用」這句提示只在她真的有技能可引用時才講，
    // 否則會叫她用一個系統根本沒教過她的 op（觸發沒學過技能就提 use_skill 的誤導）。
    let use_skill_wb_hint = if known_skill_names.is_empty() {
        ""
    } else {
        "若你已經會做工作台/熔爐這件事，優先用 use_skill 引用，別重新拆解。"
    };
    // 工作台／熔爐的教學要看「目標材料是不是它本身」而分岔：
    // 若目標就是工作台/熔爐這個材料，做到 craft/craft_wb 那步、東西已經在背包裡就是達成了，
    // **不該再 place**——place 會把它放到地上、從背包消耗掉，反而讓背包裡沒有目標材料，
    // 白費一趟（這正是修這支 prompt 前，居民想要「熔爐」卻總是失敗的根因：她照著
    // 「拿它當中繼站」的教法多做一步 place，結果目標材料反而從背包裡消失）。
    // 只有當工作台/熔爐是**拿來當中繼站**（要接著 craft_wb / smelt 別的東西）時，才需要放置。
    let workbench_hint_line = if goal.block_id == WORKBENCH_BLOCK_ID {
        "- 你的目標材料正是「工作台」本身：先 craft plank（木頭×2→木板×4）、\
        再 craft workbench（木板×4→工作台），做到這裡東西已經在背包裡，就算達成目標了——\
        **先別 place**，place 會把它放到地上、從背包裡消耗掉，反而達不成目標。\n"
            .to_string()
    } else {
        "- 工作台本身的正確做法：先 craft plank（木頭×2→木板×4）、再 craft workbench\
        （木板×4→工作台）、再 place 放置到腳邊，才能接著用 craft_wb 做 3×3 合成；\
        若你附近已經有工作台，就不必再做一個、直接 craft_wb。\n"
            .to_string()
    };
    let furnace_hint_line = if goal.block_id == FURNACE_BLOCK_ID {
        "- 你的目標材料正是「熔爐」本身：先備妥工作台（同上）、再 craft_wb furnace_wb\
        （石頭×8→熔爐，需工作台），做到這裡東西已經在背包裡，就算達成目標了——\
        **先別 place**，place 會把它放到地上、從背包裡消耗掉，反而達不成目標。\n"
            .to_string()
    } else {
        "- 熔爐本身的正確做法：先備妥工作台（同上）、再 craft_wb furnace_wb\
        （石頭×8→熔爐，需工作台）、再 place 放置到腳邊，才能接著 smelt；\
        若你附近已經有熔爐，就不必再做一個、直接 smelt。\n"
            .to_string()
    };
    let system = format!(
        "你是{resident_name}，乙太方界的居民。你要自己想辦法解決一個處境：把你會的基礎動作\
        組合成一個新技能。你只會這幾種基礎動作（原語）：\n\
        1. 採集：{{\"op\":\"gather\",\"resource\":\"<資源>\",\"count\":<數量1~{max_c}>}}，\
        resource 只能是 grass / sand / dirt / stone / wood / coal_ore / iron_ore\
        （coal_ore=煤礦、iron_ore=鐵礦，埋在深層，你會自己鑿井挖到；礦石數量保守拿捏）。\n\
        2. 隨身合成（2×2）：{{\"op\":\"craft\",\"recipe\":\"<配方id>\"}}。\n\
        3. 工作台合成（3×3）：{{\"op\":\"craft_wb\",\"recipe\":\"<配方id>\"}}——\
        **必須先有工作台放在你旁邊**才能執行。\n\
        4. 放置：{{\"op\":\"place\",\"block\":\"workbench\"或\"furnace\"}}——把背包裡的工作台\
        或熔爐放到腳邊（會消耗背包裡那一個）。\n\
        5. 熔爐冶煉：{{\"op\":\"smelt\",\"recipe\":\"<配方id>\"}}——\
        **必須先有熔爐放在你旁邊**才能執行；冶煉需要時間慢慢煨煮，不是立刻拿到成品，\
        放心，煨好了自然會收到。\n\
        6. 釣魚：{{\"op\":\"fish\",\"count\":<數量1~{max_c}>}}——**必須附近有水**才能執行；\
        釣魚跟冶煉一樣要等上鉤，不是拋竿立刻有魚，放心，上鉤了自然會收到，釣起的是生小魚。\n\
        7. 收成：{{\"op\":\"harvest\",\"crop\":\"<作物>\",\"count\":<數量1~{max_c}>}}，\
        crop 只能是 wheat / carrot / potato / berry（小麥/胡蘿蔔/馬鈴薯/莓果）——**必須附近\
        有一畦這種作物（或一叢已結果的莓果叢）已經熟了**才能執行；你不會種田播種，只能收成\
        已經熟在那裡的作物，收成的是生料（不是麵包/烤地薯/莓果醬——那些還要再合成/冶煉）。\n\
        {use_skill_line}\
        你知道的隨身合成配方（事實，不可捏造別的）：\n{recipes}\n\
        你知道的工作台配方（要先有工作台在旁邊，才能用 craft_wb 做這些）：\n{wb_recipes}\n\
        你知道的熔爐冶煉配方（要先有熔爐在旁邊，才能用 smelt 做這些）：\n{furnace_recipes}\n\
        注意：\n\
        - 合成／冶煉會**消耗**配料——採集步驟的數量必須足以支付後續所有合成/冶煉所需的配料\
        （例如要合成需要木頭×2的配方，前面就得先採集至少 2 個木頭）。\n\
        - craft 只能用「隨身合成配方」清單裡的 id；craft_wb 只能用「工作台配方」清單裡的 id；\
        smelt 只能用「熔爐冶煉配方」清單裡的 id——三張清單不可混用\
        （workbench 在隨身清單，要用 craft 做）。\n\
        {workbench_hint}\
        {furnace_hint}{use_skill_wb_hint}\n\
        請只輸出一個 JSON 物件（不要任何其他文字或說明）：\n\
        {{\"name\":\"<你給這個技能取的名字，繁體中文，最多{max_n}字>\",\"steps\":[<原語序列，最多{max_s}步>]}}",
        max_c = MAX_GATHER_COUNT,
        max_n = SKILL_NAME_MAX_CHARS,
        max_s = MAX_STEPS,
        recipes = recipe_lines.join("\n"),
        wb_recipes = wb_recipe_lines.join("\n"),
        furnace_recipes = furnace_recipe_lines.join("\n"),
        workbench_hint = workbench_hint_line,
        furnace_hint = furnace_hint_line,
    );
    let user = format!(
        "處境：你心裡想著「{desire}」，想要「{goal}」這種材料，但你的背包裡沒有。\
        你的背包現況：{bag}。你附近{wb}、{furnace}。\
        請用你的原語組合出一個能讓背包裡出現「{goal}」的步驟計畫。",
        goal = goal.name_zh,
        bag = if bag_note.is_empty() { "空的" } else { bag_note },
        wb = if wb_nearby { "已經有一座放置好的工作台" } else { "沒有工作台" },
        furnace = if furnace_nearby { "已經有一座放置好的熔爐" } else { "沒有熔爐" },
    );
    (system, user)
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
            CheckedStep::Smelt { recipe_id } => {
                let name = vcraft::find_furnace_recipe(recipe_id)
                    .map(|r| r.name_zh)
                    .unwrap_or("？");
                format!("熔爐冶煉{name}")
            }
            CheckedStep::Fish { count } => format!("釣魚×{count}"),
            CheckedStep::Harvest { crop, count } => format!("收成{}×{count}", crop.display_name()),
        })
        .collect::<Vec<_>>()
        .join("→")
}

/// 放好站點方塊的冒泡（放置原語完成那一刻——玩家看得到「她把工作台擺出來了」）。
pub fn placed_line(block_name: &str) -> String {
    format!("我把{block_name}放好了！")
}

/// 開爐冶煉那一刻的冒泡（第四刀——她把生料放進熔爐，靜候熟成）。
pub fn smelting_started_line(recipe_name: &str) -> String {
    format!("把材料放進熔爐開始冶煉{recipe_name}了，先去忙別的，等它煨好～")
}

/// 冶煉熟成、收成那一刻的冒泡（第四刀）。
pub fn smelting_done_line(recipe_name: &str) -> String {
    format!("熔爐煨好{recipe_name}了！")
}

/// 拋竿那一刻的冒泡（第六刀——她找到附近的水面，靜候上鉤）。
pub fn fishing_started_line() -> String {
    "這附近有水，我拋竿釣魚看看～".to_string()
}

/// 收竿、釣起一條那一刻的冒泡（第六刀）。
pub fn fishing_done_line() -> String {
    "上鉤了！釣到一條小魚～".to_string()
}

/// 附近找不到水，釣魚步誠實失敗的 Feed 詳情（第六刀，比照 `backoff_no_resource_feed`）。
pub fn no_water_feed(goal_name: &str) -> String {
    format!("為了做出{goal_name}想找水釣魚，但附近找不到水，這次沒能成功")
}

/// 收成一畦附近熟作物那一刻的冒泡（第七刀——她撞見一畦別人種熟的作物，順手收進背包）。
pub fn harvest_line(crop_name: &str) -> String {
    format!("這附近有熟了的{crop_name}，我收一點～")
}

/// 附近找不到熟作物，收成步誠實失敗的 Feed 詳情（第七刀，比照 `no_water_feed`）。
pub fn no_crop_feed(goal_name: &str, crop_name: &str) -> String {
    format!("為了做出{goal_name}想收一點{crop_name}，但附近找不到熟了的{crop_name}，這次沒能成功")
}

/// 就地翻土播種那一刻的冒泡（第九刀·播種自給——附近沒有熟作物，但她手邊有種子）。
pub fn planted_seed_line(crop_name: &str) -> String {
    format!("附近沒有熟的{crop_name}，我先翻土把種子種下去，等它長吧～")
}

/// 播種那一刻的 Feed 詳情（第九刀）：誠實標明這次仍未收成，只是為將來鋪路。
pub fn planted_seed_feed(goal_name: &str, crop_name: &str) -> String {
    format!(
        "為了做出{goal_name}想收一點{crop_name}，附近沒有熟的，於是就地翻土播下一顆{crop_name}種子——這次還沒能成功，但田已經種下去了"
    )
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

/// 定向礦井開挖的 Feed 詳情（鑿井尋礦 v1；只在該次發明的**第一口**礦井播，不洗版）。
/// 這一幕是「鐵的時代」開場——世界第一次看到有居民為了心裡的計畫往地底去。
pub fn ore_well_feed(goal_name: &str, ore_name: &str) -> String {
    format!("為了做出{goal_name}，朝著地底的{ore_name}鑿下一口階梯礦井——邊挖邊留階，挖得到也回得來")
}

/// 定向礦井開挖的頭頂冒泡（同上，第一口才冒）。
pub fn ore_well_line(ore_name: &str) -> String {
    format!("{ore_name}在地底下……我挖階梯下去找！")
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

// ── 好奇心自主學習（北極星第三刀）──────────────────────────────────────────────
//
// 維護者實測回饋：「不 push 他好像無法學習？進化那個好像還沒成功？」——發明引擎
// 技術上成功過，但**有機自發幾乎不會發生**：(1) 心願腦對配方世界一無所知，許的願
// 全是詩意句、不含可合成材料；(2) 發明觸發＝心願含可合成材料，條件幾乎不自然成立；
// (3) 她們整天忙採集蓋家。本段補上兩塊拼圖，讓她們**不用玩家 push 也會自己成長**：
//
// 1. **可能性目錄**（知識，不是技能包）：世界上「做得出的東西」清單，注入自主思考／
//    許願 prompt——引導她許「做得到的願」。會不會做仍要她自己發明（存進技能庫才算會）。
// 2. **好奇心迴圈**：閒置居民低頻（每位獨立計時＋機率門檻）自發「想試做一樣新東西」，
//    直接種下含材料名的心願（sparked_by=好奇心）→ 既有發明引擎自然接手。
//
// 全部純函式；鎖／計時遞減／Feed／廣播在 voxel_ws 呼叫端。

/// 好奇心週期基準（秒）：每位居民獨立倒數，到期＋閒置＋過機率門檻才「好奇一下」。
/// 12 分鐘＝夠低頻（發明本身另有 [`INVENT_COOLDOWN_SECS`] 冷卻與防重入），成本有界。
pub const CURIOSITY_INTERVAL_SECS: f32 = 720.0;
/// 好奇心機率門檻：計時到期時擲一次亂數，小於此值才真的好奇——
/// 不機械準點，平均約每 1~2 個週期好奇一次，更像生活。
pub const CURIOSITY_CHANCE: f64 = 0.6;
/// 可能性目錄注入 prompt 時最多列幾樣（防 prompt 膨脹；好奇心挑選仍看整份目錄）。
pub const CATALOG_NOTE_MAX_ITEMS: usize = 10;

/// 好奇心週期（秒）：預設 [`CURIOSITY_INTERVAL_SECS`]；隔離實測可設
/// `BUTFUN_CURIOSITY_SECS` 縮短觀察全鏈（prod 不設，走預設低頻）。
pub fn curiosity_base_secs() -> f32 {
    std::env::var("BUTFUN_CURIOSITY_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<f32>().ok())
        .filter(|v| *v > 0.0)
        .unwrap_or(CURIOSITY_INTERVAL_SECS)
}

/// 第 `idx` 位居民的好奇心計時初值：基準 × (1 + idx×0.25)——全員錯開、不同 tick
/// 一起好奇；**比例式**錯開讓測試模式縮短基準時錯開間距同步縮短。純函式、可測。
pub fn curiosity_interval_for(idx: usize, base: f32) -> f32 {
    base * (1.0 + idx as f32 * 0.25)
}

/// 機率門檻判定（比照 `npc_agent::should_pray` 把亂數來源分離出去，邊界可測）。
pub fn curiosity_gate(roll: f64) -> bool {
    roll < CURIOSITY_CHANCE
}

/// 好奇心的閒置判定：沒在發明／跑腿、沒有進行中的建造計畫、發明不在冷卻——
/// 這種時候起好奇才不打斷正事，也保證發明引擎接手時立刻能動。純函式、可測。
pub fn curiosity_idle(
    has_invent_run: bool,
    has_fetch: bool,
    has_build_plan: bool,
    invent_cooldown: f32,
) -> bool {
    !has_invent_run && !has_fetch && !has_build_plan && invent_cooldown <= 0.0
}

/// **可能性目錄**（知識，不是技能包）：世界上「她真的有路子自己做出來」的東西——
/// 兩張配方表中**可發明**配方的產物，去重、排掉 `excluded`（技能庫已會的目標；
/// 呼叫端也可把「背包已有的」併進來——有了就不必好奇）。
/// 確定性（照配方表順序）、純函式、可測。會不會做仍要她自己發明。
pub fn possibility_catalog(excluded: &HashSet<u8>) -> Vec<MaterialGoal> {
    let mut seen: HashSet<u8> = HashSet::new();
    let mut out = Vec::new();
    // 熔爐產物（拋光石/鐵錠/烤魚…）也要進目錄——第四刀起就可發明卻漏了 chain，
    // 第六刀（漁獲入自採閉包）唯一玩家可見產出正是熔爐產物，這個既有缺口第一次讓
    // 「居民自己想釣魚」完全不會發生（好奇心目錄裡根本沒有烤魚可挑）。
    for r in inventable_recipes().chain(inventable_wb_recipes()).chain(inventable_furnace_recipes()) {
        if excluded.contains(&r.output_block) || !seen.insert(r.output_block) {
            continue;
        }
        out.push(MaterialGoal { block_id: r.output_block, name_zh: r.name_zh });
    }
    out
}

/// 目錄 → 自主思考／許願 prompt 的「世界可能性」注入段。目錄空（能學的全會了）
/// 回 `None`（不注入、不多花 token）。**只進 think/pray 路徑，不進 talk**（成本紀律）。
pub fn catalog_note(catalog: &[MaterialGoal]) -> Option<String> {
    if catalog.is_empty() {
        return None;
    }
    let names: Vec<&str> =
        catalog.iter().take(CATALOG_NOTE_MAX_ITEMS).map(|g| g.name_zh).collect();
    Some(format!(
        "聽說這個世界上做得出這些東西（你還不會做）：{}。\
        你若有嚮往，可以許個「想要某樣東西」的願——之後你會自己想辦法把它做出來。",
        names.join("、")
    ))
}

/// 從目錄**確定性**挑一樣（seed 取模；呼叫端用位置 bits 等當種子，錯開又可重現）。
/// 目錄空回 `None`（她全會了）。
pub fn curiosity_pick(catalog: &[MaterialGoal], seed: u64) -> Option<MaterialGoal> {
    if catalog.is_empty() {
        None
    } else {
        Some(catalog[(seed % catalog.len() as u64) as usize])
    }
}

// ── 階梯式好奇心（真進化·踏腳石）─────────────────────────────────────────────
// 線上日誌實證：`curiosity_pick` 從整份目錄**均勻亂挑**，居民常挑到「石劍／熔爐鏈深處」
// 這種前置技能還沒攢夠、單一計畫塞不進 MAX_STEPS 的目標，一天到晚提案不可行、退避冷卻、
// 猛撞牆。真進化該像**階梯**：先發明搆得著的踏腳石，用學到的技能當積木，複雜目標等她攢夠
// 前置後自然變得搆得著。下面用一支純函式估「這個目標她現在搆不搆得著」，讓好奇心優先挑
// 「當前踏得到的一階」——玩家因此看得到居民能力**像階梯一樣往上長**。

/// 「一個原語 op 就到手」的葉節點資源 id 集合（單一事實源：`obtainable_ids()` 的閉包
/// 種子與階梯好奇心的 [`base_resource_ids`] 都從此出發，別讓兩邊分岔——第六刀（漁獲入
/// 自採閉包）曾只補了前者、漏了後者，導致烤魚被階梯過濾器誤判「弄不到」）。
/// 可自採資源（草/沙/土/石/木＋煤礦/鐵礦，`GatherResource::ALL`）＋小魚（`Fish` 原語，
/// 釣法與挖方塊不同軸，故不進 `GatherResource::ALL`，改在此直接補一步到手的葉節點）＋
/// 小麥/胡蘿蔔/馬鈴薯生料（第七刀 `Harvest` 原語，收成法也不同軸，同理直接補葉節點；
/// 南瓜刻意不含，見模組頭註）。
fn one_step_resource_ids() -> &'static HashSet<u8> {
    static SET: OnceLock<HashSet<u8>> = OnceLock::new();
    SET.get_or_init(|| {
        let mut set: HashSet<u8> = GatherResource::ALL.iter().map(|r| r.block_id()).collect();
        set.insert(FISH_ID);
        set.insert(vfarm::WHEAT_ID);
        set.insert(vfarm::CARROT_ID);
        set.insert(vfarm::POTATO_ID);
        // 第十刀：莓果（已結果的莓果叢收成的產物）也是一步到手的葉節點。
        set.insert(vberry::BERRY_ID);
        set
    })
}

/// 可自採的基礎資源 block id 集合——階梯好奇心估算成本用的「一步到手」葉節點。
/// 與 `obtainable_ids()` 共用同一份種子集（見 [`one_step_resource_ids`]）。
fn base_resource_ids() -> &'static HashSet<u8> {
    one_step_resource_ids()
}

/// 某產物需要的站台層級（None＝隨身 2×2／工作台／熔爐）。
#[derive(Clone, Copy, PartialEq)]
enum StationTier {
    None,
    Workbench,
    Furnace,
}

/// 找「產出某 block」的可發明配方 + 它需要的站台。依「隨身→工作台→熔爐」順序挑第一條
/// （同一產物優先較省事的做法，估算成本時取樂觀的那條）；查無回 `None`（弄不到）。
fn producing_recipe(block: u8) -> Option<(&'static vcraft::Recipe, StationTier)> {
    if let Some(r) = inventable_recipes().find(|r| r.output_block == block) {
        return Some((r, StationTier::None));
    }
    if let Some(r) = inventable_wb_recipes().find(|r| r.output_block == block) {
        return Some((r, StationTier::Workbench));
    }
    if let Some(r) = inventable_furnace_recipes().find(|r| r.output_block == block) {
        return Some((r, StationTier::Furnace));
    }
    None
}

/// 遞迴估「把某材料弄進背包」需要多少個原語 op（對齊 LLM 提案的 raw op 數）。
/// - 她已學會技能能產出的 → 1（use_skill 整條子鏈收成一步：這正是階梯往上長的積木）
/// - 基礎可採資源 → 1（一個 gather op；數量由該步的 count 欄位涵蓋）
/// - 否則查可發明配方：1（craft op）+ 各配料成本 +（若需站台）造站台成本 + 1（place）
/// - 弄不到／遞迴超深／路徑有環 → `None`
/// 刻意略保守（配料去重但共用中繼品重複計、站台每遇一次算一次）——寧可低估可及性，
/// 也不挑到執行必敗的目標；真的高估了退避機制仍會止血。
fn material_op_cost(
    block: u8,
    known_goals: &HashSet<u8>,
    visited: &mut HashSet<u8>,
    depth: u32,
) -> Option<usize> {
    const DEPTH_CAP: u32 = 12;
    if depth > DEPTH_CAP {
        return None;
    }
    if known_goals.contains(&block) {
        return Some(1); // 她已經會——use_skill 一步搞定整條子鏈
    }
    if base_resource_ids().contains(&block) {
        return Some(1); // 採一步就到手
    }
    if !visited.insert(block) {
        return None; // 環：這條路不通（防禦，靜態配方表理論無環）
    }
    let result = (|| {
        let (r, tier) = producing_recipe(block)?;
        let mut c = 1usize; // 這一步 craft／craft_wb／smelt
        // 各配料去重：同一種料只算一次它的產出成本（數量由採集 count 一步涵蓋）。
        let mut seen_inputs: HashSet<u8> = HashSet::new();
        for (bid, _) in r.inputs {
            if !seen_inputs.insert(*bid) {
                continue;
            }
            c += material_op_cost(*bid, known_goals, visited, depth + 1)?;
        }
        // 站台：需要工作台/熔爐而她還沒把它收成技能 → 造一座 + 放置（place 一步）。
        match tier {
            StationTier::None => {}
            StationTier::Workbench => {
                c += material_op_cost(WORKBENCH_BLOCK_ID, known_goals, visited, depth + 1)? + 1;
            }
            StationTier::Furnace => {
                c += material_op_cost(FURNACE_BLOCK_ID, known_goals, visited, depth + 1)? + 1;
            }
        }
        Some(c)
    })();
    visited.remove(&block);
    result
}

/// 估「這個目標她現在搆不搆得著」：估出目標材料的原語 op 成本。`None`＝弄不到/太迂迴；
/// `Some(n)` 且 `n ≤ MAX_STEPS` 即「當前踏得到的一階」。`known_goals`＝她已發明技能能
/// 產出的 block（這些收成 1 步 use_skill，正是往上搆的積木）。
pub fn goal_reach_cost(goal_block: u8, known_goals: &HashSet<u8>) -> Option<usize> {
    let mut visited = HashSet::new();
    material_op_cost(goal_block, known_goals, &mut visited, 0)
}

/// 階梯式好奇心挑目標（取代整份目錄均勻亂挑）：**優先從「當前搆得著」的子集挑**
/// （估算 op 數 ≤ [`MAX_STEPS`]），先發明踏得到的踏腳石；複雜目標等她攢夠前置技能後
/// 自然浮現。reachable 子集非空 → 從中 seed 取模挑（保留好奇心的探索、可重現）；
/// 全都搆不著（理論罕見：木板/石磚這種 2 步的一定在）→ 退回原均勻挑（絕不卡死、零回歸）。
pub fn curiosity_pick_laddered(
    catalog: &[MaterialGoal],
    known_goals: &HashSet<u8>,
    seed: u64,
) -> Option<MaterialGoal> {
    if catalog.is_empty() {
        return None;
    }
    let reachable: Vec<MaterialGoal> = catalog
        .iter()
        .filter(|g| goal_reach_cost(g.block_id, known_goals).map_or(false, |c| c <= MAX_STEPS))
        .copied()
        .collect();
    let pool: &[MaterialGoal] = if reachable.is_empty() { catalog } else { &reachable };
    Some(pool[(seed % pool.len() as u64) as usize])
}

/// 好奇心種下的自發心願文字——**保證含材料名**，[`detect_missing_material`]
/// 一定接得住（round-trip 由測試釘住），發明引擎自然接手。
pub fn curiosity_desire_text(name: &str) -> String {
    format!("好想自己做出一個{name}試試")
}

/// 好奇心冒泡（她自言自語——玩家看得到她在自主探索）。
pub fn curiosity_line(name: &str) -> String {
    format!("咦…聽說世界上做得出{name}？好想自己試試！")
}

/// 好奇心 Feed 詳情（玩家回來能讀到「她自己起了好奇心」）。
pub fn curiosity_feed(name: &str) -> String {
    format!("對{name}起了好奇心，想自己摸索著做出來")
}

/// 好奇心寫進記憶（日記走既有事件管道自然反映）。
pub fn curiosity_memory(name: &str) -> String {
    format!("我對{name}起了好奇心——沒有人教我，我想自己摸索著做出來")
}

/// 目錄空（能學的全學會了）時的冒泡——**零 LLM**，不打腦。
pub fn nothing_new_line() -> &'static str {
    "最近沒什麼新東西想試呢～我會的已經不少啦"
}

/// 退避：資源採不到、發明卡住時的 Feed 行（「這附近找不到木頭呢…」）。
/// `goal_name`：目標材料名；`missing_resource`：找不到的資源名（可為空）。
pub fn backoff_no_resource_feed(goal_name: &str, missing_resource: &str) -> String {
    if missing_resource.is_empty() {
        format!("試了幾次，{goal_name}這次做不出來，先放一放")
    } else {
        format!("這附近找不到{missing_resource}呢…{goal_name}先擱著，改天再試")
    }
}

/// 退避：換目標冒泡（「釣竿太難了，先試試別的」）。
pub fn backoff_switch_line(goal_name: &str) -> String {
    format!("{goal_name}太難了，先試試別的～")
}

/// 退避：換目標的 Feed 行。
pub fn backoff_switch_feed(goal_name: &str) -> String {
    format!("連試 {INVENT_BACKOFF_THRESHOLD} 次都沒成功，暫時不再試{goal_name}，換個方向探索")
}

/// 連敗退避判定（#972 延伸）：把一次失敗計入該目標的連敗計數（`fail_count`），
/// 達 [`INVENT_BACKOFF_THRESHOLD`] 就回 `true`（呼叫端該啟動退避）並把計數歸零
/// （退避到期後可重新累計）；未達門檻回 `false`。
///
/// **首次發明**與**技能重用**兩條失敗路徑共用同一套判定——一個老是失敗的**已學會技能**
/// （多半是身邊暫時沒料）若不退避，會讓她每個 build tick 重用同一技能、卡在同一步無限
/// 鬼打牆（線上實見 `reuse=true step=0` 每 ~9 秒重試一次）。純函式、可窮舉測。
pub fn note_fail_should_backoff(fail_count: &mut u8) -> bool {
    *fail_count = fail_count.saturating_add(1);
    if *fail_count >= INVENT_BACKOFF_THRESHOLD {
        *fail_count = 0; // 退避後歸零，到期可重新累計
        true
    } else {
        false
    }
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
        // 床要葉片（居民採不到）→ 不可發明，拒絕。
        // （火把/鐵鎬在第五刀鑿井尋礦後礦石可採、麵包在第七刀作物入自採閉包後小麥可收成，
        // 皆已改為可發明，不再當反例。）
        let raw = r#"{"name":"做床","steps":[{"op":"craft","recipe":"bed"}]}"#;
        assert!(parse_plan(raw).is_none());
        // 兩張表都查無此 id → 拒絕。
        let raw = r#"{"name":"亂","steps":[{"op":"craft","recipe":"no_such_recipe"}]}"#;
        assert!(parse_plan(raw).is_none());
    }

    // ── 技能組合技能（use_skill 展開，第三刀）────────────────────────────────────

    #[test]
    fn expand_step_passes_through_ordinary_primitives() {
        // 非 use_skill 的原語照舊 1:1 展開成單元素 Vec，行為與展開前完全一致。
        let out = expand_step(&PrimStep::Gather { resource: "wood".into(), count: 2 }, &[])
            .expect("普通原語應展開成功");
        assert_eq!(out, vec![CheckedStep::Gather { resource: GatherResource::Wood, count: 2 }]);
    }

    #[test]
    fn expand_step_use_skill_flattens_known_skill() {
        let known = vec![(
            "自製木板".to_string(),
            vec![
                PrimStep::Gather { resource: "wood".into(), count: 2 },
                PrimStep::Craft { recipe: "plank".into() },
            ],
        )];
        let out = expand_step(&PrimStep::UseSkill { name: "自製木板".to_string() }, &known)
            .expect("已學會的技能應展開成具體原語");
        assert_eq!(
            out,
            vec![
                CheckedStep::Gather { resource: GatherResource::Wood, count: 2 },
                CheckedStep::Craft { recipe_id: "plank" },
            ]
        );
    }

    #[test]
    fn expand_step_use_skill_trims_name_and_rejects_unknown() {
        let known = vec![("木板".to_string(), vec![PrimStep::Craft { recipe: "plank".into() }])];
        // 頭尾空白容忍。
        assert!(expand_step(&PrimStep::UseSkill { name: " 木板 ".to_string() }, &known).is_ok());
        // 引用一個她根本沒學過的名字 → 具體拒絕原因，不是靜默失敗。
        let err = expand_step(&PrimStep::UseSkill { name: "隱形斗篷".to_string() }, &known)
            .expect_err("沒學過的技能名應被拒絕");
        assert!(err.contains("隱形斗篷") && err.contains("還沒學會"));
        // 空清單（新生兒/從沒發明成功過）一樣誠實拒絕，不 panic。
        assert!(expand_step(&PrimStep::UseSkill { name: "木板".to_string() }, &[]).is_err());
    }

    #[test]
    fn expand_step_rejects_nested_use_skill() {
        // 引用的技能自己內部又含 use_skill（理論上到不了——存檔前一律展平；
        // 這裡直接構造來驗證防線本身有效）→ 拒絕整段引用，不遞迴展開。
        let known = vec![(
            "疊疊樂".to_string(),
            vec![PrimStep::UseSkill { name: "木板".to_string() }],
        )];
        let err = expand_step(&PrimStep::UseSkill { name: "疊疊樂".to_string() }, &known)
            .expect_err("巢狀 use_skill 應被拒絕");
        assert!(err.contains("疊兩層"));
    }

    #[test]
    fn check_step_rejects_bare_use_skill() {
        // UseSkill 單獨出現（沒展開）一律無效——必須透過 expand_step。
        assert!(check_step(&PrimStep::UseSkill { name: "隨便".to_string() }).is_none());
    }

    #[test]
    fn parse_plan_with_skills_expands_use_skill_beyond_raw_step_cap() {
        // 三個 use_skill 步驟（raw op 數＝3，遠低於 MAX_STEPS），各自展開成多步，
        // 疊起來超過 MAX_STEPS——證明「組合已學技能」真的能讓深度鏈塞進淺淺的 raw 上限。
        let known = vec![
            (
                "備木板".to_string(),
                vec![
                    PrimStep::Gather { resource: "wood".into(), count: 2 },
                    PrimStep::Craft { recipe: "plank".into() },
                ],
            ),
            (
                "蓋工作台".to_string(),
                vec![
                    PrimStep::Gather { resource: "wood".into(), count: 2 },
                    PrimStep::Craft { recipe: "plank".into() },
                    PrimStep::Craft { recipe: "workbench".into() },
                    PrimStep::Place { block: "workbench".into() },
                ],
            ),
        ];
        let raw = r#"{"name":"組合技","steps":[
            {"op":"use_skill","name":"備木板"},
            {"op":"use_skill","name":"備木板"},
            {"op":"use_skill","name":"蓋工作台"}]}"#;
        let plan = parse_plan_detailed_with_skills(raw, &known)
            .expect("引用三個已學技能應展開成功");
        assert_eq!(plan.steps.len(), 8, "2+2+4 步展開後應是 8 步（遠超過 3 個 raw op）");
        assert!(plan.steps.len() > 3, "展開後步數應遠超過 raw op 數（3）");
        // 這一層的 raw_steps 刻意保留她原話（含 use_skill，供重試提示回顯用）；
        // 真正落地存檔前的展平在 accept_proposal_with_skills（見該測試）。
        assert_eq!(plan.raw_steps.len(), 3, "這一層 raw_steps 應保留原始 3 個 use_skill 呼叫");
    }

    #[test]
    fn parse_plan_detailed_unchanged_when_no_skills_passed() {
        // 舊呼叫端（&[]）行為與第三刀之前完全一致：普通計畫照舊解析、無回歸。
        let p = parse_plan_detailed(glass_plan_json()).expect("普通計畫不受影響");
        assert_eq!(p.steps.len(), 2);
    }

    #[test]
    fn accept_proposal_with_skills_end_to_end() {
        // 完整鏈：她已經會「備木板」，這次發明目標是熔爐——直接引用備木板蓋工作台，
        // 放置後再用工作台合熔爐（目標材料是熔爐，而非中途被放置消耗掉的工作台本身）。
        let known = vec![(
            "備木板".to_string(),
            vec![
                PrimStep::Gather { resource: "wood".into(), count: 2 },
                PrimStep::Craft { recipe: "plank".into() },
            ],
        )];
        let raw = r#"{"name":"快速蓋熔爐","steps":[
            {"op":"use_skill","name":"備木板"},
            {"op":"craft","recipe":"workbench"},
            {"op":"place","block":"workbench"},
            {"op":"gather","resource":"stone","count":8},
            {"op":"craft_wb","recipe":"furnace_wb"}]}"#;
        let plan = accept_proposal_with_skills(raw, &HashMap::new(), FURNACE_BLOCK_ID, false, false, &known)
            .expect("組合已學技能的計畫應通過完整驗證管線");
        assert!(simulate_plan(&plan.steps, &HashMap::new(), FURNACE_BLOCK_ID, false, false).is_ok());
        // 存檔版本已完全展平，不依賴「備木板」這個名字繼續存在。
        assert!(plan.raw_steps.iter().all(|s| !matches!(s, PrimStep::UseSkill { .. })));
    }

    #[test]
    fn accept_proposal_backward_compatible_with_empty_skills() {
        // accept_proposal（舊 API）等同 accept_proposal_with_skills(..., false, &[])，
        // 不受影響——既有呼叫端與既有測試皆不需要跟著改。
        let raw = r#"{"name":"燒玻璃","steps":[{"op":"gather","resource":"sand","count":2},{"op":"craft","recipe":"glass"}]}"#;
        let a = accept_proposal(raw, &HashMap::new(), 10, false, false).unwrap();
        let b = accept_proposal_with_skills(raw, &HashMap::new(), 10, false, false, &[]).unwrap();
        assert_eq!(a.steps, b.steps);
    }

    #[test]
    fn known_steps_for_returns_only_that_residents_skills() {
        let mut store = InventedSkillStore::new();
        store.add("露娜", "燒玻璃", 10, vec![PrimStep::Craft { recipe: "glass".into() }]);
        store.add("諾娃", "備木板", 8, vec![PrimStep::Craft { recipe: "plank".into() }]);
        let luna_known = store.known_steps_for("露娜");
        assert_eq!(luna_known.len(), 1);
        assert_eq!(luna_known[0].0, "燒玻璃");
        assert!(store.known_steps_for("奧瑞").is_empty(), "沒發明過任何技能的居民應回空清單");
    }

    #[test]
    fn invention_prompt_mentions_use_skill_only_when_known_nonempty() {
        let goal = MaterialGoal { block_id: 10, name_zh: "玻璃" };
        let (sys_empty, _) = invention_prompt("露娜", &goal, "想要玻璃", "", false, false, &[]);
        assert!(!sys_empty.contains("use_skill"), "沒學過任何技能就不該提這個 op");
        let names = vec!["備木板".to_string(), "燒玻璃".to_string()];
        let (sys_known, _) = invention_prompt("露娜", &goal, "想要玻璃", "", false, false, &names);
        assert!(sys_known.contains("use_skill") && sys_known.contains("備木板"));
    }

    #[test]
    fn accept_proposal_repairs_weak_brain_arithmetic() {
        // qwen2.5:3b 第四輪實測原樣輸出（重試版）：結構全對（採木→合板→合工作台→
        // 放置→3×3 合箱子）、數量全錯（箱子要木板×8+工作台×4=該合 3 次板，它只合 1 次）
        // ——提案階段正規化要機械補足，讓這個計畫直接可行。
        let raw = r#"{"name":"箱子合成器修正版","steps":[
            {"op":"gather","resource":"wood","count":8},
            {"op":"craft","recipe":"plank"},
            {"op":"craft_wb","recipe":"workbench"},
            {"op":"place","block":"workbench"},
            {"op":"craft_wb","recipe":"chest"}]}"#;
        let plan = accept_proposal(raw, &HashMap::new(), 42, false, false)
            .expect("結構對、數量錯的弱腦計畫應被正規化接受");
        // 正規化後從空背包模擬可達目標（存檔語意一致）。
        assert!(simulate_plan(&plan.steps, &HashMap::new(), 42, false, false).is_ok());
        // 正規化版本身就是存下來的 raw_steps（存檔＝執行＝提案，單一事實）。
        assert!(check_stored_steps(&plan.raw_steps).is_some());
        // 引擎補了木板備料：craft plank 至少 3 次（工作台 4＋箱子 8＝12 板，一次合 4）。
        let plank_crafts = plan
            .raw_steps
            .iter()
            .filter(|s| matches!(s, PrimStep::Craft { recipe } if recipe == "plank"))
            .count();
        assert!(plank_crafts >= 3, "應補足 3 次合板，實得 {plank_crafts}");
        // 模擬把關仍在：計畫根本沒做目標物 → 照樣拒絕（腦選錯配方救不了）。
        let raw = r#"{"name":"只做工作台","steps":[
            {"op":"gather","resource":"wood","count":2},
            {"op":"craft","recipe":"plank"},
            {"op":"craft","recipe":"workbench"}]}"#;
        let err = accept_proposal(raw, &HashMap::new(), 42, false, false).unwrap_err();
        assert!(err.contains("箱子"), "要點名缺目標材料：{err}");
        // 解析失敗的具體原因也照樣傳出。
        assert!(accept_proposal("嗯我想想", &HashMap::new(), 42, false, false)
            .unwrap_err()
            .contains("JSON"));
    }

    #[test]
    fn check_step_repairs_op_tag_confusion() {
        // 小模型實測最常犯（qwen2.5:3b 連重試都改不動）：隨身配方用 craft_wb 做——
        // 意圖無歧義（配方真實存在、只是掛在另一張表）→ 自動修復成正確步型。
        let s = PrimStep::CraftWb { recipe: "workbench".into() };
        assert_eq!(check_step(&s), Some(CheckedStep::Craft { recipe_id: "workbench" }));
        // 反向：工作台配方用 craft 做 → 修成 CraftWb（模擬仍會驗「先有工作台」）。
        let s = PrimStep::Craft { recipe: "glass_wb".into() };
        assert_eq!(check_step(&s), Some(CheckedStep::CraftWb { recipe_id: "glass_wb" }));
        // 修復後整個計畫可過（含正確依賴）：qwen 實測輸出的計畫形狀（craft_wb workbench）
        // 修復成 craft workbench 後，只剩缺料問題會由模擬給出具體回饋。
        let raw = r#"{"name":"收納箱","steps":[
            {"op":"gather","resource":"wood","count":6},
            {"op":"craft","recipe":"plank"},{"op":"craft","recipe":"plank"},{"op":"craft","recipe":"plank"},
            {"op":"craft_wb","recipe":"workbench"},
            {"op":"place","block":"workbench"},
            {"op":"craft_wb","recipe":"chest"}]}"#;
        let plan = parse_plan(raw).expect("op 修復後應可解析");
        assert!(simulate_plan(&plan.steps, &HashMap::new(), 42, false, false).is_ok());
    }

    #[test]
    fn parse_detailed_gives_specific_reasons() {
        // 詳細原因是 Voyager 式重試的關鍵回饋：每型失敗都要點得出具體錯處。
        // 兩張表都查無此 id → 講清「兩張清單都查無」。
        let raw = r#"{"name":"亂","steps":[{"op":"craft_wb","recipe":"no_such"}]}"#;
        assert!(parse_plan_detailed(raw).unwrap_err().contains("不在任何一張配方清單"));
        // 配料弄不到（床要葉片；火把在第五刀後可發明，換反例）→ 點名配方與原因。
        let raw = r#"{"name":"床","steps":[{"op":"craft","recipe":"bed"}]}"#;
        let err = parse_plan_detailed(raw).unwrap_err();
        assert!(err.contains("bed") && err.contains("弄不到"), "要點名配方與原因：{err}");
        // 亂 place token → 講白名單；亂資源（"iron" 第五刀起是合法鐵礦 token，換 "diamond"）
        // → 列出合法資源（含新開放的礦石）。
        let raw = r#"{"name":"亂","steps":[{"op":"place","block":"chest"}]}"#;
        assert!(parse_plan_detailed(raw).unwrap_err().contains("place 只能放"));
        let raw = r#"{"name":"亂","steps":[{"op":"gather","resource":"diamond","count":2}]}"#;
        let err = parse_plan_detailed(raw).unwrap_err();
        assert!(err.contains("grass / sand / dirt / stone / wood") && err.contains("iron_ore"));
        // 空步驟/找不到 JSON 也都有具體原因。
        assert!(parse_plan_detailed("我不知道").unwrap_err().contains("JSON"));
        assert!(parse_plan_detailed(r#"{"name":"空","steps":[]}"#).unwrap_err().contains("steps"));
        // 好計畫走詳細版仍通過（與 parse_plan 一致）。
        assert!(parse_plan_detailed(glass_plan_json()).is_ok());
    }

    // ── relax_json：便宜腦近似 JSON 的寬容修復（嚴格解析失敗後的退路）─────────────

    #[test]
    fn relax_leaves_valid_json_untouched() {
        // 合法 JSON 過修復器應原樣不動（修復器只在嚴格解析失敗後呼叫，但必須自身無害且冪等）。
        let valid = glass_plan_json();
        assert_eq!(relax_json(valid), valid);
        assert_eq!(relax_json(&relax_json(valid)), relax_json(valid), "應冪等");
    }

    #[test]
    fn relax_quotes_bare_object_keys() {
        // 便宜腦實測吐 `count:8`（裸鍵）→ 補雙引號後 serde 可解。
        let raw = r#"{"name":"造爐","steps":[{"op":"gather","resource":"stone",count:8}]}"#;
        let fixed = relax_json(raw);
        assert!(fixed.contains(r#""count":8"#), "裸鍵應補引號：{fixed}");
        let plan: RawPlan = serde_json::from_str(&fixed).expect("修復後應可解析");
        assert_eq!(plan.steps.len(), 1);
    }

    #[test]
    fn relax_drops_trailing_commas() {
        // 陣列與物件的 trailing comma 都要去掉。
        let raw = r#"{"name":"x","steps":[{"op":"craft","recipe":"glass",},],}"#;
        let fixed = relax_json(raw);
        assert!(!fixed.contains(",]") && !fixed.contains(",}"), "不應殘留 trailing comma：{fixed}");
        assert!(serde_json::from_str::<RawPlan>(&fixed).is_ok(), "修復後應可解析：{fixed}");
    }

    #[test]
    fn relax_strips_comments() {
        // 行註解與區塊註解整段刪除。
        let raw = "{\n  // 這是我的計畫\n  \"name\":\"x\", /* 步驟 */ \"steps\":[{\"op\":\"craft\",\"recipe\":\"glass\"}]\n}";
        let fixed = relax_json(raw);
        assert!(!fixed.contains("//") && !fixed.contains("/*"), "註解應被刪除：{fixed}");
        assert!(serde_json::from_str::<RawPlan>(&fixed).is_ok(), "修復後應可解析：{fixed}");
    }

    #[test]
    fn relax_converts_single_quotes() {
        // 單引號字串 → 雙引號。
        let raw = "{'name':'燒玻璃','steps':[{'op':'craft','recipe':'glass'}]}";
        let fixed = relax_json(raw);
        let plan: RawPlan = serde_json::from_str(&fixed).expect("單引號修復後應可解析");
        assert_eq!(plan.name, "燒玻璃");
    }

    #[test]
    fn relax_never_touches_string_contents() {
        // 字串「內容」裡的 `,` `}` `//` 與看似裸鍵的字都不可被改動——只動結構位置。
        let raw = r#"{"name":"a // b, c: d}","steps":[{"op":"craft","recipe":"glass"}]}"#;
        let fixed = relax_json(raw);
        let plan: RawPlan = serde_json::from_str(&fixed).expect("應可解析");
        assert_eq!(plan.name, "a // b, c: d}", "字串內容必須原封不動：{fixed}");
    }

    #[test]
    fn parse_recovers_from_cheap_brain_json_quirks() {
        // 端到端：帶多種瑕疵的「近似 JSON」經修復退路後，parse_plan 仍成功解析出合法計畫。
        // 綜合裸鍵＋trailing comma＋行註解（皆取自線上便宜腦真實失敗樣態）。
        let raw = "{\n \"name\":\"燒玻璃\", // 我的技能\n \"steps\":[{\"op\":\"gather\",\"resource\":\"sand\",count:2},{\"op\":\"craft\",\"recipe\":\"glass\"},]\n}";
        let p = parse_plan(raw).expect("帶瑕疵的計畫應經修復退路救回");
        assert_eq!(p.steps.len(), 2);
        assert_eq!(p.name, "燒玻璃");
    }

    #[test]
    fn parse_still_rejects_unrepairable_garbage() {
        // 修復退路不是「什麼都收」——語意壞的（白名單外資源）修好語法仍該被白名單擋下。
        // （"iron" 第五刀起是合法鐵礦 token，反例換真的不存在的 "diamond"。）
        let raw = r#"{"name":"亂",steps:[{"op":"gather",resource:"diamond",count:2}]}"#;
        assert!(parse_plan(raw).is_none(), "語法修好但白名單外資源仍應拒絕");
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
        let err = simulate_plan(&p.steps, &HashMap::new(), 8, false, false).unwrap_err();
        assert!(err.contains("木板"), "錯誤原因應點名不夠料的配方：{err}");
    }

    #[test]
    fn simulate_accepts_correct_plan() {
        let raw = r#"{"name":"備木成板","steps":[{"op":"gather","resource":"wood","count":2},{"op":"craft","recipe":"plank"}]}"#;
        let p = parse_plan(raw).unwrap();
        assert!(simulate_plan(&p.steps, &HashMap::new(), 8, false, false).is_ok());
    }

    #[test]
    fn simulate_rejects_plan_missing_goal() {
        // 計畫做得出玻璃、但目標是木板 → 跑完背包沒有目標材料，擋下。
        let p = parse_plan(glass_plan_json()).unwrap();
        let err = simulate_plan(&p.steps, &HashMap::new(), 8, false, false).unwrap_err();
        assert!(err.contains("木板"));
    }

    #[test]
    fn simulate_uses_existing_bag_stock() {
        // 背包已有 2 木 → 「直接合成木板」的計畫也可行（後置條件語意、少採不浪費）。
        let raw = r#"{"name":"就地取材","steps":[{"op":"craft","recipe":"plank"}]}"#;
        let p = parse_plan(raw).unwrap();
        let bag = HashMap::from([(5u8, 2u32)]);
        assert!(simulate_plan(&p.steps, &bag, 8, false, false).is_ok());
        // 空背包則不可行。
        assert!(simulate_plan(&p.steps, &HashMap::new(), 8, false, false).is_err());
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
        assert!(simulate_plan(&p.steps, &HashMap::new(), 8, false, false).is_err());
        // 正規化後：補了採集步 → 空背包可行。
        let canon = canonicalize_steps(&p.steps);
        let checked = check_stored_steps(&canon).expect("正規化版應過存檔白名單");
        assert!(simulate_plan(&checked, &HashMap::new(), 8, false, false).is_ok(), "正規化技能應自足");
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
        assert!(simulate_plan(&checked, &HashMap::new(), 10, false, false).is_ok());
        // 原本的採集步仍在最前面。
        assert_eq!(
            checked[0],
            CheckedStep::Gather { resource: GatherResource::Sand, count: 2 }
        );
    }

    #[test]
    fn canonicalize_shared_raw_between_inputs_wood_pickaxe() {
        // 迴歸（隔離實測 res_4 反覆放棄「木鎬」「木斧」的真兇）：木鎬＝木×3＋木板×1，
        // 而木板＝木×2——兩個配料共用同一原料「木頭」。舊正規化依序備料：先備木×3、
        // 再備木板時 `craft plank` 吃掉 2 木剩 1 木，最後合木鎬缺木、被防禦性忽略，
        // 產出的計畫其實缺料，simulate_plan 才擋下 → 居民白試又放棄。ensure_craftable
        // 的補料迴圈要「缺多少再補多少」直到能一次合成。
        for (recipe, goal) in [("wood_pickaxe", vcraft::PICKAXE_WOOD_ID), ("wood_axe", vcraft::AXE_WOOD_ID)] {
            let steps = vec![CheckedStep::Craft { recipe_id: recipe }];
            let canon = canonicalize_steps(&steps);
            let checked = check_stored_steps(&canon).expect("正規化版應過存檔白名單");
            assert!(
                simulate_plan(&checked, &HashMap::new(), goal, false, false).is_ok(),
                "{recipe}：正規化後從空背包應能真的合成出目標（原料被共用配料吃掉的缺口要補回）"
            );
        }
    }

    #[test]
    fn accept_proposal_repairs_wood_tool_undercount() {
        // 端到端（對應日誌 res_4 的重試計畫）：便宜腦排對依賴、數量算不動——採木×3、
        // 合工作台、合木鎬，卻沒算到合工作台的木板會吃掉木頭。提案階段正規化應補足，
        // 讓這個「結構對、算術錯」的計畫直接可行、被接受存成技能。
        let raw = r#"{"name":"自製木鎬","steps":[
            {"op":"gather","resource":"wood","count":3},
            {"op":"craft_wb","recipe":"workbench"},
            {"op":"craft","recipe":"wood_pickaxe"}]}"#;
        let plan = accept_proposal(raw, &HashMap::new(), vcraft::PICKAXE_WOOD_ID, false, false)
            .expect("結構對、共用原料算術錯的計畫應被正規化接受");
        assert!(
            simulate_plan(&plan.steps, &HashMap::new(), vcraft::PICKAXE_WOOD_ID, false, false).is_ok(),
            "正規化後應真能做出木鎬"
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

    // ── 發明採集·地下資源改走階梯井（石器 0% 成功根治）─────────────────────────
    #[test]
    fn underground_resources_route_to_quarry() {
        // 石／泥埋在地表底下、surface-mine 碰不到 → 改走礦井。
        assert!(resource_is_underground(GatherResource::Stone));
        assert!(resource_is_underground(GatherResource::Dirt));
        // 木頭（樹）／細沙（灘地）／草皮（地表頂）本就在地表、或礦井挖不到 → 維持地表採集。
        assert!(!resource_is_underground(GatherResource::Wood));
        assert!(!resource_is_underground(GatherResource::Sand));
        assert!(!resource_is_underground(GatherResource::Grass));
    }

    #[test]
    fn invent_max_wells_is_bounded_and_positive() {
        // 至少能開一口（否則地下資源永遠採不到）、又有界防呆（不無限挖）。
        assert!(INVENT_MAX_WELLS >= 1);
        assert!(INVENT_MAX_WELLS <= 8);
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
            source: None,
            taught: false,
        };
        let new = InventedSkillRecord {
            resident: "vox_res_0".into(),
            name: "新玻璃法".into(),
            goal_block: 10,
            steps: plan.raw_steps,
            seq: 5,
            source: None,
            taught: false,
        };
        let s = InventedSkillStore::from_entries(vec![old, new]);
        assert_eq!(s.find_for("vox_res_0", 10).unwrap().name, "新玻璃法");
    }

    #[test]
    fn inherit_copies_parent_skill_with_source() {
        let plan = parse_plan(glass_plan_json()).unwrap();
        let mut s = InventedSkillStore::new();
        // 父母（諾娃 vox_res_1）自己發明了「燒玻璃」。
        s.add("vox_res_1", "燒玻璃", 10, plan.raw_steps.clone()).unwrap();
        let parent_skills = s.inheritable_for("vox_res_1");
        assert_eq!(parent_skills.len(), 1);
        // 新生兒 vox_res_4 繼承 → 名下多一筆同原語、source 標父母名的技能。
        let inherited = s.inherit("vox_res_4", &parent_skills[0], "諾娃").unwrap();
        assert_eq!(inherited.resident, "vox_res_4");
        assert_eq!(inherited.name, "燒玻璃");
        assert_eq!(inherited.steps, plan.raw_steps);
        assert_eq!(inherited.source.as_deref(), Some("諾娃"));
        // 一出生就會做（零 LLM 重用照舊：查得到她自己的技能）。
        assert!(s.find_for("vox_res_4", 10).is_some());
        // 同處境已會 → 不重複繼承。
        assert!(s.inherit("vox_res_4", &parent_skills[0], "諾娃").is_none());
    }

    #[test]
    fn old_record_without_source_loads_as_none() {
        // 向後相容：舊 jsonl 沒有 source 欄位 → 載回 source=None（不 panic）。
        let line = r#"{"resident":"vox_res_0","name":"燒玻璃","goal_block":10,"steps":[],"seq":0}"#;
        let rec: InventedSkillRecord = serde_json::from_str(line).unwrap();
        assert_eq!(rec.source, None);
    }

    // ── 技能互教（北極星第四刀）：learn_from ＋師承鏈 ─────────────────────────────

    #[test]
    fn learn_from_copies_skill_and_marks_teacher() {
        let plan = parse_plan(glass_plan_json()).unwrap();
        let mut s = InventedSkillStore::new();
        // 露娜（vox_res_0）自己發明了「燒玻璃」，就地教給諾娃（vox_res_1）。
        s.add("vox_res_0", "燒玻璃", 10, plan.raw_steps.clone()).unwrap();
        let skill = s.find_for("vox_res_0", 10).unwrap().clone();
        let learned = s.learn_from("vox_res_1", &skill, "露娜").unwrap();
        assert_eq!(learned.resident, "vox_res_1");
        assert_eq!(learned.name, "燒玻璃");
        assert_eq!(learned.steps, plan.raw_steps);
        assert_eq!(learned.source.as_deref(), Some("露娜"));
        assert!(learned.taught, "教學學來的要標師承，不能混同親子承繼");
        // 零 LLM 重用照舊：她自己的技能庫查得到、存檔驗證也過得了（下次同處境直接做）。
        assert!(s.find_for("vox_res_1", 10).is_some());
        assert!(check_steps(&learned.steps).is_some(), "學來的原語序列應能通過白名單驗證");
    }

    #[test]
    fn learn_from_dedups_when_student_already_knows() {
        let plan = parse_plan(glass_plan_json()).unwrap();
        let mut s = InventedSkillStore::new();
        s.add("vox_res_0", "燒玻璃", 10, plan.raw_steps.clone()).unwrap();
        s.add("vox_res_1", "自己的燒玻璃法", 10, plan.raw_steps).unwrap();
        let skill = s.find_for("vox_res_0", 10).unwrap().clone();
        assert!(s.learn_from("vox_res_1", &skill, "露娜").is_none(), "同處境已會就不重複學");
    }

    #[test]
    fn taught_skill_can_be_taught_onward_forming_a_chain() {
        // 師承鏈一節一節：露娜發明 → 教諾娃 → 諾娃再教賽勒，賽勒的 source 是諾娃（直系老師）。
        let plan = parse_plan(glass_plan_json()).unwrap();
        let mut s = InventedSkillStore::new();
        s.add("vox_res_0", "燒玻璃", 10, plan.raw_steps).unwrap();
        let luna_skill = s.find_for("vox_res_0", 10).unwrap().clone();
        s.learn_from("vox_res_1", &luna_skill, "露娜").unwrap();
        // 諾娃學會後，teachable 查得到「諾娃會、賽勒不會」→ 可再往下教。
        let nova_skill = s.teachable("vox_res_1", "vox_res_2").expect("學來的技能也可再教").clone();
        let seler = s.learn_from("vox_res_2", &nova_skill, "諾娃").unwrap();
        assert_eq!(seler.source.as_deref(), Some("諾娃"));
        assert!(seler.taught);
    }

    #[test]
    fn lineage_label_distinguishes_three_origins() {
        let plan = parse_plan(glass_plan_json()).unwrap();
        let mut s = InventedSkillStore::new();
        let invented = s.add("vox_res_0", "燒玻璃", 10, plan.raw_steps).unwrap();
        assert_eq!(lineage_label(&invented), "自己發明");
        let inherited = s.inherit("vox_res_4", &invented, "露娜").unwrap();
        assert_eq!(lineage_label(&inherited), "承自露娜");
        let taught = s.learn_from("vox_res_1", &invented, "露娜").unwrap();
        assert_eq!(lineage_label(&taught), "師承露娜");
    }

    #[test]
    fn self_invented_count_counts_only_own_inventions() {
        // 第一次發明立碑 v1：只數「自己發明」（source=None）的，繼承／師承的不算。
        let plan = parse_plan(glass_plan_json()).unwrap();
        let mut s = InventedSkillStore::new();
        // 露娜（vox_res_0）自己發明第一項技能：0 → 1。
        assert_eq!(s.self_invented_count("vox_res_0"), 0, "還沒發明過應為 0");
        let invented = s.add("vox_res_0", "燒玻璃", 10, plan.raw_steps.clone()).unwrap();
        assert_eq!(s.self_invented_count("vox_res_0"), 1, "自己發明一項後為 1");
        // 諾娃（vox_res_4）出生承繼露娜的技能：source 有值 → 不算她自己發明。
        s.inherit("vox_res_4", &invented, "露娜").unwrap();
        assert_eq!(s.self_invented_count("vox_res_4"), 0, "繼承來的不算自己發明");
        // 賽勒（vox_res_1）在世師承露娜：source 有值 → 一樣不算。
        s.learn_from("vox_res_1", &invented, "露娜").unwrap();
        assert_eq!(s.self_invented_count("vox_res_1"), 0, "師承來的不算自己發明");
        // 賽勒之後又自己發明另一項（不同目標材料）：0 → 1。
        let plan2 = parse_plan(glass_plan_json()).unwrap();
        s.add("vox_res_1", "自己的拋光法", 16, plan2.raw_steps).unwrap();
        assert_eq!(s.self_invented_count("vox_res_1"), 1, "師承後再自己發明才算 1");
    }

    #[test]
    fn old_record_without_taught_loads_as_parent_inheritance() {
        // 向後相容：#998 親子繼承的舊 jsonl 沒有 taught 欄位 → 載回 taught=false，
        // 來歷仍顯示「承自XX」，語意不變（不會被誤標成師承）。
        let line = r#"{"resident":"vox_res_4","name":"燒玻璃","goal_block":10,"steps":[],"seq":3,"source":"諾娃"}"#;
        let rec: InventedSkillRecord = serde_json::from_str(line).unwrap();
        assert!(!rec.taught);
        assert_eq!(lineage_label(&rec), "承自諾娃");
    }

    #[test]
    fn records_for_aligns_with_names_for_order() {
        let plan = parse_plan(glass_plan_json()).unwrap();
        let mut s = InventedSkillStore::new();
        s.add("vox_res_0", "燒玻璃", 10, plan.raw_steps.clone()).unwrap();
        s.add("vox_res_0", "備木板", 11, plan.raw_steps).unwrap();
        let names = s.names_for("vox_res_0");
        let recs = s.records_for("vox_res_0");
        assert_eq!(names.len(), recs.len());
        for (n, r) in names.iter().zip(recs.iter()) {
            assert_eq!(n, &r.name, "records_for 順序須與 names_for 對齊（技能簿並排呈現）");
        }
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
        // 第五刀（鑿井尋礦）：煤礦可採 → 火把也可發明了。
        assert!(ids.contains(&"torch"), "第五刀後火把（木＋煤礦）應可發明");
        // 第七刀（作物入自採閉包）後小麥可收成 → 麵包（3 小麥）也第一次可發明了。
        assert!(ids.contains(&"bread"), "第七刀後麵包（小麥可收成）應可發明");
        // 床要葉片 → 她弄不到料，仍不可發明（誠實邊界，葉片留給更後面一刀）。
        for no in ["bed"] {
            assert!(!ids.contains(&no), "{no} 不應可發明");
        }
        // 3×3 工作台配方：熔爐（8 石）/箱子（8 木板）/大量玻璃…在鏈上 → 可發明；
        // 第五刀後鐵錠（smelt_iron 冶煉）在鏈上 → 鐵系 3×3 也開放了。
        let wb_ids: Vec<&str> = inventable_wb_recipes().map(|r| r.id).collect();
        for want in ["furnace_wb", "chest", "glass_wb", "plank_wb", "stone_wood_mix", "farm_kit"] {
            assert!(wb_ids.contains(&want), "{want} 應可發明（工作台鏈）");
        }
        for want in ["iron_block", "iron_pickaxe", "iron_axe", "iron_shovel"] {
            assert!(wb_ids.contains(&want), "{want} 第五刀後鐵錠在鏈上，應可發明");
        }
        // 閉包集合本身：熔爐/箱子可取得；第四刀後拋光石（配料是可自採的石頭）也進了閉包；
        // 第五刀後鐵錠（鐵礦＋煤礦可自採冶煉）也第一次進了閉包。
        assert!(obtainable_ids().contains(&FURNACE_BLOCK_ID));
        assert!(obtainable_ids().contains(&42u8), "箱子在鏈上");
        assert!(obtainable_ids().contains(&22u8), "第五刀後鐵錠（礦石可採＋冶煉）在鏈上");
        assert!(obtainable_ids().contains(&17u8), "第四刀後拋光石（石頭冶煉）在鏈上");
    }

    #[test]
    fn inventable_furnace_recipes_respect_raw_material_closure() {
        // 熔爐冶煉配方（第四刀）：配料全可自採的才可發明；生料她弄不到的誠實排除。
        let ids: Vec<&str> = inventable_furnace_recipes().map(|r| r.id).collect();
        // 第五刀（鑿井尋礦）後礦石可採 → smelt_iron 也第一次可發明。
        // 第六刀（漁獲入自採閉包）後小魚可釣 → smelt_fish（烤魚）也第一次可發明。
        // 第七刀（作物入自採閉包）後馬鈴薯可收成 → smelt_potato（烤地薯）也第一次可發明。
        // 第十刀（莓果入自採閉包）後莓果可收成 → smelt_jam（莓果醬）也第一次可發明——
        // 全部 7 條熔爐配方至此皆已落進可發明閉包。
        for want in [
            "smelt_stone", "smelt_glass", "smelt_brick", "smelt_iron", "smelt_fish",
            "smelt_potato", "smelt_jam",
        ] {
            assert!(ids.contains(&want), "{want} 配料可自採，應可發明");
        }
    }

    // ── 第二刀：工作台鏈的可行性模擬（依賴順序：先有工作台才能 3×3）────────────────

    #[test]
    fn simulate_rejects_craft_wb_without_workbench() {
        // 沒放工作台就 3×3 → 擋下，錯誤原因要教它工作台怎麼來（回饋給便宜腦修正）。
        let raw = r#"{"name":"直接開爐","steps":[
            {"op":"gather","resource":"stone","count":8},
            {"op":"craft_wb","recipe":"furnace_wb"}]}"#;
        let p = parse_plan(raw).unwrap();
        let err = simulate_plan(&p.steps, &HashMap::new(), FURNACE_BLOCK_ID, false, false).unwrap_err();
        assert!(err.contains("工作台"), "錯誤原因應點名缺工作台：{err}");
        assert!(err.contains("workbench"), "錯誤原因應附上工作台配方 id：{err}");
        // 同一計畫、但她附近本來就有工作台 → 可行（不必重做一個）。
        assert!(simulate_plan(&p.steps, &HashMap::new(), FURNACE_BLOCK_ID, true, false).is_ok());
    }

    #[test]
    fn simulate_accepts_full_workbench_chain() {
        // 全鏈：採木→合板→合工作台→放置→採石→3×3 合熔爐，空背包、附近沒工作台也可行。
        let p = parse_plan(furnace_chain_json()).unwrap();
        assert!(simulate_plan(&p.steps, &HashMap::new(), FURNACE_BLOCK_ID, false, false).is_ok());
    }

    #[test]
    fn simulate_rejects_place_without_item() {
        // 沒先合成工作台就放置 → 擋下（背包裡沒有它）。
        let raw = r#"{"name":"憑空放台","steps":[
            {"op":"place","block":"workbench"},
            {"op":"gather","resource":"stone","count":8},
            {"op":"craft_wb","recipe":"furnace_wb"}]}"#;
        let p = parse_plan(raw).unwrap();
        let err = simulate_plan(&p.steps, &HashMap::new(), FURNACE_BLOCK_ID, false, false).unwrap_err();
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
        assert!(simulate_plan(&p.steps, &HashMap::new(), FURNACE_BLOCK_ID, false, false).is_err());
    }

    #[test]
    fn simulate_place_skips_when_workbench_already_nearby() {
        // 附近已有工作台時，place 步是後置條件 no-op（不需要背包裡真的有一個）。
        let raw = r#"{"name":"就地用台","steps":[
            {"op":"place","block":"workbench"},
            {"op":"gather","resource":"sand","count":6},
            {"op":"craft_wb","recipe":"glass_wb"}]}"#;
        let p = parse_plan(raw).unwrap();
        assert!(simulate_plan(&p.steps, &HashMap::new(), 10, true, false).is_ok());
        // 附近沒有 → 背包也沒有 → 擋下。
        assert!(simulate_plan(&p.steps, &HashMap::new(), 10, false, false).is_err());
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
        assert!(simulate_plan(&p.steps, &HashMap::new(), 42, false, false).is_ok());
        // 少採一輪木（4 木只夠 2 次合成 = 8 木板，工作台用掉 4、剩 4 < 8）→ 擋下。
        let short = raw.replace(r#""count":6"#, r#""count":4"#);
        let p2 = parse_plan(&short).unwrap();
        assert!(simulate_plan(&p2.steps, &HashMap::new(), 42, false, false).is_err());
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
                StepAction::DoSmelt { .. }
                | StepAction::Waiting
                | StepAction::CollectSmelt { .. }
                | StepAction::StartFish
                | StepAction::CollectFish
                | StepAction::DoHarvest { .. } => {
                    unreachable!("此計畫（合出熔爐本身）不含冶煉／釣魚／收成步驟")
                }
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
        assert!(simulate_plan(&checked, &HashMap::new(), FURNACE_BLOCK_ID, false, false).is_ok());
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
        assert!(simulate_plan(&p.steps, &HashMap::new(), FURNACE_BLOCK_ID, false, false).is_err());
        assert!(simulate_plan(&p.steps, &HashMap::new(), FURNACE_BLOCK_ID, true, false).is_ok());
        // 正規化後：空背包、附近沒工作台也可行（技能是帶著走的本事）。
        let canon = canonicalize_steps(&p.steps);
        let checked = check_stored_steps(&canon).expect("正規化版應過存檔白名單");
        assert!(
            simulate_plan(&checked, &HashMap::new(), FURNACE_BLOCK_ID, false, false).is_ok(),
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
        assert!(simulate_plan(&checked, &HashMap::new(), 42, false, false).is_ok(), "箱子鏈技能應自足");
    }

    // ── 第四刀：熔爐冶煉（世界前提＋煨煮時間）───────────────────────────────────

    /// 第四刀全鏈（拋光石）：接續造爐之路，放置熔爐後採石冶煉——9 步已超過 LLM 提案的
    /// [`MAX_STEPS`]（8，見發明本身的節制），故直接組 `CheckedStep`（等同「已通過解析」
    /// 的存檔/執行引擎輸入），不經 `parse_plan`；驗的是 `simulate_plan`/`steps_summary`
    /// 對冶煉步的處理，非提案階段的原始步數上限。
    fn smelt_stone_chain_checked() -> Vec<CheckedStep> {
        vec![
            CheckedStep::Gather { resource: GatherResource::Wood, count: 2 },
            CheckedStep::Craft { recipe_id: "plank" },
            CheckedStep::Craft { recipe_id: "workbench" },
            CheckedStep::Place { block_id: WORKBENCH_BLOCK_ID },
            CheckedStep::Gather { resource: GatherResource::Stone, count: 8 },
            CheckedStep::CraftWb { recipe_id: "furnace_wb" },
            CheckedStep::Place { block_id: FURNACE_BLOCK_ID },
            CheckedStep::Gather { resource: GatherResource::Stone, count: 3 },
            CheckedStep::Smelt { recipe_id: "smelt_stone" },
        ]
    }

    #[test]
    fn parse_accepts_smelt_op() {
        let raw = r#"{"name":"冶煉","steps":[{"op":"smelt","recipe":"smelt_stone"}]}"#;
        let p = parse_plan(raw).expect("smelt op 應可解析（配料可自採）");
        assert_eq!(p.steps[0], CheckedStep::Smelt { recipe_id: "smelt_stone" });
    }

    #[test]
    fn check_step_accepts_all_furnace_recipes_after_berry_closure() {
        // 第十刀（莓果入自採閉包）後，最後一條熔爐配方 smelt_jam（莓果醬）也可發明——
        // 至此 7 條熔爐配方全數落進閉包，無一還在誠實拒絕清單裡。
        let ok_jam = PrimStep::Smelt { recipe: "smelt_jam".into() };
        assert_eq!(check_step(&ok_jam), Some(CheckedStep::Smelt { recipe_id: "smelt_jam" }));
        // 第五刀正例：鐵錠冶煉（礦石生料如今可自採）通過檢查。
        let ok = PrimStep::Smelt { recipe: "smelt_iron".into() };
        assert_eq!(check_step(&ok), Some(CheckedStep::Smelt { recipe_id: "smelt_iron" }));
        // 第六刀正例：烤魚冶煉（小魚生料如今可自釣）通過檢查。
        let ok_fish = PrimStep::Smelt { recipe: "smelt_fish".into() };
        assert_eq!(check_step(&ok_fish), Some(CheckedStep::Smelt { recipe_id: "smelt_fish" }));
        // 第七刀正例：烤地薯冶煉（馬鈴薯生料如今可自採）通過檢查。
        let ok_potato = PrimStep::Smelt { recipe: "smelt_potato".into() };
        assert_eq!(
            check_step(&ok_potato),
            Some(CheckedStep::Smelt { recipe_id: "smelt_potato" })
        );
    }

    #[test]
    fn check_step_rejects_unknown_furnace_recipe() {
        let s = PrimStep::Smelt { recipe: "no_such_recipe".into() };
        assert_eq!(check_step(&s), None);
        assert!(explain_bad_step(&s).contains("不在熔爐配方清單裡"), "{}", explain_bad_step(&s));
    }

    #[test]
    fn simulate_smelt_requires_furnace_nearby() {
        let raw = r#"{"name":"沒爐硬冶煉","steps":[
            {"op":"gather","resource":"stone","count":3},
            {"op":"smelt","recipe":"smelt_stone"}]}"#;
        let p = parse_plan(raw).unwrap();
        let err = simulate_plan(&p.steps, &HashMap::new(), 17, false, false).unwrap_err();
        assert!(err.contains("熔爐"), "錯誤原因應點名缺熔爐：{err}");
        assert!(err.contains("furnace_wb"), "錯誤原因應附上熔爐配方 id：{err}");
        // 附近本來就有熔爐 → 可行。
        assert!(simulate_plan(&p.steps, &HashMap::new(), 17, false, true).is_ok());
    }

    #[test]
    fn simulate_smelt_consumes_raw_material() {
        // 熔爐在旁但沒採夠料 → 冶煉本身仍走一般缺料檢查。
        let raw = r#"{"name":"料不夠冶煉","steps":[
            {"op":"gather","resource":"stone","count":1},
            {"op":"smelt","recipe":"smelt_stone"}]}"#;
        let p = parse_plan(raw).unwrap();
        assert!(simulate_plan(&p.steps, &HashMap::new(), 17, false, true).is_err());
    }

    #[test]
    fn simulate_accepts_full_smelt_chain() {
        // 全鏈：造爐之路接續放置熔爐、採石、冶煉出拋光石，空背包/附近皆無也可行。
        let steps = smelt_stone_chain_checked();
        assert!(simulate_plan(&steps, &HashMap::new(), 17, false, false).is_ok());
    }

    #[test]
    fn canonicalize_inserts_furnace_group_for_bare_smelt() {
        // 她發明時附近剛好有熔爐 → 腦可能只提「採石→冶煉」；存檔版必須自足：
        // 正規化應自動補上整組「工作台鏈→熔爐鏈」。
        let raw = r#"{"name":"就地冶煉","steps":[
            {"op":"gather","resource":"stone","count":3},
            {"op":"smelt","recipe":"smelt_stone"}]}"#;
        let p = parse_plan(raw).unwrap();
        // 原計畫只有在「附近有熔爐」時可行。
        assert!(simulate_plan(&p.steps, &HashMap::new(), 17, false, false).is_err());
        assert!(simulate_plan(&p.steps, &HashMap::new(), 17, false, true).is_ok());
        // 正規化後：空背包、附近沒工作台/熔爐也可行（技能是帶著走的本事）。
        let canon = canonicalize_steps(&p.steps);
        let checked = check_stored_steps(&canon).expect("正規化版應過存檔白名單");
        assert!(
            simulate_plan(&checked, &HashMap::new(), 17, false, false).is_ok(),
            "正規化技能應自足（含工作台＋熔爐備妥組）"
        );
        assert!(canon.iter().any(|s| matches!(s, PrimStep::Place { block } if block == "furnace")));
        assert!(canon.iter().any(|s| matches!(s, PrimStep::Smelt { recipe } if recipe == "smelt_stone")));
    }

    #[test]
    fn next_action_smelt_state_machine() {
        let raw = r#"{"name":"冶煉拋光石","steps":[
            {"op":"gather","resource":"stone","count":3},
            {"op":"smelt","recipe":"smelt_stone"}]}"#;
        let plan = parse_plan(raw).unwrap();
        let mut run = InventRun::from_plan(17, "拋光石", &plan, false);
        run.step_idx = 1; // 冶煉步
        let bag = HashMap::new();
        // 還沒開爐 → DoSmelt。
        assert_eq!(next_action(&run, &bag, no_station), StepAction::DoSmelt { recipe_id: "smelt_stone" });
        // 開了爐、還在煨煮 → Waiting。
        run.smelt_wait = Some(5.0);
        assert_eq!(next_action(&run, &bag, no_station), StepAction::Waiting);
        // 熟成（<=0.0）→ CollectSmelt。
        run.smelt_wait = Some(0.0);
        assert_eq!(
            next_action(&run, &bag, no_station),
            StepAction::CollectSmelt { recipe_id: "smelt_stone" }
        );
    }

    #[test]
    fn smelt_start_apply_deducts_or_refuses() {
        let recipe = vcraft::find_furnace_recipe("smelt_stone").unwrap();
        let mut bag = HashMap::from([(3u8, 3u32)]); // 3 石頭，恰好夠
        assert!(smelt_start_apply(&mut bag, recipe), "料夠應成功開爐");
        assert_eq!(bag.get(&3u8).copied(), Some(0), "生料應被扣除");
        assert!(!bag.contains_key(&17u8), "開爐當下不產出成品，要等收成");
        let mut short = HashMap::from([(3u8, 2u32)]); // 差 1 個
        assert!(!smelt_start_apply(&mut short, recipe), "料不夠不能開爐");
        assert_eq!(short.get(&3u8).copied(), Some(2), "失敗不應動到背包");
    }

    #[test]
    fn invention_prompt_mentions_smelt_and_furnace_recipes() {
        let goal = MaterialGoal { block_id: 17, name_zh: "拋光石" };
        let (sys, user) = invention_prompt("露娜", &goal, "想要拋光石", "", false, false, &[]);
        assert!(sys.contains("smelt"), "system prompt 應教 smelt op");
        assert!(sys.contains("smelt_stone"), "應列出可發明的熔爐配方");
        assert!(user.contains("沒有熔爐"), "user prompt 應告知附近沒有熔爐");
        let (_, user2) = invention_prompt("露娜", &goal, "想要拋光石", "", false, true, &[]);
        assert!(user2.contains("已經有一座放置好的熔爐"));
    }

    #[test]
    fn steps_summary_includes_smelt_line() {
        let summary = steps_summary(&smelt_stone_chain_checked());
        assert!(summary.contains("熔爐冶煉拋光石"), "{summary}");
    }

    #[test]
    fn smelting_lines_mention_recipe_name() {
        assert!(smelting_started_line("拋光石").contains("拋光石"));
        assert!(smelting_done_line("拋光石").contains("拋光石"));
    }

    // ── 第六刀：釣魚（世界前提＋上鉤等候時間）───────────────────────────────────

    #[test]
    fn check_step_validates_fish_count_bounds() {
        assert_eq!(check_step(&PrimStep::Fish { count: 1 }), Some(CheckedStep::Fish { count: 1 }));
        assert_eq!(
            check_step(&PrimStep::Fish { count: MAX_GATHER_COUNT }),
            Some(CheckedStep::Fish { count: MAX_GATHER_COUNT })
        );
        assert_eq!(check_step(&PrimStep::Fish { count: 0 }), None, "0 條不合理");
        assert_eq!(check_step(&PrimStep::Fish { count: MAX_GATHER_COUNT + 1 }), None, "超上限");
        let bad = PrimStep::Fish { count: 0 };
        assert!(explain_bad_step(&bad).contains(&MAX_GATHER_COUNT.to_string()));
    }

    #[test]
    fn obtainable_ids_include_fish_and_cooked_fish() {
        // 第六刀：小魚（釣魚原語的產物）與烤魚（smelt_fish，唯一途徑）都應落進閉包。
        assert!(obtainable_ids().contains(&FISH_ID), "小魚應在種子集裡");
        assert!(
            obtainable_ids().contains(&crate::voxel_fishing::COOKED_FISH_ID),
            "烤魚（生魚可自釣→smelt_fish）應在閉包裡"
        );
        // 第七刀（作物入自採閉包）後馬鈴薯生料也進了閉包（見 obtainable_ids_include_crops_and_their_recipes）；
        // 第十刀（莓果入自採閉包）後莓果生料也進了閉包（見 obtainable_ids_include_berry_and_jam）。
    }

    #[test]
    fn obtainable_ids_include_berry_and_jam() {
        // 第十刀：莓果（收成原語的產物）與莓果醬（smelt_jam，唯一途徑）都應落進閉包。
        assert!(obtainable_ids().contains(&vberry::BERRY_ID), "莓果應在種子集裡");
        assert!(
            obtainable_ids().contains(&vberry::JAM_ID),
            "莓果醬（莓果可自採→smelt_jam）應在閉包裡"
        );
    }

    #[test]
    fn next_action_fish_state_machine() {
        let raw = r#"{"name":"釣魚烤魚","steps":[
            {"op":"fish","count":1},
            {"op":"smelt","recipe":"smelt_fish"}]}"#;
        let plan = parse_plan(raw).unwrap();
        let mut run = InventRun::from_plan(crate::voxel_fishing::COOKED_FISH_ID, "烤魚", &plan, false);
        let mut bag = HashMap::new();
        // 還沒拋竿 → StartFish。
        assert_eq!(next_action(&run, &bag, no_station), StepAction::StartFish);
        // 拋了竿、還在等 → Waiting。
        run.fish_wait = Some(3.0);
        assert_eq!(next_action(&run, &bag, no_station), StepAction::Waiting);
        // 上鉤（<=0.0）→ CollectFish。
        run.fish_wait = Some(0.0);
        assert_eq!(next_action(&run, &bag, no_station), StepAction::CollectFish);
        // 收竿入袋一條、需求恰好 1 條 → 下一輪滿足後置條件、直接 Advance。
        bag.insert(FISH_ID, 1);
        run.fish_wait = None;
        assert_eq!(next_action(&run, &bag, no_station), StepAction::Advance);
    }

    #[test]
    fn next_action_fish_reloops_until_count_satisfied() {
        // count=2：收到第 1 條後仍不足 → 應再拋一輪（StartFish），不會提早 Advance。
        let mut run = InventRun {
            goal_block: FISH_ID,
            goal_name: "小魚".into(),
            skill_name: "釣魚".into(),
            raw_steps: vec![PrimStep::Fish { count: 2 }],
            steps: vec![CheckedStep::Fish { count: 2 }],
            step_idx: 0,
            reuse: false,
            deadline: RUN_TIMEOUT_SECS,
            smelt_wait: None,
            fish_wait: None,
        };
        let mut bag = HashMap::from([(FISH_ID, 1u32)]); // 已釣到 1 條，還差 1 條
        assert_eq!(next_action(&run, &bag, no_station), StepAction::StartFish, "還差 1 條，應再拋竿");
        run.fish_wait = Some(0.0);
        assert_eq!(next_action(&run, &bag, no_station), StepAction::CollectFish);
        bag.insert(FISH_ID, 2);
        run.fish_wait = None;
        assert_eq!(next_action(&run, &bag, no_station), StepAction::Advance, "湊滿 2 條才推進");
    }

    #[test]
    fn simulate_plan_idealizes_fish_like_gather() {
        // 模擬階段不判水源可得性（同 Gather——執行期才真的去找，找不到才誠實失敗）。
        let steps = [CheckedStep::Fish { count: 2 }];
        let bag = HashMap::new();
        assert!(simulate_plan(&steps, &bag, FISH_ID, false, false).is_ok());
    }

    #[test]
    fn simulate_plan_fish_then_smelt_reaches_cooked_fish() {
        let steps = [CheckedStep::Fish { count: 1 }, CheckedStep::Smelt { recipe_id: "smelt_fish" }];
        let bag = HashMap::new();
        assert!(
            simulate_plan(&steps, &bag, crate::voxel_fishing::COOKED_FISH_ID, false, true).is_ok(),
            "有水釣得到魚、旁邊有熔爐 → 烤魚計畫應可行"
        );
        // 附近沒熔爐 → 冶煉步依賴順序擋下（同既有熔爐鏈守則）。
        let err = simulate_plan(&steps, &bag, crate::voxel_fishing::COOKED_FISH_ID, false, false)
            .expect_err("沒熔爐應失敗");
        assert!(err.contains("熔爐"), "{err}");
    }

    #[test]
    fn canonicalize_steps_inserts_fish_step_for_smelt_fish() {
        // 只給冶煉步（模擬便宜腦省略備料，如同既有木頭/石頭案例）→ 正規化應自動補釣魚步。
        let steps = [CheckedStep::Smelt { recipe_id: "smelt_fish" }];
        let canon = canonicalize_steps(&steps);
        assert!(
            canon.iter().any(|s| matches!(s, PrimStep::Fish { count } if *count >= 1)),
            "應自動補上釣魚備料步：{canon:?}"
        );
        let checked = check_stored_steps(&canon).expect("應過存檔白名單");
        assert!(
            simulate_plan(&checked, &HashMap::new(), crate::voxel_fishing::COOKED_FISH_ID, false, false).is_ok(),
            "正規化後應從空背包也能做出烤魚（放置了熔爐）"
        );
    }

    #[test]
    fn steps_summary_includes_fish_line() {
        let summary = steps_summary(&[CheckedStep::Fish { count: 2 }]);
        assert_eq!(summary, "釣魚×2");
    }

    #[test]
    fn fishing_lines_and_no_water_feed_are_sane() {
        assert!(fishing_started_line().contains("釣"));
        assert!(fishing_done_line().contains("小魚"));
        assert!(no_water_feed("烤魚").contains("烤魚"));
        assert!(no_water_feed("烤魚").contains("水"));
    }

    #[test]
    fn invention_prompt_mentions_fish_op() {
        let goal = MaterialGoal { block_id: crate::voxel_fishing::COOKED_FISH_ID, name_zh: "烤魚" };
        let (sys, _) = invention_prompt("露娜", &goal, "想要烤魚", "", false, false, &[]);
        assert!(sys.contains("\"op\":\"fish\""), "system prompt 應教 fish op: {sys}");
        assert!(sys.contains("必須附近有水"), "應點明水源前提: {sys}");
    }

    #[test]
    fn water_nearby_detects_within_radius_not_beyond() {
        let (fx, fy, fz) = (60, 5, 60);
        let mut world: WorldDelta = WorldDelta::new();
        assert!(!water_nearby(&world, fx, fy, fz), "還沒有水 → 查無");
        voxel::set_block(&mut world, fx + 3, fy, fz, Block::Water);
        assert!(water_nearby(&world, fx, fy, fz), "半徑內的水應查得到");
        // 半徑外放水 → 查無（獨立世界，避免上面那格水干擾）。
        let far = WATER_NEAR_RADIUS + 5;
        let mut world2: WorldDelta = WorldDelta::new();
        voxel::set_block(&mut world2, fx + far, fy, fz, Block::Water);
        assert!(!water_nearby(&world2, fx, fy, fz), "太遠不算附近");
    }

    // ── find_water：移動去資源（第八刀）遠距螺旋搜尋 ──────────────────────────────

    #[test]
    fn find_water_locates_source_beyond_near_radius() {
        let (fx, fy, fz) = (60, 5, 60);
        let mut world: WorldDelta = WorldDelta::new();
        // 放在鄰近檢查搆不到、但仍在遠距搜尋半徑內的位置——「移動去資源」該補的正是這段。
        let beyond_near = WATER_NEAR_RADIUS + 8;
        assert!(beyond_near <= INVENT_GATHER_RADIUS, "測試前提：夾在鄰近與遠距半徑之間");
        voxel::set_block(&mut world, fx + beyond_near, fy, fz, Block::Water);
        assert!(!water_nearby(&world, fx, fy, fz), "鄰近檢查本該查無（才輪得到遠距搜尋出場）");
        assert_eq!(
            find_water(&world, fx, fy, fz, INVENT_GATHER_RADIUS),
            Some((fx + beyond_near, fy, fz)),
            "遠距螺旋搜尋該找到這格水"
        );
    }

    #[test]
    fn find_water_none_beyond_search_radius() {
        let (fx, fy, fz) = (60, 5, 60);
        let mut world: WorldDelta = WorldDelta::new();
        voxel::set_block(&mut world, fx + 20, fy, fz, Block::Water);
        // 搜尋半徑故意設得比水還近 → 仍誠實查無（不是漫無邊際亂找）。
        assert_eq!(find_water(&world, fx, fy, fz, 10), None, "半徑外的水不該被找到");
    }

    /// 第六刀全鏈模擬（純邏輯側證據，比照第二刀 `full_workbench_chain_simulated_execution_reaches_goal`）：
    /// 空背包 → 釣魚（拋竿→等待→收竿）→ 熔爐冶煉（開爐→煨煮→收成）→ 後置條件成立
    /// （背包真的有烤魚）。驗證釣魚與冶煉兩種「需要等待」的原語可以串在同一條鏈裡。
    #[test]
    fn full_fish_then_smelt_chain_simulated_execution_reaches_goal() {
        let raw = r#"{"name":"釣魚烤魚","steps":[
            {"op":"fish","count":1},
            {"op":"smelt","recipe":"smelt_fish"}]}"#;
        let plan = parse_plan(raw).unwrap();
        let mut run =
            InventRun::from_plan(crate::voxel_fishing::COOKED_FISH_ID, "烤魚", &plan, false);
        let mut bag: HashMap<u8, u32> = HashMap::new();
        let has_furnace = true; // 這條鏈只驗釣魚/冶煉本身，熔爐已在旁（同既有 smelt 測試慣例）。
        let mut guard = 0;
        loop {
            guard += 1;
            assert!(guard < 60, "執行應在有限步內收斂");
            match next_action(&run, &bag, move |bid| has_furnace && bid == FURNACE_BLOCK_ID) {
                StepAction::StartFish => {
                    run.fish_wait = Some(INVENT_FISH_WAIT_SECS);
                }
                StepAction::CollectFish => {
                    *bag.entry(FISH_ID).or_insert(0) += 1;
                    run.fish_wait = None;
                }
                StepAction::Waiting => {
                    if let Some(w) = &mut run.fish_wait {
                        *w = 0.0; // 模擬時間流逝到熟成/上鉤
                    }
                    if let Some(w) = &mut run.smelt_wait {
                        *w = 0.0;
                    }
                }
                StepAction::Advance => run.step_idx += 1,
                StepAction::DoSmelt { recipe_id } => {
                    let r = vcraft::find_furnace_recipe(recipe_id).unwrap();
                    assert!(smelt_start_apply(&mut bag, r), "照計畫備好料，開爐應成功");
                    run.smelt_wait = Some(1.0);
                }
                StepAction::CollectSmelt { recipe_id } => {
                    let r = vcraft::find_furnace_recipe(recipe_id).unwrap();
                    *bag.entry(r.output_block).or_insert(0) += r.output_count;
                    run.smelt_wait = None;
                    run.step_idx += 1;
                }
                StepAction::Done => break,
                StepAction::StartGather { .. } | StepAction::DoCraft { .. }
                | StepAction::DoCraftWb { .. } | StepAction::DoPlace { .. }
                | StepAction::DoHarvest { .. } => {
                    unreachable!("此計畫（釣魚→冶煉）不含採集/合成/放置/收成步")
                }
            }
        }
        assert!(
            goal_met(&bag, crate::voxel_fishing::COOKED_FISH_ID),
            "後置條件：背包真的有烤魚"
        );
    }

    // ── 第七刀：作物入自採閉包（世界前提，無等待）───────────────────────────────

    #[test]
    fn crop_token_round_trips() {
        for c in [
            CropResource::Wheat, CropResource::Carrot, CropResource::Potato, CropResource::Berry,
        ] {
            assert_eq!(crop_from_token(c.token()), Some(c));
        }
        assert_eq!(crop_from_token("wheat"), Some(CropResource::Wheat));
        assert_eq!(crop_from_token("胡蘿蔔"), Some(CropResource::Carrot));
        assert_eq!(crop_from_token("薯"), Some(CropResource::Potato));
        assert_eq!(crop_from_token("莓果"), Some(CropResource::Berry));
        assert_eq!(crop_from_token("莓"), Some(CropResource::Berry));
        assert_eq!(crop_from_token("pumpkin"), None, "南瓜不在白名單");
    }

    #[test]
    fn check_step_validates_harvest_crop_and_count_bounds() {
        assert_eq!(
            check_step(&PrimStep::Harvest { crop: "wheat".into(), count: 1 }),
            Some(CheckedStep::Harvest { crop: CropResource::Wheat, count: 1 })
        );
        assert_eq!(
            check_step(&PrimStep::Harvest { crop: "carrot".into(), count: 0 }),
            None,
            "0 份不合理"
        );
        assert_eq!(
            check_step(&PrimStep::Harvest { crop: "potato".into(), count: MAX_GATHER_COUNT + 1 }),
            None,
            "超上限"
        );
        assert_eq!(
            check_step(&PrimStep::Harvest { crop: "pumpkin".into(), count: 1 }),
            None,
            "南瓜不在白名單"
        );
        let bad_crop = PrimStep::Harvest { crop: "pumpkin".into(), count: 1 };
        assert!(explain_bad_step(&bad_crop).contains("wheat"));
        let bad_count = PrimStep::Harvest { crop: "wheat".into(), count: 0 };
        assert!(explain_bad_step(&bad_count).contains(&MAX_GATHER_COUNT.to_string()));
        // 第十刀：莓果也是合法收成目標。
        assert_eq!(
            check_step(&PrimStep::Harvest { crop: "berry".into(), count: 2 }),
            Some(CheckedStep::Harvest { crop: CropResource::Berry, count: 2 })
        );
    }

    #[test]
    fn obtainable_ids_include_crops_and_their_recipes() {
        // 第七刀：小麥/胡蘿蔔/馬鈴薯生料（收成原語的產物）都應落進閉包。
        assert!(obtainable_ids().contains(&vfarm::WHEAT_ID), "小麥應在種子集裡");
        assert!(obtainable_ids().contains(&vfarm::CARROT_ID), "胡蘿蔔應在種子集裡");
        assert!(obtainable_ids().contains(&vfarm::POTATO_ID), "馬鈴薯應在種子集裡");
        // 三個下游配方（唯一途徑）第一次自動落進閉包。
        assert!(obtainable_ids().contains(&vfarm::BREAD_ID), "麵包（3 小麥→麵包）應在閉包裡");
        assert!(
            obtainable_ids().contains(&vfarm::BAKED_POTATO_ID),
            "烤地薯（smelt_potato，唯一途徑）應在閉包裡"
        );
        assert!(
            obtainable_ids().contains(&vcraft::STEW_ID),
            "野菜暖湯（三種作物齊聚）應在閉包裡"
        );
        // 南瓜刻意仍不可自採（季限作物，留給下一刀），閉包邊界保持誠實。
        assert!(!obtainable_ids().contains(&vfarm::PUMPKIN_ID));
    }

    #[test]
    fn next_action_harvest_state_machine() {
        let raw = r#"{"name":"種麵包","steps":[{"op":"harvest","crop":"wheat","count":3},{"op":"craft","recipe":"bread"}]}"#;
        let plan = parse_plan(raw).unwrap();
        let mut run = InventRun::from_plan(vfarm::BREAD_ID, "麵包", &plan, false);
        let mut bag = HashMap::new();
        // 還沒收成 → DoHarvest。
        assert_eq!(
            next_action(&run, &bag, no_station),
            StepAction::DoHarvest { crop: CropResource::Wheat }
        );
        // 收成滿足需求量（一次到位，無等待）→ Advance。
        bag.insert(vfarm::WHEAT_ID, 3);
        assert_eq!(next_action(&run, &bag, no_station), StepAction::Advance);
        run.step_idx += 1;
        assert_eq!(next_action(&run, &bag, no_station), StepAction::DoCraft { recipe_id: "bread" });
    }

    #[test]
    fn next_action_harvest_reloops_until_count_satisfied() {
        // count=3：一畦只夠收 1 份時，應反覆 DoHarvest 直到湊滿，不會提早 Advance。
        let run = InventRun {
            goal_block: vfarm::WHEAT_ID,
            goal_name: "小麥".into(),
            skill_name: "收成".into(),
            raw_steps: vec![PrimStep::Harvest { crop: "wheat".into(), count: 3 }],
            steps: vec![CheckedStep::Harvest { crop: CropResource::Wheat, count: 3 }],
            step_idx: 0,
            reuse: false,
            deadline: RUN_TIMEOUT_SECS,
            smelt_wait: None,
            fish_wait: None,
        };
        let mut bag = HashMap::from([(vfarm::WHEAT_ID, 1u32)]);
        assert_eq!(
            next_action(&run, &bag, no_station),
            StepAction::DoHarvest { crop: CropResource::Wheat },
            "還差 2 份，應再收成"
        );
        bag.insert(vfarm::WHEAT_ID, 2);
        assert_eq!(
            next_action(&run, &bag, no_station),
            StepAction::DoHarvest { crop: CropResource::Wheat },
            "還差 1 份，應再收成"
        );
        bag.insert(vfarm::WHEAT_ID, 3);
        assert_eq!(next_action(&run, &bag, no_station), StepAction::Advance, "湊滿 3 份才推進");
    }

    #[test]
    fn simulate_plan_idealizes_harvest_like_fish() {
        // 模擬階段不判熟作物可得性（同 Fish——執行期才真的去找，找不到才誠實失敗）。
        let steps = [CheckedStep::Harvest { crop: CropResource::Wheat, count: 3 }];
        let bag = HashMap::new();
        assert!(simulate_plan(&steps, &bag, vfarm::WHEAT_ID, false, false).is_ok());
    }

    #[test]
    fn simulate_plan_harvest_then_craft_reaches_bread() {
        let steps = [
            CheckedStep::Harvest { crop: CropResource::Wheat, count: 3 },
            CheckedStep::Craft { recipe_id: "bread" },
        ];
        let bag = HashMap::new();
        assert!(
            simulate_plan(&steps, &bag, vfarm::BREAD_ID, false, false).is_ok(),
            "附近收得到熟小麥 → 麵包計畫應可行"
        );
    }

    #[test]
    fn canonicalize_steps_inserts_harvest_step_for_bread() {
        // 只給合成步（模擬便宜腦省略備料，如同既有木頭/石頭案例）→ 正規化應自動補收成步。
        let steps = [CheckedStep::Craft { recipe_id: "bread" }];
        let canon = canonicalize_steps(&steps);
        assert!(
            canon.iter().any(
                |s| matches!(s, PrimStep::Harvest { crop, count } if crop == "wheat" && *count >= 3)
            ),
            "應自動補上收成備料步（3 小麥）：{canon:?}"
        );
        let checked = check_stored_steps(&canon).expect("應過存檔白名單");
        assert!(
            simulate_plan(&checked, &HashMap::new(), vfarm::BREAD_ID, false, false).is_ok(),
            "正規化後應從空背包也能做出麵包"
        );
    }

    #[test]
    fn steps_summary_includes_harvest_line() {
        let summary = steps_summary(&[CheckedStep::Harvest { crop: CropResource::Potato, count: 2 }]);
        assert_eq!(summary, "收成馬鈴薯×2");
    }

    #[test]
    fn harvest_lines_and_no_crop_feed_are_sane() {
        assert!(harvest_line("小麥").contains("小麥"));
        assert!(no_crop_feed("麵包", "小麥").contains("麵包"));
        assert!(no_crop_feed("麵包", "小麥").contains("小麥"));
    }

    #[test]
    fn invention_prompt_mentions_harvest_op() {
        let goal = MaterialGoal { block_id: vfarm::BREAD_ID, name_zh: "麵包" };
        let (sys, _) = invention_prompt("露娜", &goal, "想要麵包", "", false, false, &[]);
        assert!(sys.contains("\"op\":\"harvest\""), "system prompt 應教 harvest op: {sys}");
        assert!(sys.contains("已經熟了"), "應點明作物已熟的前提: {sys}");
    }

    #[test]
    fn ripe_crop_nearby_detects_within_radius_not_beyond() {
        let (fx, fy, fz) = (60, 5, 60);
        let mut world: WorldDelta = WorldDelta::new();
        assert_eq!(ripe_crop_nearby(&world, fx, fy, fz, Block::WheatMature, |_, _| false), None, "還沒有熟作物 → 查無");
        voxel::set_block(&mut world, fx + 3, fy, fz, Block::WheatMature);
        assert_eq!(
            ripe_crop_nearby(&world, fx, fy, fz, Block::WheatMature, |_, _| false),
            Some((fx + 3, fy, fz)),
            "半徑內的熟作物應查得到"
        );
        // 半徑外種熟作物 → 查無（獨立世界，避免上面那格干擾）。
        let far = CROP_NEAR_RADIUS + 5;
        let mut world2: WorldDelta = WorldDelta::new();
        voxel::set_block(&mut world2, fx + far, fy, fz, Block::WheatMature);
        assert_eq!(ripe_crop_nearby(&world2, fx, fy, fz, Block::WheatMature, |_, _| false), None, "太遠不算附近");
        // 作物型別不符 → 查無（找小麥時不誤撿旁邊熟了的胡蘿蔔）。
        let mut world3: WorldDelta = WorldDelta::new();
        voxel::set_block(&mut world3, fx + 3, fy, fz, Block::CarrotMature);
        assert_eq!(
            ripe_crop_nearby(&world3, fx, fy, fz, Block::WheatMature, |_, _| false),
            None,
            "型別不符不該誤撿"
        );
    }

    /// review 修正（PR #1255）：`blocked` 必須推進迴圈裡逐格跳過，不能只擋掉「最近那格」
    /// 就讓整次搜尋放棄——擺一格更近但被擋、一格較遠但合法，斷言回傳的是合法那格。
    #[test]
    fn ripe_crop_nearby_skips_blocked_candidate_keeps_searching() {
        let (fx, fy, fz) = (60, 5, 60);
        let mut world: WorldDelta = WorldDelta::new();
        let blocked_spot = (fx + 2, fy, fz); // 更近，但被擋（模擬落在玩家領地內）
        let allowed_spot = (fx + 6, fy, fz); // 較遠，合法
        voxel::set_block(&mut world, blocked_spot.0, blocked_spot.1, blocked_spot.2, Block::WheatMature);
        voxel::set_block(&mut world, allowed_spot.0, allowed_spot.1, allowed_spot.2, Block::WheatMature);
        assert_eq!(
            ripe_crop_nearby(&world, fx, fy, fz, Block::WheatMature, |x, z| (x, z) == (blocked_spot.0, blocked_spot.2)),
            Some(allowed_spot),
            "最近那格被擋不該讓整次搜尋放棄，該繼續找到旁邊合法的那格"
        );
    }

    // ── find_ripe_crop_far：移動去資源（第八刀）遠距螺旋搜尋 ──────────────────────

    #[test]
    fn find_ripe_crop_far_locates_specific_crop_beyond_near_radius() {
        let (fx, fy, fz) = (60, 5, 60);
        let mut world: WorldDelta = WorldDelta::new();
        let beyond_near = CROP_NEAR_RADIUS + 8;
        assert!(beyond_near <= INVENT_GATHER_RADIUS, "測試前提：夾在鄰近與遠距半徑之間");
        voxel::set_block(&mut world, fx + beyond_near, fy, fz, Block::WheatMature);
        assert_eq!(
            ripe_crop_nearby(&world, fx, fy, fz, Block::WheatMature, |_, _| false),
            None,
            "鄰近檢查本該查無（才輪得到遠距搜尋出場）"
        );
        assert_eq!(
            find_ripe_crop_far(&world, fx, fy, fz, INVENT_GATHER_RADIUS, Block::WheatMature, |_, _| false),
            Some((fx + beyond_near, fy, fz)),
            "遠距螺旋搜尋該找到這畦小麥"
        );
    }

    #[test]
    fn find_ripe_crop_far_type_mismatch_still_none() {
        let (fx, fy, fz) = (60, 5, 60);
        let mut world: WorldDelta = WorldDelta::new();
        let beyond_near = CROP_NEAR_RADIUS + 8;
        voxel::set_block(&mut world, fx + beyond_near, fy, fz, Block::CarrotMature);
        // 遠處確實有熟作物，但型別不對（找小麥卻只有胡蘿蔔）→ 誠實查無，不誤撿。
        assert_eq!(
            find_ripe_crop_far(&world, fx, fy, fz, INVENT_GATHER_RADIUS, Block::WheatMature, |_, _| false),
            None,
            "型別不符不該誤撿"
        );
    }

    /// review 修正（PR #1255）同款：螺旋由近而遠，第一個命中被擋不該讓整次搜尋放棄——這正是
    /// review 描述的「fallback 到 find_ripe_crop_far，第一個命中還是玩家那畦」場景。
    #[test]
    fn find_ripe_crop_far_skips_blocked_candidate_keeps_searching() {
        let (fx, fy, fz) = (60, 5, 60);
        let mut world: WorldDelta = WorldDelta::new();
        let beyond_near = CROP_NEAR_RADIUS + 4;
        let blocked_spot = (fx + beyond_near, fy, fz); // 螺旋較早掃到，但被擋
        let allowed_spot = (fx + beyond_near + 6, fy, fz); // 較遠，合法
        voxel::set_block(&mut world, blocked_spot.0, blocked_spot.1, blocked_spot.2, Block::WheatMature);
        voxel::set_block(&mut world, allowed_spot.0, allowed_spot.1, allowed_spot.2, Block::WheatMature);
        assert_eq!(
            find_ripe_crop_far(
                &world, fx, fy, fz, INVENT_GATHER_RADIUS, Block::WheatMature,
                |x, z| (x, z) == (blocked_spot.0, blocked_spot.2),
            ),
            Some(allowed_spot),
            "螺旋掃到的第一格被擋不該讓整次搜尋放棄，該繼續找到較遠但合法的那格"
        );
    }

    // ── 播種自給（第九刀）：CropResource 種子/翻土映射 ───────────────────────────

    #[test]
    fn crop_seed_id_only_carrot_and_potato() {
        assert_eq!(CropResource::Wheat.seed_id(), None, "小麥種子只從葉片來，居民不採葉片");
        assert_eq!(CropResource::Carrot.seed_id(), Some(vfarm::CARROT_SEEDS_ID));
        assert_eq!(CropResource::Potato.seed_id(), Some(vfarm::POTATO_SEEDS_ID));
        assert_eq!(CropResource::Berry.seed_id(), None, "自種新叢留給更後面一刀");
    }

    #[test]
    fn crop_tillable_block_matches_seed_source() {
        assert_eq!(CropResource::Wheat.tillable_block(), None);
        assert_eq!(CropResource::Carrot.tillable_block(), Some(Block::Grass));
        assert_eq!(CropResource::Potato.tillable_block(), Some(Block::Dirt));
        assert_eq!(CropResource::Berry.tillable_block(), None);
    }

    #[test]
    fn crop_seeded_block_and_kind_roundtrip() {
        assert_eq!(CropResource::Wheat.seeded_block(), Block::FarmSoilSeeded);
        assert_eq!(CropResource::Carrot.seeded_block(), Block::CarrotSeeded);
        assert_eq!(CropResource::Potato.seeded_block(), Block::PotatoSeeded);
        assert_eq!(CropResource::Berry.seeded_block(), Block::BerryBush);
        assert_eq!(CropResource::Wheat.crop_kind(), Some(vfarm::CropKind::Wheat));
        assert_eq!(CropResource::Carrot.crop_kind(), Some(vfarm::CropKind::Carrot));
        assert_eq!(CropResource::Potato.crop_kind(), Some(vfarm::CropKind::Potato));
        assert_eq!(CropResource::Berry.crop_kind(), None, "莓果不走 voxel_farm 計時");
    }

    // ── tillable_ground_nearby：播種自給（第九刀）鄰近翻土地表搜尋 ─────────────────

    #[test]
    fn tillable_ground_nearby_finds_matching_block() {
        // y=300：遠高於地表生成高度（見 `block_at`，地表以上非特殊地標一律回 Air），
        // 避免程序生成的天然草皮/泥土干擾「找不到」斷言（草皮/泥土本身就是常見地表方塊，
        // 不像 WheatMature/Water 等既有測試用的合成方塊天然不會出現）。
        let (fx, fy, fz) = (10, 300, 10);
        let mut world: WorldDelta = WorldDelta::new();
        voxel::set_block(&mut world, fx + 3, fy, fz + 2, Block::Grass);
        assert_eq!(
            tillable_ground_nearby(&world, fx, fy, fz, Block::Grass, |_, _| false),
            Some((fx + 3, fy, fz + 2)),
        );
    }

    #[test]
    fn tillable_ground_nearby_type_mismatch_none() {
        let (fx, fy, fz) = (10, 300, 10); // 高空，同上理由避開天然地表干擾
        let mut world: WorldDelta = WorldDelta::new();
        voxel::set_block(&mut world, fx + 1, fy, fz, Block::Dirt);
        assert_eq!(
            tillable_ground_nearby(&world, fx, fy, fz, Block::Grass, |_, _| false),
            None,
            "泥土不是草皮，不該誤判"
        );
    }

    /// review 修正（PR #1255）同款：草皮到處都是，若在回傳值上事後過濾，居民只要站在玩家
    /// 領地邊上，螺旋第一格幾乎必中領地內；推進迴圈裡才會正確繞開、找到隔壁合法地表。
    #[test]
    fn tillable_ground_nearby_skips_blocked_candidate_keeps_searching() {
        let (fx, fy, fz) = (10, 300, 10); // 高空，避開天然地表干擾
        let mut world: WorldDelta = WorldDelta::new();
        let blocked_spot = (fx + 1, fy, fz); // 螺旋較早掃到，但被擋
        let allowed_spot = (fx + 4, fy, fz); // 較遠，合法
        voxel::set_block(&mut world, blocked_spot.0, blocked_spot.1, blocked_spot.2, Block::Grass);
        voxel::set_block(&mut world, allowed_spot.0, allowed_spot.1, allowed_spot.2, Block::Grass);
        assert_eq!(
            tillable_ground_nearby(
                &world, fx, fy, fz, Block::Grass,
                |x, z| (x, z) == (blocked_spot.0, blocked_spot.2),
            ),
            Some(allowed_spot),
            "螺旋掃到的第一格被擋不該讓整次搜尋放棄，該繼續找到較遠但合法的那格"
        );
    }

    #[test]
    fn tillable_ground_nearby_out_of_radius_none() {
        let (fx, fy, fz) = (10, 300, 10); // 高空，同上理由避開天然地表干擾
        let mut world: WorldDelta = WorldDelta::new();
        voxel::set_block(&mut world, fx + TILL_NEAR_RADIUS + 5, fy, fz, Block::Grass);
        assert_eq!(tillable_ground_nearby(&world, fx, fy, fz, Block::Grass, |_, _| false), None);
    }

    /// 審查點名（PR#1236）：舊版三層迴圈固定順序（最遠角落先掃、命中即回），面對真實
    /// 地形（滿地天然草皮/泥土）會把種子種到地底。改用真實地形（非高空）跑一次，斷言
    /// 選到的是腳邊裸露地表，不是遠處/深層。
    #[test]
    fn tillable_ground_nearby_real_terrain_picks_surface_not_buried() {
        // 找一個乾淨陸地欄位（高於海平面、上方無樹遮擋），比照 voxel.rs 同名測試手法。
        let z = 500;
        let mut fx = 0;
        for cand in 0..20000 {
            let h = voxel::height_at(cand, z);
            if h > 64 && (1..=4).all(|d| voxel::block_at(cand, h + d, z) == Block::Air) {
                fx = cand;
                break;
            }
        }
        let world: WorldDelta = WorldDelta::new(); // 純程序生成地形，無任何 overlay
        let h = voxel::height_at(fx, z);
        let fy = h + 1; // 站在地表上

        // 草皮（胡蘿蔔用）：地表本身就是裸露草皮，該就地找到腳邊，不是遠處角落。
        let spot = tillable_ground_nearby(&world, fx, fy, z, Block::Grass, |_, _| false)
            .expect("腳邊地表就是草皮，該找到");
        assert_eq!(spot, (fx, h, z), "該選到腳邊真地表，不是遠處/深層的巧合命中");
        assert_eq!(
            voxel::effective_block_at(&world, spot.0, spot.1 + 1, spot.2),
            Block::Air,
            "選到的必須是暴露地表（上方是空氣），不是被草皮蓋住的深層方塊"
        );

        // 泥土（馬鈴薯用）：純天然地形的泥土永遠被草皮蓋住、沒有裸露泥土，該誠實查無，
        // 不能像舊版那樣把種子種到看不見的地底。
        assert_eq!(
            tillable_ground_nearby(&world, fx, fy, z, Block::Dirt, |_, _| false),
            None,
            "天然地形沒有裸露泥土，翻土播種該誠實失敗"
        );
    }

    // ── consume_seed：播種消耗（第九刀）───────────────────────────────────────────

    #[test]
    fn consume_seed_decrements_when_available() {
        let mut bag: HashMap<u8, u32> = HashMap::new();
        bag.insert(vfarm::CARROT_SEEDS_ID, 2);
        assert!(consume_seed(&mut bag, vfarm::CARROT_SEEDS_ID));
        assert_eq!(bag[&vfarm::CARROT_SEEDS_ID], 1);
    }

    #[test]
    fn consume_seed_fails_when_absent_or_zero() {
        let mut bag: HashMap<u8, u32> = HashMap::new();
        assert!(!consume_seed(&mut bag, vfarm::CARROT_SEEDS_ID), "沒有這把種子鑰匙");
        bag.insert(vfarm::POTATO_SEEDS_ID, 0);
        assert!(!consume_seed(&mut bag, vfarm::POTATO_SEEDS_ID), "數量 0 不該扣成負數");
    }

    /// 第七刀全鏈模擬（純邏輯側證據，比照第六刀 `full_fish_then_smelt_chain_simulated_execution_reaches_goal`）：
    /// 空背包 → 收成×3（無等待，找到即收）→ 合成 → 後置條件成立（背包真的有麵包）。
    #[test]
    fn full_harvest_then_craft_chain_simulated_execution_reaches_bread() {
        let raw = r#"{"name":"種麵包","steps":[{"op":"harvest","crop":"wheat","count":3},{"op":"craft","recipe":"bread"}]}"#;
        let plan = parse_plan(raw).unwrap();
        let mut run = InventRun::from_plan(vfarm::BREAD_ID, "麵包", &plan, false);
        let mut bag: HashMap<u8, u32> = HashMap::new();
        let mut guard = 0;
        loop {
            guard += 1;
            assert!(guard < 60, "執行應在有限步內收斂");
            match next_action(&run, &bag, no_station) {
                StepAction::DoHarvest { crop } => {
                    *bag.entry(crop.raw_id()).or_insert(0) += 1;
                }
                StepAction::Advance => run.step_idx += 1,
                StepAction::DoCraft { recipe_id } => {
                    let r = vcraft::find_recipe(recipe_id).unwrap();
                    assert!(craft_apply(&mut bag, r), "照計畫備好料，合成應成功");
                    run.step_idx += 1;
                }
                StepAction::Done => break,
                other => unreachable!("此計畫（收成→合成）不該走到 {other:?}"),
            }
        }
        assert!(goal_met(&bag, vfarm::BREAD_ID), "後置條件：背包真的有麵包");
    }

    #[test]
    fn laddered_pick_can_reach_bread() {
        // 回歸鎖：比照第六刀（漁獲）曾讓 base_resource_ids 與 obtainable_ids 分岔的教訓——
        // 本刀從一開始就共用 one_step_resource_ids，這裡釘住小麥與麵包不會重蹈覆轍。
        let none = HashSet::new();
        assert!(
            goal_reach_cost(vfarm::BREAD_ID, &none).is_some(),
            "小麥應與其他一步可得資源同屬 base_resource_ids，麵包成本不該恆為 None"
        );
        // 麵包不需站台（2×2 隨身配方），零技能時就該落在步數上限內——不像烤魚/拋光石
        // 得先蓋熔爐才搆得著。
        let cost = goal_reach_cost(vfarm::BREAD_ID, &none);
        assert!(
            cost.map_or(false, |c| c <= MAX_STEPS),
            "零技能時麵包應已落在步數上限內，實得 {cost:?}"
        );
        let cat = possibility_catalog(&HashSet::new());
        let picked_bread = (0..cat.len() as u64)
            .any(|seed| curiosity_pick_laddered(&cat, &none, seed).map(|g| g.block_id) == Some(vfarm::BREAD_ID));
        assert!(picked_bread, "零技能時，階梯好奇心掃過真實目錄的 seed 應能挑到麵包");
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

    /// 修「空玻璃瓶／乙太沃肥」發明永遠失敗的根因：`material_name` 此前只有一張
    /// 8 個 id 的手寫小表，鏈上中繼加工品（玻璃/石磚）與居民自製品目標
    /// （空玻璃瓶/乙太沃肥）一律落回泛稱「材料」，便宜腦看不懂到底缺什麼、
    /// goal_met 失敗訊息也講不出具體材料名。改委派 `voxel_gift::item_name_zh`
    /// 後應覆蓋到這些 id，不再是泛稱。
    #[test]
    fn material_name_covers_chain_intermediates_and_own_products() {
        assert_eq!(material_name(10), "玻璃"); // 鏈上中繼加工品（玻璃瓶/水井藍圖的配料）
        assert_eq!(material_name(9), "石磚"); // 鏈上中繼加工品（瞭望台藍圖的配料）
        assert_eq!(material_name(17), "拋光石"); // 熔爐產物，非站點特例仍要有正確名
        assert_eq!(material_name(crate::voxel_bottle::BOTTLE_ID), "空玻璃瓶");
        assert_eq!(material_name(crate::voxel_compost::FERTILIZER_ID), "乙太沃肥");
    }

    /// `recipe_fact`（透過 `invention_prompt` 節錄）此前對玻璃這種鏈上配料一律印出
    /// 「材料×2」，便宜腦完全看不出「bottle」配方到底要吃什麼——本測試釘住系統
    /// 提示裡「空玻璃瓶」配方那行必須點名「玻璃」，不能再淪為泛稱。
    #[test]
    fn invention_prompt_names_chain_ingredient_not_generic() {
        let goal = MaterialGoal { block_id: crate::voxel_bottle::BOTTLE_ID, name_zh: "空玻璃瓶" };
        let (sys, _) = invention_prompt("露娜", &goal, "想要空玻璃瓶", "", false, false, &[]);
        assert!(sys.contains("玻璃×2"), "bottle 配方事實行應點名玻璃，而非泛稱「材料」: {sys}");
        assert!(!sys.contains("材料×2"), "不該再出現泛稱材料的配方事實行: {sys}");
    }

    /// 目標材料是居民自製品（如空玻璃瓶/乙太沃肥）時，計畫跑完仍缺料的失敗訊息
    /// 此前會講「背包裡仍然不會有目標材料「材料」」——便宜腦收到這句等於沒收到
    /// 有效回饋、修正必然再失敗。改用委派後應點名真正的目標材料。
    #[test]
    fn goal_unmet_message_names_self_made_goal() {
        let bag: HashMap<u8, u32> = HashMap::new();
        let err = simulate_plan(&[], &bag, crate::voxel_bottle::BOTTLE_ID, false, false)
            .expect_err("空背包、零步驟，目標不可能達成");
        assert!(err.contains("空玻璃瓶"), "{err}");
        assert!(!err.contains("「材料」"), "{err}");
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
        let (sys, user) = invention_prompt("露娜", &goal, "好想要一塊玻璃", "木頭×1", false, false, &[]);
        // 原語白名單與嚴格輸出格式都在 system。
        assert!(sys.contains("gather") && sys.contains("craft"));
        assert!(sys.contains("JSON"));
        // grounded 配方事實：玻璃那條一定在（2 沙 → 玻璃）。
        // 材料名單一事實源改委派 voxel_gift::item_name_zh 後，沙子統一稱「沙」
        // （原本 input_name 私有小表寫的「沙子」只是這支 prompt 自己的措辭，
        // 不影響配方語意，改斷言貼合唯一事實源的真實輸出）。
        assert!(sys.contains("glass") && sys.contains("沙"));
        // 處境與背包現況在 user。
        assert!(user.contains("玻璃") && user.contains("木頭×1"));
    }

    #[test]
    fn invention_prompt_teaches_workbench_chain() {
        let goal = MaterialGoal { block_id: FURNACE_BLOCK_ID, name_zh: "熔爐" };
        let (sys, user) = invention_prompt("露娜", &goal, "想要一座熔爐", "", false, false, &[]);
        // 新原語與工作台規則都在 system。
        assert!(sys.contains("craft_wb") && sys.contains("place"));
        assert!(sys.contains("workbench"), "要教她工作台配方 id");
        // grounded 工作台配方事實：熔爐那條一定在（8 石 → 熔爐）。
        assert!(sys.contains("furnace_wb") && sys.contains("石頭"));
        // 第五刀（鑿井尋礦）後鐵錠在鏈上 → 鐵鎬也進了可發明配方節錄。
        assert!(sys.contains("iron_pickaxe"), "鐵鎬第五刀後在鏈上，應列給她");
        // user 帶「附近沒有工作台」的世界事實。
        assert!(user.contains("沒有工作台"));
        let (_, user2) = invention_prompt("露娜", &goal, "想要一座熔爐", "", true, false, &[]);
        assert!(user2.contains("已經有一座放置好的工作台"));
    }

    /// 修根因：目標材料就是「熔爐」本身時，prompt 不該教她 place（place 會把它從背包
    /// 放到地上、消耗掉，反而讓「背包裡有熔爐」這個目標永遠達不成——這正是修這支
    /// prompt 前，居民想要熔爐卻總是失敗的根因（見 furnace_chain_json：正確計畫本就
    /// 停在 craft_wb，不含 place）。
    #[test]
    fn invention_prompt_furnace_goal_says_dont_place() {
        let goal = MaterialGoal { block_id: FURNACE_BLOCK_ID, name_zh: "熔爐" };
        let (sys, _) = invention_prompt("露娜", &goal, "想要一座熔爐", "", false, false, &[]);
        assert!(sys.contains("你的目標材料正是「熔爐」本身"), "{sys}");
        assert!(sys.contains("先別 place"), "{sys}");
        // 工作台在這裡仍是中繼站（她還得先放工作台才能 craft_wb），教法不變。
        assert!(sys.contains("才能接著用 craft_wb 做 3×3 合成"), "{sys}");
    }

    /// 同理：目標材料是「工作台」本身時，工作台那條教法也該改口別 place；
    /// 熔爐維持原本「拿來當中繼站」的教法（她可能還想接著 craft_wb/smelt 別的東西）。
    #[test]
    fn invention_prompt_workbench_goal_says_dont_place() {
        let goal = MaterialGoal { block_id: WORKBENCH_BLOCK_ID, name_zh: "工作台" };
        let (sys, _) = invention_prompt("露娜", &goal, "想要一座工作台", "", false, false, &[]);
        assert!(sys.contains("你的目標材料正是「工作台」本身"), "{sys}");
        assert!(sys.contains("先別 place"), "{sys}");
        assert!(sys.contains("才能接著 smelt"), "{sys}");
    }

    /// 目標材料是別的東西（如玻璃）時，工作台／熔爐若被提到，仍是「拿來當中繼站」——
    /// 兩條教法都該保留 place（附近沒有就得先放好才能繼續下一步）。
    #[test]
    fn invention_prompt_non_site_goal_keeps_place_guidance() {
        let goal = MaterialGoal { block_id: 10, name_zh: "玻璃" };
        let (sys, _) = invention_prompt("露娜", &goal, "好想要一塊玻璃", "", false, false, &[]);
        assert!(sys.contains("才能接著用 craft_wb 做 3×3 合成"), "{sys}");
        assert!(sys.contains("才能接著 smelt"), "{sys}");
        assert!(!sys.contains("先別 place"), "{sys}");
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

    // ── 好奇心自主學習（北極星第三刀）────────────────────────────────────────

    #[test]
    fn possibility_catalog_lists_inventables_and_dedups() {
        let cat = possibility_catalog(&HashSet::new());
        let ids: Vec<u8> = cat.iter().map(|g| g.block_id).collect();
        // 可發明鏈上的代表都在：木板(8)、梯子(35)、熔爐(16)、箱子(42)、釣竿(60)。
        for want in [8u8, 35, 16, 42, 60] {
            assert!(ids.contains(&want), "目錄應含 id {want}：{ids:?}");
        }
        // 第五刀（鑿井尋礦）後礦石可採：火把(31)、鐵鎬也自動長進目錄。
        for want in [31u8, vcraft::PICKAXE_IRON_ID] {
            assert!(ids.contains(&want), "id {want} 第五刀後在鏈上，應進目錄：{ids:?}");
        }
        // 第六刀（漁獲入自採閉包）後小魚可釣：烤魚（熔爐產物）也該進目錄——
        // 上一版漏了 chain `inventable_furnace_recipes()`，熔爐產物從沒進過目錄。
        assert!(
            ids.contains(&crate::voxel_fishing::COOKED_FISH_ID),
            "烤魚（熔爐產物）第六刀後應進目錄：{ids:?}"
        );
        // 第七刀（作物入自採閉包）後小麥可收成：麵包(19)也自動長進目錄。
        assert!(
            ids.contains(&crate::voxel_farm::BREAD_ID),
            "麵包（3 小麥）第七刀後應進目錄：{ids:?}"
        );
        // 不可發明的誠實不列：床(45 要葉片)、冰晶燈(57)。
        for bad in [45u8, 57] {
            assert!(!ids.contains(&bad), "id {bad} 她備不了料，不該進目錄");
        }
        // 第十刀（莓果入自採閉包）後莓果醬（熔爐產物）也該進目錄。
        assert!(
            ids.contains(&vberry::JAM_ID),
            "莓果醬第十刀後應進目錄：{ids:?}"
        );
        // 去重：同產物（木板 8 同時是 2×2 與 3×3 產物）只列一次。
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len(), "目錄不得有重複產物");
    }

    #[test]
    fn possibility_catalog_excludes_known() {
        // 她已會做木板(8)與玻璃(10) → 目錄排掉這兩樣（知識隨著會的變多而變短）。
        let known: HashSet<u8> = [8u8, 10].into_iter().collect();
        let cat = possibility_catalog(&known);
        assert!(cat.iter().all(|g| g.block_id != 8 && g.block_id != 10));
        // 沒排掉的仍在。
        assert!(cat.iter().any(|g| g.block_id == 42), "箱子還沒會，應仍在目錄");
    }

    #[test]
    fn curiosity_desire_roundtrips_through_detection() {
        // 全鏈關鍵不變量：對目錄裡**每一樣**，好奇心種下的心願文字必被
        // detect_missing_material 接住、且目標 id 一致——發明引擎保證接得了手。
        for g in possibility_catalog(&HashSet::new()) {
            let desire = curiosity_desire_text(g.name_zh);
            let hit = detect_missing_material(&desire)
                .unwrap_or_else(|| panic!("「{desire}」應被偵測到"));
            assert_eq!(hit.block_id, g.block_id, "「{}」偵測到的 id 應一致", g.name_zh);
        }
    }

    #[test]
    fn detect_missing_material_dynamic_covers_new_products() {
        // 動態擴充：手工關鍵詞表沒有的可發明產物（梯子/木鎬/釣竿）也接得住。
        assert_eq!(detect_missing_material("好想要一把梯子").map(|g| g.block_id), Some(35));
        assert_eq!(
            detect_missing_material("要是有木鎬就好了").map(|g| g.block_id),
            Some(vcraft::PICKAXE_WOOD_ID)
        );
        assert_eq!(detect_missing_material("我想要釣竿去釣魚").map(|g| g.block_id), Some(60));
        // 第五刀（鑿井尋礦）後煤礦可採 → 火把第一次可觸發發明。
        assert_eq!(detect_missing_material("好想要火把").map(|g| g.block_id), Some(31));
        // 不可發明的仍誠實不觸發（床要葉片——她備不了料，別種死願）。
        assert!(detect_missing_material("好想要一張床").is_none());
        // 一般詩意心願照舊不誤觸發。
        assert!(detect_missing_material("願市集的水果攤永遠新鮮").is_none());
    }

    #[test]
    fn detect_missing_material_prefers_longer_specific_match_over_substring_keyword() {
        // 漂流瓶 v1 回歸測試：「空玻璃瓶」內含手工關鍵詞表的「玻璃」子字串，
        // 但應偵測到更具體的瓶子(83)本身，而非被短關鍵詞搶先誤判成玻璃(10)。
        assert_eq!(
            detect_missing_material("好想自己做出一個空玻璃瓶試試").map(|g| g.block_id),
            Some(crate::voxel_bottle::BOTTLE_ID)
        );
        // 沒有更長的具體產物名時，手工關鍵詞表仍照舊運作。
        assert_eq!(detect_missing_material("好想要一扇玻璃窗").map(|g| g.block_id), Some(10));
    }

    #[test]
    fn curiosity_pick_is_deterministic() {
        let cat = possibility_catalog(&HashSet::new());
        // 同種子同結果（可重現）；不同種子可覆蓋整份目錄。
        assert_eq!(curiosity_pick(&cat, 7), curiosity_pick(&cat, 7));
        assert_eq!(
            curiosity_pick(&cat, 3).map(|g| g.block_id),
            Some(cat[3 % cat.len()].block_id)
        );
        // 空目錄（全會了）→ None，呼叫端冒「沒新東西想試」泡泡、零 LLM。
        assert!(curiosity_pick(&[], 42).is_none());
    }

    // ── 階梯式好奇心（真進化·踏腳石）測試 ─────────────────────────────────────

    #[test]
    fn reach_cost_cheap_handheld_goals_are_low() {
        let none = HashSet::new();
        // 木板（8）＝採木×1 + 合成×1 → 2 op；石磚（9）＝採石 + 合成 → 2 op。
        assert_eq!(goal_reach_cost(8, &none), Some(2));
        assert_eq!(goal_reach_cost(9, &none), Some(2));
        // 工作台（15）＝採木→合木板→合工作台 → 3 op（隨身配方、免站台）。
        assert_eq!(goal_reach_cost(WORKBENCH_BLOCK_ID, &none), Some(3));
    }

    #[test]
    fn reach_cost_deep_furnace_chain_exceeds_budget_from_scratch() {
        // 拋光石（17）需熔爐冶煉；零技能時要 造工作台→放→合熔爐→放→採石→冶煉，
        // 估算 op 數 > MAX_STEPS，因此**從零搆不著**——正是線上居民猛撞牆的那種目標。
        let cost = goal_reach_cost(17, &HashSet::new());
        assert!(
            cost.map_or(false, |c| c > MAX_STEPS),
            "拋光石從零應估為超出步數上限，實得 {cost:?}"
        );
    }

    #[test]
    fn reach_cost_drops_once_prerequisite_skill_learned() {
        // 階梯核心：一旦她發明過「熔爐」技能（收成 1 步 use_skill），拋光石的估算
        // 成本大幅下降到 MAX_STEPS 內——複雜目標因前置技能到位而**變得搆得著**。
        let mut known = HashSet::new();
        known.insert(FURNACE_BLOCK_ID); // 她已會做熔爐
        let cost = goal_reach_cost(17, &known);
        assert!(
            cost.map_or(false, |c| c <= MAX_STEPS),
            "會做熔爐後拋光石應落在步數上限內，實得 {cost:?}"
        );
        // 且比從零便宜（積木收成 use_skill 一步）。
        assert!(cost < goal_reach_cost(17, &HashSet::new()));
    }

    #[test]
    fn laddered_pick_prefers_reachable_over_unreachable() {
        // 手搭一份含「搆得著（木板）」與「搆不著（拋光石）」的目錄，零技能。
        let cat = vec![
            MaterialGoal { block_id: 17, name_zh: "拋光石" }, // 從零超上限
            MaterialGoal { block_id: 8, name_zh: "木板" },    // 2 op，搆得著
        ];
        let none = HashSet::new();
        // 掃過各種 seed，永遠只挑到搆得著的那個（不會挑到會撞牆的拋光石）。
        for seed in 0..20u64 {
            let picked = curiosity_pick_laddered(&cat, &none, seed).unwrap();
            assert_eq!(picked.block_id, 8, "seed={seed} 竟挑到搆不著的目標");
        }
    }

    #[test]
    fn laddered_pick_falls_back_when_nothing_reachable() {
        // 目錄裡全是搆不著的目標（只放拋光石）→ 不卡死，退回原均勻挑（零回歸）。
        let cat = vec![MaterialGoal { block_id: 17, name_zh: "拋光石" }];
        let none = HashSet::new();
        assert_eq!(curiosity_pick_laddered(&cat, &none, 3).map(|g| g.block_id), Some(17));
        // 空目錄 → None（她全會了）。
        assert!(curiosity_pick_laddered(&[], &none, 7).is_none());
    }

    #[test]
    fn laddered_pick_is_deterministic() {
        let cat = possibility_catalog(&HashSet::new());
        let none = HashSet::new();
        assert_eq!(
            curiosity_pick_laddered(&cat, &none, 11),
            curiosity_pick_laddered(&cat, &none, 11)
        );
    }

    #[test]
    fn laddered_pick_can_reach_cooked_fish() {
        // 回歸鎖：第六刀（漁獲入自採閉包）曾讓 base_resource_ids 與 obtainable_ids 的種子集
        // 分岔（小魚只補進後者）→ material_op_cost(小魚) 恆為 None → goal_reach_cost(烤魚)
        // 恆為 None，無論 known_goals 是什麼，階梯好奇心永遠挑不到烤魚——「居民自己想
        // 釣魚」在自走路徑上根本不會發生。
        let none = HashSet::new();
        assert!(
            goal_reach_cost(crate::voxel_fishing::COOKED_FISH_ID, &none).is_some(),
            "小魚應與其他一步可得資源同屬 base_resource_ids，烤魚成本不該恆為 None"
        );
        // 烤魚要冶煉（需熔爐），零技能時跟拋光石/鐵錠同一類——從零連熔爐一起蓋超出步數
        // 上限，誠實不算「當前踏得到」（見 reach_cost_deep_furnace_chain_exceeds_budget_from_scratch）。
        assert!(
            goal_reach_cost(crate::voxel_fishing::COOKED_FISH_ID, &none)
                .map_or(false, |c| c > MAX_STEPS),
            "零技能時烤魚（連熔爐一起蓋）應超出步數上限，與拋光石/鐵錠同一階梯"
        );
        // 階梯核心：一旦她已會做熔爐（收成 1 步 use_skill），烤魚的估算成本大幅下降到
        // 步數上限內——這時真實目錄＋階梯好奇心應該真的挑得到烤魚。
        let mut known = HashSet::new();
        known.insert(FURNACE_BLOCK_ID);
        let cost = goal_reach_cost(crate::voxel_fishing::COOKED_FISH_ID, &known);
        assert!(
            cost.map_or(false, |c| c <= MAX_STEPS),
            "會做熔爐後烤魚應落在步數上限內，實得 {cost:?}"
        );
        let cat = possibility_catalog(&HashSet::new());
        let picked_fish = (0..cat.len() as u64).any(|seed| {
            curiosity_pick_laddered(&cat, &known, seed).map(|g| g.block_id)
                == Some(crate::voxel_fishing::COOKED_FISH_ID)
        });
        assert!(picked_fish, "會做熔爐後，階梯好奇心掃過真實目錄的 seed 應能挑到烤魚");
    }

    #[test]
    fn curiosity_idle_requires_truly_free() {
        assert!(curiosity_idle(false, false, false, 0.0), "全閒＋冷卻到 → 可好奇");
        assert!(curiosity_idle(false, false, false, -3.0));
        assert!(!curiosity_idle(true, false, false, 0.0), "發明中不好奇");
        assert!(!curiosity_idle(false, true, false, 0.0), "跑腿中不好奇");
        assert!(!curiosity_idle(false, false, true, 0.0), "建造中不好奇");
        assert!(!curiosity_idle(false, false, false, 10.0), "發明冷卻中不好奇");
    }

    #[test]
    fn curiosity_gate_threshold() {
        assert!(curiosity_gate(0.0));
        assert!(curiosity_gate(CURIOSITY_CHANCE - 0.01));
        assert!(!curiosity_gate(CURIOSITY_CHANCE), "等於門檻不過（嚴格小於）");
        assert!(!curiosity_gate(1.0));
    }

    #[test]
    fn curiosity_intervals_stagger_and_scale() {
        // 錯開：後面的居民初值更大（不同 tick 全員一起好奇）。
        let base = CURIOSITY_INTERVAL_SECS;
        assert_eq!(curiosity_interval_for(0, base), base);
        assert!(curiosity_interval_for(1, base) > curiosity_interval_for(0, base));
        // 比例式：測試模式縮短基準時，錯開間距同步縮短（整條鏈可在測試內等到）。
        let fast = curiosity_interval_for(3, 20.0);
        assert!(fast < 60.0, "縮短基準後第 4 位也應在一分鐘內就緒：{fast}");
    }

    #[test]
    fn catalog_note_injects_names_and_caps() {
        let cat = possibility_catalog(&HashSet::new());
        let note = catalog_note(&cat).expect("目錄非空應有注入段");
        assert!(note.contains("你還不會做"), "{note}");
        assert!(note.contains(cat[0].name_zh), "至少列出第一樣：{note}");
        // 上限：最多列 CATALOG_NOTE_MAX_ITEMS 樣（數「、」分隔數）。
        let listed = note.matches('、').count() + 1;
        assert!(listed <= CATALOG_NOTE_MAX_ITEMS, "列了 {listed} 樣，超過上限");
        // 目錄空（全會了）→ 不注入、不花 token。
        assert!(catalog_note(&[]).is_none());
    }

    #[test]
    fn known_goals_for_collects_per_resident() {
        let mut store = InventedSkillStore::new();
        store.add("vox_res_0", "燒玻璃", 10, vec![]);
        store.add("vox_res_0", "合木板", 8, vec![]);
        store.add("vox_res_1", "造箱子", 42, vec![]);
        let known = store.known_goals_for("vox_res_0");
        assert_eq!(known, [10u8, 8].into_iter().collect::<HashSet<u8>>());
        assert_eq!(store.known_goals_for("vox_res_2"), HashSet::new(), "沒學過＝空集合");
    }

    #[test]
    fn curiosity_texts_mention_goal_and_are_nonempty() {
        assert!(curiosity_desire_text("梯子").contains("梯子"));
        assert!(curiosity_line("梯子").contains("梯子"));
        assert!(curiosity_feed("梯子").contains("好奇心"));
        assert!(curiosity_memory("梯子").contains("沒有人教我"));
        assert!(!nothing_new_line().is_empty());
    }

    // ── 退避台詞（#972 防鬼打牆）────────────────────────────────────────────────

    #[test]
    fn backoff_texts_mention_goal_and_resource() {
        // 資源找不到時 Feed 帶資源名。
        let f = backoff_no_resource_feed("釣竿", "木頭");
        assert!(f.contains("木頭"), "應提及缺少的資源：{f}");
        assert!(f.contains("釣竿"), "應提及目標：{f}");
        // 資源名空時退化為不提資源的版本。
        let f2 = backoff_no_resource_feed("釣竿", "");
        assert!(f2.contains("釣竿"), "應提及目標：{f2}");
        assert!(!f2.is_empty());
        // 換目標冒泡/Feed 帶目標名。
        assert!(backoff_switch_line("釣竿").contains("釣竿"));
        assert!(backoff_switch_feed("釣竿").contains("釣竿"));
        // Feed 帶門檻數，呼應「連試 N 次」。
        assert!(backoff_switch_feed("釣竿").contains(&INVENT_BACKOFF_THRESHOLD.to_string()));
    }

    #[test]
    fn backoff_constants_are_sane() {
        // 門檻 ≥ 2（至少讓她試兩次，不過度敏感）。
        assert!(INVENT_BACKOFF_THRESHOLD >= 2);
        // 退避時間 ≥ 1 小時（讓她有足夠時間換方向探索，不是秒回頭再撞）。
        assert!(INVENT_BACKOFF_SECS >= 3600.0);
    }

    #[test]
    fn note_fail_backoff_triggers_at_threshold_and_resets() {
        // 從零開始累計：未達門檻前每次回 false，計數遞增。
        let mut count: u8 = 0;
        let mut trips = 0;
        for _ in 1..INVENT_BACKOFF_THRESHOLD {
            assert!(!note_fail_should_backoff(&mut count), "未達門檻不該退避");
            trips += 1;
        }
        assert_eq!(count, trips, "未退避前計數應等於失敗次數");
        // 第 THRESHOLD 次失敗 → 回 true 並歸零（退避到期後可重新累計）。
        assert!(note_fail_should_backoff(&mut count), "達門檻應退避");
        assert_eq!(count, 0, "退避後計數歸零，供到期重新累計");
    }

    #[test]
    fn note_fail_backoff_saturates_not_panics() {
        // 極端：計數已在 u8 上限也不 panic（saturating_add）；達門檻仍歸零退避。
        let mut count: u8 = u8::MAX;
        assert!(note_fail_should_backoff(&mut count), "已達上限視為超過門檻，退避");
        assert_eq!(count, 0);
    }

    #[test]
    fn gather_radius_is_larger_than_default() {
        // 發明採集半徑應大於日常採集半徑，讓她能找到較遠的資源。
        use crate::voxel_skills::GATHER_MAX_RADIUS;
        assert!(
            INVENT_GATHER_RADIUS > GATHER_MAX_RADIUS,
            "發明採集半徑 {INVENT_GATHER_RADIUS} 應大於日常半徑 {GATHER_MAX_RADIUS}"
        );
    }

    #[test]
    fn catalog_excludes_backoff_goals() {
        // 驗證退避目標（如 goal_block_id=60 釣竿）被加進 excluded 後目錄就不含它。
        let all = possibility_catalog(&HashSet::new());
        // 找一個目錄裡有的 id 來模擬「目前退避中」。
        if let Some(backoff_goal) = all.first() {
            let bid = backoff_goal.block_id;
            let mut excluded = HashSet::new();
            excluded.insert(bid);
            let filtered = possibility_catalog(&excluded);
            assert!(
                filtered.iter().all(|g| g.block_id != bid),
                "退避目標 {bid} 應從目錄排除"
            );
        }
    }

    // ── 鑿井尋礦 v1（第五刀）：礦石進閉包、鐵系鏈開放 ────────────────────────────

    #[test]
    fn ore_tokens_roundtrip_and_underground() {
        for res in [GatherResource::CoalOre, GatherResource::IronOre] {
            assert_eq!(resource_from_token(token_of(res)), Some(res), "token 雙向一致");
            assert!(resource_is_underground(res), "礦石屬地下資源、走礦井");
        }
        assert_eq!(resource_from_token("煤礦"), Some(GatherResource::CoalOre));
        assert_eq!(resource_from_token("鐵礦"), Some(GatherResource::IronOre));
        // 「鐵」單字與鐵錠/鐵磚歧義，誠實不收（逼便宜腦寫明確）。
        assert_eq!(resource_from_token("鐵"), None);
    }

    #[test]
    fn iron_chain_now_obtainable() {
        // 第五刀核心：礦石進種子集 → 火把、鐵錠、紅陶磚整條鐵系鏈自動落進閉包。
        let ids = obtainable_ids();
        for b in [Block::CoalOre, Block::IronOre, Block::IronIngot, Block::Torch] {
            assert!(ids.contains(&(b as u8)), "{b:?} 應在第五刀後的可取得閉包");
        }
        // 鐵錠（熔爐冶煉）成為可發明配方 → 心願提到「鐵錠」第一次觸發發明。
        let g = detect_missing_material("好想要一塊鐵錠，聽說又硬又亮").expect("鐵錠應可觸發發明");
        assert_eq!(g.block_id, Block::IronIngot as u8);
        // 好奇心「可能性目錄」也自動長出火把——她們會自己想去學。
        let catalog = possibility_catalog(&HashSet::new());
        assert!(catalog.iter().any(|g| g.block_id == Block::Torch as u8), "目錄應含火把");
    }

    #[test]
    fn torch_plan_with_ore_gather_simulates_ok() {
        // 火把（1 木＋1 煤礦）：第五刀前 coal_ore 連白名單都過不了；現在全鏈可行。
        let steps = vec![
            PrimStep::Gather { resource: "wood".into(), count: 1 },
            PrimStep::Gather { resource: "coal_ore".into(), count: 1 },
            PrimStep::Craft { recipe: "torch".into() },
        ];
        let checked = check_steps(&steps).expect("礦石採集步應過白名單");
        let bag: HashMap<u8, u32> = HashMap::new();
        assert!(simulate_plan(&checked, &bag, Block::Torch as u8, false, false).is_ok());
    }

    #[test]
    fn ore_well_feed_lines_mention_goal_and_ore() {
        let feed = ore_well_feed("火把", "煤礦");
        assert!(feed.contains("火把") && feed.contains("煤礦") && !feed.contains("{"));
        let line = ore_well_line("鐵礦");
        assert!(line.contains("鐵礦") && !line.contains("{"));
    }
}
