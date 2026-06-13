//! 野生動物系統（ROADMAP 140~143）。
//!
//! ROADMAP 140：野鳥/野鹿/小動物——中立、只逃跑、不攻擊。
//! ROADMAP 141：野狼獵野鹿、野狐獵小動物；族群此消彼長（湧現平衡）。
//! ROADMAP 142：死亡餵養生命——獵物死亡釋出乙太微粒；玩家靠近採集得乙太，死亡是循環的一環。
//! ROADMAP 143：物種聚落——各物種有巢穴/聚落與群體防禦，不只人類城。
//!   - 6 個聚落分散世界（2 鳥巢・1 鹿棲地・1 小動物洞穴・1 狼窩・1 狐狸洞）。
//!   - 玩家進入聚落守衛半徑 → 同種動物切換為 Guarding（向威脅靠近，不逃跑）。
//!   - 每個聚落獨立冷卻（90 秒）廣播世界聊天：「🛡️ 野鹿棲地 察覺到入侵者，正在驅離！」
//!
//! 行為規則：
//! - 捕食者進入 HUNT_RADIUS 內偵測到獵物 → Hunting（追獵）。
//! - 追及 KILL_RADIUS 內 → 獵物死亡 + 捕食者進入 Digesting。
//! - 玩家與捕食者都會令獵物 Fleeing；同種獵物見捕食者靠近也一起竄逃（群逃）。
//! - 玩家進入聚落守衛半徑 → 附近同種動物進入 Guarding（向玩家靠近）。
//! - 死亡獵物 ~50 秒後在家附近重生（代表族群新個體）。
//! - 死亡時在原地生成乙太微粒；玩家靠近採集得 CARION_ETHER 乙太（死亡是循環的一環）。
//! - 捕食者每分鐘最多廣播一次捕獵事件，不塞頻道。
//!
//! 效能：全純算術、零 LLM、零 migration、記憶體模式，重啟全重置。

use rand::{Rng, SeedableRng, rngs::StdRng};
// ROADMAP 165：怪物食物鏈需要識別 EnemyKind。
use crate::combat::EnemyKind;

// ─── 常數 ────────────────────────────────────────────────────────────────────

/// 野生動物總數（獵物 18 + 捕食者 4）。
const WILDLIFE_COUNT: usize = 22;

/// 玩家或捕食者靠近多少像素觸發獵物驚逃。
const FLEE_RADIUS: f32 = 180.0;
/// 驚逃速度（像素/秒）。
const FLEE_SPEED: f32 = 200.0;
/// 驚逃計時器（秒）。
const FLEE_DURATION: f32 = 4.5;

/// 閒晃速度（像素/秒）——獵物。
const WANDER_SPEED: f32 = 35.0;
/// 閒晃速度——捕食者（稍快）。
const PRED_WANDER_SPEED: f32 = 52.0;
/// 漫遊半徑。
const WANDER_RADIUS: f32 = 180.0;
const WANDER_TIMER_MIN: f32 = 2.5;
const WANDER_TIMER_MAX: f32 = 7.0;
const REST_TIMER_MIN: f32 = 1.5;
const REST_TIMER_MAX: f32 = 4.5;

/// 返家速度。
const RETURN_SPEED: f32 = 60.0;
/// 距巢穴多近算「到家」。
const HOME_ARRIVE_DIST: f32 = 20.0;

/// 捕食者搜尋獵物的半徑。
const HUNT_RADIUS: f32 = 320.0;
/// 追獵速度。
const HUNT_SPEED: f32 = 155.0;
// ─── ROADMAP 213：孤獵潛行突襲 ────────────────────────────────────────────────
/// 潛行接近速度——遠慢於全速追獵（約其半），讀起來像壓低身子匍匐逼近。
const STALK_SPEED: f32 = 78.0;
/// 撲擊距離——潛近到此距離內即爆衝轉入 Hunting 全速撲殺。
/// 刻意 ＞ FLEE_RADIUS(180)：爆衝恰在獵物即將察覺的剎那發動；亦小於哨兵警戒
/// SENTINEL_FLEE_RADIUS(300)，讓哨兵能在掠食者仍潛行時就先發現。
const POUNCE_RANGE: f32 = 200.0;
/// 潛行耐性上限（秒）——超時仍追不到就放棄返家，不會永遠尾隨。
const STALK_TIMEOUT: f32 = 12.0;
/// 進入此距離觸發擊殺。
const KILL_RADIUS: f32 = 22.0;
/// 追獵超時（秒），超過後放棄。
const HUNT_TIMEOUT: f32 = 18.0;
/// 吃完後消化休息時間。
const DIGEST_DURATION: f32 = 25.0;
/// 獵物死亡後重生秒數。
const PREY_RESPAWN_SECS: f32 = 50.0;
/// 捕獵廣播最短間隔（秒），避免塞頻道。
const KILL_BROADCAST_INTERVAL: f32 = 30.0;

// ─── ROADMAP 143：物種聚落常數 ───────────────────────────────────────────────

/// 守衛速度（像素/秒）——動物向威脅靠近，刻意比逃跑慢，更像「領地巡邏」。
const GUARD_SPEED: f32 = 65.0;
/// 守衛行為持續時間（秒），之後恢復正常。
const GUARD_DURATION: f32 = 12.0;
/// 每個聚落的廣播冷卻（秒）——避免玩家徘徊時刷屏。
const COLONY_THREAT_COOLDOWN: f32 = 90.0;
/// 進入守衛狀態的範圍倍率（相對於 guard_radius）。
const COLONY_ACTIVATE_MULTIPLIER: f32 = 1.8;

// ─── ROADMAP 144：人類↔物種關係常數 ─────────────────────────────────────────

/// 敵視物種主動偵測玩家並攻擊的半徑（像素）。
const HOSTILE_DETECT_RADIUS: f32 = 200.0;
/// 敵視守衛動物近身攻擊觸發距離（像素）。
const HOSTILE_ATTACK_REACH: f32 = 35.0;
/// 敵視野生動物的攻擊傷害（HP）。
const HOSTILE_ATTACK_DAMAGE: u32 = 2;
/// 敵視攻擊後動物的冷卻（秒）——映射成 guard_timer 重設值。
const HOSTILE_ATTACK_COOLDOWN: f32 = 3.0;
/// 友善物種（attitude ≥ 此值）不把玩家加入逃跑威脅清單。
const FRIENDLY_ATTITUDE: i32 = 65;
/// 敵視物種（attitude < 此值）會主動攻擊玩家。
const HOSTILE_ATTITUDE: i32 = 25;

// ─── ROADMAP 142：乙太微粒常數 ───────────────────────────────────────────────

/// 乙太微粒採集有效距離（像素）。
pub const CARION_COLLECT_RADIUS: f32 = 80.0;
/// 每顆乙太微粒給予的乙太數量。
pub const CARION_ETHER: u32 = 4;
/// 乙太微粒存在時長（秒），逾時自動消失。
const CARION_ORB_TTL: f32 = 90.0;
/// 同時存在乙太微粒的上限（防止無限堆積）。
const MAX_CARION_ORBS: usize = 8;

// ─── ROADMAP 205：餵食馴養 ───────────────────────────────────────────────────
// 反覆餵食「同一隻」野生動物，會累積個體親近度（0~1）。
// 親近度達 TAME_FAMILIARITY 後該隻動物被「馴養」：不再把玩家視為威脅（不逃跑），
// 玩家靠近時溫順地走向你、保持舒適距離（彷彿跟著你）。牠仍會逃離捕食者/獵食怪物
// （信任的是你、不是狼）。親近度隨時間緩慢衰減、死亡歸零——是一段需要維繫的關係。

/// 親近度上限（餵食累積的封頂）。
const MAX_FAMILIARITY: f32 = 1.0;
/// 個體親近度達此值即視為「已馴養」。刻意低於上限，留出緩衝——餵滿後即使緩慢衰減，
/// 也要好一陣子才會掉出馴養狀態（不會因每幀微小衰減就立刻「退馴」）。
const TAME_FAMILIARITY: f32 = 0.8;
/// 每餵食一次提升的親近度（需數次餵食才馴養，過程才有溫度）。
const FEED_FAMILIARITY_GAIN: f32 = 0.25;
/// 親近度每秒自然衰減（很慢——約 30 分鐘從滿值歸零，關係需偶爾維繫但不易斷）。
const FAMILIARITY_DECAY_PER_SEC: f32 = 1.0 / 1800.0;
/// 馴養動物「察覺到附近玩家」而走向他的範圍（像素）。
const FOLLOW_RANGE: f32 = 260.0;
/// 馴養動物跟隨時與玩家保持的舒適距離（像素）——更近就停下，不黏在腳邊。
const FOLLOW_COMFORT_DIST: f32 = 60.0;
/// 馴養動物走向玩家的速度（像素/秒）——比逃跑慢，溫順小跑。
const FOLLOW_SPEED: f32 = 60.0;

// ─── ROADMAP 206：群聚結伴 ───────────────────────────────────────────────────
// 同種野生動物（獵物）漫遊時，選下一個閒晃目標會朝「附近同種夥伴的平均位置」
// 拉一把，於是鬆散成群移動：草原上的野鹿三兩成群、野鳥成簇飄移，
// 世界不再是一盤各走各的散點。純啟發式、零 LLM、零持久化、無 migration。
// 捕食者（狼/狐）刻意維持獨來獨往（更顯孤狼氣場），不參與群聚。

/// 尋找同種群聚夥伴的半徑（像素）——只看這個範圍內的同種存活獵物算「一群」。
const HERD_RADIUS: f32 = 280.0;
/// 選新漫遊目標時朝群體中心混合的比例（0=純隨機家附近、1=直奔群體中心）。
/// 刻意取中段：既明顯成群、又保留各自散布，不會擠成一個點。
const HERD_PULL: f32 = 0.5;

// ─── ROADMAP 207：幼獸誕生（族群繁衍）─────────────────────────────────────────
// 承接 206（群聚結伴）：當同種獵物成群、且周遭安穩（附近沒有捕食者）一段時間，
// 群體會孕育出一隻「幼獸」——在群體中心誕生、體型小小的、隨時間慢慢長大成成體。
// 於是世界的獸群不再是固定 18 隻散點，而會從稀疏慢慢繁衍成興旺的家族（封頂避免暴增）。
// 純啟發式、零 LLM、零持久化、無 migration、記憶體模式（重啟回到初始族群）。
// 捕食者不繁衍（維持稀少、孤獨的掠食者氣場），只有獵物會。

/// 構成「可繁衍的一群」所需的同種成年存活個體數（含被選為基準的那隻）。
const BREED_HERD_MIN: usize = 2;
/// 判定群聚與安穩的半徑（像素）——同種成年彼此聚在此範圍內、且範圍稍大內無捕食者。
const BREED_RADIUS: f32 = 240.0;
/// 捕食者干擾半徑（像素）：群體中心此範圍內有捕食者就停止孕育（緊張的群體不生育）。
const BREED_DISTURB_RADIUS: f32 = 360.0;
/// 孕育所需的累計「安穩成群」秒數，達標即誕生一隻幼獸。刻意偏長，讓繁衍是緩慢、難得的成長。
const BREED_THRESHOLD_SECS: f32 = 90.0;
/// 幼獸長成成體所需秒數（期間體型由小漸大）。
const MATURE_DURATION_SECS: f32 = 120.0;
/// 剛誕生幼獸的相對體型（成體為 1.0）——前端據此把幼獸畫小一號。
const JUVENILE_MIN_SCALE: f32 = 0.45;

// ─── ROADMAP 208：幼獸依偎母獸（親子跟隨）───────────────────────────────────────
// 承接 207（幼獸誕生）：剛出生的幼獸不再各自亂晃，而會主動依偎、跟隨最近的同種成體
// （像小鹿緊跟母鹿）——平靜時黏在成體身邊小跑、保持依偎距離，成體漫遊時被牽著走。
// 受掠食者驚擾時威脅優先（仍會逃命）、長成成體後自然脫離（不再是幼獸）。
// 純啟發式、零 LLM、零協議改動（位置本就每幀廣播）、無新狀態欄位。
/// 幼獸尋找可依偎成體的範圍（像素）——只在這個範圍內找最近的同種成體當「母獸」。
const NURSE_RANGE: f32 = 320.0;
/// 幼獸依偎時與成體保持的舒適距離（像素）——更近就停，不疊在一起。
const NURSE_COMFORT_DIST: f32 = 36.0;
/// 幼獸依偎跟隨的速度（像素/秒）——略快於閒晃，才追得上緩緩漫遊的成體。
const NURSE_SPEED: f32 = 48.0;

// ─── ROADMAP 215：幼獸嬉戲（在母獸身邊蹦跳玩耍）──────────────────────────────────
// 承接 207（幼獸誕生）＋ 208（幼獸依偎母獸）：幼獸不再只是靜靜貼著母獸，而會在白天、
// 平靜、已依偎到母獸身邊時，偶爾在媽媽周圍小範圍蹦跳玩耍（頭頂浮 ✨），玩一段就回到依偎。
// 嬉戲落點每次隨機（圍著母獸、玩不離媽媽），受威脅一律優先逃命（威脅永遠優先）、夜間只依偎
// 不玩耍。純啟發式、零 LLM、零協議改動（state 本就每幀廣播）、無新狀態欄位（落點/計時隨變體攜帶）。
/// 嬉戲落點離母獸的最大半徑（像素）——蹦跳只在媽媽身邊小範圍，玩不遠、不脫群、不入險。
const FROLIC_RADIUS: f32 = 70.0;
/// 蹦跳速度（像素/秒）——快於依偎跟隨，幼獸玩起來活潑蹦跳。
const FROLIC_SPEED: f32 = 64.0;
/// 視為「已蹦到落點」的判定距離（像素）——到了就結束這一段嬉戲、回母獸身邊依偎。
const FROLIC_REACH: f32 = 12.0;
/// 單段嬉戲（朝一個蹦跳落點）的最短/最長持續秒數——到期未達落點也收尾回依偎，不會玩個沒完。
const FROLIC_DURATION_MIN: f32 = 1.2;
const FROLIC_DURATION_MAX: f32 = 2.6;
/// 已依偎到母獸身邊、且白天平靜時，本幀起一段嬉戲的機率——偏低，讓幼獸玩玩停停、多數時候靜靜依偎。
const FROLIC_PROB: f32 = 0.025;

// ─── ROADMAP 209：驚群炸開（恐慌連鎖）─────────────────────────────────────────
// 承接 206（群聚結伴）：獸群會聚在一起，但危險來時過去卻是「各跑各的」——只有
// 直接看到捕食者、且在 FLEE_RADIUS 內的那幾隻會逃，旁邊沒看到的同伴照樣閒晃。
// 真正的獸群不是這樣：一隻驚跳、恐慌就像漣漪般傳遍全群，整群朝同方向一起炸開奔逃。
// 本切片補上這塊：附近同種夥伴正在逃竄、而自己附近沒有「直接威脅」時，也被恐慌
// 感染、朝同伴逃竄的方向一起竄逃。恐慌每 tick 只傳一圈（吃逃竄快照），於是看起來像
// 一波由威脅源向外擴散的炸群。純啟發式、零 LLM、零協議改動（state 本就每幀廣播）。
/// 恐慌連鎖半徑（像素）——同種夥伴在此範圍內逃竄，會把恐慌傳染給自己。
/// 略小於群聚半徑（HERD_RADIUS 280），讓恐慌只在「真的成群」的近鄰間擴散。
const ALARM_RADIUS: f32 = 220.0;
/// 被感染的驚逃時長（秒）——略短於直接目擊威脅的 FLEE_DURATION，二手恐慌較快平復。
const ALARM_FLEE_DURATION: f32 = 3.0;

// ─── ROADMAP 210：晝夜作息 ────────────────────────────────────────────────────
// 把既有晝夜系統接進生態：晝行的獵物入夜歸巢沉睡、夜行的掠食者入夜狩獵範圍更廣。
// 純啟發式、零 LLM、零協議改動（state 本就每幀廣播；夜間 is_night 由 game.rs 傳入）。
/// 夜間掠食者狩獵搜尋半徑倍率——夜行獵手入夜後覓食範圍更廣（與 enemy_field 夜間加成同調）。
const NIGHT_HUNT_RADIUS_MULT: f32 = 1.4;
/// 夜間歸巢沉睡的休息時長（秒）——遠長於白天的 REST_TIMER，讓晝行獵物安睡到天明。
/// 此值在「平靜夜晚」期間不會被遞減（夜眠分支不走 tick_idle），故獵物會一路睡到白天才甦醒。
const NIGHT_SLEEP_REST_SECS: f32 = 600.0;

// ─── ROADMAP 211：白晝吃草 ────────────────────────────────────────────────────
// 承接 210（晝夜作息）：晝行獵物白天抵達漫遊目標後，有機率低頭吃草（原地不動數秒、頭頂浮
// 🌿）而非單純休息——補上「白天醒著做什麼」這層，與夜眠 💤 對成完整的晝夜作息。
// 純啟發式、零 LLM、零協議改動（state 本就每幀廣播；新增的 grazing 字串沿用 state_str）。
// 只有平靜的晝行獵物白天才吃草：夜間/掠食者一律傳 graze_prob=0（行為與切片前逐位元一致）。
/// 平靜的晝行獵物白天抵達漫遊目標時轉入「吃草」（而非單純休息）的機率。
const GRAZE_PROB: f32 = 0.45;
/// 一次吃草的最短／最長時長（秒）——原地低頭覓食數秒後再回漫遊。
const GRAZE_DURATION_MIN: f32 = 3.0;
const GRAZE_DURATION_MAX: f32 = 7.0;

// ─── ROADMAP 212：群體警戒哨 ──────────────────────────────────────────────────
// 承接 211（白晝吃草）+ 209（驚群炸開）：白天成群的成體獵物中，由群內 id 最小那隻擔任
// 「哨兵」——不低頭吃草，而是抬頭放哨（頭頂浮 👀）。哨兵的警戒範圍放大
// （SENTINEL_FLEE_RADIUS > 一般 FLEE_RADIUS），比埋頭吃草的同伴更早察覺逼近的危險；牠一
// 旦炸群逃竄，經 209 的恐慌感染，整群隨之一起炸開奔散。於是「一隻站崗、其餘安心吃草，哨兵
// 先發現狼影、全群跟著逃」這幕在野地自然湧現。純啟發式、零 LLM、零協議改動（新增的 watching
// 字串沿用 state_str；哨兵去中心地由「群內最小 id」推定，每群恰一隻、穩定不抖動）。
/// 判定哨兵時計入同群夥伴的半徑（略小於 HERD_RADIUS，只罩住「真的成群」的近鄰）。
const SENTINEL_HERD_RADIUS: f32 = 220.0;
/// 成群門檻：半徑內同種成體（含自己）達此數才設哨兵——孤獸不必放哨。
const SENTINEL_MIN_HERD: usize = 2;
/// 哨兵的警戒（逃竄觸發）半徑——放大版的 FLEE_RADIUS(180)，使其比同伴更早發現威脅。
const SENTINEL_FLEE_RADIUS: f32 = 300.0;
/// 一次站崗放哨的最短／最長時長（秒）——抬頭警戒數秒後跟著群體挪步、再重新站崗。
const WATCH_DURATION_MIN: f32 = 4.0;
const WATCH_DURATION_MAX: f32 = 8.0;

// ─── ROADMAP 214：母獸護幼 ────────────────────────────────────────────────────
// 承接 208（幼獸依偎母獸）+ 213（掠食者潛行突襲）：當掠食者鎖定（潛行/追獵）一隻幼獸時，
// 離那隻幼獸最近的同種成體會「挺身護幼」——不逃反而轉身朝掠食者衝去（頭頂浮 🛡），把狼／狐
// 逼到威嚇距離內就驅退牠（掠食者放棄、退走）。於是「狼悄悄逼近落單的小鹿、母鹿猛地衝出擋在
// 中間把狼趕走」這幕在野地自然湧現——掠食（213）終於有了「反捕食」這一側的對偶。純啟發式、
// 零 LLM、零協議改動（新增的 defending 字串沿用 state_str）；幼獸本就在 prey_snap 裡（會被獵），
// 故本切片不動掠食者目標選擇的平衡，只新增成體的護幼反應。
/// 護幼觸發半徑——成體會為「DEFEND_GUARD_RADIUS 內、且自己是最近成體」的受脅同種幼獸挺身。
const DEFEND_GUARD_RADIUS: f32 = 240.0;
/// 護幼衝刺速度——介於漫遊(35)與逃竄(200)之間，比掠食者潛行(78)快、足以及時擋在幼獸與狼之間。
const DEFEND_SPEED: f32 = 150.0;
/// 威嚇半徑——護幼成體逼進掠食者此距離內，掠食者即放棄狩獵、退走（大於 KILL_RADIUS，故狼來不
/// 及咬到幼獸就先被趕跑；母獸自身非掠食者目標、不會被咬，護幼安全）。
const INTIMIDATE_RADIUS: f32 = 90.0;

// ─── ROADMAP 216：成體相依理毛（herd social grooming）───────────────────────────
// 承接 206（群聚結伴）+ 215（幼獸嬉戲）：過去成體獵物白天只會吃草／休息／站崗，彼此之間
// 沒有任何「親暱互動」——群只是聚在一起的個體。本切片補上群居動物最溫柔的一塊：成體在白天
// 平靜歇息的當口，偶爾轉向身邊的同種成體夥伴互相理毛（頭頂浮 💕），數秒後再起身。於是同一
// 片草原，你會看到兩頭鹿安靜地依偎著彼此梳理——群第一次有了「成員之間的羈絆」，而不只是
// 一群各自吃草的點。幼獸嬉戲（215）是「孩子繞著母親玩」，理毛則是「大人之間互相照拂」，兩者
// 把生態的「親密」補成完整一對。純啟發式、零 LLM、零 tick 簽名改動、零協議改動（新增的
// grooming 字串沿用 state_str；夥伴座標／計時隨狀態變體攜帶，無新欄位）、記憶體模式。
/// 理毛夥伴半徑（像素）——身邊有同種成體在此近距離內，歇息時才可能轉去互相理毛（親暱貼近）。
const GROOM_RADIUS: f32 = 60.0;
/// 視為「已理毛中」的單段最短／最長時長（秒）——靜靜替彼此梳理數秒後再起身漫遊。
const GROOM_DURATION_MIN: f32 = 3.0;
const GROOM_DURATION_MAX: f32 = 6.5;
/// 成體在白天歇息、且身邊有同種夥伴時，本幀轉入理毛的機率——偏低，讓理毛是偶爾的溫柔片刻、
/// 而非時時黏著（多數時候仍照常吃草／休息）。
const GROOM_PROB: f32 = 0.05;

// ─── ROADMAP 217：掠食者夜嚎（predator night howl）─────────────────────────────
// 承接 210（晝夜作息）+ 213（孤獵潛行）：過去獵物入夜歸巢沉睡、夜晚成了「獵物缺席」的安靜時段，
// 但夜其實是掠食者的主場——牠們只會默默巡遊獵食，從不發出聲音，夜的氛圍裡少了最標誌性的一筆：
// 狼嗥。本切片補上掠食者夜間的嗓音：夜裡無獵可追的平靜空檔，狼／狐偶爾停下腳步、仰首長嚎
// （頭頂浮 🌙），數秒後再回到巡遊。於是當你夜行荒野，遠處會傳來一聲聲嗥叫——夜第一次有了
// 掠食者的存在感，世界在入夜後不再只是「獵物睡了」、而是「換掠食者的世界醒著」。純夜間氛圍
// 行為：不移動、不群聚（守掠食者「獨來獨往」設定，不與 206 群聚混淆）、不改狩獵優先（一發現
// 獵物即改去獵殺）。純啟發式、零 LLM、零 tick 簽名改動、零協議改動（新增的 howling 字串沿用
// state_str；計時隨狀態變體攜帶，無新欄位）、記憶體模式。
/// 掠食者在夜間歇息、且附近無獵物可追時，本幀仰首長嚎的機率——偏低，讓嗥叫是夜裡偶爾的一聲、
/// 而非時時嚎個不停（多數時候仍照常巡遊獵食）。
const HOWL_PROB: f32 = 0.02;
/// 一聲長嚎的最短／最長時長（秒）——仰首嚎數秒後再低頭回到巡遊。
const HOWL_DURATION_MIN: f32 = 2.0;
const HOWL_DURATION_MAX: f32 = 3.5;

// ─── ROADMAP 218：群嚎呼應（howl chorus / contagion）─────────────────────────
// 承接 217（掠食者夜嚎）：217 讓夜裡無獵可追的狼／狐偶爾仰首獨嚎（🌙），但每一聲都是孤零零的——
// 一隻嚎完、四下無回應，少了狼群最攝人的那一幕：一聲起，四方應，此起彼落連成一片。本切片補上
// 「呼應」：當一隻掠食者開始長嚎，附近同樣夜裡歇息、無獵可追的同類「聽見」了，會被牽動跟著仰首
// 接嚎——一聲引發一片，嚎叫像漣漪般在夜裡的荒野間傳開（就像 209 驚群把恐慌一圈圈傳染，這裡把
// 嚎聲一圈圈傳染）。於是夜行荒野時，遠近的狼影會此起彼落地對嚎成一支夜曲，而非各嚎各的。
// 純啟發式、零 LLM、零 tick 簽名改動、零協議改動（仍走既有 howling 狀態與 state_str，無新欄位）、
// 記憶體模式。與 217 的區隔：217＝「自發起頭的第一聲」（低機率 HOWL_PROB 隨機觸發），
// 218＝「聽見後的接力」（被附近嚎聲牽動而跟嚎）——一個點火、一個傳染，合起來才是「群嚎」。
/// 一聲長嚎能傳多遠、牽動多遠外的同類接嚎（世界座標像素）——比狩獵搜尋半徑更遠，嚎聲傳得比
/// 腳步聲遠，讓散在夜野各處的狼也聽得到、應得上。
const HOWL_HEAR_RADIUS: f32 = 460.0;
/// 「聽見」附近嚎聲時，本幀跟著接嚎的機率——偏高（嚎聲很有感染力），但不設 1.0：留一點隨機，
/// 讓接力是錯落地一隻接一隻（而非同幀整齊齊嚎），讀起來更像真實的此起彼落。又因新接嚎者本幀
/// 不在「起始嚎聲快照」裡，故牽動每 tick 只向外擴一圈——嚎聲像漣漪般逐圈傳開，不會瞬間全嚎。
const HOWL_JOIN_PROB: f32 = 0.5;

// ─── ROADMAP 219：破曉甦醒伸展（dawn waking stretch）──────────────────────────
// 承接 210（晝夜作息：晝行獵物入夜歸巢沉睡 💤）：210 讓鹿群入夜後一隻隻窩回家裡安睡，補上了
// 「夜」的作息；但牠們的「破曉」卻是瞬間的——天一亮，沉睡的獵物上一幀還癱在家裡、下一幀已直接
// 起身漫遊，少了動物甦醒最自然的那一拍：先伸個懶腰。本切片補上這層：天明喚醒夜眠的晝行獵物時，
// 不再讓牠立刻起步，而是先原地伸展一小段（頭頂浮一輪緩緩升起的朝陽 🌅、像睜眼伸懶腰），伸展完
// 才起身投入新一天的閒晃。於是同一片草原，晨光鋪上草地時，鹿群會一隻隻先在家門口舒展身子、再
// 慢慢散開吃草——「日出而作」第一次有了「甦醒」這一拍，與夜眠 💤 對成完整的晝夜起落。純啟發式、
// 零 LLM、零 tick 簽名改動、零協議改動（新增的 waking 字串沿用 state_str；計時隨狀態變體攜帶，
// 無新欄位）、記憶體模式。威脅永遠優先：伸展中若有掠食者逼近，立刻中斷改逃竄（睡醒遇險先逃命）。
/// 破曉甦醒的伸展時長（秒）——天明喚醒夜眠者後，原地舒展身子數秒再起身漫遊。偏短：只是醒來的
/// 一拍過渡，不是又一段休息（過長會讓整群鹿天亮後還賴在原地，反而失了「晨起散開」的生氣）。
const WAKE_DURATION_MIN: f32 = 1.5;
const WAKE_DURATION_MAX: f32 = 3.0;

// ─── ROADMAP 220：鳥群振翅升空盤旋（bird flock takes flight）─────────────────
// 承接 206（群聚結伴）與全套獵物作息（207~219）：野鳥（WildBird）至今行為與野鹿、小動物幾乎
// 一樣——在地上走走停停、吃草、逃竄，唯獨少了鳥最本該有的那一面：飛。本切片給野鳥補上專屬的
// 「振翅升空」：白天平靜時，野鳥偶爾會整群一起拍翅升空、繞著群心緩緩盤旋一陣，再陸續降落回地面
// 閒晃。升空像 218 群嚎一樣會「呼應」——一隻起飛，附近同類被牽動跟著飛起，整群一齊盤旋成一片
// （而非各飛各的）。威脅永遠優先：盤旋中若掠食者逼近，立刻降下逃竄（飛行是悠閒的盤旋、不是逃命
// 手段）。只有野鳥會飛（鹿／小動物不適用），生態第一次有了「物種專屬行為」與「空中維度」。純
// 啟發式、零 LLM、零 tick 簽名改動、零協議改動（新增的 flying 字串沿用 state_str；計時與盤旋角度
// 隨狀態變體攜帶，無新欄位）、記憶體模式。前端依 state==="flying" 把鳥往上抬起、地面留一抹投影，
// 讀起來就是「飛在空中」。
/// 平靜的野鳥本幀自發振翅升空的機率——偏低，讓起飛是白天偶爾的一陣騷動、而非時時在飛。
const FLIGHT_PROB: f32 = 0.012;
/// 一段盤旋的最短／最長時長（秒）——升空繞圈數秒後再降落回地面閒晃。
const FLIGHT_DURATION_MIN: f32 = 3.0;
const FLIGHT_DURATION_MAX: f32 = 6.0;
/// 升空能「帶動」多遠外的同種野鳥跟著飛起（世界座標像素）——一隻起飛、近旁同類呼應。
const FLIGHT_HEAR_RADIUS: f32 = 320.0;
/// 「看見」附近同類升空時，本幀跟著飛起的機率——偏高（鳥群起飛極富感染力），但不設 1.0：留一點
/// 隨機，讓起飛是錯落地一隻接一隻；又因新起飛者本幀不在「起始升空快照」裡，故牽動每 tick 只向外
/// 擴一圈——升空像漣漪般逐圈傳開，整群在一兩秒內陸續拍翅而起，而非同幀整齊齊飛。
const FLIGHT_JOIN_PROB: f32 = 0.6;
/// 盤旋半徑（世界座標像素）——繞著群心（無群則繞自家）轉圈的圈半徑。
const FLIGHT_CIRCLE_RADIUS: f32 = 26.0;
/// 盤旋角速度（弧度／秒）——每秒繞行的角度，決定盤旋快慢。
const FLIGHT_ANGULAR_SPEED: f32 = 1.6;

// ─── ROADMAP 221：晝日鳥鳴呼應（daytime bird song chorus / contagion）─────────
// 承接 218（群嚎呼應）：218 把夜的氛圍從「偶爾一聲孤嗥」補成「此起彼落的群嚎」，讓夜行荒野頭皮
// 發麻；可白天的草原卻反而更顯安靜——鳥群只是默默低頭吃草、無聲升空，少了真實野地白天最有感染力
// 的那層底噪：鳥鳴。本切片把 218 的整套「呼應」手法從夜的狼嚎搬到晝的鳥鳴：白天平靜時，野鳥偶爾
// 停下啁啾一小段（頭頂浮 🎵），鳴聲會「傳染」——一隻起鳴，附近同類「聽見」了被牽動跟著啁啾，鳴聲
// 像漣漪般逐圈在草原上傳開，遠近的鳥此起彼落地對鳴成一片晨間合唱。與 218 對成完整的晝夜聲景：夜有
// 狼嚎（🌙）、晝有鳥鳴（🎵）。純啟發式、零 LLM、零 tick 簽名改動、零協議改動（新增的 chirping 字串
// 沿用 state_str；計時隨狀態變體攜帶，無新欄位）、記憶體模式。與 220 升空的區隔：啁啾是原地出聲、
// 不離地、不盤旋的另一種白天行為（鳥可站著鳴、也可飛起，兩者互斥）。威脅永遠優先：啁啾中若有掠食者
// 逼近，立刻收聲逃竄。
/// 平靜的野鳥本幀自發起鳴的機率——偏低，讓起鳴是白天偶爾的一聲、而非時時鳴個不停（多數時候仍照常
/// 閒晃吃草）。對應 218 的 HOWL_PROB，是「自發起頭的第一聲」。
const CHIRP_PROB: f32 = 0.015;
/// 一段啁啾的最短／最長時長（秒）——仰首鳴數秒後再低頭回到閒晃。比狼嚎略短：鳥鳴輕快、一串就停。
const CHIRP_DURATION_MIN: f32 = 1.5;
const CHIRP_DURATION_MAX: f32 = 3.0;
/// 一段啁啾能傳多遠、牽動多遠外的同類跟鳴（世界座標像素）——比升空牽動半徑（FLIGHT_HEAR_RADIUS）
/// 更遠，鳴聲傳得比拍翅遠，讓散在草原各處的鳥也聽得到、應得上。
const CHIRP_HEAR_RADIUS: f32 = 420.0;
/// 「聽見」附近啁啾時，本幀跟著起鳴的機率——偏高（鳥鳴很有感染力），但不設 1.0：留一點隨機，讓
/// 接力是錯落地一隻接一隻；又因新起鳴者本幀不在「起始啁啾快照」裡，故牽動每 tick 只向外擴一圈——
/// 鳴聲像漣漪般逐圈傳開，不會瞬間全鳴（對應 218 的 HOWL_JOIN_PROB，是「聽見後的接力」）。
const CHIRP_JOIN_PROB: f32 = 0.45;

