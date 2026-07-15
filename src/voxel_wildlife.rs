//! 乙太方界·野兔 v1——世界第一種環境生物（自主提案切片，ROADMAP 847）。
//!
//! **真缺口**：乙太方界至今只有 4 位具名 AI 居民會在世界裡走動，草地與森林從未有過
//! 任何一絲野生的動態——世界看得出「有人住」，卻看不出「有生機」。本模組補上世界
//! 第一種環境生物：幾隻在村莊周圍悠閒遊蕩的野兔，見到玩家靠近就受驚跳開。
//!
//! **刻意的範圍收斂**：純點綴、無 AI 大腦（零 LLM）、無戰鬥、無記憶、無持久化
//! （純記憶體、重啟於固定家域點重新生成，比照既有 `drops`/`stalls` 世界暫態慣例）。
//! 這不是居民↔居民/居民↔玩家關係軸線的第 N 刀，是全新的「世界環境」軸線第一刀。
//!
//! **純邏輯層**：家域遊蕩沿用既有 [`crate::voxel_residents`] 的
//! `wander_target`/`wander_center`/`step_toward`/`gravity_step`/`dry_ground_spawn`，
//! 本模組只補野兔專屬的「受驚偵測」與「逃跑方向」兩個確定性純函式，
//! 零 LLM、零鎖、零 IO，鎖/連線/tick 驅動全在 `voxel_ws.rs`。
//!
//! **餵野兔馴服 v1（自主提案切片）**：847/848 讓世界第一次看得出有生機，但那份生機
//! 至今只能遠遠看——玩家從沒有一條路能真正「碰」到牠。本刀補上世界環境軸線與玩家互動
//! 軸線第一次的交會：手持胡蘿蔔靠近一隻野兔並餵食，牠就此**永遠不再怕你**。因為
//! [`FLEE_RADIUS`] 大於 [`TAME_REACH`]，這一刀是刻意的——要餵到牠，得先追上一隻正在
//! 受驚逃跑的兔子，第一次成功的餵食因此帶著「追上牠」的小小成就感。**850 v1 說明裡
//! 明講「刻意只做『不再逃跑』，不做跟隨/寵物/繁殖」——跟隨正是本刀要補的那一半。**
//!
//! **馴服兔子跟隨你 v1（自主提案切片，ROADMAP 851）**：馴服至今只讓兔子「原地不怕你」，
//! 牠依舊只在自己的家域打轉，追上牠的那份成就感沒有下文——馴服一隻兔子和沒馴服看起來
//! 幾乎沒兩樣（除了牠不逃）。本刀讓馴服真正產生看得見的羈絆：**已馴服的兔子只要你靠近，
//! 就會像隻小跟班一樣跟上你走**（[`FOLLOW_RADIUS`] 內起跟、[`FOLLOW_LOSE_RADIUS`] 外才
//! 走失遲滯 hysteresis，同 [`should_flee`] 手法），跟到 [`FOLLOW_STOP_DIST`] 就停下不再
//! 往你身上擠；你若越走越遠，牠會安心跟丟、回到原本的閒晃。**v1 刻意收斂**：不分玩家
//! 身份（任何靠近的玩家都能被跟）、不繁殖、不能召回/放開、無寵物 UI——第一次讓「馴服」
//! 這件事在世界裡真的看得出差異，就是最小、最有感的一步。
//!
//! **馴服兔子生寶寶 v1（自主提案切片，ROADMAP 855）**：850/851 明講 v1 刻意不做「繁殖」——
//! 但那正是「世界環境」軸線唯一還空著的一格：野兔/游魚至今是固定數量的點綴生物，
//! 世界本身從沒有「自己長大」過。本刀補上：兩隻已馴服的兔子只要湊得夠近
//! （[`BREED_RADIUS`] 內），隔一段夠久的節流時間（[`BREED_INTERVAL_SECS`]）就有機率
//! （[`BREED_CHANCE`]）誕生一隻小兔子——牠一出生就是**已馴服**的（跟父母一樣認得你、
//! 立刻跟著走），世界第一次因為「你馴服了牠們」而自己長出新的生命。**刻意收斂**：
//! 全域節流（不分哪一對，同一時間全世界至多生一隻）、population 天花板
//! （[`MAX_RABBITS`]）防止無限增長、寶寶落在雙親中點附近最近的乾地、純記憶體
//! （重啟歸零，比照 wildlife 系統既有慣例）——不做基因/外觀差異，最小、最有感的一步。
//!
//! **幼獸長大 v1（自主提案切片）**：855 讓小兔子第一次誕生，但誕生之後呢？盤點下來，
//! 小兔子從出生那一刻起就與成兔**毫無二致**——同樣的體型、同樣「一出生就已馴服」，甚至
//! **同一 tick 就能被 [`find_breeding_pair`] 選為下一輪的親代**，世界裡因此可能出現「剛出生
//! 的寶寶下一秒就當了爸媽」的怪異時序（與居民成年禮 942 補上之前「還沒長大就當了父母」
//! 是同一種缺口，只是這次發生在野生動物身上）。本刀補上「長大」本身：小兔子要活過
//! [`GROWTH_SECS`] 才算長大成兔——**長大前**：體型明顯偏小（[`growth_scale`] 隨時間漸漸長到
//! 1.0，前端直接讀取伺服器算好的縮放套用，零額外前端數學）、**不會被選為繁殖親代**
//! （行為後果，不只是視覺裝飾）；**長大那一刻**：世界動態牆播報一句「小兔子長大了」，
//! 一生僅此一次（`grown_announced` 純記憶體旗標，比照 `tamed`/`following` 同款 wildlife
//! 暫態慣例，重啟歸零、不持久化）。世界初始生成的兔子與寶寶誕生前的既有兔群一律視為
//! **已成年**（`born_unix == 0`），不受影響、零回歸。

