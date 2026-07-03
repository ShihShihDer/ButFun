//! 乙太方界·居民遠行探野 v1（PLAN_ETHERVOX item 7「居民散佈世界各處住」第一刀，ROADMAP 756）。
//!
//! **設計依據**：路線圖 item 7 是「居民散佈世界各處住（別擠主城，麥塊式散居）」——一個至今
//! 完全沒動過的**空間／聚落維度**。目前四位居民各有一個固定家域中心（露娜在世界原點、其餘三位
//! 在南／西／東 75 格），日常只在自家域半徑（[`crate::voxel_residents::HOME_RADIUS`]=20 格）內
//! 閒晃——世界的荒野遠處始終空無一人，玩家永遠只在主城一帶撞見居民。這一刀把「散佈各處」的第一步
//! 做出來：**讓天生愛四處走的 Wanderer 人格居民（奧瑞，東方「山林、遠足感」），偶爾放下手邊的事、
//! 獨自遠行到遠離主城的世界邊陲住上一陣子，再走回家。**
//!
//! 玩家第一次會在遠離主城的荒野撞見居民的身影——「奧瑞獨自往東方的邊陲遠行了」浮上動態牆，過一
//! 陣子再讀到「奧瑞遠行歸來」。世界不再只圍著主城打轉，居民的足跡第一次真的散進了荒野。這是把
//! 居民從「主城的固定住戶」推向「散佈世界各處的居民」的地基——日後可在此之上長出「在遠方紮營／
//! 蓋第二個家／真的搬過去住」（item 7 後續）。
//!
//! **遠行 v3（ROADMAP 758，本刀「在邊陲紮營、雛形第二個家」）**：v1 讓居民走進荒野再走回、v2 讓牠
//! 抵達時升起營火路標——但每趟遠行的落點都由當下身位抖動而定，居民每次都跑到荒野裡**不同的**一點，
//! 留下的是散落各處的一次性營火，稱不上「住」。item 7 講的是「散佈世界各處**住**」——住意味著有一個
//! **固定會回去的據點**，不是無盡漂泊。這一刀把「漂泊」收斂成「安頓」：①遠行落點改由**家的方位**
//! 確定性算出（見 `pick_frontier` 呼叫端改餵家座標 seq）——同一位居民每趟遠行都回到**同一處**邊陲
//! 營地，玩家會一再在那個熟悉的荒野角落撞見牠，那裡漸漸長成「奧瑞的邊陲營地」；②抵達時除了營火，
//! 居民還在營火旁**搭起一座紮營小棚**（背牆＋頂＋一張床的簡易 lean-to，走既有 world delta 持久化、
//! 冪等只搭一次）——荒野裡第一次有一張床、一個過夜的地方，是「第二個家」最初的雛形。世界因居民散居
//! 而在主城外從「一個記號」長成「一處可歇腳過夜的據點」。
//!
//! **與既有『離家』行為的定位區隔**：
//! - 探訪鄰居（671，`voxel_visit`）／登門串門子（751，`voxel_neighborvisit`）走向的是**另一位
//!   居民的家域**（主城範圍內、社交目的）；本模組走向的是**無人的荒野邊陲**（空間探索、不為找人）。
//! - 重返心中的牌子（743，`voxel_readsign`）走向的是一塊**玩家立過、讓牠印象深刻的告示牌**（記憶
//!   驅動、有具體地標）；本模組走向的是**由方位算出的遠方一點**（天性驅動、不倚賴任何既有地標）。
//! - 孤獨尋伴（678）／久別奔迎（747）走向的是**玩家**；本模組是獨自遠行，不朝任何人。
//!
//! **純邏輯層**：本檔全是零 IO、零鎖、零 LLM、零 async 的確定性純函式；朝邊陲走的狀態機、逗留計時、
//! 記憶昇華與 Feed 廣播都在 `voxel_ws.rs`（沿用探訪／朝聖既有的短鎖手法與 wander 中心覆寫慣例）。

use crate::resident_npc::ResidentPersona;
use crate::voxel::{biome_name, Block, VoxelBiome};

/// 遠行動機——由人格決定「誰會散往荒野、又為了什麼」（散居 v6，ROADMAP 762）。
///
/// 散居 v1~v5（756~761）只有奧瑞（Wanderer）憑漂泊天性遠行，世界的荒野裡始終只有他一個人的
/// 足跡與一處據點——稱不上「散佈**各處**住」。本切片把「誰會遠行」從一人擴成兩人，且**各有其
/// 真實動機**，讓第二處邊陲家園是角色天性長出來的、不是把奧瑞的行為複製到另一隻 NPC 身上：
/// - [`ExpeditionMotive::Roam`]（Wanderer·奧瑞）：天生腳癢、愛四處看看的漂泊天性（既有）。
/// - [`ExpeditionMotive::SeekLand`]（FarmWorker·諾娃）：農人為主城外尋覓一片沃野而遠行——
///   同一套「啟程→紮營→過夜→認地→帶風物回家」的行為，但**出發的理由**不同，世界因此有了
///   兩位動機各異的散居者。
///
/// 動機只在**啟程**（`embark_bubble`／`embark_feed_line`）分岔——那是「為什麼走」最該現形的地方；
/// 抵達／歸來／營火／小棚等文字談的是「那個**地方**」（已隨生物群系而異），維持通用不隨動機再分岔，
/// 把新增的文字面收斂在最有感、最不喧賓奪主的一處。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpeditionMotive {
    /// 漂泊天性：想四處走走看看（Wanderer·奧瑞）。
    Roam,
    /// 尋覓沃野：農人想在主城外找一片能耕作的好地方（FarmWorker·諾娃）。
    SeekLand,
}

/// 由人格算出遠行動機——`Some` 者會散往荒野，`None` 者（MarketBrowser·露娜／TownSquare·賽勒）
/// 留守主城不遠行。**散居是漸進的、不是全員一次散開**：先讓天性最相容的兩位（漂泊的奧瑞、
/// 尋地的諾娃）踏出去，市集人／廣場人留在主城，世界的散居分佈才自然、有層次。
/// 純函式、確定性、無 IO——「誰散居、為何散居」這個設計決策集中於此、可單元測試釘住。
pub fn expedition_motive(persona: ResidentPersona) -> Option<ExpeditionMotive> {
    match persona {
        ResidentPersona::Wanderer => Some(ExpeditionMotive::Roam),
        ResidentPersona::FarmWorker => Some(ExpeditionMotive::SeekLand),
        ResidentPersona::MarketBrowser | ResidentPersona::TownSquare => None,
    }
}

/// 遠行落點距家域中心的基準距離（世界座標）：家已在主城外圍（±75），再往外推這麼遠 → 落在
/// 離主城逾百格的荒野，玩家平常閒晃絕不會誤入，撞見居民在那才顯得「牠真的走遠了」。
pub const EXPEDITION_DIST: f32 = 95.0;

