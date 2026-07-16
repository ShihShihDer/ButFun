//! 乙太方界·騎乘馴養夥伴 v1（自主提案切片，ROADMAP 1021）——馴養羈絆線第一次接上「移動」。
//!
//! **真缺口**：馴養羈絆線疊了餵食馴服（847/870）→跟隨（851）→取名（895）→安置召回（898）
//! →寵愛（899）五刀，你天天帶在身邊、取了名、疼過的小夥伴，卻永遠只是徒步跟在你腳邊——
//! 「騎乘」這個動詞至今只屬於冰冷的機械載具（蒸汽獨輪車 976／木筏 1017）：同樣是移動代步，
//! 載具與你毫無感情，有生命、記得你的寵物卻只能陪跑，這是馴養軸線唯一還沒接上的一塊。
//!
//! **本刀補的**：已馴服的兔／雞（非魚——魚沒有陸上跟隨行為，游得滑溜騎不上）第一次能被
//! 「騎上去」，牠會貼在你身邊、精神抖擻地陪你飛奔，移動速度比純徒步快。比照 976 蒸汽獨輪車
//! ／1017 木筏同款「單鍵切換＋伺服器權威複驗」手法（`SetMounted`），零新持久化欄位——
//! `mounted_by` 純記憶體、重啟歸零（比照 `tamed`/`following`/`settled` 同款 wildlife 暫態慣例）。
//!
//! **與既有機械載具 razor-sharp 區隔**：機械載具騎乘只查背包持有（庫存物品，無生命）；
//! 本刀騎乘只查「這隻活的動物是不是你馴服的、有沒有人已經騎著牠、離得夠不夠近」，對象是
//! 有名字、記得被你疼過的小夥伴——情感重量不同，判定邏輯也刻意不同（零庫存消耗）。
//!
//! **純邏輯層**：可否騎乘的判定＋失敗原因＋暖句挑選皆為零 IO 純函式；鎖／廣播／位置貼合
//! 全在 `voxel_ws.rs`（`tick_wildlife` 依 `mounted_by` 把動物位置貼到騎乘者身邊）。

/// 騎乘觸及距離（沿用馴服／指揮同款近身尺標，見 `crate::voxel_wildlife::TAME_REACH`）。
pub const MOUNT_REACH: f32 = 3.0;

/// 騎乘中小夥伴貼在騎乘者身後的視覺偏移量（世界座標，純融合視覺、非玩法判定）。
pub const MOUNT_OFFSET: f32 = 0.4;

/// 暖句的字元上限（泡泡框友善，比照 `voxel_pettreat::TREAT_LINE_MAX_CHARS` 同款慣例）。
pub const MOUNT_LINE_MAX_CHARS: usize = 42;

/// 依騎乘者朝向算出小夥伴貼身的偏移量（世界座標 x/z）。與 `voxel_residents::yaw_from_move`
/// 的朝向慣例對齊（`yaw = atan2(dx,dz)` ⇒ 前向量 = `(sin(yaw), cos(yaw))`），取反向即為身後。
pub fn mount_offset(yaw: f32) -> (f32, f32) {
    (-yaw.sin() * MOUNT_OFFSET, -yaw.cos() * MOUNT_OFFSET)
}

/// 能不能騎上這隻動物？任一條件不成立回傳失敗原因；全過回 `None`（可以騎）。
/// 濫用防護：不信客戶端自報——已馴服／非魚／沒被別人騎走／在觸及範圍內，四項都由呼叫端
/// 用真實 `WildlifeAnimal` 欄位查出才傳進來，前端準心提示只是手感，伺服器仍權威複驗。
pub fn mount_fail_reason(
    tamed: bool,
    is_fish: bool,
    already_mounted: bool,
    dist_sq: f32,
) -> Option<&'static str> {
    if is_fish {
        Some("魚游得滑溜溜，騎不上去。")
    } else if !tamed {
        Some("牠還沒被馴服，先餵牠一份零食吧。")
    } else if already_mounted {
        Some("已經有人騎著牠了。")
    } else if dist_sq > MOUNT_REACH * MOUNT_REACH {
        Some("走近一點才騎得上牠。")
    } else {
        None
    }
}

/// [`mount_fail_reason`] 的布林版本，方便呼叫端只想要 true/false 時用。
pub fn can_mount(tamed: bool, is_fish: bool, already_mounted: bool, dist_sq: f32) -> bool {
    mount_fail_reason(tamed, is_fish, already_mounted, dist_sq).is_none()
}

/// 上騎那一刻的暖句（確定性挑選，零 LLM）。`pet_name` 沿用既有「未命名以小夥伴泛稱」慣例。
pub fn mount_line(is_rabbit: bool, pet_name: Option<&str>, pick: usize) -> String {
    let name = pet_name
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("小夥伴");
    let pool: &[&str] = if is_rabbit {
        &[
            "🐇 你跨上{name}的背，牠精神抖擻地載你飛奔起來！",
            "🐇 {name}蹲低身子讓你騎上，長耳朵隨奔跑一晃一晃。",
            "🐇 你穩穩坐上{name}背上，牠邁開腿飛快跑了起來。",
        ]
    } else {
        &[
            "🐔 你跨坐上{name}的背，牠拍了拍翅膀載你小跑起來。",
            "🐔 {name}讓你騎上牠背，咕咕叫著邁開步子。",
            "🐔 你騎上{name}，牠精神抖擻地小跑帶你前進。",
        ]
    };
    pool[pick % pool.len()].replace("{name}", name)
}

