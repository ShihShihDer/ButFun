//! 乙太方界·居民早上會把昨晚的夢說給你聽 v1（voxel-dreamshare，記憶驅動行為）。
//!
//! **設計依據**：做夢 v1（805，`voxel_dream`）讓居民熟睡中偶爾從整座記憶庫裡挑一段珍貴往事
//! 浮成夢——冒「💤 夢見…」泡泡、記進城鎮動態。但那個夢只活在**夜裡那一刻**：夜裡路過的玩家
//! 瞥見一眼，天亮居民一醒，那個夢就跟著散了、再沒有下文。世界的睡夢少了最後一拍「醒來之後」
//! 的回響——她夢見了什麼，白天卻從不會**主動告訴你**。
//!
//! 本模組把那一拍補上：**夜裡做過夢的居民，白天遇到你時，偶爾會主動把昨晚的夢說給你聽**——
//! 「阿光，我昨晚做了個夢，夢見……和奧瑞一起整地呢。」那個原本只在夜裡孤零零浮現、你頂多
//! 隔著窗瞥一眼的夢，第一次被她**主動分享**、讓你走進她的內心；而「把夢說給你聽」這份親近
//! 本身，也記進她對你的記憶、讓交情更深一層。正中 PLAN_ETHERVOX 核心信念「**記憶要驅動行為、
//! 你的互動真的有後果**」：昨晚那個夢（805）不再是夜裡一閃即逝的孤景，它在隔天成了她主動
//! 對你開口的緣由——記憶不只在睡夢裡浮現，還驅動了她醒來後對你說的第一句話。
//!
//! **與既有系統的分界（razor-sharp，換維度不同軸）**：
//! - **做夢 805**（`voxel_dream`）：夜裡、熟睡中、**獨自**浮現的孤景（居民自己都不知道自己在夢），
//!   玩家只能被動旁觀。本刀：白天、清醒時、**主動說給你聽**的分享——夜的孤景第一次有了白天的
//!   回響、有了聽眾。時段（夜／日）、意識（潛意識／清醒）、對象（獨自／對你）皆不同。
//! - **主動聊心事 781**（`voxel_confide`）：分享的是**當前的渴望**（`voxel_desires`，朝前看「我想…」），
//!   且只對好感達門檻的熟人開口。本刀：分享的是**昨晚那個具體的夢**（源自 805、朝後看「我夢見了…」），
//!   因果上綁在「昨晚真的做了那個夢」上（沒做夢就沒得說）——夢是輕盈奇妙的、樂於與人分享，
//!   故不設好感門檻、遇到你就可能說；心事是私密的、要熟才掏。內容、朝向、門檻皆不同。
//! - **就寢反思 744 / 老友問候 675**：一在睡前有意識回味今天、一在招呼時回憶你我做過的事；
//!   本刀著眼「昨晚我夢見的那件事」——同是記憶驅動的一句話，緣由各不相同。
//!
//! **純邏輯層**：是否開口（[`should_share_dream`]）、把夢包成一句分享（[`dreamshare_line`]）、
//! 分享後記進記憶的摘要（[`dreamshare_memory_line`]）、動態牆一句（[`dreamshare_feed_line`]）
//! 全是確定性純函式，零 LLM、零鎖、零 IO。冷卻計時 / 夢的暫存 / 記憶寫入全在 `voxel_ws.rs`，
//! 沿用既有招呼 / 掏心那條已驗證的短鎖循序（守死鎖鐵律）。
//!
//! **成本 / 濫用防護**：句子全走固定模板，只包住居民**自己已抽象過的夢核心**（源自 805，本就是
//! episodic/persistent 記憶摘要、由記憶端截過長，**永不夾帶玩家原話**——無注入 / NSFW 風險）
//! 與玩家顯示名；不觸發 LLM、不開對外端點、不動帳號權限。每位居民 [`DREAMSHARE_COOLDOWN_SECS`]
//! 的長冷卻 ＋ 分享後即清空那個夢（`last_dream`）＋ 夜裡本就稀有的做夢頻率 ＝ 天然防洗版、也防
//! 好感（記憶筆數）被刷爆。訪客（名字空白）不記交情。零 migration、零新協議欄位、零前端改動、
//! 零新美術、FPS 零影響（純後端、僅招呼時序偶發、無新尋路無新實體）。

/// 分享夢泡的字元上限（與泡泡框上限一致，超出截斷不破框）。
pub const DREAMSHARE_MAX_CHARS: usize = 40;

/// 嵌進分享泡泡 / 記憶 / Feed 的「夢核心」字元上限（比照 `voxel_dream::DREAM_CORE_CHARS`）。
pub const DREAMSHARE_CORE_CHARS: usize = 16;

