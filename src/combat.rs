//! 敵人模型（Phase 1 戰鬥 MVP「自動打怪」的純邏輯層）。
//!
//! 這層只管「一隻敵人怎麼被打、被打倒後掉什麼、之後怎麼重生」，是純資料 + 純函式，
//! 無 IO、不碰 WebSocket / 遊戲迴圈，便於自動測試。延續 `gather.rs` / `crops.rs` 的
//! 慣例：純邏輯獨立可測，由上層接線餵呼叫。**戰鬥已接線上線**，呼叫鏈如下：
//!   - 世界：`enemy_field` 在曠野撒佈若干 `Enemy`（比照 `gather_field.rs` 之於
//!     `gather.rs`），並管「敵人擺哪、角色自動鎖定最近的哪一隻」。
//!   - ws / 遊戲迴圈：角色靠近時自動攻擊 → `Enemy::attack(power)`（見 `game.rs`，攻擊力
//!     目前是寫死常數 `PLAYER_ATTACK_POWER`，待武器接線後改查表）；打倒回傳掉落 → `add`
//!     進背包；快照把 `remaining_hp` 廣播給前端畫血條。
//!   - 遊戲迴圈：每 tick 對被打倒的敵人呼叫 `tick(dt)` 倒數重生。
//!   - 載入：`enemy_field::from_saved` 收存檔敵人時逐隻走 `is_loadable` 驗證。
//!
//! 唯一尚未接的是「敵人狀態進 0-E 持久化」（目前 `EnemyField` 每次啟動重新撒佈）。
//!
//! 戰鬥迴圈刻意鏡像採集（`ResourceNode`）：敵人有「生命」（像耐久），每次攻擊扣血、
//! 打到 0 即被打倒並**一次性**掉落戰利品，接著進入重生倒數，倒數到了滿血復活再次可打——
//! 敵人不是無限白刷，打完一處得換地方或等它重生，給世界一點節奏。
//!
//! 主題是療癒的蒸汽龐克太空歌劇，敵人不是嚇人的怪物，而是失控的機械 / 野化的乙太生靈；
//! 「打倒」更接近安撫 / 拆解，落下可用的素材與乙太。`threat` 是這隻敵人每次反擊的傷害，
//! 由 `enemy_field::EnemyField::threat_at` 聚合成「玩家此刻承受的反擊威脅」、接線層再餵進
//! 玩家 `Vitals`——讓「自動打怪」不是無傷收割，而是雙向有來有回。

use serde::{Deserialize, Serialize};

use crate::inventory::{Inventory, ItemKind};

// ───────────────────────── 武器（Phase 1 武器 MVP，純邏輯查表）─────────────────────────
//
// 戰鬥的「裝備」這環：採集那側 `tools::gather_speed_multiplier` 依背包工具決定採集倍率，
// 戰鬥這側鏡像它——`weapon_power` 依背包武器決定每下攻擊力。`game.rs` 的攻擊接線目前寫死
// 常數 `PLAYER_ATTACK_POWER`（徒手值 2），接線輪只要把那一行換成 `weapon_power(&inv)`：
// 身上有武器回高攻擊力、沒有回徒手值。無 IO、無新 protocol、不動廣播 shape——背包已隨快照
// 廣播，武器只是多一種背包物品。本層純資料 + 純函式，便於自動測試。

/// 玩家用來戰鬥的武器。`Unarmed` 是身上沒武器時的退路（只有徒手攻擊力）。
/// 鏡像 `tools::ToolKind`：日後加新武器階級（強化武器…）時，往這個 enum 加一個變體、
/// 補進 `attack_power` 與 `weapon_from_item` 的窮舉 `match` 即可。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeaponKind {
    /// 徒手——任何人都能打，但只有基礎攻擊力。
    Unarmed,
    /// 武器（合成產物）：每下攻擊更痛。
    Blade,
    /// 晶石之刃（ROADMAP 19）：晶石碎片鑄造的利刃，攻擊力 +8。
    CrystalBlade,
    /// 珊瑚矛（ROADMAP 19）：深海珍珠打磨的長矛，攻擊力 +12，全遊戲最強武器。
    CoralLance,
    /// 蕈菇杖（ROADMAP 19 續）：蕈菇孢子凝聚的魔杖，攻擊力 +7，森林生態專屬武器。
    MushroomStaff,
    /// 符文刃（ROADMAP 19 續）：古代碎片鑄造的符文刀刃，攻擊力 +10，沙漠生態專屬武器。
    RuneBlade,
    /// 翠幽刃（ROADMAP 21）：翠幽碎片鑄造的異星刀刃，攻擊力 +15，
    /// 翠幽星特有，象徵星際探索的高級戰鬥獎賞。
    JadeBlade,
    /// 赤焰刃（ROADMAP 22）：熔晶碎片鑄造的蒸汽龐克刀刃，攻擊力 +20，
    /// 赤焰星特有，超越翠幽刃（+15），蒸汽文明的最高武裝結晶。
    CrimsonBlade,
    /// 虛空刃（ROADMAP 23）：虛空碎片凝聚的宇宙深淵刀刃，攻擊力 +25，全遊戲最強武器，
    /// 虛空星特有，超越赤焰刃（+20），宇宙邊界能量的終極武裝。
    VoidBlade,
}

/// 持有某類護甲所提供的防禦加成。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArmorKind {
    /// 草原護符：每次受傷減 1 點傷害。
    Meadow,
    /// 晶石護盾：每次受傷減 2 點傷害。
    Crystal,
}

/// 徒手的基礎攻擊力。沒有武器就是這個——刻意等於 `game.rs` 現行寫死的 `PLAYER_ATTACK_POWER`，
/// 讓接線（把常數換成 `weapon_power` 查表）對「沒武器」的玩家行為零變化、純加法。
pub const UNARMED_ATTACK_POWER: u32 = 2;

/// 武器的攻擊力：嚴格高於徒手，讓「合成武器」這條配方鏈真的有感、值得攢素材去合
/// （對齊 PLAN 驗收「合成出的武器真的讓打怪明顯變強」）。
pub const WEAPON_ATTACK_POWER: u32 = 5;