/// 野兔閒晃速度（方塊/秒）——比居民散步（2.6）更悠閒，符合小動物碎步的觀感。
pub const WANDER_SPEED: f32 = 1.4;
/// 受驚逃跑速度——明顯比閒晃快，一眼看得出「嚇到了」。
pub const FLEE_SPEED: f32 = 4.2;
/// 玩家進入這個距離內，野兔就會受驚逃跑（方塊）。
pub const FLEE_RADIUS: f32 = 4.0;
/// 已受驚時，玩家要遠到超過這個距離才安心恢復閒晃（比 [`FLEE_RADIUS`] 稍大，
/// 這道遲滯（hysteresis）避免野兔在臨界距離上受驚/平靜來回抖動）。
pub const CALM_RADIUS: f32 = 6.0;
/// 逃跑目標離當下位置的距離（方塊）。
pub const FLEE_DIST: f32 = 6.0;
/// 野兔閒晃半徑下限（方塊）——比居民 `HOME_RADIUS`（20）小得多，野兔活動範圍更侷限。
pub const WANDER_MIN_R: f32 = 1.5;
/// 野兔閒晃半徑上限（方塊）。
pub const WANDER_MAX_R: f32 = 6.0;

/// 餵食馴服的觸及範圍（方塊）——刻意小於 [`FLEE_RADIUS`]：要餵到牠就得先追上正在
/// 受驚逃跑的兔子，第一次成功馴服因此帶著「追上牠」的小小成就感。
pub const TAME_REACH: f32 = 3.0;

/// 判斷這次餵食是否能成功馴服：距離要夠近、且這隻兔子還沒被馴服過（馴服是一次性、
/// 永久的——重複餵已馴服的兔子不會有任何效果，避免玩家對著同一隻兔子洗馴服訊息）。
pub fn should_tame(already_tamed: bool, player_dist_sq: f32) -> bool {
    !already_tamed && player_dist_sq < TAME_REACH * TAME_REACH
}

/// 馴服成功那一刻的回饋句（確定性輪替，`pick` 由呼叫端提供隨機源）。
const TAME_LINES: [&str; 4] = [
    "🥕 牠湊近你的手心，安心地嚼了起來——牠好像不再那麼怕你了。",
    "🥕 牠豎起耳朵愣了一下，接著才小口小口啃了起來，眼神放鬆了不少。",
    "🥕 牠終於停下逃跑的腳步，就地啃起你遞出的胡蘿蔔。",
    "🥕 牠蹭了蹭你的手，往後看見你也不會再拔腿就跑了。",
];

/// 依 `pick` 取一句馴服回饋（越界安全取模，永不 panic）。
pub fn tame_line(pick: usize) -> &'static str {
    TAME_LINES[pick % TAME_LINES.len()]
}

/// 已馴服的兔子開始跟隨的距離（方塊）——比 [`FLEE_RADIUS`] 寬鬆許多：不必刻意逼近，
/// 平常靠近牠就會主動跟上。
pub const FOLLOW_RADIUS: f32 = 8.0;
/// 已在跟隨時，玩家要遠到超過這個距離才安心跟丟（遲滯，避免臨界距離上跟隨/走失來回抖動，
/// 手法同 [`should_flee`] 的 `FLEE_RADIUS`/`CALM_RADIUS` 兩段式門檻）。
pub const FOLLOW_LOSE_RADIUS: f32 = 14.0;
/// 跟隨速度——比閒晃（[`WANDER_SPEED`]）快一些才追得上你的腳步，但不到受驚逃跑那麼急。
pub const FOLLOW_SPEED: f32 = 2.4;
/// 跟到這個距離就停下，不再往玩家身上擠（方塊）。
pub const FOLLOW_STOP_DIST: f32 = 2.5;

/// 依「目前是否正在跟隨」+「與最近玩家的距離平方」，判斷這一 tick 該不該跟隨（或維持跟隨）。
///
/// 遲滯避免抖動：還沒跟上時要近到 [`FOLLOW_RADIUS`] 內才起跟；已在跟隨時要遠到
/// [`FOLLOW_LOSE_RADIUS`] 外才安心跟丟——與 [`should_flee`] 同一手法，只是換了一組半徑。
pub fn should_follow(currently_following: bool, nearest_player_dist_sq: f32) -> bool {
    let threshold = if currently_following { FOLLOW_LOSE_RADIUS } else { FOLLOW_RADIUS };
    nearest_player_dist_sq < threshold * threshold
}

/// 已在跟隨時，這一 tick 是否還要再往玩家的方向邁一步——跟到 [`FOLLOW_STOP_DIST`] 內
/// 就別再擠過去（純距離判定，供呼叫端決定要 `step_toward` 還是原地 `gravity_step`）。
pub fn should_close_follow_gap(player_dist_sq: f32) -> bool {
    player_dist_sq > FOLLOW_STOP_DIST * FOLLOW_STOP_DIST
}

