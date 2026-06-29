//! 故鄉茶棚系統（ROADMAP 641，禱告驅動）。
//!
//! **緣起**：AI 居民「露娜」反覆向世界禱告——「盼能在街角小攤買杯熱茶，暖和一下疲憊的身心」
//! 「願今晚的市場能熱鬧點，讓我找點好吃的補充活力」「願今晚的集市有熱鬧的表演，讓我振作起來」
//! 「願今晚的星光指引我在市集找到新朋友」（見 `data/prayers.jsonl`，是露娜出現最多次的願望）。
//! 造世界的 AI 裁決這份願望合乎世界、對居民好、也療癒，於是在市集旁立起一座**故鄉茶棚**作為回應。
//!
//! **效用**：茶棚每隔 `TEA_INTERVAL` 秒自動「出爐一壺熱茶」，給全鎮 NPC 一小份**歸屬暖意**
//! （`NpcNeedsState::warm_community`，小幅回暖歸屬感）。這正是露娜祈願的「街角熱茶暖身、市集
//! 熱鬧讓人振作、在市集找到新朋友」——一盞熱茶把疏離的人心稍稍拉近。純正向、零懲罰，幅度刻意
//! 比整場村慶（`VillageFestival`）輕得多：它只是日常的一盞熱茶，不是節慶。
//!
//! **成本紀律**：茶棚是世界固定設施（位置由常數決定），**零持久化、零 migration、零 LLM、
//! 零經濟**（不產出任何物品／乙太，只給既有 NPC 一點歸屬暖意，與古井只滋潤既有作物同性質）。

use crate::state::{PUB_FIELD_ORIGIN_X, PUB_FIELD_ORIGIN_Y};

/// 出爐間隔（秒）：每隔這麼久茶棚自動出一壺熱茶、給全鎮一份歸屬暖意。
/// 取一個比古井（40s）略慢的節律——它免費、無人看管、惠及全鎮，節奏放緩較不失衡。
pub const TEA_INTERVAL: f32 = 50.0;

/// 出爐後「熱氣蒸騰」的視覺窗（秒）：剛出爐的這段時間，前端在茶棚上方畫嫋嫋蒸汽。
/// 取一個略寬的窗（>一個廣播節律）讓蒸汽在快照節流下仍穩定可見，不會一閃即逝。
pub const TEA_STEAM_SECS: f32 = 5.0;

/// 茶棚的世界座標（像素）。立在公共農田（origin 2200,2200，6×4 格 ×48px）右緣外側一點，
/// 與立在田左緣的古井遙遙相對，像市集一角真有一座熱茶攤——`PUB_FIELD_ORIGIN_*` 是田左上角，
/// 這裡取右緣外、縱向置中（6 欄 ×48＝288 寬）。
pub const TEA_X: f32 = PUB_FIELD_ORIGIN_X + 288.0 + 44.0;
pub const TEA_Y: f32 = PUB_FIELD_ORIGIN_Y + 96.0;

/// 一座故鄉茶棚的執行期狀態（記憶體模式，重啟由常數重新立起，無存檔）。
#[derive(Debug, Clone)]
pub struct VillageTeaStall {
    /// 距下次出爐還剩幾秒。
    cooldown: f32,
    /// 距上次出爐已過幾秒（`f32::INFINITY` 表示從未出爐——剛開服時不畫蒸汽）。
    since_brewed: f32,
}

impl Default for VillageTeaStall {
    fn default() -> Self {
        Self {
            // 開服即進入第一個間隔倒數（不一上線就出爐，給市集一點甦醒的起頭）。
            cooldown: TEA_INTERVAL,
            since_brewed: f32::INFINITY,
        }
    }
}

impl VillageTeaStall {
    pub fn new() -> Self {
        Self::default()
    }

    /// 推進 `dt` 秒；倒數到 0 時出一壺熱茶，重置倒數並記下「剛出爐」、回傳 `true`。
    /// 回傳 `false`＝還沒到出爐時刻。實際的歸屬暖意由呼叫端（game loop）施加到 `NpcNeedsState`，
    /// 讓本結構保持單純（只管計時），與古井把澆水委給 `Field` 同理、易測。
    /// 壞 dt（負/NaN）夾成 0 不推進，確定性、好測。
    pub fn tick(&mut self, dt: f32) -> bool {
        let dt = if dt.is_finite() && dt > 0.0 { dt } else { 0.0 };
        // 累積「距上次出爐」（封頂於蒸汽窗 + 1，避免浮點無限增長；只需區分窗內／窗外）。
        if self.since_brewed.is_finite() {
            self.since_brewed = (self.since_brewed + dt).min(TEA_STEAM_SECS + 1.0);
        }
        self.cooldown -= dt;
        if self.cooldown > 0.0 {
            return false;
        }
        self.cooldown = TEA_INTERVAL;
        self.since_brewed = 0.0;
        true
    }

    /// 是否剛出過爐（位於蒸汽視覺窗內）——供前端在茶棚上方畫嫋嫋蒸汽。純讀狀態、好測。
    pub fn recently_brewed(&self) -> bool {
        self.since_brewed.is_finite() && self.since_brewed < TEA_STEAM_SECS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn does_not_brew_before_interval() {
        let mut stall = VillageTeaStall::new();
        // 還沒到間隔：不出爐。
        let brewed = stall.tick(TEA_INTERVAL - 1.0);
        assert!(!brewed, "未到出爐間隔不應出爐");
        assert!(!stall.recently_brewed(), "從未出爐不應顯示蒸汽");
    }

    #[test]
    fn brews_at_interval() {
        let mut stall = VillageTeaStall::new();
        let brewed = stall.tick(TEA_INTERVAL);
        assert!(brewed, "到出爐間隔應出爐");
        assert!(stall.recently_brewed(), "剛出爐應在蒸汽窗內");
    }

    #[test]
    fn steam_window_expires() {
        let mut stall = VillageTeaStall::new();
        stall.tick(TEA_INTERVAL); // 出爐
        assert!(stall.recently_brewed());
        // 過了蒸汽窗：不再顯示蒸汽（剛好不再跨過下一個出爐間隔）。
        let brewed = stall.tick(TEA_STEAM_SECS + 0.5);
        assert!(!brewed, "蒸汽窗這一步還沒到下一次出爐");
        assert!(!stall.recently_brewed(), "超過蒸汽窗應停止顯示蒸汽");
    }

    #[test]
    fn bad_dt_does_not_advance() {
        let mut stall = VillageTeaStall::new();
        let b1 = stall.tick(f32::NAN);
        let b2 = stall.tick(-5.0);
        assert!(!b1 && !b2, "壞 dt 不推進、不出爐");
        assert!(!stall.recently_brewed(), "壞 dt 不顯示蒸汽");
    }

    #[test]
    fn repeated_cycles_keep_brewing() {
        // 連續兩個間隔：每輪都會再出爐一次（茶棚長期溫暖市集）。
        let mut stall = VillageTeaStall::new();
        assert!(stall.tick(TEA_INTERVAL), "第一輪出爐");
        // 再推進一個完整間隔，應再次出爐。
        assert!(stall.tick(TEA_INTERVAL), "第二輪再次出爐——茶棚長期溫暖市集");
    }

    #[test]
    fn stall_sits_to_the_right_of_the_well() {
        // 茶棚立在公田右緣外、古井立在左緣外——兩者分立田的兩側，不重疊。
        assert!(TEA_X > PUB_FIELD_ORIGIN_X + 288.0, "茶棚應在公田右緣外");
        assert!(TEA_X > crate::village_well::WELL_X, "茶棚應在古井（田左側）的右邊");
    }
}
