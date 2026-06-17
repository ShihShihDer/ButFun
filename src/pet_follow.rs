//! 寵物跟隨物理（ROADMAP 343「寵物現身相伴」的純邏輯層）。
//!
//! 寵物自 ROADMAP 46 上線至今，一直只是「黏在主人名牌旁的一枚靜態 emoji ＋一份隱形被動加成」——
//! 牠沒有身體、沒有座標、不存在於世界裡。本模組給寵物第一個真實行為：擁有自己的座標、像隻忠犬一樣
//! 跟著主人在世界裡跑動。延續 `gather.rs` / `combat.rs` 的慣例：這層是純資料 ＋ 純函式，無 IO、
//! 不碰 WebSocket / 遊戲迴圈，由 `game.rs` 每 tick 餵呼叫、把回傳寫回 `Player::{pet_x, pet_y}`。
//!
//! 跟隨手感刻意「像隻黏人的小夥伴」：主人停下時，寵物會小跑到腳邊歇著（停在 `FOLLOW_STOP` 環上，
//! 不貼著主人重疊、也不在原地抖動）；主人走動時，寵物在後頭追；主人全力衝刺（比寵物快）時，寵物會
//! 暫時落在後頭，等主人放慢就追回來——讀起來就像真的有隻夥伴跟著你。主人瞬移（換星球 / 重生 / 衝太遠）
//! 時，寵物直接出現在主人身邊，不慢吞吞橫越整個世界。
//!
//! 主題是療癒的蒸汽龐克太空歌劇，寵物是被安撫馴服的野化乙太生靈，跟在身邊作伴——「現身相伴」本身
//! 就是獲得感，與既有的被動加成正交（加成管數值、跟隨管陪伴）。

/// 寵物歇腳時與主人保持的距離（px）。在這個圈內就停下歇著，不貼著主人重疊、也不抖動。
pub const FOLLOW_STOP: f32 = 30.0;

/// 寵物的追趕速度（px/s）。比走路基速（`PLAYER_SPEED` 180）快、比衝刺（×1.6＝288）慢——
/// 故主人走路時寵物輕鬆跟上、停下時小跑到腳邊，全力衝刺時暫時落後、等主人放慢再追回。
pub const FOLLOW_SPEED: f32 = 240.0;

/// 瞬移門檻（px）。主人與寵物相距超過這個距離（換星球 / 重生 / 衝太遠），寵物直接出現在主人身邊，
/// 不慢吞吞橫越世界。
pub const FOLLOW_SNAP: f32 = 600.0;

/// 推進一步寵物跟隨：給定寵物當前座標、主人當前座標、時間增量 `dt`（秒），
/// 回傳寵物的新座標與「這一步是否在移動」（供前端判斷要不要播走路彈跳）。
///
/// 用預設歇腳距離 `FOLLOW_STOP`（無性格 / 預設手感）。純函式、無狀態、結果確定可重現。
pub fn follow_step(pet: (f32, f32), owner: (f32, f32), dt: f32) -> (f32, f32, bool) {
    follow_step_with_stop(pet, owner, dt, FOLLOW_STOP)
}