// ── 寵物指令「安置／召回」v1（自主提案切片，ROADMAP 898）─────────────────────
// 馴服→跟隨（851）→取名（895）這條羈絆線至今，寵物永遠只會**黏著你走**——你走到哪牠
// 跟到哪，一刻也停不下來。跟隨 v1（851）的說明自己講明「不能召回/放開」；你沒有一條路能
// 叫牠「乖乖在這等我」。本刀補上那半：**點一下你取過名的小夥伴，就在「跟著你」與「在這安家
// 待命」之間切換**——叫牠待命，牠便在你放下牠的那一小塊地方安穩踱步，成為你家園的固定風景，
// 直到你再喚牠跟上。馴服的動物第一次真正「聽你的」。

/// 命令的觸及範圍（方塊，XZ 平面）——你得走到小夥伴身邊才能安置／召回牠。
/// 沿用取名同款「站到牠身邊」的親近距離（比餵食馴服 [`TAME_REACH`] 稍寬一點點）。
pub const COMMAND_REACH: f32 = 3.5;

/// 安置（待命）中的寵物在原地小範圍徘徊的半徑（方塊）——遠比一般閒晃（[`WANDER_MAX_R`]=6）
/// 收斂，牠只在你放牠下來的那一小塊地方安穩踱步，不會晃遠，成為你家園的固定風景。
pub const SETTLE_WANDER_R: f32 = 2.0;

/// 這一 tick 已馴服的寵物該不該跟隨你——在「安置（待命）」狀態下一律不跟
///（`settled=true` 蓋過一切距離判定，牠乖乖待在原地）；否則沿用既有 [`should_follow`]
/// 的兩段式遲滯。純函式、可窮舉測試。
pub fn follow_when_settleable(
    settled: bool,
    currently_following: bool,
    nearest_player_dist_sq: f32,
) -> bool {
    !settled && should_follow(currently_following, nearest_player_dist_sq)
}

/// 安置／召回成功那一刻回饋給玩家的暖句（`settled=true` 是叫牠待命、`false` 是喚牠跟上）。
/// 面向玩家、i18n 友善、含寵物名；確定性、可窮舉測試。名字已在呼叫端經清洗，這裡只組句。
pub fn command_ack_line(settled: bool, name: &str) -> String {
    if settled {
        format!("🐾『{name}』乖乖在這兒待命，等你回來。")
    } else {
        format!("🐾『{name}』又蹦蹦跳跳地跟上你了！")
    }
}

/// 依「目前是否已受驚」+「與最近玩家的距離平方」，判斷這一 tick 該不該受驚（或維持受驚）。
///
/// 用遲滯避免抖動：平靜時要近到 [`FLEE_RADIUS`] 內才受驚；已受驚時要遠到
/// [`CALM_RADIUS`] 外才平靜下來——兩段式門檻讓「快靠近/快遠離」的邊界不反覆橫跳。
pub fn should_flee(currently_fleeing: bool, nearest_player_dist_sq: f32) -> bool {
    let threshold = if currently_fleeing { CALM_RADIUS } else { FLEE_RADIUS };
    nearest_player_dist_sq < threshold * threshold
}

/// 兔群數量天花板（世界初始 6 隻 + 最多再生 6 隻）——馴服兔子生寶寶 v1 防止無限增長。
pub const MAX_RABBITS: usize = 12;
/// 兩隻已馴服的兔子要湊到多近才算「在一起」、有機會生寶寶（方塊）。
pub const BREED_RADIUS: f32 = 3.0;
/// 全域生育節流：至少間隔這麼久才會再檢查一次生育（秒）——比照人口成長 v1 的
/// elapsed 節流手法，避免同一對兔子黏在一起就無限連生。
pub const BREED_INTERVAL_SECS: f32 = 90.0;
/// 節流窗口到了、且找得到湊近的一對時，這次判定的生育機率。
pub const BREED_CHANCE: f32 = 0.35;

// ── 春回兔繁 v1（自主提案切片）：季節（798）× 繁殖（855）首次機械交會 ──────────────
// 繁殖（855）至今不分季節、恆用 [`BREED_CHANCE`]/[`BREED_INTERVAL_SECS`]；季節（798）至今
// 只換天色＋觸發居民感言，從不牽動野生動物。本刀讓「春天＝萬物復甦的繁殖季」：春季裡馴服
// 兔子繁殖更旺（機率更高、節流更短），玩家一眼看得出「春天兔寶寶特別多」。
// **只獎不罰**（守療癒優先鐵律）：春季加成，其餘季節維持基礎值、不減速、不影響資料。

/// 春季（繁殖季）的生育機率——明顯高於基礎 [`BREED_CHANCE`]（0.35），讓春天的繁殖旺盛有感。
pub const SPRING_BREED_CHANCE: f32 = 0.55;
/// 春季（繁殖季）的生育節流間隔（秒）——比基礎 [`BREED_INTERVAL_SECS`]（90）更短，春天生得更勤。
pub const SPRING_BREED_INTERVAL_SECS: f32 = 60.0;

/// 依季節回傳生育機率：春季用加成值，其餘季節維持基礎值（只獎不罰）。
pub fn seasonal_breed_chance(season: crate::voxel_season::Season) -> f32 {
    match season {
        crate::voxel_season::Season::Spring => SPRING_BREED_CHANCE,
        _ => BREED_CHANCE,
    }
}

