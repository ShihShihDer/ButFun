//! 乙太方界·合成台 v1 + 工作台 3×3 v1 + 熔爐 v1 + 鎬具 v1 + 斧頭 v1 + 鏟子 v1
//! （ROADMAP 658/665/666/687/689/690）。
//!
//! **玩家有感**：挖了木頭可合成更工整的木板（2×2），合出工作台後放置到世界→
//! 右鍵互動開 3×3 格→合成大量物品或混合配方；合出熔爐→放置→右鍵開冶煉面板→
//! 把石頭冶煉成拋光石（獨特建材）——「採集→合成→冶煉→建造」循環更深。
//! 鎬具 v1（ROADMAP 687）：木鎬/石鎬（背包 2×2）＋鐵鎬（工作台 3×3）合成；
//! 持鎬採石/礦時，前端採礦手感大幅加速——療癒循環再加一圈。
//! 斧頭 v1（ROADMAP 689）：木斧/石斧（背包 2×2）＋鐵斧（工作台 3×3）合成；
//! 持斧砍木頭/葉片/木板時前端大幅加速，讓砍樹第一次有「工具感」。
//! 鏟子 v1（ROADMAP 690）：木鏟/石鏟（背包 2×2）＋鐵鏟（工作台 3×3）合成；
//! 持鏟挖草地/泥土/沙地/農田時前端大幅加速，完成採集三件套（鎬+斧+鏟）。
//!
//! **純邏輯層**：`Recipe` 表 + `find_recipe` + `find_workbench_recipe` +
//! `find_furnace_recipe` + `find_any_recipe` + `can_craft`，確定性、無副作用、全可測。
//! 鎖 / WS / IO 全在 `voxel_ws.rs`，本模組零 async、零鎖、零 IO。
//!
//! **成本鐵律**：零 LLM、零 migration、零新協議欄位（WS "craft" 訊息 additive）。

// ── 鎬具物品 ID（純物品，不可放置於世界，與 Block enum 分開）─────────────────────
/// 木鎬（ROADMAP 687）——物品 ID 32；不可放置，合成後住在物品欄。
pub const PICKAXE_WOOD_ID: u8 = 32;
/// 石鎬（ROADMAP 687）——物品 ID 33。
pub const PICKAXE_STONE_ID: u8 = 33;
/// 鐵鎬（ROADMAP 687）——物品 ID 34，需工作台合成。
pub const PICKAXE_IRON_ID: u8 = 34;

// ── 斧頭物品 ID（純物品，不可放置於世界，ROADMAP 689）─────────────────────────────
/// 木斧（ROADMAP 689）——物品 ID 36；砍木頭/葉片/木板加速 2.5×。
pub const AXE_WOOD_ID: u8 = 36;
/// 石斧（ROADMAP 689）——物品 ID 37；砍木材加速 4×。
pub const AXE_STONE_ID: u8 = 37;
/// 鐵斧（ROADMAP 689）——物品 ID 38；砍木材加速 6×，需工作台合成。
pub const AXE_IRON_ID: u8 = 38;

// ── 鏟子物品 ID（純物品，不可放置於世界，ROADMAP 690）─────────────────────────────
/// 木鏟（ROADMAP 690）——物品 ID 39；挖草地/泥土/沙地/農田加速 2.5×。
pub const SHOVEL_WOOD_ID: u8 = 39;
/// 石鏟（ROADMAP 690）——物品 ID 40；挖軟方塊加速 4×。
pub const SHOVEL_STONE_ID: u8 = 40;
/// 鐵鏟（ROADMAP 690）——物品 ID 41；挖軟方塊加速 6×，需工作台合成。
pub const SHOVEL_IRON_ID: u8 = 41;

// ── 料理物品 ID（純物品，不可放置於世界，ROADMAP 778）─────────────────────────────
/// 野菜暖湯（ROADMAP 778）——物品 ID 67；在工作台把胡蘿蔔＋馬鈴薯＋小麥三種親手種的
/// 作物拌在一起煮成的一鍋暖心料理。純物品、不可放置。這是乙太方界第一道**多食材料理**：
/// 烤魚(63)、烤地薯(64) 都是單一食材下爐，而暖湯要湊齊三種作物才煮得成——最費心、也最療癒，
/// 是「種田→料理→餽贈→享用」循環的頂點，居民收到時比任何食物都珍視。
pub const STEW_ID: u8 = 67;

// ── 獨門配方物品 ID（純物品，不可放置於世界，居民教你一道獨門配方 v1，自主提案切片）───
/// 護身符（ROADMAP 849）——物品 ID 97；居民好感夠深時主動教你的獨門配方，貼身佩戴的
/// 心意信物。純物品、不可放置。96（野花藍花）是目前最大已用 id，97 是首個空號。
pub const AMULET_ID: u8 = 97;

// ── 貨幣物品 ID（純物品，不可放置於世界，乙太幣 v1，自主提案切片）───────────────────
/// 乙太幣（ROADMAP 873）——物品 ID 98；玩家↔玩家至今唯一的交換手段是以物易物
/// （自由市集 832），沒有一種**通用**、任何人都想要的媒介——想換到手上沒有的東西，
/// 得先剛好找到「對方想要的正是我有的」那個人。乙太幣把常見的原礦資源鑄成一枚
/// 可攜帶、任何人都認得的信物，讓「先賺、再買我真正想要的」第一次可行。純物品、
/// 不可放置。97（護身符）是目前最大已用 id，98 是首個空號。
pub const COIN_ID: u8 = 98;

// ── 武器物品 ID（純物品，不可放置於世界，驅影之劍 v1，自主提案切片）───────────────────
/// 木劍（ROADMAP 887）——物品 ID 99；世界至今有鎬/斧/鏟三件採集工具，卻沒有一件**武器**——
/// 夜裡驅散暗影只能徒手挖擊、固定三下，備不備戰毫無差別。木劍是第一把武器：握在手上驅散
/// 暗影一下抵兩下（三下→兩下）。純物品、不可放置。98（乙太幣）是目前最大已用 id，99 是首個空號。
pub const SWORD_WOOD_ID: u8 = 99;
/// 石劍（ROADMAP 887）——物品 ID 100；比木劍更利，握在手上一擊即驅散暗影（一下抵三下）。
pub const SWORD_STONE_ID: u8 = 100;
/// 鐵劍（ROADMAP 887）——物品 ID 101，需工作台合成；一擊驅散暗影，且驅散時暗影多凝一枚
/// 乙太礦（雙倍溫柔獎勵）——精煉武器的頂點回報。純物品、不可放置。
pub const SWORD_IRON_ID: u8 = 101;

// ── 載具物品 ID（純物品，不可放置於世界，蒸汽獨輪車 v1，自主提案切片）───────────────
/// 蒸汽獨輪車（ROADMAP 976）——物品 ID 115；世界至今玩家唯一的移動方式是走路，遠征首領
/// （90~160 格外）、地底遺跡神殿（得深挖才到）都讓「遠行」更吃重，卻沒有一件能讓移動變快
/// 的代步工具。工作台合成後可在移動時切換騎乘，水平移動速度提升（見前端
/// `RIDING_SPEED_MULT`）。純物品、不可放置。114（遺跡核心）是目前最大已用 id，115 是首個空號。
pub const STEAM_UNICYCLE_ID: u8 = 115;

/// 是否允許把 `riding` 設為 `true`（純判定，voxel_ws 的 SetRiding handler 用）。
/// 唯一條件：真實背包持有至少 1 輛蒸汽獨輪車——伺服器必須自己查背包算出 `has_item`，
/// 不能信任客戶端自報「我有車」（比照 `voxel_worldboss` 的 BossHit 持劍驗證手法）。
/// 下車（riding=false）不經過這條判定，呼叫端直接放行。純函式、確定性、可測。
pub fn can_start_riding(has_item: bool) -> bool {
    has_item
}

// ── 樂器物品 ID（純物品，不可放置於世界，街頭手風琴 v1，自主提案切片）───────────────
/// 街頭手風琴（ROADMAP 977）——物品 ID 116；世界至今與音樂沾邊的唯一一筆是居民心情好時
/// 自己哼的歌（`voxel_humming`）——玩家完全插不上手；集會鐘能把居民召到你身邊，但你到了
/// 之後什麼都做不了。世界從沒有一件玩家能主動操作、對著居民表演的樂器。工作台合成後可
/// 切換演奏／收起，附近閒著的居民會停下腳步聆聽。純物品、不可放置。115（蒸汽獨輪車）
/// 是目前最大已用 id，116 是首個空號。
pub const STREET_ACCORDION_ID: u8 = 116;

// ── 生計信物物品 ID（純物品，不可放置於世界，居民教你一道獨門配方 v2，自主提案切片）───
// v1（849）先驗證機制、只放一道護身符（任何居民都可能教）。本刀補上 v1 自己文件開頭
// 誠實留白的缺口：居民↔玩家「教」這件事至今與居民的生計身分（`voxel_vocation`）完全無關，
// 農夫、鐵匠、漁夫、獵人、商人教出來的都是同一件護身符。五種生計各自的信物，讓「這位
// 居民的本事」第一次帶著她的身分——見 `voxel_player_recipe::recipe_id_for_vocation`。
/// 豐收吊飾（ROADMAP 1003）——物品 ID 117；農夫的獨門配方，用親手種收的麥穗與胡蘿蔔編成。
pub const FARMER_CHARM_ID: u8 = 117;
/// 鍛環（ROADMAP 1003）——物品 ID 118；鐵匠的獨門配方，自己鍛的鐵錠嵌著一塊爐邊的石。
pub const SMITH_RING_ID: u8 = 118;
/// 潮貝墜（ROADMAP 1003）——物品 ID 119；漁夫的獨門配方，沙灘拾來的貝形墜子配一尾親手釣的魚。
pub const FISHER_CHARM_ID: u8 = 119;
/// 獵具束（ROADMAP 1003）——物品 ID 120；獵人的獨門配方，編織雜草纏上一截木柄的隨身小物。
pub const HUNTER_CHARM_ID: u8 = 120;
/// 秤砣墜（ROADMAP 1003）——物品 ID 121；商人的獨門配方，秤貨用的鐵砣鑄進一捧沙模，商旅隨身帶著。
pub const MERCHANT_CHARM_ID: u8 = 121;

/// 是否允許把 `performing` 設為 `true`（純判定，voxel_ws 的 SetPerforming handler 用）。
/// 唯一條件：真實背包持有至少 1 把街頭手風琴——伺服器必須自己查背包算出 `has_item`，
/// 不能信任客戶端自報「我有手風琴」（比照 `can_start_riding` 同款持有驗證手法）。
/// 收起演奏（performing=false）不經過這條判定，呼叫端直接放行。純函式、確定性、可測。
pub fn can_start_performing(has_item: bool) -> bool {
    has_item
}

/// 一把武器每次揮擊對暗影造成的「傷害點」（暗影耐久＝[`crate::voxel_shadow::HITS_TO_DISSIPATE`]=3）。
/// 徒手不是武器→回 0（呼叫端據此退回既有徒手 1 點）。木劍 2、石劍 3（一擊即散）、鐵劍 3。
/// 純函式、確定性、無副作用、可測。i18n 無涉（純數值）。
pub fn sword_power(item_id: u8) -> u8 {
    match item_id {
        SWORD_WOOD_ID => 2,
        SWORD_STONE_ID => 3,
        SWORD_IRON_ID => 3,
        _ => 0,
    }
}

/// 該物品是否為一把武器（劍）。純函式、可測。
pub fn is_sword(item_id: u8) -> bool {
    sword_power(item_id) > 0
}

/// 用這把武器驅散一隻暗影時，「額外」多凝的乙太礦數（基礎恆掉 1 枚，見 `voxel_shadow`）。
/// 只有鐵劍給 +1（雙倍溫柔獎勵），其餘武器與徒手皆 0。純函式、可測。
pub fn sword_extra_shards(item_id: u8) -> u32 {
    if item_id == SWORD_IRON_ID { 1 } else { 0 }
}

use crate::voxel_inventory::InvStore;

/// 一條合成配方（靜態常數，不含任何鎖或 IO 呼叫）。
pub struct Recipe {
    /// 配方識別碼（英文、穩定，WS 訊息用）。
    pub id: &'static str,
    /// 面向玩家的繁中名稱（i18n 集中管理用）。
    pub name_zh: &'static str,
    /// 配料列表：`(block_id, count)`。block_id 對齊後端 `Block as u8`。
    pub inputs: &'static [(u8, u32)],
    /// 產出方塊 id（合成台 v1 新方塊：Plank=8 / StoneBrick=9 / Glass=10）。
    pub output_block: u8,
    /// 一次合成的產出數量。
    pub output_count: u32,
}

