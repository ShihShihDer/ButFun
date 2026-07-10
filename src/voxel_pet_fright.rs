//! 乙太方界·臨危依偎 v1（voxel-pet-fright，
//! PLAN_ETHERVOX「玩家↔居民/生物的羈絆有後果」，ROADMAP 903）。
//!
//! **真缺口**：馴養羈絆線一路疊到了餵食馴服（851）、跟隨你走（851）、取名（895）、
//! 安置／召回（898）、餵零食蹭你（899）——但這一切至今都只發生在**白天的安寧裡**。
//! 世界另一頭，暗影生物 v1（`voxel_shadow`）早已在夜裡遠離燈火的暗處漂近，居民見了會
//! 嚇得冒泡逃回家（療癒底線：只怕、不掉血）。可是**你親手馴服、取了名、每天跟前跟後的
//! 小夥伴，夜裡撞見漂近的暗影時卻毫無反應**——牠照樣若無其事地閒晃、下蛋、踱步，彷彿那
//! 團黑影不存在。馴養的羈絆從沒在「危險」面前被考驗過，也就從沒真的有過「牠把你當庇護」
//! 的那一刻。
//!
//! **本刀**：把「馴服的小夥伴」接上「夜裡的暗影」——當一隻**已馴服**的兔／雞在夜裡撞見
//! 暗影漂進害怕半徑（沿用 `vshadow::FEAR_RADIUS`，與居民同一條驚嚇線），牠會嚇得**竄回
//! 最近玩家的腳邊依偎討庇護**（就算牠原本被你安置待命在別處，也會拔腿奔向你），頭頂冒起
//! 一枚受驚的表情，直到暗影走遠、驚魂稍定才慢慢鬆開。療癒底線一如既往：暗影**碰不到、也
//! 傷不了**你的小夥伴（純怕、不掉血）——牠只是需要你。馴養的羈絆第一次在危險面前有了
//! 意義：你蓋的燈火、你走的路，成了牠夜裡唯一敢靠的地方。
//!
//! **與既有元素的定位區隔**：
//! - **居民害怕暗影（`voxel_shadow` fear 反應）**是居民**逃回自己家**（各自歸巢躲牆）；
//!   本刀是馴服寵物**奔向玩家**（把你當庇護、往你身上靠），方向與依附對象截然不同。
//! - **馴服跟隨（851）**是白天沒事時、玩家在附近就悠哉跟著；本刀是**夜裡受驚**時、不論
//!   原本在跟隨／閒晃／安置待命，一律拋下手邊的事**急奔**回你腳邊（速度更急、貼得更近）。
//! - **餵零食蹭你（899）**是安寧時主動示好；本刀是危難時討庇護——一個是撒嬌、一個是求安全感。
//!
//! **純邏輯層**：本檔全是零 IO／零鎖／零 LLM／零 async 的確定性純函式（受驚計時推進、是否
//! 該依偎、受驚表情選字），可獨立窮舉單元測試。暗影偵測沿用 `vshadow::frightened_by`，實際
//! 依偎移動（奔向最近玩家、貼近停步）在 `voxel_ws.rs` 的 `tick_wildlife` 寵物分支落地。
//!
//! **成本／安全紀律**：零 LLM（判定＋表情皆確定性）、零 migration（受驚計時是純記憶體暫態，
//! 比照 `tamed`/`following`/`name` 同款 wildlife 重啟歸零慣例，不新增任何持久欄位）、零協議
//! 破壞（`WildlifeView` 只**新增**一個 `Option` 表情欄位，`None` 不送、向後相容）、零新美術
//! （表情＝既有名牌貼圖通道多掛一枚 emoji，不新增 draw call）、FPS 零影響（受驚偵測搭在原本
//! 就在跑的暗影 tick 上、僅在夜裡真有暗影時才掃數隻寵物一次）。
//! **濫用防護**：不收玩家輸入、不觸發 LLM、不開對外端點、不動帳號權限——純伺服器內部確定性
//! 反應，受驚與否由後端據暗影位置算出，玩家無從觸發、注入或洗版；表情皆內建常數。

/// 撞見暗影後，小夥伴維持受驚（奔向玩家依偎）的持續秒數。暗影只要還在害怕半徑內，
/// 每個暗影 tick 都會把這個計時「刷新」回滿；暗影走遠後才開始遞減、慢慢鬆開回歸日常。
pub const SPOOK_SECS: f32 = 6.0;

/// 受驚依偎時停步的距離（格，中心對中心）：比平常跟隨的 [`crate::voxel_wildlife::FOLLOW_STOP_DIST`]
/// 更近——害怕時會往你身上緊緊靠，而不是悠哉跟在兩三格外。
pub const HUDDLE_STOP_DIST: f32 = 1.3;