/// 等級攻擊加成：每升兩級 +1 傷害，讓玩家感受到成長但不至於破壞早期平衡。
/// Lv.2 = +1, Lv.4 = +2, Lv.10 = +5, Lv.20 = +10。
pub fn level_attack_bonus(level: u32) -> u32 {
    level / 2
}

impl WeaponKind {
    /// 此武器每下攻擊造成的傷害。
    pub fn attack_power(self) -> u32 {
        match self {
            WeaponKind::Unarmed => UNARMED_ATTACK_POWER,
            WeaponKind::Blade => WEAPON_ATTACK_POWER,
            WeaponKind::CrystalBlade => 8,
            WeaponKind::CoralLance => 12,
            WeaponKind::MushroomStaff => 7,
            WeaponKind::RuneBlade => 10,
            WeaponKind::JadeBlade => 15,
            WeaponKind::CrimsonBlade => 20,
            WeaponKind::VoidBlade => 25,
        }
    }
}

impl ArmorKind {
    /// 此護甲每次受傷減少的傷害點數。
    pub fn defense(self) -> u32 {
        match self {
            ArmorKind::Meadow => 1,
            ArmorKind::Crystal => 2,
        }
    }
}

/// 某個背包物品若是武器，回對應的 `WeaponKind`；不是武器（資源／採集工具）回 `None`。
/// 刻意用窮舉 `match`（不寫 `_` 萬用分支）：日後在 `ItemKind` 加新武器變體時，編譯器會
/// 強制回來補上它對應的武器，避免漏接（比照 `tools::tool_from_item`）。
#[allow(dead_code)]
pub fn weapon_from_item(item: ItemKind) -> Option<WeaponKind> {
    match item {
        ItemKind::Weapon => Some(WeaponKind::Blade),
        ItemKind::CrystalBlade => Some(WeaponKind::CrystalBlade),
        ItemKind::CoralLance => Some(WeaponKind::CoralLance),
        ItemKind::MushroomStaff => Some(WeaponKind::MushroomStaff),
        ItemKind::RuneBlade => Some(WeaponKind::RuneBlade),
        ItemKind::JadeBlade => Some(WeaponKind::JadeBlade),
        // 資源原料、建造材料、採集工具、消耗品、護甲都不是武器。
        ItemKind::Wood
        | ItemKind::Dirt
        | ItemKind::Stone
        | ItemKind::Ether
        | ItemKind::Pickaxe
        | ItemKind::ReinforcedPickaxe
        | ItemKind::CrystalShard
        | ItemKind::MushroomSpore
        | ItemKind::AncientFragment
        | ItemKind::DeepSeaPearl
        | ItemKind::WildflowerSeed
        | ItemKind::HealingPotion
        | ItemKind::CrystalPotion
        | ItemKind::MushroomElixir
        | ItemKind::EtherPill
        | ItemKind::PearlPotion
        | ItemKind::MeadowAmulet
        | ItemKind::CrystalShield
        | ItemKind::StarChart
        | ItemKind::JadeShard
        | ItemKind::JadeElixir
        | ItemKind::LavaCrystal
        | ItemKind::SteamElixir
        | ItemKind::VoidShard
        | ItemKind::VoidElixir => None,
        ItemKind::CrimsonBlade => Some(WeaponKind::CrimsonBlade),
        ItemKind::VoidBlade => Some(WeaponKind::VoidBlade),
    }
}

/// 某個背包物品若是護甲，回對應的 `ArmorKind`；不是護甲回 `None`。
#[allow(dead_code)]
pub fn armor_from_item(item: ItemKind) -> Option<ArmorKind> {
    match item {
        ItemKind::MeadowAmulet => Some(ArmorKind::Meadow),
        ItemKind::CrystalShield => Some(ArmorKind::Crystal),
        ItemKind::Wood
        | ItemKind::Dirt
        | ItemKind::Stone
        | ItemKind::Ether
        | ItemKind::Pickaxe
        | ItemKind::ReinforcedPickaxe
        | ItemKind::Weapon
        | ItemKind::CrystalShard
        | ItemKind::MushroomSpore
        | ItemKind::AncientFragment
        | ItemKind::DeepSeaPearl
        | ItemKind::WildflowerSeed
        | ItemKind::HealingPotion
        | ItemKind::CrystalPotion
        | ItemKind::MushroomElixir
        | ItemKind::EtherPill
        | ItemKind::PearlPotion
        | ItemKind::CrystalBlade
        | ItemKind::CoralLance
        | ItemKind::MushroomStaff
        | ItemKind::RuneBlade
        | ItemKind::StarChart
        | ItemKind::JadeShard
        | ItemKind::JadeElixir
        | ItemKind::JadeBlade
        | ItemKind::LavaCrystal
        | ItemKind::SteamElixir
        | ItemKind::CrimsonBlade
        | ItemKind::VoidShard
        | ItemKind::VoidElixir
        | ItemKind::VoidBlade => None,
    }
}

/// 玩家背包裡最高防禦護甲的減傷值（累加所有護甲的防禦）。
/// 敵人攻擊時先扣去此值再扣血，最低歸零不倒扣。
#[allow(dead_code)]
pub fn armor_defense(inv: &Inventory) -> u32 {
    inv.entries()
        .filter_map(|(item, _)| armor_from_item(item))
        .map(|a| a.defense())
        .sum()
}

/// 玩家背包裡攻擊力最高的武器：挑出持有武器中攻擊力最高者；都沒有就回 `Unarmed`。
/// 戰鬥接線據此決定每下傷害（比照 `tools::best_gather_tool`）。
#[allow(dead_code)]
pub fn best_weapon(inv: &Inventory) -> WeaponKind {
    inv.entries()
        .filter_map(|(item, _)| weapon_from_item(item))
        .max_by_key(|w| w.attack_power())
        .unwrap_or(WeaponKind::Unarmed)
}

