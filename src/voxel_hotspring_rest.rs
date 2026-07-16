//! 乙太方界·居民也去溫泉歇腳 v1（自主提案切片，接續 838/839「乙太方界·古代遺跡／溫泉遺跡」，
//! ROADMAP 1025）。
//!
//! **真缺口**：838 讓世界第一次有了值得走遠尋訪的地標，839 補上第二種「功能性、可重複造訪」的
//! 溫泉——走遠巧遇後泡進去能主動加速回血、讓飢餓消耗打折。可是這份回饋至今**只服務玩家**：
//! 溫泉的判定（`voxel::feet_in_hot_spring`）、生存 tick 加速全接在玩家連線上，居民對這泓暖泉
//! 完全視若無睹——世界近 200 刀以來，「地標」（838/839/940/975/1006/1019……）與「居民的日常
//! 行為」始終是兩條互不相干的線：居民會採集、會蓋家、會探訪彼此，卻從沒有一刀讓居民**走向**
//! 一處玩家專屬的地標。這是本刀要接上的第一次交會。
//!
//! **本刀補上**：閒著的居民偶爾放下手邊的事，走去村莊附近那泓已知的溫泉（`voxel::
//! village_hotspring_target`，由世界原點就近找到的固定一泓）泡個舒服的澡，逗留一會兒再回家。
//! 若你恰好也在那泓溫泉裡泡著，她會多說一句「沒想到你也在」，把這次巧遇記進心裡——地標第一次
//! 不只是玩家的祕境，也成了居民會去、也會記得的地方。
//!
//! **與既有元素 razor-sharp 區隔（非同軸換皮）**：
//! - 遠行探野（756~762 `voxel_expedition`）／邊陲探友（821）／跨村商隊（950）＝居民走向**由
//!   方位/朋友/聚落算出的抽象落點**，落點因居民而異、每位居民各有自己的一套；本刀＝**所有居民
//!   走向同一個、玩家早就認得的具體地標**——世界只有一泓「村子的溫泉」，是「居民的日常」與
//!   「玩家的地標」的交會，不是又一種抽象遠行。
//! - 摯友結伴同行（925）／居民邀你散步同行（926）＝**移動的過程本身**才是重點、走到哪算哪；
//!   本刀＝**有明確目的地與逗留動作**（泡溫泉），移動只是手段。
//! - 溫泉遺跡本體（839，`voxel_player_stats` 泡溫泉版生存 tick）＝**玩家專屬**的回血/飢餓加速；
//!   本刀完全不碰居民的血量/飢餓數值（居民本就沒有玩家那套生存 tick），純粹是**行為與社交**層
//!   的交會——居民走去、逗留、記憶、偶遇，不涉入 839 的生存數值系統，兩者互不重疊。
//! - 居民也會生病（自主提案）＝**被照顧**的脆弱狀態；本刀＝**主動的自我照顧**，與生病與否無關
//!   （v1 刻意不做「生病去溫泉能好得更快」的交叉——那是明顯可期待的下一步，留給未來一刀）。
//!
//! **成本紀律**：零 LLM（觸發/台詞/記憶全確定性）、零新持久化格式（純記憶體暫態，比照
//! `expedition`/`caravan`/`frontier_visit` 同款「重啟歸零、居民照常回家域」慣例，零 migration
//! 風險）、零新協議欄位（前端不必知道這是「溫泉之旅」，只看到居民走去一處已存在的地標、冒句
//! 泡泡，走既有居民座標/台詞廣播管線）、零新美術、FPS 零影響（只在既有低頻閒置巡檢的一個 tick
//! 判定，命中後複用既有 `step_toward`/`gravity_step` 移動管線，非每幀額外計算）。
//!
//! **濫用防護**：不收玩家自由文字輸入、不觸發 LLM、不開任何新對外端點、不動帳號權限——啟程
//! 完全由伺服器權威的「閒置狀態＋冷卻＋機率骰」決定，玩家無從自報或催發；「巧遇同泡」的台詞
//! 與記憶全為確定性模板，只嵌入既有登入時已清洗過的玩家顯示名（單行、無換行、`.chars().take`
//! 收斂長度），與既有 `walk_with`/`stroll` embed 玩家名同一手法，無新增注入面。
//!
//! **這裡只放確定性純邏輯**（啟程判定、抵達/逗留/返家的台詞與記憶文案），零 IO、零鎖、零
//! async、可單元測試。鎖 / 移動狀態機 / 廣播 / 記憶落地全在 `voxel_ws.rs`（沿用跨村商隊 950 的
//! 「去程持續朝目標走→抵達逗留→逾時放棄→冷卻」狀態機，鎖序不巢狀、Feed 走鎖外，守死鎖鐵律）。