// ─── ROADMAP 222：小動物捧食啃咬（critter sits up to nibble）──────────────────
// 承接 220（鳥群振翅升空）開的「物種專屬行為」這條線：220 給了野鳥專屬的「飛」，但生態裡的另一
// 種小傢伙——小動物（SmallCritter，松鼠般的齧齒小獸）——行為卻仍與野鹿幾乎一模一樣：在地上走走停停、
// 低頭吃草、逃竄。牠少了松鼠最招牌、最惹人憐愛的那一幕：直起身子、捧著找到的堅果／種子，捧在胸前
// 一小口一小口地啃。本切片給小動物補上專屬的「捧食啃咬」：白天平靜時，小動物漫步到定點後偶爾不只是
// 發呆或低頭吃草，而會坐起來捧著食物啃一小段（頭頂浮一顆 🌰），啃完再起身閒晃。與 211 白晝吃草的
// 區隔：吃草（🌿）是所有晝行獵物低頭啃地上的草，啃咬（🌰）是小動物專屬、直起身捧食而啃的另一種姿態；
// 兩者互斥（同一隻同一刻只會其一）。與鳥的飛／鳴（220/221）不同，松鼠覓食是各顧各的、不傳染，故
// 本切片不做「呼應」（無快照、無接力）——只是一隻隻自顧自地坐起來啃，更像真實的松鼠。純啟發式、
// 零 LLM、零 tick 簽名改動、零協議改動（新增的 nibbling 字串沿用 state_str；計時隨狀態變體攜帶，
// 無新欄位）、記憶體模式。威脅永遠優先：啃到一半若掠食者／玩家逼近，立刻丟食逃竄。
/// 平靜的小動物本幀坐起來捧食啃咬的機率——偏低，讓啃咬是白天偶爾的一小段、而非時時在啃（多數時候
/// 仍照常閒晃／吃草）。
const NIBBLE_PROB: f32 = 0.03;
/// 一段啃咬的最短／最長時長（秒）——坐起來捧食啃數秒後再起身回到閒晃。
const NIBBLE_DURATION_MIN: f32 = 2.0;
const NIBBLE_DURATION_MAX: f32 = 5.0;

/// 三種會繁衍的獵物（捕食者不列入）。
const BREEDING_KINDS: [WildlifeKind; 3] =
    [WildlifeKind::WildBird, WildlifeKind::WildDeer, WildlifeKind::SmallCritter];

/// 每種獵物在世界中的個體數上限（含存活與待重生者）——封頂避免族群無限暴增、保護效能。
/// 初始數量：野鳥 6、野鹿 5、小動物 7；各留約 +3 的繁衍成長空間。
fn species_cap(kind: WildlifeKind) -> usize {
    match kind {
        WildlifeKind::WildBird     => 9,
        WildlifeKind::WildDeer     => 8,
        WildlifeKind::SmallCritter => 10,
        // 捕食者不繁衍，給個與初始相同的封頂（永不觸發）。
        WildlifeKind::WildWolf | WildlifeKind::WildFox => 2,
    }
}

// ─── 種類與營養階 ────────────────────────────────────────────────────────────

/// 野生動物種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WildlifeKind {
    WildBird,
    WildDeer,
    SmallCritter,
    /// 捕食者：獵食野鹿。
    WildWolf,
    /// 捕食者：獵食小動物。
    WildFox,
}

/// 食物鏈層級。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrophicLevel {
    Prey,
    Predator,
}

impl WildlifeKind {
    pub fn display_name(self) -> &'static str {
        match self {
            WildlifeKind::WildBird     => "野鳥",
            WildlifeKind::WildDeer     => "野鹿",
            WildlifeKind::SmallCritter => "小動物",
            WildlifeKind::WildWolf     => "野狼",
            WildlifeKind::WildFox      => "野狐",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            WildlifeKind::WildBird     => "wild_bird",
            WildlifeKind::WildDeer     => "wild_deer",
            WildlifeKind::SmallCritter => "small_critter",
            WildlifeKind::WildWolf     => "wild_wolf",
            WildlifeKind::WildFox      => "wild_fox",
        }
    }

    pub fn trophic_level(self) -> TrophicLevel {
        match self {
            WildlifeKind::WildWolf | WildlifeKind::WildFox => TrophicLevel::Predator,
            _ => TrophicLevel::Prey,
        }
    }

    /// 此捕食者的獵食對象（None 表示非捕食者）。
    pub fn hunts(self) -> Option<WildlifeKind> {
        match self {
            WildlifeKind::WildWolf => Some(WildlifeKind::WildDeer),
            WildlifeKind::WildFox  => Some(WildlifeKind::SmallCritter),
            _ => None,
        }
    }
}

/// ROADMAP 165：怪物物種食物鏈配對——定義哪種怪物主動獵食哪種野生動物。
/// 三對配對（食性與分佈合理）：
///   - 乙太鬼火 → 野鳥（光靈追逐飛行生物）
///   - 蕈菇潛行者 → 小動物（森林潛行者獵食小型獵物）
///   - 廢鐵無人機 → 野鹿（機械無人機追蹤大型目標）
pub fn monster_hunts_wildlife(kind: EnemyKind) -> Option<WildlifeKind> {
    match kind {
        EnemyKind::EtherWisp       => Some(WildlifeKind::WildBird),
        EnemyKind::MushroomStalker => Some(WildlifeKind::SmallCritter),
        EnemyKind::ScrapDrone      => Some(WildlifeKind::WildDeer),
        _                          => None,
    }
}

// ─── ROADMAP 142：乙太微粒 ───────────────────────────────────────────────────

/// 獵物死亡時釋出的乙太微粒——死亡是循環的一環。
#[derive(Debug, Clone)]
pub struct CarrionOrb {
    pub id: u32,
    pub x: f32,
    pub y: f32,
    pub ttl: f32,
}

// ─── ROADMAP 143：物種聚落 ───────────────────────────────────────────────────

/// 物種聚落——各物種的巢穴/棲地，有領地守衛行為。
#[derive(Debug, Clone)]
pub struct Colony {
    pub id: u32,
    pub kind: WildlifeKind,
    /// 聚落顯示名稱（繁中）。
    pub name: &'static str,
    pub cx: f32,
    pub cy: f32,
    /// 守衛半徑（像素）——玩家進入此範圍觸發群體防禦。
    pub guard_radius: f32,
}

/// 給協議層用的聚落視圖（靜態資料，每幀隨快照廣播）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ColonyView {
    pub id: u32,
    pub kind: String,
    pub name: String,
    pub cx: f32,
    pub cy: f32,
    pub guard_radius: f32,
}

// ─── 事件 ────────────────────────────────────────────────────────────────────

pub enum WildlifeEvent {
    /// 捕食者成功捕獵，應廣播至全服聊天。
    Kill {
        predator_kind: WildlifeKind,
        prey_kind: WildlifeKind,
        x: f32,
        y: f32,
    },
    /// ROADMAP 143：聚落偵測到入侵者，應廣播至全服聊天。
    ColonyThreatened {
        colony_name: &'static str,
        cx: f32,
        cy: f32,
    },
    /// ROADMAP 144：敵視物種守衛攻擊玩家——近身時對附近玩家造成傷害。
    /// 外層（game.rs）應找出 near_x/near_y 附近的玩家並扣血。
    WildlifeAttack {
        attacker_kind: WildlifeKind,
        near_x: f32,
        near_y: f32,
        damage: u32,
    },
    /// ROADMAP 165：怪物成功獵殺野生動物——應廣播至全服聊天並已生成乙太微粒。
    MonsterHunted {
        monster_kind: EnemyKind,
        wildlife_kind: WildlifeKind,
        x: f32,
        y: f32,
    },
    /// ROADMAP 207：安穩成群的獵物孕育出一隻幼獸——應廣播至全服聊天（低頻、療癒向）。
    Born {
        kind: WildlifeKind,
        x: f32,
        y: f32,
    },
}

// ─── 狀態 ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum WildlifeState {
    Wandering { target_x: f32, target_y: f32, wander_timer: f32 },
    Resting { rest_timer: f32 },
    Fleeing { vx: f32, vy: f32, flee_timer: f32 },
    Returning,
    /// ROADMAP 213：孤獵潛行——掠食者鎖定獵物後壓低身子緩緩潛近（頭頂浮 🐾），
    /// 逼近到撲擊距離才爆衝轉入 Hunting；stalk_timer 耗盡仍追不到就放棄返家。
    Stalking { target_id: u32, stalk_timer: f32 },
    /// 捕食者正在追獵指定 ID 的獵物。
    Hunting { target_id: u32, hunt_timer: f32 },
    /// 捕食者吃完後消化休息。
    Digesting { timer: f32 },
    /// ROADMAP 211：白晝吃草——平靜的晝行獵物白天原地低頭覓食數秒（頭頂浮 🌿）。
    Grazing { graze_timer: f32 },
    /// ROADMAP 212：群體警戒哨——成群獵物中的哨兵抬頭放哨（頭頂浮 👀），數秒後跟群挪步再站崗。
    Watching { watch_timer: f32 },
    /// ROADMAP 143：聚落守衛——動物向入侵玩家靠近，不逃跑。
    Guarding { threat_x: f32, threat_y: f32, guard_timer: f32 },
    /// ROADMAP 214：母獸護幼——成體不逃反而朝威脅幼獸的掠食者衝去（頭頂浮 🛡），逼到威嚇距離把牠趕走。
    /// 每幀依當下受脅幼獸即時重算衝刺方向，故狀態本身不需攜帶座標（無資料的單元變體）。
    Defending,
    /// ROADMAP 215：幼獸嬉戲——已依偎到母獸身邊的幼獸在媽媽周圍蹦跳玩耍（頭頂浮 ✨）。
    /// 朝當前蹦跳落點 (hop_x, hop_y) 蹦去，到達或 frolic_timer 耗盡就回到依偎（下一幀再決定要不要再玩）。
    Frolicking { hop_x: f32, hop_y: f32, frolic_timer: f32 },
    /// ROADMAP 216：成體相依理毛——白天歇息時轉向身邊同種成體夥伴互相理毛（頭頂浮 💕）。
    /// 原地不動（不更新座標）、groom_timer 倒數，到期就回到漫遊；威脅一旦逼近一律優先逃竄。
    Grooming { groom_timer: f32 },
    /// ROADMAP 217：掠食者夜嚎——夜裡無獵可追時仰首長嚎（頭頂浮 🌙）。原地不動（不更新座標）、
    /// howl_timer 倒數，到期就回到巡遊；一發現獵物即由呼叫端改去獵殺（狩獵優先）。
    Howling { howl_timer: f32 },
    /// ROADMAP 219：破曉甦醒伸展——天明喚醒夜眠的晝行獵物時，先原地伸展一小段（頭頂浮 🌅）、
    /// wake_timer 倒數，到期才起身漫遊；伸展中若有威脅逼近一律優先中斷改逃竄。
    Waking { wake_timer: f32 },
    /// ROADMAP 220：鳥群振翅升空盤旋——白天平靜的野鳥升空後，繞著群心（無群則繞自家）以 angle
    /// 為當前盤旋角、每幀依角速度推進繞圈；fly_timer 倒數，到期就降落回漫遊；盤旋中若有威脅
    /// 逼近一律優先降下逃竄。前端依此狀態把鳥往上抬、地面留投影，讀起來是「飛在空中」。
    Flying { fly_timer: f32, angle: f32 },
    /// ROADMAP 221：晝日鳥鳴呼應——白天平靜的野鳥停下啁啾（頭頂浮 🎵）。原地不動（不更新座標）、
    /// chirp_timer 倒數，到期就回到漫遊（沿用群聚拉力）；啁啾中若有威脅逼近一律優先收聲逃竄。
    Chirping { chirp_timer: f32 },
    /// ROADMAP 222：小動物捧食啃咬——白天平靜的小動物坐起來捧食啃咬（頭頂浮 🌰）。原地不動
    /// （不更新座標）、nibble_timer 倒數，到期就回到漫遊（沿用群聚拉力）；啃咬中若有威脅逼近
    /// 一律優先丟食逃竄。只有小動物（SmallCritter）會啃咬。
    Nibbling { nibble_timer: f32 },
}

// ─── 實體 ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Wildlife {
    pub id: u32,
    pub kind: WildlifeKind,
    pub x: f32,
    pub y: f32,
    pub alive: bool,
    respawn_timer: f32,
    home_x: f32,
    home_y: f32,
    state: WildlifeState,
    /// ROADMAP 205：個體親近度（0~1）——反覆餵食累積，達 TAME_FAMILIARITY 即馴養。
    familiarity: f32,
    /// ROADMAP 207：成熟度（0~1）。初始族群皆為成體（1.0）；繁衍誕生的幼獸由 0 起、
    /// 隨時間長到 1.0。未滿 1.0 即「幼獸」，體型較小（前端據 `scale()` 縮小繪製）。
    maturity: f32,
}

impl Wildlife {
    fn new(id: u32, kind: WildlifeKind, hx: f32, hy: f32, rng: &mut StdRng) -> Self {
        let offset_x = rng.gen_range(-50.0_f32..50.0);
        let offset_y = rng.gen_range(-50.0_f32..50.0);
        Self {
            id,
            kind,
            x: hx + offset_x,
            y: hy + offset_y,
            home_x: hx,
            home_y: hy,
            alive: true,
            respawn_timer: 0.0,
            state: WildlifeState::Resting {
                rest_timer: rng.gen_range(REST_TIMER_MIN..=REST_TIMER_MAX),
            },
            familiarity: 0.0,
            // 初始族群皆為成體。
            maturity: 1.0,
        }
    }

    /// ROADMAP 207：誕生一隻幼獸（成熟度由 0 起）。家設在群體中心，讓牠生來就屬於這群。
    fn new_juvenile(id: u32, kind: WildlifeKind, cx: f32, cy: f32, rng: &mut StdRng) -> Self {
        let mut w = Wildlife::new(id, kind, cx, cy, rng);
        w.maturity = 0.0;
        w
    }

    /// ROADMAP 205：此隻動物目前的親近度（0~1）。
    pub fn familiarity(&self) -> f32 {
        self.familiarity
    }

    /// ROADMAP 205：是否已被馴養（親近度達門檻）。
    pub fn is_tamed(&self) -> bool {
        self.familiarity >= TAME_FAMILIARITY
    }

    /// ROADMAP 207：是否為尚未長成的幼獸。
    pub fn is_juvenile(&self) -> bool {
        self.maturity < 1.0
    }

    /// ROADMAP 207：相對體型（幼獸小、成體 1.0）——供前端縮放繪製。
    /// 由成熟度線性插值：剛誕生 `JUVENILE_MIN_SCALE`、長成後 1.0。
    pub fn scale(&self) -> f32 {
        JUVENILE_MIN_SCALE + (1.0 - JUVENILE_MIN_SCALE) * self.maturity.clamp(0.0, 1.0)
    }

    /// 非追獵行為 tick：閒晃 / 休息 / 逃跑 / 返家。
    /// `flee_threats`：需要逃離的座標（玩家 + 捕食者）；捕食者呼叫時傳空。
    /// `herd_anchor`：ROADMAP 206——附近同種夥伴的平均位置；選新漫遊目標時朝它拉，
    /// 同種動物便鬆散成群移動。捕食者傳 `None`（獨來獨往）。
    /// `graze_prob`：ROADMAP 211——抵達漫遊目標時轉入「吃草」（而非單純休息）的機率。
    /// 只有平靜的晝行獵物白天才 > 0；夜間/掠食者一律傳 0（行為與切片前逐位元一致）。
    fn tick_idle(&mut self, dt: f32, flee_threats: &[(f32, f32)], speed: f32, herd_anchor: Option<(f32, f32)>, graze_prob: f32, rng: &mut StdRng) {
        let already_fleeing = matches!(self.state, WildlifeState::Fleeing { .. });
        if !already_fleeing {
            if let Some((tx, ty)) = nearest_in_range(self.x, self.y, flee_threats, FLEE_RADIUS) {
                let dx = self.x - tx;
                let dy = self.y - ty;
                let len = (dx * dx + dy * dy).sqrt().max(1.0);
                self.state = WildlifeState::Fleeing {
                    vx: dx / len * FLEE_SPEED,
                    vy: dy / len * FLEE_SPEED,
                    flee_timer: FLEE_DURATION,
                };
                return;
            }
        }

        match self.state.clone() {
            WildlifeState::Fleeing { vx, vy, flee_timer } => {
                self.x += vx * dt;
                self.y += vy * dt;
                let remaining = flee_timer - dt;
                if remaining <= 0.0 {
                    self.state = WildlifeState::Returning;
                } else if let Some((tx, ty)) = nearest_in_range(self.x, self.y, flee_threats, FLEE_RADIUS) {
                    let dx = self.x - tx;
                    let dy = self.y - ty;
                    let len = (dx * dx + dy * dy).sqrt().max(1.0);
                    self.state = WildlifeState::Fleeing {
                        vx: dx / len * FLEE_SPEED,
                        vy: dy / len * FLEE_SPEED,
                        flee_timer: remaining,
                    };
                } else {
                    self.state = WildlifeState::Fleeing { vx, vy, flee_timer: remaining };
                }
            }
            WildlifeState::Returning => {
                let dx = self.home_x - self.x;
                let dy = self.home_y - self.y;
                let dist = (dx * dx + dy * dy).sqrt();
                if dist <= HOME_ARRIVE_DIST {
                    self.x = self.home_x;
                    self.y = self.home_y;
                    let timer = rng.gen_range(WANDER_TIMER_MIN..=WANDER_TIMER_MAX);
                    let (tx, ty) = herd_wander_target(self.home_x, self.home_y, herd_anchor, rng);
                    self.state = WildlifeState::Wandering { target_x: tx, target_y: ty, wander_timer: timer };
                } else {
                    self.x += (dx / dist) * RETURN_SPEED * dt;
                    self.y += (dy / dist) * RETURN_SPEED * dt;
                }
            }
            WildlifeState::Resting { rest_timer } => {
                let remaining = rest_timer - dt;
                if remaining <= 0.0 {
                    let timer = rng.gen_range(WANDER_TIMER_MIN..=WANDER_TIMER_MAX);
                    let (tx, ty) = herd_wander_target(self.home_x, self.home_y, herd_anchor, rng);
                    self.state = WildlifeState::Wandering { target_x: tx, target_y: ty, wander_timer: timer };
                } else {
                    self.state = WildlifeState::Resting { rest_timer: remaining };
                }
            }
            WildlifeState::Wandering { target_x, target_y, wander_timer } => {
                let remaining = wander_timer - dt;
                let dx = target_x - self.x;
                let dy = target_y - self.y;
                let dist = (dx * dx + dy * dy).sqrt();
                if dist < 8.0 || remaining <= 0.0 {
                    // ROADMAP 211：白晝吃草——抵達目標時，晝行獵物有 graze_prob 機率低頭吃草
                    //（原地不動數秒、頭頂浮 🌿）而非單純休息；graze_prob==0（掠食者/夜間）時逐位元同原本。
                    if graze_prob > 0.0 && rng.gen::<f32>() < graze_prob {
                        let graze = rng.gen_range(GRAZE_DURATION_MIN..=GRAZE_DURATION_MAX);
                        self.state = WildlifeState::Grazing { graze_timer: graze };
                    } else {
                        let rest = rng.gen_range(REST_TIMER_MIN..=REST_TIMER_MAX);
                        self.state = WildlifeState::Resting { rest_timer: rest };
                    }
                } else {
                    self.x += (dx / dist) * speed * dt;
                    self.y += (dy / dist) * speed * dt;
                    self.state = WildlifeState::Wandering { target_x, target_y, wander_timer: remaining };
                }
            }
            WildlifeState::Grazing { graze_timer } => {
                // ROADMAP 211：吃草中——原地不動（不更新座標）、計時遞減；到期後再挑下一個漫遊目標。
                let remaining = graze_timer - dt;
                if remaining <= 0.0 {
                    let timer = rng.gen_range(WANDER_TIMER_MIN..=WANDER_TIMER_MAX);
                    let (tx, ty) = herd_wander_target(self.home_x, self.home_y, herd_anchor, rng);
                    self.state = WildlifeState::Wandering { target_x: tx, target_y: ty, wander_timer: timer };
                } else {
                    self.state = WildlifeState::Grazing { graze_timer: remaining };
                }
            }
            // Hunting/Digesting 由管理器處理。
            _ => {}
        }
    }

    /// ROADMAP 212：背向 (tx,ty) 炸出逃竄——設為 Fleeing（與 tick_idle 的逃竄初始化一致）。
    /// 供哨兵在「放大警戒半徑」內發現威脅時率先逃竄（再經 209 恐慌感染帶動全群）。
    fn flee_from(&mut self, tx: f32, ty: f32) {
        let dx = self.x - tx;
        let dy = self.y - ty;
        let len = (dx * dx + dy * dy).sqrt().max(1.0);
        self.state = WildlifeState::Fleeing {
            vx: dx / len * FLEE_SPEED,
            vy: dy / len * FLEE_SPEED,
            flee_timer: FLEE_DURATION,
        };
    }

    /// ROADMAP 212：群體警戒哨——哨兵「站崗放哨」行為（無威脅時才走此分支；威脅由呼叫端
    /// 先以放大半徑攔截並改走逃竄）。哨兵抬頭警戒數秒（Watching，原地不動、不吃草），時間到
    /// 就跟著群體挪一步（免得被群拋下），抵達後重新站崗。其餘狀態（剛由吃草／休息／夜眠轉
    /// 來）一律收斂成站崗。移動模型沿用 tick_idle 的漫遊／返家段，行為一致、便於測試。
    fn tick_watch(&mut self, dt: f32, herd_anchor: Option<(f32, f32)>, rng: &mut StdRng) {
        match self.state.clone() {
            WildlifeState::Watching { watch_timer } => {
                let remaining = watch_timer - dt;
                if remaining <= 0.0 {
                    // 放哨告一段落：跟群體挪一步，稍後再站崗。
                    let timer = rng.gen_range(WANDER_TIMER_MIN..=WANDER_TIMER_MAX);
                    let (tx, ty) = herd_wander_target(self.home_x, self.home_y, herd_anchor, rng);
                    self.state = WildlifeState::Wandering { target_x: tx, target_y: ty, wander_timer: timer };
                } else {
                    self.state = WildlifeState::Watching { watch_timer: remaining };
                }
            }
            WildlifeState::Wandering { target_x, target_y, wander_timer } => {
                let remaining = wander_timer - dt;
                let dx = target_x - self.x;
                let dy = target_y - self.y;
                let dist = (dx * dx + dy * dy).sqrt();
                if dist < 8.0 || remaining <= 0.0 {
                    // 抵達 → 站崗放哨。
                    let watch = rng.gen_range(WATCH_DURATION_MIN..=WATCH_DURATION_MAX);
                    self.state = WildlifeState::Watching { watch_timer: watch };
                } else {
                    self.x += (dx / dist) * WANDER_SPEED * dt;
                    self.y += (dy / dist) * WANDER_SPEED * dt;
                    self.state = WildlifeState::Wandering { target_x, target_y, wander_timer: remaining };
                }
            }
            WildlifeState::Returning => {
                let dx = self.home_x - self.x;
                let dy = self.home_y - self.y;
                let dist = (dx * dx + dy * dy).sqrt();
                if dist <= HOME_ARRIVE_DIST {
                    self.x = self.home_x;
                    self.y = self.home_y;
                    let watch = rng.gen_range(WATCH_DURATION_MIN..=WATCH_DURATION_MAX);
                    self.state = WildlifeState::Watching { watch_timer: watch };
                } else {
                    self.x += (dx / dist) * RETURN_SPEED * dt;
                    self.y += (dy / dist) * RETURN_SPEED * dt;
                }
            }
            // Resting / Grazing /（剛由夜眠轉白天）等其餘狀態：立即收斂成站崗放哨。
            _ => {
                let watch = rng.gen_range(WATCH_DURATION_MIN..=WATCH_DURATION_MAX);
                self.state = WildlifeState::Watching { watch_timer: watch };
            }
        }
    }

    /// ROADMAP 210：晝夜作息——夜間「平靜歸巢沉睡」行為。
    /// 呼叫端（Phase 4）已確保：此隻為晝行獵物、當下未在逃竄、附近也無威脅；
    /// 故本函式只管「回家睡覺」——尚未到家就朝家走（Returning），到家就轉入長時休息
    /// （沉睡）。沉睡的 rest_timer 在平靜夜晚不會被遞減（不走 tick_idle），故會一路睡到
    /// 天明；威脅一旦逼近，Phase 4 會在進到此分支前就改走逃竄（威脅永遠優先）。
    fn tick_night_rest(&mut self, dt: f32) {
        let dx = self.home_x - self.x;
        let dy = self.home_y - self.y;
        let dist = (dx * dx + dy * dy).sqrt();
        if dist <= HOME_ARRIVE_DIST {
            // 已到家：安睡。已在休息就維持原狀（不重設計時、不抖動），否則轉入長時沉睡。
            self.x = self.home_x;
            self.y = self.home_y;
            if !matches!(self.state, WildlifeState::Resting { .. }) {
                self.state = WildlifeState::Resting { rest_timer: NIGHT_SLEEP_REST_SECS };
            }
        } else {
            // 尚未到家：朝家緩步歸返。
            self.x += (dx / dist) * RETURN_SPEED * dt;
            self.y += (dy / dist) * RETURN_SPEED * dt;
            self.state = WildlifeState::Returning;
        }
    }

    /// ROADMAP 210：破曉甦醒——天明時把仍在夜眠的晝行獵物主動喚回閒晃。
    /// 夜眠用的 NIGHT_SLEEP_REST_SECS(=一整個日夜週期長) 遠大於日間小憩上限
    /// REST_TIMER_MAX，故以「rest_timer 是否超過日間小憩上限」即可分辨夜眠與小憩——
    /// 只喚醒夜眠者、不打斷白天的正常小憩。不靠計時器自然到期，因為那計時器比整段
    /// 白天還長,鹿會癱在家裡跨越整個白天(與「晨光鋪上草地、鹿群一隻隻醒來」相反)。
    fn wake_from_night_sleep(&mut self, rng: &mut StdRng) {
        if let WildlifeState::Resting { rest_timer } = self.state {
            if rest_timer > REST_TIMER_MAX {
                // ROADMAP 219：破曉甦醒不再瞬間起步——先轉入「伸展」一小段（頭頂浮 🌅），
                // 由 tick_wake 倒數到期才起身投入漫遊（晨光鋪上草地、鹿群先舒展再散開）。
                let wake = rng.gen_range(WAKE_DURATION_MIN..=WAKE_DURATION_MAX);
                self.state = WildlifeState::Waking { wake_timer: wake };
            }
        }
    }

    /// ROADMAP 219：破曉甦醒伸展——伸展中（Waking）原地不動、倒數計時；到期就挑下一個漫遊目標
    /// （沿用群聚拉力 herd_anchor）起身投入新一天的閒晃。只在 Waking 狀態下生效（呼叫端已確保
    /// 此隻為晝行獵物、天明、且當下平靜；威脅一旦逼近，呼叫端不會走到此分支、改去逃竄——威脅永遠優先）。
    fn tick_wake(&mut self, dt: f32, herd_anchor: Option<(f32, f32)>, rng: &mut StdRng) {
        if let WildlifeState::Waking { wake_timer } = self.state {
            let remaining = wake_timer - dt;
            if remaining <= 0.0 {
                let timer = rng.gen_range(WANDER_TIMER_MIN..=WANDER_TIMER_MAX);
                let (tx, ty) = herd_wander_target(self.home_x, self.home_y, herd_anchor, rng);
                self.state = WildlifeState::Wandering { target_x: tx, target_y: ty, wander_timer: timer };
            } else {
                self.state = WildlifeState::Waking { wake_timer: remaining };
            }
        }
    }

    /// 供協議層使用的狀態字串。
    /// ROADMAP 215：幼獸嬉戲——已依偎到母獸身邊的幼獸在媽媽周圍蹦跳玩耍。朝當前蹦跳落點
    /// (hop_x, hop_y) 蹦去；蹦到落點或 frolic_timer 耗盡，就回到依偎（朝母獸 (mx,my) 的溫順
    /// Wandering），下一幀再由呼叫端決定要不要重新開一段嬉戲。`mx,my` 為母獸座標：到期收尾時
    /// 回到母獸身邊，玩不離媽媽。只在 Frolicking 狀態下生效（呼叫端已確保）。
    fn tick_frolic(&mut self, dt: f32, mx: f32, my: f32) {
        if let WildlifeState::Frolicking { hop_x, hop_y, frolic_timer } = self.state {
            let (nx, ny) = frolic_hop(self.x, self.y, hop_x, hop_y, dt);
            self.x = nx;
            self.y = ny;
            let remaining = frolic_timer - dt;
            let reached =
                (hop_x - self.x).powi(2) + (hop_y - self.y).powi(2) <= FROLIC_REACH * FROLIC_REACH;
            if remaining <= 0.0 || reached {
                // 玩夠這一段：回到母獸身邊依偎（朝母獸的溫順 Wandering）。
                self.state = WildlifeState::Wandering { target_x: mx, target_y: my, wander_timer: 1.0 };
            } else {
                self.state = WildlifeState::Frolicking { hop_x, hop_y, frolic_timer: remaining };
            }
        }
    }

    /// ROADMAP 216：成體相依理毛——理毛中（Grooming）原地不動、倒數計時；到期就挑下一個
    /// 漫遊目標（沿用群聚拉力 herd_anchor）回到漫遊。只在 Grooming 狀態下生效（呼叫端已確保
    /// 此隻為成體、白天、平靜、且身邊有同種夥伴；威脅逼近時呼叫端不會走到此分支、改逃竄）。
    fn tick_groom(&mut self, dt: f32, herd_anchor: Option<(f32, f32)>, rng: &mut StdRng) {
        if let WildlifeState::Grooming { groom_timer } = self.state {
            let remaining = groom_timer - dt;
            if remaining <= 0.0 {
                let timer = rng.gen_range(WANDER_TIMER_MIN..=WANDER_TIMER_MAX);
                let (tx, ty) = herd_wander_target(self.home_x, self.home_y, herd_anchor, rng);
                self.state = WildlifeState::Wandering { target_x: tx, target_y: ty, wander_timer: timer };
            } else {
                self.state = WildlifeState::Grooming { groom_timer: remaining };
            }
        }
    }

    /// ROADMAP 217：掠食者夜嚎——長嚎中（Howling）原地不動、倒數計時；到期就挑下一個漫遊目標
    /// 回到巡遊（掠食者獨來獨往，故用 random_target 純隨機、無群聚拉力）。只在 Howling 狀態下
    /// 生效（呼叫端已確保此隻為掠食者、夜間、附近無可追獵物；發現獵物時呼叫端不會走到此分支、
    /// 改去獵殺——狩獵永遠優先）。
    fn tick_howl(&mut self, dt: f32, rng: &mut StdRng) {
        if let WildlifeState::Howling { howl_timer } = self.state {
            let remaining = howl_timer - dt;
            if remaining <= 0.0 {
                let timer = rng.gen_range(WANDER_TIMER_MIN..=WANDER_TIMER_MAX);
                let (tx, ty) = random_target(self.home_x, self.home_y, WANDER_RADIUS, rng);
                self.state = WildlifeState::Wandering { target_x: tx, target_y: ty, wander_timer: timer };
            } else {
                self.state = WildlifeState::Howling { howl_timer: remaining };
            }
        }
    }

    /// ROADMAP 220：鳥群振翅升空盤旋——盤旋中（Flying）繞著群心 `anchor`（無群則繞自家）轉圈：
    /// 每幀把盤旋角 `angle` 依角速度推進、沿 FLIGHT_CIRCLE_RADIUS 重算座標，整群共用同一群心、
    /// 各以不同起始角繞行，便讀成「一群鳥一起盤旋」。fly_timer 倒數到期就降落、起身漫遊（沿用
    /// 群聚拉力挑落點）。只在 Flying 狀態下生效（呼叫端已確保此隻為野鳥、白天、平靜；威脅一旦
    /// 逼近呼叫端不會走到此分支、改去降下逃竄——威脅永遠優先）。
    fn tick_fly(&mut self, dt: f32, herd_anchor: Option<(f32, f32)>, rng: &mut StdRng) {
        if let WildlifeState::Flying { fly_timer, angle } = self.state {
            let remaining = fly_timer - dt;
            if remaining <= 0.0 {
                let timer = rng.gen_range(WANDER_TIMER_MIN..=WANDER_TIMER_MAX);
                let (tx, ty) = herd_wander_target(self.home_x, self.home_y, herd_anchor, rng);
                self.state = WildlifeState::Wandering { target_x: tx, target_y: ty, wander_timer: timer };
            } else {
                let (cx, cy) = herd_anchor.unwrap_or((self.home_x, self.home_y));
                let new_angle = angle + FLIGHT_ANGULAR_SPEED * dt;
                self.x = cx + new_angle.cos() * FLIGHT_CIRCLE_RADIUS;
                self.y = cy + new_angle.sin() * FLIGHT_CIRCLE_RADIUS;
                self.state = WildlifeState::Flying { fly_timer: remaining, angle: new_angle };
            }
        }
    }

    /// ROADMAP 221：晝日鳥鳴呼應——啁啾中（Chirping）原地不動、倒數計時；到期就挑下一個漫遊目標
    /// （沿用群聚拉力 herd_anchor，鳥成群，與獨來獨往的狼 tick_howl 用 random_target 不同）回到
    /// 閒晃。只在 Chirping 狀態下生效（呼叫端已確保此隻為野鳥、白天、平靜；威脅一旦逼近呼叫端不會
    /// 走到此分支、改去收聲逃竄——威脅永遠優先）。
    fn tick_chirp(&mut self, dt: f32, herd_anchor: Option<(f32, f32)>, rng: &mut StdRng) {
        if let WildlifeState::Chirping { chirp_timer } = self.state {
            let remaining = chirp_timer - dt;
            if remaining <= 0.0 {
                let timer = rng.gen_range(WANDER_TIMER_MIN..=WANDER_TIMER_MAX);
                let (tx, ty) = herd_wander_target(self.home_x, self.home_y, herd_anchor, rng);
                self.state = WildlifeState::Wandering { target_x: tx, target_y: ty, wander_timer: timer };
            } else {
                self.state = WildlifeState::Chirping { chirp_timer: remaining };
            }
        }
    }

    /// ROADMAP 222：小動物捧食啃咬——啃咬中（Nibbling）原地不動、倒數計時；到期就挑下一個漫遊目標
    /// （沿用群聚拉力 herd_anchor）回到閒晃。只在 Nibbling 狀態下生效（呼叫端已確保此隻為小動物、
    /// 白天、平靜；威脅一旦逼近呼叫端不會走到此分支、改去丟食逃竄——威脅永遠優先）。
    fn tick_nibble(&mut self, dt: f32, herd_anchor: Option<(f32, f32)>, rng: &mut StdRng) {
        if let WildlifeState::Nibbling { nibble_timer } = self.state {
            let remaining = nibble_timer - dt;
            if remaining <= 0.0 {
                let timer = rng.gen_range(WANDER_TIMER_MIN..=WANDER_TIMER_MAX);
                let (tx, ty) = herd_wander_target(self.home_x, self.home_y, herd_anchor, rng);
                self.state = WildlifeState::Wandering { target_x: tx, target_y: ty, wander_timer: timer };
            } else {
                self.state = WildlifeState::Nibbling { nibble_timer: remaining };
            }
        }
    }