/// 玩家每下攻擊的傷害（自動取背包裡最好的武器）。`UNARMED_ATTACK_POWER`＝徒手基礎攻擊力。
/// 戰鬥接線：`game.rs` 把寫死的 `PLAYER_ATTACK_POWER` 換成 `weapon_power(&inv)` 即可
/// （有武器更痛、沒武器與現行一致）。
#[allow(dead_code)]
pub fn weapon_power(inv: &Inventory) -> u32 {
    best_weapon(inv).attack_power()
}

/// 敵人的種類。種類決定生命多寡、掉落什麼、危險度、重生多久。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnemyKind {
    /// 銹蝕巡邏機：故障的舊機械，皮厚、打倒落下礦石（拆解的廢鐵）。
    ScrapDrone,
    /// 迷途乙太靈：野化的乙太生靈，較脆、安撫後散出乙太（種田之外的另一條乙太來源）。
    EtherWisp,
    /// 飄舞精靈（草原）：輕盈的花粉生靈，安撫後散落野花種子——草原生態最脆弱的守護者。
    FlutterSprite,
    /// 蕈菇潛行者（森林）：由孢子凝聚而成的生靈，打倒後釋放蕈菇孢子——森林深處的隱匿者。
    MushroomStalker,
    /// 晶石傀儡（岩地）：被晶洞乙太灌入的礦石傀儡，堅不可摧，碎裂後留下晶石碎片——深層晶洞的看守。
    CrystalGolem,
    /// 符文守衛（沙漠）：古代遺跡自動啟動的機械守衛，被制伏後掉落古代碎片——沙漠遺跡的最後防線。
    RuneGuardian,
    /// 珊瑚蟹（水域）：珊瑚礁叢生的甲殼生物，拆殼後取出深海珍珠——四大生態最稀有的守門者。
    CoralCrab,
    /// 翠幽魅影（翠幽星）：異星乙太凝聚的幽靈生靈，半透明翠色身形，
    /// 擊散後留下翠幽碎片——翠幽星的第一道守護者，強度超越故鄉所有敵人。
    JadeWraith,
    /// 蒸汽構裝（赤焰星）：赤焰星古代文明打造的蒸汽動力機械戰士，全身熔岩裝甲，
    /// 解體後留下熔晶碎片——赤焰星的鋼鐵守護者，強度超越翠幽魅影，全遊戲最難的敵人。
    SteamConstruct,
    /// 虛空幽靈（虛空星）：宇宙深淵能量凝聚而成的黑暗幽靈，半透明紫黑身形，
    /// 碎滅後留下虛空碎片——虛空星的黑暗守護者，強度超越蒸汽構裝，宇宙邊界的終極威脅。
    VoidPhantom,
}

impl EnemyKind {
    /// 此種類滿血時的生命值（要扣到 0 才算打倒）。
    pub fn max_hp(self) -> u32 {
        match self {
            EnemyKind::ScrapDrone => 6,
            EnemyKind::EtherWisp => 4,
            // 草原精靈最脆——草原是新手區、生態門檻最低。
            EnemyKind::FlutterSprite => 3,
            // 森林潛行者中等。
            EnemyKind::MushroomStalker => 5,
            // 晶石傀儡最硬——晶洞是深層探索才碰得到的敵人。
            EnemyKind::CrystalGolem => 8,
            // 符文守衛皮厚，守著高價值古代碎片。
            EnemyKind::RuneGuardian => 7,
            // 珊瑚蟹最難打——守著最稀有的深海珍珠。
            EnemyKind::CoralCrab => 9,
            // 翠幽魅影強——異星守護者，超越故鄉所有敵人。
            EnemyKind::JadeWraith => 11,
            // 蒸汽構裝最強——赤焰星鋼鐵守衛，全遊戲最硬的敵人。
            EnemyKind::SteamConstruct => 15,
            // 虛空幽靈超強——虛空星宇宙深淵守衛，超越蒸汽構裝的終極敵人。
            EnemyKind::VoidPhantom => 20,
        }
    }

    /// 打倒後掉落的戰利品 `(物品, 數量)`。刻意沿用既有 `ItemKind`，不另開新物品
    /// 變體——戰鬥因此自包含、不動 backend 正在接線的 `inventory.rs`，掉落也直接咬進
    /// 採集 / 合成已有的資源經濟。
    ///
    /// 生態域敵人掉落與挖掘相同的特產，提供「戰鬥」與「採礦」兩條平行獲取路線。
    pub fn drop_loot(self) -> (ItemKind, u32) {
        match self {
            // 銹蝕機械拆出廢鐵（礦石）。
            EnemyKind::ScrapDrone => (ItemKind::Stone, 2),
            // 乙太靈散出乙太，但量少、貼合「稀有資源」手感。
            EnemyKind::EtherWisp => (ItemKind::Ether, 1),
            // 草原精靈安撫後散落野花種子（與採野花叢相同）。
            EnemyKind::FlutterSprite => (ItemKind::WildflowerSeed, 1),
            // 森林潛行者碎裂釋放蕈菇孢子（與挖蕈菇叢相同）。
            EnemyKind::MushroomStalker => (ItemKind::MushroomSpore, 1),
            // 晶石傀儡碎成晶石碎片（與挖晶洞相同）。
            EnemyKind::CrystalGolem => (ItemKind::CrystalShard, 1),
            // 符文守衛被制伏後掉落古代碎片（與挖遺跡相同）。
            EnemyKind::RuneGuardian => (ItemKind::AncientFragment, 1),
            // 珊瑚蟹拆殼取出深海珍珠（與挖珊瑚礁相同）。
            EnemyKind::CoralCrab => (ItemKind::DeepSeaPearl, 1),
            // 翠幽魅影擊散後結晶成翠幽碎片（與挖翠玉藤相同，異星能量的精華）。
            EnemyKind::JadeWraith => (ItemKind::JadeShard, 1),
            // 蒸汽構裝解體後留下熔晶碎片（與挖熔岩石相同，赤焰星熔爐的結晶）。
            EnemyKind::SteamConstruct => (ItemKind::LavaCrystal, 1),
            // 虛空幽靈碎滅後凝聚成虛空碎片（與挖虛空晶體相同，宇宙深淵的能量結晶）。
            EnemyKind::VoidPhantom => (ItemKind::VoidShard, 1),
        }
    }