/// 背包 2×2 合成配方（稀少、有感、易驗證）。
///
/// 材料 id 常數（對齊 `Block` enum）：
///   Wood=5, Stone=3, Sand=4 → Plank=8, StoneBrick=9, Glass=10
///   Dirt=2 → FarmSoil=11（種田 v1）
///   Plank=8 → Workbench=15（工作台 v1，ROADMAP 665）
///   Wheat=18 → Bread=19（麵包 v1，ROADMAP 668；18/19 為純物品 id，非方塊 enum）
pub const RECIPES: &[Recipe] = &[
    Recipe {
        id: "plank",
        name_zh: "木板",
        inputs: &[(5, 2)],   // 2 木 → 4 木板
        output_block: 8,
        output_count: 4,
    },
    Recipe {
        id: "stone_brick",
        name_zh: "石磚",
        inputs: &[(3, 2)],   // 2 石 → 2 石磚
        output_block: 9,
        output_count: 2,
    },
    Recipe {
        id: "glass",
        name_zh: "玻璃",
        inputs: &[(4, 2)],   // 2 沙 → 1 玻璃
        output_block: 10,
        output_count: 1,
    },
    Recipe {
        id: "till",
        name_zh: "農田土",
        inputs: &[(2, 2)],   // 2 泥土 → 2 農田土（種田 v1）
        output_block: 11,
        output_count: 2,
    },
    Recipe {
        id: "workbench",
        name_zh: "工作台",
        inputs: &[(8, 4)],   // 4 木板 → 1 工作台（2×2 剛好放滿四格）
        output_block: 15,
        output_count: 1,
    },
    Recipe {
        id: "bread",
        name_zh: "麵包",
        inputs: &[(18, 3)],  // 3 小麥顆粒 → 1 麵包（療癒農業循環終點；ROADMAP 668）
        output_block: 19,
        output_count: 1,
    },
    Recipe {
        id: "torch",
        name_zh: "火把",
        inputs: &[(5, 1), (20, 1)],  // 1 木頭 + 1 煤礦 → 4 火把（ROADMAP 685）
        output_block: 31,
        output_count: 4,
    },
    // ── 水桶 v1（自主提案切片）：3 鐵錠 → 1 水桶（背包 2×2；舀水引水灌溉乾田）──────────
    // {22:3} 為獨特多重集（鐵磚 {22:6}、鐵鎬/鐵斧 {22:3,8:2}、鐵鏟 {22:2,8:3} 皆不相撞）；
    // 鐵錠本身需熔爐冶煉，故不必再過工作台即可打一只水桶。空水桶是純物品，對準水源舀水、
    // 對準空格倒出永久來源水（接既有水流模擬與鄰水加速種田）。
    Recipe {
        id: "bucket",
        name_zh: "水桶",
        inputs: &[(22, 3)], // 3 鐵錠 → 1 水桶
        output_block: crate::voxel_bucket::BUCKET_ID, // 71（純物品，不可放置）
        output_count: 1,
    },
    // ── 鋤頭 v1（自主提案切片）：2 木頭 + 1 木板 → 1 木鋤頭（背包 2×2；就地把草/土開墾成農田土）──
    // {5:2,8:1} 為獨特多重集（木鎬/木斧 {5:3,8:1}、木鏟 {5:1,8:1} 皆不相撞）；便宜好上手＝
    // 「就地開墾」這件事一開始就伸手可得。鋤頭是純物品工具、反覆使用不耗損（比照鎬/斧）。
    Recipe {
        id: "hoe",
        name_zh: "木鋤頭",
        inputs: &[(5, 2), (8, 1)], // 2 木頭 + 1 木板 → 1 木鋤頭
        output_block: crate::voxel_hoe::HOE_ID, // 73（純物品，不可放置）
        output_count: 1,
    },
    // ── 梯子 v1（ROADMAP 688）：3 木板 → 3 梯子（垂直攀爬，深礦上下自如）──────────
    Recipe {
        id: "ladder",
        name_zh: "梯子",
        inputs: &[(8, 3)],              // 3 木板 → 3 梯子（剛好用掉三格 2×2）
        output_block: 35,               // Block::Ladder = 35
        output_count: 3,
    },
    // ── 鎬具 v1（ROADMAP 687）：採石/採礦更快、療癒循環加深 ─────────────────────
    Recipe {
        id: "wood_pickaxe",
        name_zh: "木鎬",
        inputs: &[(5, 3), (8, 1)],   // 3 木頭 + 1 木板 → 1 木鎬（剛好放滿 2×2 四格）
        output_block: PICKAXE_WOOD_ID,
        output_count: 1,
    },
    Recipe {
        id: "stone_pickaxe",
        name_zh: "石鎬",
        inputs: &[(3, 3), (8, 1)],   // 3 石頭 + 1 木板 → 1 石鎬（比木鎬耐用、速度更快）
        output_block: PICKAXE_STONE_ID,
        output_count: 1,
    },
    // ── 斧頭 v1（ROADMAP 689）：砍木加速，和鎬具互補的工具線 ─────────────────────
    Recipe {
        id: "wood_axe",
        name_zh: "木斧",
        inputs: &[(5, 3), (8, 1)],   // 3 木頭 + 1 木板 → 1 木斧（剛好填滿 2×2 四格）
        output_block: AXE_WOOD_ID,
        output_count: 1,
    },
    Recipe {
        id: "stone_axe",
        name_zh: "石斧",
        inputs: &[(3, 3), (8, 1)],   // 3 石頭 + 1 木板 → 1 石斧（4× 砍木速度）
        output_block: AXE_STONE_ID,
        output_count: 1,
    },
    // ── 鏟子 v1（ROADMAP 690）：挖草/土/沙/農田加速，完成採集三件套 ─────────────
    Recipe {
        id: "wood_shovel",
        name_zh: "木鏟",
        inputs: &[(5, 1), (8, 1)],   // 1 木頭 + 1 木板 → 1 木鏟（輕巧配方，2 格填 2×2）
        output_block: SHOVEL_WOOD_ID,
        output_count: 1,
    },
    Recipe {
        id: "stone_shovel",
        name_zh: "石鏟",
        inputs: &[(3, 1), (8, 1)],   // 1 石頭 + 1 木板 → 1 石鏟（4× 挖軟方塊速度）
        output_block: SHOVEL_STONE_ID,
        output_count: 1,
    },
    // ── 木門 v1（ROADMAP 693）：4 木板 → 2 門（封閉房間的第一步）──────────────────
    Recipe {
        id: "door",
        name_zh: "木門",
        inputs: &[(8, 4)],          // 4 木板 → 2 門（填滿 2×2 格）
        output_block: 43,           // Block::DoorClosed = 43
        output_count: 2,
    },
    // ── 床 v1：3 木板 + 3 葉片（無棉花，葉片當被褥）→ 1 床 ─────────────────────────
    Recipe {
        id: "bed",
        name_zh: "床",
        inputs: &[(8, 3), (6, 3)],  // 3 木板 + 3 葉片 → 1 床
        output_block: 45,           // Block::Bed = 45
        output_count: 1,
    },
    // ── 釣竿 v1（垂釣 v1，ROADMAP 734）：3 木板 → 1 釣竿（輕巧配方，背包 2×2 即可）───────
    // 沒有繩線材料，就用木板削竿代替；便宜好上手＝垂釣這條療癒支線一開始就伸手可得。
    Recipe {
        id: "fishing_rod",
        name_zh: "釣竿",
        inputs: &[(8, 3)],            // 3 木板 → 1 釣竿
        output_block: crate::voxel_fishing::FISHING_ROD_ID, // 60（純物品，不可放置）
        output_count: 1,
    },
    // ── 冰晶燈 v1：1 冰晶 + 2 玻璃 → 1 冰晶燈 ─────────────────────────────────────
    // 把雪原專程採回的稀有冰晶(56)封進玻璃(10)，做成一盞泛著冷藍幽光的裝飾燈(57)。
    // 冰晶是特殊材料（雪原獨有、稀疏難尋）、玻璃便宜（2 沙一片）＝珍寶當燈芯、玻璃當燈罩。
    Recipe {
        id: "ice_lantern",
        name_zh: "冰晶燈",
        inputs: &[(56, 1), (10, 2)],  // 1 冰晶 + 2 玻璃 → 1 冰晶燈（3 格，剛好塞進 2×2）
        output_block: 57,             // Block::IceLantern = 57
        output_count: 1,
    },
    // ── 告示牌 v1（ROADMAP 740）：2 木板 → 1 告示牌（輕巧配方，背包 2×2 即可）──────────────
    // 便宜好上手＝立牌標記基地這件事一開始就伸手可得；放置後右鍵寫字。
    // 用 2 木板的唯一多重集（避開 4 木板＝工作台/木門、3 木板＝梯子等既有配方的遮蔽）。
    Recipe {
        id: "sign",
        name_zh: "告示牌",
        inputs: &[(8, 2)],            // 2 木板 → 1 告示牌
        output_block: 66,             // Block::Sign = 66
        output_count: 1,
    },
    // ── 莓果叢苗 v1（自主提案切片 806）：樹苗(65) + 種子(14)×2 → 1 莓果叢苗(75)────────────
    // 把砍葉得來的樹苗與種子育成一叢會結果的灌木。多重集 {65:1,14:2} 獨一無二
    //（既有 2×2 配方無一用到樹苗 65 或種子 14），不與任何配方相撞。種下後多年生、可反覆採收。
    Recipe {
        id: "berry_bush",
        name_zh: "莓果叢苗",
        inputs: &[(65, 1), (14, 2)],  // 1 樹苗 + 2 種子 → 1 莓果叢苗（3 格，剛好塞進 2×2）
        output_block: crate::voxel_berry::BUSH_UNRIPE_ID, // 75（可放置的未結果灌木）
        output_count: 1,
    },
    // ── 木長椅 v1（自主提案切片）：木頭(5)×2 + 木板(8)×2 → 1 木長椅(79)────────────────
    // 木頭當椅腳、木板當椅面，做一張能坐的長椅。多重集 {5:2,8:2} 獨一無二（既有 2×2 配方
    // 無一是「2 木頭 + 2 木板」：木鋤 {5:2,8:1}、木板 {5:2}、工作台 {8:4} 皆不同），不撞任何配方。
    // 剛好填滿 2×2 四格。擺在世界裡→白天路過的居民會停下坐上去歇腳。
    Recipe {
        id: "bench",
        name_zh: "木長椅",
        inputs: &[(5, 2), (8, 2)],  // 2 木頭 + 2 木板 → 1 木長椅（剛好放滿 2×2 四格）
        output_block: crate::voxel_bench::BENCH_ID, // 79（可放置的家具方塊）
        output_count: 1,
    },
    // ── 空玻璃瓶 v1（自主提案切片 825）：2 玻璃 → 1 空玻璃瓶（丟進水裡寫瓶中信）───────────
    // {10:2} 為獨特多重集：玻璃本身的配方輸入是 {4:2}=2 沙（輸出才是玻璃 10），既有配方
    // 沒有任何一條「吃 2 個玻璃」，不與任何配方相撞。
    Recipe {
        id: "bottle",
        name_zh: "空玻璃瓶",
        inputs: &[(10, 2)], // 2 玻璃 → 1 空玻璃瓶
        output_block: crate::voxel_bottle::BOTTLE_ID, // 83（純物品，不可放置）
        output_count: 1,
    },
    // ── 染色建材 v1（自主提案切片）：用天然礦物給沙子染色，燒出彩色建材 ────────────────
    // 建造近 200 刀以來，玩家能放的純建材幾乎全是灰棕色系（木板/石磚/玻璃/拋光石/鐵磚），
    // 本刀補上世界第一批**彩色**建材：礦物本身的天然色澤正是現實赤陶/黑陶染料的來源。
    // 四組多重集 {4:2,21:1}/{4:2,20:1}/{4:2,55:1}/{4:2,58:1} 彼此互異，也與既有配方
    // （玻璃 {4:2} 單一材料、乙太煙火 {58:1,20:2,4:2} 三料）皆不相撞。每色皆 3 格，比照
    // 冰晶燈（3 格用到雪原限定材料）precedent 留在背包 2×2，不因材料稀有就硬塞進工作台。
    Recipe {
        id: "terracotta_red",
        name_zh: "紅陶磚",
        inputs: &[(4, 2), (21, 1)], // 2 沙 + 1 鐵礦 → 2 紅陶磚（鐵鏽紅顏料）
        output_block: 89,           // Block::TerracottaRed = 89
        output_count: 2,
    },
    Recipe {
        id: "terracotta_black",
        name_zh: "黑陶磚",
        inputs: &[(4, 2), (20, 1)], // 2 沙 + 1 煤礦 → 2 黑陶磚（煤炭黑顏料）
        output_block: 90,           // Block::TerracottaBlack = 90
        output_count: 2,
    },
    Recipe {
        id: "terracotta_white",
        name_zh: "白陶磚",
        inputs: &[(4, 2), (55, 1)], // 2 沙 + 1 雪 → 2 白陶磚（雪原限定純白顏料）
        output_block: 91,           // Block::TerracottaWhite = 91
        output_count: 2,
    },
    Recipe {
        id: "terracotta_blue",
        name_zh: "青陶磚",
        inputs: &[(4, 2), (58, 1)], // 2 沙 + 1 乙太礦 → 2 青陶磚（世界最深最稀有的顏料）
        output_block: 92,           // Block::TerracottaBlue = 92
        output_count: 2,
    },
    // ── 乙太幣鑄造 v1（ROADMAP 873，自主提案切片）：把常見原礦鑄成通用貨幣 ───────────
    // 三種最容易大量湊齊的原礦各開一條「賣出換幣」配方，統一 4 份原礦→1 枚乙太幣的
    // 匯率（v1 刻意不分稀有度分級，先讓「賺錢」這件事本身成立，之後再視經濟數據調整）。
    // {5:4}/{3:4}/{4:4} 皆為單一材料、不同數量的多重集，與既有配方（木板 {5:2}、
    // 石磚 {3:2}、玻璃 {4:2}、鎬斧鏟等混合多重集）逐一核對皆不相撞，見下方測試。
    Recipe {
        id: "coin_from_wood",
        name_zh: "乙太幣（木頭）",
        inputs: &[(5, 4)], // 4 木頭 → 1 乙太幣
        output_block: COIN_ID,
        output_count: 1,
    },
    Recipe {
        id: "coin_from_stone",
        name_zh: "乙太幣（石頭）",
        inputs: &[(3, 4)], // 4 石頭 → 1 乙太幣
        output_block: COIN_ID,
        output_count: 1,
    },
    Recipe {
        id: "coin_from_sand",
        name_zh: "乙太幣（沙）",
        inputs: &[(4, 4)], // 4 沙 → 1 乙太幣
        output_block: COIN_ID,
        output_count: 1,
    },
    // ── 驅影之劍 v1（ROADMAP 887，自主提案切片）：世界第一批武器，握在手上驅散夜之暗影 ──────
    // 木劍多重集 {5:1,8:2}（1 木頭＋2 木板）獨一無二：木鏟 {5:1,8:1}、木長椅 {5:2,8:2}、
    // 告示牌 {8:2}、木劍 {5:1,8:2} 皆相異，不撞任何既有 2×2 配方。木板當劍身、木頭當劍柄。
    Recipe {
        id: "wood_sword",
        name_zh: "木劍",
        inputs: &[(5, 1), (8, 2)], // 1 木頭 + 2 木板 → 1 木劍
        output_block: SWORD_WOOD_ID,
        output_count: 1,
    },
    // 石劍多重集 {3:2,8:1}（2 石頭＋1 木板）獨一無二：石磚 {3:2}、石鏟 {3:1,8:1}、
    // 石鎬/石斧 {3:3,8:1} 皆相異。石片當劍身、木板當劍柄，比木劍更利。
    Recipe {
        id: "stone_sword",
        name_zh: "石劍",
        inputs: &[(3, 2), (8, 1)], // 2 石頭 + 1 木板 → 1 石劍
        output_block: SWORD_STONE_ID,
        output_count: 1,
    },
    // ── 玩家裝飾傢俱 v1（ROADMAP 931，自主提案切片）：世界第一批純裝飾傢俱 ──────────────────
    // 世界可放方塊至今不是建材就是有用途的功能家具，唯獨缺純為好看擺著佈置家園的小傢俱。
    // 四樣皆走背包 2×2、便宜好上手（葉片/木板/石磚/陶磚等常見料），讓「佈置一個屬於自己的
    // 角落」一開始就伸手可得。四組多重集彼此互異、也與既有配方皆不相撞（見下方測試逐一核對）。
    // 小地毯 {6:3}（3 葉片織一片毯）：床 {8:3,6:3}、雞舍 {5:4,6:2}、掛旗 {5:1,6:2} 皆相異；
    // 沒有任何一條既有 2×2 配方是「只吃 3 葉片」。
    Recipe {
        id: "carpet",
        name_zh: "小地毯",
        inputs: &[(6, 3)], // 3 葉片 → 2 小地毯（一片織毯鋪地）
        output_block: crate::voxel_furniture::CARPET_ID, // 102
        output_count: 2,
    },
    // 花盆 {89:2,6:1}（2 紅陶磚捏一只陶盆 + 1 葉片栽株綠意）：沒有任何既有 2×2 配方消耗紅陶磚(89)。
    Recipe {
        id: "flowerpot",
        name_zh: "花盆",
        inputs: &[(89, 2), (6, 1)], // 2 紅陶磚 + 1 葉片 → 1 花盆
        output_block: crate::voxel_furniture::FLOWERPOT_ID, // 103
        output_count: 1,
    },
    // 小圓桌 {8:2,9:1}（2 木板作桌面 + 1 石磚作底座）：告示牌 {8:2}、木長椅 {5:2,8:2} 皆相異；
    // 沒有任何既有 2×2 配方是「2 木板 + 1 石磚」。
    Recipe {
        id: "table",
        name_zh: "小圓桌",
        inputs: &[(8, 2), (9, 1)], // 2 木板 + 1 石磚 → 1 小圓桌
        output_block: crate::voxel_furniture::TABLE_ID, // 104
        output_count: 1,
    },
    // 掛旗 {5:1,6:2}（1 木頭作旗桿 + 2 葉片織旗面）：木劍 {5:1,8:2}、木鏟 {5:1,8:1} 皆相異；
    // 沒有任何既有 2×2 配方是「1 木頭 + 2 葉片」。
    Recipe {
        id: "banner",
        name_zh: "掛旗",
        inputs: &[(5, 1), (6, 2)], // 1 木頭 + 2 葉片 → 1 掛旗
        output_block: crate::voxel_furniture::BANNER_ID, // 105
        output_count: 1,
    },
];