/// 抵達邊陲的判定距離（世界座標，平方比較用）：落在此半徑內即視為「到了遠方」。比探訪抵達距離
/// 寬鬆些——邊陲是一片開闊荒野、不是一個精確地標，走到大概位置就算到了。
pub const EXPEDITION_ARRIVE_DIST: f32 = 4.0;

/// 抵達邊陲後在遠方逗留（探索、四處走走）的秒數：夠久讓玩家有機會撞見牠獨自在荒野的身影，
/// 又不會久到牠整天不回家。逗留期間以邊陲為閒晃中心自由走動（見 `voxel_ws` wander 中心覆寫）。
pub const EXPEDITION_STAY_SECS: f32 = 120.0;

/// 去程逾時秒數：啟程時設此值、未抵達時每 tick 遞減；走太久（地形擋路、繞遠路等）還沒到就放棄
/// 這趟遠行、交回一般 wander 帶牠回家，不無限走。路遠故給得寬裕。
pub const EXPEDITION_TIMEOUT: f32 = 150.0;

/// 遠行冷卻秒數：一趟遠行（歸來或放棄）後至少隔這麼久才可能再啟程——遠行是稀少而有份量的事件，
/// 不洗版；各居民初始冷卻另行錯開。
pub const EXPEDITION_COOLDOWN: f32 = 900.0;

/// 逗留期間的閒晃半徑（世界座標）：比家域半徑略小，讓牠在邊陲一小片範圍內自然走動、不再散得更開。
pub const EXPEDITION_WANDER_RADIUS: f32 = 8.0;

/// 每次「是否啟程遠行」判定過機率門檻的機率（低頻節流）：實際還要層層過閘（Wanderer 人格、閒置
/// 自由、白天、冷卻到期），故有感頻率遠低於此。稀少才顯得是一趟鄭重的遠行。
pub const EMBARK_CHANCE: f32 = 0.02;

/// 遠行記憶掛的哨兵「玩家名」：與 `voxel_bedtime` / `voxel_readsign` 同慣例，用一個絕不與真實玩家
/// 撞名的內部鍵，讓「我到過遠方」這類記憶不汙染任何玩家的好感度／回想。
pub const EXPEDITION_MEMORY_PLAYER: &str = "__voxel_expedition__";

/// 邊陲過夜 v4（ROADMAP 759）：判定「已走到營地那張床邊」的水平半徑（世界座標）。與家用睡眠的
/// [`crate::voxel_sleep::SLEEP_NEAR_RADIUS`] 略窄——邊陲小棚只有一張床，走到它跟前才躺下才自然。
pub const OUTPOST_BED_NEAR: f32 = 3.0;

/// 泡泡台詞字元上限（比照其他泡泡台詞）。
pub const SAY_MAX_CHARS: usize = 40;

/// 是否啟程遠行：能遠行的人格（`can_embark`＝[`expedition_motive`] 回 `Some`，散居 v6 起為
/// Wanderer·奧瑞 或 FarmWorker·諾娃）+ 閒置自由（沒在忙別的意圖）+ 白天 + 冷卻到期 + 此刻沒在
/// 說話 + 過機率門檻。`roll` 由呼叫端以 `rand::random::<f32>()` 餵入（與本專案其他機率骰同慣例）。
/// 純函式、確定性、無 IO。
pub fn should_embark(
    can_embark: bool,
    idle_free: bool,
    is_day: bool,
    cooldown: f32,
    say_empty: bool,
    roll: f32,
) -> bool {
    can_embark && idle_free && is_day && cooldown <= 0.0 && say_empty && roll < EMBARK_CHANCE
}

/// 由方位向量算出玩家看得懂的方位名（繁中）。本世界座標約定：+x = 東、+z = 南
/// （見 `voxel_residents::resident_home_base`：南方在 (0,75)、東方在 (75,0)）。
pub fn bearing_label(dx: f32, dz: f32) -> &'static str {
    if dx.abs() >= dz.abs() {
        if dx >= 0.0 {
            "東方"
        } else {
            "西方"
        }
    } else if dz >= 0.0 {
        "南方"
    } else {
        "北方"
    }
}

/// 算出這趟遠行的落點與方位：由主城（世界原點）朝這位居民家的方向再往外推 [`EXPEDITION_DIST`]
/// ——落在離主城更遠的荒野，實現「散往世界各處、別擠主城」。依 `seq` 給落點一點角度抖動與距離
/// 變化，讓每趟遠行不落在同一格、不機械。家恰在原點（罕見）時退回用 `seq` 選一個基本方位。
/// 回傳 (落點 x, 落點 z, 方位名)。純函式、確定性。
pub fn pick_frontier(home_x: f32, home_z: f32, seq: usize) -> (f32, f32, &'static str) {
    let (mut ux, mut uz) = (home_x, home_z);
    let len = (ux * ux + uz * uz).sqrt();
    if len < 1.0 {
        // 家就在世界原點：沒有「往外」的方向可依，用 seq 選一個基本方位當作出發朝向。
        let dirs = [(1.0_f32, 0.0_f32), (0.0, 1.0), (-1.0, 0.0), (0.0, -1.0)];
        let d = dirs[seq % 4];
        ux = d.0;
        uz = d.1;
    } else {
        ux /= len;
        uz /= len;
    }
    // 角度抖動（約 ±0.45 rad ≈ ±26°）：讓遠行落點依 seq 散開，不每趟都同一點。
    let jitter = ((seq % 7) as f32 - 3.0) * 0.15;
    let (s, c) = jitter.sin_cos();
    let rx = ux * c - uz * s;
    let rz = ux * s + uz * c;
    // 距離也依 seq 微調（0~32 格），讓深淺不一。
    let dist = EXPEDITION_DIST + (seq % 5) as f32 * 8.0;
    let fx = home_x + rx * dist;
    let fz = home_z + rz * dist;
    (fx, fz, bearing_label(rx, rz))
}

/// 擷取字串前 [`SAY_MAX_CHARS`] 個字元（安全截斷、不破多位元組）。
fn cap(s: String) -> String {
    s.chars().take(SAY_MAX_CHARS).collect()
}

/// 啟程遠行時冒的泡泡（依**動機**分岔——漂泊的奧瑞 vs 尋地的諾娃出發理由不同；再依 `pick`
/// 輪替不機械）。動機讓兩位散居者的踏出各有其口吻，世界不再只有一種「遠行」。
pub fn embark_bubble(motive: ExpeditionMotive, bearing: &str, pick: usize) -> String {
    let lines = match motive {
        // 漂泊天性：純粹想往荒野走走看看。
        ExpeditionMotive::Roam => [
            format!("今天想往{bearing}走遠一點，去世界的邊陲看看～"),
            format!("待在城裡太久了，我想一個人去{bearing}的荒野走走。"),
            format!("腳癢了！這就動身往{bearing}遠行一趟。"),
        ],
        // 尋覓沃野：農人為主城外一片能耕作的好地方而遠行。
        ExpeditionMotive::SeekLand => [
            format!("城裡的地都種滿了，我想往{bearing}尋一片新的沃野。"),
            format!("聽說{bearing}的荒野土肥，我去看看能不能落腳耕作。"),
            format!("背上種子，動身往{bearing}的邊陲找塊好地～"),
        ],
    };
    cap(lines[pick % lines.len()].clone())
}