    pub fn state_str(&self) -> &'static str {
        match &self.state {
            WildlifeState::Wandering { .. } => "wandering",
            WildlifeState::Resting { .. }   => "resting",
            WildlifeState::Fleeing { .. }   => "fleeing",
            WildlifeState::Returning        => "returning",
            WildlifeState::Stalking { .. }  => "stalking",
            WildlifeState::Hunting { .. }   => "hunting",
            WildlifeState::Digesting { .. } => "digesting",
            WildlifeState::Guarding { .. }  => "guarding",
            WildlifeState::Grazing { .. }   => "grazing",
            WildlifeState::Watching { .. }  => "watching",
            WildlifeState::Defending        => "defending",
            WildlifeState::Frolicking { .. } => "frolicking",
            WildlifeState::Grooming { .. }  => "grooming",
            WildlifeState::Howling { .. }   => "howling",
            WildlifeState::Waking { .. }    => "waking",
            WildlifeState::Flying { .. }    => "flying",
            WildlifeState::Chirping { .. }  => "chirping",
            WildlifeState::Nibbling { .. }  => "nibbling",
        }
    }
}

// ─── 管理器 ──────────────────────────────────────────────────────────────────

pub struct WildlifeManager {
    pub animals: Vec<Wildlife>,
    rng: StdRng,
    /// 距上次捕獵廣播的累計秒數（限流用）。
    kill_broadcast_cooldown: f32,
    /// ROADMAP 142：活躍乙太微粒列表。
    pub carion_orbs: Vec<CarrionOrb>,
    /// 微粒 ID 計數器（跨生命週期唯一）。
    orb_counter: u32,
    /// ROADMAP 143：物種聚落定義（靜態）。
    pub colonies: Vec<Colony>,
    /// 每個聚落的廣播冷卻倒數（索引對應 colonies）。
    colony_threat_cooldowns: Vec<f32>,
    /// ROADMAP 207：下一隻新生個體的 ID（繁衍誕生用，確保全程唯一、不與初始 22 隻衝突）。
    next_animal_id: u32,
    /// ROADMAP 207：各獵物物種的「安穩成群」累計秒數；達門檻即誕生一隻幼獸後歸零。
    breed_progress: std::collections::HashMap<WildlifeKind, f32>,
}

impl WildlifeManager {
    pub fn new() -> Self {
        let mut rng = StdRng::seed_from_u64(7654321);
        let animals = spawn_all_wildlife(&mut rng);
        let colonies = build_colonies();
        let n = colonies.len();
        let next_animal_id = animals.len() as u32;
        Self {
            animals, rng,
            kill_broadcast_cooldown: 0.0,
            carion_orbs: Vec::new(),
            orb_counter: 0,
            colonies,
            colony_threat_cooldowns: vec![0.0; n],
            next_animal_id,
            breed_progress: std::collections::HashMap::new(),
        }
    }

    /// 供快照廣播的聚落視圖列表（靜態，每幀傳出）。
    pub fn colony_views(&self) -> Vec<ColonyView> {
        self.colonies.iter().map(|c| ColonyView {
            id: c.id,
            kind: c.kind.as_str().to_string(),
            name: c.name.to_string(),
            cx: c.cx,
            cy: c.cy,
            guard_radius: c.guard_radius,
        }).collect()
    }

    /// ROADMAP 142：嘗試採集距玩家最近的乙太微粒。
    /// 成功回傳乙太量，並移除該微粒；否則回傳 None。
    pub fn collect_carion_orb(&mut self, orb_id: u32, px: f32, py: f32) -> Option<u32> {
        let r2 = CARION_COLLECT_RADIUS * CARION_COLLECT_RADIUS;
        let idx = self.carion_orbs.iter().position(|o| {
            o.id == orb_id && (o.x - px).powi(2) + (o.y - py).powi(2) <= r2
        })?;
        self.carion_orbs.swap_remove(idx);
        Some(CARION_ETHER)
    }

    /// ROADMAP 144：玩家攻擊野生動物——在攻擊距離內找到該 ID 的存活動物並使其死亡。
    /// 回傳被擊殺動物的種類（`None` 表示不存在/超出距離/已死亡）。
    pub fn attack_wildlife(
        &mut self,
        wildlife_id: u32,
        px: f32,
        py: f32,
        reach: f32,
    ) -> Option<WildlifeKind> {
        let reach2 = reach * reach;
        if let Some(a) = self.animals.iter_mut().find(|a| {
            a.id == wildlife_id
                && a.alive
                && (a.x - px).powi(2) + (a.y - py).powi(2) <= reach2
        }) {
            let kind = a.kind;
            a.alive = false;
            a.respawn_timer = PREY_RESPAWN_SECS;
            a.state = WildlifeState::Resting { rest_timer: 0.0 };
            Some(kind)
        } else {
            None
        }
    }

    /// ROADMAP 205：餵食指定 ID 的存活動物，提升其個體親近度。
    /// 回傳 `(種類, 提升後親近度, 是否「剛跨過馴養門檻」)`；找不到/已死亡則 `None`。
    /// 距離 / 種子消耗由呼叫端（ws.rs 的 feed_wildlife）負責，本函式只管親近度。
    pub fn on_feed_animal(&mut self, wildlife_id: u32) -> Option<(WildlifeKind, f32, bool)> {
        let a = self.animals.iter_mut().find(|a| a.id == wildlife_id && a.alive)?;
        let was_tamed = a.familiarity >= TAME_FAMILIARITY;
        a.familiarity = (a.familiarity + FEED_FAMILIARITY_GAIN).min(MAX_FAMILIARITY);
        let now_tamed = a.familiarity >= TAME_FAMILIARITY;
        Some((a.kind, a.familiarity, now_tamed && !was_tamed))
    }

    /// ROADMAP 165：回傳所有存活野生動物的快照（ID, 種類, x, y）。
    /// 供怪物追獵目標計算用（取讀鎖後呼叫）。
    pub fn alive_snapshot(&self) -> Vec<(u32, WildlifeKind, f32, f32)> {
        self.animals.iter()
            .filter(|a| a.alive)
            .map(|a| (a.id, a.kind, a.x, a.y))
            .collect()
    }

    /// ROADMAP 165：怪物獵殺野生動物——標記獵物死亡、生成乙太微粒、回傳事件。
    /// 若 wildlife_id 不存在或已死亡，回傳 None（冪等，安全可重呼叫）。
    pub fn on_monster_kills_wildlife(
        &mut self,
        wildlife_id: u32,
        monster_kind: EnemyKind,
    ) -> Option<WildlifeEvent> {
        let prey = self.animals.iter_mut().find(|a| a.id == wildlife_id && a.alive)?;
        let wildlife_kind = prey.kind;
        let kx = prey.x;
        let ky = prey.y;
        prey.alive = false;
        prey.respawn_timer = PREY_RESPAWN_SECS;
        prey.state = WildlifeState::Resting { rest_timer: 0.0 };
        // 在死亡位置生成乙太微粒（死亡是循環的一環，不分陣營）。
        if self.carion_orbs.len() < MAX_CARION_ORBS {
            let id = self.orb_counter;
            self.orb_counter = self.orb_counter.wrapping_add(1);
            self.carion_orbs.push(CarrionOrb { id, x: kx, y: ky, ttl: CARION_ORB_TTL });
        }
        Some(WildlifeEvent::MonsterHunted { monster_kind, wildlife_kind, x: kx, y: ky })
    }

    /// 每幀推進所有野生動物，回傳本幀產生的事件列表。
    ///
    /// `attitudes`：各物種目前態度值（0-100）。用於：
    ///   - 友善（≥65）：獵物不把玩家加入逃跑威脅清單（不逃）。
    ///   - 敵視（<25）：獵物主動向玩家靠近（守衛行為），近身時發出 WildlifeAttack 事件。
    ///
    /// `is_night`（ROADMAP 210）：晝夜作息。為 true 時——晝行獵物（鹿/鳥/小動物）在平靜
    /// 無威脅時歸巢沉睡、不再閒晃；夜行掠食者（狼/狐）狩獵搜尋範圍放大（更積極覓食）。
    pub fn tick(
        &mut self,
        dt: f32,
        player_positions: &[(f32, f32)],
        attitudes: &std::collections::HashMap<WildlifeKind, i32>,
        monster_threats: &[(EnemyKind, f32, f32)],
        is_night: bool,
    ) -> Vec<WildlifeEvent> {
        let mut events = Vec::new();
        self.kill_broadcast_cooldown = (self.kill_broadcast_cooldown - dt).max(-1.0);

        // ── Phase 0a: 乙太微粒 TTL 倒數（ROADMAP 142）────────────────────────
        for orb in &mut self.carion_orbs {
            orb.ttl -= dt;
        }
        self.carion_orbs.retain(|o| o.ttl > 0.0);

        // ── Phase 0b: 聚落廣播冷卻倒數（ROADMAP 143）────────────────────────
        for cd in &mut self.colony_threat_cooldowns {
            *cd = (*cd - dt).max(0.0);
        }

        // ── Phase 1: 死亡倒數 + 重生 + 親近度衰減（ROADMAP 205）─────────────────
        for a in &mut self.animals {
            if !a.alive {
                a.respawn_timer -= dt;
            } else {
                if a.familiarity > 0.0 {
                    // 親近度隨時間緩慢衰減——羈絆需偶爾以餵食維繫，但不易斷。
                    a.familiarity = (a.familiarity - FAMILIARITY_DECAY_PER_SEC * dt).max(0.0);
                }
                // ROADMAP 207：幼獸隨時間長大，成熟度趨近 1.0（體型隨之變大）。
                if a.maturity < 1.0 {
                    a.maturity = (a.maturity + dt / MATURE_DURATION_SECS).min(1.0);
                }
            }
        }
        let respawn_ready: Vec<usize> = self.animals.iter().enumerate()
            .filter(|(_, a)| !a.alive && a.respawn_timer <= 0.0)
            .map(|(i, _)| i)
            .collect();
        for i in respawn_ready {
            let ox: f32 = self.rng.gen_range(-40.0..40.0);
            let oy: f32 = self.rng.gen_range(-40.0..40.0);
            let a = &mut self.animals[i];
            a.alive = true;
            a.x = a.home_x + ox;
            a.y = a.home_y + oy;
            a.state = WildlifeState::Resting { rest_timer: 2.0 };
            // ROADMAP 205：重生的是「新的個體」，與玩家的羈絆隨上一隻回歸乙太而散——親近度歸零。
            a.familiarity = 0.0;
        }

        // ── Phase 2: 快照（供決策使用） ────────────────────────────────────────
        let prey_snap: Vec<(u32, WildlifeKind, f32, f32)> = self.animals.iter()
            .filter(|a| a.alive && a.kind.trophic_level() == TrophicLevel::Prey)
            .map(|a| (a.id, a.kind, a.x, a.y))
            .collect();

        // ROADMAP 208：成體獵物位置快照（maturity 已滿），供幼獸尋找依偎的「母獸」。
        let adult_snap: Vec<(u32, WildlifeKind, f32, f32)> = self.animals.iter()
            .filter(|a| a.alive && a.kind.trophic_level() == TrophicLevel::Prey && !a.is_juvenile())
            .map(|a| (a.id, a.kind, a.x, a.y))
            .collect();

        // 捕食者位置：獵物逃跑時參考此清單。
        let pred_positions: Vec<(f32, f32)> = self.animals.iter()
            .filter(|a| a.alive && a.kind.trophic_level() == TrophicLevel::Predator)
            .map(|a| (a.x, a.y))
            .collect();

        // ROADMAP 214：母獸護幼——正在護幼的成體位置快照（種類＋座標），供 Phase 3 把逼近的
        // 同種掠食者嚇退。刻意在此處（變更前）取一次，反映上一幀設下的 Defending 狀態：
        // 掠食者本幀讀到、放棄狩獵，成體本幀（Phase 4）再依當下威脅刷新護衛——一幀延遲、自然。
        let defending_snap: Vec<(WildlifeKind, f32, f32)> = self.animals.iter()
            .filter(|a| a.alive && matches!(a.state, WildlifeState::Defending))
            .map(|a| (a.kind, a.x, a.y))
            .collect();

        // ROADMAP 218：群嚎呼應——本幀「起始時正在長嚎」的掠食者座標快照，供 Phase 3 讓附近
        // 同樣夜裡歇息的同類聽見後接嚎。刻意在 Phase 3 變更前取一次：本幀新接嚎者不在此快照裡，
        // 故牽動每 tick 只向外擴一圈，嚎聲像漣漪般逐圈傳開（與 209 驚群恐慌的逐圈傳染同手法）。
        let howling_snap: Vec<(f32, f32)> = self.animals.iter()
            .filter(|a| a.alive && matches!(a.state, WildlifeState::Howling { .. }))
            .map(|a| (a.x, a.y))
            .collect();

        // ROADMAP 220：本幀起始時正在盤旋（Flying）的野鳥座標快照——供其餘平靜野鳥據此
        // 「看見」附近升空的同類而被牽動跟著飛起（接力升空，仿 218 群嚎快照）。新起飛者本幀
        // 不在此快照裡，故牽動每 tick 只向外擴一圈，升空像漣漪般逐圈傳開。
        let flying_snap: Vec<(f32, f32)> = self.animals.iter()
            .filter(|a| a.alive && matches!(a.state, WildlifeState::Flying { .. }))
            .map(|a| (a.x, a.y))
            .collect();

        // ROADMAP 221：本幀起始時正在啁啾（Chirping）的野鳥座標快照——供其餘平靜野鳥據此
        // 「聽見」附近啁啾的同類而被牽動跟鳴（接力起鳴，仿 218 群嚎快照）。新起鳴者本幀不在此
        // 快照裡，故牽動每 tick 只向外擴一圈，鳴聲像漣漪般逐圈傳開。
        let chirping_snap: Vec<(f32, f32)> = self.animals.iter()
            .filter(|a| a.alive && matches!(a.state, WildlifeState::Chirping { .. }))
            .map(|a| (a.x, a.y))
            .collect();

        // ── Phase 2b: 聚落威脅偵測（ROADMAP 143）────────────────────────────
        // 對每個聚落：若有玩家進入守衛半徑，啟動同種動物的 Guarding 行為。
        for (idx, col) in self.colonies.iter().enumerate() {
            // 找出在守衛半徑內最近的玩家。
            let threat = player_positions.iter().find(|&&(px, py)| {
                let dx = px - col.cx;
                let dy = py - col.cy;
                dx * dx + dy * dy <= col.guard_radius * col.guard_radius
            }).copied();

            let Some((threat_x, threat_y)) = threat else { continue };

            // 廣播世界聊天（有冷卻）。
            if self.colony_threat_cooldowns[idx] <= 0.0 {
                events.push(WildlifeEvent::ColonyThreatened {
                    colony_name: col.name,
                    cx: col.cx,
                    cy: col.cy,
                });
                self.colony_threat_cooldowns[idx] = COLONY_THREAT_COOLDOWN;
            }

            // 啟動聚落範圍內同種動物的守衛行為。
            let activate_r2 = (col.guard_radius * COLONY_ACTIVATE_MULTIPLIER).powi(2);
            let col_kind = col.kind;
            let col_cx = col.cx;
            let col_cy = col.cy;
            for a in &mut self.animals {
                if !a.alive || a.kind != col_kind { continue; }
                let ddx = a.x - col_cx;
                let ddy = a.y - col_cy;
                if ddx * ddx + ddy * ddy > activate_r2 { continue; }
                // 不干擾正在追獵/消化/已守衛的狀態。
                if matches!(a.state, WildlifeState::Stalking { .. } | WildlifeState::Hunting { .. } | WildlifeState::Digesting { .. } | WildlifeState::Guarding { .. }) {
                    continue;
                }
                a.state = WildlifeState::Guarding { threat_x, threat_y, guard_timer: GUARD_DURATION };
            }
        }

        // ── Phase 2b-extra: 敵視物種主動偵測玩家（ROADMAP 144）─────────────
        // attitude < HOSTILE_ATTITUDE 的物種：不等聚落觸發，直接向附近玩家靠近。
        for a in &mut self.animals {
            if !a.alive { continue; }
            if matches!(a.state, WildlifeState::Stalking { .. } | WildlifeState::Hunting { .. } | WildlifeState::Digesting { .. } | WildlifeState::Guarding { .. }) {
                continue;
            }
            let kind_attitude = *attitudes.get(&a.kind).unwrap_or(&50);
            if kind_attitude >= HOSTILE_ATTITUDE { continue; }
            // 找 HOSTILE_DETECT_RADIUS 內最近的玩家。
            let threat = nearest_in_range(a.x, a.y, player_positions, HOSTILE_DETECT_RADIUS);
            if let Some((tx, ty)) = threat {
                a.state = WildlifeState::Guarding { threat_x: tx, threat_y: ty, guard_timer: GUARD_DURATION };
            }
        }

        // ── Phase 2c: 守衛行為 tick（ROADMAP 143 + 144）─────────────────────
        // 處理所有物種（獵物與捕食者）的 Guarding 狀態。
        // ROADMAP 144：若物種為敵視且動物已靠近玩家 HOSTILE_ATTACK_REACH 內，發出傷害事件。
        for i in 0..self.animals.len() {
            if !self.animals[i].alive { continue; }
            let WildlifeState::Guarding { threat_x, threat_y, guard_timer } = self.animals[i].state else { continue };
            let dx = threat_x - self.animals[i].x;
            let dy = threat_y - self.animals[i].y;
            let dist = (dx * dx + dy * dy).sqrt();
            let remaining = guard_timer - dt;

            // 敵視物種近身攻擊（ROADMAP 144）。
            let kind_attitude = *attitudes.get(&self.animals[i].kind).unwrap_or(&50);
            if kind_attitude < HOSTILE_ATTITUDE && dist <= HOSTILE_ATTACK_REACH {
                events.push(WildlifeEvent::WildlifeAttack {
                    attacker_kind: self.animals[i].kind,
                    near_x: self.animals[i].x,
                    near_y: self.animals[i].y,
                    damage: HOSTILE_ATTACK_DAMAGE,
                });
                // 攻擊後回到休息（冷卻），再被 Phase 2b-extra 重新觸發。
                self.animals[i].state = WildlifeState::Resting { rest_timer: HOSTILE_ATTACK_COOLDOWN };
                continue;
            }

            if remaining <= 0.0 || dist < 30.0 {
                // 計時到或已靠近，回到休息。
                self.animals[i].state = WildlifeState::Resting { rest_timer: 2.0 };
            } else {
                self.animals[i].x += (dx / dist) * GUARD_SPEED * dt;
                self.animals[i].y += (dy / dist) * GUARD_SPEED * dt;
                self.animals[i].state = WildlifeState::Guarding { threat_x, threat_y, guard_timer: remaining };
            }
        }

        // ── Phase 3: 捕食者行為 ────────────────────────────────────────────────
        // 收集本幀的擊殺：(pred_id, prey_id, pred_kind, prey_kind, x, y)
        let mut kills: Vec<(u32, u32, WildlifeKind, WildlifeKind, f32, f32)> = Vec::new();

        for i in 0..self.animals.len() {
            if !self.animals[i].alive { continue; }
            if self.animals[i].kind.trophic_level() != TrophicLevel::Predator { continue; }
            // 守衛狀態已在 Phase 2c 處理，跳過。
            if matches!(self.animals[i].state, WildlifeState::Guarding { .. }) { continue; }

            let state = self.animals[i].state.clone();
            let pred_kind = self.animals[i].kind;
            let pred_id   = self.animals[i].id;
            let pred_x    = self.animals[i].x;
            let pred_y    = self.animals[i].y;

            // ROADMAP 214：母獸護幼——若有「自己所獵物種」的護幼成體已逼到威嚇半徑內，
            // 放棄狩獵、退走（被母獸趕跑）。在 match 前統一攔截：不論潛行/追獵/消化/閒晃，
            // 一隻挺身護幼的母鹿都能把附近的狼逼退（咬不到幼獸就先被趕走）。
            if let Some(target_kind) = pred_kind.hunts() {
                let intim_r2 = INTIMIDATE_RADIUS * INTIMIDATE_RADIUS;
                let driven_off = defending_snap.iter().any(|&(k, dx, dy)| {
                    k == target_kind && (dx - pred_x).powi(2) + (dy - pred_y).powi(2) <= intim_r2
                });
                if driven_off {
                    self.animals[i].state = WildlifeState::Returning;
                    continue;
                }
            }

            match state {
                WildlifeState::Hunting { target_id, hunt_timer } => {
                    if let Some(&(_, prey_kind, px, py)) = prey_snap.iter()
                        .find(|&&(id, _, _, _)| id == target_id)
                    {
                        let dx = px - pred_x;
                        let dy = py - pred_y;
                        let dist = (dx * dx + dy * dy).sqrt();
                        if dist <= KILL_RADIUS {
                            kills.push((pred_id, target_id, pred_kind, prey_kind, px, py));
                            self.animals[i].state = WildlifeState::Digesting { timer: DIGEST_DURATION };
                        } else {
                            self.animals[i].x += dx / dist * HUNT_SPEED * dt;
                            self.animals[i].y += dy / dist * HUNT_SPEED * dt;
                            let remaining = hunt_timer - dt;
                            self.animals[i].state = if remaining <= 0.0 {
                                WildlifeState::Returning
                            } else {
                                WildlifeState::Hunting { target_id, hunt_timer: remaining }
                            };
                        }
                    } else {
                        // 獵物已死或不見，放棄。
                        self.animals[i].state = WildlifeState::Returning;
                    }
                }
                WildlifeState::Stalking { target_id, stalk_timer } => {
                    // ROADMAP 213：孤獵潛行——壓低身子緩緩潛近鎖定的獵物，逼到撲擊距離即爆衝。
                    if let Some(&(_, _, px, py)) = prey_snap.iter()
                        .find(|&&(id, _, _, _)| id == target_id)
                    {
                        match stalk_creep(pred_x, pred_y, px, py, dt) {
                            None => {
                                // 已進入撲擊距離——爆衝轉入全速追獵撲殺。
                                self.animals[i].state =
                                    WildlifeState::Hunting { target_id, hunt_timer: HUNT_TIMEOUT };
                            }
                            Some((nx, ny)) => {
                                self.animals[i].x = nx;
                                self.animals[i].y = ny;
                                let remaining = stalk_timer - dt;
                                // 潛行有耐性上限：耗盡仍追不到就放棄返家，不永遠尾隨。
                                self.animals[i].state = if remaining <= 0.0 {
                                    WildlifeState::Returning
                                } else {
                                    WildlifeState::Stalking { target_id, stalk_timer: remaining }
                                };
                            }
                        }
                    } else {
                        // 獵物已死或不見，放棄。
                        self.animals[i].state = WildlifeState::Returning;
                    }
                }
                WildlifeState::Digesting { timer } => {
                    let remaining = timer - dt;
                    if remaining <= 0.0 {
                        let home_x = self.animals[i].home_x;
                        let home_y = self.animals[i].home_y;
                        let (tx, ty) = random_target(home_x, home_y, WANDER_RADIUS, &mut self.rng);
                        self.animals[i].state = WildlifeState::Wandering { target_x: tx, target_y: ty, wander_timer: 5.0 };
                    } else {
                        self.animals[i].state = WildlifeState::Digesting { timer: remaining };
                    }
                }
                _ => {
                    // 閒晃/返家：嘗試找獵物。ROADMAP 210：夜行掠食者入夜後搜尋範圍放大。
                    if let Some(target_kind) = pred_kind.hunts() {
                        let hunt_r2 = night_hunt_radius(is_night).powi(2);
                        let nearest = prey_snap.iter()
                            .filter(|&&(_, k, _, _)| k == target_kind)
                            .filter(|&&(_, _, px, py)| {
                                let dx = px - pred_x;
                                let dy = py - pred_y;
                                dx * dx + dy * dy <= hunt_r2
                            })
                            .min_by(|&&(_, _, ax, ay), &&(_, _, bx, by)| {
                                let da = (ax - pred_x).powi(2) + (ay - pred_y).powi(2);
                                let db = (bx - pred_x).powi(2) + (by - pred_y).powi(2);
                                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
                            });
                        if let Some(&(target_id, _, npx, npy)) = nearest {
                            // ROADMAP 213：孤獵潛行突襲——發現獵物後，若已貼到撲擊距離內就直接撲
                            // （Hunting 全速撲殺），否則先進潛行（Stalking）壓低身子緩緩逼近。
                            let dist = ((npx - pred_x).powi(2) + (npy - pred_y).powi(2)).sqrt();
                            self.animals[i].state = if within_pounce_range(dist) {
                                WildlifeState::Hunting { target_id, hunt_timer: HUNT_TIMEOUT }
                            } else {
                                WildlifeState::Stalking { target_id, stalk_timer: STALK_TIMEOUT }
                            };
                        } else {
                            // 無獵物，正常閒晃（捕食者不怕玩家，傳空威脅；獨來獨往不群聚）。
                            let rng = &mut self.rng;
                            let a = &mut self.animals[i];
                            // ROADMAP 217：掠食者夜嚎——夜裡無獵可追的平靜空檔，偶爾仰首長嚎。
                            // 已在長嚎中就把這一聲嚎完（原地不動、計時倒數）；否則夜間歇息時以
                            // HOWL_PROB 開一段長嚎。白天、或正在巡遊/返家時一律照常閒晃（不嚎）。
                            // ROADMAP 218：群嚎呼應——夜間歇息的掠食者若「聽見」附近同類正在長嚎
                            // （HOWL_HEAR_RADIUS 內），會以較高的 HOWL_JOIN_PROB 被牽動接嚎；沒聽見
                            // 才退回 217 的低機率自發起頭。本幀新接嚎者不在 howling_snap 裡，故嚎聲
                            // 逐圈外傳、此起彼落，而非同幀整片齊嚎。
                            if matches!(a.state, WildlifeState::Howling { .. }) {
                                a.tick_howl(dt, rng);
                            } else if is_night && matches!(a.state, WildlifeState::Resting { .. })
                                && {
                                    let join = hears_howl(a.x, a.y, &howling_snap)
                                        && rng.gen::<f32>() < HOWL_JOIN_PROB;
                                    join || rng.gen::<f32>() < HOWL_PROB
                                }
                            {
                                let timer = rng.gen_range(HOWL_DURATION_MIN..=HOWL_DURATION_MAX);
                                a.state = WildlifeState::Howling { howl_timer: timer };
                            } else {
                                // ROADMAP 211：掠食者（狼/狐）不吃草——graze_prob 永遠傳 0。
                                a.tick_idle(dt, &[], PRED_WANDER_SPEED, None, 0.0, rng);
                            }
                        }
                    }
                }
            }
        }

        // ROADMAP 209：驚群炸開——本幀開始時「正在逃竄」的獵物快照（id/kind/x/y/vx/vy），
        // 供恐慌連鎖判定。刻意在 Phase 4 變更前取一次：被感染者本幀不再回傳到此快照，
        // 故恐慌每 tick 只向外傳一圈，看起來像一波由威脅源擴散開的炸群（不會瞬間全炸）。
        let fleeing_snap: Vec<(u32, WildlifeKind, f32, f32, f32, f32)> = self.animals.iter()
            .filter(|a| a.alive && a.kind.trophic_level() == TrophicLevel::Prey)
            .filter_map(|a| match a.state {
                WildlifeState::Fleeing { vx, vy, .. } => Some((a.id, a.kind, a.x, a.y, vx, vy)),
                _ => None,
            })
            .collect();

        // ROADMAP 214：母獸護幼——蒐集「正被掠食者鎖定（潛行/追獵）的幼獸」快照
        // （juv_id/juv_kind/jx/jy/pred_x/pred_y）。由 Phase 3 剛更新過的掠食者狀態即時推得，
        // 供下方 Phase 4 讓離該幼獸最近的同種成體（母獸）挺身護衛。動物總數少（~22），O(n²) 無虞。
        let threatened_juv_snap: Vec<(u32, WildlifeKind, f32, f32, f32, f32)> = {
            let mut v = Vec::new();
            for a in self.animals.iter() {
                if !a.alive || a.kind.trophic_level() != TrophicLevel::Predator { continue; }
                let target_id = match a.state {
                    WildlifeState::Stalking { target_id, .. } | WildlifeState::Hunting { target_id, .. } => target_id,
                    _ => continue,
                };
                if let Some(juv) = self.animals.iter()
                    .find(|j| j.id == target_id && j.alive && j.is_juvenile())
                {
                    v.push((juv.id, juv.kind, juv.x, juv.y, a.x, a.y));
                }
            }
            v
        };

        // ── Phase 4: 獵物行為（閒晃 + 逃離玩家/捕食者） ─────────────────────
        for i in 0..self.animals.len() {
            if !self.animals[i].alive { continue; }
            if self.animals[i].kind.trophic_level() != TrophicLevel::Prey { continue; }
            // 守衛狀態已在 Phase 2c 處理，跳過正常閒晃（不逃跑）。
            if matches!(self.animals[i].state, WildlifeState::Guarding { .. }) { continue; }

            let animal_kind = self.animals[i].kind;
            // ROADMAP 205：被馴養的個體把玩家當朋友（不逃跑），未馴養則沿用 144 物種態度判定。
            let tamed = self.animals[i].is_tamed();

            // ROADMAP 214：母獸護幼——成體優先為「被掠食者鎖定的同種幼獸」挺身（凌駕自身逃跑：
            // 母獸不顧自己的恐懼，衝去擋在幼獸與狼之間）。只有「離受脅幼獸最近的同種成體」會出面，
            // 故每隻受脅幼獸至多由一隻母獸護衛、不致整群暴衝。幼獸本身不護幼（牠是被護的一方）。
            if !self.animals[i].is_juvenile() {
                let ax = self.animals[i].x;
                let ay = self.animals[i].y;
                if let Some((tx, ty)) = defend_target(
                    self.animals[i].id, animal_kind, ax, ay, &threatened_juv_snap, &adult_snap,
                ) {
                    let (nx, ny) = defend_charge(ax, ay, tx, ty, dt);
                    self.animals[i].x = nx;
                    self.animals[i].y = ny;
                    self.animals[i].state = WildlifeState::Defending;
                    continue;
                }
                // 已無需護衛的幼獸（剛把狼趕走 / 威脅解除）：若仍處 Defending 就收斂回返家，
                // 下方/次幀再走正常閒晃，不會卡在護衛姿態。
                if matches!(self.animals[i].state, WildlifeState::Defending) {
                    self.animals[i].state = WildlifeState::Returning;
                }
            }

            // 威脅 = 捕食者 + ROADMAP 165 獵食此物種的怪物——馴養與否都仍會逃離掠食者（信任的是你、不是狼）。
            let mut threats: Vec<(f32, f32)> = pred_positions.clone();
            for &(mk, mx, my) in monster_threats {
                if monster_hunts_wildlife(mk) == Some(animal_kind) {
                    threats.push((mx, my));
                }
            }
            // ROADMAP 144：未馴養且物種對人類不夠友善時，玩家也算威脅。
            let kind_attitude = *attitudes.get(&animal_kind).unwrap_or(&50);
            if !tamed && kind_attitude < FRIENDLY_ATTITUDE {
                threats.extend_from_slice(player_positions);
            }

            // ROADMAP 209：驚群炸開——自己附近沒有「直接威脅」（否則交給下方 tick_idle 算
            // 正確的背向威脅逃竄），但有同種夥伴正在近旁逃竄時，被恐慌感染、朝同伴逃竄的
            // 方向一起炸開。恐慌優先於馴養跟隨/幼獸依偎/群聚閒晃——連你養熟的鹿也會跟著炸群。
            if !matches!(self.animals[i].state, WildlifeState::Fleeing { .. }) {
                let ax = self.animals[i].x;
                let ay = self.animals[i].y;
                let direct_threat = nearest_in_range(ax, ay, &threats, FLEE_RADIUS).is_some();
                if !direct_threat {
                    if let Some((vx, vy)) = panic_velocity_from_herd(
                        self.animals[i].id, animal_kind, ax, ay, &fleeing_snap, ALARM_RADIUS,
                    ) {
                        self.animals[i].state = WildlifeState::Fleeing {
                            vx, vy, flee_timer: ALARM_FLEE_DURATION,
                        };
                        continue;
                    }
                }
            }

            // ROADMAP 205：馴養個體在沒有掠食者威脅時，溫順地走向附近玩家、保持舒適距離（彷彿跟著你）。
            if tamed {
                let ax = self.animals[i].x;
                let ay = self.animals[i].y;
                let fleeing_now = matches!(self.animals[i].state, WildlifeState::Fleeing { .. });
                let predator_near = nearest_in_range(ax, ay, &threats, FLEE_RADIUS).is_some();
                if !fleeing_now && !predator_near {
                    if let Some((px, py)) = nearest_in_range(ax, ay, player_positions, FOLLOW_RANGE) {
                        let dx = px - ax;
                        let dy = py - ay;
                        let dist = (dx * dx + dy * dy).sqrt();
                        if dist > FOLLOW_COMFORT_DIST {
                            self.animals[i].x += dx / dist * FOLLOW_SPEED * dt;
                            self.animals[i].y += dy / dist * FOLLOW_SPEED * dt;
                        }
                        // 朝向玩家的溫順狀態（已到舒適距離則原地陪著你）。
                        self.animals[i].state = WildlifeState::Wandering { target_x: px, target_y: py, wander_timer: 1.0 };
                        continue;
                    }
                }
            }

            // ROADMAP 208：幼獸依偎母獸——未受威脅時，幼獸主動靠近並跟隨最近的同種成體
            // （像小鹿緊跟母鹿）；附近沒有成體則退回正常閒晃/群聚。仍會逃離掠食者（威脅優先）。
            if self.animals[i].is_juvenile() {
                let ax = self.animals[i].x;
                let ay = self.animals[i].y;
                let fleeing_now = matches!(self.animals[i].state, WildlifeState::Fleeing { .. });
                let predator_near = nearest_in_range(ax, ay, &threats, FLEE_RADIUS).is_some();
                if !fleeing_now && !predator_near {
                    if let Some((px, py)) = nearest_adult_of_kind(
                        self.animals[i].id, animal_kind, ax, ay, &adult_snap, NURSE_RANGE,
                    ) {
                        // ROADMAP 215：幼獸嬉戲——已在玩耍中的幼獸繼續這一段蹦跳（圍著母獸
                        // (px,py)），蹦到落點或計時耗盡就在 tick_frolic 裡收斂回依偎。受威脅
                        // 一律優先逃（上方 predator_near 已擋），故玩耍只在平靜時延續。
                        if matches!(self.animals[i].state, WildlifeState::Frolicking { .. }) {
                            self.animals[i].tick_frolic(dt, px, py);
                            continue;
                        }
                        let dx = px - ax;
                        let dy = py - ay;
                        let dist = (dx * dx + dy * dy).sqrt();
                        if dist > NURSE_COMFORT_DIST {
                            self.animals[i].x += dx / dist * NURSE_SPEED * dt;
                            self.animals[i].y += dy / dist * NURSE_SPEED * dt;
                            // 還在追母獸：依偎於母獸的溫順狀態。
                            self.animals[i].state = WildlifeState::Wandering { target_x: px, target_y: py, wander_timer: 1.0 };
                            continue;
                        }
                        // ROADMAP 215：已依偎到母獸身邊（舒適距離內）——白天平靜時有 FROLIC_PROB
                        // 機率開一段嬉戲（在媽媽周圍蹦跳玩耍 ✨），否則靜靜依偎。夜間只依偎不玩耍。
                        if !is_night && self.rng.gen::<f32>() < FROLIC_PROB {
                            let (hx, hy) = frolic_target(px, py, &mut self.rng);
                            let timer = self.rng.gen_range(FROLIC_DURATION_MIN..=FROLIC_DURATION_MAX);
                            self.animals[i].state = WildlifeState::Frolicking { hop_x: hx, hop_y: hy, frolic_timer: timer };
                        } else {
                            self.animals[i].state = WildlifeState::Wandering { target_x: px, target_y: py, wander_timer: 1.0 };
                        }
                        continue;
                    }
                }
            }

            // ROADMAP 206：群聚結伴——算出附近同種夥伴的平均位置（群體中心），
            // 作為下一個漫遊目標的拉力；HERD_RADIUS 內無同種夥伴則 None（退回純隨機）。
            let herd_anchor = herd_center(
                self.animals[i].id, animal_kind, self.animals[i].x, self.animals[i].y, &prey_snap,
            );

            // ROADMAP 212：群體警戒哨——白天成群的成體獵物中，由群內 id 最小那隻擔任哨兵。
            // 哨兵不吃草、抬頭放哨（警戒半徑放大），比埋頭的同伴更早察覺危險、率先炸群（再經
            // 209 帶動全群）。只在白天、未在逃竄、成體、且確實成群時成立；其餘照常閒晃/吃草。
            let act_as_sentinel = !is_night
                && is_diurnal(animal_kind)
                && !self.animals[i].is_juvenile()
                && !matches!(self.animals[i].state, WildlifeState::Fleeing { .. })
                && herd_sentinel(
                    self.animals[i].id, animal_kind, self.animals[i].x, self.animals[i].y, &adult_snap,
                );

            // ROADMAP 210：晝夜作息——夜間，晝行獵物若平靜（未在逃竄、附近也無威脅），
            // 就歸巢沉睡而非繼續閒晃；白天、或有威脅/逃竄時一律走原本的閒晃/逃竄邏輯
            // （威脅永遠優先，tick_idle 內部會先處理逃跑）。
            let calm_at_night = is_night
                && is_diurnal(animal_kind)
                && !matches!(self.animals[i].state, WildlifeState::Fleeing { .. })
                && nearest_in_range(self.animals[i].x, self.animals[i].y, &threats, FLEE_RADIUS).is_none();

            // ROADMAP 216：成體相依理毛——白天的成體獵物若身邊有同種成體夥伴（GROOM_RADIUS 內），
            // 在歇息的當口偶爾轉去互相理毛。此處先判定「是否有可理毛的夥伴」（重用 208 的最近同種
            // 成體查詢）；幼獸／逃竄中／夜間一律不理毛（走依偎/逃竄/夜眠分支），故順手短路。
            let groom_has_partner = !is_night
                && !self.animals[i].is_juvenile()
                && !matches!(self.animals[i].state, WildlifeState::Fleeing { .. })
                && nearest_adult_of_kind(
                    self.animals[i].id, animal_kind, self.animals[i].x, self.animals[i].y, &adult_snap, GROOM_RADIUS,
                ).is_some();

            let rng = &mut self.rng;
            let a = &mut self.animals[i];
            if calm_at_night {
                a.tick_night_rest(dt);
            } else if act_as_sentinel {
                // ROADMAP 212：哨兵——放大警戒半徑內若有威脅，率先背向炸出逃竄（次幀經 209
                // 感染全群一起炸開）；無威脅則站崗放哨（抬頭警戒、不吃草）。
                if let Some((tx, ty)) = nearest_in_range(a.x, a.y, &threats, SENTINEL_FLEE_RADIUS) {
                    a.flee_from(tx, ty);
                } else {
                    a.tick_watch(dt, herd_anchor, rng);
                }
            } else {
                // ROADMAP 210：破曉甦醒——天亮（非夜間）後，仍處夜眠的晝行獵物先喚醒；
                // 否則 600s 夜眠計時器比整段白天還長，鹿會癱在家裡跨越整個白天。
                // ROADMAP 219：喚醒不再瞬間起步，而是先轉入「伸展」（Waking）一小段再起身漫遊。
                if !is_night && is_diurnal(animal_kind) {
                    a.wake_from_night_sleep(rng);
                }
                // ROADMAP 216：成體相依理毛——理毛永遠讓位給逃命（威脅優先）。先看附近有無威脅：
                let threat_near = nearest_in_range(a.x, a.y, &threats, FLEE_RADIUS).is_some();
                let is_bird = animal_kind == WildlifeKind::WildBird;
                let is_critter = animal_kind == WildlifeKind::SmallCritter;
                if matches!(a.state, WildlifeState::Watching { .. }) {
                    // ROADMAP 212 修補：走到此處代表本隻已非自群哨兵（act_as_sentinel 為偽——
                    // 例如有更小 id 的同種成體漂進了 SENTINEL_HERD_RADIUS、接手放哨），卻仍滯留在
                    // Watching。若不主動釋放，tick_idle 的 catch-all（`_ => {}`）會讓牠永遠卡在放哨，
                    // 形成「一群兩哨」。這裡把卸任的哨兵收斂回休息，下一幀再交還一般作息（漫遊/吃草）。
                    let rest = rng.gen_range(REST_TIMER_MIN..=REST_TIMER_MAX);
                    a.state = WildlifeState::Resting { rest_timer: rest };
                } else if matches!(a.state, WildlifeState::Waking { .. }) {
                    // ROADMAP 219：破曉伸展中——威脅一旦逼近就立刻中斷改逃竄（睡醒遇險先逃命），
                    // 否則原地舒展身子、計時倒數，到期才起身投入新一天的閒晃。
                    if let Some((tx, ty)) = nearest_in_range(a.x, a.y, &threats, FLEE_RADIUS) {
                        a.flee_from(tx, ty);
                    } else {
                        a.tick_wake(dt, herd_anchor, rng);
                    }
                } else if is_bird && matches!(a.state, WildlifeState::Flying { .. }) {
                    // ROADMAP 220：已在空中盤旋——威脅一旦逼近就立刻降下逃竄（飛行是悠閒的盤旋、
                    // 不是逃命手段），否則繞著群心繼續盤旋、計時倒數，到期降落回閒晃。
                    if let Some((tx, ty)) = nearest_in_range(a.x, a.y, &threats, FLEE_RADIUS) {
                        a.flee_from(tx, ty);
                    } else {
                        a.tick_fly(dt, herd_anchor, rng);
                    }
                } else if is_bird && matches!(a.state, WildlifeState::Chirping { .. }) {
                    // ROADMAP 221：已在啁啾中——威脅一旦逼近就立刻收聲改逃竄（鳴叫永遠讓位逃命），
                    // 否則原地把這一段鳴唱走完、計時倒數，到期回到閒晃。
                    if let Some((tx, ty)) = nearest_in_range(a.x, a.y, &threats, FLEE_RADIUS) {
                        a.flee_from(tx, ty);
                    } else {
                        a.tick_chirp(dt, herd_anchor, rng);
                    }
                } else if is_bird
                    && !is_night
                    && !threat_near
                    && matches!(a.state, WildlifeState::Resting { .. } | WildlifeState::Wandering { .. })
                    && {
                        // ROADMAP 220：白天平靜的野鳥——看見附近同類升空（FLIGHT_HEAR_RADIUS 內）便以
                        // 較高的 FLIGHT_JOIN_PROB 被牽動跟著飛起（接力）；沒看見才退回低機率 FLIGHT_PROB
                        // 自發起飛。本幀新起飛者不在 flying_snap 裡，故升空逐圈外擴、整群錯落而起。
                        let join = sees_flight(a.x, a.y, &flying_snap) && rng.gen::<f32>() < FLIGHT_JOIN_PROB;
                        join || rng.gen::<f32>() < FLIGHT_PROB
                    }
                {
                    // 振翅升空：繞著群心盤旋（起始角隨機，整群各以不同角繞同一群心 → 一群鳥一起盤旋）。
                    let timer = rng.gen_range(FLIGHT_DURATION_MIN..=FLIGHT_DURATION_MAX);
                    let angle: f32 = rng.gen_range(0.0..std::f32::consts::TAU);
                    a.state = WildlifeState::Flying { fly_timer: timer, angle };
                } else if is_bird
                    && !is_night
                    && !threat_near
                    && matches!(a.state, WildlifeState::Resting { .. } | WildlifeState::Wandering { .. })
                    && {
                        // ROADMAP 221：白天平靜的野鳥——聽見附近同類啁啾（CHIRP_HEAR_RADIUS 內）便以
                        // 較高的 CHIRP_JOIN_PROB 被牽動跟著起鳴（接力）；沒聽見才退回低機率 CHIRP_PROB
                        // 自發起鳴。本幀新起鳴者不在 chirping_snap 裡，故鳴聲逐圈外擴、整群錯落而鳴。
                        let join = hears_song(a.x, a.y, &chirping_snap) && rng.gen::<f32>() < CHIRP_JOIN_PROB;
                        join || rng.gen::<f32>() < CHIRP_PROB
                    }
                {
                    // 停下啁啾：原地仰首鳴唱一小段（頭頂浮 🎵）。
                    let timer = rng.gen_range(CHIRP_DURATION_MIN..=CHIRP_DURATION_MAX);
                    a.state = WildlifeState::Chirping { chirp_timer: timer };
                } else if is_critter && matches!(a.state, WildlifeState::Nibbling { .. }) {
                    // ROADMAP 222：已在啃咬中——威脅一旦逼近就立刻丟食改逃竄（覓食永遠讓位逃命），
                    // 否則原地把這一段啃完、計時倒數，到期回到閒晃。
                    if let Some((tx, ty)) = nearest_in_range(a.x, a.y, &threats, FLEE_RADIUS) {
                        a.flee_from(tx, ty);
                    } else {
                        a.tick_nibble(dt, herd_anchor, rng);
                    }
                } else if is_critter
                    && !is_night
                    && !threat_near
                    && matches!(a.state, WildlifeState::Resting { .. } | WildlifeState::Wandering { .. })
                    && rng.gen::<f32>() < NIBBLE_PROB
                {
                    // ROADMAP 222：白天平靜的小動物——偶爾坐起來捧食啃咬一小段（頭頂浮 🌰）。
                    // 各顧各的、不傳染（與鳥的飛／鳴呼應刻意區隔），只是一隻隻自顧自地坐起來啃。
                    let timer = rng.gen_range(NIBBLE_DURATION_MIN..=NIBBLE_DURATION_MAX);
                    a.state = WildlifeState::Nibbling { nibble_timer: timer };
                } else if matches!(a.state, WildlifeState::Grooming { .. }) && !threat_near {
                    // 已在理毛中且仍平靜：把這一段梳理走完（原地不動、計時倒數）。
                    a.tick_groom(dt, herd_anchor, rng);
                } else if groom_has_partner
                    && !threat_near
                    && matches!(a.state, WildlifeState::Resting { .. })
                    && rng.gen::<f32>() < GROOM_PROB
                {
                    // 白天歇息的成體、身邊有同種夥伴、平靜——偶爾轉去互相理毛（頭頂浮 💕）。
                    let timer = rng.gen_range(GROOM_DURATION_MIN..=GROOM_DURATION_MAX);
                    a.state = WildlifeState::Grooming { groom_timer: timer };
                } else {
                    // ROADMAP 211：白晝吃草——只有白天的晝行獵物才會吃草（夜間傳 0：夜眠不吃草）。
                    // Phase 4 本就只處理獵物，故此處 is_diurnal 恆真；以 is_night 區隔晝夜即可。
                    // （理毛中卻有威脅逼近時也落到這裡，tick_idle 內會先轉逃竄——威脅永遠優先。）
                    let graze_prob = if is_night { 0.0 } else { GRAZE_PROB };
                    a.tick_idle(dt, &threats, WANDER_SPEED, herd_anchor, graze_prob, rng);
                }
            }
        }

        // ── Phase 5: 套用擊殺 ──────────────────────────────────────────────────
        for (pred_id, prey_id, pred_kind, prey_kind, kx, ky) in kills {
            // 將獵物標記為死亡。
            if let Some(prey) = self.animals.iter_mut().find(|a| a.id == prey_id) {
                prey.alive = false;
                prey.respawn_timer = PREY_RESPAWN_SECS;
                prey.state = WildlifeState::Resting { rest_timer: 0.0 };
            }
            // 確認捕食者仍存在（應為不死，但安全起見檢查）。
            let _ = pred_id;
            // 限流廣播：30 秒內最多一條。
            if self.kill_broadcast_cooldown <= 0.0 {
                events.push(WildlifeEvent::Kill { predator_kind: pred_kind, prey_kind, x: kx, y: ky });
                self.kill_broadcast_cooldown = KILL_BROADCAST_INTERVAL;
            }
            // ROADMAP 142：在死亡位置生成乙太微粒（上限 MAX_CARION_ORBS）。
            if self.carion_orbs.len() < MAX_CARION_ORBS {
                let id = self.orb_counter;
                self.orb_counter = self.orb_counter.wrapping_add(1);
                self.carion_orbs.push(CarrionOrb { id, x: kx, y: ky, ttl: CARION_ORB_TTL });
            }
        }

        // ── Phase 6: 族群繁衍（ROADMAP 207）────────────────────────────────────
        // 各獵物物種：若有一群安穩成群的成體（彼此聚在 BREED_RADIUS 內、且群心附近沒有
        // 捕食者），就持續累積「安穩成群」秒數；達門檻且未達族群上限時，在群體中心誕生
        // 一隻幼獸。受擾或滿額時進度緩退，群體散開時更快流失——繁衍是難得、需要安穩的成果。
        for kind in BREEDING_KINDS {
            let center = breeding_cluster_center(&self.animals, kind, BREED_RADIUS, BREED_HERD_MIN);
            let total = species_total(&self.animals, kind);
            let cap = species_cap(kind);

            let mut born_at: Option<(f32, f32)> = None;
            {
                let prog = self.breed_progress.entry(kind).or_insert(0.0);
                match center {
                    Some((cx, cy)) => {
                        let disturbed = nearest_in_range(cx, cy, &pred_positions, BREED_DISTURB_RADIUS).is_some();
                        if !disturbed && total < cap {
                            *prog += dt;
                            if *prog >= BREED_THRESHOLD_SECS {
                                *prog = 0.0;
                                born_at = Some((cx, cy));
                            }
                        } else {
                            // 群體緊張或已滿額：進度緩退（不立即歸零，保留一點韌性）。
                            *prog = (*prog - dt).max(0.0);
                        }
                    }
                    // 沒有成群：進度更快流失（散開的個體不繁衍）。
                    None => *prog = (*prog - dt * 2.0).max(0.0),
                }
            }

            if let Some((cx, cy)) = born_at {
                let id = self.next_animal_id;
                self.next_animal_id = self.next_animal_id.wrapping_add(1);
                self.animals.push(Wildlife::new_juvenile(id, kind, cx, cy, &mut self.rng));
                events.push(WildlifeEvent::Born { kind, x: cx, y: cy });
            }
        }

        events
    }
}