/// 一位閒著的居民在某個 tick 起意去溫泉歇腳的機率（低頻、稀有感，比照跨村商隊
/// `vcaravan::EMBARK_CHANCE`/遠行探野 `vexp::EMBARK_CHANCE` 同量級——泡溫泉是偶爾為之的
/// 小確幸，不是每隔幾秒就上演）。
pub const DEPART_CHANCE: f32 = 0.015;

/// 抵達溫泉後的逗留秒數：安靜地泡一會兒就好，比摯友結伴同行（30 秒）稍長一些，畢竟是專程走
/// 遠路來的，值得多待一下。
pub const SOAK_SECS: f32 = 35.0;

/// 去程逾時倒數秒數（秒）：溫泉可能落在離主城 140~700+ 格的範圍（`voxel::
/// VILLAGE_HOTSPRING_SEARCH_RADIUS`），比一般遠行/商隊遠得多，逾時門檻同步放大，讓居民有足夠
/// 時間真的走到，同時仍是有限值——地形擋路等意外仍會讓這趟歇腳誠實放棄，不無限跋涉。
pub const TRAVEL_TIMEOUT_SECS: f32 = 900.0;

/// 一趟歇腳（抵達返家或去程放棄）落幕後的冷卻秒數：讓「去泡溫泉」稀少而有份量，不會同一位
/// 居民三兩下又想去。約合日夜循環（[`crate::daynight::DAY_LENGTH_SECS`]=600）的 2.5 倍。
pub const REST_COOLDOWN_SECS: f32 = 1500.0;

/// 判定「已走到溫泉池邊」的水平距離門檻（世界方塊）：比遠行/商隊的抵達判定稍寬，池子本身有
/// 一定範圍（`voxel::HOT_SPRING_RADIUS`），走到池緣就算到了，不必精準踩中池心。
pub const ARRIVE_RADIUS: f32 = 4.0;

/// 「巧遇玩家同泡」判定的水平距離門檻（世界方塊）：與抵達門檻同量級，玩家與居民都站在同一泓
/// 小池子附近才算「一起泡」，不會隔著老遠也算數。
pub const SOAK_TOGETHER_RADIUS: f32 = 4.0;

/// 抵達後就地打轉的閒晃半徑（世界方塊，供 `voxel_ws.rs` 閒晃中心鏈使用）：刻意比一般遠行/商隊
/// 的量級（8）更小——溫泉池本身只有 `voxel::HOT_SPRING_RADIUS`(2)+池緣一圈，安靜地泡在池子
/// 附近打轉，不該晃到乾地上去。
pub const WANDER_RADIUS: f32 = 3.0;

/// 動態牆分類（讓玩家一眼看出這是居民去溫泉歇腳，與「遠行」「商隊」等其他長途行為分開）。
pub const FEED_KIND: &str = "溫泉歇腳";

/// 獨自歇腳時的記憶帳戶哨兵鍵（不是特定玩家；比照 `voxel_expedition::EXPEDITION_MEMORY_PLAYER`
/// 同款慣例）——供 `all_memories_for` 之類的查詢照樣掃得到，只是明確標示「這不是關於哪位玩家」。
pub const SOLO_MEMORY_KEY: &str = "__voxel_hotspring_rest__";

/// 是否啟程去溫泉歇腳：閒置自由（由呼叫端把各種「正忙」狀態收斂成一個 bool）+ 冷卻已到期 +
/// 此刻沒在說別的話 + 過機率門檻。純函式、確定性、無 IO。`roll` 由呼叫端以
/// `rand::random::<f32>()` 餵入（與本專案其他機率骰同慣例）。
pub fn should_depart(idle_free: bool, cooldown: f32, say_empty: bool, roll: f32) -> bool {
    idle_free && cooldown <= 0.0 && say_empty && roll < DEPART_CHANCE
}

