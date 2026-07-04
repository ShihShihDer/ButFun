//! 乙太方界·集會鐘 v1（bell）——玩家親手鑄一座鐘、右鍵敲響，附近閒著的居民會**循著鐘聲走過來**
//! 聚到你身邊，說句話、心情變好、把「你敲鐘召我來」記進交情。
//!
//! **這一刀補的缺口**：世界至今所有「把居民聚起來」的場面都是**被動、自動**的——營火（791）要
//! 入夜、居民恰好路過才駐足；聚會（711）由關係自己湊；探訪（671）由居民自發。玩家**從來沒有一個
//! 主動把村民召集到自己身邊**的動詞。集會鐘把這一環補上：這是玩家第一次能**主動發起一場聚集**——
//! 你敲響鐘，散在各處、閒著的居民便循聲朝你走來。人類第一次能像村長一樣「搖鈴喚人」，
//! 呼應路線圖「人類是園丁／村長，看著小社會長」的精神。
//!
//! **與既有元素刻意區隔**：營火是「夜間、居民恰好路過才被動圍暖」；本鐘是「白天黑夜皆可、玩家主動
//! 敲響、居民主動走過來」——一個是被動場所、一個是主動召喚，觸發者（世界 vs 玩家）與方向
//! （居民路過 vs 居民循聲趕來）都不同。
//!
//! **純函式層**：本模組只有確定性純函式（範圍判定、資格閘、抵達判定、台詞），零 LLM、零鎖、
//! 零 async、零 IO、可單元測試。連線／鎖／廣播全留在 `voxel_ws.rs`（沿用既有短鎖循序＋鎖外事件
//! 佇列慣例，守 prod 死鎖鐵律）。

/// 集會鐘方塊／物品 ID（74：60~73 已被純物品/方塊佔用，74 是首個空號）。
pub const BELL_ID: u8 = 74;

/// 鐘聲召集半徑（世界方塊；水平距離）——這麼近的閒著居民聽得到鐘聲、會循聲趕來。
/// 給得比營火取暖（4）大得多——鐘聲本就要傳得遠、能把散在各處的村民召到一塊。
pub const SUMMON_RADIUS: f32 = 22.0;

/// 抵達判定半徑（世界方塊）——走到鐘的這麼近就算「聚到了」，停下反應。
pub const GATHER_RADIUS: f32 = 2.5;

/// 召集逾時（秒）：被召的居民朝鐘走這麼久還沒到（地形擋路等）就放棄，回歸平常作息，
/// 不無限鬼打牆（守北極星「卡住自救」精神）。
pub const SUMMON_TIMEOUT_SECS: f32 = 22.0;

/// 每位居民的召集冷卻（秒）：應召一次後隔這麼久才會再被鐘聲拉動——**濫用防護的主閘**：
/// 就算有人狂敲鐘（或到處擺鐘連環敲），同一位居民也不會被反覆拖著跑，日子仍過得下去。
pub const SUMMON_COOLDOWN_SECS: f32 = 75.0;

/// 一位居民正在應召的狀態（純記憶體、重啟歸零）：鐘的水平座標 + 剩餘逾時秒數 + 敲鐘者名。
#[derive(Clone, Debug)]
pub struct Summon {
    /// 鐘的水平中心 x（世界座標，方塊中心）。
    pub x: f32,
    /// 鐘的水平中心 z。
    pub z: f32,
    /// 剩餘逾時秒數（每 tick 遞減，歸零即放棄應召）。
    pub timer: f32,
    /// 敲鐘者玩家名（抵達時點名道謝＋記進交情；訪客名可為空）。
    pub ringer: String,
}

/// 敲鐘時，這位居民是否「聽得到、且有空應召」——三閘：沒睡著、沒在遠行、召集冷卻已到期。
/// 純函式，好窮舉測邊界。距離另判（見 `within_summon`）。
pub fn eligible(asleep: bool, on_expedition: bool, summon_cooldown: f32) -> bool {
    !asleep && !on_expedition && summon_cooldown <= 0.0
}

/// 居民（在 `(rx,rz)`）是否落在鐘（`(bx,bz)`）的召集半徑內——聽得到鐘聲。
pub fn within_summon(bx: f32, bz: f32, rx: f32, rz: f32, radius: f32) -> bool {
    let dx = bx - rx;
    let dz = bz - rz;
    dx * dx + dz * dz <= radius * radius
}

/// 應召的居民是否已走到鐘邊（水平偏移 `(dx,dz)` 在抵達半徑內）——到了就停下聚攏反應。
pub fn arrived(dx: f32, dz: f32, gather_radius: f32) -> bool {
    dx * dx + dz * dz <= gather_radius * gather_radius
}

/// 抵達鐘邊時的聚攏泡泡（通用、不點名）——五句輪替，字數短不破泡泡框。
/// `pick` 由呼叫端用座標 bits 合成，讓每次挑到的句子自然分散。
pub fn gather_bubble(pick: usize) -> &'static str {
    const LINES: [&str; 5] = [
        "聽到鐘聲了，我來啦！",
        "鐘響了，是有什麼事嗎？",
        "來了來了，什麼事這麼急？",
        "鐘一響，我就趕過來了。",
        "大家都被鐘聲喚來了呢。",
    ];
    LINES[pick % LINES.len()]
}