    /// 危險度：這隻敵人每次反擊對玩家造成的傷害。由 `enemy_field::EnemyField::threat_at`
    /// 聚合（範圍內存活敵人的 `threat` 相加），接線層再把那份威脅套進玩家 `Vitals`——
    /// 是「自動打怪」裡敵人會還手、戰鬥才有風險的那一向。
    pub fn threat(self) -> u32 {
        match self {
            EnemyKind::ScrapDrone => 2,
            EnemyKind::EtherWisp => 1,
            // 草原精靈最溫和——療癒向、新手友善。
            EnemyKind::FlutterSprite => 1,
            // 森林潛行者中等威脅。
            EnemyKind::MushroomStalker => 2,
            // 晶石傀儡最危險——深層探索的風險代價。
            EnemyKind::CrystalGolem => 3,
            // 符文守衛高威脅，對應高價值掉落。
            EnemyKind::RuneGuardian => 3,
            // 珊瑚蟹最強——最稀有材料理應最難打。
            EnemyKind::CoralCrab => 4,
            // 翠幽魅影威脅高——異星守護者，對應高戰鬥風險。
            EnemyKind::JadeWraith => 5,
            // 蒸汽構裝威脅最高——赤焰星最強守衛，鋼鐵熔岩裝甲。
            EnemyKind::SteamConstruct => 6,
            // 虛空幽靈威脅超高——宇宙深淵守衛，存在即危險的終極威脅。
            EnemyKind::VoidPhantom => 8,
        }
    }

    /// 打倒這隻敵人獲得的經驗值（難度越高給越多）。
    pub fn exp_reward(self) -> u32 {
        match self {
            EnemyKind::FlutterSprite => 8,
            EnemyKind::EtherWisp => 10,
            EnemyKind::ScrapDrone => 12,
            EnemyKind::MushroomStalker => 15,
            EnemyKind::RuneGuardian => 20,
            EnemyKind::CrystalGolem => 22,
            EnemyKind::CoralCrab => 28,
            // 翠幽魅影給予豐厚 exp——異星強守護者的獎賞。
            EnemyKind::JadeWraith => 35,
            // 蒸汽構裝給予最多 exp——全遊戲最難敵人應有最豐厚獎賞。
            EnemyKind::SteamConstruct => 45,
            // 虛空幽靈給予最多 exp——超越蒸汽構裝的終極敵人，最豐厚的 exp 獎賞。
            EnemyKind::VoidPhantom => 55,
        }
    }

    /// 被打倒後到滿血復活所需的重生秒數。
    pub fn respawn_secs(self) -> f32 {
        match self {
            EnemyKind::ScrapDrone => 50.0,
            EnemyKind::EtherWisp => 35.0,
            EnemyKind::FlutterSprite => 28.0,
            EnemyKind::MushroomStalker => 38.0,
            EnemyKind::CrystalGolem => 55.0,
            EnemyKind::RuneGuardian => 50.0,
            EnemyKind::CoralCrab => 62.0,
            // 翠幽魅影重生時間長——擊散強守護者，讓玩家享受戰果。
            EnemyKind::JadeWraith => 75.0,
            // 蒸汽構裝重生時間最長——全遊戲最難敵人，擊倒後充分獎勵。
            EnemyKind::SteamConstruct => 90.0,
            // 虛空幽靈重生時間最長——宇宙深淵終極守衛，碎滅後充分享受戰果。
            EnemyKind::VoidPhantom => 110.0,
        }
    }
}

/// 世界裡一隻可被打倒、之後會重生的敵人。
///
/// 狀態只有「剩餘生命」與「重生倒數」兩個欄位，階段（存活 / 被打倒）皆由生命推導，
/// 維持單一真實來源——比照 `ResourceNode` 以剩餘耐久推導可採 / 採空。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Enemy {
    /// 敵人種類（決定生命 / 掉落 / 危險度 / 重生時間）。
    kind: EnemyKind,
    /// 剩餘生命（可再承受的傷害量）。歸零＝被打倒。
    remaining_hp: u32,
    /// 重生倒數（秒）。只有被打倒（`remaining_hp == 0`）時才有意義；倒數到 0 滿血復活。
    respawn_timer: f32,
}

impl Enemy {
    /// 生出一隻滿血、可立即被攻擊的新敵人。
    pub fn new(kind: EnemyKind) -> Self {
        Self {
            kind,
            remaining_hp: kind.max_hp(),
            respawn_timer: 0.0,
        }
    }

    /// 敵人種類。
    pub fn kind(&self) -> EnemyKind {
        self.kind
    }

    /// 剩餘生命。
    pub fn remaining_hp(&self) -> u32 {
        self.remaining_hp
    }

    /// 是否已被打倒（需等重生）。接線層的判斷都走對稱的 `is_alive()`，這個版本目前只剩
    /// 測試用到，故單獨保留 `allow(dead_code)`（同 impl 的其餘方法皆有 production 呼叫端）。
    #[allow(dead_code)]
    pub fn is_defeated(&self) -> bool {
        self.remaining_hp == 0
    }

    /// 是否還活著、可被攻擊。
    pub fn is_alive(&self) -> bool {
        self.remaining_hp > 0
    }

    /// 攻擊一下，造成 `power` 點傷害。
    ///
    /// 回傳語意刻意只在**打倒的那一下**給出掉落 `Some((物品, 數量))`，其餘情況回 `None`：
    ///   - 未致命的攻擊：扣血、回 `None`（還沒倒、不掉東西）。
    ///   - 致命的那一下：扣到 0、啟動重生倒數、回 `Some(掉落)`（戰利品只給一次）。
    ///   - 攻擊已被打倒（重生中）的敵人：no-op、回 `None`（不重複掉落）。
    ///   - `power == 0`：no-op、回 `None`（比照 `ResourceNode::gather` 對採空的 no-op）。
    ///
    /// `power` 由接線層決定（將來武器 / 戰鬥技能讓每下傷害更高，比照採集的工具倍率），
    /// 這層只吃整數傷害；血量過量被打（`power` 大於剩餘血）一律夾到 0，不會變負、不會多掉。
    pub fn attack(&mut self, power: u32) -> Option<(ItemKind, u32)> {
        if power == 0 || self.remaining_hp == 0 {
            return None;
        }
        // 飽和扣血：傷害超過剩餘血時夾到 0，不會 underflow。
        self.remaining_hp = self.remaining_hp.saturating_sub(power);
        if self.remaining_hp == 0 {
            // 被打倒：開始倒數重生，並一次性掉落戰利品。
            self.respawn_timer = self.kind.respawn_secs();
            Some(self.kind.drop_loot())
        } else {
            None
        }
    }