/// 同一位居民主動分享夢的冷卻（秒）。設得長（200s）——分享夢是偶爾為之的溫暖一刻，不是每次
/// 靠近都說；配合「分享後即清空那個夢」與夜裡本就稀有的做夢頻率，稀有才有份量、天然防洗版。
pub const DREAMSHARE_COOLDOWN_SECS: f32 = 200.0;

/// 靠近時、冷卻已過、懷著昨晚的夢，這一 tick 主動開口分享的機率（tick 為 10Hz）。
/// 設得低——即使她懷著夢、你就站在旁邊，也是偶爾才說出口，讓這一刻自然而不機械。
pub const DREAMSHARE_CHANCE_PER_TICK: f32 = 0.03;

/// 把夢核心去頭尾空白 + 截到 [`DREAMSHARE_CORE_CHARS`]（比照 `voxel_dream::trim_core`）。
fn trim_core(dream_core: &str) -> String {
    dream_core
        .trim()
        .chars()
        .take(DREAMSHARE_CORE_CHARS)
        .collect()
}

/// 判斷此刻是否要主動把昨晚的夢說給你聽：懷著夢 ＋ 靠得夠近 ＋ 冷卻到期 ＋ 過了機率門檻。
///
/// 純函式、確定性（機率骰由呼叫端傳入）。刻意**不設好感門檻**（夢是輕盈樂於分享的，遇到你
/// 就可能說）——這是與 781 主動聊心事（私密、要熟才掏）的關鍵區隔。
pub fn should_share_dream(has_dream: bool, near: bool, cooldown_ok: bool, roll: f32, chance: f32) -> bool {
    has_dream && near && cooldown_ok && roll < chance
}

/// 把昨晚那個夢的核心，包成一句「主動說給你聽」的分享泡泡。
///
/// 依 `pick` 在幾組固定語氣模板間確定性輪替；夢核心原封放進模板（已由記憶端 / 805 抽象截過長、
/// 不含玩家原話）。整句以字元為單位截到 [`DREAMSHARE_MAX_CHARS`] 內，永不破泡泡框、永不回空。
///
/// `player` 為玩家顯示名（可能空——呼叫端保證只對非空名分享，此處空名仍保守落回不嵌名的模板）。
pub fn dreamshare_line(dream_core: &str, player: &str, pick: usize) -> String {
    let core = trim_core(dream_core);
    let p = player.trim();
    // 夢核心空掉（理論上呼叫端已濾）→ 落回一句不倚賴內容的通用分享，仍是「主動說夢」的味道。
    let line = if core.is_empty() {
        match pick % 3 {
            0 => "我昨晚做了個好夢呢，醒來心裡都是暖的。".to_string(),
            1 => "昨晚睡得真好，還做了個溫柔的夢。".to_string(),
            _ => "昨晚的夢真教人捨不得醒來呀。".to_string(),
        }
    } else if p.is_empty() {
        match pick % 3 {
            0 => format!("我昨晚做了個夢，夢見……{core}呢。"),
            1 => format!("昨晚睡著後，{core}的畫面又回到夢裡了。"),
            _ => format!("跟你說，我昨晚夢見了……{core}。"),
        }
    } else {
        match pick % 3 {
            0 => format!("{p}，我昨晚做了個夢，夢見……{core}呢。"),
            1 => format!("{p}，昨晚睡著後，{core}又回到我夢裡了。"),
            _ => format!("跟你說呀{p}，我昨晚夢見了……{core}。"),
        }
    };
    line.chars().take(DREAMSHARE_MAX_CHARS).collect()
}

/// 「把昨晚的夢說給你聽」記進居民對這位玩家的記憶摘要（episodic，累積好感、不夾帶夢原文）。
///
/// 刻意停在情節層（只述「我把夢分享給對方」這件事本身，不放夢核心內容）——與 781 掏心記憶同款
/// 隱私姿態：記憶記的是「我們親近了一點」，不是把夢的內容再存一份。
pub fn dreamshare_memory_line(player: &str) -> String {
    let p = player.trim();
    if p.is_empty() {
        "我把昨晚做的夢，說給了一位路過的旅人聽，心裡覺得親近了些。".to_string()
    } else {
        format!("我把昨晚做的夢，說給了{p}聽，心裡覺得跟對方親近了些。")
    }
}