/// 下馬那一刻的暖句。
pub fn dismount_line(is_rabbit: bool, pet_name: Option<&str>, pick: usize) -> String {
    let name = pet_name
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("小夥伴");
    let pool: &[&str] = if is_rabbit {
        &[
            "🐇 你從{name}背上下來，牠甩了甩耳朵繼續在你身邊晃悠。",
            "🐇 {name}停下腳步讓你下馬，喘了口氣蹭了蹭你。",
        ]
    } else {
        &[
            "🐔 你從{name}背上下來，牠拍了拍翅膀，恢復悠閒踱步。",
            "🐔 {name}停下讓你下馬，咕咕叫著抖了抖羽毛。",
        ]
    };
    pool[pick % pool.len()].replace("{name}", name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mount_fail_reason_fish_always_blocked() {
        // 魚不論馴服/距離一律不能騎——即使緊貼身邊、即使（假設性地）已標記馴服。
        assert_eq!(
            mount_fail_reason(true, true, false, 0.0),
            Some("魚游得滑溜溜，騎不上去。")
        );
        assert!(!can_mount(true, true, false, 0.0));
    }

    #[test]
    fn mount_fail_reason_untamed_blocked() {
        assert!(mount_fail_reason(false, false, false, 0.0).is_some());
        assert!(!can_mount(false, false, false, 0.0));
    }

    #[test]
    fn mount_fail_reason_already_mounted_blocked() {
        assert!(mount_fail_reason(true, false, true, 0.0).is_some());
        assert!(!can_mount(true, false, true, 0.0));
    }

    #[test]
    fn mount_fail_reason_distance_boundary() {
        let edge = MOUNT_REACH * MOUNT_REACH;
        assert!(
            mount_fail_reason(true, false, false, edge - 0.01).is_none(),
            "略近於 MOUNT_REACH 應騎得上"
        );
        assert!(
            mount_fail_reason(true, false, false, edge + 0.01).is_some(),
            "略遠於 MOUNT_REACH 不該騎得上"
        );
    }

    #[test]
    fn can_mount_matches_fail_reason() {
        for &tamed in &[true, false] {
            for &is_fish in &[true, false] {
                for &already in &[true, false] {
                    for &d in &[0.0, 100.0] {
                        assert_eq!(
                            can_mount(tamed, is_fish, already, d),
                            mount_fail_reason(tamed, is_fish, already, d).is_none()
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn mount_offset_faces_behind_rider() {
        // yaw=0（面向 +z）：前向量=(0,1) ⇒ 偏移應在 -z 方向、x 分量近 0。
        let (ox, oz) = mount_offset(0.0);
        assert!(ox.abs() < 1e-4, "yaw=0 時 x 偏移應近 0，實際 {ox}");
        assert!(oz < 0.0, "yaw=0 時應偏在身後（-z），實際 {oz}");
        // 偏移量大小恆為 MOUNT_OFFSET（純旋轉，長度不變）。
        let mag = (ox * ox + oz * oz).sqrt();
        assert!((mag - MOUNT_OFFSET).abs() < 1e-4, "偏移量應恆為 MOUNT_OFFSET，實際 {mag}");
    }

    #[test]
    fn mount_line_is_deterministic_and_cycles() {
        for &is_rabbit in &[true, false] {
            let a = mount_line(is_rabbit, Some("小星"), 0);
            let b = mount_line(is_rabbit, Some("小星"), 0);
            assert_eq!(a, b, "同輸入必同輸出");
            let wrapped = mount_line(is_rabbit, Some("小星"), 3);
            let head = mount_line(is_rabbit, Some("小星"), 0);
            assert_eq!(wrapped, head, "pick=3 應循環回池頭（池長 3）");
        }
    }

    #[test]
    fn mount_line_injects_pet_name() {
        let s = mount_line(true, Some("雪球"), 1);
        assert!(s.contains("雪球"), "上騎句該喊出寵物名：{s}");
        assert!(!s.contains("{name}"), "佔位符必須被替換乾淨：{s}");
    }

    #[test]
    fn mount_line_falls_back_when_unnamed() {
        for pet in [None, Some(""), Some("   ")] {
            let s = mount_line(false, pet, 2);
            assert!(s.contains("小夥伴"), "未命名應以小夥伴泛稱：{s}");
            assert!(!s.contains("{name}"), "佔位符必須被替換乾淨：{s}");
        }
    }

    #[test]
    fn dismount_line_is_deterministic_and_names() {
        let a = dismount_line(true, Some("雪球"), 0);
        let b = dismount_line(true, Some("雪球"), 0);
        assert_eq!(a, b);
        assert!(a.contains("雪球"));
        let unnamed = dismount_line(false, None, 1);
        assert!(unnamed.contains("小夥伴"));
    }

    #[test]
    fn lines_stay_within_bubble_cap() {
        for &is_rabbit in &[true, false] {
            for pick in 0..6 {
                let m = mount_line(is_rabbit, Some("小星"), pick);
                let d = dismount_line(is_rabbit, Some("小星"), pick);
                assert!(
                    m.chars().count() <= MOUNT_LINE_MAX_CHARS,
                    "上騎句超出泡泡上限：{m}"
                );
                assert!(
                    d.chars().count() <= MOUNT_LINE_MAX_CHARS,
                    "下馬句超出泡泡上限：{d}"
                );
            }
        }
    }
}