    /// 推進 `dt` 秒。只有被打倒的敵人在倒數；倒數到 0 滿血復活、再次可被攻擊。
    /// 還活著或非正 `dt` 皆為 no-op（比照 `ResourceNode::tick` 擋非正 dt）。
    pub fn tick(&mut self, dt: f32) {
        if self.remaining_hp > 0 || dt <= 0.0 {
            return;
        }
        self.respawn_timer -= dt;
        if self.respawn_timer <= 0.0 {
            self.remaining_hp = self.kind.max_hp();
            self.respawn_timer = 0.0;
        }
    }

    /// 從存檔載入的值是否「健全」：生命不超過該種類上限、重生倒數有限且非負。
    /// 這是與調校常數無關的最小不變式——正常流程（`new` 滿血、`attack` 只遞減、
    /// `tick` 倒數一律夾在 `>= 0`）絕不會產生界外生命或 `NaN`/`Inf`/負倒數，所以這些
    /// 只會來自壞檔或被竄改的存檔。`remaining_hp` 是 `u32`、型別本身就擋掉 `NaN`/負值，
    /// 故只需驗上界。延續 `gather::is_loadable` / `field::from_tiles` 的載入時驗證脈絡，
    /// 已由 `enemy_field::from_saved` 逐隻呼叫把關（敵人狀態接 0-E 持久化後即沿用同一道防線）。
    pub fn is_loadable(&self) -> bool {
        self.respawn_timer.is_finite()
            && self.respawn_timer >= 0.0
            && self.remaining_hp <= self.kind.max_hp()
    }