// ─── 輔助函式 ────────────────────────────────────────────────────────────────

fn nearest_in_range(ax: f32, ay: f32, pts: &[(f32, f32)], radius: f32) -> Option<(f32, f32)> {
    let r2 = radius * radius;
    pts.iter()
        .filter(|&&(px, py)| {
            let dx = px - ax;
            let dy = py - ay;
            dx * dx + dy * dy <= r2
        })
        .min_by(|&&(ax2, ay2), &&(bx2, by2)| {
            let da = (ax2 - ax).powi(2) + (ay2 - ay).powi(2);
            let db = (bx2 - ax).powi(2) + (by2 - ay).powi(2);
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        })
        .copied()
}

/// ROADMAP 218：群嚎呼應——位於 (px,py) 的掠食者是否「聽得見」附近任一正在長嚎的同類
/// （`howling` 為本幀起始時正在長嚎者的座標快照）。只要有一聲嚎在 HOWL_HEAR_RADIUS 內就回 true，
/// 由呼叫端據此（以 HOWL_JOIN_PROB）牽動牠接嚎。純距離判定、不分種（狼狐皆會對嚎），無副作用。
fn hears_howl(px: f32, py: f32, howling: &[(f32, f32)]) -> bool {
    let r2 = HOWL_HEAR_RADIUS * HOWL_HEAR_RADIUS;
    howling.iter().any(|&(hx, hy)| {
        let dx = hx - px;
        let dy = hy - py;
        dx * dx + dy * dy <= r2
    })
}

/// ROADMAP 220：鳥群振翅升空盤旋——位於 (px,py) 的野鳥是否「看見」附近任一正在盤旋的同類
/// （`flying` 為本幀起始時正在盤旋者的座標快照）。只要有一隻在 FLIGHT_HEAR_RADIUS 內就回 true，
/// 由呼叫端據此（以 FLIGHT_JOIN_PROB）牽動牠跟著飛起。純距離判定、無副作用。
fn sees_flight(px: f32, py: f32, flying: &[(f32, f32)]) -> bool {
    let r2 = FLIGHT_HEAR_RADIUS * FLIGHT_HEAR_RADIUS;
    flying.iter().any(|&(fx, fy)| {
        let dx = fx - px;
        let dy = fy - py;
        dx * dx + dy * dy <= r2
    })
}

/// ROADMAP 221：晝日鳥鳴呼應——位於 (px,py) 的野鳥是否「聽得見」附近任一正在啁啾的同類
/// （`chirping` 為本幀起始時正在啁啾者的座標快照）。只要有一聲鳴在 CHIRP_HEAR_RADIUS 內就回 true，
/// 由呼叫端據此（以 CHIRP_JOIN_PROB）牽動牠跟鳴。純距離判定、無副作用（仿 218 的 hears_howl）。
fn hears_song(px: f32, py: f32, chirping: &[(f32, f32)]) -> bool {
    let r2 = CHIRP_HEAR_RADIUS * CHIRP_HEAR_RADIUS;
    chirping.iter().any(|&(cx, cy)| {
        let dx = cx - px;
        let dy = cy - py;
        dx * dx + dy * dy <= r2
    })
}

fn random_target(hx: f32, hy: f32, radius: f32, rng: &mut StdRng) -> (f32, f32) {
    let angle: f32 = rng.gen_range(0.0..std::f32::consts::TAU);
    let dist: f32  = rng.gen_range(0.0..radius);
    (hx + angle.cos() * dist, hy + angle.sin() * dist)
}

/// ROADMAP 206：附近同種存活獵物的平均位置（不含自己），即「群體中心」。
/// 只統計 `HERD_RADIUS` 內、同 `kind` 的個體；範圍內無夥伴則回 `None`。
/// 純函式（吃 `prey_snap` 快照），便於測試。
fn herd_center(
    self_id: u32,
    kind: WildlifeKind,
    x: f32,
    y: f32,
    prey_snap: &[(u32, WildlifeKind, f32, f32)],
) -> Option<(f32, f32)> {
    let r2 = HERD_RADIUS * HERD_RADIUS;
    let mut sx = 0.0_f32;
    let mut sy = 0.0_f32;
    let mut n = 0u32;
    for &(id, k, px, py) in prey_snap {
        if id == self_id || k != kind {
            continue;
        }
        let dx = px - x;
        let dy = py - y;
        if dx * dx + dy * dy <= r2 {
            sx += px;
            sy += py;
            n += 1;
        }
    }
    if n == 0 {
        None
    } else {
        Some((sx / n as f32, sy / n as f32))
    }
}

/// ROADMAP 208：幼獸依偎——在 `adult_snap`（同種成體位置快照）中，找出離 (ax,ay) 最近、
/// 且距離在 `radius` 內的同種成體位置作為依偎對象；範圍內無同種成體則 `None`。
/// 排除自己（理論上幼獸本就不在成體快照裡，仍以 id 保險）。純函式，便於測試。
fn nearest_adult_of_kind(
    self_id: u32,
    kind: WildlifeKind,
    ax: f32,
    ay: f32,
    adult_snap: &[(u32, WildlifeKind, f32, f32)],
    radius: f32,
) -> Option<(f32, f32)> {
    let r2 = radius * radius;
    adult_snap.iter()
        .filter(|&&(id, k, _, _)| id != self_id && k == kind)
        .map(|&(_, _, px, py)| (px, py, (px - ax).powi(2) + (py - ay).powi(2)))
        .filter(|&(_, _, d2)| d2 <= r2)
        .min_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(px, py, _)| (px, py))
}

/// ROADMAP 215：幼獸嬉戲——在母獸 (mx,my) 周圍 FROLIC_RADIUS 內隨機挑一個蹦跳落點。
/// 角度與半徑皆隨機（半徑取 sqrt 讓落點在圓內均勻分布），故每段嬉戲蹦的方向都不同、由 rng
/// 即時決定（湧現不寫死），且永遠圍著母獸（玩不離媽媽）。純函式，便於測試。
fn frolic_target(mx: f32, my: f32, rng: &mut StdRng) -> (f32, f32) {
    let angle = rng.gen_range(0.0_f32..std::f32::consts::TAU);
    let r = FROLIC_RADIUS * rng.gen_range(0.0_f32..=1.0).sqrt();
    (mx + angle.cos() * r, my + angle.sin() * r)
}

/// ROADMAP 215：幼獸嬉戲——朝蹦跳落點 (hx,hy) 以 FROLIC_SPEED 移動一幀，回傳新位置。
/// 單幀位移受「到落點的剩餘距離」clamp，故永遠不會蹦過頭（最多剛好到達落點）。純函式。
fn frolic_hop(x: f32, y: f32, hx: f32, hy: f32, dt: f32) -> (f32, f32) {
    let dx = hx - x;
    let dy = hy - y;
    let dist = (dx * dx + dy * dy).sqrt();
    if dist <= 1e-3 {
        return (x, y);
    }
    let step = (FROLIC_SPEED * dt).min(dist);
    (x + dx / dist * step, y + dy / dist * step)
}

/// ROADMAP 209：驚群炸開——在 `fleeing_snap`（正在逃竄的同種獵物：id/kind/x/y/vx/vy）中，
/// 找離 (ax,ay) 最近、且距離在 `radius` 內的同種逃竄夥伴，回傳其逃竄方向（正規化後乘
/// FLEE_SPEED）作為被感染者的逃竄速度——於是整群朝同一方向一起炸開（恐慌如漣漪傳開）。
/// 範圍內無逃竄同伴則 `None`。排除自己。純函式，便於測試。
fn panic_velocity_from_herd(
    self_id: u32,
    kind: WildlifeKind,
    ax: f32,
    ay: f32,
    fleeing_snap: &[(u32, WildlifeKind, f32, f32, f32, f32)],
    radius: f32,
) -> Option<(f32, f32)> {
    let r2 = radius * radius;
    fleeing_snap.iter()
        .filter(|&&(id, k, _, _, _, _)| id != self_id && k == kind)
        .map(|&(_, _, px, py, vx, vy)| (vx, vy, (px - ax).powi(2) + (py - ay).powi(2)))
        .filter(|&(_, _, d2)| d2 <= r2)
        .min_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(vx, vy, _)| {
            let len = (vx * vx + vy * vy).sqrt().max(1.0);
            (vx / len * FLEE_SPEED, vy / len * FLEE_SPEED)
        })
}

/// ROADMAP 210：晝夜作息——某種野生動物是否「晝行性」（白天活躍、入夜歸巢沉睡）。
/// 獵物晝行（鹿/鳥/小動物白天閒晃、夜裡睡覺）；掠食者夜行（狼/狐入夜更活躍地狩獵）。
/// 純函式，便於測試。
fn is_diurnal(kind: WildlifeKind) -> bool {
    kind.trophic_level() == TrophicLevel::Prey
}

/// ROADMAP 212：群體警戒哨——判定 (ax,ay) 這隻成體是否擔任所屬群的哨兵。
/// 在 `adult_snap`（同種成體位置快照，含自己）中，若 SENTINEL_HERD_RADIUS 內的同種成體
/// （含自己）達 SENTINEL_MIN_HERD 隻，且自己是其中 id 最小者，便由自己放哨。以「群內最小
/// id」去中心地推定，保證每群恰一隻哨兵、且穩定不抖動（不靠隨機、不需額外狀態）。純函式。
fn herd_sentinel(
    self_id: u32,
    kind: WildlifeKind,
    ax: f32,
    ay: f32,
    adult_snap: &[(u32, WildlifeKind, f32, f32)],
) -> bool {
    let r2 = SENTINEL_HERD_RADIUS * SENTINEL_HERD_RADIUS;
    let mut count = 0usize;
    let mut min_id = self_id;
    for &(oid, k, px, py) in adult_snap {
        if k != kind {
            continue;
        }
        if (px - ax).powi(2) + (py - ay).powi(2) <= r2 {
            count += 1;
            if oid < min_id {
                min_id = oid;
            }
        }
    }
    count >= SENTINEL_MIN_HERD && min_id == self_id
}

/// ROADMAP 210：掠食者本幀的狩獵搜尋半徑——夜行獵手入夜後覓食範圍放大（×NIGHT_HUNT_RADIUS_MULT）。
/// 純函式，便於測試。
fn night_hunt_radius(is_night: bool) -> f32 {
    if is_night { HUNT_RADIUS * NIGHT_HUNT_RADIUS_MULT } else { HUNT_RADIUS }
}

// ─── ROADMAP 213：孤獵潛行突襲純函式（可測） ─────────────────────────────────

/// 與獵物的距離是否已進入撲擊距離（true＝該爆衝轉入全速追獵）。純函式，便於測試。
fn within_pounce_range(dist: f32) -> bool {
    dist <= POUNCE_RANGE
}

/// 掠食者潛行接近獵物的逐幀位移（純函式，便於測試）。
/// 回傳 `None`＝已在撲擊距離內（呼叫端應爆衝轉入 Hunting）；
/// 回傳 `Some((nx, ny))`＝仍在潛近，以遠慢於追獵的 `STALK_SPEED` 壓低身子朝獵物 creep 後的新位置。
/// 單幀位移受「與獵物距離」上限約束，故潛近不會越過獵物（最多貼到原地）。
fn stalk_creep(pred_x: f32, pred_y: f32, prey_x: f32, prey_y: f32, dt: f32) -> Option<(f32, f32)> {
    let dx = prey_x - pred_x;
    let dy = prey_y - pred_y;
    let dist = (dx * dx + dy * dy).sqrt();
    if within_pounce_range(dist) {
        return None;
    }
    // 朝獵物移動 STALK_SPEED*dt，但不越過獵物（step 受 dist 上限）。
    let step = (STALK_SPEED * dt).min(dist);
    Some((pred_x + dx / dist * step, pred_y + dy / dist * step))
}

// ─── ROADMAP 214：母獸護幼純函式（可測） ─────────────────────────────────────

/// 在 `adult_snap`（同種成體位置快照）中，找出離 (jx,jy) 最近、且距離在 `radius` 內的
/// 同種成體 id；範圍內無同種成體則 `None`。供護幼判定「我是不是離這隻幼獸最近的成體」。
/// 純函式，便於測試。
fn nearest_adult_id_of_kind(
    kind: WildlifeKind,
    jx: f32,
    jy: f32,
    adult_snap: &[(u32, WildlifeKind, f32, f32)],
    radius: f32,
) -> Option<u32> {
    let r2 = radius * radius;
    adult_snap.iter()
        .filter(|&&(_, k, _, _)| k == kind)
        .map(|&(id, _, px, py)| (id, (px - jx).powi(2) + (py - jy).powi(2)))
        .filter(|&(_, d2)| d2 <= r2)
        .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(id, _)| id)
}