/// 工作台 3×3 合成配方（需放置工作台方塊後右鍵開啟面板才能合成）。
///
/// 這些配方需要 5-6 格材料，超出 2×2 上限，或組合多種材料，
/// 象徵「精工合成」需要工作台才能完成。
pub const WORKBENCH_RECIPES: &[Recipe] = &[
    Recipe {
        id: "plank_wb",
        name_zh: "木板（大量）",
        inputs: &[(5, 6)],      // 6 木 → 16 木板（高效批量，需 3×3）
        output_block: 8,
        output_count: 16,
    },
    Recipe {
        id: "stone_brick_wb",
        name_zh: "石磚（大量）",
        inputs: &[(3, 6)],      // 6 石 → 10 石磚（大批建材）
        output_block: 9,
        output_count: 10,
    },
    Recipe {
        id: "glass_wb",
        name_zh: "玻璃（大量）",
        inputs: &[(4, 6)],      // 6 沙 → 8 玻璃（大批玻璃）
        output_block: 10,
        output_count: 8,
    },
    Recipe {
        id: "stone_wood_mix",
        name_zh: "混合石磚",
        inputs: &[(3, 3), (8, 3)],  // 3 石 + 3 木板 → 6 石磚（混合工藝，獨特配方）
        output_block: 9,
        output_count: 6,
    },
    Recipe {
        id: "farm_kit",
        name_zh: "農耕大包",
        inputs: &[(2, 4), (5, 2)],  // 4 泥土 + 2 木 → 8 農田土（農場快速開墾）
        output_block: 11,
        output_count: 8,
    },
    Recipe {
        id: "furnace_wb",
        name_zh: "熔爐",
        inputs: &[(3, 8)],          // 8 石頭 → 1 熔爐（需工作台大格）
        output_block: 16,
        output_count: 1,
    },
    Recipe {
        id: "iron_block",
        name_zh: "鐵磚",
        inputs: &[(22, 6)],         // 6 鐵錠 → 2 鐵磚（ROADMAP 684）壓縮精煉金屬建材
        output_block: 23,
        output_count: 2,
    },
    Recipe {
        id: "iron_pickaxe",
        name_zh: "鐵鎬",
        inputs: &[(22, 3), (8, 2)], // 3 鐵錠 + 2 木板 → 1 鐵鎬（ROADMAP 687；5 格需工作台）
        output_block: PICKAXE_IRON_ID,
        output_count: 1,
    },
    // ── 鐵斧（ROADMAP 689）：5 格需工作台 3×3 ──────────────────────────────────────
    Recipe {
        id: "iron_axe",
        name_zh: "鐵斧",
        inputs: &[(22, 3), (8, 2)], // 3 鐵錠 + 2 木板 → 1 鐵斧（6× 砍木速度）
        output_block: AXE_IRON_ID,
        output_count: 1,
    },
    // ── 鐵鏟（ROADMAP 690）：2 鐵錠 + 3 木板（5 格，需工作台）──────────────────────
    Recipe {
        id: "iron_shovel",
        name_zh: "鐵鏟",
        inputs: &[(22, 2), (8, 3)], // 2 鐵錠 + 3 木板 → 1 鐵鏟（6× 挖軟方塊速度）
        output_block: SHOVEL_IRON_ID,
        output_count: 1,
    },
    // ── 箱子 v1（ROADMAP 692）：8 木板 → 1 箱子（工作台合成，需 8 格）────────────────
    Recipe {
        id: "chest",
        name_zh: "箱子",
        inputs: &[(8, 8)],   // 8 木板 → 1 箱子（工作台合成，放置後可儲存物品）
        output_block: 42,    // Block::Chest = 42
        output_count: 1,
    },
    // ── 乙太燈 v1（乙太礦脈 v1）：1 乙太礦 + 4 玻璃 → 1 乙太燈 ───────────────────────
    // 把世界最深處挖回的珍稀乙太礦(58)封進四片玻璃(10)燈罩，做成一盞散發清冷青藍光的明燈(59)。
    // 5 格材料超出 2×2、且用到最難尋的礦——放進「精工合成」的工作台層，配得上它的稀有。
    Recipe {
        id: "aether_lamp",
        name_zh: "乙太燈",
        inputs: &[(58, 1), (10, 4)],  // 1 乙太礦 + 4 玻璃 → 1 乙太燈
        output_block: 59,             // Block::AetherLamp = 59
        output_count: 1,
    },
    // ── 野菜暖湯 v1（ROADMAP 778）：乙太方界第一道「多食材料理」───────────────────────
    // 2 胡蘿蔔(49) + 2 馬鈴薯(53) + 1 小麥(18) → 1 野菜暖湯(STEW_ID=67)。三種親手種的作物、
    // 滿滿一大鍋（5 格材料）拌在一起煮——要湊齊三種不同作物（不像烤魚/烤地薯只需單一食材），
    // 且份量超出背包 2×2 塞不下，是最費心的料理，配得上「複雜合成走工作台」。接起
    // 「種田→料理→餽贈→享用」完整循環的頂點。
    Recipe {
        id: "veggie_stew",
        name_zh: "野菜暖湯",
        inputs: &[(49, 2), (53, 2), (18, 1)],  // 2 胡蘿蔔 + 2 馬鈴薯 + 1 小麥（一大鍋）
        output_block: STEW_ID,
        output_count: 1,
    },
    // ── 乙太煙火 v1（ROADMAP 785）：玩家朝夜空施放的慶祝道具 ────────────────────────
    // 1 乙太礦(58) + 2 煤礦(20) + 2 沙(4) → 3 乙太煙火(FIREWORK_ID=68)。煤與沙拌成火藥、
    // 封進最深處挖回的乙太礦引信裡，做成能升空綻放的煙火。5 格材料超出背包 2×2 需工作台，
    // {58:1,20:2,4:2} 是唯一多重集（乙太燈用 {58,10}、火把用 {5,20}，皆不相撞）；純物品不可放置，
    // 施放即消耗（voxel_ws 的 FireworkLaunch 處理）。
    Recipe {
        id: "aether_firework",
        name_zh: "乙太煙火",
        inputs: &[(58, 1), (20, 2), (4, 2)],  // 1 乙太礦 + 2 煤礦 + 2 沙 → 3 乙太煙火
        output_block: crate::voxel_firework::FIREWORK_ID,
        output_count: 3,
    },
    // ── 乙太沃肥 v1（ROADMAP 789）：把用不完的雜草＋泥土漚成催熟幼苗的沃肥 ──────────
    // 3 雜草(1) + 2 泥土(2) → 2 乙太沃肥(FERTILIZER_ID=69)。割草採集、挖土時囤下的
    // 平價廢料漚成肥；5 格材料超出背包 2×2 需工作台，{1:3,2:2} 為唯一多重集（農田土 till 用
    // 背包 {2:2}、不在工作台表，不相撞）；純物品不可放置，施肥即消耗（voxel_ws 的 Fertilize 處理）。
    Recipe {
        id: "aether_fertilizer",
        name_zh: "乙太沃肥",
        inputs: &[(1, 3), (2, 2)],  // 3 雜草 + 2 泥土 → 2 乙太沃肥
        output_block: crate::voxel_compost::FERTILIZER_ID,
        output_count: 2,
    },
    // ── 乙太營火 v1（自主提案切片）：玩家親手蓋的一處發光火堆 ──────────────────────
    // 3 石頭(3) + 2 木頭(5) + 1 煤礦(20) → 1 營火(CAMPFIRE_ID=70)。石頭圍一圈爐灶、
    // 木頭當柴、煤礦引火＝一眼就是座火堆。6 格材料超出背包 2×2 需工作台，{3:3,5:2,20:1}
    // 為唯一多重集（不與既有工作台配方相撞）；可放置的發光方塊，放下即照亮營地、夜裡
    // 吸引路過居民駐足圍暖（voxel_ws 的 Place/tick_residents 處理）。
    Recipe {
        id: "campfire",
        name_zh: "營火",
        inputs: &[(3, 3), (5, 2), (20, 1)],  // 3 石頭 + 2 木頭 + 1 煤礦 → 1 營火
        output_block: crate::voxel_campfire::CAMPFIRE_ID,
        output_count: 1,
    },
    // ── 集會鐘 v1（自主提案切片）：玩家像村長一樣召集村民的一座鐘 ──────────────────────
    // 4 鐵錠(22) + 1 木頭(5) → 1 集會鐘(BELL_ID=74)。鐵鑄的鐘身掛在木樑上——鐘身要金屬才響亮，
    // 用到需採鐵→冶煉的鐵錠＝這是村莊中後期的「聚會核心」，成本配得上它的份量。5 格材料超出
    // 背包 2×2 需工作台，{22:4,5:1} 為唯一多重集（鐵磚 {22:6}、鐵鎬/斧 {22:3,8:2}、鐵鏟 {22:2,8:3}
    // 皆不相撞）；可放置的方塊，放下後右鍵敲響即召集附近居民（voxel_ws 的 Place/RingBell 處理）。
    Recipe {
        id: "bell",
        name_zh: "集會鐘",
        inputs: &[(22, 4), (5, 1)],  // 4 鐵錠 + 1 木頭 → 1 集會鐘
        output_block: crate::voxel_bell::BELL_ID,
        output_count: 1,
    },
    // ── 雞舍 v1（自主提案切片）：世界第一種「動物產物」資源節點 ──────────────────────
    // 木頭(5)×4 + 葉片(6)×2 → 1 雞舍(COOP_ID=80)。木架撐頂、葉片鋪成溫暖的窩。6 格材料超出
    // 背包 2×2 需工作台，{5:4,6:2} 為唯一多重集（床 {8:3,6:3} 用木板非木頭、木鎬 {5:3,8:1} 無葉片，
    // 皆不相撞）；可放置的方塊，放下後靜候一段時間會生蛋，收下就地回退繼續孵（與莓果叢對成
    // 植物／動物兩條可反覆採收的資源軸）。
    Recipe {
        id: "coop",
        name_zh: "雞舍",
        inputs: &[(5, 4), (6, 2)],  // 4 木頭 + 2 葉片 → 1 雞舍
        output_block: crate::voxel_coop::COOP_ID,
        output_count: 1,
    },
    // ── 建築藍圖 v1（自主提案切片）：直接指定居民蓋哪一種建物，不再只能猜關鍵詞 ──────────
    // 五張藍圖各對應一種既有建物（House/Well/Tower/Garden/Pavilion），送給居民後直接改寫
    // 她的心願（見 voxel_blueprint.rs + voxel_ws.rs Gift 接線）。材料呼應各建物的既有建材
    // 色盤（小屋=木+石、水井=石磚+玻璃、瞭望台=大量石磚、花圃=葉片+種子、涼亭=木頭+火把），
    // 五組多重集彼此互異、也與既有配方皆不相撞（見下方 all_recipes_output_crafted_block_ids）。
    Recipe {
        id: "blueprint_house",
        name_zh: "小屋藍圖",
        inputs: &[(8, 4), (3, 2)],  // 4 木板 + 2 石頭 → 1 小屋藍圖
        output_block: crate::voxel_blueprint::BLUEPRINT_HOUSE,
        output_count: 1,
    },
    Recipe {
        id: "blueprint_well",
        name_zh: "水井藍圖",
        inputs: &[(9, 3), (10, 2)],  // 3 石磚 + 2 玻璃 → 1 水井藍圖（5 格，需工作台）
        output_block: crate::voxel_blueprint::BLUEPRINT_WELL,
        output_count: 1,
    },
    Recipe {
        id: "blueprint_tower",
        name_zh: "瞭望台藍圖",
        inputs: &[(9, 5)],  // 5 石磚 → 1 瞭望台藍圖
        output_block: crate::voxel_blueprint::BLUEPRINT_TOWER,
        output_count: 1,
    },
    Recipe {
        id: "blueprint_garden",
        name_zh: "花圃藍圖",
        inputs: &[(6, 3), (14, 2)],  // 3 葉片 + 2 種子 → 1 花圃藍圖
        output_block: crate::voxel_blueprint::BLUEPRINT_GARDEN,
        output_count: 1,
    },
    Recipe {
        id: "blueprint_pavilion",
        name_zh: "涼亭藍圖",
        inputs: &[(5, 3), (31, 2)],  // 3 木頭 + 2 火把 → 1 涼亭藍圖（5 格，需工作台）
        output_block: crate::voxel_blueprint::BLUEPRINT_PAVILION,
        output_count: 1,
    },
    // ── 驅影之劍 v1（ROADMAP 887，自主提案切片）：精煉武器的頂點，需工作台大格 ──────────────
    // 鐵劍多重集 {5:2,22:3}（3 鐵錠鑄劍身＋2 木頭作握柄）獨一無二：鐵磚 {22:6}、鐵鎬/鐵斧/鐵鏟
    // {8:2,22:3}（木板握柄）皆相異，不撞任何既有工作台配方；總 5 料 > 4 格，名正言順需工作台。
    // 一擊驅散暗影且雙倍乙太礦回報，見 sword_extra_shards。
    Recipe {
        id: "iron_sword",
        name_zh: "鐵劍",
        inputs: &[(5, 2), (22, 3)], // 3 鐵錠 + 2 木頭 → 1 鐵劍
        output_block: SWORD_IRON_ID,
        output_count: 1,
    },
    // ── 蒸汽獨輪車 v1（ROADMAP 976，自主提案切片）：世界第一件代步工具，需工作台大格 ──────
    // 4 鐵錠(22) + 2 木頭(5) + 1 乙太礦(58) → 1 蒸汽獨輪車。鐵鑄輪框、木製車架、乙太礦驅動
    // 蒸汽鍋爐——蒸汽龐克風味用既有材料拼出，不發明新素材。多重集 {22:4,5:2,58:1} 獨一無二：
    // 鐵磚 {22:6}、鐵鎬/斧 {22:3,8:2}、鐵鏟 {22:2,8:3}、鐵劍 {5:2,22:3}、乙太燈 {58:1,10:4}、
    // 乙太煙火 {58:1,20:2,4:2} 皆不相撞。7 格材料遠超背包 2×2，是一件「載具」而非小道具，
    // 比照鎬/斧的合成規模，名正言順需工作台。
    Recipe {
        id: "steam_unicycle",
        name_zh: "蒸汽獨輪車",
        inputs: &[(22, 4), (5, 2), (58, 1)], // 4 鐵錠 + 2 木頭 + 1 乙太礦 → 1 蒸汽獨輪車
        output_block: STEAM_UNICYCLE_ID,
        output_count: 1,
    },
    // ── 街頭手風琴 v1（ROADMAP 977，自主提案切片）：世界第一件玩家可操作的樂器 ──────────
    // 3 木板(8) + 2 鐵錠(22) + 1 玻璃(10) → 1 街頭手風琴。木製琴身、黃銅簧片、玻璃視窗，
    // 蒸汽龐克風味用既有材料拼出，不發明新素材。多重集 {8:3,22:2,10:1} 獨一無二：
    // 鐵鎬/鐵斧 {22:3,8:2}、鐵鏟 {22:2,8:3}、鐵劍 {5:2,22:3} 皆不相撞。6 格材料超過背包
    // 2×2，比照獨輪車/鐵劍的合成規模，名正言順需工作台。
    Recipe {
        id: "street_accordion",
        name_zh: "街頭手風琴",
        inputs: &[(8, 3), (22, 2), (10, 1)], // 3 木板 + 2 鐵錠 + 1 玻璃 → 1 街頭手風琴
        output_block: STREET_ACCORDION_ID,
        output_count: 1,
    },
];