/// 抵達邊陲時，居民認出腳下生物群系的一句感嘆（面向玩家、i18n 友善集中此處）。
/// 生物群系第一刀（ROADMAP 725）讓世界有了「不同的地方」，但遠行到那裡的居民一直對「這是
/// 什麼地方」毫無知覺——只喊「這裡好開闊」，森林、沙漠、雪原都是同一句。本函式把地方感補上。
fn biome_flavor(biome: VoxelBiome) -> &'static str {
    match biome {
        VoxelBiome::Grassland => "一片開闊的草原，風吹草浪好舒服",
        VoxelBiome::Forest => "一片幽深的森林，樹影遮天、格外清涼",
        VoxelBiome::Desert => "一片焦黃的沙漠，好熱、只有仙人掌作伴",
        VoxelBiome::Snow => "一片皚皚的雪原，冷得直哆嗦、卻美得屏息",
    }
}

/// 抵達遠方邊陲時冒的泡泡（生物群系版——居民第一次認出「這是什麼地方」）。
/// 方位點出「往哪走」、群系點出「到了什麼地方」，兩者合起來地方感才具體。依 `pick` 輪替開頭不機械。
pub fn arrive_bubble(bearing: &str, biome: VoxelBiome, pick: usize) -> String {
    let flavor = biome_flavor(biome);
    let lines = [
        format!("終於走到{bearing}的邊陲了——這裡是{flavor}……"),
        format!("原來{bearing}這麼遠的地方，是{flavor}。"),
        format!("我走到{bearing}的盡頭，眼前是{flavor}。"),
    ];
    cap(lines[pick % lines.len()].clone())
}

/// 遠行歸來（回到家域）時冒的泡泡（生物群系版，ROADMAP 761）：不再只是「回來了」——還點出
/// 從邊陲帶回了什麼當地風物（要種在家門前做紀念）。方位／群系合起來，一趟遠行的收穫才具體。
pub fn return_bubble(biome: VoxelBiome, pick: usize) -> String {
    let item = keepsake_name_zh(biome);
    let lines = [
        format!("遠行回來啦！還帶回了{item}，種在家門口做紀念～"),
        format!("走了好遠一圈，帶著{item}回家，家附近最讓人安心。"),
        format!("這趟遠行帶回{item}，家門前又多了一件遠方的紀念。"),
    ];
    cap(lines[pick % lines.len()].clone())
}

/// 抵達邊陲時昇華成的記憶摘要（生物群系版，掛 [`EXPEDITION_MEMORY_PLAYER`] 哨兵；日記／內心
/// 可引用「牠去過什麼地方」）。記下的不再只是「一片荒野」，而是具體的草原／森林／沙漠／雪原。
pub fn arrive_memory_summary(bearing: &str, biome: VoxelBiome) -> String {
    format!("我獨自遠行到{bearing}的邊陲，那裡是一片{}，和主城很不一樣。", biome_name(biome))
}

/// 抵達邊陲的 Feed 播報詳情（生物群系版）：不在場的玩家回來也能從動態牆讀到「牠到了什麼地方」。
pub fn arrive_feed_line(bearing: &str, biome: VoxelBiome) -> String {
    format!("抵達{bearing}的邊陲——那裡是一片{}", biome_name(biome))
}

/// 啟程遠行的 Feed 播報詳情（依動機分岔，面向玩家、集中可 i18n）：動態牆上，兩位散居者的
/// 遠行各自寫著不同的緣由——不在場的玩家回來也讀得出「誰為了什麼走進了荒野」。
pub fn embark_feed_line(motive: ExpeditionMotive, bearing: &str) -> String {
    match motive {
        ExpeditionMotive::Roam => format!("獨自往{bearing}的邊陲遠行了"),
        ExpeditionMotive::SeekLand => format!("往{bearing}的邊陲遠行，去尋一片能耕作的沃野"),
    }
}

/// 遠行歸來的 Feed 播報詳情（生物群系版）：帶回的見聞點名去過的地方，動態牆上的一趟遠行有始有終。
pub fn return_feed_line(biome: VoxelBiome) -> String {
    format!("遠行歸來，帶回了{}盡頭的見聞", biome_name(biome))
}

// ── 邊陲營火路標（遠行 v2，PLAN_ETHERVOX item 7 後續「在遠方留下痕跡」）─────────────────
// item 7 第一刀（遠行 v1）讓居民走進荒野再走回來——足跡散進了荒野，但世界本身沒留下任何痕跡：
// 居民一走，那片邊陲又空無一物，玩家除非「正好在場」否則永遠不知道居民來過。這一刀把「暫時到訪」
// 升級成「留下永久記號」：抵達邊陲時，居民親手升起一堆營火路標（實體方塊、走 world delta 持久化），
// 日後任何玩家路過那片荒野，都會撞見這堆營火、知道「有居民的足跡到過這裡」。世界第一次因居民散佈
// 而在主城以外長出實體痕跡，是把「散往世界各處『住』」從人影過境推向落地生根的地基。

/// 遠行居民抵達邊陲時親手升起的一堆「營火路標」的方塊布局（世界座標）：以落點 `(bx,bz)`、地表
/// 站立高度 `sy`（[`crate::voxel_building::surface_y`] 回傳的地面正上方那格）為基準——中心一塊
/// StoneBrick 灶台（人為鋪過的痕跡、有別於天然亂石）、四鄰各一塊 Stone 圍成火塘圈、灶台上一支
/// Torch 當火（夜裡在荒野遠遠就能望見這點光）。共 6 塊，稀少而有份量、不喧賓奪主。
/// 純函式、確定性、無 IO；實際落地（deltas 寫 + 廣播 + 持久化）在 `voxel_ws.rs` 鎖外進行。
pub fn campfire_blocks(bx: i32, sy: i32, bz: i32) -> Vec<(i32, i32, i32, Block)> {
    let mut v = Vec::with_capacity(6);
    // 灶台：中心鋪一塊石磚（人為痕跡，一眼看得出不是天然亂石）。
    v.push((bx, sy, bz, Block::StoneBrick));
    // 圍石：四鄰各一塊石頭，圍成一圈火塘。
    for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
        v.push((bx + dx, sy, bz + dz, Block::Stone));
    }
    // 火：灶台正上方一支火把（荒野夜裡遠遠一點光）。
    v.push((bx, sy + 1, bz, Block::Torch));
    v
}

/// 營火路標中心「火」的座標（即 [`campfire_blocks`] 產出的火把位置）——落地前用來判定該落點是否
/// 已有一堆營火（冪等：同一落點不重複堆疊，也避免重啟 replay 後重放產生的多餘 append）。
pub fn campfire_flame_pos(bx: i32, sy: i32, bz: i32) -> (i32, i32, i32) {
    (bx, sy + 1, bz)
}