/// ROADMAP 214：母獸護幼——判定 (ax,ay) 這隻成體是否該為「被掠食者鎖定的同種幼獸」挺身護衛。
/// 在 `threatened`（受脅幼獸：juv_id/juv_kind/jx/jy/pred_x/pred_y）中，找出本成體
/// `DEFEND_GUARD_RADIUS` 內、同種、且「本成體就是離該幼獸最近的同種成體」的那隻幼獸，
/// 回傳其威脅掠食者的座標（成體應朝它衝去驅趕）；無則 `None`。
/// 「最近成體才護衛」確保每隻受脅幼獸至多由一隻（最近的那隻＝母獸）出面，不會整群一起暴衝。
/// 純函式，便於測試。
fn defend_target(
    self_id: u32,
    kind: WildlifeKind,
    ax: f32,
    ay: f32,
    threatened: &[(u32, WildlifeKind, f32, f32, f32, f32)],
    adult_snap: &[(u32, WildlifeKind, f32, f32)],
) -> Option<(f32, f32)> {
    let r2 = DEFEND_GUARD_RADIUS * DEFEND_GUARD_RADIUS;
    // 在所有「我該護的」幼獸中，挑離我最近的那隻去護（自己只有一個身子）。
    threatened.iter()
        .filter(|&&(_, jkind, _, _, _, _)| jkind == kind)
        .filter(|&&(_, _, jx, jy, _, _)| (jx - ax).powi(2) + (jy - ay).powi(2) <= r2)
        .filter(|&&(_, _, jx, jy, _, _)| {
            // 只有「離這隻幼獸最近的同種成體」才出面護衛（那就是牠的母獸）。
            nearest_adult_id_of_kind(kind, jx, jy, adult_snap, DEFEND_GUARD_RADIUS) == Some(self_id)
        })
        .min_by(|&&(_, _, ax1, ay1, _, _), &&(_, _, ax2, ay2, _, _)| {
            let d1 = (ax1 - ax).powi(2) + (ay1 - ay).powi(2);
            let d2 = (ax2 - ax).powi(2) + (ay2 - ay).powi(2);
            d1.partial_cmp(&d2).unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|&(_, _, _, _, px, py)| (px, py))
}

/// ROADMAP 214：母獸護幼——成體朝威脅掠食者衝刺的逐幀位移（純函式，便於測試）。
/// 以 `DEFEND_SPEED` 朝 (pred_x,pred_y) 移動，單幀位移受「與掠食者距離」上限約束
/// （不越過掠食者，最多貼到原地）；已幾乎重疊時原地不動。
fn defend_charge(ax: f32, ay: f32, pred_x: f32, pred_y: f32, dt: f32) -> (f32, f32) {
    let dx = pred_x - ax;
    let dy = pred_y - ay;
    let dist = (dx * dx + dy * dy).sqrt();
    if dist < 1.0 {
        return (ax, ay);
    }
    let step = (DEFEND_SPEED * dt).min(dist);
    (ax + dx / dist * step, ay + dy / dist * step)
}

/// ROADMAP 206：群聚結伴——選一個新的漫遊目標。
/// 先取家附近的隨機點（沿用 `random_target` 的散布），若 `anchor`（附近同種夥伴
/// 的平均位置）存在，再把目標朝群體中心按 `HERD_PULL` 混合，使同種動物鬆散聚攏、
/// 成群移動；無夥伴則退回純隨機漫遊（行為與 205 之前完全一致）。純函式，便於測試。
fn herd_wander_target(hx: f32, hy: f32, anchor: Option<(f32, f32)>, rng: &mut StdRng) -> (f32, f32) {
    let (rx, ry) = random_target(hx, hy, WANDER_RADIUS, rng);
    match anchor {
        Some((cx, cy)) => (rx + (cx - rx) * HERD_PULL, ry + (cy - ry) * HERD_PULL),
        None => (rx, ry),
    }
}

// ─── ROADMAP 207：繁衍純函式（可測） ─────────────────────────────────────────

/// 統計某物種在世界中的個體總數（含存活與待重生）——用於封頂判斷。
/// 計入待重生者，是因為死亡個體稍後會在家附近重生回到族群，
/// 故「總數」才是穩定的族群規模上限依據（避免靠不斷死亡刷出超額幼獸）。
fn species_total(animals: &[Wildlife], kind: WildlifeKind) -> usize {
    animals.iter().filter(|a| a.kind == kind).count()
}

/// 找出「可繁衍的一群」的中心：在所有存活成體中，找出第一隻其 `radius` 內
/// （含自身）同種存活成體達 `min_count` 隻者，回傳該群的平均位置；否則 `None`。
/// 只算成體（幼獸不繁衍），且只看獵物本身的聚集——捕食者干擾在呼叫端另判。
fn breeding_cluster_center(
    animals: &[Wildlife],
    kind: WildlifeKind,
    radius: f32,
    min_count: usize,
) -> Option<(f32, f32)> {
    let r2 = radius * radius;
    let adults: Vec<(f32, f32)> = animals.iter()
        .filter(|a| a.alive && a.kind == kind && !a.is_juvenile())
        .map(|a| (a.x, a.y))
        .collect();

    for &(sx, sy) in &adults {
        let group: Vec<(f32, f32)> = adults.iter()
            .copied()
            .filter(|&(x, y)| {
                let dx = x - sx;
                let dy = y - sy;
                dx * dx + dy * dy <= r2
            })
            .collect();
        if group.len() >= min_count {
            let n = group.len() as f32;
            let mx = group.iter().map(|p| p.0).sum::<f32>() / n;
            let my = group.iter().map(|p| p.1).sum::<f32>() / n;
            return Some((mx, my));
        }
    }
    None
}

/// 生成所有野生動物（獵物 + 捕食者）。
fn spawn_all_wildlife(rng: &mut StdRng) -> Vec<Wildlife> {
    let spawns: &[(WildlifeKind, f32, f32)] = &[
        // ── 獵物：草原野鳥（城鎮北方）──
        (WildlifeKind::WildBird,     1900.0, 1600.0),
        (WildlifeKind::WildBird,     2100.0, 1500.0),
        (WildlifeKind::WildBird,     1700.0, 1750.0),
        // ── 獵物：草原野鹿（城鎮西北）──
        (WildlifeKind::WildDeer,     1600.0, 1900.0),
        (WildlifeKind::WildDeer,     1750.0, 2100.0),
        // ── 獵物：小動物（草原四散）──
        (WildlifeKind::SmallCritter, 1950.0, 2000.0),
        (WildlifeKind::SmallCritter, 2200.0, 1700.0),
        (WildlifeKind::SmallCritter, 1800.0, 1650.0),
        // ── 獵物：森林野鳥（城鎮東北）──
        (WildlifeKind::WildBird,     2700.0, 1700.0),
        (WildlifeKind::WildBird,     2900.0, 1550.0),
        // ── 獵物：森林野鹿（城鎮東方）──
        (WildlifeKind::WildDeer,     2800.0, 2000.0),
        (WildlifeKind::WildDeer,     3000.0, 2200.0),
        // ── 獵物：小動物（森林）──
        (WildlifeKind::SmallCritter, 2600.0, 1900.0),
        (WildlifeKind::SmallCritter, 2950.0, 1850.0),
        // ── 獵物：南方草原 ──
        (WildlifeKind::WildBird,     2200.0, 3000.0),
        (WildlifeKind::WildDeer,     2400.0, 3100.0),
        (WildlifeKind::SmallCritter, 2100.0, 2800.0),
        (WildlifeKind::SmallCritter, 2500.0, 2900.0),
        // ── 捕食者：野狼（靠近野鹿領地）──
        (WildlifeKind::WildWolf,     2880.0, 2150.0), // 東方森林，近 (2800,2000)
        (WildlifeKind::WildWolf,     1520.0, 2080.0), // 西北草原，近 (1600,1900)
        // ── 捕食者：野狐（靠近小動物領地）──
        (WildlifeKind::WildFox,      2020.0, 2060.0), // 草原，近 (1950,2000)
        (WildlifeKind::WildFox,      2680.0, 1970.0), // 森林，近 (2600,1900)
    ];

    assert_eq!(spawns.len(), WILDLIFE_COUNT);
    spawns.iter().enumerate().map(|(i, &(kind, hx, hy))| {
        Wildlife::new(i as u32, kind, hx, hy, rng)
    }).collect()
}

// ─── ROADMAP 143：聚落定義 ───────────────────────────────────────────────────

/// 建立 6 個固定物種聚落，分散於城鎮周圍野外。
/// 位置與 spawn_all_wildlife 的家位置對應，讓動物確實守衛自己的家域。
fn build_colonies() -> Vec<Colony> {
    vec![
        // 野鳥：兩個聚落（北方草原 + 東北森林）
        Colony { id: 0, kind: WildlifeKind::WildBird,     name: "野鳥巢穴（北方草原）", cx: 1900.0, cy: 1620.0, guard_radius: 230.0 },
        Colony { id: 1, kind: WildlifeKind::WildBird,     name: "野鳥巢穴（東北森林）", cx: 2800.0, cy: 1640.0, guard_radius: 210.0 },
        // 野鹿：一個聚落（西北草原鹿群）
        Colony { id: 2, kind: WildlifeKind::WildDeer,     name: "野鹿棲地",            cx: 1675.0, cy: 2000.0, guard_radius: 250.0 },
        // 小動物：一個洞穴（草原灌木區）
        Colony { id: 3, kind: WildlifeKind::SmallCritter, name: "小動物洞穴",          cx: 1985.0, cy: 1880.0, guard_radius: 200.0 },
        // 野狼：一個狼窩（東方森林）
        Colony { id: 4, kind: WildlifeKind::WildWolf,     name: "狼窩",               cx: 2880.0, cy: 2150.0, guard_radius: 260.0 },
        // 野狐：一個狐狸洞（草原）
        Colony { id: 5, kind: WildlifeKind::WildFox,      name: "狐狸洞",             cx: 2025.0, cy: 2060.0, guard_radius: 220.0 },
    ]
}

// ─── 測試 ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_rng() -> StdRng { StdRng::seed_from_u64(99) }

    #[test]
    fn wildlife_count_matches() {
        let mgr = WildlifeManager::new();
        assert_eq!(mgr.animals.len(), WILDLIFE_COUNT);
    }

    #[test]
    fn predator_count_is_four() {
        let mgr = WildlifeManager::new();
        let preds = mgr.animals.iter().filter(|a| a.kind.trophic_level() == TrophicLevel::Predator).count();
        assert_eq!(preds, 4);
    }

    #[test]
    fn prey_count_is_eighteen() {
        let mgr = WildlifeManager::new();
        let prey = mgr.animals.iter().filter(|a| a.kind.trophic_level() == TrophicLevel::Prey).count();
        assert_eq!(prey, 18);
    }

    #[test]
    fn no_player_stays_near_home() {
        let mut rng = make_rng();
        let mut animal = Wildlife::new(0, WildlifeKind::WildBird, 2000.0, 2000.0, &mut rng);
        for _ in 0..300 {
            animal.tick_idle(0.1, &[], WANDER_SPEED, None, 0.0, &mut rng);
        }
        let dx = animal.x - animal.home_x;
        let dy = animal.y - animal.home_y;
        let dist = (dx * dx + dy * dy).sqrt();
        assert!(dist <= WANDER_RADIUS + 10.0, "漂移超出預期: {dist}");
    }

    #[test]
    fn player_nearby_triggers_prey_flee() {
        let mut rng = make_rng();
        let mut animal = Wildlife::new(0, WildlifeKind::WildDeer, 2000.0, 2000.0, &mut rng);
        let threats = vec![(2050.0_f32, 2050.0_f32)];
        animal.tick_idle(0.1, &threats, WANDER_SPEED, None, 0.0, &mut rng);
        assert!(matches!(animal.state, WildlifeState::Fleeing { .. }),
            "應轉成 Fleeing，實際: {:?}", animal.state);
    }

    #[test]
    fn predator_hunts_prey_in_range() {
        let mut mgr = WildlifeManager::new();
        // 找一隻野狼和一隻野鹿，把牠們移到彼此 HUNT_RADIUS 內。
        let wolf_idx = mgr.animals.iter().position(|a| a.kind == WildlifeKind::WildWolf).unwrap();
        let deer_idx = mgr.animals.iter().position(|a| a.kind == WildlifeKind::WildDeer).unwrap();
        let deer_id = mgr.animals[deer_idx].id;
        // 把野狼移到野鹿旁邊（距離 250px，在 HUNT_RADIUS=320 內）。
        mgr.animals[wolf_idx].x = mgr.animals[deer_idx].x + 250.0;
        mgr.animals[wolf_idx].y = mgr.animals[deer_idx].y;
        mgr.animals[wolf_idx].state = WildlifeState::Wandering { target_x: 0.0, target_y: 0.0, wander_timer: 5.0 };
        // 跑一幀觸發追獵。
        mgr.tick(0.1, &[], &std::collections::HashMap::new(), &[], false);
        let wolf = &mgr.animals[wolf_idx];
        // 野狼應開始狩獵某隻野鹿（不指定是哪隻，因附近可能有多隻）。
        // ROADMAP 213：250px ＞ 撲擊距離(200)，故「開始狩獵」表現為先進潛行（Stalking）；
        // 兩者皆鎖定了目標，故都算數。
        let target = match wolf.state {
            WildlifeState::Hunting { target_id, .. } => Some(target_id),
            WildlifeState::Stalking { target_id, .. } => Some(target_id),
            _ => None,
        };
        assert!(target.is_some(),
            "野狼應進入狩獵（Stalking/Hunting）狀態，實際: {:?}", wolf.state);
        // 確認狩獵目標確實是野鹿。
        let target_id = target.unwrap();
        assert!(
            mgr.animals.iter().any(|a| a.id == target_id && a.kind == WildlifeKind::WildDeer),
            "狩獵目標應為野鹿，target_id={target_id}"
        );
        let _ = deer_id; // 已不用直接比對
    }

    #[test]
    fn predator_kills_adjacent_prey_and_emits_event() {
        let mut mgr = WildlifeManager::new();
        // 找野狼和野鹿，放到彼此 KILL_RADIUS 內。
        let wolf_idx = mgr.animals.iter().position(|a| a.kind == WildlifeKind::WildWolf).unwrap();
        let deer_idx = mgr.animals.iter().position(|a| a.kind == WildlifeKind::WildDeer).unwrap();
        let deer_id  = mgr.animals[deer_idx].id;
        let deer_x   = mgr.animals[deer_idx].x;
        let deer_y   = mgr.animals[deer_idx].y;
        // 野狼直接貼著野鹿。
        mgr.animals[wolf_idx].x = deer_x + KILL_RADIUS * 0.5;
        mgr.animals[wolf_idx].y = deer_y;
        mgr.animals[wolf_idx].state = WildlifeState::Hunting { target_id: deer_id, hunt_timer: 10.0 };
        let events = mgr.tick(0.1, &[], &std::collections::HashMap::new(), &[], false);
        // 野鹿應死亡。
        assert!(!mgr.animals[deer_idx].alive, "野鹿應已死亡");
        // 應有 Kill 事件。
        assert!(
            events.iter().any(|e| matches!(e, WildlifeEvent::Kill { prey_kind: WildlifeKind::WildDeer, .. })),
            "應有 Kill 事件"
        );
    }

    #[test]
    fn dead_prey_respawns_after_timer() {
        let mut mgr = WildlifeManager::new();
        let deer_idx = mgr.animals.iter().position(|a| a.kind == WildlifeKind::WildDeer).unwrap();
        mgr.animals[deer_idx].alive = false;
        mgr.animals[deer_idx].respawn_timer = 0.1;
        // 跑超過 0.1 秒。
        mgr.tick(0.2, &[], &std::collections::HashMap::new(), &[], false);
        assert!(mgr.animals[deer_idx].alive, "野鹿應在計時器結束後重生");
    }

    #[test]
    fn manager_tick_no_panic() {
        let mut mgr = WildlifeManager::new();
        let players = vec![(2200.0f32, 2200.0)];
        for _ in 0..100 {
            mgr.tick(0.1, &players, &std::collections::HashMap::new(), &[], false);
        }
        assert_eq!(mgr.animals.len(), WILDLIFE_COUNT);
    }

    // ─── ROADMAP 142 測試：乙太微粒生命週期 ─────────────────────────────────

    #[test]
    fn carion_orb_spawns_on_kill() {
        let mut mgr = WildlifeManager::new();
        let wolf_idx = mgr.animals.iter().position(|a| a.kind == WildlifeKind::WildWolf).unwrap();
        let deer_idx = mgr.animals.iter().position(|a| a.kind == WildlifeKind::WildDeer).unwrap();
        let deer_id = mgr.animals[deer_idx].id;
        let deer_x  = mgr.animals[deer_idx].x;
        let deer_y  = mgr.animals[deer_idx].y;
        mgr.animals[wolf_idx].x = deer_x + KILL_RADIUS * 0.5;
        mgr.animals[wolf_idx].y = deer_y;
        mgr.animals[wolf_idx].state = WildlifeState::Hunting { target_id: deer_id, hunt_timer: 10.0 };
        mgr.tick(0.1, &[], &std::collections::HashMap::new(), &[], false);
        assert_eq!(mgr.carion_orbs.len(), 1, "擊殺後應生成一顆乙太微粒");
        let orb = &mgr.carion_orbs[0];
        let dx = orb.x - deer_x;
        let dy = orb.y - deer_y;
        assert!(dx * dx + dy * dy < 1.0, "乙太微粒應在死亡位置");
    }

    #[test]
    fn carion_orb_expires_after_ttl() {
        let mut mgr = WildlifeManager::new();
        // 手動插入一顆即將到期的乙太微粒。
        mgr.carion_orbs.push(CarrionOrb { id: 0, x: 2000.0, y: 2000.0, ttl: 0.05 });
        assert_eq!(mgr.carion_orbs.len(), 1);
        // 跑超過 TTL。
        mgr.tick(0.1, &[], &std::collections::HashMap::new(), &[], false);
        assert_eq!(mgr.carion_orbs.len(), 0, "TTL 到期後應自動消失");
    }

    #[test]
    fn collect_carion_orb_in_range_succeeds() {
        let mut mgr = WildlifeManager::new();
        mgr.carion_orbs.push(CarrionOrb { id: 42, x: 2000.0, y: 2000.0, ttl: 60.0 });
        let result = mgr.collect_carion_orb(42, 2020.0, 2020.0);
        assert_eq!(result, Some(CARION_ETHER), "在範圍內採集應得到乙太");
        assert_eq!(mgr.carion_orbs.len(), 0, "採集後微粒應消失");
    }

    #[test]
    fn collect_carion_orb_out_of_range_fails() {
        let mut mgr = WildlifeManager::new();
        mgr.carion_orbs.push(CarrionOrb { id: 7, x: 2000.0, y: 2000.0, ttl: 60.0 });
        let result = mgr.collect_carion_orb(7, 2200.0, 2200.0);
        assert!(result.is_none(), "超出範圍不應成功採集");
        assert_eq!(mgr.carion_orbs.len(), 1, "失敗後微粒仍存在");
    }

    #[test]
    fn collect_carion_orb_wrong_id_fails() {
        let mut mgr = WildlifeManager::new();
        mgr.carion_orbs.push(CarrionOrb { id: 1, x: 2000.0, y: 2000.0, ttl: 60.0 });
        let result = mgr.collect_carion_orb(99, 2000.0, 2000.0);
        assert!(result.is_none(), "錯誤 ID 不應成功採集");
    }

    #[test]
    fn max_orb_limit_is_respected() {
        let mut mgr = WildlifeManager::new();
        // 塞滿上限。
        for i in 0..MAX_CARION_ORBS {
            mgr.carion_orbs.push(CarrionOrb { id: i as u32, x: 2000.0, y: 2000.0, ttl: 60.0 });
        }
        assert_eq!(mgr.carion_orbs.len(), MAX_CARION_ORBS);
        // 模擬一次擊殺（找野狼和野鹿）。
        let wolf_idx = mgr.animals.iter().position(|a| a.kind == WildlifeKind::WildWolf).unwrap();
        let deer_idx = mgr.animals.iter().position(|a| a.kind == WildlifeKind::WildDeer).unwrap();
        let deer_id = mgr.animals[deer_idx].id;
        let deer_x  = mgr.animals[deer_idx].x;
        let deer_y  = mgr.animals[deer_idx].y;
        mgr.animals[wolf_idx].x = deer_x + KILL_RADIUS * 0.5;
        mgr.animals[wolf_idx].y = deer_y;
        mgr.animals[wolf_idx].state = WildlifeState::Hunting { target_id: deer_id, hunt_timer: 10.0 };
        mgr.tick(0.1, &[], &std::collections::HashMap::new(), &[], false);
        // 上限不超出。
        assert!(mgr.carion_orbs.len() <= MAX_CARION_ORBS, "乙太微粒不應超過上限");
    }

    #[test]
    fn carion_ether_value_is_positive() {
        assert!(CARION_ETHER > 0, "乙太微粒的乙太數量應 > 0");
    }

    #[test]
    fn carion_orb_ids_are_unique() {
        let mut mgr = WildlifeManager::new();
        for _ in 0..3 {
            let id = mgr.orb_counter;
            mgr.orb_counter = mgr.orb_counter.wrapping_add(1);
            mgr.carion_orbs.push(CarrionOrb { id, x: 0.0, y: 0.0, ttl: 60.0 });
        }
        let ids: Vec<u32> = mgr.carion_orbs.iter().map(|o| o.id).collect();
        let unique: std::collections::HashSet<u32> = ids.iter().copied().collect();
        assert_eq!(ids.len(), unique.len(), "乙太微粒 ID 應唯一");
    }

    // ─── ROADMAP 143 測試：物種聚落與守衛行為 ─────────────────────────────────

    #[test]
    fn colony_count_is_six() {
        let mgr = WildlifeManager::new();
        assert_eq!(mgr.colonies.len(), 6, "應有 6 個物種聚落");
    }

    #[test]
    fn colony_ids_are_unique() {
        let mgr = WildlifeManager::new();
        let ids: Vec<u32> = mgr.colonies.iter().map(|c| c.id).collect();
        let unique: std::collections::HashSet<u32> = ids.iter().copied().collect();
        assert_eq!(ids.len(), unique.len(), "聚落 ID 應唯一");
    }

    #[test]
    fn player_in_colony_triggers_guarding() {
        let mut mgr = WildlifeManager::new();
        // 找野鹿聚落（id=2，位於 1675,2000）。
        let deer_colony = mgr.colonies.iter().find(|c| c.kind == WildlifeKind::WildDeer).unwrap();
        let (cx, cy) = (deer_colony.cx, deer_colony.cy);
        // 把一隻野鹿放到聚落中心附近，確保在 activate 範圍內。
        let deer_idx = mgr.animals.iter().position(|a| a.kind == WildlifeKind::WildDeer).unwrap();
        mgr.animals[deer_idx].x = cx + 50.0;
        mgr.animals[deer_idx].y = cy + 50.0;
        mgr.animals[deer_idx].state = WildlifeState::Resting { rest_timer: 5.0 };
        // 玩家站在聚落中心。
        let players = vec![(cx, cy)];
        mgr.tick(0.1, &players, &std::collections::HashMap::new(), &[], false);
        // 野鹿應進入 Guarding 狀態。
        let deer = &mgr.animals[deer_idx];
        assert!(
            matches!(deer.state, WildlifeState::Guarding { .. }),
            "野鹿應進入 Guarding 狀態，實際: {:?}", deer.state
        );
    }

    #[test]
    fn colony_threat_event_emitted_on_intrusion() {
        let mut mgr = WildlifeManager::new();
        let deer_colony = mgr.colonies.iter().find(|c| c.kind == WildlifeKind::WildDeer).unwrap();
        let (cx, cy) = (deer_colony.cx, deer_colony.cy);
        // 玩家站在聚落中心。
        let players = vec![(cx, cy)];
        let events = mgr.tick(0.1, &players, &std::collections::HashMap::new(), &[], false);
        assert!(
            events.iter().any(|e| matches!(e, WildlifeEvent::ColonyThreatened { .. })),
            "玩家進入聚落應觸發 ColonyThreatened 事件"
        );
    }

    #[test]
    fn colony_threat_cooldown_prevents_repeat_events() {
        let mut mgr = WildlifeManager::new();
        let deer_colony = mgr.colonies.iter().find(|c| c.kind == WildlifeKind::WildDeer).unwrap();
        let (cx, cy) = (deer_colony.cx, deer_colony.cy);
        let players = vec![(cx, cy)];
        // 第一次觸發。
        let events1 = mgr.tick(0.1, &players, &std::collections::HashMap::new(), &[], false);
        assert!(events1.iter().any(|e| matches!(e, WildlifeEvent::ColonyThreatened { .. })));
        // 馬上再觸發：冷卻中，不應再發出事件。
        let events2 = mgr.tick(0.1, &players, &std::collections::HashMap::new(), &[], false);
        assert!(
            !events2.iter().any(|e| matches!(e, WildlifeEvent::ColonyThreatened { .. })),
            "冷卻中不應再發出 ColonyThreatened 事件"
        );
    }

    #[test]
    fn guard_timer_expires_and_animal_returns_to_rest() {
        let mut mgr = WildlifeManager::new();
        let mut deer = mgr.animals.iter()
            .find(|a| a.kind == WildlifeKind::WildDeer).unwrap().clone();
        deer.id = 0;
        // 手動設定守衛狀態，計時即將到期。
        deer.state = WildlifeState::Guarding { threat_x: 2000.0, threat_y: 2000.0, guard_timer: 0.05 };
        // 單獨測「守衛到期→休息」這一轉換：場上只留這隻（孤獸不成群、不會接管哨兵 watching，
        // 也沒有近旁掠食者觸發 ROADMAP 212 哨兵的放大警戒逃竄）——把單元行為與群聚行為隔開。
        mgr.animals = vec![deer];
        // 跑超過計時。
        mgr.tick(0.2, &[], &std::collections::HashMap::new(), &[], false);
        let deer = &mgr.animals[0];
        assert!(
            matches!(deer.state, WildlifeState::Resting { .. }),
            "計時到期後應回到 Resting，實際: {:?}", deer.state
        );
    }

    #[test]
    fn colony_views_returns_all_colonies() {
        let mgr = WildlifeManager::new();
        let views = mgr.colony_views();
        assert_eq!(views.len(), 6, "colony_views 應回傳 6 個視圖");
        assert!(views.iter().any(|v| v.kind == "wild_wolf"), "應含狼窩");
        assert!(views.iter().any(|v| v.kind == "wild_bird"), "應含野鳥巢穴");
    }

    #[test]
    fn different_species_not_affected_by_wrong_colony() {
        let mut mgr = WildlifeManager::new();
        // 找狐狸洞聚落。
        let fox_colony = mgr.colonies.iter().find(|c| c.kind == WildlifeKind::WildFox).unwrap();
        let (cx, cy) = (fox_colony.cx, fox_colony.cy);
        // 找一隻野鳥（不是狐狸），放到狐狸洞附近。
        let bird_idx = mgr.animals.iter().position(|a| a.kind == WildlifeKind::WildBird).unwrap();
        mgr.animals[bird_idx].x = cx + 80.0;
        mgr.animals[bird_idx].y = cy + 80.0;
        mgr.animals[bird_idx].state = WildlifeState::Resting { rest_timer: 5.0 };
        // 玩家站在狐狸洞。
        let players = vec![(cx, cy)];
        mgr.tick(0.1, &players, &std::collections::HashMap::new(), &[], false);
        // 野鳥不應受狐狸洞影響。
        let bird = &mgr.animals[bird_idx];
        assert!(
            !matches!(bird.state, WildlifeState::Guarding { .. }),
            "野鳥不應因狐狸洞的入侵而守衛，實際: {:?}", bird.state
        );
    }

    #[test]
    fn guard_radius_values_are_positive() {
        let mgr = WildlifeManager::new();
        for c in &mgr.colonies {
            assert!(c.guard_radius > 0.0, "聚落 {} 守衛半徑應 > 0", c.name);
        }
    }

    // ─── ROADMAP 165 測試 ────────────────────────────────────────────────────

    #[test]
    fn monster_hunts_wildlife_returns_correct_pairs() {
        use crate::combat::EnemyKind;
        assert_eq!(monster_hunts_wildlife(EnemyKind::EtherWisp),       Some(WildlifeKind::WildBird));
        assert_eq!(monster_hunts_wildlife(EnemyKind::MushroomStalker), Some(WildlifeKind::SmallCritter));
        assert_eq!(monster_hunts_wildlife(EnemyKind::ScrapDrone),      Some(WildlifeKind::WildDeer));
        assert_eq!(monster_hunts_wildlife(EnemyKind::CrystalGolem),    None);
        assert_eq!(monster_hunts_wildlife(EnemyKind::FlutterSprite),   None);
    }

    #[test]
    fn on_monster_kills_wildlife_marks_dead_and_creates_orb() {
        use crate::combat::EnemyKind;
        let mut mgr = WildlifeManager::new();
        let bird_id = mgr.animals.iter()
            .find(|a| a.alive && a.kind == WildlifeKind::WildBird)
            .map(|a| a.id).unwrap();
        let before_orbs = mgr.carion_orbs.len();
        let ev = mgr.on_monster_kills_wildlife(bird_id, EnemyKind::EtherWisp);
        assert!(matches!(ev, Some(WildlifeEvent::MonsterHunted { .. })), "應回傳 MonsterHunted 事件");
        let bird = mgr.animals.iter().find(|a| a.id == bird_id).unwrap();
        assert!(!bird.alive, "被獵殺的野鳥應標記為死亡");
        assert_eq!(mgr.carion_orbs.len(), before_orbs + 1, "應生成一顆乙太微粒");
    }

    #[test]
    fn on_monster_kills_wildlife_idempotent_on_dead_animal() {
        use crate::combat::EnemyKind;
        let mut mgr = WildlifeManager::new();
        let bird_id = mgr.animals.iter()
            .find(|a| a.alive && a.kind == WildlifeKind::WildBird)
            .map(|a| a.id).unwrap();
        let _ = mgr.on_monster_kills_wildlife(bird_id, EnemyKind::EtherWisp);
        let ev2 = mgr.on_monster_kills_wildlife(bird_id, EnemyKind::EtherWisp);
        assert!(ev2.is_none(), "已死亡的動物再次呼叫應回傳 None");
    }

    #[test]
    fn alive_snapshot_counts_decrease_after_kill() {
        use crate::combat::EnemyKind;
        let mut mgr = WildlifeManager::new();
        let initial_count = mgr.alive_snapshot().len();
        assert_eq!(initial_count, WILDLIFE_COUNT);
        let bird_id = mgr.animals.iter()
            .find(|a| a.alive && a.kind == WildlifeKind::WildBird)
            .map(|a| a.id).unwrap();
        let _ = mgr.on_monster_kills_wildlife(bird_id, EnemyKind::EtherWisp);
        assert_eq!(mgr.alive_snapshot().len(), initial_count - 1, "死亡後快照應少一隻");
    }

    #[test]
    fn prey_flees_from_hunting_monster_in_tick() {
        use crate::combat::EnemyKind;
        let mut rng = make_rng();
        // 建立一隻靜止野鳥（在 home 位置）。
        let mut bird = Wildlife::new(0, WildlifeKind::WildBird, 2000.0, 2000.0, &mut rng);
        bird.state = WildlifeState::Resting { rest_timer: 10.0 };
        bird.x = 2000.0;
        bird.y = 2000.0;
        // 把 EtherWisp 放在 FLEE_RADIUS 內（100px）。
        let threats = vec![(EnemyKind::EtherWisp, 2100.0_f32, 2000.0_f32)];
        bird.tick_idle(0.1, &threats.iter().map(|&(_, x, y)| (x, y)).collect::<Vec<_>>(), WANDER_SPEED, None, 0.0, &mut rng);
        assert!(
            matches!(bird.state, WildlifeState::Fleeing { .. }),
            "怪物在 FLEE_RADIUS 內，野鳥應進入 Fleeing 狀態"
        );
    }

    #[test]
    fn non_prey_kind_not_affected_by_monster_threats_in_tick() {
        use crate::combat::EnemyKind;
        // CrystalGolem 不獵食任何野生動物，野鹿不應因它逃跑。
        let threats = vec![(EnemyKind::CrystalGolem, 2100.0_f32, 2000.0_f32)];
        assert!(
            monster_hunts_wildlife(EnemyKind::CrystalGolem).is_none(),
            "CrystalGolem 不應有食物鏈配對"
        );
        let _ = threats;
    }

    // ─── ROADMAP 205：餵食馴養 測試 ─────────────────────────────────────────
    use std::collections::HashMap;

    /// 把 mgr 內第一隻指定種類的動物搬到 (x,y)、設定親近度與休息狀態，回傳其 id。
    fn place_test_animal(mgr: &mut WildlifeManager, kind: WildlifeKind, x: f32, y: f32, familiarity: f32) -> u32 {
        let id = mgr.animals.iter().find(|a| a.kind == kind).map(|a| a.id).unwrap();
        let a = mgr.animals.iter_mut().find(|a| a.id == id).unwrap();
        a.alive = true;
        a.x = x; a.y = y;
        a.home_x = x; a.home_y = y;
        a.familiarity = familiarity;
        a.state = WildlifeState::Resting { rest_timer: 10.0 };
        id
    }

    #[test]
    fn feeding_raises_familiarity_and_tames_exactly_once() {
        let mut mgr = WildlifeManager::new();
        let id = place_test_animal(&mut mgr, WildlifeKind::WildDeer, 5000.0, 5000.0, 0.0);
        let needed = (TAME_FAMILIARITY / FEED_FAMILIARITY_GAIN).ceil() as i32;
        let mut tamed_events = 0;
        for _ in 0..needed {
            let (_, _, just_tamed) = mgr.on_feed_animal(id).unwrap();
            if just_tamed { tamed_events += 1; }
        }
        assert!(mgr.animals.iter().find(|a| a.id == id).unwrap().is_tamed(), "餵足次數後應已馴養");
        assert_eq!(tamed_events, 1, "「剛馴養」事件應只觸發一次");
        // 已馴養後再餵不應再觸發馴養事件。
        let (_, _, again) = mgr.on_feed_animal(id).unwrap();
        assert!(!again, "已馴養後再餵不應重複觸發馴養");
    }

    #[test]
    fn on_feed_animal_unknown_id_returns_none() {
        let mut mgr = WildlifeManager::new();
        assert!(mgr.on_feed_animal(999_999).is_none(), "不存在的 ID 應回傳 None");
    }

    #[test]
    fn tamed_prey_does_not_flee_player() {
        let mut mgr = WildlifeManager::new();
        let id = place_test_animal(&mut mgr, WildlifeKind::WildDeer, 5000.0, 5000.0, MAX_FAMILIARITY);
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        // 玩家就在 FLEE_RADIUS 內。
        mgr.tick(0.1, &[(5050.0, 5000.0)], &att, &[], false);
        let a = mgr.animals.iter().find(|a| a.id == id).unwrap();
        assert!(!matches!(a.state, WildlifeState::Fleeing { .. }), "馴養個體不應逃離玩家，實際: {:?}", a.state);
    }

    #[test]
    fn untamed_prey_still_flees_player() {
        let mut mgr = WildlifeManager::new();
        let id = place_test_animal(&mut mgr, WildlifeKind::WildDeer, 5000.0, 5000.0, 0.0);
        let att: HashMap<WildlifeKind, i32> = HashMap::new(); // 預設態度 50 < FRIENDLY，玩家是威脅
        mgr.tick(0.1, &[(5050.0, 5000.0)], &att, &[], false);
        let a = mgr.animals.iter().find(|a| a.id == id).unwrap();
        assert!(matches!(a.state, WildlifeState::Fleeing { .. }), "未馴養個體應逃離玩家，實際: {:?}", a.state);
    }

    #[test]
    fn tamed_prey_follows_nearby_player() {
        let mut mgr = WildlifeManager::new();
        let id = place_test_animal(&mut mgr, WildlifeKind::WildDeer, 5000.0, 5000.0, MAX_FAMILIARITY);
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        // 玩家在 FOLLOW_RANGE 內、舒適距離外（右側 200px）。
        mgr.tick(0.2, &[(5200.0, 5000.0)], &att, &[], false);
        let a = mgr.animals.iter().find(|a| a.id == id).unwrap();
        assert!(a.x > 5000.0, "馴養個體應朝玩家移動（x 變大），實際 x={}", a.x);
    }

    #[test]
    fn tamed_prey_still_flees_hunting_monster() {
        use crate::combat::EnemyKind;
        let mut mgr = WildlifeManager::new();
        // ScrapDrone 獵食 WildDeer。
        assert_eq!(monster_hunts_wildlife(EnemyKind::ScrapDrone), Some(WildlifeKind::WildDeer));
        let id = place_test_animal(&mut mgr, WildlifeKind::WildDeer, 5000.0, 5000.0, MAX_FAMILIARITY);
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        // 玩家在旁（馴養→不怕），但獵食怪物在 FLEE_RADIUS 內。
        mgr.tick(0.1, &[(5040.0, 5000.0)], &att, &[(EnemyKind::ScrapDrone, 5050.0, 5000.0)], false);
        let a = mgr.animals.iter().find(|a| a.id == id).unwrap();
        assert!(matches!(a.state, WildlifeState::Fleeing { .. }), "馴養個體仍應逃離掠食怪物，實際: {:?}", a.state);
    }

    #[test]
    fn familiarity_decays_over_time() {
        let mut mgr = WildlifeManager::new();
        let id = place_test_animal(&mut mgr, WildlifeKind::WildDeer, 5000.0, 5000.0, MAX_FAMILIARITY);
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        for _ in 0..100 { mgr.tick(1.0, &[], &att, &[], false); } // 100 秒、無餵食
        let f = mgr.animals.iter().find(|a| a.id == id).unwrap().familiarity();
        assert!(f < MAX_FAMILIARITY, "親近度應隨時間衰減，實際 {f}");
        assert!(f > 0.0, "100 秒衰減不應歸零（衰減很慢），實際 {f}");
    }

    #[test]
    fn respawn_resets_familiarity() {
        let mut mgr = WildlifeManager::new();
        let id = place_test_animal(&mut mgr, WildlifeKind::WildDeer, 5000.0, 5000.0, MAX_FAMILIARITY);
        // 擊殺該隻（玩家攻擊），再推進到重生。
        assert!(mgr.attack_wildlife(id, 5000.0, 5000.0, 30.0).is_some(), "應成功擊殺");
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        for _ in 0..((PREY_RESPAWN_SECS as i32) + 2) { mgr.tick(1.0, &[], &att, &[], false); }
        let a = mgr.animals.iter().find(|a| a.id == id).unwrap();
        assert!(a.alive, "應已重生");
        assert_eq!(a.familiarity(), 0.0, "重生個體親近度應歸零（羈絆隨上一隻散去）");
    }

    // ─── ROADMAP 206：群聚結伴 測試 ─────────────────────────────────────────

    #[test]
    fn herd_center_none_when_alone() {
        // 同種只有自己一隻 → 範圍內無夥伴 → None。
        let snap = vec![(0u32, WildlifeKind::WildDeer, 100.0_f32, 100.0_f32)];
        assert_eq!(herd_center(0, WildlifeKind::WildDeer, 100.0, 100.0, &snap), None);
    }

    #[test]
    fn herd_center_excludes_self_and_other_species() {
        // 三隻同種夥伴（皆在範圍內）+ 一隻自己 + 一隻他種 → 只平均那三隻同種。
        let snap = vec![
            (0u32, WildlifeKind::WildDeer, 0.0_f32, 0.0_f32),     // 自己（排除）
            (1u32, WildlifeKind::WildDeer, 10.0, 0.0),
            (2u32, WildlifeKind::WildDeer, 30.0, 0.0),
            (3u32, WildlifeKind::WildDeer, 50.0, 0.0),
            (4u32, WildlifeKind::WildBird, 10.0, 0.0),            // 他種（排除）
        ];
        let c = herd_center(0, WildlifeKind::WildDeer, 0.0, 0.0, &snap).expect("應有群體中心");
        assert!((c.0 - 30.0).abs() < 0.01 && c.1.abs() < 0.01, "群體中心應為三同種平均 (30,0)，實際 {c:?}");
    }

    #[test]
    fn herd_center_ignores_neighbors_beyond_radius() {
        // 同種夥伴在 HERD_RADIUS 外 → 不算入 → None。
        let far = HERD_RADIUS + 50.0;
        let snap = vec![
            (0u32, WildlifeKind::WildDeer, 0.0_f32, 0.0_f32),
            (1u32, WildlifeKind::WildDeer, far, 0.0),
        ];
        assert_eq!(herd_center(0, WildlifeKind::WildDeer, 0.0, 0.0, &snap), None,
            "範圍外夥伴不應觸發群聚");
    }

    #[test]
    fn herd_wander_target_pulls_toward_anchor() {
        // 有群體中心時，新目標應比「純隨機家附近目標」更靠近群體中心。
        // 家在原點，群體中心遠在 (10000,10000)：拉力後的目標與中心的距離，
        // 應明顯小於家到中心的距離（被朝中心拉了 HERD_PULL 比例）。
        let mut rng = make_rng();
        let anchor = (10000.0_f32, 10000.0_f32);
        let home_to_anchor = (anchor.0.powi(2) + anchor.1.powi(2)).sqrt();
        for _ in 0..50 {
            let (tx, ty) = herd_wander_target(0.0, 0.0, Some(anchor), &mut rng);
            let d = ((tx - anchor.0).powi(2) + (ty - anchor.1).powi(2)).sqrt();
            // 隨機點僅落在家附近 WANDER_RADIUS 內，混合 HERD_PULL 後距中心必縮短。
            assert!(d < home_to_anchor * (1.0 - HERD_PULL + 0.01),
                "拉力後距群體中心 {d} 應明顯小於 {home_to_anchor}");
        }
    }

    #[test]
    fn herd_wander_target_no_anchor_is_pure_random_near_home() {
        // 無夥伴時行為應與純隨機漫遊一致：目標落在家附近 WANDER_RADIUS 內。
        let mut rng = make_rng();
        for _ in 0..50 {
            let (tx, ty) = herd_wander_target(2000.0, 2000.0, None, &mut rng);
            let d = ((tx - 2000.0_f32).powi(2) + (ty - 2000.0_f32).powi(2)).sqrt();
            assert!(d <= WANDER_RADIUS + 0.01, "無夥伴目標應在家附近，實際距離 {d}");
        }
    }

    #[test]
    fn herding_does_not_disturb_flee() {
        // 群聚只影響「選漫遊目標」，不該蓋過逃跑：玩家逼近時仍進入 Fleeing。
        // （群聚夥伴就在身邊，但威脅優先。）
        let mut rng = make_rng();
        let mut deer = Wildlife::new(0, WildlifeKind::WildDeer, 2000.0, 2000.0, &mut rng);
        let threats = vec![(2030.0_f32, 2000.0_f32)];
        let anchor = Some((2010.0_f32, 2000.0_f32));
        deer.tick_idle(0.1, &threats, WANDER_SPEED, anchor, 0.0, &mut rng);
        assert!(matches!(deer.state, WildlifeState::Fleeing { .. }),
            "威脅在 FLEE_RADIUS 內，群聚不應蓋過逃跑，實際 {:?}", deer.state);
    }

    // ─── ROADMAP 207：幼獸誕生（族群繁衍）測試 ──────────────────────────────

    /// 測試用：在指定座標放一隻成體（覆蓋 new() 的隨機偏移）。
    fn adult_at(kind: WildlifeKind, x: f32, y: f32) -> Wildlife {
        let mut rng = make_rng();
        let mut w = Wildlife::new(0, kind, x, y, &mut rng);
        w.x = x; w.y = y; w.maturity = 1.0;
        w
    }
    /// 測試用：在指定座標放一隻幼獸。
    fn juvenile_at(kind: WildlifeKind, x: f32, y: f32) -> Wildlife {
        let mut w = adult_at(kind, x, y);
        w.maturity = 0.0;
        w
    }

    #[test]
    fn juvenile_scale_grows_with_maturity() {
        let baby = juvenile_at(WildlifeKind::WildBird, 0.0, 0.0);
        assert!(baby.is_juvenile(), "成熟度 0 應為幼獸");
        assert!((baby.scale() - JUVENILE_MIN_SCALE).abs() < 1e-4, "剛誕生體型應為 JUVENILE_MIN_SCALE");
        let adult = adult_at(WildlifeKind::WildBird, 0.0, 0.0);
        assert!(!adult.is_juvenile(), "成熟度 1 不應為幼獸");
        assert!((adult.scale() - 1.0).abs() < 1e-4, "成體體型應為 1.0");
    }

    #[test]
    fn species_total_counts_alive_and_dead() {
        let mut alive = adult_at(WildlifeKind::WildDeer, 0.0, 0.0);
        alive.id = 1;
        let mut dead = adult_at(WildlifeKind::WildDeer, 10.0, 0.0);
        dead.id = 2; dead.alive = false;
        let bird = adult_at(WildlifeKind::WildBird, 0.0, 0.0);
        let animals = vec![alive, dead, bird];
        assert_eq!(species_total(&animals, WildlifeKind::WildDeer), 2, "存活+待重生皆計入");
        assert_eq!(species_total(&animals, WildlifeKind::WildBird), 1);
    }

    #[test]
    fn breeding_cluster_center_none_when_scattered() {
        // 兩隻成體相距遠大於 BREED_RADIUS → 各自落單 → None。
        let animals = vec![
            adult_at(WildlifeKind::WildDeer, 0.0, 0.0),
            adult_at(WildlifeKind::WildDeer, 0.0, BREED_RADIUS + 100.0),
        ];
        assert_eq!(breeding_cluster_center(&animals, WildlifeKind::WildDeer, BREED_RADIUS, BREED_HERD_MIN), None);
    }

    #[test]
    fn breeding_cluster_center_returns_mean_of_group() {
        let animals = vec![
            adult_at(WildlifeKind::WildDeer, 0.0, 0.0),
            adult_at(WildlifeKind::WildDeer, 40.0, 0.0),
        ];
        let c = breeding_cluster_center(&animals, WildlifeKind::WildDeer, BREED_RADIUS, BREED_HERD_MIN)
            .expect("緊鄰兩成體應構成可繁衍群");
        assert!((c.0 - 20.0).abs() < 0.01 && c.1.abs() < 0.01, "群心應為兩者平均 (20,0)，實際 {c:?}");
    }

    #[test]
    fn breeding_cluster_center_excludes_juveniles() {
        // 只有一隻成體（另一隻是幼獸）→ 成體不足 BREED_HERD_MIN → 幼獸不繁衍。
        let animals = vec![
            adult_at(WildlifeKind::WildDeer, 0.0, 0.0),
            juvenile_at(WildlifeKind::WildDeer, 30.0, 0.0),
        ];
        assert_eq!(breeding_cluster_center(&animals, WildlifeKind::WildDeer, BREED_RADIUS, BREED_HERD_MIN), None,
            "幼獸不算可繁衍成體");
    }

    #[test]
    fn grouped_peaceful_herd_breeds_a_juvenile() {
        // 兩隻成年野鹿緊鄰、無捕食者、無玩家，進度逼近門檻 → 一個 tick 即誕生一隻幼獸。
        let mut mgr = WildlifeManager::new();
        let mut d1 = adult_at(WildlifeKind::WildDeer, 5000.0, 5000.0); d1.id = 100;
        let mut d2 = adult_at(WildlifeKind::WildDeer, 5030.0, 5000.0); d2.id = 101;
        mgr.animals = vec![d1, d2];
        mgr.next_animal_id = 102;
        mgr.breed_progress.insert(WildlifeKind::WildDeer, BREED_THRESHOLD_SECS - 0.01);

        let attitudes = std::collections::HashMap::new();
        mgr.tick(0.1, &[], &attitudes, &[], false);

        assert_eq!(species_total(&mgr.animals, WildlifeKind::WildDeer), 3, "安穩成群應誕生一隻幼鹿");
        let baby = mgr.animals.last().unwrap();
        assert_eq!(baby.kind, WildlifeKind::WildDeer);
        assert!(baby.is_juvenile(), "新生個體應為幼獸");
    }

    #[test]
    fn predator_near_blocks_breeding() {
        // 同樣逼近門檻，但群心附近有捕食者 → 緊張不育，進度回退、不誕生。
        let mut mgr = WildlifeManager::new();
        let mut d1 = adult_at(WildlifeKind::WildDeer, 5000.0, 5000.0); d1.id = 100;
        let mut d2 = adult_at(WildlifeKind::WildDeer, 5030.0, 5000.0); d2.id = 101;
        // 狼就在群心旁（BREED_DISTURB_RADIUS 內），但不在 KILL_RADIUS 內。
        let mut wolf = adult_at(WildlifeKind::WildWolf, 5120.0, 5000.0); wolf.id = 102;
        mgr.animals = vec![d1, d2, wolf];
        mgr.next_animal_id = 103;
        mgr.breed_progress.insert(WildlifeKind::WildDeer, BREED_THRESHOLD_SECS - 0.01);

        let attitudes = std::collections::HashMap::new();
        mgr.tick(0.1, &[], &attitudes, &[], false);

        assert_eq!(species_total(&mgr.animals, WildlifeKind::WildDeer), 2, "捕食者在旁不應誕生幼獸");
    }

    #[test]
    fn breeding_respects_species_cap() {
        // 野鹿已達上限 → 即使成群安穩、進度滿，也不再誕生（封頂保護效能）。
        let mut mgr = WildlifeManager::new();
        let cap = species_cap(WildlifeKind::WildDeer);
        let mut herd = Vec::new();
        for i in 0..cap {
            let mut d = adult_at(WildlifeKind::WildDeer, 5000.0 + i as f32 * 20.0, 5000.0);
            d.id = 200 + i as u32;
            herd.push(d);
        }
        mgr.animals = herd;
        mgr.next_animal_id = 300;
        mgr.breed_progress.insert(WildlifeKind::WildDeer, BREED_THRESHOLD_SECS - 0.01);

        let attitudes = std::collections::HashMap::new();
        mgr.tick(0.1, &[], &attitudes, &[], false);

        assert_eq!(species_total(&mgr.animals, WildlifeKind::WildDeer), cap, "達上限後不應再繁衍");
    }

    #[test]
    fn juvenile_matures_into_adult_over_time() {
        // 幼獸在族群中隨時間長大；足夠時間後成熟度達 1.0、不再是幼獸。
        let mut mgr = WildlifeManager::new();
        let baby = juvenile_at(WildlifeKind::WildBird, 6000.0, 6000.0);
        mgr.animals = vec![baby];
        let attitudes = std::collections::HashMap::new();
        for _ in 0..(MATURE_DURATION_SECS as usize + 5) {
            mgr.tick(1.0, &[], &attitudes, &[], false);
        }
        assert!(!mgr.animals[0].is_juvenile(), "足夠時間後幼獸應長成成體");
        assert!((mgr.animals[0].scale() - 1.0).abs() < 1e-4, "長成後體型應為 1.0");
    }

    // ─── ROADMAP 208：幼獸依偎母獸（親子跟隨）測試 ─────────────────────────────

    #[test]
    fn nearest_adult_none_when_no_same_kind_in_range() {
        // 範圍內只有他種成體 → None。
        let snap = vec![(1u32, WildlifeKind::WildBird, 10.0, 0.0)];
        assert!(nearest_adult_of_kind(0, WildlifeKind::WildDeer, 0.0, 0.0, &snap, NURSE_RANGE).is_none());
        // 同種但遠在範圍外 → None。
        let far = vec![(1u32, WildlifeKind::WildDeer, 9999.0, 0.0)];
        assert!(nearest_adult_of_kind(0, WildlifeKind::WildDeer, 0.0, 0.0, &far, NURSE_RANGE).is_none());
    }

    #[test]
    fn nearest_adult_picks_closest_same_kind() {
        let snap = vec![
            (1u32, WildlifeKind::WildDeer, 300.0, 0.0),   // 同種、較遠（仍在範圍內）
            (2u32, WildlifeKind::WildDeer, 50.0, 0.0),    // 同種、最近 → 應選此
            (3u32, WildlifeKind::WildBird, 10.0, 0.0),    // 他種 → 排除
            (0u32, WildlifeKind::WildDeer, 5.0, 0.0),     // 自己 id → 排除
            (4u32, WildlifeKind::WildDeer, 9999.0, 0.0),  // 同種但超出範圍 → 排除
        ];
        let (px, _) = nearest_adult_of_kind(0, WildlifeKind::WildDeer, 0.0, 0.0, &snap, NURSE_RANGE)
            .expect("應找到同種成體");
        assert!((px - 50.0).abs() < 1e-4, "應選最近的同種成體 x=50，實際 {px}");
    }

    #[test]
    fn juvenile_follows_nearest_adult() {
        // 幼獸在成體右側 200px、無威脅 → 應朝成體（左）依偎移動。
        let mut mgr = WildlifeManager::new();
        let mut adult = adult_at(WildlifeKind::WildDeer, 5000.0, 5000.0);
        adult.id = 1;
        let mut juv = juvenile_at(WildlifeKind::WildDeer, 5200.0, 5000.0);
        juv.id = 2; juv.home_x = 5200.0; juv.home_y = 5000.0;
        juv.state = WildlifeState::Resting { rest_timer: 10.0 };
        mgr.animals = vec![adult, juv];
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        mgr.tick(0.2, &[], &att, &[], false);
        let jx = mgr.animals.iter().find(|a| a.id == 2).unwrap().x;
        assert!(jx < 5200.0, "幼獸應朝同種成體（左側）依偎移動，實際 x={jx}");
    }

    #[test]
    fn juvenile_flees_predator_instead_of_nursing() {
        use crate::combat::EnemyKind;
        // 身旁有可依偎的成體，但獵食幼獸的怪物更近 → 威脅優先、仍逃命（不依偎）。
        let mut mgr = WildlifeManager::new();
        let mut adult = adult_at(WildlifeKind::WildDeer, 5000.0, 5000.0);
        adult.id = 1;
        let mut juv = juvenile_at(WildlifeKind::WildDeer, 5200.0, 5000.0);
        juv.id = 2; juv.home_x = 5200.0; juv.home_y = 5000.0;
        juv.state = WildlifeState::Resting { rest_timer: 10.0 };
        mgr.animals = vec![adult, juv];
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        // ScrapDrone 獵食 WildDeer，置於幼獸右側 40px（FLEE_RADIUS 內）。
        mgr.tick(0.1, &[], &att, &[(EnemyKind::ScrapDrone, 5240.0, 5000.0)], false);
        let j = mgr.animals.iter().find(|a| a.id == 2).unwrap();
        assert!(matches!(j.state, WildlifeState::Fleeing { .. }), "幼獸應逃離掠食者而非依偎，實際: {:?}", j.state);
    }

    // ─── ROADMAP 209：驚群炸開（恐慌連鎖）測試 ─────────────────────────────────

    #[test]
    fn panic_velocity_copies_nearest_fleeing_kin_direction() {
        // 同種逃竄夥伴在範圍內、朝東逃（vx>0）→ 被感染者沿同方向、速度正規化為 FLEE_SPEED。
        let snap = vec![
            (1u32, WildlifeKind::WildDeer, 100.0, 0.0, FLEE_SPEED, 0.0),
        ];
        let (vx, vy) = panic_velocity_from_herd(0, WildlifeKind::WildDeer, 0.0, 0.0, &snap, ALARM_RADIUS)
            .expect("近旁有同種逃竄夥伴應被感染");
        assert!((vx - FLEE_SPEED).abs() < 1e-3 && vy.abs() < 1e-3,
            "應沿夥伴方向（東）以 FLEE_SPEED 逃竄，實際 ({vx},{vy})");
    }

    #[test]
    fn panic_velocity_excludes_self_other_kind_and_out_of_range() {
        let snap = vec![
            (0u32, WildlifeKind::WildDeer, 10.0, 0.0, FLEE_SPEED, 0.0),       // 自己 → 排除
            (2u32, WildlifeKind::WildBird, 10.0, 0.0, FLEE_SPEED, 0.0),       // 他種 → 排除
            (3u32, WildlifeKind::WildDeer, ALARM_RADIUS + 50.0, 0.0, FLEE_SPEED, 0.0), // 超出範圍 → 排除
        ];
        assert!(panic_velocity_from_herd(0, WildlifeKind::WildDeer, 0.0, 0.0, &snap, ALARM_RADIUS).is_none(),
            "排除自己/他種/範圍外後應無可感染來源");
    }

    #[test]
    fn fleeing_kin_panics_calm_neighbor() {
        // 一隻野鹿正在逃竄、近旁另一隻平靜野鹿（無玩家/捕食者直接威脅）→ 被恐慌感染、一起炸開。
        let mut mgr = WildlifeManager::new();
        let mut runner = adult_at(WildlifeKind::WildDeer, 5000.0, 5000.0);
        runner.id = 1;
        runner.state = WildlifeState::Fleeing { vx: FLEE_SPEED, vy: 0.0, flee_timer: FLEE_DURATION };
        let mut calm = adult_at(WildlifeKind::WildDeer, 5100.0, 5000.0); // 100px，ALARM_RADIUS 內
        calm.id = 2;
        calm.state = WildlifeState::Resting { rest_timer: 10.0 };
        mgr.animals = vec![runner, calm];
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        mgr.tick(0.1, &[], &att, &[], false);

        let c = mgr.animals.iter().find(|a| a.id == 2).unwrap();
        match c.state {
            WildlifeState::Fleeing { vx, .. } => assert!(vx > 0.0, "應沿逃竄夥伴方向（東）炸開，實際 vx={vx}"),
            ref s => panic!("平靜同伴應被恐慌感染轉為 Fleeing，實際 {s:?}"),
        }
    }

    #[test]
    fn distant_kin_does_not_panic_neighbor() {
        // 逃竄夥伴遠在 ALARM_RADIUS 外 → 恐慌傳不到，平靜的同伴不應炸群。
        let mut mgr = WildlifeManager::new();
        let mut runner = adult_at(WildlifeKind::WildDeer, 5000.0, 5000.0);
        runner.id = 1;
        runner.state = WildlifeState::Fleeing { vx: FLEE_SPEED, vy: 0.0, flee_timer: FLEE_DURATION };
        let mut calm = adult_at(WildlifeKind::WildDeer, 5000.0 + ALARM_RADIUS + 80.0, 5000.0);
        calm.id = 2;
        calm.state = WildlifeState::Resting { rest_timer: 10.0 };
        mgr.animals = vec![runner, calm];
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        mgr.tick(0.1, &[], &att, &[], false);

        let c = mgr.animals.iter().find(|a| a.id == 2).unwrap();
        assert!(!matches!(c.state, WildlifeState::Fleeing { .. }),
            "逃竄夥伴在 ALARM_RADIUS 外不應傳染恐慌，實際 {:?}", c.state);
    }

    #[test]
    fn panic_does_not_cross_species() {
        // 一隻野鳥逃竄、近旁一隻平靜野鹿 → 異種恐慌不互傳，野鹿不炸群。
        let mut mgr = WildlifeManager::new();
        let mut bird = adult_at(WildlifeKind::WildBird, 5000.0, 5000.0);
        bird.id = 1;
        bird.state = WildlifeState::Fleeing { vx: FLEE_SPEED, vy: 0.0, flee_timer: FLEE_DURATION };
        let mut deer = adult_at(WildlifeKind::WildDeer, 5080.0, 5000.0);
        deer.id = 2;
        deer.state = WildlifeState::Resting { rest_timer: 10.0 };
        mgr.animals = vec![bird, deer];
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        mgr.tick(0.1, &[], &att, &[], false);

        let d = mgr.animals.iter().find(|a| a.id == 2).unwrap();
        assert!(!matches!(d.state, WildlifeState::Fleeing { .. }),
            "異種不應互傳恐慌，野鹿不應因野鳥逃竄而炸群，實際 {:?}", d.state);
    }

    // ─── ROADMAP 210：晝夜作息 測試 ─────────────────────────────────────────

    #[test]
    fn prey_diurnal_predator_nocturnal() {
        // 獵物晝行（白天活躍、入夜歸巢眠）；掠食者夜行（入夜更活躍狩獵）。
        assert!(is_diurnal(WildlifeKind::WildDeer), "野鹿應為晝行性");
        assert!(is_diurnal(WildlifeKind::WildBird), "野鳥應為晝行性");
        assert!(is_diurnal(WildlifeKind::SmallCritter), "小動物應為晝行性");
        assert!(!is_diurnal(WildlifeKind::WildWolf), "狼應為夜行性（非晝行）");
        assert!(!is_diurnal(WildlifeKind::WildFox), "狐應為夜行性（非晝行）");
    }

    #[test]
    fn night_hunt_radius_expands_at_night() {
        // 夜間掠食者搜尋半徑放大；白天維持原 HUNT_RADIUS。
        assert_eq!(night_hunt_radius(false), HUNT_RADIUS, "白天搜尋半徑應為原值");
        assert!(night_hunt_radius(true) > HUNT_RADIUS, "夜間搜尋半徑應放大");
        assert!((night_hunt_radius(true) - HUNT_RADIUS * NIGHT_HUNT_RADIUS_MULT).abs() < 0.01);
    }

    #[test]
    fn diurnal_prey_heads_home_to_sleep_at_night() {
        // 夜間、無威脅時，遠離家的晝行獵物應朝家歸返（準備入眠）。
        let mut mgr = WildlifeManager::new();
        let mut deer = adult_at(WildlifeKind::WildDeer, 5000.0, 5000.0); // home=(5000,5000)
        deer.id = 1;
        deer.x = 5400.0; deer.y = 5000.0; // 離家 400px（遠在 HOME_ARRIVE_DIST 外）
        deer.state = WildlifeState::Wandering { target_x: 5500.0, target_y: 5000.0, wander_timer: 10.0 };
        mgr.animals = vec![deer];
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        mgr.tick(0.2, &[], &att, &[], true); // 夜間
        let d = mgr.animals.iter().find(|a| a.id == 1).unwrap();
        assert!(d.x < 5400.0, "夜間應朝家（西側）歸返，x 應變小，實際 x={}", d.x);
        assert!(matches!(d.state, WildlifeState::Returning), "歸返途中狀態應為 Returning，實際 {:?}", d.state);
    }

    #[test]
    fn sleeping_prey_stays_at_home_through_night() {
        // 已在家休息的晝行獵物，夜間應持續安睡（不甦醒去閒晃、位置不動）。
        let mut mgr = WildlifeManager::new();
        let mut deer = adult_at(WildlifeKind::WildDeer, 5000.0, 5000.0);
        deer.id = 1;
        deer.x = 5000.0; deer.y = 5000.0; // 就在家
        deer.state = WildlifeState::Resting { rest_timer: 0.5 }; // 即將到期的短休息
        mgr.animals = vec![deer];
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        // 連推進數秒（遠超 0.5 秒休息計時）。
        for _ in 0..10 { mgr.tick(1.0, &[], &att, &[], true); }
        let d = mgr.animals.iter().find(|a| a.id == 1).unwrap();
        assert!(matches!(d.state, WildlifeState::Resting { .. }),
            "夜間應持續安睡（不甦醒去閒晃），實際 {:?}", d.state);
        assert!((d.x - 5000.0).abs() < 1.0 && (d.y - 5000.0).abs() < 1.0,
            "安睡中位置應留在家，實際 ({},{})", d.x, d.y);
    }

    #[test]
    fn diurnal_prey_still_flees_threat_at_night() {
        use crate::combat::EnemyKind;
        // 夜間安睡的獵物，仍會被逼近的掠食威脅驚醒逃命（威脅永遠優先於入眠）。
        let mut mgr = WildlifeManager::new();
        let mut deer = adult_at(WildlifeKind::WildDeer, 5000.0, 5000.0);
        deer.id = 1;
        deer.state = WildlifeState::Resting { rest_timer: 10.0 };
        mgr.animals = vec![deer];
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        // 獵食野鹿的怪物（ScrapDrone）在 FLEE_RADIUS 內。
        mgr.tick(0.1, &[], &att, &[(EnemyKind::ScrapDrone, 5050.0, 5000.0)], true);
        let d = mgr.animals.iter().find(|a| a.id == 1).unwrap();
        assert!(matches!(d.state, WildlifeState::Fleeing { .. }),
            "夜間遇威脅仍應逃命（不繼續睡），實際 {:?}", d.state);
    }

    #[test]
    fn diurnal_prey_wakes_at_dawn() {
        // 入夜歸巢沉睡（長時 Resting）的晝行獵物，天亮後應主動甦醒、恢復閒晃，
        // 而非癱在家裡跨越整個白天（不靠那個比白天還長的夜眠計時器自然到期）。
        let mut mgr = WildlifeManager::new();
        let mut deer = adult_at(WildlifeKind::WildDeer, 5000.0, 5000.0);
        deer.id = 1;
        deer.x = 5000.0; deer.y = 5000.0; // 在家
        // 模擬入夜沉睡：長時 Resting（NIGHT_SLEEP_REST_SECS，遠大於日間小憩上限）。
        deer.state = WildlifeState::Resting { rest_timer: NIGHT_SLEEP_REST_SECS };
        mgr.animals = vec![deer];
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        // 推進幾幀「白天」（is_night=false）——夜眠計時器(600s)遠未到期，
        // 若只靠自然倒數會永遠醒不來，破曉甦醒邏輯必須主動喚醒。
        for _ in 0..3 { mgr.tick(0.2, &[], &att, &[], false); }
        let d = mgr.animals.iter().find(|a| a.id == 1).unwrap();
        assert!(!matches!(d.state, WildlifeState::Resting { .. }),
            "天亮後夜眠的晝行獵物應甦醒、離開沉睡，實際仍 {:?}", d.state);
    }

    #[test]
    fn dawn_does_not_interrupt_short_daytime_rest() {
        // 破曉甦醒只該喚醒「夜眠」（長時 Resting），不該打斷白天正常的短暫小憩；
        // 否則白天獵物會永遠無法停下休息。短休應如常倒數，不被當成夜眠強制喚醒。
        let mut mgr = WildlifeManager::new();
        let mut deer = adult_at(WildlifeKind::WildDeer, 5000.0, 5000.0);
        deer.id = 1;
        deer.x = 5000.0; deer.y = 5000.0;
        deer.state = WildlifeState::Resting { rest_timer: REST_TIMER_MAX }; // 日間正常小憩
        mgr.animals = vec![deer];
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        mgr.tick(0.1, &[], &att, &[], false); // 白天推進一小步（遠不足以耗盡小憩）
        let d = mgr.animals.iter().find(|a| a.id == 1).unwrap();
        assert!(matches!(d.state, WildlifeState::Resting { .. }),
            "白天的短暫小憩不應被破曉甦醒打斷，實際 {:?}", d.state);
    }

    #[test]
    fn dawn_wake_enters_stretching_first() {
        // ROADMAP 219：破曉甦醒不再瞬間起步——天明喚醒夜眠的晝行獵物時，應先轉入「伸展」
        // （Waking）一小段、原地不動，而非上一幀沉睡、下一幀就直接漫遊。
        let mut mgr = WildlifeManager::new();
        let mut deer = adult_at(WildlifeKind::WildDeer, 5000.0, 5000.0);
        deer.id = 1;
        deer.x = 5000.0; deer.y = 5000.0; // 在家
        deer.state = WildlifeState::Resting { rest_timer: NIGHT_SLEEP_REST_SECS }; // 夜眠
        mgr.animals = vec![deer];
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        mgr.tick(0.1, &[], &att, &[], false); // 破曉第一幀（白天、短到不足以伸展完）
        let d = mgr.animals.iter().find(|a| a.id == 1).unwrap();
        assert!(matches!(d.state, WildlifeState::Waking { .. }),
            "破曉喚醒應先轉入伸展（Waking），實際 {:?}", d.state);
        assert!((d.x - 5000.0).abs() < 1.0 && (d.y - 5000.0).abs() < 1.0,
            "伸展中應原地不動，實際 ({},{})", d.x, d.y);
    }

    #[test]
    fn waking_resumes_wandering_after_stretch() {
        // 伸展數秒後應起身投入新一天的閒晃（離開 Waking、不再賴在原地）。
        let mut mgr = WildlifeManager::new();
        let mut deer = adult_at(WildlifeKind::WildDeer, 5000.0, 5000.0);
        deer.id = 1;
        deer.x = 5000.0; deer.y = 5000.0;
        deer.state = WildlifeState::Resting { rest_timer: NIGHT_SLEEP_REST_SECS };
        mgr.animals = vec![deer];
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        // 推進總時長遠超過伸展上限（WAKE_DURATION_MAX=3.0s）：0.5s × 10 = 5s。
        for _ in 0..10 { mgr.tick(0.5, &[], &att, &[], false); }
        let d = mgr.animals.iter().find(|a| a.id == 1).unwrap();
        assert!(!matches!(d.state, WildlifeState::Waking { .. }),
            "伸展完應起身（離開 Waking），實際仍 {:?}", d.state);
        assert!(!matches!(d.state, WildlifeState::Resting { .. }),
            "伸展完不應又退回沉睡，實際 {:?}", d.state);
    }

    #[test]
    fn waking_flees_when_threatened() {
        use crate::combat::EnemyKind;
        // 威脅永遠優先：破曉伸展中若有掠食威脅逼近（FLEE_RADIUS 內），應立刻中斷伸展改逃竄。
        let mut mgr = WildlifeManager::new();
        let mut deer = adult_at(WildlifeKind::WildDeer, 5000.0, 5000.0);
        deer.id = 1;
        deer.x = 5000.0; deer.y = 5000.0;
        deer.state = WildlifeState::Waking { wake_timer: 2.0 }; // 正在伸展
        mgr.animals = vec![deer];
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        // 獵食野鹿的怪物在 FLEE_RADIUS 內、白天。
        mgr.tick(0.1, &[], &att, &[(EnemyKind::ScrapDrone, 5050.0, 5000.0)], false);
        let d = mgr.animals.iter().find(|a| a.id == 1).unwrap();
        assert!(matches!(d.state, WildlifeState::Fleeing { .. }),
            "伸展中遇威脅應立刻逃命（不繼續伸展），實際 {:?}", d.state);
    }

    #[test]
    fn nocturnal_predator_does_not_head_home_at_night() {
        // 夜行掠食者入夜不歸巢眠——無獵物時照常閒晃（朝漫遊目標移動，而非朝家歸返）。
        let mut mgr = WildlifeManager::new();
        let mut wolf = adult_at(WildlifeKind::WildWolf, 5000.0, 5000.0); // home=(5000,5000)
        wolf.id = 1;
        wolf.x = 5400.0; wolf.y = 5000.0; // 離家 400px（東側）
        wolf.state = WildlifeState::Wandering { target_x: 5800.0, target_y: 5000.0, wander_timer: 10.0 };
        mgr.animals = vec![wolf]; // 場上無獵物
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        mgr.tick(0.2, &[], &att, &[], true); // 夜間
        let w = mgr.animals.iter().find(|a| a.id == 1).unwrap();
        assert!(w.x > 5400.0, "夜行掠食者應朝漫遊目標（東側）移動、不歸巢，x 應變大，實際 x={}", w.x);
    }

    #[test]
    fn predator_night_hunt_reaches_farther_than_day() {
        // 同一場景：狼與一隻 400px 外的鹿（介於白天 320 與夜間 448 之間）。
        // 白天搆不著（不獵）；夜間搜尋範圍放大後搆得著（開始獵）。
        // ROADMAP 213：400px ＞ 撲擊距離(200)，故「開始獵」表現為先進潛行（Stalking）；
        // 兩者皆屬「已鎖定獵物、開始狩獵」，故都算數。
        let hunts_after_tick = |is_night: bool| -> bool {
            let mut mgr = WildlifeManager::new();
            let mut wolf = adult_at(WildlifeKind::WildWolf, 5000.0, 5000.0);
            wolf.id = 1;
            wolf.state = WildlifeState::Resting { rest_timer: 10.0 };
            let mut deer = adult_at(WildlifeKind::WildDeer, 5400.0, 5000.0); // 距狼 400px
            deer.id = 2;
            deer.state = WildlifeState::Resting { rest_timer: 10.0 };
            mgr.animals = vec![wolf, deer];
            let att: HashMap<WildlifeKind, i32> = HashMap::new();
            mgr.tick(0.1, &[], &att, &[], is_night);
            matches!(
                mgr.animals.iter().find(|a| a.id == 1).unwrap().state,
                WildlifeState::Hunting { .. } | WildlifeState::Stalking { .. }
            )
        };
        assert!(!hunts_after_tick(false), "白天 400px 外的鹿應搆不著、狼不獵");
        assert!(hunts_after_tick(true), "夜間搜尋範圍放大、400px 外的鹿搆得著、狼開始獵");
    }

    // ─── ROADMAP 211：白晝吃草 ───────────────────────────────────────────────

    #[test]
    fn arrival_grazes_when_prob_one() {
        // graze_prob=1 時，漫遊抵達目標的獵物應轉入吃草（而非休息）。
        let mut rng = make_rng();
        let mut deer = adult_at(WildlifeKind::WildDeer, 5000.0, 5000.0);
        // 已站在目標點上（dist<8 → 視為抵達）。
        deer.state = WildlifeState::Wandering { target_x: 5000.0, target_y: 5000.0, wander_timer: 10.0 };
        deer.tick_idle(0.1, &[], WANDER_SPEED, None, 1.0, &mut rng);
        assert!(matches!(deer.state, WildlifeState::Grazing { .. }),
            "抵達目標且 graze_prob=1 應轉入吃草，實際 {:?}", deer.state);
    }

    #[test]
    fn arrival_rests_not_grazes_when_prob_zero() {
        // graze_prob=0（掠食者/夜間）時，抵達目標一律轉入休息、絕不吃草（與切片前一致）。
        let mut rng = make_rng();
        let mut deer = adult_at(WildlifeKind::WildDeer, 5000.0, 5000.0);
        deer.state = WildlifeState::Wandering { target_x: 5000.0, target_y: 5000.0, wander_timer: 10.0 };
        deer.tick_idle(0.1, &[], WANDER_SPEED, None, 0.0, &mut rng);
        assert!(matches!(deer.state, WildlifeState::Resting { .. }),
            "graze_prob=0 抵達目標應休息、不吃草，實際 {:?}", deer.state);
    }

    #[test]
    fn grazing_stays_still_then_returns_to_wander() {
        // 吃草中原地不動（座標不變）；計時到期後回到漫遊。
        let mut rng = make_rng();
        let mut deer = adult_at(WildlifeKind::WildDeer, 5000.0, 5000.0);
        deer.state = WildlifeState::Grazing { graze_timer: 0.3 };
        deer.tick_idle(0.1, &[], WANDER_SPEED, None, GRAZE_PROB, &mut rng);
        assert!(matches!(deer.state, WildlifeState::Grazing { .. }), "未到期仍應吃草中");
        assert!((deer.x - 5000.0).abs() < 0.01 && (deer.y - 5000.0).abs() < 0.01,
            "吃草中位置應不動，實際 ({},{})", deer.x, deer.y);
        // 再推進到超過計時 → 回漫遊。
        deer.tick_idle(0.5, &[], WANDER_SPEED, None, GRAZE_PROB, &mut rng);
        assert!(matches!(deer.state, WildlifeState::Wandering { .. }),
            "吃草到期後應回到漫遊，實際 {:?}", deer.state);
    }

    #[test]
    fn grazing_prey_flees_on_threat() {
        // 吃草中的獵物，威脅進入 FLEE_RADIUS 仍應立即逃命（威脅永遠優先於吃草）。
        let mut rng = make_rng();
        let mut deer = adult_at(WildlifeKind::WildDeer, 5000.0, 5000.0);
        deer.state = WildlifeState::Grazing { graze_timer: 5.0 };
        deer.tick_idle(0.1, &[(5050.0, 5000.0)], WANDER_SPEED, None, GRAZE_PROB, &mut rng);
        assert!(matches!(deer.state, WildlifeState::Fleeing { .. }),
            "吃草中遇威脅應改逃竄，實際 {:?}", deer.state);
    }

    #[test]
    fn predator_never_grazes_during_day() {
        // 整管理器白天連跑多幀：掠食者（狼/狐）永遠不會進入吃草狀態。
        let mut mgr = WildlifeManager::new();
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        for _ in 0..200 { mgr.tick(0.2, &[], &att, &[], false); }
        let pred_grazing = mgr.animals.iter().any(|a|
            a.kind.trophic_level() == TrophicLevel::Predator
            && matches!(a.state, WildlifeState::Grazing { .. }));
        assert!(!pred_grazing, "掠食者不該吃草");
    }

    #[test]
    fn prey_eventually_grazes_during_day_but_not_at_night() {
        // 白天連跑多幀：晝行獵物群中至少有一隻會吃草（白晝吃草確實會發生）。
        let mut day = WildlifeManager::new();
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        let mut saw_graze_by_day = false;
        for _ in 0..400 {
            day.tick(0.2, &[], &att, &[], false);
            if day.animals.iter().any(|a| matches!(a.state, WildlifeState::Grazing { .. })) {
                saw_graze_by_day = true;
                break;
            }
        }
        assert!(saw_graze_by_day, "白天連跑多幀後，應有晝行獵物吃草");

        // 夜間連跑同樣多幀：獵物入夜歸巢沉睡、絕不吃草。
        let mut night = WildlifeManager::new();
        for _ in 0..400 {
            night.tick(0.2, &[], &att, &[], true);
            let any_graze = night.animals.iter().any(|a| matches!(a.state, WildlifeState::Grazing { .. }));
            assert!(!any_graze, "夜間獵物應沉睡、不吃草");
        }
    }

    // ─── ROADMAP 212：群體警戒哨 ─────────────────────────────────────────────

    #[test]
    fn herd_sentinel_is_lowest_id_in_group() {
        // 同種三隻成體聚在一起：只有 id 最小者（id=2）擔任哨兵，其餘不是。
        let snap = vec![
            (5u32, WildlifeKind::WildDeer, 0.0_f32, 0.0_f32),
            (2u32, WildlifeKind::WildDeer, 30.0, 0.0),
            (8u32, WildlifeKind::WildDeer, 0.0, 40.0),
        ];
        assert!(herd_sentinel(2, WildlifeKind::WildDeer, 30.0, 0.0, &snap), "群內最小 id 應為哨兵");
        assert!(!herd_sentinel(5, WildlifeKind::WildDeer, 0.0, 0.0, &snap), "非最小 id 不該是哨兵");
        assert!(!herd_sentinel(8, WildlifeKind::WildDeer, 0.0, 40.0, &snap), "非最小 id 不該是哨兵");
    }

    #[test]
    fn lone_animal_is_not_sentinel() {
        // 孤獸（半徑內無同種夥伴）不必放哨——不成群就沒有哨兵。
        let snap = vec![(3u32, WildlifeKind::WildDeer, 0.0_f32, 0.0_f32)];
        assert!(!herd_sentinel(3, WildlifeKind::WildDeer, 0.0, 0.0, &snap),
            "孤獸不成群、不該設哨兵");
    }

    #[test]
    fn herd_sentinel_excludes_other_species_and_far_kin() {
        // 不同物種與超出 SENTINEL_HERD_RADIUS 的同種都不算同群——故仍是孤獸、無哨兵。
        let snap = vec![
            (0u32, WildlifeKind::WildDeer, 0.0_f32, 0.0_f32),                    // 自己
            (1u32, WildlifeKind::WildBird, 20.0, 0.0),                           // 異種 → 排除
            (2u32, WildlifeKind::WildDeer, SENTINEL_HERD_RADIUS + 50.0, 0.0),    // 超範圍 → 排除
        ];
        assert!(!herd_sentinel(0, WildlifeKind::WildDeer, 0.0, 0.0, &snap),
            "異種與超範圍同種都不算同群，自己應視為孤獸、無哨兵");
    }

    #[test]
    fn sentinel_flees_threat_outside_normal_radius() {
        // 哨兵的放大警戒：在「一般 FLEE_RADIUS 之外、SENTINEL_FLEE_RADIUS 之內」就先察覺逃竄。
        // 同場兩隻成鹿成群（觸發哨兵），威脅放在 240px（>180 一般半徑、<300 哨兵半徑）。
        let mut mgr = WildlifeManager::new();
        let mut s = adult_at(WildlifeKind::WildDeer, 5000.0, 5000.0); // 哨兵（id 最小）
        s.id = 1;
        s.state = WildlifeState::Watching { watch_timer: 5.0 };
        let mut mate = adult_at(WildlifeKind::WildDeer, 5060.0, 5000.0); // 同群夥伴
        mate.id = 2;
        mate.state = WildlifeState::Resting { rest_timer: 5.0 };
        mgr.animals = vec![s, mate];
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        // 威脅在哨兵正東 240px：超出一般 FLEE_RADIUS(180)、落在 SENTINEL_FLEE_RADIUS(300) 內。
        mgr.tick(0.1, &[(5240.0, 5000.0)], &att, &[], false);
        let sent = mgr.animals.iter().find(|a| a.id == 1).unwrap();
        assert!(matches!(sent.state, WildlifeState::Fleeing { .. }),
            "哨兵應以放大半徑提早察覺 240px 外的威脅而逃竄，實際 {:?}", sent.state);
    }

    #[test]
    fn non_sentinel_ignores_threat_outside_normal_radius() {
        // 對照：同樣 240px 的威脅，對「非哨兵」的一般獵物來說仍在 FLEE_RADIUS(180) 之外、
        // 不該觸發逃竄——證明提早察覺是哨兵獨有的放大警戒，而非全體。
        let mut rng = make_rng();
        let mut deer = adult_at(WildlifeKind::WildDeer, 5000.0, 5000.0);
        deer.state = WildlifeState::Resting { rest_timer: 5.0 };
        deer.tick_idle(0.1, &[(5240.0, 5000.0)], WANDER_SPEED, None, GRAZE_PROB, &mut rng);
        assert!(!matches!(deer.state, WildlifeState::Fleeing { .. }),
            "一般獵物對 240px（>FLEE_RADIUS）的威脅不該逃，實際 {:?}", deer.state);
    }

    #[test]
    fn herd_posts_exactly_one_sentinel_during_day() {
        // 整管理器白天連跑多幀：成群的成鹿中應「至少有一隻」站崗放哨（watching），
        // 且同一群（同種、彼此在哨兵半徑內）至多一隻——一隻看守、其餘安心活動。
        let mut mgr = WildlifeManager::new();
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        let mut saw_watch = false;
        // 上一幀「身分不符」的放哨者 id 集合。放哨身分以「決策當幀起始（pre-tick）」的群組為準，
        // 但這裡是在 tick 後（post-tick）用已位移的座標重算——故群組剛合併/剛離散的「那一幀」，
        // 會出現短暫的身分不符（例如哨兵的同群這一幀正好走散、剩牠一隻）。這是合法的單幀過渡，
        // 下一幀必被收斂（卸任哨兵釋放回休息，見 Phase 4 的 ROADMAP 212 修補）。真正要抓的壞味道
        // 是「卡死」——身分不符卻一直賴在 Watching。故不變式改為：同一隻的身分不符**不得連跨兩幀**。
        let mut prev_bad: std::collections::HashSet<u32> = std::collections::HashSet::new();
        for _ in 0..400 {
            mgr.tick(0.2, &[], &att, &[], false);
            let watchers: Vec<&Wildlife> = mgr.animals.iter()
                .filter(|a| a.alive && matches!(a.state, WildlifeState::Watching { .. }))
                .collect();
            if !watchers.is_empty() {
                saw_watch = true;
            }
            // 每隻放哨者都應是其同群（同種、SENTINEL_HERD_RADIUS 內成體）的最小 id（去中心、每群恰一隻）。
            let adult_snap: Vec<(u32, WildlifeKind, f32, f32)> = mgr.animals.iter()
                .filter(|a| a.alive && a.kind.trophic_level() == TrophicLevel::Prey && !a.is_juvenile())
                .map(|a| (a.id, a.kind, a.x, a.y)).collect();
            let bad: std::collections::HashSet<u32> = watchers.iter()
                .filter(|w| !herd_sentinel(w.id, w.kind, w.x, w.y, &adult_snap))
                .map(|w| w.id)
                .collect();
            // 連跨兩幀仍身分不符 = 卡死的滯留哨兵（如永遠出不了 Watching 的孤獸）→ 不合法。
            let stuck: Vec<u32> = bad.intersection(&prev_bad).copied().collect();
            assert!(stuck.is_empty(),
                "放哨者身分不符不得連跨兩幀（卸任哨兵應於次幀釋放回休息）：卡死 id={:?}", stuck);
            prev_bad = bad;
        }
        assert!(saw_watch, "白天連跑多幀後，成群獵物中應出現站崗放哨者");
    }

    #[test]
    fn no_sentinel_at_night() {
        // 夜間獵物歸巢沉睡，不站崗放哨（哨兵只在白天）。
        let mut mgr = WildlifeManager::new();
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        for _ in 0..400 {
            mgr.tick(0.2, &[], &att, &[], true);
            let any_watch = mgr.animals.iter().any(|a| matches!(a.state, WildlifeState::Watching { .. }));
            assert!(!any_watch, "夜間不該有獵物放哨");
        }
    }

    #[test]
    fn predator_never_watches() {
        // 掠食者（狼/狐）不放哨——放哨是成群獵物的行為。
        let mut mgr = WildlifeManager::new();
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        for _ in 0..200 { mgr.tick(0.2, &[], &att, &[], false); }
        let pred_watch = mgr.animals.iter().any(|a|
            a.kind.trophic_level() == TrophicLevel::Predator
            && matches!(a.state, WildlifeState::Watching { .. }));
        assert!(!pred_watch, "掠食者不該放哨");
    }

    #[test]
    fn watching_drifts_with_herd_after_timer() {
        // 站崗到期：哨兵應跟群挪步（轉入 Wandering），不會永遠釘在原地被群拋下。
        let mut rng = make_rng();
        let mut s = adult_at(WildlifeKind::WildDeer, 5000.0, 5000.0);
        s.state = WildlifeState::Watching { watch_timer: 0.05 };
        s.tick_watch(0.1, Some((5200.0, 5000.0)), &mut rng);
        assert!(matches!(s.state, WildlifeState::Wandering { .. }),
            "站崗到期應轉入漫遊跟群，實際 {:?}", s.state);
    }

    // ─── ROADMAP 213：孤獵潛行突襲 ───────────────────────────────────────────
    #[test]
    fn within_pounce_range_boundary() {
        // 撲擊距離邊界判定：等於 POUNCE_RANGE 算「在範圍內」，略大則否。
        assert!(within_pounce_range(POUNCE_RANGE - 1.0), "撲擊距離內應為 true");
        assert!(within_pounce_range(POUNCE_RANGE), "恰在邊界應為 true");
        assert!(!within_pounce_range(POUNCE_RANGE + 1.0), "超出撲擊距離應為 false");
    }

    #[test]
    fn stalk_creep_returns_none_within_pounce_range() {
        // 已在撲擊距離內：回傳 None，示意呼叫端該爆衝轉入追獵。
        let r = stalk_creep(5000.0, 5000.0, 5000.0 + POUNCE_RANGE - 10.0, 5000.0, 0.1);
        assert!(r.is_none(), "撲擊距離內應回傳 None（該爆衝），實際 {:?}", r);
    }

    #[test]
    fn stalk_creep_moves_toward_prey_when_far() {
        // 撲擊距離外：壓低身子緩緩潛近，回傳更靠近獵物的新位置。
        let (px, py) = (5000.0_f32, 5000.0_f32);
        let (preyx, preyy) = (5400.0_f32, 5000.0_f32); // 400px 遠（> POUNCE_RANGE 200）
        let before = ((preyx - px).powi(2) + (preyy - py).powi(2)).sqrt();
        let (nx, ny) = stalk_creep(px, py, preyx, preyy, 0.1).expect("遠距離應回傳潛近新位置");
        let after = ((preyx - nx).powi(2) + (preyy - ny).powi(2)).sqrt();
        assert!(after < before, "潛行後應更靠近獵物（{after} < {before}）");
        // 單幀位移約 STALK_SPEED*dt（方向正確、沿直線）。
        let moved = ((nx - px).powi(2) + (ny - py).powi(2)).sqrt();
        assert!((moved - STALK_SPEED * 0.1).abs() < 0.01,
            "單幀位移應約 STALK_SPEED*dt，實際 {moved}");
    }

    #[test]
    fn stalk_creep_does_not_overshoot_prey() {
        // 潛近單幀位移受「與獵物距離」上限約束——不會越過獵物（最壞貼到原地）。
        // 構造一個 dt 大到 STALK_SPEED*dt 會超過剩餘距離的情境（但距離仍 > POUNCE_RANGE 才會 creep，
        // 故改以「剛好超出撲擊距離一點」配大 dt 驗證 step 被 dist clamp）。
        let (px, py) = (5000.0_f32, 5000.0_f32);
        let (preyx, preyy) = (5000.0 + POUNCE_RANGE + 5.0, 5000.0); // 略超撲擊距離
        let (nx, _ny) = stalk_creep(px, py, preyx, preyy, 100.0).expect("應仍在潛近");
        assert!(nx <= preyx + 0.001, "潛近不可越過獵物 x（{nx} <= {preyx}）");
    }

    #[test]
    fn stalk_speed_strictly_slower_than_hunt() {
        // 潛行必須遠慢於全速追獵——這正是「潛近 vs 撲擊」張力的來源。
        assert!(STALK_SPEED < HUNT_SPEED, "潛速應嚴格小於追速");
    }

    #[test]
    fn pounce_range_between_flee_and_sentinel() {
        // 撲擊距離刻意 ＞ 一般獵物警戒(FLEE_RADIUS) 且 ＜ 哨兵警戒(SENTINEL_FLEE_RADIUS)：
        // 爆衝恰在獵物即將察覺的剎那，且哨兵仍能在掠食者潛行時提早發現。
        assert!(POUNCE_RANGE > FLEE_RADIUS,
            "撲擊距離應大於一般警戒，爆衝才在獵物即將察覺時發動");
        assert!(POUNCE_RANGE < SENTINEL_FLEE_RADIUS,
            "撲擊距離應小於哨兵警戒，哨兵才能在掠食者潛行時先發現");
    }

    #[test]
    fn predator_stalks_when_prey_far_pounces_when_near() {
        // 整管理器：把一隻狼放在遠處鹿的搜尋半徑內、但撲擊距離外，連跑幾幀後
        // 該狼應進入「潛行（stalking）」而非立刻全速追獵（hunting）。
        let mut mgr = WildlifeManager::new();
        // 強制布置：第一隻狼移到 (5000,5000)，最近的鹿放到撲擊距離外、搜尋半徑內（260px）。
        let mut wolf_idx = None;
        let mut deer_idx = None;
        for (i, a) in mgr.animals.iter().enumerate() {
            if wolf_idx.is_none() && a.kind == WildlifeKind::WildWolf { wolf_idx = Some(i); }
            else if deer_idx.is_none() && a.kind == WildlifeKind::WildDeer { deer_idx = Some(i); }
        }
        let (wi, di) = (wolf_idx.unwrap(), deer_idx.unwrap());
        // 把其他鹿挪到極遠處，確保 di 是狼的最近獵物（不被別的鹿搶鎖定）。
        for (i, a) in mgr.animals.iter_mut().enumerate() {
            if i != di && a.kind == WildlifeKind::WildDeer { a.x = 90000.0; a.y = 90000.0; }
        }
        mgr.animals[wi].x = 5000.0; mgr.animals[wi].y = 5000.0;
        mgr.animals[wi].state = WildlifeState::Returning;
        mgr.animals[di].x = 5260.0; mgr.animals[di].y = 5000.0; // 260px：> POUNCE(200)、< HUNT_RADIUS(320)
        mgr.animals[di].state = WildlifeState::Resting { rest_timer: 100.0 };
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        // 跑一幀：狼應鎖定該鹿並進入潛行（而非 hunting）。鹿此時離狼 260 > FLEE_RADIUS，不會逃。
        mgr.tick(0.05, &[], &att, &[], false);
        assert!(matches!(mgr.animals[wi].state, WildlifeState::Stalking { .. }),
            "遠距離發現獵物應先潛行，實際 {:?}", mgr.animals[wi].state);
    }

    #[test]
    fn predator_pounces_immediately_when_prey_already_close() {
        // 對照：獵物一開始就貼在撲擊距離內，狼應「直接」進入全速追獵（不必再潛）。
        let mut mgr = WildlifeManager::new();
        let mut wolf_idx = None;
        let mut deer_idx = None;
        for (i, a) in mgr.animals.iter().enumerate() {
            if wolf_idx.is_none() && a.kind == WildlifeKind::WildWolf { wolf_idx = Some(i); }
            else if deer_idx.is_none() && a.kind == WildlifeKind::WildDeer { deer_idx = Some(i); }
        }
        let (wi, di) = (wolf_idx.unwrap(), deer_idx.unwrap());
        for (i, a) in mgr.animals.iter_mut().enumerate() {
            if i != di && a.kind == WildlifeKind::WildDeer { a.x = 90000.0; a.y = 90000.0; }
        }
        mgr.animals[wi].x = 5000.0; mgr.animals[wi].y = 5000.0;
        mgr.animals[wi].state = WildlifeState::Returning;
        mgr.animals[di].x = 5120.0; mgr.animals[di].y = 5000.0; // 120px < POUNCE_RANGE(200)
        mgr.animals[di].state = WildlifeState::Resting { rest_timer: 100.0 };
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        mgr.tick(0.05, &[], &att, &[], false);
        assert!(matches!(mgr.animals[wi].state, WildlifeState::Hunting { .. }),
            "貼近獵物應直接撲（hunting），實際 {:?}", mgr.animals[wi].state);
    }

    // ─── ROADMAP 214：母獸護幼 ───────────────────────────────────────────────

    #[test]
    fn nearest_adult_id_picks_closest_same_kind() {
        // nearest_adult_id_of_kind 應回傳半徑內、同種、離目標最近的成體 id。
        let snap = vec![
            (10u32, WildlifeKind::WildDeer, 5300.0, 5000.0), // 遠
            (11u32, WildlifeKind::WildDeer, 5050.0, 5000.0), // 近 ← 應選這隻
            (12u32, WildlifeKind::WildBird, 5010.0, 5000.0), // 更近但異種，不算
        ];
        let id = nearest_adult_id_of_kind(WildlifeKind::WildDeer, 5000.0, 5000.0, &snap, 240.0);
        assert_eq!(id, Some(11), "應選同種中最近的成體");
        // 範圍外則 None。
        let none = nearest_adult_id_of_kind(WildlifeKind::WildDeer, 0.0, 0.0, &snap, 240.0);
        assert_eq!(none, None, "半徑內無同種成體應為 None");
    }

    #[test]
    fn defend_charge_moves_toward_predator_without_overshoot() {
        // 護幼衝刺朝掠食者移動 DEFEND_SPEED*dt；單幀位移受距離上限約束，不越過掠食者。
        let (ax, ay) = (5000.0_f32, 5000.0_f32);
        let (px, py) = (5400.0_f32, 5000.0_f32); // 400px 遠
        let (nx, ny) = defend_charge(ax, ay, px, py, 0.1);
        let moved = ((nx - ax).powi(2) + (ny - ay).powi(2)).sqrt();
        assert!((moved - DEFEND_SPEED * 0.1).abs() < 0.01, "單幀位移應約 DEFEND_SPEED*dt，實際 {moved}");
        assert!(nx > ax, "應朝掠食者（東側）移動");
        // 大 dt 不越過掠食者（step 受距離 clamp）。
        let (nx2, _) = defend_charge(ax, ay, px, py, 100.0);
        assert!(nx2 <= px + 0.001, "不可衝過掠食者");
    }

    #[test]
    fn defend_charge_faster_than_stalk() {
        // 護幼衝刺需快過掠食者潛行，母獸才來得及擋在幼獸與狼之間。
        assert!(DEFEND_SPEED > STALK_SPEED, "護幼衝刺應快於掠食者潛行");
    }

    fn threatened(juv_id: u32, kind: WildlifeKind, jx: f32, jy: f32, px: f32, py: f32)
        -> (u32, WildlifeKind, f32, f32, f32, f32) { (juv_id, kind, jx, jy, px, py) }

    #[test]
    fn defend_target_when_self_is_nearest_adult() {
        // 同種受脅幼獸在護衛半徑內、且自己是離牠最近的成體 → 回傳掠食者座標（該衝去護衛）。
        let self_id = 2u32;
        let adult_snap = vec![(2u32, WildlifeKind::WildDeer, 5100.0, 5000.0)];
        let threats = vec![threatened(3, WildlifeKind::WildDeer, 5050.0, 5000.0, 4900.0, 5000.0)];
        let r = defend_target(self_id, WildlifeKind::WildDeer, 5100.0, 5000.0, &threats, &adult_snap);
        assert_eq!(r, Some((4900.0, 5000.0)), "應回傳威脅掠食者的座標");
    }

    #[test]
    fn defend_target_none_when_fawn_too_far() {
        // 受脅幼獸在護衛半徑外 → 不護（None）。
        let adult_snap = vec![(2u32, WildlifeKind::WildDeer, 5000.0, 5000.0)];
        // 幼獸遠在 DEFEND_GUARD_RADIUS(240) 之外。
        let threats = vec![threatened(3, WildlifeKind::WildDeer, 5000.0 + 400.0, 5000.0, 5500.0, 5000.0)];
        let r = defend_target(2, WildlifeKind::WildDeer, 5000.0, 5000.0, &threats, &adult_snap);
        assert_eq!(r, None, "幼獸太遠不該護");
    }

    #[test]
    fn defend_target_none_when_other_adult_is_closer() {
        // 另一隻成體離受脅幼獸更近（牠才是母獸）→ 自己不出面（避免整群暴衝）。
        let adult_snap = vec![
            (2u32, WildlifeKind::WildDeer, 5200.0, 5000.0), // 自己，較遠
            (9u32, WildlifeKind::WildDeer, 5060.0, 5000.0), // 別隻，較近 ← 牠才護
        ];
        let threats = vec![threatened(3, WildlifeKind::WildDeer, 5050.0, 5000.0, 4900.0, 5000.0)];
        let r = defend_target(2, WildlifeKind::WildDeer, 5200.0, 5000.0, &threats, &adult_snap);
        assert_eq!(r, None, "不是最近成體就不該出面護衛");
    }

    #[test]
    fn defend_target_ignores_other_kind_fawn() {
        // 異種受脅幼獸不護（鹿不為鳥的幼獸挺身）。
        let adult_snap = vec![(2u32, WildlifeKind::WildDeer, 5050.0, 5000.0)];
        let threats = vec![threatened(3, WildlifeKind::WildBird, 5050.0, 5000.0, 4900.0, 5000.0)];
        let r = defend_target(2, WildlifeKind::WildDeer, 5050.0, 5000.0, &threats, &adult_snap);
        assert_eq!(r, None, "不該為異種幼獸護衛");
    }

    #[test]
    fn doe_enters_defending_when_wolf_hunts_nearby_fawn() {
        // 整管理器：狼正追獵一隻幼鹿，附近最近的成鹿（母獸）應挺身轉入 Defending（不逃）。
        let mut mgr = WildlifeManager::new();
        let mut wolf = adult_at(WildlifeKind::WildWolf, 5000.0, 5000.0);
        wolf.id = 1;
        wolf.state = WildlifeState::Hunting { target_id: 3, hunt_timer: 10.0 };
        let mut doe = adult_at(WildlifeKind::WildDeer, 5120.0, 5000.0);
        doe.id = 2;
        let mut fawn = juvenile_at(WildlifeKind::WildDeer, 5040.0, 5000.0);
        fawn.id = 3;
        mgr.animals = vec![wolf, doe, fawn];
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        mgr.tick(0.1, &[], &att, &[], false);
        let d = mgr.animals.iter().find(|a| a.id == 2).unwrap();
        assert!(matches!(d.state, WildlifeState::Defending),
            "狼追獵幼鹿時，最近的成鹿應挺身護幼（Defending），實際 {:?}", d.state);
    }

    #[test]
    fn defending_doe_drives_off_wolf() {
        // 護幼成鹿逼到威嚇半徑內，正在追獵幼鹿的狼應放棄、退走（Returning）。
        let mut mgr = WildlifeManager::new();
        let mut wolf = adult_at(WildlifeKind::WildWolf, 5000.0, 5000.0);
        wolf.id = 1;
        wolf.state = WildlifeState::Hunting { target_id: 3, hunt_timer: 10.0 };
        let mut doe = adult_at(WildlifeKind::WildDeer, 5050.0, 5000.0); // 距狼 50px < INTIMIDATE(90)
        doe.id = 2;
        doe.state = WildlifeState::Defending; // 已在護幼姿態
        let mut fawn = juvenile_at(WildlifeKind::WildDeer, 5030.0, 5000.0);
        fawn.id = 3;
        mgr.animals = vec![wolf, doe, fawn];
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        mgr.tick(0.1, &[], &att, &[], false);
        let w = mgr.animals.iter().find(|a| a.id == 1).unwrap();
        assert!(matches!(w.state, WildlifeState::Returning),
            "護幼成鹿逼近時，狼應被嚇退（Returning），實際 {:?}", w.state);
    }

    #[test]
    fn wolf_not_intimidated_by_distant_defender() {
        // 護幼成鹿在威嚇半徑外時，狼不受影響（照常追獵），確認嚇退是距離觸發、非旗標硬設。
        let mut mgr = WildlifeManager::new();
        let mut wolf = adult_at(WildlifeKind::WildWolf, 5000.0, 5000.0);
        wolf.id = 1;
        wolf.state = WildlifeState::Hunting { target_id: 3, hunt_timer: 10.0 };
        let mut doe = adult_at(WildlifeKind::WildDeer, 5400.0, 5000.0); // 400px > INTIMIDATE(90)
        doe.id = 2;
        doe.state = WildlifeState::Defending;
        // 幼鹿在狼搜尋半徑內、但已超出擊殺距離（>KILL_RADIUS，本幀不會被咬死），
        // 且離那隻遠方護衛成鹿太遠（>DEFEND_GUARD_RADIUS），故該成鹿本幀不會續護。
        let mut fawn = juvenile_at(WildlifeKind::WildDeer, 5060.0, 5000.0);
        fawn.id = 3;
        mgr.animals = vec![wolf, doe, fawn];
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        mgr.tick(0.1, &[], &att, &[], false);
        let w = mgr.animals.iter().find(|a| a.id == 1).unwrap();
        assert!(matches!(w.state, WildlifeState::Hunting { .. }),
            "遠方護衛成鹿不該嚇退狼，狼應續獵，實際 {:?}", w.state);
    }

    #[test]
    fn predator_not_a_defender_target() {
        // 掠食者自己不會「護幼」——defend 只作用於獵物成體（掠食者無同種幼獸可護）。
        // 連跑整管理器多幀，斷言任何掠食者都不會進入 Defending。
        let mut mgr = WildlifeManager::new();
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        for _ in 0..200 {
            mgr.tick(0.2, &[], &att, &[], false);
            let pred_defend = mgr.animals.iter().any(|a|
                a.kind.trophic_level() == TrophicLevel::Predator
                && matches!(a.state, WildlifeState::Defending));
            assert!(!pred_defend, "掠食者不該護幼");
        }
    }

    // ─── ROADMAP 215：幼獸嬉戲 ───────────────────────────────────────────────

    #[test]
    fn frolic_target_stays_within_radius_of_mother() {
        // 蹦跳落點永遠落在母獸周圍 FROLIC_RADIUS 內（玩不離媽媽）。
        let mut rng = make_rng();
        let (mx, my) = (5000.0_f32, 5000.0_f32);
        for _ in 0..200 {
            let (tx, ty) = frolic_target(mx, my, &mut rng);
            let d = ((tx - mx).powi(2) + (ty - my).powi(2)).sqrt();
            assert!(d <= FROLIC_RADIUS + 0.01, "蹦跳落點不該離母獸超過 FROLIC_RADIUS，實際 {d}");
        }
    }

    #[test]
    fn frolic_hop_moves_toward_target_without_overshoot() {
        // 蹦跳朝落點移動 FROLIC_SPEED*dt；單幀位移受距離 clamp，不會蹦過頭。
        let (x, y) = (5000.0_f32, 5000.0_f32);
        let (hx, hy) = (5300.0_f32, 5000.0_f32); // 300px 遠
        let (nx, ny) = frolic_hop(x, y, hx, hy, 0.1);
        let moved = ((nx - x).powi(2) + (ny - y).powi(2)).sqrt();
        assert!((moved - FROLIC_SPEED * 0.1).abs() < 0.01, "單幀位移應約 FROLIC_SPEED*dt，實際 {moved}");
        assert!(nx > x && (ny - y).abs() < 0.001, "應朝落點（東側）直線蹦去");
        // 大 dt 不蹦過落點（step 受距離 clamp）。
        let (nx2, _) = frolic_hop(x, y, hx, hy, 100.0);
        assert!(nx2 <= hx + 0.001, "不可蹦過落點");
    }

    #[test]
    fn frolic_hop_faster_than_nurse_follow() {
        // 玩耍蹦跳要比依偎跟隨活潑（快），幼獸玩起來才像在蹦蹦跳跳。
        assert!(FROLIC_SPEED > NURSE_SPEED, "嬉戲蹦跳應快於依偎跟隨");
    }

    #[test]
    fn tick_frolic_returns_to_nursing_when_reached() {
        // 已蹦到落點（在 FROLIC_REACH 內）→ tick_frolic 收斂回依偎（朝母獸的 Wandering）。
        let mut w = juvenile_at(WildlifeKind::WildDeer, 5000.0, 5000.0);
        w.state = WildlifeState::Frolicking { hop_x: 5002.0, hop_y: 5000.0, frolic_timer: 5.0 };
        w.tick_frolic(0.1, 4980.0, 5000.0);
        match w.state {
            WildlifeState::Wandering { target_x, target_y, .. } => {
                assert!((target_x - 4980.0).abs() < 0.001 && (target_y - 5000.0).abs() < 0.001,
                    "玩夠應回母獸身邊依偎");
            }
            other => panic!("到達落點後應回依偎（Wandering），實際 {other:?}"),
        }
    }

    #[test]
    fn tick_frolic_returns_to_nursing_when_timer_expires() {
        // 計時耗盡（即使還沒蹦到落點）→ 也收尾回依偎，不會玩個沒完。
        let mut w = juvenile_at(WildlifeKind::WildDeer, 5000.0, 5000.0);
        w.state = WildlifeState::Frolicking { hop_x: 5300.0, hop_y: 5000.0, frolic_timer: 0.05 };
        w.tick_frolic(0.1, 4990.0, 5000.0); // dt > timer
        assert!(matches!(w.state, WildlifeState::Wandering { .. }),
            "計時耗盡應收尾回依偎，實際 {:?}", w.state);
    }

    #[test]
    fn fawn_frolics_near_mother_in_daytime() {
        // 整管理器：白天，一隻已貼在母鹿身邊的幼鹿，連跑多幀後應有機會進入嬉戲（Frolicking）。
        let mut mgr = WildlifeManager::new();
        let mut doe = adult_at(WildlifeKind::WildDeer, 5000.0, 5000.0);
        doe.id = 1;
        doe.state = WildlifeState::Resting { rest_timer: 100000.0 }; // 母獸定住，便於觀察幼獸
        let mut fawn = juvenile_at(WildlifeKind::WildDeer, 5010.0, 5000.0); // 已在舒適距離內
        fawn.id = 2;
        mgr.animals = vec![doe, fawn];
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        let mut saw_frolic = false;
        for _ in 0..2000 {
            mgr.tick(0.1, &[], &att, &[], false); // is_night=false
            let f = mgr.animals.iter().find(|a| a.id == 2).unwrap();
            if matches!(f.state, WildlifeState::Frolicking { .. }) { saw_frolic = true; break; }
        }
        assert!(saw_frolic, "白天依偎在母獸身邊的幼獸應會開始嬉戲");
    }

    #[test]
    fn fawn_does_not_frolic_at_night() {
        // 夜間：幼獸只依偎不玩耍——連跑多幀都不該進入 Frolicking。
        let mut mgr = WildlifeManager::new();
        let mut doe = adult_at(WildlifeKind::WildDeer, 5000.0, 5000.0);
        doe.id = 1;
        let mut fawn = juvenile_at(WildlifeKind::WildDeer, 5010.0, 5000.0);
        fawn.id = 2;
        mgr.animals = vec![doe, fawn];
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        for _ in 0..2000 {
            mgr.tick(0.1, &[], &att, &[], true); // is_night=true
            let f = mgr.animals.iter().find(|a| a.id == 2).unwrap();
            assert!(!matches!(f.state, WildlifeState::Frolicking { .. }), "夜間幼獸不該嬉戲");
        }
    }

    #[test]
    fn frolicking_fawn_flees_when_predator_approaches() {
        // 威脅優先：正在嬉戲的幼獸，掠食者逼近時應改逃竄（不會繼續玩）。
        let mut mgr = WildlifeManager::new();
        let mut doe = adult_at(WildlifeKind::WildDeer, 5000.0, 5000.0);
        doe.id = 1;
        let mut fawn = juvenile_at(WildlifeKind::WildDeer, 5010.0, 5000.0);
        fawn.id = 2;
        fawn.state = WildlifeState::Frolicking { hop_x: 5030.0, hop_y: 5000.0, frolic_timer: 5.0 };
        // 狼貼近幼獸（FLEE_RADIUS 內），形成直接威脅。
        let mut wolf = adult_at(WildlifeKind::WildWolf, 5060.0, 5000.0);
        wolf.id = 3;
        mgr.animals = vec![doe, fawn, wolf];
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        mgr.tick(0.1, &[], &att, &[], false);
        let f = mgr.animals.iter().find(|a| a.id == 2).unwrap();
        assert!(matches!(f.state, WildlifeState::Fleeing { .. }),
            "掠食者逼近時嬉戲幼獸應改逃竄，實際 {:?}", f.state);
    }

    #[test]
    fn adult_never_frolics() {
        // 嬉戲只屬於幼獸——連跑整管理器多幀，任何成體都不該進入 Frolicking。
        let mut mgr = WildlifeManager::new();
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        for _ in 0..200 {
            mgr.tick(0.2, &[], &att, &[], false);
            let adult_frolic = mgr.animals.iter().any(|a|
                !a.is_juvenile() && matches!(a.state, WildlifeState::Frolicking { .. }));
            assert!(!adult_frolic, "成體不該嬉戲");
        }
    }

    // ─── ROADMAP 216：成體相依理毛 測試 ─────────────────────────────────────────

    #[test]
    fn tick_groom_returns_to_wander_when_timer_expires() {
        // 理毛計時耗盡 → 收尾回漫遊（再起身找下一個目標），不會一直黏在原地理毛。
        let mut rng = make_rng();
        let mut w = adult_at(WildlifeKind::WildDeer, 5000.0, 5000.0);
        w.state = WildlifeState::Grooming { groom_timer: 0.05 };
        w.tick_groom(0.1, None, &mut rng); // dt > timer
        assert!(matches!(w.state, WildlifeState::Wandering { .. }),
            "理毛計時耗盡應回漫遊，實際 {:?}", w.state);
    }

    #[test]
    fn tick_groom_holds_position_while_timer_remaining() {
        // 理毛中：原地不動（不更新座標）、計時遞減、維持 Grooming。
        let mut rng = make_rng();
        let mut w = adult_at(WildlifeKind::WildDeer, 5000.0, 5000.0);
        let (x0, y0) = (w.x, w.y);
        w.state = WildlifeState::Grooming { groom_timer: 5.0 };
        w.tick_groom(0.1, None, &mut rng);
        assert!((w.x - x0).abs() < 1e-6 && (w.y - y0).abs() < 1e-6, "理毛中應原地不動");
        match w.state {
            WildlifeState::Grooming { groom_timer } => {
                assert!((groom_timer - 4.9).abs() < 1e-4, "計時應遞減 dt");
            }
            other => panic!("計時未耗盡應維持理毛，實際 {other:?}"),
        }
    }

    #[test]
    fn adults_groom_partner_in_daytime() {
        // 整管理器：白天，兩隻緊鄰的成鹿——非哨兵那隻（較大 id）連跑多幀後應有機會進入理毛。
        let mut mgr = WildlifeManager::new();
        let mut a = adult_at(WildlifeKind::WildDeer, 5000.0, 5000.0);
        a.id = 1; // 群內最小 id → 擔任哨兵（站崗、不理毛）
        a.state = WildlifeState::Resting { rest_timer: 100000.0 };
        let mut b = adult_at(WildlifeKind::WildDeer, 5012.0, 5000.0); // 距夥伴 12px < GROOM_RADIUS
        b.id = 2; // 非哨兵 → 可理毛
        b.state = WildlifeState::Resting { rest_timer: 100000.0 };
        mgr.animals = vec![a, b];
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        let mut saw_groom = false;
        for _ in 0..2000 {
            mgr.tick(0.1, &[], &att, &[], false); // is_night=false
            let w = mgr.animals.iter().find(|x| x.id == 2).unwrap();
            if matches!(w.state, WildlifeState::Grooming { .. }) { saw_groom = true; break; }
        }
        assert!(saw_groom, "白天身邊有同種夥伴的成體應會開始理毛");
    }

    #[test]
    fn lone_adult_never_grooms() {
        // 身邊沒有同種成體夥伴的孤獸——連跑多幀都不該理毛（理毛是「相依」行為）。
        let mut mgr = WildlifeManager::new();
        let mut lone = adult_at(WildlifeKind::WildDeer, 5000.0, 5000.0);
        lone.id = 1;
        mgr.animals = vec![lone];
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        for _ in 0..1000 {
            mgr.tick(0.1, &[], &att, &[], false);
            let w = mgr.animals.iter().find(|x| x.id == 1).unwrap();
            assert!(!matches!(w.state, WildlifeState::Grooming { .. }), "孤獸不該理毛");
        }
    }

    #[test]
    fn adults_do_not_groom_at_night() {
        // 夜間：成體歸巢沉睡，不理毛——連跑多幀都不該進入 Grooming。
        let mut mgr = WildlifeManager::new();
        let mut a = adult_at(WildlifeKind::WildDeer, 5000.0, 5000.0);
        a.id = 1;
        let mut b = adult_at(WildlifeKind::WildDeer, 5012.0, 5000.0);
        b.id = 2;
        mgr.animals = vec![a, b];
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        for _ in 0..2000 {
            mgr.tick(0.1, &[], &att, &[], true); // is_night=true
            let groom = mgr.animals.iter().any(|x| matches!(x.state, WildlifeState::Grooming { .. }));
            assert!(!groom, "夜間成體不該理毛");
        }
    }

    #[test]
    fn grooming_adult_flees_when_predator_approaches() {
        // 威脅優先：正在理毛的成體，掠食者逼近時應改逃竄（不會繼續理毛）。
        let mut mgr = WildlifeManager::new();
        let mut deer = adult_at(WildlifeKind::WildDeer, 5000.0, 5000.0);
        deer.id = 1;
        deer.state = WildlifeState::Grooming { groom_timer: 5.0 };
        // 狼貼近成鹿（FLEE_RADIUS 內），形成直接威脅。
        let mut wolf = adult_at(WildlifeKind::WildWolf, 5060.0, 5000.0);
        wolf.id = 2;
        mgr.animals = vec![deer, wolf];
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        mgr.tick(0.1, &[], &att, &[], false);
        let d = mgr.animals.iter().find(|x| x.id == 1).unwrap();
        assert!(matches!(d.state, WildlifeState::Fleeing { .. }),
            "掠食者逼近時理毛成體應改逃竄，實際 {:?}", d.state);
    }

    #[test]
    fn juvenile_never_grooms() {
        // 理毛只屬於成體——身邊放滿同種成體的幼獸，連跑多幀都不該進入 Grooming（牠走的是嬉戲/依偎）。
        let mut mgr = WildlifeManager::new();
        let mut doe = adult_at(WildlifeKind::WildDeer, 5000.0, 5000.0);
        doe.id = 1;
        doe.state = WildlifeState::Resting { rest_timer: 100000.0 };
        let mut fawn = juvenile_at(WildlifeKind::WildDeer, 5012.0, 5000.0);
        fawn.id = 2;
        mgr.animals = vec![doe, fawn];
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        for _ in 0..2000 {
            // 把受測對象釘在幼獸（成熟度 < 1）：本測試要驗的是「身為幼獸時不理毛」，
            // 而幼獸每幀會成長（dt/MATURE_DURATION_SECS），2000 幀(200s)遠超成熟期(120s)會長成成體；
            // 不釘住的話受測對象中途轉成體、理毛就成了正當行為（216），測不到原本要驗的不變量。
            if let Some(f) = mgr.animals.iter_mut().find(|x| x.id == 2) {
                f.maturity = 0.0;
            }
            mgr.tick(0.1, &[], &att, &[], false);
            let f = mgr.animals.iter().find(|x| x.id == 2).unwrap();
            assert!(!matches!(f.state, WildlifeState::Grooming { .. }), "幼獸不該理毛");
        }
    }

    // ─── ROADMAP 217：掠食者夜嚎 ────────────────────────────────────────────

    #[test]
    fn tick_howl_returns_to_wander_when_timer_expires() {
        // 長嚎計時耗盡後，掠食者應回到漫遊（巡遊）。
        let mut wolf = adult_at(WildlifeKind::WildWolf, 5000.0, 5000.0);
        wolf.state = WildlifeState::Howling { howl_timer: 0.05 };
        let mut rng = make_rng();
        wolf.tick_howl(0.1, &mut rng); // dt > 剩餘 → 到期
        assert!(matches!(wolf.state, WildlifeState::Wandering { .. }),
            "長嚎到期應回到漫遊，實際 {:?}", wolf.state);
    }

    #[test]
    fn tick_howl_holds_position_while_timer_remaining() {
        // 長嚎進行中：原地不動（座標不變）、計時遞減、仍維持 Howling。
        let mut wolf = adult_at(WildlifeKind::WildWolf, 5000.0, 5000.0);
        wolf.state = WildlifeState::Howling { howl_timer: 3.0 };
        let (x0, y0) = (wolf.x, wolf.y);
        let mut rng = make_rng();
        wolf.tick_howl(0.1, &mut rng);
        assert_eq!((wolf.x, wolf.y), (x0, y0), "長嚎中掠食者應原地不動");
        match wolf.state {
            WildlifeState::Howling { howl_timer } => assert!((howl_timer - 2.9).abs() < 1e-4, "計時應遞減 dt"),
            _ => panic!("長嚎未到期應維持 Howling，實際 {:?}", wolf.state),
        }
    }

    #[test]
    fn predator_howls_at_night_when_no_prey() {
        // 整管理器：夜間，附近無獵物可追的掠食者（歇息中）連跑多幀後應有機會仰首長嚎。
        let mut mgr = WildlifeManager::new();
        let mut wolf = adult_at(WildlifeKind::WildWolf, 5000.0, 5000.0);
        wolf.id = 1;
        wolf.state = WildlifeState::Resting { rest_timer: 100000.0 };
        mgr.animals = vec![wolf]; // 場上只有狼、沒有任何獵物
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        let mut saw_howl = false;
        for _ in 0..2000 {
            mgr.tick(0.1, &[], &att, &[], true); // is_night=true
            let w = mgr.animals.iter().find(|x| x.id == 1).unwrap();
            if matches!(w.state, WildlifeState::Howling { .. }) { saw_howl = true; break; }
        }
        assert!(saw_howl, "夜間無獵可追的掠食者應會仰首長嚎");
    }

    #[test]
    fn predator_does_not_howl_in_daytime() {
        // 白天：長嚎是夜間氛圍行為——連跑多幀都不該進入 Howling。
        let mut mgr = WildlifeManager::new();
        let mut wolf = adult_at(WildlifeKind::WildWolf, 5000.0, 5000.0);
        wolf.id = 1;
        wolf.state = WildlifeState::Resting { rest_timer: 100000.0 };
        mgr.animals = vec![wolf];
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        for _ in 0..2000 {
            mgr.tick(0.1, &[], &att, &[], false); // is_night=false
            let w = mgr.animals.iter().find(|x| x.id == 1).unwrap();
            assert!(!matches!(w.state, WildlifeState::Howling { .. }), "白天掠食者不該長嚎");
        }
    }

    #[test]
    fn prey_never_howl() {
        // 長嚎只屬於掠食者——夜間的獵物（歸巢沉睡）連跑多幀都不該進入 Howling。
        let mut mgr = WildlifeManager::new();
        let mut deer = adult_at(WildlifeKind::WildDeer, 5000.0, 5000.0);
        deer.id = 1;
        mgr.animals = vec![deer];
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        for _ in 0..2000 {
            mgr.tick(0.1, &[], &att, &[], true); // is_night=true
            let w = mgr.animals.iter().find(|x| x.id == 1).unwrap();
            assert!(!matches!(w.state, WildlifeState::Howling { .. }), "獵物不該長嚎");
        }
    }

    #[test]
    fn howling_predator_hunts_when_prey_appears() {
        // 狩獵優先：正在長嚎的掠食者，附近出現可獵物種時應改去獵殺（潛行或全速追獵），不再長嚎。
        let mut mgr = WildlifeManager::new();
        let mut wolf = adult_at(WildlifeKind::WildWolf, 5000.0, 5000.0);
        wolf.id = 1;
        wolf.state = WildlifeState::Howling { howl_timer: 3.0 };
        let mut deer = adult_at(WildlifeKind::WildDeer, 5060.0, 5000.0); // 夜獵半徑內
        deer.id = 2;
        mgr.animals = vec![wolf, deer];
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        mgr.tick(0.1, &[], &att, &[], true); // is_night=true
        let w = mgr.animals.iter().find(|x| x.id == 1).unwrap();
        assert!(
            matches!(w.state, WildlifeState::Hunting { .. } | WildlifeState::Stalking { .. }),
            "獵物出現時長嚎掠食者應改去獵殺，實際 {:?}", w.state,
        );
    }

    // ─── ROADMAP 218：群嚎呼應 ───────────────────────────────────────────────
    #[test]
    fn hears_howl_only_within_radius() {
        // 純距離判定：嚎聲在 HOWL_HEAR_RADIUS 內聽得到、外則聽不到；空快照永遠聽不到。
        let me = (5000.0_f32, 5000.0_f32);
        assert!(!hears_howl(me.0, me.1, &[]), "四下無嚎聲時聽不到");
        let near = (me.0 + HOWL_HEAR_RADIUS - 1.0, me.1);
        assert!(hears_howl(me.0, me.1, &[near]), "半徑內的嚎聲應聽得到");
        let far = (me.0 + HOWL_HEAR_RADIUS + 50.0, me.1);
        assert!(!hears_howl(me.0, me.1, &[far]), "半徑外的嚎聲聽不到");
        // 多個來源：只要有一聲在範圍內即聽得到。
        assert!(hears_howl(me.0, me.1, &[far, near]), "其中一聲在範圍內就算聽得到");
    }

    #[test]
    fn resting_predator_joins_nearby_howl() {
        // 群嚎呼應：一隻持續長嚎的狼旁，夜裡歇息、無獵可追的同類會被牽動接嚎。
        let mut mgr = WildlifeManager::new();
        let mut howler = adult_at(WildlifeKind::WildWolf, 5000.0, 5000.0);
        howler.id = 1;
        // 給極長的計時，讓牠整段測試都維持長嚎（持續發聲源）。
        howler.state = WildlifeState::Howling { howl_timer: 1.0e9 };
        let mut listener = adult_at(WildlifeKind::WildFox, 5100.0, 5000.0); // 在 HOWL_HEAR_RADIUS 內
        listener.id = 2;
        listener.state = WildlifeState::Resting { rest_timer: 1.0e9 };
        mgr.animals = vec![howler, listener];
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        // 無獵物在場（只有兩隻掠食者）→ 走閒晃分支。HOWL_JOIN_PROB=0.5，數十幀內幾乎必然接嚎。
        let mut listener_howled = false;
        for _ in 0..60 {
            mgr.tick(0.1, &[], &att, &[], true); // is_night=true
            let l = mgr.animals.iter().find(|x| x.id == 2).unwrap();
            if matches!(l.state, WildlifeState::Howling { .. }) {
                listener_howled = true;
                break;
            }
        }
        assert!(listener_howled, "夜裡歇息的同類聽見持續的嚎聲後應被牽動接嚎");
    }

    #[test]
    fn lone_resting_predator_rarely_howls_without_neighbor() {
        // 對照：四下無嚎聲時，群嚎呼應不觸發——歇息的狼僅靠 217 的低機率 HOWL_PROB 自發起頭，
        // 故單幀內絕大多數時候仍維持歇息（驗證接嚎確實是「被附近嚎聲牽動」而非無條件嚎）。
        let mut mgr = WildlifeManager::new();
        let mut wolf = adult_at(WildlifeKind::WildWolf, 5000.0, 5000.0);
        wolf.id = 1;
        wolf.state = WildlifeState::Resting { rest_timer: 1.0e9 };
        mgr.animals = vec![wolf];
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        mgr.tick(0.1, &[], &att, &[], true);
        let w = mgr.animals.iter().find(|x| x.id == 1).unwrap();
        // 單幀自發起嚎機率僅 HOWL_PROB(0.02)，這裡不斷言絕不嚎，只驗證沒有「附近嚎聲」這個牽動源時
        // 狀態仍是歇息或（極低機率）自發長嚎——不會是別的（確認沒有意外副作用）。
        assert!(
            matches!(w.state, WildlifeState::Resting { .. } | WildlifeState::Howling { .. }),
            "無鄰近嚎聲時，歇息的掠食者應維持歇息（或極低機率自發長嚎），實際 {:?}", w.state,
        );
    }

    // ─── ROADMAP 220：鳥群振翅升空盤旋 ───────────────────────────────────────

    #[test]
    fn tick_fly_circles_around_anchor_while_timer_remaining() {
        // 盤旋進行中：繞著群心轉圈——計時遞減、盤旋角依角速度推進、座標落在以群心為圓心、
        // 半徑 FLIGHT_CIRCLE_RADIUS 的圓周上，狀態維持 Flying。
        let mut rng = make_rng();
        let mut bird = adult_at(WildlifeKind::WildBird, 5000.0, 5000.0);
        let anchor = Some((5000.0_f32, 5000.0_f32));
        bird.state = WildlifeState::Flying { fly_timer: 5.0, angle: 0.0 };
        bird.tick_fly(0.1, anchor, &mut rng);
        match bird.state {
            WildlifeState::Flying { fly_timer, angle } => {
                assert!((fly_timer - 4.9).abs() < 1e-4, "計時應遞減 dt");
                assert!((angle - FLIGHT_ANGULAR_SPEED * 0.1).abs() < 1e-4, "盤旋角應依角速度推進");
            }
            _ => panic!("盤旋未到期應維持 Flying，實際 {:?}", bird.state),
        }
        let r = ((bird.x - 5000.0).powi(2) + (bird.y - 5000.0).powi(2)).sqrt();
        assert!((r - FLIGHT_CIRCLE_RADIUS).abs() < 1e-3, "盤旋座標應落在群心圓周上，實際半徑 {r}");
    }

    #[test]
    fn tick_fly_lands_to_wander_when_timer_expires() {
        // 盤旋到期：降落回漫遊（起身投入閒晃）。
        let mut rng = make_rng();
        let mut bird = adult_at(WildlifeKind::WildBird, 5000.0, 5000.0);
        bird.state = WildlifeState::Flying { fly_timer: 0.05, angle: 1.0 };
        bird.tick_fly(0.1, None, &mut rng); // dt > 剩餘 → 到期
        assert!(matches!(bird.state, WildlifeState::Wandering { .. }), "盤旋到期應降落回漫遊，實際 {:?}", bird.state);
    }

    #[test]
    fn flying_state_str_is_flying() {
        let mut bird = adult_at(WildlifeKind::WildBird, 0.0, 0.0);
        bird.state = WildlifeState::Flying { fly_timer: 1.0, angle: 0.0 };
        assert_eq!(bird.state_str(), "flying");
    }

    #[test]
    fn sees_flight_only_within_radius() {
        // 純距離判定：升空中的同類在 FLIGHT_HEAR_RADIUS 內看得見、外則看不見；空快照永遠看不見。
        let me = (5000.0_f32, 5000.0_f32);
        assert!(!sees_flight(me.0, me.1, &[]), "四下無同類升空時看不見");
        let near = (me.0 + FLIGHT_HEAR_RADIUS - 1.0, me.1);
        assert!(sees_flight(me.0, me.1, &[near]), "半徑內升空的同類應看得見");
        let far = (me.0 + FLIGHT_HEAR_RADIUS + 50.0, me.1);
        assert!(!sees_flight(me.0, me.1, &[far]), "半徑外升空的同類看不見");
        assert!(sees_flight(me.0, me.1, &[far, near]), "其中一隻在範圍內就算看得見");
    }

    #[test]
    fn calm_bird_joins_nearby_flight() {
        // 接力升空：一隻持續盤旋的野鳥旁，白天平靜、未升空的同類會被牽動跟著飛起。
        let mut mgr = WildlifeManager::new();
        let mut flyer = adult_at(WildlifeKind::WildBird, 5000.0, 5000.0);
        flyer.id = 1;
        flyer.state = WildlifeState::Flying { fly_timer: 1.0e9, angle: 0.0 }; // 整段測試持續盤旋
        // 300px：大於群聚半徑（280，故兩鳥不成群、盤旋者繞自家不貼向對方）、大於哨兵群半徑（220，
        // 故不被收編成哨兵而脫離盤旋）、小於升空牽動半徑（320，看得見彼此升空）。
        let mut listener = adult_at(WildlifeKind::WildBird, 5300.0, 5000.0);
        listener.id = 2;
        // 一般白天漫遊（目標設在自身位置、計時極長 → 幾乎原地，且不觸發夜眠喚醒邏輯）。
        listener.state = WildlifeState::Wandering { target_x: 5300.0, target_y: 5000.0, wander_timer: 1.0e9 };
        mgr.animals = vec![flyer, listener];
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        let mut joined = false;
        for _ in 0..80 {
            mgr.tick(0.1, &[], &att, &[], false); // is_night=false（白天）
            let l = mgr.animals.iter().find(|x| x.id == 2).unwrap();
            if matches!(l.state, WildlifeState::Flying { .. }) {
                joined = true;
                break;
            }
        }
        assert!(joined, "白天平靜時看見附近同類升空的野鳥，應被牽動跟著飛起");
    }

    #[test]
    fn non_bird_never_flies() {
        // 物種專屬：只有野鳥會飛——白天平靜的野鹿連跑數百幀都不該進入 Flying。
        let mut mgr = WildlifeManager::new();
        let mut deer = adult_at(WildlifeKind::WildDeer, 6000.0, 6000.0);
        deer.id = 1;
        deer.state = WildlifeState::Resting { rest_timer: 1.0e9 };
        mgr.animals = vec![deer];
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        for _ in 0..300 {
            mgr.tick(0.1, &[], &att, &[], false);
            let d = mgr.animals.iter().find(|x| x.id == 1).unwrap();
            assert!(!matches!(d.state, WildlifeState::Flying { .. }), "非鳥類不該升空，實際 {:?}", d.state);
        }
    }

    #[test]
    fn flying_bird_flees_when_threat_approaches() {
        // 威脅永遠優先：盤旋中的野鳥一旦有威脅逼近 FLEE_RADIUS 內，立刻降下逃竄（非繼續盤旋）。
        let mut mgr = WildlifeManager::new();
        let mut bird = adult_at(WildlifeKind::WildBird, 5000.0, 5000.0);
        bird.id = 1;
        bird.state = WildlifeState::Flying { fly_timer: 1.0e9, angle: 0.0 };
        mgr.animals = vec![bird];
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        // 玩家逼到 50px（< FLEE_RADIUS 180）；物種預設態度 50 < FRIENDLY_ATTITUDE 65 且未馴養 → 算威脅。
        mgr.tick(0.1, &[(5050.0, 5000.0)], &att, &[], false);
        let b = mgr.animals.iter().find(|x| x.id == 1).unwrap();
        assert!(matches!(b.state, WildlifeState::Fleeing { .. }), "盤旋中遇威脅應立刻降下逃竄，實際 {:?}", b.state);
    }

    // ── ROADMAP 221：晝日鳥鳴呼應 ────────────────────────────────────────────

    #[test]
    fn tick_chirp_holds_position_while_timer_remaining() {
        // 啁啾進行中：原地不動（座標不變）、計時遞減、狀態維持 Chirping。
        let mut rng = make_rng();
        let mut bird = adult_at(WildlifeKind::WildBird, 5000.0, 5000.0);
        bird.state = WildlifeState::Chirping { chirp_timer: 2.0 };
        bird.tick_chirp(0.1, None, &mut rng);
        match bird.state {
            WildlifeState::Chirping { chirp_timer } => {
                assert!((chirp_timer - 1.9).abs() < 1e-4, "計時應遞減 dt");
            }
            _ => panic!("啁啾未到期應維持 Chirping，實際 {:?}", bird.state),
        }
        assert!((bird.x - 5000.0).abs() < 1e-6 && (bird.y - 5000.0).abs() < 1e-6, "啁啾中應原地不動");
    }

    #[test]
    fn tick_chirp_returns_to_wander_when_timer_expires() {
        // 啁啾到期：回到漫遊（起身投入閒晃）。
        let mut rng = make_rng();
        let mut bird = adult_at(WildlifeKind::WildBird, 5000.0, 5000.0);
        bird.state = WildlifeState::Chirping { chirp_timer: 0.05 };
        bird.tick_chirp(0.1, None, &mut rng); // dt > 剩餘 → 到期
        assert!(matches!(bird.state, WildlifeState::Wandering { .. }), "啁啾到期應回漫遊，實際 {:?}", bird.state);
    }

    #[test]
    fn chirping_state_str_is_chirping() {
        let mut bird = adult_at(WildlifeKind::WildBird, 0.0, 0.0);
        bird.state = WildlifeState::Chirping { chirp_timer: 1.0 };
        assert_eq!(bird.state_str(), "chirping");
    }

    #[test]
    fn hears_song_only_within_radius() {
        // 純距離判定：啁啾中的同類在 CHIRP_HEAR_RADIUS 內聽得見、外則聽不見；空快照永遠聽不見。
        let me = (5000.0_f32, 5000.0_f32);
        assert!(!hears_song(me.0, me.1, &[]), "四下無同類啁啾時聽不見");
        let near = (me.0 + CHIRP_HEAR_RADIUS - 1.0, me.1);
        assert!(hears_song(me.0, me.1, &[near]), "半徑內啁啾的同類應聽得見");
        let far = (me.0 + CHIRP_HEAR_RADIUS + 50.0, me.1);
        assert!(!hears_song(me.0, me.1, &[far]), "半徑外啁啾的同類聽不見");
        assert!(hears_song(me.0, me.1, &[far, near]), "其中一隻在範圍內就算聽得見");
    }

    #[test]
    fn calm_bird_joins_nearby_song() {
        // 接力起鳴：一隻持續啁啾的野鳥旁，白天平靜、未鳴的同類會被牽動跟著啁啾。
        let mut mgr = WildlifeManager::new();
        let mut singer = adult_at(WildlifeKind::WildBird, 5000.0, 5000.0);
        singer.id = 1;
        singer.state = WildlifeState::Chirping { chirp_timer: 1.0e9 }; // 整段測試持續啁啾
        // 300px：大於群聚半徑（280，故兩鳥不成群）、大於哨兵群半徑（220）、小於鳴聲牽動半徑（420，
        // 聽得見彼此啁啾）、大於升空牽動半徑（320，故聽者不會被升空牽動而改去飛、確保是「跟鳴」）。
        let mut listener = adult_at(WildlifeKind::WildBird, 5300.0, 5000.0);
        listener.id = 2;
        // 一般白天漫遊（目標設在自身位置、計時極長 → 幾乎原地，且不觸發夜眠喚醒邏輯）。
        listener.state = WildlifeState::Wandering { target_x: 5300.0, target_y: 5000.0, wander_timer: 1.0e9 };
        mgr.animals = vec![singer, listener];
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        let mut joined = false;
        for _ in 0..80 {
            mgr.tick(0.1, &[], &att, &[], false); // is_night=false（白天）
            let l = mgr.animals.iter().find(|x| x.id == 2).unwrap();
            if matches!(l.state, WildlifeState::Chirping { .. }) {
                joined = true;
                break;
            }
        }
        assert!(joined, "白天平靜時聽見附近同類啁啾的野鳥，應被牽動跟著起鳴");
    }

    #[test]
    fn non_bird_never_chirps() {
        // 物種專屬：只有野鳥會啁啾——白天平靜的野鹿連跑數百幀都不該進入 Chirping。
        let mut mgr = WildlifeManager::new();
        let mut deer = adult_at(WildlifeKind::WildDeer, 6000.0, 6000.0);
        deer.id = 1;
        deer.state = WildlifeState::Resting { rest_timer: 1.0e9 };
        mgr.animals = vec![deer];
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        for _ in 0..300 {
            mgr.tick(0.1, &[], &att, &[], false);
            let d = mgr.animals.iter().find(|x| x.id == 1).unwrap();
            assert!(!matches!(d.state, WildlifeState::Chirping { .. }), "非鳥類不該啁啾，實際 {:?}", d.state);
        }
    }

    #[test]
    fn bird_does_not_chirp_at_night() {
        // 夜間：野鳥歸巢沉睡，不啁啾——連跑多幀都不該進入 Chirping。
        let mut mgr = WildlifeManager::new();
        let mut bird = adult_at(WildlifeKind::WildBird, 6000.0, 6000.0);
        bird.id = 1;
        mgr.animals = vec![bird];
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        for _ in 0..500 {
            mgr.tick(0.1, &[], &att, &[], true); // is_night=true
            let b = mgr.animals.iter().find(|x| x.id == 1).unwrap();
            assert!(!matches!(b.state, WildlifeState::Chirping { .. }), "夜間野鳥不該啁啾，實際 {:?}", b.state);
        }
    }

    #[test]
    fn chirping_bird_flees_when_threat_approaches() {
        // 威脅永遠優先：啁啾中的野鳥一旦有威脅逼近 FLEE_RADIUS 內，立刻收聲逃竄（非繼續鳴唱）。
        let mut mgr = WildlifeManager::new();
        let mut bird = adult_at(WildlifeKind::WildBird, 5000.0, 5000.0);
        bird.id = 1;
        bird.state = WildlifeState::Chirping { chirp_timer: 1.0e9 };
        mgr.animals = vec![bird];
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        // 玩家逼到 50px（< FLEE_RADIUS 180）；物種預設態度 50 < FRIENDLY_ATTITUDE 65 且未馴養 → 算威脅。
        mgr.tick(0.1, &[(5050.0, 5000.0)], &att, &[], false);
        let b = mgr.animals.iter().find(|x| x.id == 1).unwrap();
        assert!(matches!(b.state, WildlifeState::Fleeing { .. }), "啁啾中遇威脅應立刻收聲逃竄，實際 {:?}", b.state);
    }

    // ─── ROADMAP 222：小動物捧食啃咬 ──────────────────────────────────────────
    #[test]
    fn tick_nibble_holds_position_while_timer_remaining() {
        // 啃咬進行中：原地不動（座標不變）、計時遞減、狀態維持 Nibbling。
        let mut rng = make_rng();
        let mut critter = adult_at(WildlifeKind::SmallCritter, 5000.0, 5000.0);
        critter.state = WildlifeState::Nibbling { nibble_timer: 2.0 };
        critter.tick_nibble(0.1, None, &mut rng);
        match critter.state {
            WildlifeState::Nibbling { nibble_timer } => {
                assert!((nibble_timer - 1.9).abs() < 1e-4, "計時應遞減 dt");
            }
            _ => panic!("啃咬未到期應維持 Nibbling，實際 {:?}", critter.state),
        }
        assert!((critter.x - 5000.0).abs() < 1e-6 && (critter.y - 5000.0).abs() < 1e-6, "啃咬中應原地不動");
    }

    #[test]
    fn tick_nibble_returns_to_wander_when_timer_expires() {
        // 啃咬到期：回到漫遊（起身投入閒晃）。
        let mut rng = make_rng();
        let mut critter = adult_at(WildlifeKind::SmallCritter, 5000.0, 5000.0);
        critter.state = WildlifeState::Nibbling { nibble_timer: 0.05 };
        critter.tick_nibble(0.1, None, &mut rng); // dt > 剩餘 → 到期
        assert!(matches!(critter.state, WildlifeState::Wandering { .. }), "啃咬到期應回漫遊，實際 {:?}", critter.state);
    }

    #[test]
    fn nibbling_state_str_is_nibbling() {
        let mut critter = adult_at(WildlifeKind::SmallCritter, 0.0, 0.0);
        critter.state = WildlifeState::Nibbling { nibble_timer: 1.0 };
        assert_eq!(critter.state_str(), "nibbling");
    }

    #[test]
    fn non_critter_never_nibbles() {
        // 物種專屬：只有小動物會啃咬——白天平靜的野鹿連跑數百幀都不該進入 Nibbling。
        let mut mgr = WildlifeManager::new();
        let mut deer = adult_at(WildlifeKind::WildDeer, 6000.0, 6000.0);
        deer.id = 1;
        deer.state = WildlifeState::Resting { rest_timer: 1.0e9 };
        mgr.animals = vec![deer];
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        for _ in 0..300 {
            mgr.tick(0.1, &[], &att, &[], false);
            let d = mgr.animals.iter().find(|x| x.id == 1).unwrap();
            assert!(!matches!(d.state, WildlifeState::Nibbling { .. }), "非小動物不該啃咬，實際 {:?}", d.state);
        }
    }

    #[test]
    fn critter_does_not_nibble_at_night() {
        // 夜間：小動物歸巢沉睡，不啃咬——連跑多幀都不該進入 Nibbling。
        let mut mgr = WildlifeManager::new();
        let mut critter = adult_at(WildlifeKind::SmallCritter, 6000.0, 6000.0);
        critter.id = 1;
        mgr.animals = vec![critter];
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        for _ in 0..500 {
            mgr.tick(0.1, &[], &att, &[], true); // is_night=true
            let c = mgr.animals.iter().find(|x| x.id == 1).unwrap();
            assert!(!matches!(c.state, WildlifeState::Nibbling { .. }), "夜間小動物不該啃咬，實際 {:?}", c.state);
        }
    }

    #[test]
    fn calm_critter_eventually_nibbles_during_day() {
        // 白天平靜：一隻孤身小動物連跑多幀後，總會偶爾坐起來捧食啃咬（NIBBLE_PROB 之必然累積）。
        let mut mgr = WildlifeManager::new();
        let mut critter = adult_at(WildlifeKind::SmallCritter, 6000.0, 6000.0);
        critter.id = 1;
        critter.state = WildlifeState::Resting { rest_timer: 0.1 };
        mgr.animals = vec![critter];
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        let mut nibbled = false;
        for _ in 0..1000 {
            mgr.tick(0.1, &[], &att, &[], false); // 白天、無威脅
            let c = mgr.animals.iter().find(|x| x.id == 1).unwrap();
            if matches!(c.state, WildlifeState::Nibbling { .. }) {
                nibbled = true;
                break;
            }
        }
        assert!(nibbled, "白天平靜的小動物應偶爾坐起來啃咬");
    }

    #[test]
    fn nibbling_critter_flees_when_threat_approaches() {
        // 威脅永遠優先：啃咬中的小動物一旦有威脅逼近 FLEE_RADIUS 內，立刻丟食逃竄（非繼續啃）。
        let mut mgr = WildlifeManager::new();
        let mut critter = adult_at(WildlifeKind::SmallCritter, 5000.0, 5000.0);
        critter.id = 1;
        critter.state = WildlifeState::Nibbling { nibble_timer: 1.0e9 };
        mgr.animals = vec![critter];
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        // 玩家逼到 50px（< FLEE_RADIUS 180）；物種預設態度 < FRIENDLY 且未馴養 → 算威脅。
        mgr.tick(0.1, &[(5050.0, 5000.0)], &att, &[], false);
        let c = mgr.animals.iter().find(|x| x.id == 1).unwrap();
        assert!(matches!(c.state, WildlifeState::Fleeing { .. }), "啃咬中遇威脅應立刻丟食逃竄，實際 {:?}", c.state);
    }
}