/// 同 `follow_step`，但可指定歇腳距離 `stop`（px）——供 ROADMAP 358 寵物性格用：黏人貼最近、
/// 慵懶／好奇愛在後頭，由性格決定停在離主人多近的環上。`stop` 由呼叫端保證為正、合理範圍內。
///
/// 純函式、無狀態、結果確定可重現（相同輸入必得相同輸出），便於自動測試。
pub fn follow_step_with_stop(
    pet: (f32, f32),
    owner: (f32, f32),
    dt: f32,
    stop: f32,
) -> (f32, f32, bool) {
    let dx = owner.0 - pet.0;
    let dy = owner.1 - pet.1;
    let dist = (dx * dx + dy * dy).sqrt();

    // 主人瞬移（換星球 / 重生 / 衝太遠）→ 寵物直接出現在主人身邊。
    if dist > FOLLOW_SNAP {
        return (owner.0, owner.1, false);
    }

    // 已在歇腳圈內 → 待在原地歇著（不貼著主人重疊、也不抖動）。
    if dist <= stop {
        return (pet.0, pet.1, false);
    }

    // 朝主人移動，但停在歇腳環上（不蓋住主人），單幀位移受 `FOLLOW_SPEED` 上限約束、
    // 且絕不越過歇腳環（`dist - stop`）。
    let step = (FOLLOW_SPEED * dt).min(dist - stop).max(0.0);
    let inv = 1.0 / dist; // dist > stop >= 0，除法安全（stop 由呼叫端保證 < dist）
    let nx = pet.0 + dx * inv * step;
    let ny = pet.1 + dy * inv * step;
    (nx, ny, step > f32::EPSILON)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
        let dx = a.0 - b.0;
        let dy = a.1 - b.1;
        (dx * dx + dy * dy).sqrt()
    }

    #[test]
    fn settles_within_stop_zone() {
        // 主人就在歇腳圈內（距離 10 < FOLLOW_STOP）→ 寵物原地不動、不移動。
        let pet = (100.0, 100.0);
        let owner = (110.0, 100.0);
        let (nx, ny, moving) = follow_step(pet, owner, 0.1);
        assert_eq!((nx, ny), pet);
        assert!(!moving);
    }

    #[test]
    fn catches_up_toward_owner() {
        // 主人在遠處 → 寵物朝主人移動、距離縮短、回報移動中。
        let pet = (100.0, 100.0);
        let owner = (300.0, 100.0);
        let before = dist(pet, owner);
        let (nx, ny, moving) = follow_step(pet, owner, 0.1);
        assert!(moving);
        assert!(nx > pet.0, "應朝主人（右）移動，nx={nx}");
        assert!((ny - 100.0).abs() < 0.001, "同 y 不該偏移");
        assert!(dist((nx, ny), owner) < before, "距離應縮短");
    }

    #[test]
    fn moves_toward_owner_when_owner_is_left() {
        // 方向性：主人在左 → 寵物向左移動。
        let pet = (300.0, 100.0);
        let owner = (100.0, 100.0);
        let (nx, _ny, moving) = follow_step(pet, owner, 0.1);
        assert!(moving);
        assert!(nx < pet.0, "應朝主人（左）移動，nx={nx}");
    }

    #[test]
    fn clamped_by_speed() {
        // 小 dt：單幀位移恰為 FOLLOW_SPEED * dt（離主人夠遠、不會撞到歇腳環）。
        let pet = (0.0, 0.0);
        let owner = (500.0, 0.0); // 500 < FOLLOW_SNAP（600），走跟隨而非瞬移
        let dt = 0.05;
        let (nx, _ny, _moving) = follow_step(pet, owner, dt);
        assert!((nx - FOLLOW_SPEED * dt).abs() < 0.01, "nx={nx}");
    }

    #[test]
    fn never_overshoots_stop_ring() {
        // 巨大 dt：寵物最多走到歇腳環上，絕不越過主人。
        let pet = (0.0, 0.0);
        let owner = (200.0, 0.0);
        let (nx, ny, _moving) = follow_step(pet, owner, 100.0);
        let d = dist((nx, ny), owner);
        assert!((d - FOLLOW_STOP).abs() < 0.01, "應停在歇腳環上，d={d}");
        assert!(nx < owner.0, "不該越過主人");
    }

    #[test]
    fn snaps_when_beyond_snap_threshold() {
        // 相距超過 FOLLOW_SNAP（瞬移 / 換星球 / 重生）→ 寵物直接出現在主人身邊。
        let pet = (0.0, 0.0);
        let owner = (5000.0, 5000.0);
        let (nx, ny, moving) = follow_step(pet, owner, 0.1);
        assert_eq!((nx, ny), owner);
        assert!(!moving, "瞬移歸位不算走路");
    }

    #[test]
    fn zero_dt_no_move() {
        // dt = 0（暫停 / 卡頓）→ 寵物不動。
        let pet = (0.0, 0.0);
        let owner = (500.0, 0.0);
        let (nx, ny, moving) = follow_step(pet, owner, 0.0);
        assert_eq!((nx, ny), pet);
        assert!(!moving);
    }

    #[test]
    fn deterministic_pure() {
        // 純函式：相同輸入兩次呼叫得相同輸出。
        let pet = (12.0, 34.0);
        let owner = (456.0, 78.0);
        let a = follow_step(pet, owner, 0.1);
        let b = follow_step(pet, owner, 0.1);
        assert_eq!(a, b);
    }

    #[test]
    fn custom_stop_settles_on_that_ring() {
        // 性格化歇腳：用較大的 stop（如慵懶 46）→ 寵物停在那個環上、不再貼近。
        let pet = (0.0, 0.0);
        let owner = (200.0, 0.0);
        let stop = 46.0;
        let (nx, ny, _moving) = follow_step_with_stop(pet, owner, 100.0, stop);
        let d = dist((nx, ny), owner);
        assert!((d - stop).abs() < 0.01, "應停在指定歇腳環上，d={d}");
    }

    #[test]
    fn clingy_stops_closer_than_lazy() {
        // 黏人（小 stop）最終停得比慵懶（大 stop）更貼主人。
        let owner = (300.0, 0.0);
        let settle = |stop: f32| {
            let mut pet = (0.0, 0.0);
            for _ in 0..400 {
                let (nx, ny, moving) = follow_step_with_stop(pet, owner, 0.05, stop);
                pet = (nx, ny);
                if !moving {
                    break;
                }
            }
            dist(pet, owner)
        };
        let clingy = settle(20.0);
        let lazy = settle(46.0);
        assert!(clingy < lazy, "黏人應停得比慵懶更貼主人：{clingy} < {lazy}");
    }

    #[test]
    fn follow_step_matches_default_stop() {
        // follow_step 應等價於用預設 FOLLOW_STOP 呼叫 follow_step_with_stop。
        let pet = (12.0, 34.0);
        let owner = (456.0, 78.0);
        assert_eq!(
            follow_step(pet, owner, 0.1),
            follow_step_with_stop(pet, owner, 0.1, FOLLOW_STOP)
        );
    }

    #[test]
    fn approaches_monotonically_until_settled() {
        // 連續多步：距離單調縮短，最終穩定在歇腳圈內、停止移動。
        let mut pet = (0.0, 0.0);
        let owner = (500.0, 0.0);
        let mut prev = dist(pet, owner);
        let mut settled = false;
        for _ in 0..200 {
            let (nx, ny, moving) = follow_step(pet, owner, 0.05);
            pet = (nx, ny);
            let now = dist(pet, owner);
            assert!(now <= prev + 0.001, "距離不該變遠：{prev}→{now}");
            prev = now;
            if !moving {
                assert!(now <= FOLLOW_STOP + 0.01, "停下時應在歇腳圈內，d={now}");
                settled = true;
                break;
            }
        }
        assert!(settled, "若干步內應追上並歇下");
    }
}