/// 升起營火時昇華成的記憶摘要（掛 [`EXPEDITION_MEMORY_PLAYER`] 哨兵，日記／內心可引用）。
pub fn campfire_memory_summary(bearing: &str) -> String {
    format!("我在{bearing}的邊陲升起一堆營火，為自己走過的足跡留下一個記號。")
}

/// 升起營火的 Feed 播報詳情（面向玩家、集中可 i18n）。
pub fn campfire_feed_line(bearing: &str) -> String {
    format!("在{bearing}的邊陲升起一堆營火，為足跡留下記號")
}

// ── 邊陲固定營地（遠行 v3，ROADMAP 758「散往世界各處『住』」）─────────────────────────
// v1/v2 每趟遠行的落點都由當下身位抖動決定，居民每次跑到荒野裡不同的一點——這一刀把落點改由
// 「家的方位」確定性算出：同一位居民每趟遠行都回到同一處邊陲營地，那裡漸漸長成牠專屬的據點。

/// 由居民的家座標算出「這位居民專屬的固定邊陲營地」的 seq——確定性、不隨身位變動，讓每趟遠行都
/// 回到同一處落點（呼叫端把此值餵給 [`pick_frontier`]）。家座標整趟遠行不變，故落點恆定。
/// 純函式、確定性、無 IO。
pub fn outpost_seq(home_x: f32, home_z: f32) -> usize {
    (home_x.to_bits() ^ home_z.to_bits()) as usize
}

// ── 邊陲紮營小棚（遠行 v3，ROADMAP 758「在遠方紮營、雛形第二個家」）────────────────────
// 抵達邊陲時，居民除了升起營火（v2），還在營火旁搭起一座簡易紮營小棚——荒野裡第一次有一張床、
// 一個過夜的地方，是「第二個家」最初的雛形。走既有 world delta 持久化，冪等（同址只搭一次）。

/// 紮營小棚相對營火中心的水平位移（世界座標，東向 +x）：擺在營火東側 3 格，避開營火本身
/// （footprint 橫跨中心 ±1 格）與抵達落點，讓「營火在前、小棚在後」一眼看得出是一處營地。
pub const SHELTER_OFFSET_X: i32 = 3;

/// 紮營小棚錨點（世界座標）：由營火中心 `(bx, bz)` 往東推 [`SHELTER_OFFSET_X`] 格。
/// 小棚自身的地表高度另由呼叫端 `surface_y(ax, az)` 算（地形起伏各欄不同）。純函式、確定性。
pub fn shelter_anchor(bx: i32, bz: i32) -> (i32, i32) {
    (bx + SHELTER_OFFSET_X, bz)
}

/// 紮營小棚的方塊布局（世界座標）：以錨點 `(ax, az)`、地表站立層 `ay`（`surface_y` 回傳的地面
/// 正上方那格）為基準，搭一座朝西（-x，面向營火）開口的簡易 lean-to——
/// **背牆**（東側 dx=+1）2 寬 × 2 高共 4 塊木板遮風；**屋頂** ay+2 一片 2×2 共 4 塊木板遮雨；
/// **床** 擺在小棚裡（錨點地面 ay）一張，是荒野裡第一個過夜的地方。共 9 塊，三面透空的 lean-to
/// 不會把居民困在裡面。純函式、確定性、無 IO；實際落地（deltas 寫＋廣播＋持久化）在 `voxel_ws.rs`
/// 鎖外進行，且每塊逐一以「該格為空氣才放」為守（起伏地形上不覆蓋既有地形/水，只是少放幾塊）。
pub fn shelter_blocks(ax: i32, ay: i32, az: i32) -> Vec<(i32, i32, i32, Block)> {
    let mut v = Vec::with_capacity(9);
    // 背牆（東側 dx=+1）：2 寬（z=az, az+1）× 2 高（ay, ay+1），共 4 塊木板遮風。
    for dz in [0, 1] {
        v.push((ax + 1, ay, az + dz, Block::Plank));
        v.push((ax + 1, ay + 1, az + dz, Block::Plank));
    }
    // 屋頂（ay+2）：蓋住整個 2×2 footprint，共 4 塊木板遮雨。
    for dx in [0, 1] {
        for dz in [0, 1] {
            v.push((ax + dx, ay + 2, az + dz, Block::Plank));
        }
    }
    // 床：擺在小棚裡、朝開口那側（dx=0）的地面站立層，一張。荒野裡第一個過夜的地方。
    v.push((ax, ay, az, Block::Bed));
    v
}

/// 紮營小棚的「床」座標（即 [`shelter_blocks`] 產出的 Bed 位置）——落地前用來判定該處是否已搭過
/// 小棚（冪等：同址不重複搭，也避免重啟 replay 後重放產生的多餘 append）。
pub fn shelter_bed_pos(ax: i32, ay: i32, az: i32) -> (i32, i32, i32) {
    (ax, ay, az)
}

/// 搭起紮營小棚時昇華成的記憶摘要（掛 [`EXPEDITION_MEMORY_PLAYER`] 哨兵，日記／內心可引用）。
pub fn shelter_memory_summary(bearing: &str) -> String {
    format!("我在{bearing}的邊陲營火旁搭起一座小棚，這裡漸漸成了我在荒野的第二個家。")
}

/// 搭起紮營小棚的 Feed 播報詳情（面向玩家、集中可 i18n）。
pub fn shelter_feed_line(bearing: &str) -> String {
    format!("在{bearing}的邊陲營火旁搭起一座紮營小棚，雛形第二個家")
}

// ── 邊陲過夜 v4（ROADMAP 759，PLAN_ETHERVOX item 7「散佈各處『住』」第四刀）──────────────
// 758 讓居民在邊陲搭起帶一張床的小棚，但那張床至今只是裝飾——居民的逗留倒數一到就返家，從沒真的
// 睡上去過。這一刀讓「住」名副其實：**遠行途中若夜色降臨，居民不趕夜路，改走到營地那張床上睡一覺、
// 天亮才啟程回主城**——第二個家的床第一次被真正睡上，「散佈各處住」從「有張床」變成「在那兒過夜」。
// 純函式層只判定「該不該就寢」「醒來說什麼／記什麼」；狀態機、鎖、廣播都在 `voxel_ws.rs`。

/// 邊陲小棚那張床的水平中心座標（世界座標，f32）：由遠行落點（營火中心）`(tx, tz)` 推得——
/// 營火中心 → [`shelter_anchor`] 錨點 →床恰在錨點（[`shelter_blocks`] 的 Bed 擺在 `(ax, ay, az)`）。
/// 供狀態機在夜裡把居民導向床邊、並判定是否已走到床跟前。純函式、確定性、無 IO。
pub fn outpost_bed_center(tx: i32, tz: i32) -> (f32, f32) {
    let (ax, az) = shelter_anchor(tx, tz);
    (ax as f32 + 0.5, az as f32 + 0.5)
}