/// 依季節回傳生育節流間隔（秒）：春季更短，其餘季節維持基礎值（只獎不罰）。
pub fn seasonal_breed_interval(season: crate::voxel_season::Season) -> f32 {
    match season {
        crate::voxel_season::Season::Spring => SPRING_BREED_INTERVAL_SECS,
        _ => BREED_INTERVAL_SECS,
    }
}

/// 判斷這一輪節流窗口是否該誕生一隻小兔子（純函式、可測）：
/// 兔群數未達天花板 + 距上次生育夠久 + 機率骰命中。
pub fn should_breed(current_rabbit_count: usize, elapsed_since_last: f32, roll: f32) -> bool {
    current_rabbit_count < MAX_RABBITS
        && elapsed_since_last >= BREED_INTERVAL_SECS
        && roll < BREED_CHANCE
}

/// 季節感知版繁殖判定（春回兔繁 v1）：等同 [`should_breed`]，但節流間隔與機率改用該季節值。
/// 春季套加成（更短間隔、更高機率），其餘季節等同 [`should_breed`]（基礎值）。純函式、可測。
pub fn should_breed_seasonal(
    current_rabbit_count: usize,
    elapsed_since_last: f32,
    roll: f32,
    season: crate::voxel_season::Season,
) -> bool {
    current_rabbit_count < MAX_RABBITS
        && elapsed_since_last >= seasonal_breed_interval(season)
        && roll < seasonal_breed_chance(season)
}

/// 在目前所有已馴服兔子的座標（`(索引, x, z)`）裡，找出第一對距離在 [`BREED_RADIUS`]
/// 內的親代、回傳兩者的索引。純函式、零隨機、O(n²) 但 n 極小（兔群天花板僅 12）。
pub fn find_breeding_pair(tamed_positions: &[(usize, f32, f32)]) -> Option<(usize, usize)> {
    for i in 0..tamed_positions.len() {
        for j in (i + 1)..tamed_positions.len() {
            let (ia, ax, az) = tamed_positions[i];
            let (ib, bx, bz) = tamed_positions[j];
            let dx = ax - bx;
            let dz = az - bz;
            if dx * dx + dz * dz <= BREED_RADIUS * BREED_RADIUS {
                return Some((ia, ib));
            }
        }
    }
    None
}

/// 由一對親代座標算出寶寶的落地點（兩者中點，純幾何、無隨機性）。
pub fn baby_spawn_point(ax: f32, az: f32, bx: f32, bz: f32) -> (f32, f32) {
    ((ax + bx) / 2.0, (az + bz) / 2.0)
}

/// 小兔子誕生那一刻的回饋句（確定性輪替，`pick` 由呼叫端提供隨機源）。
const BABY_LINES: [&str; 3] = [
    "🐇 草地上多了一隻毛茸茸的小兔子，正跌跌撞撞地跟著爸媽學走路。",
    "🐇 兩隻兔子依偎了一會兒，不知不覺間，身邊多了一隻怯生生的小兔子。",
    "🐇 一隻剛出生的小兔子睜開眼，第一眼就認出了你——牠也不怕你。",
];

/// 依 `pick` 取一句誕生回饋（越界安全取模，永不 panic）。
pub fn baby_line(pick: usize) -> &'static str {
    BABY_LINES[pick % BABY_LINES.len()]
}

/// 春回兔繁 v1：春季誕生的專屬動態牆感言（與四季通用的 [`baby_line`] 語氣區隔，讓「春天＝
/// 繁殖季」在動態牆上看得見）。面向玩家字串集中此處，便於日後 i18n。
const SPRING_BABY_LINES: [&str; 3] = [
    "🌱🐇 春回大地，一窩小兔子在暖陽下睜開眼——這是萬物萌生的季節。",
    "🌸🐇 繁花初綻的春天，兔群又添了個毛茸茸的新成員，蹦蹦跳跳追著父母。",
    "🌿🐇 春意正濃，草地上多了一隻怯生生的小兔子，正是繁衍生息的好時節。",
];

/// 依 `pick` 取一句春季誕生回饋（越界安全取模，永不 panic）。
pub fn spring_baby_line(pick: usize) -> &'static str {
    SPRING_BABY_LINES[pick % SPRING_BABY_LINES.len()]
}

/// 由兔子座標與（最近）玩家座標算出「逃離玩家」的目標點（純幾何、無隨機性、可測）。
///
/// 玩家與兔子恰好同座標（距離為 0，退化情況）時預設往 +x 方向逃，避免除以零。
pub fn flee_target(rx: f32, rz: f32, px: f32, pz: f32) -> (f32, f32) {
    let dx = rx - px;
    let dz = rz - pz;
    let dist = (dx * dx + dz * dz).sqrt();
    if dist < 1e-4 {
        (rx + FLEE_DIST, rz)
    } else {
        (rx + dx / dist * FLEE_DIST, rz + dz / dist * FLEE_DIST)
    }
}

// ── 幼獸長大 v1（自主提案切片）：出生（855）之後第一次真的會「長大」 ──────────────

/// 小兔子要活過這麼久（秒）才算長大成兔——刻意設得比居民成年禮（[`COMING_OF_AGE_SECS`]
/// 一整個乙太年）短得多，動物比人類長得快，一個遊玩階段內就看得見寶寶長大的過程。
///
/// [`COMING_OF_AGE_SECS`]: crate::voxel_coming_of_age::COMING_OF_AGE_SECS
pub const GROWTH_SECS: f32 = 600.0;

