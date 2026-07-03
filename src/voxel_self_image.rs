//! 乙太方界·居民自我印象 v1（ROADMAP 770·PLAN_ETHERVOX 路線圖 item 2「reflection：把舊記憶
//! 摘要成高階印象」）。
//!
//! **核心信念**（見 `docs/PLAN_ETHERVOX.md`）：記憶不是功能，是讓居民「真的活著」的土壤。
//! 至今居民的每一筆記憶都停在**單筆**層次——蓋了什麼、遇到誰、聊了什麼各自散落，從沒有一刻
//! 讓居民**回頭看自己這一路，昇華出一個「我是個什麼樣的人」的自我概念**。本模組補上這一環：
//! 把居民累積的episodic記憶依「生活領域」分類、統計，若某個領域**明顯**是牠這陣子最投入的事，
//! 就昇華成一句第一人稱的自我印象——
//!
//!   「不知不覺，我好像成了這一帶最愛動手蓋東西的人了。」
//!
//! 這句自我印象①顯示在日記頁頂端（訪客翻日記第一眼就讀到「這位居民如何看待自己」），
//! ②偶爾在閒暇時被居民自言自語說出口＋記進動態牆——**記憶第一次不只被記住、被說出，還昇華成
//! 居民對自己的理解**。這是「記憶→行為」的一種：累積的生活形塑了牠怎麼看自己、怎麼開口。
//!
//! **隱私 / 濫用防護鐵律**（沿用 `voxel_diary` 同一守則）：自我印象輸出**永遠是固定模板**
//! （生活領域→確定性句子），**絕不回放任何記憶摘要原文、玩家原話或玩家名**——記憶內容只在
//! 內部用來「數哪個領域最多」，不外洩一個字。零新輸入面、零新端點、零 LLM。
//!
//! 純邏輯層：分類、統計、選句皆為**確定性純函式**，零 IO / 零鎖 / 零 async / 零 LLM，窮舉可測。
//! 鎖與副作用（讀記憶、冒泡泡、記動態）全在 `voxel_ws.rs`。不抄外部碼；繁中註解。

use crate::voxel_memory::MemoryEntry;

/// 居民的「生活領域」——把散落的記憶歸類到牠這陣子在做的事。
///
/// 分類刻意聚焦「**居民自己做了什麼**」而非「跟玩家聊了什麼主題」（後者是 `voxel_diary` 的職責）：
/// 自我印象問的是「我這一路成了個怎樣的人」，答案來自牠的作為，不是牠的談資。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelfDomain {
    /// 動手蓋——建造、堆疊、家園、塔橋牆。
    Builder,
    /// 侍弄泥土——種田、澆灌、作物、菜園。
    Farmer,
    /// 鑿石探洞——採礦、挖掘、晶石、礦坑。
    Miner,
    /// 傍水垂釣——釣魚、湖池、水邊。
    Angler,
    /// 仰望星空——觀星、夜空、月亮、銀河。
    Stargazer,
    /// 漫遊荒野——遠行、邊陲、漂泊、散居、紮營。
    Wanderer,
    /// 待人以暖——幫忙、照料、送禮、分享、迎客、留心意。
    Caretaker,
    /// 重情念舊——情誼、老朋友、探訪、相聚、想念、惦記。
    Companion,
}

impl SelfDomain {
    /// 這個領域在自我印象裡的稱呼（i18n 預留點：集中此一處替換）。
    fn epithet(self) -> &'static str {
        match self {
            SelfDomain::Builder => "最愛動手蓋東西的人",
            SelfDomain::Farmer => "離不開泥土的人",
            SelfDomain::Miner => "總往石頭與洞穴裡鑽的人",
            SelfDomain::Angler => "離不開水邊的人",
            SelfDomain::Stargazer => "老是抬頭望著夜空的人",
            SelfDomain::Wanderer => "待不住、總想往荒野走的人",
            SelfDomain::Caretaker => "見不得別人有難、總想搭把手的人",
            SelfDomain::Companion => "把身邊每段情誼都放在心上的人",
        }
    }
}

/// 至少累積這麼多筆同領域記憶，才可能昇華出自我印象（太少＝還看不出牠是個怎樣的人）。
pub const MIN_DOMAIN_MEMORIES: usize = 4;

/// 主導領域必須比第二名多出這麼多筆，才算「明顯」是牠的自我印象（防兩件事勢均力敵時搖擺）。
pub const LEAD_MARGIN: usize = 2;