    /// 測試用：直接組出指定狀態（含壞值）的敵人，驗證載入防線。
    #[cfg(test)]
    pub fn from_raw(kind: EnemyKind, remaining_hp: u32, respawn_timer: f32) -> Self {
        Self {
            kind,
            remaining_hp,
            respawn_timer,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KINDS: [EnemyKind; 10] = [
        EnemyKind::ScrapDrone,
        EnemyKind::EtherWisp,
        EnemyKind::FlutterSprite,
        EnemyKind::MushroomStalker,
        EnemyKind::CrystalGolem,
        EnemyKind::RuneGuardian,
        EnemyKind::CoralCrab,
        EnemyKind::JadeWraith,
        EnemyKind::SteamConstruct,
        EnemyKind::VoidPhantom,
    ];

    // ───── 武器查表（鏡像 tools.rs 的採集倍率測試）─────

    // 編譯期不變式：武器一定比徒手痛，否則「合成武器讓打怪變強」這條閉環不成立。
    const _: () = assert!(WEAPON_ATTACK_POWER > UNARMED_ATTACK_POWER);

    #[test]
    fn weapon_hits_harder_than_fist() {
        assert_eq!(WeaponKind::Blade.attack_power(), WEAPON_ATTACK_POWER);
        assert_eq!(WeaponKind::Unarmed.attack_power(), UNARMED_ATTACK_POWER);
    }

    #[test]
    fn only_weapons_map_from_items() {
        assert_eq!(weapon_from_item(ItemKind::Weapon), Some(WeaponKind::Blade));
        // 資源與採集工具都不是武器。
        assert_eq!(weapon_from_item(ItemKind::Wood), None);
        assert_eq!(weapon_from_item(ItemKind::Stone), None);
        assert_eq!(weapon_from_item(ItemKind::Ether), None);
        assert_eq!(weapon_from_item(ItemKind::Pickaxe), None);
        assert_eq!(weapon_from_item(ItemKind::ReinforcedPickaxe), None);
    }

    #[test]
    fn empty_inventory_fights_unarmed() {
        let inv = Inventory::new();
        assert_eq!(best_weapon(&inv), WeaponKind::Unarmed);
        assert_eq!(weapon_power(&inv), UNARMED_ATTACK_POWER);
    }

    #[test]
    fn weapon_in_inventory_raises_attack_power() {
        let mut inv = Inventory::new();
        inv.add(ItemKind::Weapon, 1);
        assert_eq!(best_weapon(&inv), WeaponKind::Blade);
        assert_eq!(weapon_power(&inv), WEAPON_ATTACK_POWER);
        // 有武器嚴格比徒手痛——這是 MVP 驗收「武器讓打怪明顯變強」的數值面。
        assert!(weapon_power(&inv) > UNARMED_ATTACK_POWER);
    }

    #[test]
    fn carrying_only_a_pickaxe_still_fights_unarmed() {
        // 採集工具不是武器：只揹鎬子打怪仍是徒手攻擊力（守工具／武器兩條查表互不串味）。
        let mut inv = Inventory::new();
        inv.add(ItemKind::Pickaxe, 1);
        assert_eq!(weapon_power(&inv), UNARMED_ATTACK_POWER);
    }

    #[test]
    fn weapon_actually_downs_an_enemy_faster() {
        // 端到端把查表接上 `Enemy::attack`：同一隻敵人，持武器所需的攻擊次數不多於徒手，
        // 鎖住「武器→每下更痛→更快打趴」這條因果（接線輪 game.rs 餵 `weapon_power` 即得此效果）。
        fn hits_to_down(power: u32, kind: EnemyKind) -> u32 {
            let mut e = Enemy::new(kind);
            let mut hits = 0;
            while e.is_alive() {
                e.attack(power);
                hits += 1;
            }
            hits
        }
        let mut armed = Inventory::new();
        armed.add(ItemKind::Weapon, 1);
        let unarmed = Inventory::new();
        for kind in KINDS {
            let armed_hits = hits_to_down(weapon_power(&armed), kind);
            let fist_hits = hits_to_down(weapon_power(&unarmed), kind);
            assert!(
                armed_hits <= fist_hits,
                "{:?}：持武器 {} 下、徒手 {} 下——武器不該更慢",
                kind,
                armed_hits,
                fist_hits
            );
        }
    }

    #[test]
    fn new_enemy_is_full_hp_and_alive() {
        for kind in KINDS {
            let e = Enemy::new(kind);
            assert_eq!(e.remaining_hp(), kind.max_hp());
            assert!(e.is_alive());
            assert!(!e.is_defeated());
        }
    }

    #[test]
    fn non_lethal_attack_damages_but_drops_nothing() {
        let mut e = Enemy::new(EnemyKind::ScrapDrone);
        // 一下打不死（max_hp 6 > 1），扣血但不掉落。
        assert_eq!(e.attack(1), None);
        assert_eq!(e.remaining_hp(), EnemyKind::ScrapDrone.max_hp() - 1);
        assert!(e.is_alive());
    }

    #[test]
    fn killing_blow_drops_loot_and_starts_respawn() {
        let mut e = Enemy::new(EnemyKind::EtherWisp);
        // 一口氣打掉所有血：致命那下回傳掉落。
        let loot = e.attack(EnemyKind::EtherWisp.max_hp());
        assert_eq!(loot, Some(EnemyKind::EtherWisp.drop_loot()));
        assert!(e.is_defeated());
        assert!(!e.is_alive());
    }

    #[test]
    fn loot_drops_exactly_once_on_the_lethal_blow() {
        let mut e = Enemy::new(EnemyKind::ScrapDrone);
        let max = EnemyKind::ScrapDrone.max_hp();
        // 逐下打：只有最後一下（扣到 0）掉落，其餘都 None。
        let mut drops = Vec::new();
        for _ in 0..max {
            if let Some(loot) = e.attack(1) {
                drops.push(loot);
            }
        }
        assert_eq!(drops, vec![EnemyKind::ScrapDrone.drop_loot()]);
        assert!(e.is_defeated());
    }

    #[test]
    fn attacking_a_defeated_enemy_is_noop_and_drops_nothing() {
        let mut e = Enemy::new(EnemyKind::EtherWisp);
        e.attack(EnemyKind::EtherWisp.max_hp());
        let defeated = e.clone();
        // 重生中再打：不掉落、不改變狀態（不重複掉戰利品）。
        assert_eq!(e.attack(10), None);
        assert_eq!(e, defeated);
    }

    #[test]
    fn zero_power_attack_is_noop() {
        let mut e = Enemy::new(EnemyKind::ScrapDrone);
        let before = e.clone();
        assert_eq!(e.attack(0), None);
        assert_eq!(e, before);
    }

    #[test]
    fn overkill_clamps_to_zero_and_drops_once() {
        let mut e = Enemy::new(EnemyKind::EtherWisp);
        // 傷害遠超血量：夾到 0、只掉一次、不 underflow 變負。
        let loot = e.attack(EnemyKind::EtherWisp.max_hp() + 999);
        assert_eq!(loot, Some(EnemyKind::EtherWisp.drop_loot()));
        assert_eq!(e.remaining_hp(), 0);
        assert!(e.is_defeated());
    }

    #[test]
    fn defeated_enemy_respawns_after_timer() {
        let mut e = Enemy::new(EnemyKind::ScrapDrone);
        e.attack(EnemyKind::ScrapDrone.max_hp());
        assert!(e.is_defeated());
        // 還沒到重生時間，仍被打倒。
        e.tick(EnemyKind::ScrapDrone.respawn_secs() - 1.0);
        assert!(e.is_defeated());
        // 補足剩餘時間，滿血復活、再次可打。
        e.tick(1.0);
        assert!(e.is_alive());
        assert_eq!(e.remaining_hp(), EnemyKind::ScrapDrone.max_hp());
    }

    #[test]
    fn tick_on_living_enemy_is_noop() {
        let mut e = Enemy::new(EnemyKind::ScrapDrone);
        e.attack(2); // 受了點傷但還活著
        let before = e.clone();
        e.tick(100.0);
        assert_eq!(e, before);
    }

    #[test]
    fn zero_or_negative_dt_is_noop() {
        let mut e = Enemy::new(EnemyKind::EtherWisp);
        e.attack(EnemyKind::EtherWisp.max_hp());
        let defeated = e.clone();
        e.tick(0.0);
        assert_eq!(e, defeated);
        e.tick(-5.0);
        assert_eq!(e, defeated);
    }

    #[test]
    fn full_cycle_kill_respawn_kill_again() {
        let mut e = Enemy::new(EnemyKind::EtherWisp);
        // 打倒。
        assert!(e.attack(EnemyKind::EtherWisp.max_hp()).is_some());
        assert!(e.is_defeated());
        // 一次大步推過重生時間，滿血復活。
        e.tick(EnemyKind::EtherWisp.respawn_secs());
        assert!(e.is_alive());
        // 復活後又能再打倒一次、再掉一次。
        assert_eq!(
            e.attack(EnemyKind::EtherWisp.max_hp()),
            Some(EnemyKind::EtherWisp.drop_loot())
        );
    }

    #[test]
    fn each_kind_drops_an_existing_resource() {
        // 掉落沿用既有採集 / 經濟資源，戰鬥自包含、不另開物品變體。
        assert_eq!(EnemyKind::ScrapDrone.drop_loot(), (ItemKind::Stone, 2));
        assert_eq!(EnemyKind::EtherWisp.drop_loot(), (ItemKind::Ether, 1));
        // 生態域敵人——各掉對應生態特產（與挖掘路線相同素材，戰鬥是另一條供給管道）。
        assert_eq!(EnemyKind::FlutterSprite.drop_loot(), (ItemKind::WildflowerSeed, 1));
        assert_eq!(EnemyKind::MushroomStalker.drop_loot(), (ItemKind::MushroomSpore, 1));
        assert_eq!(EnemyKind::CrystalGolem.drop_loot(), (ItemKind::CrystalShard, 1));
        assert_eq!(EnemyKind::RuneGuardian.drop_loot(), (ItemKind::AncientFragment, 1));
        assert_eq!(EnemyKind::CoralCrab.drop_loot(), (ItemKind::DeepSeaPearl, 1));
        assert_eq!(EnemyKind::JadeWraith.drop_loot(), (ItemKind::JadeShard, 1));
    }

    #[test]
    fn is_loadable_accepts_normal_and_rejects_corrupt() {
        // 正常流程產出的狀態都該可載入。
        assert!(Enemy::new(EnemyKind::ScrapDrone).is_loadable());
        let mut e = Enemy::new(EnemyKind::EtherWisp);
        e.attack(EnemyKind::EtherWisp.max_hp());
        assert!(e.is_loadable()); // 被打倒且帶重生倒數，仍健全
        // 壞值：生命超過上限、NaN / Inf / 負重生倒數。
        assert!(!Enemy::from_raw(EnemyKind::ScrapDrone, 99, 0.0).is_loadable());
        assert!(!Enemy::from_raw(EnemyKind::EtherWisp, 0, f32::NAN).is_loadable());
        assert!(!Enemy::from_raw(EnemyKind::EtherWisp, 0, f32::INFINITY).is_loadable());
        assert!(!Enemy::from_raw(EnemyKind::EtherWisp, 0, -1.0).is_loadable());
    }

    #[test]
    fn serde_round_trip_preserves_state() {
        let mut e = Enemy::new(EnemyKind::ScrapDrone);
        e.attack(2); // 打了一下，留個半血狀態
        let json = serde_json::to_string(&e).unwrap();
        let back: Enemy = serde_json::from_str(&json).unwrap();
        assert_eq!(e, back);
    }

    // ── Phase 1 戰鬥掉落咬進共同資源經濟的組合測試 ─────────────────────────
    // 本模組刻意「掉落沿用既有 `ItemKind`、不另開物品變體」，好讓戰鬥自包含又直接咬進
    // 採集 / 合成已有的資源經濟（見模組頂註）。但「打怪掉的礦石／乙太，真的疊進採集填的
    // 同一個背包格」這條跨 combat→inventory→gather 的接縫此前沒有測試保證。這個組合測試
    // 走一遍：先採一點資源進背包，再打倒敵人、把掉落 `add` 進**同一個** `Inventory`，
    // 驗證兩條來源落在同一個 `ItemKind` 槽位、數量相加——鎖住「戰鬥不是孤島，而是同一套
    // 經濟的另一條供給」這個設計契約，任一邊的物品型別漂移都會在此斷掉。

    use crate::gather::{NodeKind, ResourceNode};
    use crate::inventory::Inventory;

    /// 把一隻敵人逐下打到倒，回傳致命那下的掉落（必有）。
    fn defeat(kind: EnemyKind) -> (ItemKind, u32) {
        let mut e = Enemy::new(kind);
        let mut loot = None;
        while e.is_alive() {
            if let Some(dropped) = e.attack(2) {
                loot = Some(dropped);
            }
        }
        loot.expect("致命那下一定掉落")
    }

    #[test]
    fn combat_loot_stacks_into_the_same_gathered_inventory() {
        let mut inv = Inventory::new();

        // 預先在背包放入一些礦石與乙太。
        inv.add(ItemKind::Stone, 1);
        inv.add(ItemKind::Ether, 2);
        assert_eq!(inv.count(ItemKind::Stone), 1);
        assert_eq!(inv.count(ItemKind::Ether), 2);

        // 打倒銹蝕巡邏機掉 (Stone, 2)，疊進採集得來的同一個礦石槽。
        let (drone_item, drone_qty) = defeat(EnemyKind::ScrapDrone);
        assert_eq!((drone_item, drone_qty), (ItemKind::Stone, 2));
        inv.add(drone_item, drone_qty);
        assert_eq!(inv.count(ItemKind::Stone), 1 + 2);

        // 打倒迷途乙太靈掉 (Ether, 1)，疊進同一個乙太槽。
        let (wisp_item, wisp_qty) = defeat(EnemyKind::EtherWisp);
        assert_eq!((wisp_item, wisp_qty), (ItemKind::Ether, 1));
        inv.add(wisp_item, wisp_qty);
        assert_eq!(inv.count(ItemKind::Ether), 2 + 1);

        // 掉落全是既有採集 / 經濟資源，戰鬥沒有自立一套平行物品。
        assert!(matches!(drone_item, ItemKind::Stone));
        assert!(matches!(wisp_item, ItemKind::Ether));
    }

    #[test]
    fn every_enemy_drop_is_a_usable_economic_resource() {
        // 跨模組不變式（1-F 戰鬥 × 1-B 物品 × 1-C 合成 × 0-G-O2 經濟），與 `crafting` 的
        // `every_recipe_input_is_obtainable`、`tools` 的 `every_tool_item_is_obtainable` 同屬
        // 「無死路」守則家族，但方向是**掉落側**：每一種敵人打倒後掉的東西，玩家都要有地方
        // 用掉，否則就是「打了半天掉一堆沒去處的垃圾」的死掉落。
        //
        // 本模組頂註白紙黑字：掉落「沿用既有 `ItemKind`、不另開物品變體」「直接咬進採集 /
        // 合成已有的資源經濟」——掉落該是能再投入經濟的**原料或乙太**。此前
        // `each_kind_drops_an_existing_resource` 只**寫死**現有兩種掉落的具體值，察覺不到日後
        // 新增的第三種敵人掉了個沒去處的東西。PLAN 自己就指向再加敵人 / 掉落，屆時這正是會踩
        // 的坑。趁敵人種類還少，把「凡掉落必有去處」鎖成遍歷所有 `EnemyKind` 的組合測試：日後
        // 加敵人時若讓它掉一個沒人消耗的死物品，當場紅燈。
        //
        // 「有去處」＝該物品被某條配方當素材消耗（合成原料），**或**它是乙太（`economy` 的擴地
        // 消耗點吃掉它）。日後若有意讓敵人掉「成品工具」之類非原料的東西，會在此紅燈，逼人確認
        // 是有意設計再更新本不變式（比照工具 / 配方家族遇到漂移時的做法）。
        use crate::crafting::RECIPES;

        // 窮舉守衛：新增 EnemyKind 變體卻忘了加進 KINDS 時，此 match 不窮舉、編譯失敗，
        // 逼人回來把新種類納入本遍歷（比照 crafting 對 NodeKind 的窮舉守衛）。
        for kind in KINDS {
            match kind {
                EnemyKind::ScrapDrone
                | EnemyKind::EtherWisp
                | EnemyKind::FlutterSprite
                | EnemyKind::MushroomStalker
                | EnemyKind::CrystalGolem
                | EnemyKind::RuneGuardian
                | EnemyKind::CoralCrab
                | EnemyKind::JadeWraith
                | EnemyKind::SteamConstruct
                | EnemyKind::VoidPhantom => {}
            }
        }

        for kind in KINDS {
            let (item, qty) = kind.drop_loot();
            assert!(qty > 0, "敵人 {kind:?} 的掉落數量應 > 0");
            let is_crafting_input = RECIPES
                .iter()
                .any(|r| r.inputs.iter().any(|&(i, _)| i == item));
            let is_currency = item == ItemKind::Ether;
            assert!(
                is_crafting_input || is_currency,
                "敵人 {kind:?} 掉落 {item:?}，但它既不是任何配方的素材、也不是乙太貨幣——\
                 玩家打倒它拿到的是沒有去處的死掉落；請讓掉落沿用能再投入合成 / 經濟的原料或\
                 乙太，或若有意讓敵人掉成品，再更新本不變式"
            );
        }
    }

    #[test]
    fn enemy_table_is_well_formed() {
        // 敵人表健全性（與戰鬥平衡的調校數值無關的最小不變式，防日後加敵人時打錯），
        // 對照 `crafting::recipe_table_is_well_formed` 之於配方表。此前每個 `EnemyKind`
        // 常數（`max_hp` / `respawn_secs` / `threat`）都只被**寫死特定種類**的測試零星
        // 覆蓋，沒有一條遍歷整張敵人表、守住「每一種敵人的這些常數都落在合法範圍」的不變式。
        // PLAN 自己就指向再加敵人，屆時這正是會踩的坑：
        //   - `max_hp == 0` 的敵人一出生 `remaining_hp == 0` 即被判為「已被打倒」，`attack`
        //     的 `remaining_hp == 0` 早退讓它**永遠打不倒、永遠不掉落**，`tick` 又把它「復活」
        //     回 0 血——一隻玩家永遠碰不到的鬼敵人。
        //   - `respawn_secs` 非有限（NaN / Inf）或 <= 0：被打倒後 `respawn_timer` 被種成壞值，
        //     `Inf` 永遠倒數不完（再也不重生）、`NaN` 毒化比較、<= 0 則下一 tick 即「瞬間重生」，
        //     全都壞掉重生節奏（模組頂註白紙黑字「打完一處得換地方或等它重生，給世界一點節奏」）。
        //   - `threat == 0`：模組頂註明言戰鬥要「雙向有來有回」、`threat` 是敵人反擊的傷害；
        //     反擊為 0 等於零風險白嫖、破壞戰鬥的風險／回報。若日後有意設計「無害的敵人」，
        //     在此紅燈逼人確認是有意設計再更新本不變式（比照工具／配方／掉落家族的逃生口）。

        // 窮舉守衛：新增 EnemyKind 變體卻忘了加進 KINDS 時，此 match 不窮舉、編譯失敗，
        // 逼人回來把新種類納入本遍歷（比照 `every_enemy_drop_is_a_usable_economic_resource`）。
        for kind in KINDS {
            match kind {
                EnemyKind::ScrapDrone
                | EnemyKind::EtherWisp
                | EnemyKind::FlutterSprite
                | EnemyKind::MushroomStalker
                | EnemyKind::CrystalGolem
                | EnemyKind::RuneGuardian
                | EnemyKind::CoralCrab
                | EnemyKind::JadeWraith
                | EnemyKind::SteamConstruct
                | EnemyKind::VoidPhantom => {}
            }
        }

        for kind in KINDS {
            // 生命為正：否則一出生即被判定打倒、永遠無法被攻擊／掉落。
            assert!(
                kind.max_hp() > 0,
                "敵人 {kind:?} 的 max_hp 應 > 0，否則一出生即被判定打倒、玩家永遠打不倒它"
            );
            // 重生秒數有限且為正：否則被打倒後重生節奏壞掉（永不重生／瞬間重生／NaN 毒化）。
            let respawn = kind.respawn_secs();
            assert!(
                respawn.is_finite() && respawn > 0.0,
                "敵人 {kind:?} 的 respawn_secs（{respawn}）應為有限正數，否則重生節奏壞掉"
            );
            // 反擊傷害為正：維持模組頂註承諾的「雙向有來有回」戰鬥風險。
            assert!(
                kind.threat() > 0,
                "敵人 {kind:?} 的 threat 應 > 0，否則戰鬥零風險、可無傷白嫖；若有意設計無害\
                 敵人，再更新本不變式"
            );
            // 擊殺 exp 獎勵為正：打倒敵人應有進度感，0 獎勵破壞升級閉環。
            assert!(
                kind.exp_reward() > 0,
                "敵人 {kind:?} 的 exp_reward 應 > 0，擊殺應推進升級進度"
            );
        }
    }

    #[test]
    fn harder_enemies_give_more_exp() {
        // 難度較高的敵人（threat 較大）應給較多 exp，驗查最大與最小的相對關係。
        let max_exp = KINDS.iter().map(|k| k.exp_reward()).max().unwrap();
        let min_exp = KINDS.iter().map(|k| k.exp_reward()).min().unwrap();
        assert!(max_exp > min_exp, "不同難度的敵人 exp 獎勵應有差異，給玩家挑戰動機");
    }
}