/// 長大前的最小體型縮放（出生那一刻）；長大後固定為 1.0（見 [`growth_scale`]）。
pub const BABY_MIN_SCALE: f32 = 0.5;

/// 判斷這隻兔子此刻是不是還沒長大的寶寶。`born_unix == 0` 代表世界初始生成／出生系統
/// 補上前既有的兔子，一律視為已成年（不受本刀影響，零回歸）。壞值（`now < born_unix`，
/// 理論上不該發生但保守處理）視為剛出生、仍是寶寶，不會 panic。
pub fn is_baby(born_unix: u64, now: u64) -> bool {
    born_unix != 0 && now.saturating_sub(born_unix) < GROWTH_SECS as u64
}

/// 依出生時刻算出此刻的體型縮放：出生那一刻是 [`BABY_MIN_SCALE`]，隨時間線性長到
/// [`GROWTH_SECS`] 那一刻滿 `1.0`，之後恆為 `1.0`。`born_unix == 0`（既有成兔）恆 `1.0`。
/// 純函式、確定性、對壞值（`now < born_unix`）安全夾限，永不越界或 panic。
pub fn growth_scale(born_unix: u64, now: u64) -> f32 {
    if born_unix == 0 {
        return 1.0;
    }
    let elapsed = now.saturating_sub(born_unix) as f32;
    let frac = (elapsed / GROWTH_SECS).clamp(0.0, 1.0);
    BABY_MIN_SCALE + (1.0 - BABY_MIN_SCALE) * frac
}

/// 從「已馴服兔子」候選名單裡濾掉還沒長大的寶寶——長大前不會被 [`find_breeding_pair`]
/// 選為親代（行為後果，不只是體型變化）。純函式、保留原本的索引/座標不動。
pub fn eligible_breeders(
    tamed_positions: &[(usize, f32, f32, u64)],
    now: u64,
) -> Vec<(usize, f32, f32)> {
    tamed_positions
        .iter()
        .filter(|(_, _, _, born_unix)| !is_baby(*born_unix, now))
        .map(|(i, x, z, _)| (*i, *x, *z))
        .collect()
}

/// 長大成兔那一刻的動態牆播報句（確定性輪替，`pick` 由呼叫端提供隨機源）。
const GROWN_UP_LINES: [&str; 3] = [
    "🐰 那隻曾經跌跌撞撞的小兔子，不知不覺已經長得跟爸媽一樣大了。",
    "🐰 一眨眼的功夫，小兔子已經長大成兔，蹦跳的模樣沉穩了不少。",
    "🐰 曾經怯生生的寶寶，如今已經是一隻像模像樣的成兔了。",
];