/// 是否已走到溫泉池邊（純幾何門檻，供呼叫端拿 `dx*dx+dz*dz` 餵入）。
pub fn has_arrived(dist_sq: f32) -> bool {
    dist_sq <= ARRIVE_RADIUS * ARRIVE_RADIUS
}

/// 是否與附近某位玩家「同泡」（純幾何門檻，供呼叫端拿 `dx*dx+dz*dz` 餵入）。
pub fn is_soaking_together(dist_sq: f32) -> bool {
    dist_sq <= SOAK_TOGETHER_RADIUS * SOAK_TOGETHER_RADIUS
}

/// 啟程時冒的一句（確定性輪替，`pick` 由呼叫端給隨機源）。
pub fn depart_bubble(pick: usize) -> &'static str {
    let variants: [&str; 3] = [
        "說要去那泓溫泉泡個澡，晚點回來。",
        "難得閒著，去溫泉那頭歇口氣。",
        "走了好一陣子的路，該去暖泉裡泡一泡才是。",
    ];
    variants[pick % variants.len()]
}

/// 抵達、獨自泡著時冒的一句（確定性輪替）。
pub fn soak_bubble_alone(pick: usize) -> &'static str {
    let variants: [&str; 3] = [
        "啊……這暖意，整個人都鬆下來了。",
        "泡在這裡，什麼煩心事都暫時不想了。",
        "早該找時間來泡一泡的。",
    ];
    variants[pick % variants.len()]
}

/// 抵達時發現玩家也在泡（或稍後才巧遇）冒的一句（確定性輪替；點名玩家更有溫度）。
pub fn soak_bubble_with_player(player: &str, pick: usize) -> String {
    let variants: [&str; 3] = [
        "沒想到{player}也在這裡泡溫泉呀，真愜意。",
        "{player}也來了？一起泡，更暖了。",
        "難得能跟{player}一起歇口氣，真好。",
    ];
    variants[pick % variants.len()].replace("{player}", player)
}

/// 逗留落幕、啟程返家時冒的一句（確定性輪替）。
pub fn return_bubble(pick: usize) -> &'static str {
    let variants: [&str; 3] = [
        "好啦，該回去了，人輕鬆多了。",
        "泡得差不多了，回家去吧。",
        "這一趟真值得，該回村裡了。",
    ];
    variants[pick % variants.len()]
}

/// 獨自歇腳落幕後記進心裡的一筆 episodic 記憶。純文字、確定性、單行防注入。
pub fn rest_memory_line() -> &'static str {
    "去村外那泓溫泉泡了個舒服的澡，整個人都放鬆下來了。"
}

/// 與玩家同泡時記進心裡的一筆 episodic 記憶（點名同泡的玩家；累積對他的好感）。
/// 純文字、確定性、單行防注入。
pub fn soak_with_player_memory_line(player: &str) -> String {
    format!("和{player}一起泡在溫泉裡，聊了好一會兒，很暖心。")
}

/// 啟程時的動態牆一行（點名居民；讓不在場的玩家也讀得到「居民也去了那泓溫泉」）。
pub fn depart_feed_line(resident: &str) -> String {
    format!("{resident}放下手邊的事，往村外那泓溫泉走去，說是想泡個舒服的澡。")
}

/// 巧遇玩家同泡時的動態牆一行（點名居民與玩家）。
pub fn soak_together_feed_line(resident: &str, player: &str) -> String {
    format!("{resident}和{player}一起泡在溫泉裡，聊得挺開心。")
}