/// 分享夢寫進城鎮動態 Feed 的一句（讓非同步回訪的玩家也讀到「某居民把夢分享給了某人」）。
pub fn dreamshare_feed_line(resident: &str, player: &str) -> String {
    let r = resident.trim();
    let p = player.trim();
    let r = if r.is_empty() { "一位居民" } else { r };
    if p.is_empty() {
        format!("{r}把昨晚做的夢，說給了路過的旅人聽。")
    } else {
        format!("{r}把昨晚做的夢，說給了{p}聽。")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_share_needs_all_gates() {
        // 全部條件滿足 → 分享。
        assert!(should_share_dream(true, true, true, 0.0, DREAMSHARE_CHANCE_PER_TICK));
        // 缺任一 → 不分享。
        assert!(!should_share_dream(false, true, true, 0.0, DREAMSHARE_CHANCE_PER_TICK)); // 沒夢
        assert!(!should_share_dream(true, false, true, 0.0, DREAMSHARE_CHANCE_PER_TICK)); // 不夠近
        assert!(!should_share_dream(true, true, false, 0.0, DREAMSHARE_CHANCE_PER_TICK)); // 冷卻沒到
        // 機率門檻：roll 達門檻（含）不觸發、低於才觸發（嚴格小於）。
        assert!(!should_share_dream(true, true, true, DREAMSHARE_CHANCE_PER_TICK, DREAMSHARE_CHANCE_PER_TICK));
        assert!(should_share_dream(true, true, true, DREAMSHARE_CHANCE_PER_TICK - 0.001, DREAMSHARE_CHANCE_PER_TICK));
    }

    #[test]
    fn share_line_embeds_name_and_core_and_is_bounded() {
        let core = "和奧瑞一起把那片地整平了";
        let a = dreamshare_line(core, "阿光", 0);
        let b = dreamshare_line(core, "阿光", 1);
        let c = dreamshare_line(core, "阿光", 2);
        // 三組模板確定性輪替、彼此不同。
        assert_ne!(a, b);
        assert_ne!(b, c);
        for line in [&a, &b, &c] {
            assert!(!line.is_empty());
            assert!(line.chars().count() <= DREAMSHARE_MAX_CHARS);
            assert!(line.contains("阿光"), "應嵌玩家名");
        }
        // 至少一組把夢核心嵌了進去（截斷後開頭仍在）。
        assert!(a.contains("整") || b.contains("整") || c.contains("整"));
        // pick 取模不越界。
        let _ = dreamshare_line(core, "阿光", usize::MAX);
    }

    #[test]
    fn long_core_is_trimmed_and_line_stays_bounded() {
        let long: String = "很".repeat(200);
        for pick in 0..3 {
            let line = dreamshare_line(&long, "諾瓦", pick);
            assert!(line.chars().count() <= DREAMSHARE_MAX_CHARS, "超長夢核心仍不破泡泡框");
            assert!(!line.is_empty());
        }
    }

    #[test]
    fn empty_core_falls_back_to_generic_share() {
        // 空 / 全空白夢核心 → 落回通用分享句，仍非空、仍在上限、不 panic。
        for c in ["", "   ", "　"] {
            for pick in 0..3 {
                let line = dreamshare_line(c, "阿光", pick);
                assert!(!line.is_empty());
                assert!(line.chars().count() <= DREAMSHARE_MAX_CHARS);
            }
        }
    }

    #[test]
    fn empty_player_name_falls_back_without_panicking() {
        // 訪客名空（呼叫端本會濾掉，但純函式仍要安全）→ 落回不嵌名模板、不 panic。
        let line = dreamshare_line("和奧瑞一起整地", "", 0);
        assert!(!line.is_empty());
        assert!(line.chars().count() <= DREAMSHARE_MAX_CHARS);
        let line2 = dreamshare_line("和奧瑞一起整地", "   ", 1);
        assert!(!line2.is_empty());
    }

    #[test]
    fn memory_line_embeds_name_but_not_dream_content() {
        let m = dreamshare_memory_line("諾瓦");
        assert!(m.contains("諾瓦"), "記憶應含玩家名");
        assert!(!m.is_empty());
        // 記憶刻意停在情節層、不夾帶夢核心內容（隱私姿態同 781 掏心）。
        assert!(!m.contains("整地"));
        // 空名落回不 panic、仍非空。
        let m2 = dreamshare_memory_line("");
        assert!(!m2.is_empty());
    }

    #[test]
    fn feed_line_embeds_names_and_handles_empty() {
        let f = dreamshare_feed_line("露娜", "阿光");
        assert!(f.contains("露娜"));
        assert!(f.contains("阿光"));
        // 空居民名 / 空玩家名皆落回通用稱呼、不 panic、非空。
        let f2 = dreamshare_feed_line("", "");
        assert!(!f2.is_empty());
        assert!(f2.contains("一位居民"));
        let f3 = dreamshare_feed_line("露娜", "");
        assert!(f3.contains("露娜"));
        assert!(!f3.is_empty());
    }
}