/// 冷卻歸零後、每 tick（10Hz）真的開口說自我印象的機率（低頻，讓它是偶爾的溫柔）。
/// 真正有感的頻率由下方長冷卻主宰，本機率只讓觸發那一刻不機械地卡在整秒。
pub const SPEAK_CHANCE: f32 = 0.02;

/// 說出一次自我印象後的冷卻（秒）：15 分鐘，久久才再回望自己一次，不反覆碎念。
pub const SPEAK_COOLDOWN: f32 = 900.0;

/// 冷卻到、但當下還昇華不出明顯主導領域時的重試冷卻（秒）：4 分鐘後再看，
/// 避免每 tick 都去讀一次記憶鎖白忙。
pub const RETRY_COOLDOWN: f32 = 240.0;

/// 把一筆記憶摘要歸類到某個生活領域；歸不了類（寒暄／無關）→ `None`。
///
/// 命中優先序刻意由「最具體的作為」往「最泛的情緒」排：例如「幫朋友蓋家」同時含「幫」與「蓋」，
/// 歸給 [`SelfDomain::Builder`]（牠實際做的事）而非 [`SelfDomain::Caretaker`]——先判具體行為領域，
/// 情誼／待人之類的泛領域殿後兜底。確定性、可窮舉測試。
pub fn classify_domain(summary: &str) -> Option<SelfDomain> {
    const BUILDER: &[&str] = &["蓋", "建造", "搭建", "塔", "橋", "房子", "家園", "堆砌", "牆", "築"];
    const FARMER: &[&str] = &["種", "田", "農", "澆", "作物", "菜園", "播種", "收成", "幼苗", "耕"];
    const MINER: &[&str] = &["礦", "挖", "石頭", "洞", "晶", "礦坑", "鑿", "採石"];
    const ANGLER: &[&str] = &["釣", "垂釣", "魚", "湖", "水邊", "池畔"];
    const STARGAZER: &[&str] = &["星空", "星星", "觀星", "夜空", "月亮", "銀河", "星斗", "星辰"];
    const WANDERER: &[&str] = &["遠行", "荒野", "邊陲", "漂泊", "散居", "紮營", "旅途", "遠方"];
    const CARETAKER: &[&str] = &["幫", "照料", "送", "分享", "迎客", "招待", "心意", "餽贈", "搭把手"];
    const COMPANION: &[&str] = &["老朋友", "情誼", "相識", "探訪", "相聚", "想念", "惦記", "重逢", "敘舊"];

    // 具體作為領域優先（Builder→…→Stargazer→Wanderer），再落到待人／情誼泛領域殿後。
    let table: &[(&[&str], SelfDomain)] = &[
        (BUILDER, SelfDomain::Builder),
        (FARMER, SelfDomain::Farmer),
        (MINER, SelfDomain::Miner),
        (ANGLER, SelfDomain::Angler),
        (STARGAZER, SelfDomain::Stargazer),
        (WANDERER, SelfDomain::Wanderer),
        (CARETAKER, SelfDomain::Caretaker),
        (COMPANION, SelfDomain::Companion),
    ];
    for (kws, domain) in table {
        if kws.iter().any(|k| summary.contains(k)) {
            return Some(*domain);
        }
    }
    None
}

/// 統計居民全部記憶落在各領域的筆數，回傳「明顯主導」的那個領域及其筆數；
/// 沒有夠明顯的主導領域 → `None`（門檻見 [`MIN_DOMAIN_MEMORIES`] / [`LEAD_MARGIN`]）。
///
/// 「明顯」= 最多筆的領域 ≥ [`MIN_DOMAIN_MEMORIES`] 且比第二名多出 ≥ [`LEAD_MARGIN`] 筆。
/// 平手或差距不足 → `None`，寧可暫時「還看不出自己是誰」也不亂貼標籤。純函式、確定性。
pub fn dominant_domain(memories: &[MemoryEntry]) -> Option<(SelfDomain, usize)> {
    // 八個領域的計數（順序對齊 classify_domain 的表，供確定性 tie-break：同分時取表中較前者）。
    const DOMAINS: [SelfDomain; 8] = [
        SelfDomain::Builder,
        SelfDomain::Farmer,
        SelfDomain::Miner,
        SelfDomain::Angler,
        SelfDomain::Stargazer,
        SelfDomain::Wanderer,
        SelfDomain::Caretaker,
        SelfDomain::Companion,
    ];
    let mut counts = [0usize; 8];
    for e in memories {
        if let Some(d) = classify_domain(&e.summary) {
            // 窮舉映射到 DOMAINS 的索引：日後加變體時編譯器會在此報錯（漏配 match 臂），
            // 而非留到執行期 `position().unwrap()` 落空 panic 掉整條遊戲迴圈。
            let idx = match d {
                SelfDomain::Builder => 0,
                SelfDomain::Farmer => 1,
                SelfDomain::Miner => 2,
                SelfDomain::Angler => 3,
                SelfDomain::Stargazer => 4,
                SelfDomain::Wanderer => 5,
                SelfDomain::Caretaker => 6,
                SelfDomain::Companion => 7,
            };
            counts[idx] += 1;
        }
    }

    // 找最高分（同分取索引較小＝表中較前，確定性）與次高分。
    let mut best_idx = 0usize;
    for i in 1..counts.len() {
        if counts[i] > counts[best_idx] {
            best_idx = i;
        }
    }
    let best = counts[best_idx];
    if best < MIN_DOMAIN_MEMORIES {
        return None;
    }
    let second = counts
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != best_idx)
        .map(|(_, c)| *c)
        .max()
        .unwrap_or(0);
    if best < second + LEAD_MARGIN {
        return None; // 主導不夠明顯，先不貼標籤。
    }
    Some((DOMAINS[best_idx], best))
}