/// 返家時的動態牆一行（點名居民）。
pub fn return_feed_line(resident: &str) -> String {
    format!("{resident}從溫泉那頭回來了，整個人看起來鬆快不少。")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_depart_needs_idle_cooldown_say_and_roll() {
        assert!(should_depart(true, 0.0, true, DEPART_CHANCE - 0.001));
        // 忙著別的事 → 不啟程。
        assert!(!should_depart(false, 0.0, true, 0.0));
        // 還在冷卻中 → 不啟程。
        assert!(!should_depart(true, 1.0, true, 0.0));
        // 正在說別的話 → 不啟程（等 say 清空）。
        assert!(!should_depart(true, 0.0, false, 0.0));
        // 擲骰沒過門檻 → 不啟程。
        assert!(!should_depart(true, 0.0, true, DEPART_CHANCE));
    }

    #[test]
    fn should_depart_cooldown_boundary_is_ready_at_zero() {
        // 冷卻恰好等於 0：視為已就緒（<=0 含端點）。
        assert!(should_depart(true, 0.0, true, 0.0));
        // 略大於 0：仍在冷卻中。
        assert!(!should_depart(true, 0.001, true, 0.0));
    }

    #[test]
    fn has_arrived_boundary_and_beyond() {
        let just_inside = (ARRIVE_RADIUS - 0.01).powi(2);
        assert!(has_arrived(just_inside));
        let exact = ARRIVE_RADIUS * ARRIVE_RADIUS;
        assert!(has_arrived(exact), "恰好等於門檻應視為抵達（含端點）");
        let outside = (ARRIVE_RADIUS + 0.5).powi(2);
        assert!(!has_arrived(outside));
    }

    #[test]
    fn is_soaking_together_boundary_and_beyond() {
        let inside = (SOAK_TOGETHER_RADIUS - 0.01).powi(2);
        assert!(is_soaking_together(inside));
        let outside = (SOAK_TOGETHER_RADIUS + 1.0).powi(2);
        assert!(!is_soaking_together(outside));
    }

    #[test]
    fn bubbles_are_never_empty_and_rotate() {
        for pick in [0usize, 1, 2, 3, 9, 200] {
            assert!(!depart_bubble(pick).is_empty());
            assert!(!soak_bubble_alone(pick).is_empty());
            assert!(!return_bubble(pick).is_empty());
        }
        // 越界 pick 安全取模，不 panic（上面迴圈本身已隱含驗證）。
        assert_ne!(depart_bubble(0), depart_bubble(1));
        assert_ne!(soak_bubble_alone(0), soak_bubble_alone(1));
        assert_ne!(return_bubble(0), return_bubble(1));
    }

    #[test]
    fn soak_with_player_bubble_names_player_and_is_bounded() {
        for pick in [0usize, 1, 2, 5, 100] {
            let s = soak_bubble_with_player("旅人", pick);
            assert!(s.contains("旅人"));
            assert!(!s.is_empty());
            assert!(!s.contains('\n'));
        }
        assert_ne!(soak_bubble_with_player("旅人", 0), soak_bubble_with_player("旅人", 1));
    }

    #[test]
    fn memory_and_feed_lines_are_single_line_and_name_participants() {
        assert!(!rest_memory_line().is_empty());
        assert!(!rest_memory_line().contains('\n'));

        let with_player = soak_with_player_memory_line("露娜");
        assert!(with_player.contains("露娜"));
        assert!(!with_player.contains('\n'));

        let depart = depart_feed_line("諾娃");
        assert!(depart.contains("諾娃"));
        assert!(!depart.contains('\n'));

        let together = soak_together_feed_line("諾娃", "旅人");
        assert!(together.contains("諾娃") && together.contains("旅人"));
        assert!(!together.contains('\n'));

        let ret = return_feed_line("賽勒");
        assert!(ret.contains("賽勒"));
        assert!(!ret.contains('\n'));
    }

    #[test]
    fn feed_does_not_falsely_claim_first_ever() {
        // 措辭不謊稱「史上第一次」——泡溫泉是偶爾為之的日常小事，不是里程碑。
        assert!(!depart_feed_line("露娜").contains("第一次"));
        assert!(!return_feed_line("露娜").contains("第一次"));
        assert!(!soak_together_feed_line("露娜", "旅人").contains("第一次"));
    }

    #[test]
    fn solo_memory_key_is_not_empty_and_distinct_from_typical_player_names() {
        // 哨兵鍵本身不應與任何常見顯示名撞在一起（雙底線前後綴一望即知非玩家名）。
        assert!(SOLO_MEMORY_KEY.starts_with("__"));
        assert!(SOLO_MEMORY_KEY.ends_with("__"));
    }
}