/// 受驚依偎時的奔跑速度（格/秒）：沿用野兔逃跑那般急促（[`crate::voxel_wildlife::FLEE_SPEED`]），
/// 「嚇得竄回你腳邊」該是拔腿奔、不是散步。集中為常數方便日後平衡微調。
pub const HUDDLE_SPEED: f32 = crate::voxel_wildlife::FLEE_SPEED;

/// 受驚時頭頂冒出的表情池（確定性輪替、皆為單一 emoji，掛在既有名牌通道、不新增 draw call）。
const SPOOK_EMOTES: &[&str] = &["😨", "😰", "🫣"];

/// 推進受驚計時：這一 tick 若還在暗影害怕半徑內就刷新回滿 [`SPOOK_SECS`]，否則遞減 `dt`（夾在 0 以上）。
/// 純函式——暗影是否在半徑內（`near_shadow`）由呼叫端用 `vshadow::frightened_by` 算好餵進來。
pub fn next_spook_secs(current: f32, near_shadow: bool, dt: f32) -> f32 {
    if near_shadow {
        SPOOK_SECS
    } else {
        (current - dt).max(0.0)
    }
}

/// 此刻是否正受驚依偎中（計時未歸零）——`tick_wildlife` 據此把寵物的移動覆寫成「奔向玩家貼近」。
pub fn is_spooked(spook_secs: f32) -> bool {
    spook_secs > 0.0
}

/// 受驚時該冒的表情（確定性選字，`pick` 取模輪替、對任意 `pick` 都有界不 panic）。
pub fn spook_emote(pick: usize) -> &'static str {
    SPOOK_EMOTES[pick % SPOOK_EMOTES.len()]
}

/// 受驚奔向玩家時，這一步是否還要再往玩家逼近（尚未貼到 [`HUDDLE_STOP_DIST`] 內就繼續奔）。
/// 與平常跟隨的 `should_close_follow_gap` 同手法，但用更近的依偎停步距離。
pub fn should_close_huddle_gap(player_dist_sq: f32) -> bool {
    player_dist_sq > HUDDLE_STOP_DIST * HUDDLE_STOP_DIST
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 受驚計時_見影刷滿離影遞減() {
        // 撞見暗影 → 不論原本剩多少，一律刷新回滿。
        assert_eq!(next_spook_secs(0.0, true, 0.1), SPOOK_SECS);
        assert_eq!(next_spook_secs(2.0, true, 0.1), SPOOK_SECS);
        assert_eq!(next_spook_secs(SPOOK_SECS, true, 0.1), SPOOK_SECS);
        // 暗影走遠 → 按 dt 遞減。
        let after = next_spook_secs(SPOOK_SECS, false, 0.1);
        assert!((after - (SPOOK_SECS - 0.1)).abs() < 1e-5);
        // 遞減夾在 0 以上，不會變負（避免永遠「受驚」或計時亂掉）。
        assert_eq!(next_spook_secs(0.05, false, 0.1), 0.0);
        assert_eq!(next_spook_secs(0.0, false, 0.1), 0.0);
    }

    #[test]
    fn 是否受驚_計時未歸零才算() {
        assert!(is_spooked(SPOOK_SECS));
        assert!(is_spooked(0.01));
        assert!(!is_spooked(0.0));
        assert!(!is_spooked(-1.0)); // 理論上不會出現（已夾 0），但防禦性驗證。
    }

    #[test]
    fn 受驚表情_非空且輪替有界() {
        let mut seen = std::collections::HashSet::new();
        for pick in 0..SPOOK_EMOTES.len() {
            let e = spook_emote(pick);
            assert!(!e.is_empty(), "表情不該為空");
            seen.insert(e);
        }
        assert_eq!(seen.len(), SPOOK_EMOTES.len(), "每個表情應相異（輪替有變化）");
        // 任意大 pick 取模不 panic、且與同餘者一致（有界輪替）。
        assert_eq!(spook_emote(SPOOK_EMOTES.len()), spook_emote(0));
        assert_eq!(spook_emote(999), spook_emote(999 % SPOOK_EMOTES.len()));
    }

    #[test]
    fn 依偎逼近_貼到停步距離內才停() {
        // 遠 → 還要繼續奔。
        assert!(should_close_huddle_gap((HUDDLE_STOP_DIST + 1.0).powi(2)));
        // 恰在停步距離上 → 不再逼近（貼夠近了）。
        assert!(!should_close_huddle_gap(HUDDLE_STOP_DIST * HUDDLE_STOP_DIST));
        // 更近 → 不逼近。
        assert!(!should_close_huddle_gap(0.0));
        // 依偎停步距離比平常跟隨更近（害怕時貼得更緊）。
        assert!(HUDDLE_STOP_DIST < crate::voxel_wildlife::FOLLOW_STOP_DIST);
    }
}