/// 日記頁頂端的「自我印象」句（第一人稱內心獨白，供 `voxel_diary` 呈現給訪客）。
/// `memories` 需最新在前（本函式只數領域、不看順序）。無明顯主導 → `None`。
///
/// **隱私鐵律**：輸出只由領域決定（固定模板），永不含任何記憶原文 / 玩家原話 / 玩家名。
pub fn self_impression(memories: &[MemoryEntry]) -> Option<String> {
    let (domain, _count) = dominant_domain(memories)?;
    Some(format!("不知不覺，我好像成了這一帶{}。", domain.epithet()))
}

/// 居民閒暇時**說出口**的自我印象泡泡（比日記獨白更口語一點）。無明顯主導 → `None`。
/// 面向玩家字串；長度由呼叫端再保險截一次（沿用泡泡 50 字上限）。
pub fn self_image_bubble(memories: &[MemoryEntry]) -> Option<String> {
    let (domain, _count) = dominant_domain(memories)?;
    let line = match domain {
        SelfDomain::Builder => "這陣子淨顧著東蓋西砌……我大概天生就是個閒不下手的人吧。",
        SelfDomain::Farmer => "低頭一看，指縫裡全是泥土。我這雙手啊，是離不開這片田了。",
        SelfDomain::Miner => "又想往洞裡鑽了。石頭與礦脈，好像才是我最自在的去處。",
        SelfDomain::Angler => "坐在水邊發著呆——我這人啊，總被這片水給留住。",
        SelfDomain::Stargazer => "又不自覺抬頭了。夜空這麼大，我的心思老是飄上去。",
        SelfDomain::Wanderer => "腳又癢了。這一帶再好，我這人終究待不住、總想往遠方走。",
        SelfDomain::Caretaker => "看到誰有難處就想搭把手——我好像天生就是這樣的人。",
        SelfDomain::Companion => "細數起來，這一帶的情誼我都記在心上……大概我是個重情的人吧。",
    };
    Some(line.to_string())
}