/// 距離平方判定「已走到營地床邊」（避免開根號），半徑 [`OUTPOST_BED_NEAR`]。純函式。
pub fn near_outpost_bed(bx: f32, bz: f32, bedx: f32, bedz: f32) -> bool {
    let dx = bx - bedx;
    let dz = bz - bedz;
    dx * dx + dz * dz <= OUTPOST_BED_NEAR * OUTPOST_BED_NEAR
}

/// 是否該此刻在邊陲營地就寢：正逗留邊陲 + 已入深夜 + 已走到床邊。三者皆備才躺下——
/// 白天照常逗留探索、天還沒全黑（傍晚）先走向床邊等、還沒走到床邊不就地睡死。純函式。
pub fn should_sleep_at_outpost(is_deep_night: bool, near_bed: bool) -> bool {
    is_deep_night && near_bed
}

/// 在邊陲床上入睡時頭頂冒的泡泡（點出「在第二個家過夜」，與家用睡眠語有別）。
pub fn outpost_sleep_bubble(bearing: &str, pick: usize) -> String {
    let lines = [
        format!("天黑了，今晚就在{bearing}的營地睡一覺吧～"),
        "不趕夜路了，第二個家的床，正好過一夜。".to_string(),
        "荒野的夜好靜……躺在自己搭的床上睡了。".to_string(),
    ];
    cap(lines[pick % lines.len()].clone())
}

/// 邊陲過夜醒來、準備啟程回主城時冒的泡泡。
pub fn outpost_wake_bubble(bearing: &str, pick: usize) -> String {
    let lines = [
        format!("在{bearing}的營地睡飽了，該啟程回主城囉～"),
        "荒野的清晨真清爽，收拾收拾回家吧。".to_string(),
        "在自己的第二個家過了一夜，神清氣爽！".to_string(),
    ];
    cap(lines[pick % lines.len()].clone())
}

/// 「在邊陲第二個家過了一夜」昇華成的記憶摘要（掛 [`EXPEDITION_MEMORY_PLAYER`] 哨兵，日記／內心可引用）。
pub fn outpost_sleep_memory_summary(bearing: &str) -> String {
    format!("我在{bearing}的邊陲營地過了一夜，睡在自己搭的那張床上——這裡真的成了我的第二個家。")
}

/// 邊陲就寢的 Feed 播報詳情（面向玩家、集中可 i18n）。
pub fn outpost_sleep_feed_line(bearing: &str) -> String {
    format!("在{bearing}的邊陲營地過夜，睡在自己搭的那張床上")
}

/// 邊陲過夜醒來、啟程返家的 Feed 播報詳情。
pub fn outpost_wake_feed_line(bearing: &str) -> String {
    format!("在{bearing}的邊陲營地睡醒，啟程返回主城")
}

// ── 遠行帶回的邊陲風物（遠行 v5，ROADMAP 761，PLAN_ETHERVOX item 7「不同地方採到不同資源」）──
// 760 讓居民認出腳下是草原／森林／沙漠／雪原，並埋下鉤子「這是往後不同地方採到不同資源／對地方
// 形成偏好的記憶地基」。這一刀兌現它的第一步：遠行歸來的居民從邊陲群系**帶回一件當地特產風物**，
// 種在自家門前的一小列「紀念花圃」——玩家會看到居民的家門漸漸長出一排來自遠方的紀念（草原的小樹苗、
// 森林的枝葉、沙漠的仙人掌、雪原的冰晶），每一件都對應牠去過的一個地方。四種群系各佔花圃一格、
// 每格只種一次（冪等），累積成一排「我去過哪些地方」的實體記憶。純函式層只算「種什麼／種哪／說什麼／
// 記什麼」；實際落地（deltas 寫＋廣播＋持久化）在 `voxel_ws.rs` 鎖外進行，比照營火／小棚手法。

/// 家門前「紀念花圃」的擺放基準：由家中心往南（+z）推這麼遠——避開屋舍核心（家域半徑 20 格，
/// 屋舍集中在中心一帶），又近到玩家一進家門就看得見這排遠方帶回的紀念。
pub const KEEPSAKE_ROW_Z: i32 = 6;

/// 依邊陲群系決定帶回的當地風物方塊：草原→小樹苗、森林→枝葉、沙漠→仙人掌、雪原→冰晶。
/// 皆為既有的可放置裝飾方塊；樹苗直接以 delta 落地（**不經 grove store 註冊、故不會長成樹**，
/// 只是一株靜態的紀念小苗）。純函式、確定性。
pub fn keepsake_block(biome: VoxelBiome) -> Block {
    match biome {
        VoxelBiome::Grassland => Block::Sapling,
        VoxelBiome::Forest => Block::Leaves,
        VoxelBiome::Desert => Block::Cactus,
        VoxelBiome::Snow => Block::IceCrystal,
    }
}

/// 風物的面向玩家名（繁中，i18n 友善集中此處）——泡泡／記憶／Feed 皆引用。
pub fn keepsake_name_zh(biome: VoxelBiome) -> &'static str {
    match biome {
        VoxelBiome::Grassland => "一株草原的小樹苗",
        VoxelBiome::Forest => "一叢森林的青翠枝葉",
        VoxelBiome::Desert => "一株沙漠的仙人掌",
        VoxelBiome::Snow => "一簇雪原的晶亮冰晶",
    }
}

/// 風物在紀念花圃裡的欄位 x 位移（相對家中心）：四種群系各錯開一格，排成一小列——同一位居民
/// 去過不同地方帶回的風物並排累積，不互相覆蓋。純函式、確定性。
fn keepsake_col(biome: VoxelBiome) -> i32 {
    match biome {
        VoxelBiome::Grassland => -3,
        VoxelBiome::Forest => -1,
        VoxelBiome::Desert => 1,
        VoxelBiome::Snow => 3,
    }
}

/// 風物該種下的水平座標（世界座標）：由家中心往南 [`KEEPSAKE_ROW_Z`] 格、依群系錯開 x——
/// 同一位居民同一群系的風物每趟遠行都落在同一格（冪等只種一次）。地表高度由呼叫端 `surface_y`
/// 另算（地形起伏各欄不同）。純函式、確定性、無 IO。
pub fn keepsake_pos(home_x: f32, home_z: f32, biome: VoxelBiome) -> (i32, i32) {
    let cx = home_x.round() as i32 + keepsake_col(biome);
    let cz = home_z.round() as i32 + KEEPSAKE_ROW_Z;
    (cx, cz)
}

/// 種下帶回的風物昇華成的記憶摘要（掛 [`EXPEDITION_MEMORY_PLAYER`] 哨兵，日記／內心可引用）——
/// 記下的是「我從東方的沙漠帶回一株仙人掌」，家門前那排紀念是牠去過每個地方的實體記憶。
pub fn keepsake_memory_summary(bearing: &str, biome: VoxelBiome) -> String {
    format!(
        "我從{bearing}的{}帶回{}，種在家門前——家門口那排紀念，記著我去過的每個地方。",
        biome_name(biome),
        keepsake_name_zh(biome)
    )
}