/// 熔爐冶煉配方（需放置熔爐方塊後右鍵開啟冶煉面板才能使用）。
///
/// 冶煉概念：把原始方塊「精煉」成獨特材料，或比工作台更高效地產出建材。
/// Block id：Stone=3, Sand=4, Glass=10, SmoothStone=17, CoalOre=20, IronOre=21, IronIngot=22。
pub const FURNACE_RECIPES: &[Recipe] = &[
    Recipe {
        id: "smelt_stone",
        name_zh: "拋光石",
        inputs: &[(3, 3)],          // 3 石頭 → 3 拋光石（唯一能得到拋光石的途徑）
        output_block: 17,
        output_count: 3,
    },
    Recipe {
        id: "smelt_glass",
        name_zh: "玻璃（冶煉）",
        inputs: &[(4, 2)],          // 2 沙 → 3 玻璃（比背包配方 2沙→1玻璃 更高效）
        output_block: 10,
        output_count: 3,
    },
    Recipe {
        id: "smelt_brick",
        name_zh: "石磚（冶煉）",
        inputs: &[(3, 2)],          // 2 石頭 → 4 石磚（比背包配方 2石→2磚 雙倍產量）
        output_block: 9,
        output_count: 4,
    },
    Recipe {
        id: "smelt_iron",
        name_zh: "鐵錠",
        inputs: &[(21, 1), (20, 1)], // 1 鐵礦 + 1 煤礦（煤礦當燃料）→ 2 鐵錠
        output_block: 22,
        output_count: 2,
    },
    Recipe {
        id: "smelt_fish",
        name_zh: "烤魚",
        // 1 生小魚（FISH_ID=61）→ 1 烤魚（COOKED_FISH_ID=63）。把垂釣的漁獲送進熔爐
        // 烤熟，變成居民最愛的美味贈禮，接起「垂釣→烹飪→餽贈」的療癒循環。
        inputs: &[(61, 1)],
        output_block: 63,
        output_count: 1,
    },
    Recipe {
        id: "smelt_potato",
        name_zh: "烤地薯",
        // 1 生馬鈴薯（POTATO_ID=53）→ 1 烤地薯（BAKED_POTATO_ID=64）。把種田的收成
        // 送進熔爐烤熟，變成居民最愛的美味贈禮，讓「種田→烹飪→餽贈」也連成一圈
        // （比照 smelt_fish 之於垂釣）。
        inputs: &[(53, 1)],
        output_block: 64,
        output_count: 1,
    },
    Recipe {
        id: "smelt_jam",
        name_zh: "莓果醬",
        // 3 莓果（BERRY_ID=77）→ 1 莓果醬（JAM_ID=78）。把多年生莓園採來的莓果放進熔爐
        // 小火慢熬，凝成一罐甜滋滋的果醬——乙太方界第一種「甜點」熟食，補上 806 莓果的
        // 下游用途，接起「莓園→熬煮→自己享用／餽贈」的療癒循環（比照 smelt_fish 之於垂釣）。
        inputs: &[(crate::voxel_berry::BERRY_ID, 3)],
        output_block: crate::voxel_berry::JAM_ID,
        output_count: 1,
    },
];

/// 居民教你的獨門配方池（`voxel_player_recipe`，居民教你一道獨門配方 v1/v2，自主提案切片）。
///
/// 與 `RECIPES`/`WORKBENCH_RECIPES`/`FURNACE_RECIPES` 三張表刻意分開、**不併入**
/// `find_any_recipe` 的搜尋鏈——這裡的配方要先被居民教過才能合成，`voxel_ws.rs` 的
/// Craft handler 對這張表要額外查 `PlayerRecipeStore::knows` 才放行，見 [`is_taught_recipe`]。
/// v1 先放一道（護身符，任何居民都可能教，機制驗證通過）；v2（ROADMAP 1003）補上生計
/// 專屬的五道——哪位居民教哪一道由 `voxel_player_recipe::recipe_id_for_vocation` 依她的
/// `voxel_vocation::Vocation` 決定，護身符固定歸給工匠（其餘五種生計各自一道）。
/// 六道 inputs 多重集彼此互異、也與 `RECIPES`/`WORKBENCH_RECIPES` 全表不撞（見
/// `taught_recipes_have_unique_input_multisets` 測試）。
pub const TAUGHT_RECIPES: &[Recipe] = &[
    Recipe {
        id: "amulet",
        name_zh: "護身符",
        inputs: &[(3, 1), (94, 1)], // 1 石頭(Stone=3) + 1 紅花(WildflowerRed=94)：工匠教
        output_block: AMULET_ID,
        output_count: 1,
    },
    Recipe {
        id: "farmer_charm",
        name_zh: "豐收吊飾",
        inputs: &[(18, 1), (49, 1)], // 1 小麥(Wheat=18) + 1 胡蘿蔔(Carrot=49)：農夫教
        output_block: FARMER_CHARM_ID,
        output_count: 1,
    },
    Recipe {
        id: "smith_ring",
        name_zh: "鍛環",
        inputs: &[(22, 1), (3, 1)], // 1 鐵錠(IronIngot=22) + 1 石頭(Stone=3)：鐵匠教
        output_block: SMITH_RING_ID,
        output_count: 1,
    },
    Recipe {
        id: "fisher_charm",
        name_zh: "潮貝墜",
        inputs: &[(4, 1), (61, 1)], // 1 沙(Sand=4) + 1 小魚(Fish=61)：漁夫教
        output_block: FISHER_CHARM_ID,
        output_count: 1,
    },
    Recipe {
        id: "hunter_charm",
        name_zh: "獵具束",
        inputs: &[(1, 1), (5, 1)], // 1 雜草(Grass=1) + 1 木頭(Wood=5)：獵人教
        output_block: HUNTER_CHARM_ID,
        output_count: 1,
    },
    Recipe {
        id: "merchant_charm",
        name_zh: "秤砣墜",
        inputs: &[(4, 1), (22, 1)], // 1 沙(Sand=4) + 1 鐵錠(IronIngot=22)：商人教
        output_block: MERCHANT_CHARM_ID,
        output_count: 1,
    },
];

/// 依 id 找獨門配方（找不到回 None）。**不**代表玩家已學會——呼叫端仍須另查
/// `PlayerRecipeStore::knows` 才能放行合成。
pub fn find_taught_recipe(id: &str) -> Option<&'static Recipe> {
    TAUGHT_RECIPES.iter().find(|r| r.id == id)
}

/// 此 id 是否屬於「需要居民教過才能合成」的獨門配方池。
pub fn is_taught_recipe(id: &str) -> bool {
    TAUGHT_RECIPES.iter().any(|r| r.id == id)
}

/// 依 id 找背包配方（2×2，找不到回 None）。
pub fn find_recipe(id: &str) -> Option<&'static Recipe> {
    RECIPES.iter().find(|r| r.id == id)
}

/// 依 id 找工作台配方（3×3，找不到回 None）。
pub fn find_workbench_recipe(id: &str) -> Option<&'static Recipe> {
    WORKBENCH_RECIPES.iter().find(|r| r.id == id)
}

/// 依 id 找熔爐冶煉配方（找不到回 None）。
pub fn find_furnace_recipe(id: &str) -> Option<&'static Recipe> {
    FURNACE_RECIPES.iter().find(|r| r.id == id)
}

/// 依 id 搜尋三套配方表，背包 → 工作台 → 熔爐（找不到回 None）。
/// WS Craft handler 用——前端送 recipe_id，後端統一查這裡。
pub fn find_any_recipe(id: &str) -> Option<&'static Recipe> {
    find_recipe(id)
        .or_else(|| find_workbench_recipe(id))
        .or_else(|| find_furnace_recipe(id))
}

