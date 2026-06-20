//! 作物輪作（ROADMAP 454 深度弧 A 第三塊「這塊地上輪種了啥」）。
//!
//! 452 給作物加了品種、453 給品種加了季節脾性。這兩塊都只看「這一株」自己的事；本塊第一次
//! 讓**這塊地的歷史**有意義：每格田記住「上一輪收成的是哪個品種」，下一輪若**換種**一個不同
//! 品種，新作物在「換過的土」裡長得更旺（成長加成）——種田第一次要看「這格上次種了啥、該換種了」。
//!
//! 與既有兩條土地維度**正交、互補、不重疊**：
//!   - 438 沃土輪休＝獎勵「讓地空著歇」（時間休耕 → 收成多得乙太）。
//!   - 367 連片沃土＝獎勵「把地種成連片」（空間鄰接 → 成長加速）。
//!   - 454 輪作＝獎勵「換著品種種」（品種多樣 → 成長加速）。三種真實農法各走各的軸。
//!
//! 設計原則（鏡像 438 的療癒基調）：
//! - **純正向**：換種給「額外」成長、連種同品種只是回到基準（×1.0），**永不懲罰**單一栽種。
//!   這讓「換著種」是划得來的選擇，而非「不照辦就被倒扣」（療癒向、不逼迫）。
//! - **向後相容的關鍵不變式**：新地／舊存檔沒有上輪紀錄（`None`）→ ×1.0，行為與改動前一字不差。
//! - 純邏輯、確定性、無 IO，便於自動測試。權威狀態（每格上輪品種）住 `field.rs` 的平行陣列。

use crate::crop_variety::CropVariety;

/// 換種（輪作）時新作物的成長速度倍率（>1＝換過的土裡長得更旺）。
/// 與品種速度、季節偏好、連片沃土倍率各自獨立疊乘；都只放大「長得多快」、不多耗水。
pub const ROTATION_GROWTH_MULT: f32 = 1.25;

/// 這格新種下的作物能不能吃到「輪作加成」：上一輪有紀錄、且與這次種的**不同品種** → 加成；
/// 否則（首次種／沒紀錄／連種同品種）回基準 ×1.0。純函式、純正向（永不 <1.0）。
pub fn rotation_bonus(last: Option<CropVariety>, planting: CropVariety) -> f32 {
    match last {
        Some(prev) if prev != planting => ROTATION_GROWTH_MULT,
        _ => 1.0,
    }
}

/// 把「上輪品種紀錄」編成細碼存進田格平行陣列：`None`（無紀錄）＝0、有品種＝`code`+1。
/// 0 保留給「這格還沒收成過任何作物」，與既有作物碼（0=主食穀）區隔，故 +1 偏移。
pub fn encode(last: Option<CropVariety>) -> u8 {
    match last {
        None => 0,
        Some(v) => v.code() + 1,
    }
}

/// `encode` 的反向：細碼 → 上輪品種紀錄。0＝無紀錄回 `None`；壞碼也保守回 `None`（當無紀錄）。
pub fn decode(code: u8) -> Option<CropVariety> {
    if code == 0 {
        None
    } else {
        CropVariety::from_code(code - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_rewards_switching_variety() {
        // 上輪種主食穀、這輪換速生菜 → 吃到輪作加成（換過的土長得旺）。
        assert_eq!(
            rotation_bonus(Some(CropVariety::Staple), CropVariety::Sprout),
            ROTATION_GROWTH_MULT
        );
        // 任兩個不同品種互換都算輪作。
        assert_eq!(
            rotation_bonus(Some(CropVariety::Etherbloom), CropVariety::Staple),
            ROTATION_GROWTH_MULT
        );
    }

    #[test]
    fn monocropping_and_first_planting_are_neutral_never_punished() {
        // 連種同品種：回基準 ×1.0（純正向——不倒扣、不懲罰單一栽種）。
        for v in CropVariety::ALL {
            assert_eq!(rotation_bonus(Some(v), v), 1.0, "{} 連種應為基準", v.as_str());
        }
        // 首次種（沒上輪紀錄）：也是基準 ×1.0＝改動前行為（向後相容的關鍵不變式）。
        for v in CropVariety::ALL {
            assert_eq!(rotation_bonus(None, v), 1.0, "{} 首次種應為基準", v.as_str());
        }
    }

    #[test]
    fn bonus_is_finite_and_at_least_one() {
        // 任何組合的倍率都必為有限值、且 >=1.0（純正向；grow_boosted 不會被汙染或減速）。
        let cases = [None, Some(CropVariety::Staple), Some(CropVariety::Sprout), Some(CropVariety::Etherbloom)];
        for last in cases {
            for planting in CropVariety::ALL {
                let b = rotation_bonus(last, planting);
                assert!(b.is_finite() && b >= 1.0, "倍率須有限且 >=1");
            }
        }
    }

    #[test]
    fn encode_decode_round_trips() {
        // 無紀錄 ↔ 0；各品種 ↔ code+1，往返不變。
        assert_eq!(encode(None), 0);
        assert_eq!(decode(0), None);
        for v in CropVariety::ALL {
            let c = encode(Some(v));
            assert!(c > 0, "有品種的細碼應 >0（與無紀錄 0 區隔）");
            assert_eq!(decode(c), Some(v), "{} 編解碼往返", v.as_str());
        }
        // 壞碼（超出任何品種）保守回無紀錄，不 panic、不誤判成某品種。
        assert_eq!(decode(250), None);
    }

    #[test]
    fn decoded_history_drives_bonus_end_to_end() {
        // 編入上輪品種、解出後比對這輪 → 與直接呼叫 rotation_bonus 一致（整條鏈自洽）。
        let stored = encode(Some(CropVariety::Etherbloom));
        assert_eq!(
            rotation_bonus(decode(stored), CropVariety::Sprout),
            ROTATION_GROWTH_MULT,
            "上輪乙太瓜、這輪速生菜＝換種加成"
        );
        assert_eq!(
            rotation_bonus(decode(stored), CropVariety::Etherbloom),
            1.0,
            "上輪乙太瓜、這輪又乙太瓜＝連種、基準"
        );
    }
}