/// 依 `pick` 取一句長大回饋（越界安全取模，永不 panic）。
pub fn grown_up_line(pick: usize) -> &'static str {
    GROWN_UP_LINES[pick % GROWN_UP_LINES.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_flee_triggers_within_flee_radius_when_calm() {
        assert!(should_flee(false, 3.9 * 3.9));
        assert!(!should_flee(false, 4.1 * 4.1));
    }

    #[test]
    fn should_flee_hysteresis_keeps_fleeing_until_calm_radius() {
        // 已受驚：距離落在 flee~calm 之間仍算受驚（遲滯，不提早平靜）。
        assert!(should_flee(true, 5.0 * 5.0));
        // 遠到超過 calm 半徑才真正平靜。
        assert!(!should_flee(true, 6.1 * 6.1));
    }

    #[test]
    fn should_flee_exact_boundary_is_exclusive() {
        // 距離恰好等於門檻不算「進入」（< 而非 <=），邊界一致不誤觸。
        assert!(!should_flee(false, FLEE_RADIUS * FLEE_RADIUS));
        assert!(!should_flee(true, CALM_RADIUS * CALM_RADIUS));
    }

    #[test]
    fn flee_target_points_directly_away_from_player() {
        let (tx, tz) = flee_target(0.0, 0.0, 1.0, 0.0);
        assert!(tx < 0.0, "玩家在 +x，兔子該往 -x 逃，得到 tx={tx}");
        assert!(tz.abs() < 1e-4);
    }

    #[test]
    fn flee_target_scales_to_flee_dist() {
        let (tx, tz) = flee_target(0.0, 0.0, 0.0, 3.0);
        let dist = (tx * tx + tz * tz).sqrt();
        assert!((dist - FLEE_DIST).abs() < 1e-3);
    }

    #[test]
    fn flee_target_handles_zero_distance_without_panic() {
        let (tx, tz) = flee_target(5.0, 5.0, 5.0, 5.0);
        assert!((tx - (5.0 + FLEE_DIST)).abs() < 1e-4);
        assert!((tz - 5.0).abs() < 1e-4);
    }

    #[test]
    fn flee_target_diagonal_direction() {
        let (tx, tz) = flee_target(0.0, 0.0, 1.0, 1.0);
        assert!(tx < 0.0 && tz < 0.0, "玩家在右上方，兔子該往左下逃");
        // 對角逃跑距離仍應是 FLEE_DIST（正規化過的方向向量）。
        let dist = (tx * tx + tz * tz).sqrt();
        assert!((dist - FLEE_DIST).abs() < 1e-3);
    }

    #[test]
    fn should_tame_requires_close_enough() {
        assert!(should_tame(false, 2.9 * 2.9));
        assert!(!should_tame(false, 3.1 * 3.1));
    }

    #[test]
    fn should_tame_boundary_is_exclusive() {
        assert!(!should_tame(false, TAME_REACH * TAME_REACH));
    }

    #[test]
    fn should_tame_rejects_already_tamed_regardless_of_distance() {
        assert!(!should_tame(true, 0.0));
    }

    #[test]
    fn tame_reach_tighter_than_flee_radius() {
        // 刻意設計：要餵到牠，得先追上正在逃跑的兔子（見模組說明）。
        assert!(TAME_REACH < FLEE_RADIUS);
    }

    #[test]
    fn tame_line_picks_vary_and_stay_nonempty() {
        let seen: std::collections::HashSet<&str> =
            (0..TAME_LINES.len()).map(tame_line).collect();
        assert_eq!(seen.len(), TAME_LINES.len(), "四句應各不相同");
        for pick in 0..TAME_LINES.len() {
            assert!(!tame_line(pick).is_empty());
        }
    }

    #[test]
    fn tame_line_pick_wraps_without_panic() {
        // 越界 pick 應安全取模，不 panic。
        let _ = tame_line(usize::MAX);
    }

    #[test]
    fn should_follow_triggers_within_follow_radius_when_not_following() {
        assert!(should_follow(false, 7.9 * 7.9));
        assert!(!should_follow(false, 8.1 * 8.1));
    }

    #[test]
    fn should_follow_hysteresis_keeps_following_until_lose_radius() {
        // 已在跟隨：距離落在 follow~lose 之間仍算跟著（遲滯，不提早跟丟）。
        assert!(should_follow(true, 10.0 * 10.0));
        // 遠到超過走失半徑才真正跟丟。
        assert!(!should_follow(true, 14.1 * 14.1));
    }

    #[test]
    fn should_follow_exact_boundary_is_exclusive() {
        assert!(!should_follow(false, FOLLOW_RADIUS * FOLLOW_RADIUS));
        assert!(!should_follow(true, FOLLOW_LOSE_RADIUS * FOLLOW_LOSE_RADIUS));
    }

    #[test]
    fn follow_radius_tighter_than_lose_radius() {
        // 遲滯設計前提：起跟半徑必須小於走失半徑，否則兩段式門檻無意義。
        assert!(FOLLOW_RADIUS < FOLLOW_LOSE_RADIUS);
    }

    #[test]
    fn should_close_follow_gap_stops_within_stop_dist() {
        assert!(should_close_follow_gap(2.6 * 2.6));
        assert!(!should_close_follow_gap(2.4 * 2.4));
    }

    #[test]
    fn should_close_follow_gap_boundary_is_exclusive() {
        assert!(!should_close_follow_gap(FOLLOW_STOP_DIST * FOLLOW_STOP_DIST));
    }

    // ── 馴服兔子生寶寶 v1 ────────────────────────────────────────────────

    #[test]
    fn should_breed_requires_all_three_conditions() {
        assert!(should_breed(4, BREED_INTERVAL_SECS, 0.0));
        assert!(!should_breed(MAX_RABBITS, BREED_INTERVAL_SECS, 0.0), "到天花板不該再生");
        assert!(!should_breed(4, BREED_INTERVAL_SECS - 1.0, 0.0), "節流未到不該生");
        assert!(!should_breed(4, BREED_INTERVAL_SECS, BREED_CHANCE), "機率沒中不該生");
    }

    #[test]
    fn should_breed_boundary_is_inclusive_for_elapsed() {
        // elapsed 恰好等於節流秒數應可生（>= 而非 >）。
        assert!(should_breed(0, BREED_INTERVAL_SECS, 0.0));
    }

    #[test]
    fn should_breed_chance_boundary_is_exclusive() {
        assert!(should_breed(0, BREED_INTERVAL_SECS, BREED_CHANCE - 0.001));
        assert!(!should_breed(0, BREED_INTERVAL_SECS, BREED_CHANCE));
    }

    #[test]
    fn find_breeding_pair_finds_close_pair() {
        let positions = vec![(0usize, 0.0, 0.0), (2usize, 100.0, 100.0), (5usize, 1.0, 1.0)];
        let pair = find_breeding_pair(&positions);
        assert_eq!(pair, Some((0, 5)), "索引 0 與 5 距離夠近應配成一對");
    }

    #[test]
    fn find_breeding_pair_none_when_all_far_apart() {
        let positions = vec![(0usize, 0.0, 0.0), (1usize, 100.0, 0.0), (2usize, 0.0, 100.0)];
        assert_eq!(find_breeding_pair(&positions), None);
    }

    #[test]
    fn find_breeding_pair_none_when_fewer_than_two() {
        assert_eq!(find_breeding_pair(&[]), None);
        assert_eq!(find_breeding_pair(&[(0usize, 0.0, 0.0)]), None);
    }

    #[test]
    fn find_breeding_pair_boundary_is_inclusive() {
        // 恰好等於 BREED_RADIUS 應算「湊近」（<= 而非 <，與 should_tame 等距離判定刻意不同——
        // 這裡沒有「先受驚再馴服」那種需要嚴格小於的追逐設計，純粹「夠近就算」）。
        let positions = vec![(0usize, 0.0, 0.0), (1usize, BREED_RADIUS, 0.0)];
        assert_eq!(find_breeding_pair(&positions), Some((0, 1)));
    }

    #[test]
    fn baby_spawn_point_is_midpoint() {
        let (x, z) = baby_spawn_point(0.0, 0.0, 4.0, 2.0);
        assert!((x - 2.0).abs() < 1e-4);
        assert!((z - 1.0).abs() < 1e-4);
    }

    #[test]
    fn baby_line_picks_vary_and_stay_nonempty() {
        let seen: std::collections::HashSet<&str> =
            (0..BABY_LINES.len()).map(baby_line).collect();
        assert_eq!(seen.len(), BABY_LINES.len(), "三句應各不相同");
        for pick in 0..BABY_LINES.len() {
            assert!(!baby_line(pick).is_empty());
        }
    }

    #[test]
    fn baby_line_pick_wraps_without_panic() {
        let _ = baby_line(usize::MAX);
    }

    // ── 春回兔繁 v1（自主提案切片）：季節 × 繁殖 ────────────────────────────────
    use crate::voxel_season::Season;

    #[test]
    fn spring_boosts_breed_chance_and_shortens_interval() {
        // 春季機率高於基礎、間隔短於基礎（繁殖旺盛）。
        assert!(seasonal_breed_chance(Season::Spring) > BREED_CHANCE);
        assert!(seasonal_breed_interval(Season::Spring) < BREED_INTERVAL_SECS);
        assert_eq!(seasonal_breed_chance(Season::Spring), SPRING_BREED_CHANCE);
        assert_eq!(seasonal_breed_interval(Season::Spring), SPRING_BREED_INTERVAL_SECS);
    }

    #[test]
    fn non_spring_seasons_keep_base_values() {
        // 只獎不罰：夏／秋／冬皆維持基礎值，不加成也不減速。
        for s in [Season::Summer, Season::Autumn, Season::Winter] {
            assert_eq!(seasonal_breed_chance(s), BREED_CHANCE, "非春季機率應為基礎值");
            assert_eq!(seasonal_breed_interval(s), BREED_INTERVAL_SECS, "非春季間隔應為基礎值");
        }
    }

    #[test]
    fn should_breed_seasonal_matches_should_breed_off_spring() {
        // 非春季時，季節感知版與原版判定完全一致（回歸保證）。
        for s in [Season::Summer, Season::Autumn, Season::Winter] {
            for &(cnt, elapsed, roll) in &[
                (0usize, BREED_INTERVAL_SECS, 0.0f32),
                (MAX_RABBITS, BREED_INTERVAL_SECS, 0.0),
                (4, BREED_INTERVAL_SECS - 1.0, 0.0),
                (4, BREED_INTERVAL_SECS, BREED_CHANCE),
                (4, BREED_INTERVAL_SECS, BREED_CHANCE - 0.001),
            ] {
                assert_eq!(
                    should_breed_seasonal(cnt, elapsed, roll, s),
                    should_breed(cnt, elapsed, roll),
                    "非春季季節感知版應等同原版"
                );
            }
        }
    }

    #[test]
    fn should_breed_seasonal_spring_breeds_when_base_would_not() {
        // 一個「基礎判定不會生、但春季加成會生」的具體情境：
        // 間隔介於春季門檻與基礎門檻之間，且骰值介於基礎機率與春季機率之間。
        let elapsed = (SPRING_BREED_INTERVAL_SECS + BREED_INTERVAL_SECS) / 2.0;
        let roll = (BREED_CHANCE + SPRING_BREED_CHANCE) / 2.0;
        assert!(!should_breed(0, elapsed, roll), "基礎判定：間隔未到＋機率沒中，不生");
        assert!(
            should_breed_seasonal(0, elapsed, roll, Season::Spring),
            "春季加成：間隔已過門檻＋機率命中，該生"
        );
    }

    #[test]
    fn should_breed_seasonal_spring_respects_cap() {
        // 春季再旺盛也守兔群天花板（防無限增長）。
        assert!(!should_breed_seasonal(MAX_RABBITS, SPRING_BREED_INTERVAL_SECS, 0.0, Season::Spring));
    }

    #[test]
    fn spring_baby_line_picks_vary_and_stay_nonempty() {
        let seen: std::collections::HashSet<&str> =
            (0..SPRING_BABY_LINES.len()).map(spring_baby_line).collect();
        assert_eq!(seen.len(), SPRING_BABY_LINES.len(), "春季三句應各不相同");
        for pick in 0..SPRING_BABY_LINES.len() {
            assert!(!spring_baby_line(pick).is_empty());
        }
        // 春季句與通用句應語氣區隔、不重複。
        for sp in SPRING_BABY_LINES {
            assert!(!BABY_LINES.contains(&sp), "春季句不應與通用句重複");
        }
    }

    #[test]
    fn spring_baby_line_pick_wraps_without_panic() {
        let _ = spring_baby_line(usize::MAX);
    }

    // ── 寵物指令「安置／召回」v1（ROADMAP 898）────────────────────────────
    #[test]
    fn settled_pet_never_follows_regardless_of_distance() {
        // 安置（待命）狀態蓋過一切距離判定：不管玩家貼多近、原本跟不跟，一律不跟。
        for &was_following in &[false, true] {
            for &d in &[0.0f32, 1.0, FOLLOW_RADIUS * FOLLOW_RADIUS, 1e9] {
                assert!(
                    !follow_when_settleable(true, was_following, d),
                    "安置中不該跟隨（was_following={was_following}, d²={d}）"
                );
            }
        }
    }

    #[test]
    fn unsettled_pet_matches_should_follow() {
        // 未安置時，follow_when_settleable 完全等同既有 should_follow（不改變跟隨手感）。
        for &was_following in &[false, true] {
            for &d in &[0.0f32, 60.0, 120.0, 200.0, 1e9] {
                assert_eq!(
                    follow_when_settleable(false, was_following, d),
                    should_follow(was_following, d),
                    "未安置應與 should_follow 一致（was_following={was_following}, d²={d}）"
                );
            }
        }
    }

    #[test]
    fn command_ack_line_contains_name_and_differs_by_state() {
        let stay = command_ack_line(true, "露露");
        let come = command_ack_line(false, "露露");
        assert!(stay.contains("露露") && come.contains("露露"), "回饋句應含寵物名");
        assert_ne!(stay, come, "待命／召回兩句應不同");
        for line in [&stay, &come] {
            assert!(!line.is_empty());
            assert!(!line.contains('\n'), "名牌／泡泡單行，不可含換行");
        }
    }

    #[test]
    fn command_ack_line_handles_odd_names_without_panic() {
        // 名字已在呼叫端清洗，但組句本身對空字串／長字串也不該 panic。
        let _ = command_ack_line(true, "");
        let _ = command_ack_line(false, &"喵".repeat(64));
    }

    // ── 幼獸長大 v1 ──────────────────────────────────────────────────────────

    #[test]
    fn is_baby_true_right_after_birth() {
        assert!(is_baby(1000, 1000), "剛出生那一刻仍是寶寶");
        assert!(is_baby(1000, 1000 + (GROWTH_SECS as u64) - 1), "還沒滿長大門檻仍是寶寶");
    }

    #[test]
    fn is_baby_false_once_grown_secs_elapsed() {
        assert!(!is_baby(1000, 1000 + GROWTH_SECS as u64), "剛好滿門檻該算長大");
        assert!(!is_baby(1000, 1000 + GROWTH_SECS as u64 + 999), "門檻之後恆為成兔");
    }

    #[test]
    fn is_baby_legacy_born_unix_zero_always_adult() {
        // 世界初始生成／出生系統補上前既有的兔子，born_unix==0，永遠不是寶寶。
        assert!(!is_baby(0, 0));
        assert!(!is_baby(0, 999_999));
    }

    #[test]
    fn is_baby_clock_underflow_is_safe_and_still_baby() {
        // now < born_unix 理論上不該發生，但不可 panic；saturating_sub 得 0，視為剛出生。
        assert!(is_baby(1000, 500));
    }

    #[test]
    fn growth_scale_starts_at_min_and_ends_at_one() {
        assert_eq!(growth_scale(1000, 1000), BABY_MIN_SCALE);
        assert_eq!(growth_scale(1000, 1000 + GROWTH_SECS as u64), 1.0);
        assert_eq!(growth_scale(1000, 1000 + GROWTH_SECS as u64 + 500), 1.0, "長大後恆為 1.0 不越界");
    }

    #[test]
    fn growth_scale_monotonic_and_bounded() {
        let mut prev = growth_scale(1000, 1000);
        for step in [100u64, 200, 300, 400, 500, 600] {
            let cur = growth_scale(1000, 1000 + step);
            assert!(cur >= prev, "體型應隨時間單調不減");
            assert!((BABY_MIN_SCALE..=1.0).contains(&cur), "縮放應落在 [BABY_MIN_SCALE, 1.0] 內");
            prev = cur;
        }
    }

    #[test]
    fn growth_scale_legacy_born_unix_zero_is_full_size() {
        assert_eq!(growth_scale(0, 12345), 1.0);
    }

    #[test]
    fn growth_scale_clock_underflow_is_safe() {
        let s = growth_scale(1000, 500);
        assert!((BABY_MIN_SCALE..=1.0).contains(&s), "壞值也不該越界或 panic");
    }

    #[test]
    fn eligible_breeders_excludes_babies() {
        let now = 10_000u64;
        let candidates = vec![
            (0usize, 0.0f32, 0.0f32, 0u64),        // 成兔（既有兔子）
            (1usize, 1.0, 1.0, now - 100),          // 剛出生不久，仍是寶寶
            (2usize, 2.0, 2.0, now - (GROWTH_SECS as u64) - 1), // 早就長大了
        ];
        let out = eligible_breeders(&candidates, now);
        let ids: Vec<usize> = out.iter().map(|(i, _, _)| *i).collect();
        assert_eq!(ids, vec![0, 2], "寶寶（索引1）應被濾掉，只留下已長大的親代候選");
    }

    #[test]
    fn eligible_breeders_empty_when_all_babies() {
        let now = 100u64;
        let candidates = vec![(0usize, 0.0f32, 0.0f32, now)];
        assert!(eligible_breeders(&candidates, now).is_empty());
    }

    #[test]
    fn grown_up_line_picks_vary_and_stay_nonempty() {
        let a = grown_up_line(0);
        let b = grown_up_line(1);
        assert_ne!(a, b);
        for line in [a, b, grown_up_line(2)] {
            assert!(!line.is_empty());
            assert!(!line.contains('\n'), "動態牆單行，不可含換行");
        }
    }

    #[test]
    fn grown_up_line_pick_wraps_without_panic() {
        let _ = grown_up_line(9999);
    }
}