/// 敲鐘者也在鐘邊時的聚攏泡泡（點名玩家，更親）——四句輪替，玩家名截斷不破泡泡框。
pub fn gather_bubble_with_ringer(ringer: &str, pick: usize) -> String {
    let name = clip_name(ringer);
    const TEMPLATES: [&str; 4] = [
        "{name}，你敲鐘找我嗎？我來了！",
        "聽到{name}的鐘聲，我立刻趕過來了。",
        "{name}搖鈴啦，大家快聚過來～",
        "是{name}在召集大家嗎？我到了。",
    ];
    TEMPLATES[pick % TEMPLATES.len()].replace("{name}", &name)
}

/// 昇華成一筆「你敲鐘召我來」的記憶（點名玩家、不含換行，走既有 append-only 記憶管線）。
pub fn gather_memory_line(ringer: &str) -> String {
    format!("{}敲響集會鐘把我召了過去，大夥兒聚在一塊真熱鬧。", clip_name(ringer)).replace('\n', " ")
}

/// 動態牆播報（非同步層，訪客回來能讀到誰敲了鐘、召來幾位居民）。
pub fn ring_feed_line(ringer: &str, count: usize) -> String {
    format!("{}敲響了集會鐘，{}位居民循聲聚了過來。", clip_name(ringer), count)
}

/// 玩家名截斷到 8 字（中文安全，避免超長名撐破泡泡框／Feed）。
fn clip_name(name: &str) -> String {
    name.chars().take(8).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eligible_needs_all_three_gates() {
        // 醒著 + 沒遠行 + 冷卻到期 → 可應召。
        assert!(eligible(false, false, 0.0));
        // 睡著 → 否。
        assert!(!eligible(true, false, 0.0));
        // 遠行中 → 否。
        assert!(!eligible(false, true, 0.0));
        // 冷卻未到 → 否（邊界：剛好 0 可、正值不可）。
        assert!(!eligible(false, false, 5.0));
        assert!(eligible(false, false, 0.0));
        assert!(!eligible(false, false, 0.01));
    }

    #[test]
    fn within_summon_respects_radius() {
        // 鐘在 (10,10)，居民在 (12,10)：距 2 < 22 → 聽得到。
        assert!(within_summon(10.0, 10.0, 12.0, 10.0, SUMMON_RADIUS));
        // 居民在 (40,10)：距 30 > 22 → 太遠、聽不到。
        assert!(!within_summon(10.0, 10.0, 40.0, 10.0, SUMMON_RADIUS));
        // 邊界：恰好在半徑上（距離平方 == 半徑平方）→ 算聽得到（<=）。
        assert!(within_summon(0.0, 0.0, SUMMON_RADIUS, 0.0, SUMMON_RADIUS));
        // 略超一點 → 否。
        assert!(!within_summon(0.0, 0.0, SUMMON_RADIUS + 0.1, 0.0, SUMMON_RADIUS));
    }

    #[test]
    fn arrived_only_when_close() {
        assert!(arrived(0.0, 0.0, GATHER_RADIUS));
        assert!(arrived(GATHER_RADIUS, 0.0, GATHER_RADIUS)); // 邊界含 <=
        assert!(!arrived(GATHER_RADIUS + 0.1, 0.0, GATHER_RADIUS));
        assert!(!arrived(10.0, 10.0, GATHER_RADIUS));
    }

    #[test]
    fn bubbles_rotate_and_stay_in_frame() {
        // 通用聚攏語輪替、非空。
        for p in 0..10 {
            assert!(!gather_bubble(p).is_empty());
        }
        assert_ne!(gather_bubble(0), gather_bubble(1));
        // 點名版含玩家名、輪替、超長名截斷不破框。
        let s = gather_bubble_with_ringer("旅人", 0);
        assert!(s.contains("旅人"));
        assert_ne!(
            gather_bubble_with_ringer("旅人", 0),
            gather_bubble_with_ringer("旅人", 1)
        );
        let long = gather_bubble_with_ringer("超級無敵長長長長長長長名字", 2);
        assert!(long.chars().count() < 40, "超長名應被截斷不破泡泡框：{long}");
    }

    #[test]
    fn memory_and_feed_embed_names_no_newline() {
        let m = gather_memory_line("諾娃\n注入");
        assert!(m.contains("諾娃"));
        assert!(!m.contains('\n'), "記憶不得含換行（防注入破行）：{m}");
        let f = ring_feed_line("露娜", 3);
        assert!(f.contains("露娜"));
        assert!(f.contains('3'), "Feed 應報出應召人數：{f}");
    }

    #[test]
    fn summon_struct_carries_fields() {
        // Summon 攜帶鐘座標＋逾時＋敲鐘者，clone 後仍一致（tick 迴圈會 clone 出來判定）。
        let s = Summon { x: 1.5, z: 2.5, timer: SUMMON_TIMEOUT_SECS, ringer: "露娜".into() };
        let c = s.clone();
        assert_eq!(c.x, 1.5);
        assert_eq!(c.z, 2.5);
        assert_eq!(c.ringer, "露娜");
    }

    #[test]
    fn bell_id_is_free_slot() {
        // 74 落在既有純物品/方塊 id（≤73）之後，且不與營火(70)/水桶(71)/滿水桶(72)/鋤頭(73)相撞。
        assert_eq!(BELL_ID, 74);
        assert!(BELL_ID > 73);
    }
}