/// 玩家是否有足夠材料合成指定配方（純讀、不改狀態、可在鎖外呼叫）。
pub fn can_craft(recipe: &Recipe, store: &InvStore, player: &str) -> bool {
    recipe.inputs.iter().all(|&(block_id, count)| store.count(player, block_id) >= count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_recipe_returns_correct_recipes() {
        let r = find_recipe("plank").unwrap();
        assert_eq!(r.id, "plank");
        assert_eq!(r.output_block, 8, "木板 id 應為 8（Plank）");
        assert_eq!(r.output_count, 4);
        assert!(find_recipe("stone_brick").is_some());
        assert!(find_recipe("glass").is_some());
    }

    #[test]
    fn find_recipe_workbench_in_2x2_list() {
        // 工作台配方在 2×2 表——4 木板合成
        let r = find_recipe("workbench").unwrap();
        assert_eq!(r.output_block, 15, "工作台 id 應為 15");
        assert_eq!(r.output_count, 1);
        assert_eq!(r.inputs, &[(8, 4)], "工作台需要 4 木板");
    }

    #[test]
    fn find_recipe_bed_in_2x2_list() {
        // 床配方在 2×2 表——3 木板 + 3 葉片 → 1 床
        let r = find_recipe("bed").unwrap();
        assert_eq!(r.output_block, 45, "床 id 應為 45（Block::Bed）");
        assert_eq!(r.output_count, 1);
        assert_eq!(r.inputs, &[(8, 3), (6, 3)], "床需要 3 木板 + 3 葉片");
    }

    #[test]
    fn find_recipe_ice_lantern_in_2x2_list() {
        // 冰晶燈配方在 2×2 表——1 冰晶(56) + 2 玻璃(10) → 1 冰晶燈(57)
        let r = find_recipe("ice_lantern").unwrap();
        assert_eq!(r.output_block, 57, "冰晶燈 id 應為 57（Block::IceLantern）");
        assert_eq!(r.output_count, 1);
        assert_eq!(r.inputs, &[(56, 1), (10, 2)], "冰晶燈需要 1 冰晶 + 2 玻璃");
        // 冰晶(56)是特殊燈芯、玻璃(10)是便宜燈罩——特殊材料只需一顆。
        let crystal = r.inputs.iter().find(|&&(id, _)| id == 56).unwrap();
        assert_eq!(crystal.1, 1, "稀有冰晶只需 1 顆（雪原珍寶，不該大量消耗）");
    }

    #[test]
    fn find_recipe_aether_lamp_in_workbench_list() {
        // 乙太燈是「精工合成」：走工作台 3×3、不在背包 2×2 表。
        assert!(find_recipe("aether_lamp").is_none(), "乙太燈不該在背包配方表");
        let r = find_workbench_recipe("aether_lamp").unwrap();
        assert_eq!(r.output_block, 59, "乙太燈 id 應為 59（Block::AetherLamp）");
        assert_eq!(r.output_count, 1);
        assert_eq!(r.inputs, &[(58, 1), (10, 4)], "乙太燈需要 1 乙太礦 + 4 玻璃");
        // 乙太礦(58)是世界最稀有的礦——只需一顆當燈芯，不該大量消耗。
        let ore = r.inputs.iter().find(|&&(id, _)| id == 58).unwrap();
        assert_eq!(ore.1, 1, "稀有乙太礦只需 1 顆（世界最深珍寶，不該大量消耗）");
    }

    #[test]
    fn find_recipe_berry_bush_in_bag_list() {
        // 莓果叢苗走背包 2×2（樹苗 + 種子×2，3 格），不在工作台表。
        let r = find_recipe("berry_bush").expect("莓果叢苗應在背包配方表");
        assert!(find_workbench_recipe("berry_bush").is_none(), "莓果叢苗不該在工作台配方表");
        assert_eq!(
            r.output_block,
            crate::voxel_berry::BUSH_UNRIPE_ID,
            "莓果叢苗 id 應為 75（Block::BerryBush）"
        );
        assert_eq!(r.output_count, 1);
        assert_eq!(r.inputs, &[(65, 1), (14, 2)], "莓果叢苗需 1 樹苗 + 2 種子");
        // 多重集 {65:1,14:2} 獨一無二：沒有任何一條 2×2 配方用到樹苗(65) 或種子(14)。
        for other in RECIPES.iter().filter(|o| o.id != "berry_bush") {
            assert!(
                !other.inputs.iter().any(|&(id, _)| id == 65 || id == 14),
                "配方 {} 不該用到樹苗/種子，才能保證莓果叢苗多重集不撞",
                other.id
            );
        }
    }

    #[test]
    fn find_recipe_bench_in_bag_list() {
        // 木長椅走背包 2×2（2 木頭 + 2 木板，剛好 4 格），不在工作台表。
        let r = find_recipe("bench").expect("木長椅應在背包配方表");
        assert!(find_workbench_recipe("bench").is_none(), "木長椅不該在工作台配方表");
        assert_eq!(
            r.output_block,
            crate::voxel_bench::BENCH_ID,
            "木長椅 id 應為 79（Block::Bench）"
        );
        assert_eq!(r.output_count, 1);
        assert_eq!(r.inputs, &[(5, 2), (8, 2)], "木長椅需 2 木頭 + 2 木板");
        // 多重集 {5:2,8:2} 獨一無二：沒有別條 2×2 配方剛好是「2 木頭 + 2 木板」。
        for other in RECIPES.iter().filter(|o| o.id != "bench") {
            let two_wood = other.inputs.iter().any(|&(id, n)| id == 5 && n == 2);
            let two_plank = other.inputs.iter().any(|&(id, n)| id == 8 && n == 2);
            let only_two_kinds = other.inputs.len() == 2;
            assert!(
                !(two_wood && two_plank && only_two_kinds),
                "配方 {} 與木長椅多重集 {{5:2,8:2}} 相撞",
                other.id
            );
        }
    }

    #[test]
    fn find_recipe_furniture_in_bag_list() {
        // 玩家裝飾傢俱 v1（ROADMAP 931）：四樣皆走背包 2×2、不在工作台表，產出對應方塊 id。
        use crate::voxel_furniture::{BANNER_ID, CARPET_ID, FLOWERPOT_ID, TABLE_ID};
        let cases: [(&str, u8, u32, &[(u8, u32)]); 4] = [
            ("carpet", CARPET_ID, 2, &[(6, 3)]),
            ("flowerpot", FLOWERPOT_ID, 1, &[(89, 2), (6, 1)]),
            ("table", TABLE_ID, 1, &[(8, 2), (9, 1)]),
            ("banner", BANNER_ID, 1, &[(5, 1), (6, 2)]),
        ];
        for (id, out_block, out_count, inputs) in cases {
            let r = find_recipe(id).unwrap_or_else(|| panic!("裝飾傢俱「{id}」應在背包配方表"));
            assert!(find_workbench_recipe(id).is_none(), "裝飾傢俱「{id}」不該在工作台配方表");
            assert_eq!(r.output_block, out_block, "「{id}」產出方塊 id 不符");
            assert_eq!(r.output_count, out_count, "「{id}」產出數量不符");
            assert_eq!(r.inputs, inputs, "「{id}」配料不符");
        }
        // 四組多重集彼此互異，也與任何其餘背包配方皆不相撞（把 inputs 排序後當多重集鍵比對）。
        fn key(inputs: &[(u8, u32)]) -> Vec<(u8, u32)> {
            let mut v = inputs.to_vec();
            v.sort();
            v
        }
        let furniture_ids = ["carpet", "flowerpot", "table", "banner"];
        for &fid in &furniture_ids {
            let fk = key(find_recipe(fid).unwrap().inputs);
            for other in RECIPES.iter().filter(|o| o.id != fid) {
                assert_ne!(
                    key(other.inputs),
                    fk,
                    "裝飾傢俱「{fid}」與配方「{}」多重集相撞",
                    other.id
                );
            }
        }
    }

    #[test]
    fn find_recipe_campfire_in_workbench_list() {
        // 營火是「營地大物」：走工作台 3×3（6 格材料）、不在背包 2×2 表。
        assert!(find_recipe("campfire").is_none(), "營火不該在背包配方表");
        let r = find_workbench_recipe("campfire").unwrap();
        assert_eq!(r.output_block, 70, "營火 id 應為 70（Block::Campfire）");
        assert_eq!(r.output_count, 1);
        assert_eq!(r.inputs, &[(3, 3), (5, 2), (20, 1)], "營火需 3 石頭 + 2 木頭 + 1 煤礦");
    }

    #[test]
    fn find_recipe_bell_in_workbench_list() {
        // 集會鐘是「聚會核心」：走工作台 3×3（5 格材料、用到鐵錠）、不在背包 2×2 表。
        assert!(find_recipe("bell").is_none(), "集會鐘不該在背包配方表");
        let r = find_workbench_recipe("bell").unwrap();
        assert_eq!(r.output_block, 74, "集會鐘 id 應為 74（Block::Bell）");
        assert_eq!(r.output_count, 1);
        assert_eq!(r.inputs, &[(22, 4), (5, 1)], "集會鐘需 4 鐵錠 + 1 木頭");
        // 多重集不與既有鐵製工作台配方相撞（鐵磚 {22:6}、鐵鎬/斧 {22:3,8:2}、鐵鏟 {22:2,8:3}）。
        assert_ne!(r.inputs, find_workbench_recipe("iron_block").unwrap().inputs);
        assert_ne!(r.inputs, find_workbench_recipe("iron_pickaxe").unwrap().inputs);
        assert_ne!(r.inputs, find_workbench_recipe("iron_shovel").unwrap().inputs);
    }

    #[test]
    fn find_recipe_coop_in_workbench_list() {
        // 雞舍是「動物產物」新資源軸的起點：走工作台 3×3（6 格材料）、不在背包 2×2 表。
        assert!(find_recipe("coop").is_none(), "雞舍不該在背包配方表");
        let r = find_workbench_recipe("coop").unwrap();
        assert_eq!(r.output_block, crate::voxel_coop::COOP_ID, "雞舍 id 應為 80（Block::Coop）");
        assert_eq!(r.output_count, 1);
        assert_eq!(r.inputs, &[(5, 4), (6, 2)], "雞舍需 4 木頭 + 2 葉片");
        // 多重集 {5:4,6:2} 獨一無二：沒有別條工作台配方剛好是「4 木頭 + 2 葉片」。
        for other in WORKBENCH_RECIPES.iter().filter(|o| o.id != "coop") {
            assert_ne!(other.inputs, r.inputs, "配方 {} 不該與雞舍多重集相撞", other.id);
        }
    }

    #[test]
    fn find_recipe_returns_none_for_unknown() {
        assert!(find_recipe("unknown_xyz").is_none());
        assert!(find_recipe("").is_none());
        // 大小寫不符也回 None（id 是嚴格精確配對）。
        assert!(find_recipe("PLANK").is_none());
    }

    #[test]
    fn find_workbench_recipe_returns_correct_recipes() {
        let r = find_workbench_recipe("plank_wb").unwrap();
        assert_eq!(r.id, "plank_wb");
        assert_eq!(r.output_block, 8, "plank_wb 應產出木板(8)");
        assert_eq!(r.output_count, 16);
        assert!(find_workbench_recipe("stone_brick_wb").is_some());
        assert!(find_workbench_recipe("glass_wb").is_some());
        assert!(find_workbench_recipe("stone_wood_mix").is_some());
        assert!(find_workbench_recipe("farm_kit").is_some());
    }

    #[test]
    fn find_workbench_recipe_returns_none_for_bag_recipes() {
        // 工作台配方表不包含 2×2 配方
        assert!(find_workbench_recipe("plank").is_none());
        assert!(find_workbench_recipe("glass").is_none());
    }

    #[test]
    fn find_any_recipe_finds_both_lists() {
        // 背包配方
        assert!(find_any_recipe("plank").is_some());
        assert!(find_any_recipe("workbench").is_some());
        // 工作台配方
        assert!(find_any_recipe("plank_wb").is_some());
        assert!(find_any_recipe("stone_wood_mix").is_some());
        // 不存在的
        assert!(find_any_recipe("does_not_exist").is_none());
    }

    #[test]
    fn amulet_recipe_exists_in_taught_pool_only() {
        // 護身符：1 石頭(3) + 1 紅花(94) → 1 護身符(AMULET_ID=97)（居民教你一道獨門配方 v1）。
        let r = find_taught_recipe("amulet").unwrap();
        assert_eq!(r.output_block, AMULET_ID);
        assert_eq!(r.output_count, 1);
        assert_eq!(r.inputs, &[(3, 1), (94, 1)]);
        assert!(is_taught_recipe("amulet"));
        assert!(!is_taught_recipe("does_not_exist"));
        // 獨門配方刻意不併入一般三張表的搜尋鏈——沒學過不該直接查得到能合成。
        assert!(find_any_recipe("amulet").is_none(), "護身符不該出現在一般配方搜尋鏈裡");
    }

    #[test]
    fn vocation_charm_recipes_exist_with_expected_shape() {
        // 生計信物 v2（ROADMAP 1003）：五道新獨門配方各自存在、產出正確、不併入一般搜尋鏈。
        let cases: &[(&str, u8, &[(u8, u32)])] = &[
            ("farmer_charm", FARMER_CHARM_ID, &[(18, 1), (49, 1)]),
            ("smith_ring", SMITH_RING_ID, &[(22, 1), (3, 1)]),
            ("fisher_charm", FISHER_CHARM_ID, &[(4, 1), (61, 1)]),
            ("hunter_charm", HUNTER_CHARM_ID, &[(1, 1), (5, 1)]),
            ("merchant_charm", MERCHANT_CHARM_ID, &[(4, 1), (22, 1)]),
        ];
        for &(id, out_block, inputs) in cases {
            let r = find_taught_recipe(id).unwrap_or_else(|| panic!("生計信物「{id}」應在獨門配方池"));
            assert_eq!(r.output_block, out_block, "「{id}」產出方塊 id 不符");
            assert_eq!(r.output_count, 1);
            assert_eq!(r.inputs, inputs, "「{id}」配料不符");
            assert!(is_taught_recipe(id));
            assert!(find_any_recipe(id).is_none(), "「{id}」不該出現在一般配方搜尋鏈裡");
        }
    }

    #[test]
    fn taught_recipes_have_unique_input_multisets() {
        // 六道獨門配方（護身符 + 五道生計信物）彼此多重集互異，也與 RECIPES/WORKBENCH_RECIPES
        // 全表不撞——玩家在背包 2×2 格擺料時，永遠只會唯一比對到一道配方。
        fn key(inputs: &[(u8, u32)]) -> Vec<(u8, u32)> {
            let mut v = inputs.to_vec();
            v.sort();
            v
        }
        assert_eq!(TAUGHT_RECIPES.len(), 6, "獨門配方池應為 6 道（護身符 + 五道生計信物）");
        for (i, a) in TAUGHT_RECIPES.iter().enumerate() {
            for b in TAUGHT_RECIPES.iter().skip(i + 1) {
                assert_ne!(key(a.inputs), key(b.inputs), "獨門配方「{}」與「{}」多重集相撞", a.id, b.id);
            }
            for other in RECIPES.iter().chain(WORKBENCH_RECIPES.iter()) {
                assert_ne!(key(a.inputs), key(other.inputs), "獨門配方「{}」與配方「{}」多重集相撞", a.id, other.id);
            }
        }
    }

    #[test]
    fn all_recipes_have_nonempty_inputs() {
        for r in RECIPES.iter().chain(WORKBENCH_RECIPES.iter()).chain(FURNACE_RECIPES.iter()) {
            assert!(!r.inputs.is_empty(), "配方「{}」應有配料", r.id);
            for &(_, cnt) in r.inputs {
                assert!(cnt > 0, "配方「{}」配料數量應 > 0", r.id);
            }
        }
    }

    #[test]
    fn bread_recipe_exists_and_correct() {
        // 麵包配方：3 小麥(18) → 1 麵包(19)（ROADMAP 668）
        let r = find_recipe("bread").unwrap();
        assert_eq!(r.output_block, 19, "麵包 item id 應為 19");
        assert_eq!(r.output_count, 1);
        assert_eq!(r.inputs, &[(18, 3)], "麵包需要 3 小麥顆粒(18)");
    }

    #[test]
    fn all_recipes_output_crafted_block_ids() {
        // 2×2 配方產出 id：8–11、15（工作台）、19（麵包）、31（火把）、
        //   32/33（鎬具）、35（梯子）、36/37（斧頭）、39/40（鏟子 ROADMAP 690）
        for r in RECIPES {
            let ok = (r.output_block >= 8 && r.output_block <= 11)
                || r.output_block == 15
                || r.output_block == 19 // 麵包（純物品 id）
                || r.output_block == 31 // 火把（Torch，ROADMAP 685）
                || r.output_block == PICKAXE_WOOD_ID   // 木鎬（ROADMAP 687）
                || r.output_block == PICKAXE_STONE_ID  // 石鎬（ROADMAP 687）
                || r.output_block == 35 // 梯子（ROADMAP 688）
                || r.output_block == AXE_WOOD_ID   // 木斧（ROADMAP 689）
                || r.output_block == AXE_STONE_ID  // 石斧（ROADMAP 689）
                || r.output_block == SHOVEL_WOOD_ID  // 木鏟（ROADMAP 690）
                || r.output_block == SHOVEL_STONE_ID  // 石鏟（ROADMAP 690）
                || r.output_block == 43  // 木門（DoorClosed，ROADMAP 693）
                || r.output_block == 45  // 床（Block::Bed）
                || r.output_block == 57 // 冰晶燈（Block::IceLantern，冰晶合成 v1）
                || r.output_block == 66 // 告示牌（Block::Sign，告示牌 v1 ROADMAP 740）
                || r.output_block == crate::voxel_fishing::FISHING_ROD_ID // 釣竿（垂釣 v1，純物品 id=60）
                || r.output_block == crate::voxel_bucket::BUCKET_ID // 水桶（純物品 id=71，自主提案切片）
                || r.output_block == crate::voxel_hoe::HOE_ID // 木鋤頭（純物品 id=73，自主提案切片）
                || r.output_block == crate::voxel_berry::BUSH_UNRIPE_ID // 莓果叢苗（可放置方塊 id=75，自主提案切片 806）
                || r.output_block == crate::voxel_bench::BENCH_ID // 木長椅（可放置家具方塊 id=79，自主提案切片）
                || r.output_block == crate::voxel_bottle::BOTTLE_ID // 空玻璃瓶（純物品 id=83，漂流瓶 v1 自主提案切片 825）
                || (r.output_block >= 89 && r.output_block <= 92) // 四色陶磚（染色建材 v1，自主提案切片）
                || r.output_block == COIN_ID // 乙太幣（純物品 id=98，鑄幣 v1，ROADMAP 873 自主提案切片）
                || r.output_block == SWORD_WOOD_ID   // 木劍（純物品 id=99，驅影之劍 v1，ROADMAP 887 自主提案切片）
                || r.output_block == SWORD_STONE_ID  // 石劍（純物品 id=100，驅影之劍 v1）
                || (r.output_block >= crate::voxel_furniture::CARPET_ID
                    && r.output_block <= crate::voxel_furniture::BANNER_ID); // 四樣裝飾傢俱（可放置方塊 id=102~105，ROADMAP 931 自主提案切片）
            assert!(ok, "配方「{}」產出 id={} 超出允許範圍", r.id, r.output_block);
            assert!(r.output_count > 0, "配方「{}」產出數量應 > 0", r.id);
        }
        // 3×3 工作台配方產出 id
        //   （8~17、23 = IronBlock，34 = 鐵鎬、38 = 鐵斧、41 = 鐵鏟 ROADMAP 690、42 = 箱子 ROADMAP 692）
        for r in WORKBENCH_RECIPES {
            let ok = (r.output_block >= 8 && r.output_block <= 17)
                || r.output_block == 23
                || r.output_block == PICKAXE_IRON_ID  // 鐵鎬（ROADMAP 687）
                || r.output_block == AXE_IRON_ID      // 鐵斧（ROADMAP 689）
                || r.output_block == SHOVEL_IRON_ID   // 鐵鏟（ROADMAP 690）
                || r.output_block == 42                // 箱子（ROADMAP 692）
                || r.output_block == 59                // 乙太燈（Block::AetherLamp，乙太礦脈 v1）
                || r.output_block == STEW_ID           // 野菜暖湯（純物品，多食材料理 ROADMAP 778）
                || r.output_block == crate::voxel_firework::FIREWORK_ID // 乙太煙火（純物品 id=68，ROADMAP 785）
                || r.output_block == crate::voxel_compost::FERTILIZER_ID // 乙太沃肥（純物品 id=69，ROADMAP 789）
                || r.output_block == crate::voxel_campfire::CAMPFIRE_ID // 乙太營火（可放置發光方塊 id=70，自主提案切片）
                || r.output_block == crate::voxel_bell::BELL_ID // 集會鐘（可放置方塊 id=74，自主提案切片）
                || r.output_block == crate::voxel_coop::COOP_ID // 雞舍（可放置方塊 id=80，自主提案切片）
                || r.output_block == crate::voxel_blueprint::BLUEPRINT_HOUSE // 小屋藍圖（純物品 id=84，自主提案切片）
                || r.output_block == crate::voxel_blueprint::BLUEPRINT_WELL // 水井藍圖（純物品 id=85）
                || r.output_block == crate::voxel_blueprint::BLUEPRINT_TOWER // 瞭望台藍圖（純物品 id=86）
                || r.output_block == crate::voxel_blueprint::BLUEPRINT_GARDEN // 花圃藍圖（純物品 id=87）
                || r.output_block == crate::voxel_blueprint::BLUEPRINT_PAVILION // 涼亭藍圖（純物品 id=88）
                || r.output_block == SWORD_IRON_ID // 鐵劍（純物品 id=101，驅影之劍 v1，ROADMAP 887 自主提案切片）
                || r.output_block == STEAM_UNICYCLE_ID // 蒸汽獨輪車（純物品 id=115，ROADMAP 976 自主提案切片）
                || r.output_block == STREET_ACCORDION_ID; // 街頭手風琴（純物品 id=116，ROADMAP 977 自主提案切片）
            assert!(
                ok,
                "工作台配方「{}」產出 id={} 超出範圍",
                r.id, r.output_block
            );
            assert!(r.output_count > 0, "工作台配方「{}」產出數量應 > 0", r.id);
        }
        // 熔爐冶煉配方產出 id（8~17 建材、22=IronIngot、63=烤魚、64=烤地薯、78=莓果醬 食物）
        for r in FURNACE_RECIPES {
            let ok = (r.output_block >= 8 && r.output_block <= 17)
                || r.output_block == 22
                || r.output_block == 63
                || r.output_block == 64
                || r.output_block == crate::voxel_berry::JAM_ID;
            assert!(
                ok,
                "熔爐配方「{}」產出 id={} 超出範圍",
                r.id, r.output_block
            );
            assert!(r.output_count > 0, "熔爐配方「{}」產出數量應 > 0", r.id);
        }
    }

    #[test]
    fn can_craft_with_sufficient_materials() {
        let mut store = InvStore::default();
        store.give("旅人", 5, 4); // 4 木，需 2
        let r = find_recipe("plank").unwrap();
        assert!(can_craft(r, &store, "旅人"), "4 木應能合成木板");
    }

    #[test]
    fn can_craft_with_exact_materials() {
        let mut store = InvStore::default();
        store.give("旅人", 5, 2); // 剛好 2 木
        let r = find_recipe("plank").unwrap();
        assert!(can_craft(r, &store, "旅人"), "剛好 2 木應能合成");
    }

    #[test]
    fn cannot_craft_with_insufficient_materials() {
        let mut store = InvStore::default();
        store.give("旅人", 5, 1); // 只有 1 木，需 2
        let r = find_recipe("plank").unwrap();
        assert!(!can_craft(r, &store, "旅人"), "只有 1 木不夠合成木板");
    }

    #[test]
    fn cannot_craft_with_zero_materials() {
        let store = InvStore::default();
        for r in RECIPES.iter().chain(WORKBENCH_RECIPES.iter()).chain(FURNACE_RECIPES.iter()) {
            assert!(!can_craft(r, &store, "旅人"), "零材料不能合成「{}」", r.id);
        }
    }

    #[test]
    fn find_furnace_recipe_finds_all_smelt_recipes() {
        assert!(find_furnace_recipe("smelt_stone").is_some());
        assert!(find_furnace_recipe("smelt_glass").is_some());
        assert!(find_furnace_recipe("smelt_brick").is_some());
        assert!(find_furnace_recipe("smelt_fish").is_some());
        assert!(find_furnace_recipe("unknown").is_none());
        // 熔爐配方不在背包 / 工作台表
        assert!(find_recipe("smelt_stone").is_none());
        assert!(find_workbench_recipe("smelt_stone").is_none());
    }

    #[test]
    fn find_any_recipe_finds_furnace_recipes() {
        assert!(find_any_recipe("smelt_stone").is_some());
        assert!(find_any_recipe("smelt_glass").is_some());
        assert!(find_any_recipe("smelt_brick").is_some());
        // 熔爐工作台配方也在 find_any_recipe 範圍
        assert!(find_any_recipe("furnace_wb").is_some());
    }

    #[test]
    fn smelt_stone_outputs_smooth_stone() {
        let r = find_furnace_recipe("smelt_stone").unwrap();
        assert_eq!(r.output_block, 17, "拋光石 id 應為 17（SmoothStone）");
        assert_eq!(r.output_count, 3);
        assert_eq!(r.inputs, &[(3, 3)], "需 3 石頭");
    }

    #[test]
    fn smelt_fish_cooks_raw_into_cooked() {
        // 熔爐把 1 生小魚（61）烤成 1 烤魚（63）——垂釣→烹飪→餽贈循環的中間一環。
        let r = find_furnace_recipe("smelt_fish").unwrap();
        assert_eq!(r.inputs, &[(61, 1)], "需 1 生小魚（FISH_ID=61）");
        assert_eq!(r.output_block, 63, "產出烤魚（COOKED_FISH_ID=63）");
        assert_eq!(r.output_count, 1);
        // 烤魚配方只在熔爐表，不在背包 / 工作台表（要放置熔爐才能烤）。
        assert!(find_recipe("smelt_fish").is_none());
        assert!(find_workbench_recipe("smelt_fish").is_none());
    }

    #[test]
    fn smelt_potato_bakes_raw_into_baked() {
        // 熔爐把 1 生馬鈴薯（53）烤成 1 烤地薯（64）——種田→烹飪→餽贈循環的中間一環。
        let r = find_furnace_recipe("smelt_potato").unwrap();
        assert_eq!(r.inputs, &[(53, 1)], "需 1 生馬鈴薯（POTATO_ID=53）");
        assert_eq!(r.output_block, 64, "產出烤地薯（BAKED_POTATO_ID=64）");
        assert_eq!(r.output_count, 1);
        // 烤地薯配方只在熔爐表，不在背包 / 工作台表（要放置熔爐才能烤）。
        assert!(find_recipe("smelt_potato").is_none());
        assert!(find_workbench_recipe("smelt_potato").is_none());
        assert!(find_any_recipe("smelt_potato").is_some());
    }

    #[test]
    fn smelt_jam_boils_berries_into_jam() {
        // 熔爐把 3 莓果（77）慢熬成 1 莓果醬（78）——莓園→熬煮→享用／餽贈循環的中間一環
        // （莓果醬 v1 ROADMAP 808）。用常數對齊 voxel_berry，避免魔法數字漂移。
        let r = find_furnace_recipe("smelt_jam").unwrap();
        assert_eq!(
            r.inputs,
            &[(crate::voxel_berry::BERRY_ID, 3)],
            "需 3 莓果（BERRY_ID=77）"
        );
        assert_eq!(
            r.output_block,
            crate::voxel_berry::JAM_ID,
            "產出莓果醬（JAM_ID=78）"
        );
        assert_eq!(r.output_count, 1);
        // 莓果醬配方只在熔爐表（要放置熔爐才能熬），不在背包 / 工作台表。
        assert!(find_recipe("smelt_jam").is_none());
        assert!(find_workbench_recipe("smelt_jam").is_none());
        assert!(find_any_recipe("smelt_jam").is_some());
    }

    #[test]
    fn smelt_glass_better_yield_than_bag() {
        // 熔爐 2沙→3玻璃（1.5:1）> 背包 2沙→1玻璃（0.5:1）
        let furnace = find_furnace_recipe("smelt_glass").unwrap();
        let bag = find_recipe("glass").unwrap();
        assert_eq!(furnace.inputs, &[(4, 2)]);
        assert_eq!(bag.inputs, &[(4, 2)]);
        assert!(furnace.output_count > bag.output_count,
            "熔爐玻璃產量 {} 應 > 背包 {}", furnace.output_count, bag.output_count);
    }

    #[test]
    fn smelt_brick_double_yield_vs_bag() {
        // 熔爐 2石→4磚 > 背包 2石→2磚
        let furnace = find_furnace_recipe("smelt_brick").unwrap();
        let bag = find_recipe("stone_brick").unwrap();
        assert_eq!(furnace.inputs[0].1, bag.inputs[0].1, "消耗相同石頭數");
        assert!(furnace.output_count > bag.output_count,
            "熔爐磚產量 {} 應 > 背包 {}", furnace.output_count, bag.output_count);
    }

    #[test]
    fn furnace_wb_recipe_in_workbench_table() {
        let r = find_workbench_recipe("furnace_wb").unwrap();
        assert_eq!(r.output_block, 16, "熔爐 id 應為 16（Furnace）");
        assert_eq!(r.output_count, 1);
        assert_eq!(r.inputs, &[(3, 8)], "熔爐需 8 石頭");
    }

    #[test]
    fn can_craft_each_recipe_with_enough_materials() {
        let mut store = InvStore::default();
        store.give("旅人", 5, 10);  // Wood
        store.give("旅人", 3, 10);  // Stone
        store.give("旅人", 4, 10);  // Sand
        store.give("旅人", 1, 10);  // Grass（乙太沃肥配方用，ROADMAP 789）
        store.give("旅人", 2, 10);  // Dirt
        store.give("旅人", 8, 10);  // Plank（工作台 + stone_wood_mix + 箱子 8木板 用）
        store.give("旅人", 18, 10); // Wheat（麵包配方用，WHEAT_ID）
        store.give("旅人", 20, 10); // CoalOre（smelt_iron 燃料用）
        store.give("旅人", 21, 10); // IronOre（smelt_iron 原料用）
        store.give("旅人", 22, 10); // IronIngot（iron_block 配方用，ROADMAP 684）
        store.give("旅人", 6, 10);  // Leaves（床配方用）
        store.give("旅人", 10, 10); // Glass（冰晶燈 + 乙太燈配方用）
        store.give("旅人", 56, 10); // IceCrystal（冰晶燈配方用）
        store.give("旅人", 58, 10); // AetherOre（乙太燈配方用，乙太礦脈 v1）
        store.give("旅人", 61, 10); // FISH（smelt_fish 烤魚配方用，生小魚）
        store.give("旅人", 53, 10); // POTATO（smelt_potato 烤地薯／veggie_stew 暖湯配方用，馬鈴薯）
        store.give("旅人", 49, 10); // CARROT（veggie_stew 暖湯配方用，胡蘿蔔）
        store.give("旅人", 18, 10); // WHEAT（veggie_stew 暖湯配方用，小麥）
        store.give("旅人", 65, 10); // SAPLING（berry_bush 莓果叢苗配方用，樹苗，ROADMAP 806）
        store.give("旅人", 14, 10); // SEEDS（berry_bush 莓果叢苗配方用，種子，ROADMAP 806）
        store.give("旅人", crate::voxel_berry::BERRY_ID, 10); // BERRY（smelt_jam 莓果醬配方用，ROADMAP 808）
        store.give("旅人", 9, 10);  // StoneBrick（建築藍圖·水井/瞭望台配方用）
        store.give("旅人", 31, 10); // Torch（建築藍圖·涼亭配方用）
        store.give("旅人", 55, 10); // Snow（terracotta_white 白陶磚配方用，染色建材 v1）
        store.give("旅人", 89, 10); // TerracottaRed（flowerpot 花盆配方用，裝飾傢俱 v1 ROADMAP 931）
        // 火把配方：1 木頭(5) + 1 煤礦(20) → 4 火把（Wood/CoalOre 已加，數量足夠）
        for r in RECIPES.iter().chain(WORKBENCH_RECIPES.iter()).chain(FURNACE_RECIPES.iter()) {
            assert!(can_craft(r, &store, "旅人"), "配方「{}」材料足夠應可合成", r.id);
        }
    }

    #[test]
    fn can_craft_workbench_recipe_multi_material() {
        let mut store = InvStore::default();
        store.give("旅人", 3, 3); // 3 石
        store.give("旅人", 8, 3); // 3 木板
        let r = find_workbench_recipe("stone_wood_mix").unwrap();
        assert!(can_craft(r, &store, "旅人"), "3 石 + 3 木板應能合成混合石磚");
    }

    #[test]
    fn can_craft_different_players_independent() {
        let mut store = InvStore::default();
        store.give("玩家A", 5, 2); // A 有木
        let r = find_recipe("plank").unwrap();
        // A 可以合成，B 不行（零材料）。
        assert!(can_craft(r, &store, "玩家A"));
        assert!(!can_craft(r, &store, "玩家B"));
    }

    #[test]
    fn workbench_recipe_requires_more_materials_than_2x2() {
        // 所有工作台配方的總材料格數應 > 4（無法在 2×2 中完成）
        for r in WORKBENCH_RECIPES {
            let total: u32 = r.inputs.iter().map(|&(_, c)| c).sum();
            assert!(total > 4, "工作台配方「{}」總材料 {} 不夠大（應 > 4 格）", r.id, total);
        }
    }

    #[test]
    fn smelt_iron_outputs_iron_ingot() {
        // smelt_iron：1 鐵礦(21) + 1 煤礦(20) → 2 鐵錠(22)
        let r = find_furnace_recipe("smelt_iron").unwrap();
        assert_eq!(r.output_block, 22, "鐵錠 id 應為 22（IronIngot）");
        assert_eq!(r.output_count, 2, "應產出 2 個鐵錠");
        assert!(r.inputs.contains(&(21, 1)), "smelt_iron 需要 1 鐵礦(21)");
        assert!(r.inputs.contains(&(20, 1)), "smelt_iron 需要 1 煤礦(20)當燃料");
    }

    #[test]
    fn smelt_iron_requires_both_ores() {
        // 只有鐵礦或只有煤礦都不夠
        let r = find_furnace_recipe("smelt_iron").unwrap();
        let mut store_iron_only = InvStore::default();
        store_iron_only.give("旅人", 21, 5); // 有鐵礦但沒煤礦
        assert!(!can_craft(r, &store_iron_only, "旅人"), "只有鐵礦不夠 smelt_iron");

        let mut store_coal_only = InvStore::default();
        store_coal_only.give("旅人", 20, 5); // 有煤礦但沒鐵礦
        assert!(!can_craft(r, &store_coal_only, "旅人"), "只有煤礦不夠 smelt_iron");

        let mut store_both = InvStore::default();
        store_both.give("旅人", 21, 1);
        store_both.give("旅人", 20, 1);
        assert!(can_craft(r, &store_both, "旅人"), "各 1 鐵礦+煤礦可冶煉鐵錠");
    }

    #[test]
    fn smelt_iron_in_find_any_recipe() {
        // smelt_iron 應可透過統一查詢找到
        assert!(find_any_recipe("smelt_iron").is_some());
        // 但不在 2×2 或工作台表
        assert!(find_recipe("smelt_iron").is_none());
        assert!(find_workbench_recipe("smelt_iron").is_none());
    }

    #[test]
    fn iron_block_recipe_outputs_correct_block() {
        // iron_block：6 鐵錠(22) → 2 鐵磚(23)（ROADMAP 684）
        let r = find_workbench_recipe("iron_block").unwrap();
        assert_eq!(r.output_block, 23, "鐵磚 id 應為 23（IronBlock）");
        assert_eq!(r.output_count, 2, "6 鐵錠產出 2 鐵磚");
        assert!(r.inputs.contains(&(22, 6)), "iron_block 需要 6 鐵錠(22)");
    }

    #[test]
    fn iron_block_requires_six_ingots() {
        let r = find_workbench_recipe("iron_block").unwrap();

        let mut store_5 = InvStore::default();
        store_5.give("旅人", 22, 5);
        assert!(!can_craft(r, &store_5, "旅人"), "5 鐵錠不夠合鐵磚");

        let mut store_6 = InvStore::default();
        store_6.give("旅人", 22, 6);
        assert!(can_craft(r, &store_6, "旅人"), "6 鐵錠可合 2 鐵磚");
    }

    #[test]
    fn iron_block_in_find_any_recipe() {
        // iron_block 可透過統一查詢找到（工作台配方）
        assert!(find_any_recipe("iron_block").is_some());
        assert!(find_workbench_recipe("iron_block").is_some());
        // 不在 2×2 背包或熔爐表
        assert!(find_recipe("iron_block").is_none());
        assert!(find_furnace_recipe("iron_block").is_none());
    }

    #[test]
    fn torch_recipe_exists_and_correct() {
        // 火把配方：1 木頭(5) + 1 煤礦(20) → 4 火把(31)（ROADMAP 685）
        let r = find_recipe("torch").unwrap();
        assert_eq!(r.output_block, 31, "火把 id 應為 31（Torch）");
        assert_eq!(r.output_count, 4, "1 木+1 煤礦產出 4 火把");
        assert!(r.inputs.contains(&(5, 1)), "torch 需要 1 木頭(5)");
        assert!(r.inputs.contains(&(20, 1)), "torch 需要 1 煤礦(20)");
    }

    #[test]
    fn torch_requires_both_wood_and_coal() {
        let r = find_recipe("torch").unwrap();

        let mut store_wood_only = InvStore::default();
        store_wood_only.give("旅人", 5, 5); // 只有木頭沒煤礦
        assert!(!can_craft(r, &store_wood_only, "旅人"), "只有木頭不夠合火把");

        let mut store_coal_only = InvStore::default();
        store_coal_only.give("旅人", 20, 5); // 只有煤礦沒木頭
        assert!(!can_craft(r, &store_coal_only, "旅人"), "只有煤礦不夠合火把");

        let mut store_both = InvStore::default();
        store_both.give("旅人", 5, 1);
        store_both.give("旅人", 20, 1);
        assert!(can_craft(r, &store_both, "旅人"), "各 1 木頭+煤礦可合 4 火把");
    }

    #[test]
    fn torch_in_find_any_recipe() {
        // 火把是 2×2 背包配方，應可透過統一查詢和背包表找到
        assert!(find_any_recipe("torch").is_some());
        assert!(find_recipe("torch").is_some());
        // 不在工作台或熔爐表
        assert!(find_workbench_recipe("torch").is_none());
        assert!(find_furnace_recipe("torch").is_none());
    }

    // ── 鎬具配方測試（ROADMAP 687）────────────────────────────────────────────

    #[test]
    fn wood_pickaxe_recipe_correct() {
        let r = find_recipe("wood_pickaxe").unwrap();
        assert_eq!(r.output_block, PICKAXE_WOOD_ID, "木鎬 id 應為 {}", PICKAXE_WOOD_ID);
        assert_eq!(r.output_count, 1);
        // 需要 3 木頭(5) + 1 木板(8)，恰好填滿 2×2 四格
        assert!(r.inputs.iter().any(|&(b, c)| b == 5 && c == 3), "需要 3 木頭");
        assert!(r.inputs.iter().any(|&(b, c)| b == 8 && c == 1), "需要 1 木板");
    }

    #[test]
    fn stone_pickaxe_recipe_correct() {
        let r = find_recipe("stone_pickaxe").unwrap();
        assert_eq!(r.output_block, PICKAXE_STONE_ID, "石鎬 id 應為 {}", PICKAXE_STONE_ID);
        assert_eq!(r.output_count, 1);
        assert!(r.inputs.iter().any(|&(b, c)| b == 3 && c == 3), "需要 3 石頭");
        assert!(r.inputs.iter().any(|&(b, c)| b == 8 && c == 1), "需要 1 木板");
    }

    #[test]
    fn iron_pickaxe_in_workbench_only() {
        // 鐵鎬在工作台表（5 格材料，背包 2×2 放不下）
        let r = find_workbench_recipe("iron_pickaxe").unwrap();
        assert_eq!(r.output_block, PICKAXE_IRON_ID, "鐵鎬 id 應為 {}", PICKAXE_IRON_ID);
        assert_eq!(r.output_count, 1);
        assert!(r.inputs.iter().any(|&(b, c)| b == 22 && c == 3), "需要 3 鐵錠");
        assert!(r.inputs.iter().any(|&(b, c)| b == 8 && c == 2), "需要 2 木板");
        // 不在 2×2 背包表
        assert!(find_recipe("iron_pickaxe").is_none(), "鐵鎬不應在 2×2 背包表");
    }

    #[test]
    fn pickaxe_ids_unique_and_sequential() {
        // 三種鎬的 id 不重疊、依序增加
        assert!(PICKAXE_WOOD_ID < PICKAXE_STONE_ID);
        assert!(PICKAXE_STONE_ID < PICKAXE_IRON_ID);
        // 不與任何現有方塊衝突（現有已知方塊上限 = Torch=31）
        assert!(PICKAXE_WOOD_ID > 31, "鎬具 id 應高於現有方塊上限(31)");
    }

    #[test]
    fn ladder_recipe_in_2x2_bag() {
        // 梯子 v1（ROADMAP 688）：3 木板 → 3 梯子，在 2×2 背包合成
        let r = find_recipe("ladder").unwrap();
        assert_eq!(r.output_block, 35, "梯子 Block id 應為 35");
        assert_eq!(r.output_count, 3, "3 木板應得 3 梯子");
        assert_eq!(r.inputs, &[(8, 3)], "梯子配料應為 3 木板(id=8)");
        // 不在工作台或熔爐表（2×2 足夠）
        assert!(find_workbench_recipe("ladder").is_none(), "梯子不需工作台");
        assert!(find_furnace_recipe("ladder").is_none(),   "梯子不需熔爐");
    }

    #[test]
    fn ladder_block_is_not_solid() {
        // 梯子非實心——玩家可穿入並攀爬（ROADMAP 688）
        use crate::voxel::Block;
        assert!(!Block::Ladder.is_solid(), "梯子不應為實心（攀爬語意）");
        assert!(Block::Ladder.is_climbable(), "梯子應為可攀爬");
        assert!(Block::Ladder.is_placeable(), "梯子應可放置");
        // 非水
        assert!(!Block::Ladder.is_any_water());
        assert!(!Block::Ladder.is_flowing_water());
    }

    // ── 斧頭配方測試（ROADMAP 689）────────────────────────────────────────────

    #[test]
    fn wood_axe_recipe_correct() {
        let r = find_recipe("wood_axe").unwrap();
        assert_eq!(r.output_block, AXE_WOOD_ID, "木斧 id 應為 {}", AXE_WOOD_ID);
        assert_eq!(r.output_count, 1);
        assert!(r.inputs.iter().any(|&(b, c)| b == 5 && c == 3), "需要 3 木頭");
        assert!(r.inputs.iter().any(|&(b, c)| b == 8 && c == 1), "需要 1 木板");
    }

    #[test]
    fn stone_axe_recipe_correct() {
        let r = find_recipe("stone_axe").unwrap();
        assert_eq!(r.output_block, AXE_STONE_ID, "石斧 id 應為 {}", AXE_STONE_ID);
        assert_eq!(r.output_count, 1);
        assert!(r.inputs.iter().any(|&(b, c)| b == 3 && c == 3), "需要 3 石頭");
        assert!(r.inputs.iter().any(|&(b, c)| b == 8 && c == 1), "需要 1 木板");
    }

    #[test]
    fn iron_axe_in_workbench_only() {
        // 鐵斧在工作台表（5 格材料，背包 2×2 放不下）
        let r = find_workbench_recipe("iron_axe").unwrap();
        assert_eq!(r.output_block, AXE_IRON_ID, "鐵斧 id 應為 {}", AXE_IRON_ID);
        assert_eq!(r.output_count, 1);
        assert!(r.inputs.iter().any(|&(b, c)| b == 22 && c == 3), "需要 3 鐵錠");
        assert!(r.inputs.iter().any(|&(b, c)| b == 8 && c == 2), "需要 2 木板");
        assert!(find_recipe("iron_axe").is_none(), "鐵斧不應在 2×2 背包表");
    }

    #[test]
    fn axe_ids_unique_and_not_conflicting() {
        // 三種斧頭 id 不重疊
        assert!(AXE_WOOD_ID < AXE_STONE_ID);
        assert!(AXE_STONE_ID < AXE_IRON_ID);
        // 不與鎬具 id 衝突（鎬具 32/33/34、梯子 block=35）
        assert!(AXE_WOOD_ID > PICKAXE_IRON_ID, "斧頭 id 應高於鎬具最大 id(34)");
        // 不與梯子 block id=35 衝突（36/37/38 均 > 35）
        assert!(AXE_WOOD_ID > 35, "斧頭 id 應高於梯子 block id(35)");
    }

    // ── 鏟子配方測試（ROADMAP 690）────────────────────────────────────────────

    #[test]
    fn wood_shovel_recipe_correct() {
        let r = find_recipe("wood_shovel").unwrap();
        assert_eq!(r.output_block, SHOVEL_WOOD_ID, "木鏟 id 應為 {}", SHOVEL_WOOD_ID);
        assert_eq!(r.output_count, 1);
        // 1 木頭(5) + 1 木板(8)
        assert!(r.inputs.iter().any(|&(b, c)| b == 5 && c == 1), "需要 1 木頭");
        assert!(r.inputs.iter().any(|&(b, c)| b == 8 && c == 1), "需要 1 木板");
    }

    #[test]
    fn stone_shovel_recipe_correct() {
        let r = find_recipe("stone_shovel").unwrap();
        assert_eq!(r.output_block, SHOVEL_STONE_ID, "石鏟 id 應為 {}", SHOVEL_STONE_ID);
        assert_eq!(r.output_count, 1);
        assert!(r.inputs.iter().any(|&(b, c)| b == 3 && c == 1), "需要 1 石頭");
        assert!(r.inputs.iter().any(|&(b, c)| b == 8 && c == 1), "需要 1 木板");
    }

    #[test]
    fn iron_shovel_in_workbench_only() {
        // 鐵鏟在工作台表（5 格材料，工藝精工需工作台）
        let r = find_workbench_recipe("iron_shovel").unwrap();
        assert_eq!(r.output_block, SHOVEL_IRON_ID, "鐵鏟 id 應為 {}", SHOVEL_IRON_ID);
        assert_eq!(r.output_count, 1);
        assert!(r.inputs.iter().any(|&(b, c)| b == 22 && c == 2), "需要 2 鐵錠");
        assert!(r.inputs.iter().any(|&(b, c)| b == 8 && c == 3), "需要 3 木板");
        // 不在 2×2 背包表
        assert!(find_recipe("iron_shovel").is_none(), "鐵鏟不應在 2×2 背包表");
    }

    #[test]
    fn shovel_ids_unique_and_not_conflicting() {
        // 三種鏟子 id 不重疊、依序增加
        assert!(SHOVEL_WOOD_ID < SHOVEL_STONE_ID);
        assert!(SHOVEL_STONE_ID < SHOVEL_IRON_ID);
        // 不與斧頭衝突（斧頭 36/37/38）
        assert!(SHOVEL_WOOD_ID > AXE_IRON_ID, "鏟子 id 應高於斧頭最大 id(38)");
    }

    // ── 箱子配方測試（ROADMAP 692）────────────────────────────────────────────

    #[test]
    fn chest_recipe_in_workbench_only() {
        // 箱子：8 木板(8) → 1 箱子(42)，需工作台（8 格超出 2×2 上限）
        let r = find_workbench_recipe("chest").unwrap();
        assert_eq!(r.output_block, 42, "箱子 block id 應為 42");
        assert_eq!(r.output_count, 1, "8 木板得 1 箱子");
        assert_eq!(r.inputs, &[(8, 8)], "箱子需要 8 木板(id=8)");
        // 不在 2×2 背包表
        assert!(find_recipe("chest").is_none(), "箱子不在 2×2 背包表");
        assert!(find_furnace_recipe("chest").is_none(), "箱子不在熔爐表");
    }

    #[test]
    fn chest_requires_eight_planks() {
        let r = find_workbench_recipe("chest").unwrap();

        let mut store_7 = InvStore::default();
        store_7.give("旅人", 8, 7); // 只有 7 木板
        assert!(!can_craft(r, &store_7, "旅人"), "7 木板不夠合箱子（需 8）");

        let mut store_8 = InvStore::default();
        store_8.give("旅人", 8, 8); // 剛好 8 木板
        assert!(can_craft(r, &store_8, "旅人"), "8 木板可合 1 箱子");
    }

    #[test]
    fn chest_in_find_any_recipe() {
        // 箱子可透過統一查詢找到（工作台配方）
        assert!(find_any_recipe("chest").is_some());
        assert!(find_workbench_recipe("chest").is_some());
        // 不在 2×2 背包或熔爐表
        assert!(find_recipe("chest").is_none());
        assert!(find_furnace_recipe("chest").is_none());
    }

    // ── 木門配方測試（ROADMAP 693）──────────────────────────────────────────────

    #[test]
    fn door_recipe_in_bag_2x2() {
        // 木門：4 木板(8) → 2 門(43)，在背包 2×2 格合成
        let r = find_recipe("door").unwrap();
        assert_eq!(r.output_block, 43, "木門 block id 應為 43（DoorClosed）");
        assert_eq!(r.output_count, 2, "4 木板得 2 扇門");
        assert_eq!(r.inputs, &[(8, 4)], "木門需要 4 木板(id=8)");
        // 不在工作台表或熔爐表
        assert!(find_workbench_recipe("door").is_none(), "木門不在工作台表");
        assert!(find_furnace_recipe("door").is_none(), "木門不在熔爐表");
    }

    #[test]
    fn door_craft_requires_four_planks() {
        let r = find_recipe("door").unwrap();

        let mut store_3 = InvStore::default();
        store_3.give("旅人", 8, 3); // 只有 3 木板
        assert!(!can_craft(r, &store_3, "旅人"), "3 木板不夠合木門（需 4）");

        let mut store_4 = InvStore::default();
        store_4.give("旅人", 8, 4); // 剛好 4 木板
        assert!(can_craft(r, &store_4, "旅人"), "4 木板可合 2 扇門");
    }

    #[test]
    fn door_in_find_any_recipe() {
        // 木門可透過統一查詢找到（2×2 配方）
        assert!(find_any_recipe("door").is_some());
        assert!(find_recipe("door").is_some());
        // 不在工作台或熔爐表
        assert!(find_workbench_recipe("door").is_none());
        assert!(find_furnace_recipe("door").is_none());
    }

    // ── 野菜暖湯 v1（ROADMAP 778）───────────────────────────────────────────────

    #[test]
    fn veggie_stew_is_workbench_recipe() {
        // 多食材料理是「複雜合成」→ 只在工作台表，不在背包 2×2 表
        assert!(find_workbench_recipe("veggie_stew").is_some());
        assert!(find_recipe("veggie_stew").is_none(), "暖湯是複雜料理、不該在背包 2×2 表");
        assert!(find_furnace_recipe("veggie_stew").is_none(), "暖湯走工作台拌煮、非熔爐");
    }

    #[test]
    fn veggie_stew_outputs_stew_item() {
        let r = find_workbench_recipe("veggie_stew").unwrap();
        assert_eq!(r.output_block, STEW_ID, "產出應為野菜暖湯(67)");
        assert_eq!(r.output_count, 1, "一鍋暖湯");
        assert_eq!(STEW_ID, 67, "暖湯物品 id 契約鎖定為 67（前後端對齊）");
    }

    #[test]
    fn veggie_stew_needs_three_distinct_crops() {
        // 暖湯之所以最費心：要湊齊三種不同作物、滿滿一大鍋（2+2+1），缺一不可
        let r = find_workbench_recipe("veggie_stew").unwrap();
        assert!(r.inputs.contains(&(49, 2)), "需要 2 胡蘿蔔(49)");
        assert!(r.inputs.contains(&(53, 2)), "需要 2 馬鈴薯(53)");
        assert!(r.inputs.contains(&(18, 1)), "需要 1 小麥(18)");
        assert_eq!(r.inputs.len(), 3, "剛好三種作物");
        // 一大鍋：總材料 > 4，塞不進背包 2×2，必須在工作台煮
        let total: u32 = r.inputs.iter().map(|&(_, c)| c).sum();
        assert!(total > 4, "暖湯份量應超出 2×2（總材料 {total}）");
    }

    #[test]
    fn veggie_stew_requires_all_three_crops() {
        let r = find_workbench_recipe("veggie_stew").unwrap();

        // 只有兩種作物（缺小麥）→ 湊不成一鍋湯
        let mut two = InvStore::default();
        two.give("旅人", 49, 2);
        two.give("旅人", 53, 2);
        assert!(!can_craft(r, &two, "旅人"), "缺小麥煮不成暖湯");

        // 三種齊全但份量不足（各 1）→ 也不夠一大鍋
        let mut skimpy = InvStore::default();
        skimpy.give("旅人", 49, 1);
        skimpy.give("旅人", 53, 1);
        skimpy.give("旅人", 18, 1);
        assert!(!can_craft(r, &skimpy, "旅人"), "各 1 顆不夠煮一大鍋暖湯");

        // 份量齊全 → 可煮
        let mut all = InvStore::default();
        all.give("旅人", 49, 2);
        all.give("旅人", 53, 2);
        all.give("旅人", 18, 1);
        assert!(can_craft(r, &all, "旅人"), "2 胡蘿蔔+2 馬鈴薯+1 小麥可煮一鍋暖湯");
    }

    #[test]
    fn veggie_stew_in_find_any_recipe() {
        assert!(find_any_recipe("veggie_stew").is_some());
    }

    #[test]
    fn terracotta_recipes_in_bag_list_not_workbench() {
        // 染色建材 v1：四色陶磚皆為 3 格配方，比照冰晶燈慣例留在背包 2×2，不因材料稀有硬塞工作台。
        for id in ["terracotta_red", "terracotta_black", "terracotta_white", "terracotta_blue"] {
            assert!(find_recipe(id).is_some(), "「{id}」應在背包配方表");
            assert!(find_workbench_recipe(id).is_none(), "「{id}」不該在工作台配方表");
        }
    }

    #[test]
    fn terracotta_recipes_correct_inputs_and_outputs() {
        let red = find_recipe("terracotta_red").unwrap();
        assert_eq!(red.inputs, &[(4, 2), (21, 1)], "紅陶磚需 2 沙 + 1 鐵礦");
        assert_eq!(red.output_block, 89);
        assert_eq!(red.output_count, 2);

        let black = find_recipe("terracotta_black").unwrap();
        assert_eq!(black.inputs, &[(4, 2), (20, 1)], "黑陶磚需 2 沙 + 1 煤礦");
        assert_eq!(black.output_block, 90);
        assert_eq!(black.output_count, 2);

        let white = find_recipe("terracotta_white").unwrap();
        assert_eq!(white.inputs, &[(4, 2), (55, 1)], "白陶磚需 2 沙 + 1 雪");
        assert_eq!(white.output_block, 91);
        assert_eq!(white.output_count, 2);

        let blue = find_recipe("terracotta_blue").unwrap();
        assert_eq!(blue.inputs, &[(4, 2), (58, 1)], "青陶磚需 2 沙 + 1 乙太礦");
        assert_eq!(blue.output_block, 92);
        assert_eq!(blue.output_count, 2);
    }

    #[test]
    fn terracotta_recipes_have_unique_input_multisets() {
        // 四色互不相撞，也不與既有配方（玻璃 {4:2}、乙太煙火 {58:1,20:2,4:2}）相撞。
        let sets: Vec<Vec<(u8, u32)>> = ["terracotta_red", "terracotta_black", "terracotta_white", "terracotta_blue"]
            .iter()
            .map(|id| {
                let mut v = find_recipe(id).unwrap().inputs.to_vec();
                v.sort();
                v
            })
            .collect();
        for i in 0..sets.len() {
            for j in (i + 1)..sets.len() {
                assert_ne!(sets[i], sets[j], "陶磚配方彼此不應共用同一多重集");
            }
        }
        for r in RECIPES {
            if r.id.starts_with("terracotta_") {
                continue;
            }
            let mut other = r.inputs.to_vec();
            other.sort();
            assert!(!sets.contains(&other), "配方「{}」不該與陶磚多重集相撞", r.id);
        }
    }

    #[test]
    fn terracotta_craft_requires_correct_materials() {
        let r = find_recipe("terracotta_red").unwrap();
        let mut short = InvStore::default();
        short.give("旅人", 4, 2); // 沙夠，缺鐵礦
        assert!(!can_craft(r, &short, "旅人"), "缺鐵礦不能合紅陶磚");

        let mut ok = InvStore::default();
        ok.give("旅人", 4, 2);
        ok.give("旅人", 21, 1);
        assert!(can_craft(r, &ok, "旅人"), "2 沙 + 1 鐵礦應可合紅陶磚");
    }

    #[test]
    fn coin_recipes_output_coin_id_with_expected_rate() {
        for (id, input_block, input_count) in
            [("coin_from_wood", 5u8, 4u32), ("coin_from_stone", 3, 4), ("coin_from_sand", 4, 4)]
        {
            let r = find_recipe(id).unwrap();
            assert_eq!(r.inputs, &[(input_block, input_count)]);
            assert_eq!(r.output_block, COIN_ID, "配方「{id}」應鑄出乙太幣");
            assert_eq!(r.output_count, 1);
        }
    }

    #[test]
    fn coin_recipes_have_unique_input_multisets() {
        // 三條鑄幣配方彼此不相撞，也不與其餘既有配方（木板 {5:2}、石磚 {3:2}、玻璃 {4:2}
        // 等）相撞——多重集比對的是「數量也要一致」，故 {5:4} 與 {5:2} 天生不同。
        let sets: Vec<Vec<(u8, u32)>> =
            ["coin_from_wood", "coin_from_stone", "coin_from_sand"]
                .iter()
                .map(|id| find_recipe(id).unwrap().inputs.to_vec())
                .collect();
        for i in 0..sets.len() {
            for j in (i + 1)..sets.len() {
                assert_ne!(sets[i], sets[j], "鑄幣配方彼此不應共用同一多重集");
            }
        }
        for r in RECIPES {
            if r.id.starts_with("coin_from_") {
                continue;
            }
            let mut other = r.inputs.to_vec();
            other.sort();
            assert!(!sets.contains(&other), "配方「{}」不該與鑄幣多重集相撞", r.id);
        }
    }

    // ── 驅影之劍 v1（ROADMAP 887，自主提案切片）測試 ───────────────────────────────
    #[test]
    fn sword_power_ladder_is_wood2_stone3_iron3_others0() {
        // 武器傷害階梯：木劍 2、石劍 3（一擊即散）、鐵劍 3；非武器（含徒手/工具/方塊）皆 0。
        assert_eq!(sword_power(SWORD_WOOD_ID), 2);
        assert_eq!(sword_power(SWORD_STONE_ID), 3);
        assert_eq!(sword_power(SWORD_IRON_ID), 3);
        assert_eq!(sword_power(PICKAXE_IRON_ID), 0, "鎬不是武器");
        assert_eq!(sword_power(0), 0, "空手不是武器");
        assert_eq!(sword_power(5), 0, "木頭方塊不是武器");
    }

    #[test]
    fn is_sword_only_true_for_the_three_swords() {
        assert!(is_sword(SWORD_WOOD_ID) && is_sword(SWORD_STONE_ID) && is_sword(SWORD_IRON_ID));
        assert!(!is_sword(PICKAXE_WOOD_ID) && !is_sword(AXE_IRON_ID) && !is_sword(COIN_ID));
        assert!(!is_sword(0));
    }

    #[test]
    fn only_iron_sword_gives_extra_shard() {
        // 雙倍溫柔獎勵只給鐵劍；木劍/石劍/徒手皆無額外。
        assert_eq!(sword_extra_shards(SWORD_IRON_ID), 1);
        assert_eq!(sword_extra_shards(SWORD_WOOD_ID), 0);
        assert_eq!(sword_extra_shards(SWORD_STONE_ID), 0);
        assert_eq!(sword_extra_shards(0), 0);
    }

    #[test]
    fn sword_recipes_exist_and_output_expected_ids() {
        assert_eq!(find_recipe("wood_sword").unwrap().output_block, SWORD_WOOD_ID);
        assert_eq!(find_recipe("stone_sword").unwrap().output_block, SWORD_STONE_ID);
        // 鐵劍在工作台清單，不在背包 2×2 清單。
        assert!(find_recipe("iron_sword").is_none(), "鐵劍應只在工作台配方");
        assert_eq!(find_workbench_recipe("iron_sword").unwrap().output_block, SWORD_IRON_ID);
    }

    #[test]
    fn sword_recipes_have_unique_input_multisets() {
        // 木劍/石劍不與任何其他 2×2 配方共用多重集；鐵劍不與任何其他工作台配方共用。
        let bag_ids = ["wood_sword", "stone_sword"];
        for sid in bag_ids {
            let mut mine = find_recipe(sid).unwrap().inputs.to_vec();
            mine.sort();
            for r in RECIPES {
                if r.id == sid {
                    continue;
                }
                let mut other = r.inputs.to_vec();
                other.sort();
                assert_ne!(mine, other, "劍配方「{sid}」與 2×2 配方「{}」多重集相撞", r.id);
            }
        }
        let mut iron = find_workbench_recipe("iron_sword").unwrap().inputs.to_vec();
        iron.sort();
        for r in WORKBENCH_RECIPES {
            if r.id == "iron_sword" {
                continue;
            }
            let mut other = r.inputs.to_vec();
            other.sort();
            assert_ne!(iron, other, "鐵劍與工作台配方「{}」多重集相撞", r.id);
        }
    }

    #[test]
    fn coin_craft_requires_correct_material_count() {
        let r = find_recipe("coin_from_wood").unwrap();
        let mut short = InvStore::default();
        short.give("旅人", 5, 3); // 只有 3 木頭，缺 1
        assert!(!can_craft(r, &short, "旅人"), "木頭不足 4 不能鑄幣");

        let mut ok = InvStore::default();
        ok.give("旅人", 5, 4);
        assert!(can_craft(r, &ok, "旅人"), "4 木頭應可鑄 1 乙太幣");
    }

    // ── 蒸汽獨輪車 v1（ROADMAP 976，自主提案切片）───────────────────────────────────

    #[test]
    fn steam_unicycle_id_is_first_free_slot_after_relic_glow() {
        // 114（遺跡核心）是目前最大已用 id，115 應是首個空號。
        assert_eq!(STEAM_UNICYCLE_ID, 115);
    }

    #[test]
    fn steam_unicycle_recipe_exists_only_in_workbench_table() {
        assert!(find_recipe("steam_unicycle").is_none(), "蒸汽獨輪車應只在工作台配方（7 格超出背包 2×2）");
        let r = find_workbench_recipe("steam_unicycle").unwrap();
        assert_eq!(r.output_block, STEAM_UNICYCLE_ID);
        assert_eq!(r.output_count, 1);
        assert_eq!(r.inputs, &[(22, 4), (5, 2), (58, 1)], "應為 4 鐵錠 + 2 木頭 + 1 乙太礦");
    }

    #[test]
    fn steam_unicycle_recipe_findable_via_find_any_recipe() {
        // WS Craft handler 統一走 find_any_recipe，確保新配方也能被這條路徑找到。
        let r = find_any_recipe("steam_unicycle").unwrap();
        assert_eq!(r.output_block, STEAM_UNICYCLE_ID);
    }

    #[test]
    fn steam_unicycle_recipe_has_unique_input_multiset() {
        // {22:4,5:2,58:1} 不應與任何其他工作台配方的多重集相撞。
        let mut mine = find_workbench_recipe("steam_unicycle").unwrap().inputs.to_vec();
        mine.sort();
        for r in WORKBENCH_RECIPES {
            if r.id == "steam_unicycle" {
                continue;
            }
            let mut other = r.inputs.to_vec();
            other.sort();
            assert_ne!(mine, other, "蒸汽獨輪車與工作台配方「{}」多重集相撞", r.id);
        }
    }

    #[test]
    fn steam_unicycle_craft_requires_full_material_set() {
        let r = find_workbench_recipe("steam_unicycle").unwrap();
        let mut short = InvStore::default();
        short.give("旅人", 22, 4);
        short.give("旅人", 5, 2);
        // 缺乙太礦(58) → 不能合成
        assert!(!can_craft(r, &short, "旅人"), "缺乙太礦不該能合成蒸汽獨輪車");

        let mut ok = InvStore::default();
        ok.give("旅人", 22, 4);
        ok.give("旅人", 5, 2);
        ok.give("旅人", 58, 1);
        assert!(can_craft(r, &ok, "旅人"), "湊齊 4 鐵錠+2 木頭+1 乙太礦應可合成");
    }

    #[test]
    fn can_start_riding_requires_real_item_ownership() {
        // 伺服器權威判定：持有才准騎，不持有拒絕（不信客戶端自報）。
        assert!(can_start_riding(true), "真持有蒸汽獨輪車應允許騎乘");
        assert!(!can_start_riding(false), "沒有蒸汽獨輪車不該允許騎乘");
    }

    // ── 街頭手風琴 v1（ROADMAP 977，自主提案切片）───────────────────────────────────

    #[test]
    fn street_accordion_id_is_first_free_slot_after_steam_unicycle() {
        // 115（蒸汽獨輪車）是目前最大已用 id，116 應是首個空號。
        assert_eq!(STREET_ACCORDION_ID, 116);
    }

    #[test]
    fn street_accordion_recipe_exists_only_in_workbench_table() {
        assert!(find_recipe("street_accordion").is_none(), "街頭手風琴應只在工作台配方（6 格超出背包 2×2）");
        let r = find_workbench_recipe("street_accordion").unwrap();
        assert_eq!(r.output_block, STREET_ACCORDION_ID);
        assert_eq!(r.output_count, 1);
        assert_eq!(r.inputs, &[(8, 3), (22, 2), (10, 1)], "應為 3 木板 + 2 鐵錠 + 1 玻璃");
    }

    #[test]
    fn street_accordion_recipe_findable_via_find_any_recipe() {
        // WS Craft handler 統一走 find_any_recipe，確保新配方也能被這條路徑找到。
        let r = find_any_recipe("street_accordion").unwrap();
        assert_eq!(r.output_block, STREET_ACCORDION_ID);
    }

    #[test]
    fn street_accordion_recipe_has_unique_input_multiset() {
        // {8:3,22:2,10:1} 不應與任何其他工作台配方的多重集相撞。
        let mut mine = find_workbench_recipe("street_accordion").unwrap().inputs.to_vec();
        mine.sort();
        for r in WORKBENCH_RECIPES {
            if r.id == "street_accordion" {
                continue;
            }
            let mut other = r.inputs.to_vec();
            other.sort();
            assert_ne!(mine, other, "街頭手風琴與工作台配方「{}」多重集相撞", r.id);
        }
    }

    #[test]
    fn street_accordion_craft_requires_full_material_set() {
        let r = find_workbench_recipe("street_accordion").unwrap();
        let mut short = InvStore::default();
        short.give("旅人", 8, 3);
        short.give("旅人", 22, 2);
        // 缺玻璃(10) → 不能合成
        assert!(!can_craft(r, &short, "旅人"), "缺玻璃不該能合成街頭手風琴");

        let mut ok = InvStore::default();
        ok.give("旅人", 8, 3);
        ok.give("旅人", 22, 2);
        ok.give("旅人", 10, 1);
        assert!(can_craft(r, &ok, "旅人"), "湊齊 3 木板+2 鐵錠+1 玻璃應可合成");
    }

    #[test]
    fn can_start_performing_requires_real_item_ownership() {
        // 伺服器權威判定：持有才准開演，不持有拒絕（不信客戶端自報）。
        assert!(can_start_performing(true), "真持有街頭手風琴應允許開演");
        assert!(!can_start_performing(false), "沒有街頭手風琴不該允許開演");
    }
}