/// 種下帶回風物的 Feed 播報詳情（面向玩家、集中可 i18n）：不在場的玩家回來也能從動態牆讀到
/// 「牠從哪個地方帶回了什麼、種在家門前」。
pub fn keepsake_feed_line(bearing: &str, biome: VoxelBiome) -> String {
    format!(
        "從{bearing}的{}帶回{}，種在家門前的紀念花圃",
        biome_name(biome),
        keepsake_name_zh(biome)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 遠行動機只給漂泊者與農人() {
        // 散居 v6：Wanderer（奧瑞）漂泊、FarmWorker（諾娃）尋地 → 會遠行且動機各異。
        assert_eq!(
            expedition_motive(ResidentPersona::Wanderer),
            Some(ExpeditionMotive::Roam)
        );
        assert_eq!(
            expedition_motive(ResidentPersona::FarmWorker),
            Some(ExpeditionMotive::SeekLand)
        );
        // 市集人（露娜）／廣場人（賽勒）留守主城、不遠行。
        assert_eq!(expedition_motive(ResidentPersona::MarketBrowser), None);
        assert_eq!(expedition_motive(ResidentPersona::TownSquare), None);
    }

    #[test]
    fn embark_needs_all_gates() {
        // 全條件滿足 + roll 過門檻 → true。
        assert!(should_embark(true, true, true, 0.0, true, 0.0));
        // 不能遠行的人格（can_embark=false）→ 永不遠行。
        assert!(!should_embark(false, true, true, 0.0, true, 0.0));
        // 正忙別的意圖（idle_free=false）→ 不遠行。
        assert!(!should_embark(true, false, true, 0.0, true, 0.0));
        // 夜裡 → 不往荒野跑。
        assert!(!should_embark(true, true, false, 0.0, true, 0.0));
        // 冷卻未到 → 不遠行。
        assert!(!should_embark(true, true, true, 5.0, true, 0.0));
        // 此刻正在說話 → 不遠行（不打斷冒泡）。
        assert!(!should_embark(true, true, true, 0.0, false, 0.0));
        // roll 沒過機率門檻 → 不遠行。
        assert!(!should_embark(true, true, true, 0.0, true, 0.99));
    }

    #[test]
    fn embark_chance_is_the_gate_boundary() {
        // 恰在門檻下 → true；恰在門檻（含以上）→ false。
        assert!(should_embark(true, true, true, 0.0, true, EMBARK_CHANCE - 0.001));
        assert!(!should_embark(true, true, true, 0.0, true, EMBARK_CHANCE));
    }

    #[test]
    fn bearing_label_four_quadrants() {
        assert_eq!(bearing_label(1.0, 0.0), "東方");
        assert_eq!(bearing_label(-1.0, 0.0), "西方");
        assert_eq!(bearing_label(0.0, 1.0), "南方");
        assert_eq!(bearing_label(0.0, -1.0), "北方");
        // 對角時取主導軸（x 佔優 → 東西）。
        assert_eq!(bearing_label(2.0, 1.0), "東方");
        assert_eq!(bearing_label(-2.0, 1.0), "西方");
        assert_eq!(bearing_label(1.0, 2.0), "南方");
        assert_eq!(bearing_label(1.0, -2.0), "北方");
    }

    #[test]
    fn frontier_pushes_farther_from_origin_than_home() {
        // 奧瑞家在東方 (75,0)：遠行落點應更往東、離原點更遠。
        let (fx, fz, bearing) = pick_frontier(75.0, 0.0, 0);
        let home_d = (75.0_f32 * 75.0).sqrt();
        let front_d = (fx * fx + fz * fz).sqrt();
        assert!(front_d > home_d, "遠行落點應比家離主城更遠");
        assert!(fx > 75.0, "應更往東推");
        assert_eq!(bearing, "東方");
        // z 幾乎沿東向（抖動有限）。
        assert!(fz.abs() < 60.0);
    }

    #[test]
    fn frontier_directions_match_home_direction() {
        // 南方家 (0,75) → 往南更遠。
        let (_, fz, bearing) = pick_frontier(0.0, 75.0, 0);
        assert!(fz > 75.0);
        assert_eq!(bearing, "南方");
        // 西方家 (-75,0) → 往西更遠。
        let (fx, _, bearing) = pick_frontier(-75.0, 0.0, 0);
        assert!(fx < -75.0);
        assert_eq!(bearing, "西方");
    }

    #[test]
    fn frontier_origin_home_falls_back_to_seq_cardinal() {
        // 家恰在原點：不 panic、用 seq 選基本方位，落點離原點約 EXPEDITION_DIST。
        let (fx, fz, _) = pick_frontier(0.0, 0.0, 0);
        let d = (fx * fx + fz * fz).sqrt();
        assert!(d >= EXPEDITION_DIST - 1.0 && d <= EXPEDITION_DIST + 40.0);
        // 不同 seq 落在不同基本方位。
        let (ax, _, _) = pick_frontier(0.0, 0.0, 0);
        let (bx, bz, _) = pick_frontier(0.0, 0.0, 1);
        assert!((ax - bx).abs() > 1.0 || bz.abs() > 1.0);
    }

    #[test]
    fn frontier_seq_varies_landing_spot() {
        // 同一個家、不同 seq → 落點不同（角度／距離抖動生效），不機械地永遠同一格。
        let (x0, z0, _) = pick_frontier(75.0, 0.0, 0);
        let (x1, z1, _) = pick_frontier(75.0, 0.0, 3);
        assert!((x0 - x1).abs() > 0.5 || (z0 - z1).abs() > 0.5);
    }

    #[test]
    fn bubbles_and_memory_nonempty_capped_and_mention_bearing() {
        let biomes = [
            VoxelBiome::Grassland,
            VoxelBiome::Forest,
            VoxelBiome::Desert,
            VoxelBiome::Snow,
        ];
        for pick in 0..6 {
            let e = embark_bubble(ExpeditionMotive::Roam, "東方", pick);
            let r = return_bubble(VoxelBiome::Grassland, pick);
            assert!(!e.is_empty() && e.chars().count() <= SAY_MAX_CHARS);
            assert!(!r.is_empty() && r.chars().count() <= SAY_MAX_CHARS);
            // 抵達泡泡每種群系都非空、有界，且真的帶上該群系的地方感詞。
            for b in biomes {
                let a = arrive_bubble("西方", b, pick);
                assert!(!a.is_empty() && a.chars().count() <= SAY_MAX_CHARS);
                assert!(a.contains(biome_name(b)) || a.contains(biome_flavor(b)));
            }
        }
        // 尋地版啟程泡泡（諾娃）也非空、有界、點出方位。
        for pick in 0..6 {
            let s = embark_bubble(ExpeditionMotive::SeekLand, "南方", pick);
            assert!(!s.is_empty() && s.chars().count() <= SAY_MAX_CHARS);
            assert!(s.contains("南方"));
        }
        // 啟程泡泡點出方位；抵達記憶同時點出方位與去過的群系。
        assert!(embark_bubble(ExpeditionMotive::Roam, "南方", 0).contains("南方"));
        assert!(arrive_memory_summary("北方", VoxelBiome::Desert).contains("北方"));
        assert!(arrive_memory_summary("東方", VoxelBiome::Snow).contains("雪原"));
        assert!(!arrive_memory_summary("東方", VoxelBiome::Grassland).is_empty());
    }

    #[test]
    fn bubbles_rotate_by_pick() {
        // 不同 pick 至少有兩種不同的啟程泡泡（輪替、不永遠同一句）。
        let a = embark_bubble(ExpeditionMotive::Roam, "東方", 0);
        let b = embark_bubble(ExpeditionMotive::Roam, "東方", 1);
        let c = embark_bubble(ExpeditionMotive::Roam, "東方", 2);
        assert!(a != b || b != c);
    }

    #[test]
    fn 動機不同啟程台詞也不同() {
        // 同方位同 pick，漂泊（奧瑞）與尋地（諾娃）的啟程泡泡／Feed 口吻各異。
        assert_ne!(
            embark_bubble(ExpeditionMotive::Roam, "南方", 0),
            embark_bubble(ExpeditionMotive::SeekLand, "南方", 0)
        );
        assert_ne!(
            embark_feed_line(ExpeditionMotive::Roam, "南方"),
            embark_feed_line(ExpeditionMotive::SeekLand, "南方")
        );
        // 尋地版點出「耕作／沃野」的農人動機。
        let f = embark_feed_line(ExpeditionMotive::SeekLand, "南方");
        assert!(f.contains("耕作") || f.contains("沃野"));
    }

    #[test]
    fn feed_lines_nonempty() {
        assert!(!embark_feed_line(ExpeditionMotive::Roam, "東方").is_empty());
        assert!(embark_feed_line(ExpeditionMotive::Roam, "南方").contains("南方"));
        // 歸來 Feed 帶上去過的群系名；抵達 Feed 同時帶方位與群系。
        assert!(return_feed_line(VoxelBiome::Forest).contains("森林"));
        assert!(arrive_feed_line("東方", VoxelBiome::Desert).contains("東方"));
        assert!(arrive_feed_line("東方", VoxelBiome::Desert).contains("沙漠"));
    }

    #[test]
    fn campfire_has_hearth_ring_and_flame() {
        let v = campfire_blocks(100, 33, -40);
        // 共 6 塊：1 灶台 + 4 圍石 + 1 火。
        assert_eq!(v.len(), 6);
        // 中心灶台是石磚、在地表層 sy。
        assert!(v.contains(&(100, 33, -40, Block::StoneBrick)));
        // 四鄰各一塊 Stone 圍石（同一 sy 高度）。
        for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            assert!(v.contains(&(100 + dx, 33, -40 + dz, Block::Stone)));
        }
        // 火在灶台正上方 sy+1、且恰好一支。
        let torches: Vec<_> = v.iter().filter(|(.., b)| *b == Block::Torch).collect();
        assert_eq!(torches.len(), 1);
        assert_eq!(*torches[0], (100, 34, -40, Block::Torch));
    }

    #[test]
    fn campfire_flame_pos_matches_torch_in_layout() {
        // flame_pos 必須與 campfire_blocks 產出的火把座標一致（冪等判定靠它）。
        let (bx, sy, bz) = (7, 34, 12);
        let flame = campfire_flame_pos(bx, sy, bz);
        assert_eq!(flame, (bx, sy + 1, bz));
        assert!(campfire_blocks(bx, sy, bz)
            .iter()
            .any(|(x, y, z, b)| (*x, *y, *z) == flame && *b == Block::Torch));
    }

    #[test]
    fn campfire_lines_nonempty_and_mention_bearing() {
        assert!(campfire_memory_summary("東方").contains("東方"));
        assert!(!campfire_memory_summary("西方").is_empty());
        assert!(campfire_feed_line("南方").contains("南方"));
        assert!(!campfire_feed_line("北方").is_empty());
    }

    #[test]
    fn outpost_seq_is_stable_per_home_and_yields_fixed_frontier() {
        // 同一個家座標 → outpost_seq 恆定（不隨身位變動）。
        let s0 = outpost_seq(75.0, 0.0);
        let s1 = outpost_seq(75.0, 0.0);
        assert_eq!(s0, s1, "同家座標的 outpost_seq 應恆定");
        // 用 outpost_seq 餵 pick_frontier → 同一位居民每趟遠行都回到同一處落點。
        let (ax, az, ab) = pick_frontier(75.0, 0.0, s0);
        let (bx, bz, bb) = pick_frontier(75.0, 0.0, s1);
        assert_eq!((ax, az, ab), (bx, bz, bb), "固定 seq 應落在同一處邊陲營地");
        // 不同的家 → 通常落在不同據點（散佈各處，各有各的營地）。
        let se = outpost_seq(-75.0, 0.0);
        let (ex, _, eb) = pick_frontier(-75.0, 0.0, se);
        assert!(ex < 0.0 && eb == "西方", "西方家的據點應在西邊");
    }

    #[test]
    fn shelter_anchor_is_east_of_campfire_clear_of_footprint() {
        // 小棚錨點在營火中心東側 SHELTER_OFFSET_X 格——避開營火 ±1 格 footprint。
        let (ax, az) = shelter_anchor(100, -40);
        assert_eq!((ax, az), (100 + SHELTER_OFFSET_X, -40));
        assert!(ax > 100 + 1, "小棚背牆應落在營火 footprint(中心±1)之外");
    }

    #[test]
    fn shelter_has_backwall_roof_and_a_bed() {
        let (ax, ay, az) = (50, 33, 12);
        let v = shelter_blocks(ax, ay, az);
        // 共 9 塊：4 背牆 + 4 屋頂 + 1 床。
        assert_eq!(v.len(), 9);
        // 背牆：東側 dx=+1，2 寬 × 2 高，皆木板。
        for dz in [0, 1] {
            assert!(v.contains(&(ax + 1, ay, az + dz, Block::Plank)));
            assert!(v.contains(&(ax + 1, ay + 1, az + dz, Block::Plank)));
        }
        // 屋頂：ay+2 一片 2×2 木板。
        for dx in [0, 1] {
            for dz in [0, 1] {
                assert!(v.contains(&(ax + dx, ay + 2, az + dz, Block::Plank)));
            }
        }
        // 恰一張床，在錨點地面站立層。
        let beds: Vec<_> = v.iter().filter(|(.., b)| *b == Block::Bed).collect();
        assert_eq!(beds.len(), 1);
        assert_eq!(*beds[0], (ax, ay, az, Block::Bed));
        // 三面透空（開口側 dx=0 的 ay/ay+1 除了床格外沒有牆）→ 不困住居民。
        assert!(!v.iter().any(|(x, y, ..)| *x == ax && *y == ay + 1 && (*x, *y) == (ax, ay + 1)));
    }

    #[test]
    fn shelter_bed_pos_matches_bed_in_layout() {
        // bed_pos 必須與 shelter_blocks 產出的床座標一致（冪等判定靠它）。
        let (ax, ay, az) = (8, 34, -3);
        let bed = shelter_bed_pos(ax, ay, az);
        assert_eq!(bed, (ax, ay, az));
        assert!(shelter_blocks(ax, ay, az)
            .iter()
            .any(|(x, y, z, b)| (*x, *y, *z) == bed && *b == Block::Bed));
    }

    #[test]
    fn shelter_lines_nonempty_and_mention_bearing() {
        assert!(shelter_memory_summary("東方").contains("東方"));
        assert!(!shelter_memory_summary("西方").is_empty());
        assert!(shelter_feed_line("南方").contains("南方"));
        assert!(!shelter_feed_line("北方").is_empty());
    }

    // ── 邊陲過夜 v4（ROADMAP 759）────────────────────────────────────────────

    #[test]
    fn outpost_bed_center_sits_on_shelter_bed() {
        // 床水平中心必須落在 shelter_blocks 產出的 Bed 那一格中心（狀態機導向床邊靠它）。
        let (tx, tz) = (100, -5);
        let (cx, cz) = outpost_bed_center(tx, tz);
        let (ax, az) = shelter_anchor(tx, tz);
        // 床格 = (ax, ay, az)，水平中心 = (ax+0.5, az+0.5)。
        assert_eq!((cx, cz), (ax as f32 + 0.5, az as f32 + 0.5));
        // 且該格確實是布局裡的床（ay 任取）。
        assert!(shelter_blocks(ax, 34, az)
            .iter()
            .any(|(x, _, z, b)| *x == ax && *z == az && *b == Block::Bed));
    }

    #[test]
    fn near_outpost_bed_uses_radius() {
        let (bedx, bedz) = (10.5, -3.5);
        assert!(near_outpost_bed(bedx, bedz, bedx, bedz), "正在床上 → 近");
        assert!(
            near_outpost_bed(bedx + OUTPOST_BED_NEAR, bedz, bedx, bedz),
            "恰在半徑上 → 近（含邊界）"
        );
        assert!(
            !near_outpost_bed(bedx + OUTPOST_BED_NEAR + 0.1, bedz, bedx, bedz),
            "超出半徑 → 不近"
        );
    }

    #[test]
    fn should_sleep_at_outpost_requires_deep_night_and_near_bed() {
        assert!(should_sleep_at_outpost(true, true), "深夜+到床邊 → 睡");
        assert!(!should_sleep_at_outpost(false, true), "白天不睡");
        assert!(!should_sleep_at_outpost(true, false), "還沒到床邊不睡");
        assert!(!should_sleep_at_outpost(false, false));
    }

    #[test]
    fn outpost_sleep_wake_bubbles_vary_and_cap() {
        // 依 pick 輪替、非空、不破框（≤ SAY_MAX_CHARS 字元）。
        assert_ne!(outpost_sleep_bubble("東方", 0), outpost_sleep_bubble("東方", 1));
        assert_ne!(outpost_wake_bubble("東方", 0), outpost_wake_bubble("東方", 1));
        for pick in 0..3 {
            assert!(!outpost_sleep_bubble("南方", pick).is_empty());
            assert!(!outpost_wake_bubble("南方", pick).is_empty());
            assert!(outpost_sleep_bubble("南方", pick).chars().count() <= SAY_MAX_CHARS);
            assert!(outpost_wake_bubble("南方", pick).chars().count() <= SAY_MAX_CHARS);
        }
        // pick 取模不越界。
        let _ = outpost_sleep_bubble("北方", usize::MAX);
        let _ = outpost_wake_bubble("北方", usize::MAX);
    }

    #[test]
    fn outpost_sleep_memory_and_feed_mention_bearing() {
        assert!(outpost_sleep_memory_summary("東方").contains("東方"));
        assert!(outpost_sleep_memory_summary("東方").contains("第二個家"));
        assert!(outpost_sleep_feed_line("西方").contains("西方"));
        assert!(outpost_wake_feed_line("南方").contains("南方"));
        assert!(!outpost_sleep_feed_line("北方").is_empty());
        assert!(!outpost_wake_feed_line("北方").is_empty());
    }

    // ── 遠行帶回的邊陲風物（ROADMAP 761）────────────────────────────────────────

    const ALL_BIOMES: [VoxelBiome; 4] = [
        VoxelBiome::Grassland,
        VoxelBiome::Forest,
        VoxelBiome::Desert,
        VoxelBiome::Snow,
    ];

    #[test]
    fn keepsake_block_maps_each_biome_to_its_specialty() {
        assert_eq!(keepsake_block(VoxelBiome::Grassland), Block::Sapling);
        assert_eq!(keepsake_block(VoxelBiome::Forest), Block::Leaves);
        assert_eq!(keepsake_block(VoxelBiome::Desert), Block::Cactus);
        assert_eq!(keepsake_block(VoxelBiome::Snow), Block::IceCrystal);
    }

    #[test]
    fn keepsake_pos_is_south_of_home_and_distinct_per_biome() {
        let (hx, hz) = (75.0_f32, 0.0_f32); // 奧瑞的家（東方）
        let mut xs = Vec::new();
        for b in ALL_BIOMES {
            let (kx, kz) = keepsake_pos(hx, hz, b);
            // 每種群系都落在家中心南側同一列（家 z + KEEPSAKE_ROW_Z）。
            assert_eq!(kz, hz.round() as i32 + KEEPSAKE_ROW_Z);
            // 確定性：同輸入恆得同座標。
            assert_eq!((kx, kz), keepsake_pos(hx, hz, b));
            xs.push(kx);
        }
        // 四種群系錯開成一小列，各佔不同的一格（不互相覆蓋）。
        let mut sorted = xs.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 4, "四種風物應排在四個不同的欄位");
    }

    #[test]
    fn return_bubble_mentions_the_keepsake_and_rotates() {
        for b in ALL_BIOMES {
            let item = keepsake_name_zh(b);
            for pick in 0..3 {
                let s = return_bubble(b, pick);
                assert!(!s.is_empty() && s.chars().count() <= SAY_MAX_CHARS);
                assert!(s.contains(item), "歸來泡泡應點名帶回的風物");
            }
            // 依 pick 輪替、不永遠同一句。
            assert!(return_bubble(b, 0) != return_bubble(b, 1) || return_bubble(b, 1) != return_bubble(b, 2));
        }
        // pick 取模不越界。
        let _ = return_bubble(VoxelBiome::Snow, usize::MAX);
    }

    #[test]
    fn keepsake_memory_and_feed_mention_bearing_biome_and_item() {
        for b in ALL_BIOMES {
            let item = keepsake_name_zh(b);
            let mem = keepsake_memory_summary("東方", b);
            let feed = keepsake_feed_line("東方", b);
            assert!(mem.contains("東方") && mem.contains(biome_name(b)) && mem.contains(item));
            assert!(feed.contains("東方") && feed.contains(biome_name(b)) && feed.contains(item));
            assert!(!mem.is_empty() && !feed.is_empty());
        }
    }
}