/// 動態牆（Feed）上的自我印象一句（旁白細節；居民名由 Feed 的 `resident` 欄另帶，**不重複**嵌名）。
/// 無明顯主導 → `None`。純模板、無記憶原文。
pub fn self_image_feed_line(memories: &[MemoryEntry]) -> Option<String> {
    let (domain, _count) = dominant_domain(memories)?;
    Some(format!("靜下來回望這一路，覺得自己成了{}。", domain.epithet()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem(summary: &str, seq: u64) -> MemoryEntry {
        MemoryEntry {
            resident: "vox_res_0".into(),
            player: "旅人".into(),
            summary: summary.into(),
            seq,
        }
    }

    #[test]
    fn classify_hits_each_domain() {
        assert_eq!(classify_domain("今天蓋了一座小屋"), Some(SelfDomain::Builder));
        assert_eq!(classify_domain("在田裡播種了幾株幼苗"), Some(SelfDomain::Farmer));
        assert_eq!(classify_domain("往礦坑深處挖了好久"), Some(SelfDomain::Miner));
        assert_eq!(classify_domain("在湖邊釣了一整個下午"), Some(SelfDomain::Angler));
        assert_eq!(classify_domain("抬頭看了好久的星空"), Some(SelfDomain::Stargazer));
        assert_eq!(classify_domain("獨自遠行到荒野邊陲"), Some(SelfDomain::Wanderer));
        assert_eq!(classify_domain("送了朋友一份心意"), Some(SelfDomain::Caretaker));
        assert_eq!(classify_domain("和老朋友敘舊了一番"), Some(SelfDomain::Companion));
    }

    #[test]
    fn classify_returns_none_for_unrelated() {
        assert_eq!(classify_domain("嗨，你好啊"), None);
        assert_eq!(classify_domain(""), None);
    }

    #[test]
    fn concrete_action_beats_generic_warmth() {
        // 「幫朋友蓋家」同時含「幫」(Caretaker) 與「蓋」(Builder)——具體作為優先。
        assert_eq!(classify_domain("幫朋友蓋了間房子"), Some(SelfDomain::Builder));
    }

    #[test]
    fn dominant_needs_min_and_lead() {
        // 三筆蓋、零其他：達領先但未達 MIN(4) → None。
        let few = vec![mem("蓋了塔", 1), mem("蓋了橋", 2), mem("蓋了牆", 3)];
        assert_eq!(dominant_domain(&few), None);

        // 四筆蓋、一筆種：達 MIN 且領先 3 ≥ LEAD_MARGIN → Builder。
        let mut clear = few.clone();
        clear.push(mem("又蓋了間房子", 4));
        clear.push(mem("順手種了株苗", 5));
        assert_eq!(dominant_domain(&clear), Some((SelfDomain::Builder, 4)));
    }

    #[test]
    fn dominant_none_when_tied_or_close() {
        // 四筆蓋、三筆種：領先只有 1 < LEAD_MARGIN(2) → None（不夠明顯）。
        let close = vec![
            mem("蓋了塔", 1),
            mem("蓋了橋", 2),
            mem("蓋了牆", 3),
            mem("蓋了屋", 4),
            mem("種了苗", 5),
            mem("種了花", 6),
            mem("澆了田", 7),
        ];
        assert_eq!(dominant_domain(&close), None);
    }

    #[test]
    fn dominant_tie_break_is_deterministic() {
        // 兩領域同為 5 筆：領先 0 < LEAD_MARGIN → None，永不亂選其一。
        let tie: Vec<MemoryEntry> = (0..5)
            .map(|i| mem("蓋了東西", i))
            .chain((5..10).map(|i| mem("種了東西", i)))
            .collect();
        assert_eq!(dominant_domain(&tie), None);
    }

    #[test]
    fn impression_and_bubble_present_only_when_dominant() {
        let clear: Vec<MemoryEntry> = (0..5).map(|i| mem("挖礦挖了一天", i)).collect();
        let imp = self_impression(&clear).expect("有主導領域應有印象");
        assert!(imp.contains("石頭") || imp.contains("洞"), "印象文字對應礦工領域：{imp}");
        assert!(self_image_bubble(&clear).is_some());
        assert!(self_image_feed_line(&clear).is_some());

        let vague: Vec<MemoryEntry> = vec![mem("嗨", 1), mem("你好", 2)];
        assert_eq!(self_impression(&vague), None);
        assert_eq!(self_image_bubble(&vague), None);
        assert_eq!(self_image_feed_line(&vague), None);
    }

    #[test]
    fn output_never_leaks_memory_text() {
        // 記憶摘要塞入可識別字串，輸出（固定模板）不得含之。
        let leaky: Vec<MemoryEntry> = (0..5)
            .map(|i| mem("蓋房子時聽旅人說了SECRET1234", i))
            .collect();
        let imp = self_impression(&leaky).unwrap();
        let bub = self_image_bubble(&leaky).unwrap();
        let feed = self_image_feed_line(&leaky).unwrap();
        for out in [&imp, &bub, &feed] {
            assert!(!out.contains("SECRET1234"), "自我印象洩漏記憶原文：{out}");
            assert!(!out.contains("旅人"), "自我印象洩漏玩家名：{out}");
        }
    }

    #[test]
    fn feed_line_is_nameless_detail() {
        // Feed detail 不自帶居民名（名字由 Feed 的 resident 欄另帶，避免重複）。
        let clear: Vec<MemoryEntry> = (0..5).map(|i| mem("遠行到荒野邊陲", i)).collect();
        let feed = self_image_feed_line(&clear).unwrap();
        assert!(feed.starts_with("靜下來"), "Feed detail 應為無名旁白：{feed}");
    }
}
