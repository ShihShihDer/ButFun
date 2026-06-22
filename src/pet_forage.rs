//! ROADMAP 484 寵物撈寶·小夥伴替你叼回東西——寵物逗玩接物（345）第一次「玩有所得」。
//!
//! 345 給了寵物「逗玩接物」這個純陪伴動作——你丟玩具、牠衝去叼回，但玩具叼回即消失、
//! 不回饋玩家半點攢累（當時誠實定位成純玩耍，鏡像放風箏 470／打水漂 475）。本切片把這份
//! 一來一往的「玩」接進**羈絆→成長→回饋**循環：你陪牠玩得越久、默契（羈絆）越深；當羈絆
//! 養起來，興奮叼著玩具跑回來的小夥伴偶爾會多叼一樣牠在路上「撿到」的小東西放你腳邊——
//! 依牠的出身（草原精靈叼回野花種子、岩地傀儡叼回石頭、水域珊瑚蟹叼回小魚…），有時還有
//! 一小撮乙太。沒撈到不是懲罰（療癒向），只是這趟單純玩得開心而已。
//!
//! ## 設計鐵律（正面回應 reviewer「換維度／把薄動詞接進經濟成長」）
//! - **換維度、不同機制**：483 打水漂走「技巧（甜蜜點）→風險→經濟」（每趟獨立擲骰）；本切片走
//!   全然不同的「**羈絆（累積陪伴）→成長→回饋**」——回饋不看單次手藝，而看你陪小夥伴玩出來的
//!   默契有多深（bond 隨接物次數成長）。它是寵物這條多切片維度（46／343～345／358）第一個
//!   「回饋」出口＝深度／互連，不是再開一個新動詞。
//! - **純邏輯可測**：`bond_level`／`forage_item`／`forage_gift`／`gift_ether` 皆純函式、確定可重現、
//!   無 IO、無副作用——回饋的源頭數值（羈絆門檻、機率、乙太量、各寵物叼回物）全定在此，
//!   呼叫端只負責產 seed 與發放。
//! - **療癒向、量級克制、近零經濟擾動**：叼回的都是世界本來就採得到的尋常小物（零新增 enum、
//!   無任何裝備材料）、乙太極小且封頂；羈絆熱身期（bond 0）完全不送；搭配既有接物冷卻，
//!   避免變成乙太水龍頭。撈不到只是「單純玩得開心」不扣任何東西（永不懲罰，鏡像 438／454 基調）。
//! - **零持久化、零 migration、零 LLM**：羈絆由 `Player.pet_fetch_count`（記憶體前置暫態、
//!   重連／重啟歸零，鏡像寵物本身記憶體模式）即時算出，不入存檔。

use crate::inventory::ItemKind;
use crate::pet::PetKind;

/// 羈絆最高等級。刻意只給 5 階——好辨、好測、夠表達「越玩越熟」即可。
pub const MAX_BOND: u8 = 5;

/// 每多陪寵物玩 `FETCHES_PER_BOND` 趟接物，羈絆加深一階（封頂 `MAX_BOND`）。
/// 前幾趟（bond 0）小夥伴還在跟你混熟、空手玩；養起來後才開始替你叼東西。
pub const FETCHES_PER_BOND: u64 = 3;

/// 「叼回一樣小物」的基礎機率（百分點）與每階羈絆的加成斜率。
/// bond 1 ≈ 12%、bond 5 ≈ 28%；大半時候小夥伴只是單純玩得開心、空著手回來。
pub const ITEM_BASE_PCT: u64 = 8;
pub const ITEM_PCT_PER_BOND: u64 = 4;

/// 「叼回一小撮乙太」的基礎機率（百分點）與每階羈絆的加成斜率（接在叼物段之後、不重疊）。
/// bond 1 ≈ 4%、bond 5 ≈ 8%。
pub const ETHER_BASE_PCT: u64 = 3;
pub const ETHER_PCT_PER_BOND: u64 = 1;

/// 叼回乙太的最大量。刻意小、封頂——逗寵物是療癒小活動，不是刷錢主力。
pub const MAX_GIFT_ETHER: u32 = 3;

/// 由累積接物次數算羈絆等級：每 `FETCHES_PER_BOND` 趟加一階、封頂 `MAX_BOND`。
/// 壞值（極大次數）自然被 `min` 夾住、不 panic。
pub fn bond_level(fetch_count: u64) -> u8 {
    (fetch_count / FETCHES_PER_BOND).min(MAX_BOND as u64) as u8
}

/// 各寵物依出身叼回的尋常小物（**複用既有物品、零新增 enum、皆非裝備材料**）：
/// 草原飄舞精靈→野花種子、岩地晶石傀儡→石頭、水域珊瑚蟹→小魚、翠幽魅影→幽花、
/// 星源守護者→星塵。刻意全挑「世界本來就採得到的低值小物」，近零經濟擾動。
pub fn forage_item(pet: PetKind) -> ItemKind {
    match pet {
        PetKind::FlutterSprite => ItemKind::WildflowerSeed,
        PetKind::CrystalGolem => ItemKind::Stone,
        PetKind::CoralCrab => ItemKind::FishSmall,
        PetKind::JadeWraith => ItemKind::WildFlower,
        PetKind::OriginGuardian => ItemKind::StarDust,
    }
}

/// 叼回乙太時的量：隨羈絆遞增、封頂 `MAX_GIFT_ETHER`、恆 >=1（送了就至少 1）。
/// bond 1..=5 → 1,1,2,2,3。壞 bond（0 或超界）夾在界內、不 panic、仍回合法量。
pub fn gift_ether(bond: u8) -> u32 {
    (((bond as u32) + 1) / 2).clamp(1, MAX_GIFT_ETHER)
}

/// 這趟小夥伴叼回什麼（純值、`Copy`、確定可重現）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForageGift {
    /// 空手回來、單純玩得開心（不是懲罰，療癒向）。
    Nothing,
    /// 叼回一小撮乙太（進核心貨幣餘額）。
    Ether(u32),
    /// 叼回一樣尋常小物（進背包）。
    Item(ItemKind),
}

impl ForageGift {
    /// 這趟叼回的乙太量（沒有則 0）——呼叫端據此加餘額、廣播飄字。
    pub fn ether(self) -> u32 {
        match self {
            ForageGift::Ether(n) => n,
            _ => 0,
        }
    }

    /// 這趟叼回的背包物品（沒有則 None）。
    pub fn item(self) -> Option<ItemKind> {
        match self {
            ForageGift::Item(k) => Some(k),
            _ => None,
        }
    }
}

/// 由寵物、羈絆等級與一顆 seed 算這趟叼回什麼。確定可重現、純函式、無副作用。
/// - 羈絆熱身期（`bond == 0`）：完全不送（小夥伴還在跟你混熟）。
/// - 羈絆養起來後，叼回小物／乙太的機率隨等級遞增；其餘＝`Nothing`（單純玩得開心、空手回來）。
///
/// seed 建議帶 `player_id_low64 ^ pet_fetch_count`（每趟結果不同、可重現、好測），
/// 與打水漂 `skip_find`／釣魚 `roll_fish` 同一套確定性擲骰範式。
pub fn forage_gift(pet: PetKind, bond: u8, seed: u64) -> ForageGift {
    if bond == 0 {
        return ForageGift::Nothing;
    }
    let b = bond.min(MAX_BOND) as u64;
    let pct = seed % 100; // 0..=99 的骰面

    // 1) 叼回一樣小物：窗口隨羈絆加寬，從骰面最前段切出。
    let item_threshold = ITEM_BASE_PCT + b * ITEM_PCT_PER_BOND;
    if pct < item_threshold {
        return ForageGift::Item(forage_item(pet));
    }
    // 2) 叼回一小撮乙太：接在叼物段之後（不重疊）。
    let ether_threshold = item_threshold + ETHER_BASE_PCT + b * ETHER_PCT_PER_BOND;
    if pct < ether_threshold {
        return ForageGift::Ether(gift_ether(bond));
    }
    // 3) 其餘：空手回來、單純玩得開心。
    ForageGift::Nothing
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_PETS: [PetKind; 5] = [
        PetKind::FlutterSprite,
        PetKind::CrystalGolem,
        PetKind::CoralCrab,
        PetKind::JadeWraith,
        PetKind::OriginGuardian,
    ];

    /// 羈絆等級隨接物次數不減、封頂 MAX_BOND，且如期分階。
    #[test]
    fn 羈絆隨次數遞增且封頂() {
        let mut prev = 0;
        for n in 0..(FETCHES_PER_BOND * (MAX_BOND as u64) + 20) {
            let b = bond_level(n);
            assert!(b <= MAX_BOND, "羈絆不得超過封頂：n={n} b={b}");
            assert!(b >= prev, "羈絆應隨次數不減：{prev} → {b} @ n={n}");
            prev = b;
        }
        // 分階檢查：0 趟→0、首階門檻→1、滿階及以後→MAX。
        assert_eq!(bond_level(0), 0);
        assert_eq!(bond_level(FETCHES_PER_BOND - 1), 0, "熱身期內仍是 0");
        assert_eq!(bond_level(FETCHES_PER_BOND), 1);
        assert_eq!(bond_level(FETCHES_PER_BOND * MAX_BOND as u64), MAX_BOND);
        assert_eq!(bond_level(u64::MAX), MAX_BOND, "極大次數夾在封頂、不 panic");
    }

    /// 羈絆熱身期（bond 0）永遠空手回來——小夥伴還沒熟到替你撿東西。
    #[test]
    fn 熱身期永不送() {
        for pet in ALL_PETS {
            for seed in 0..300u64 {
                assert_eq!(
                    forage_gift(pet, 0, seed),
                    ForageGift::Nothing,
                    "bond 0 不該送東西：pet={pet:?} seed={seed}"
                );
            }
        }
    }

    /// 「叼回東西」（小物或乙太）的骰面數隨羈絆單調不減——越熟越常替你撿東西。
    #[test]
    fn 叼回機率隨羈絆遞增() {
        for pet in ALL_PETS {
            let mut prev_hits = 0;
            for bond in 1..=MAX_BOND {
                let hits = (0..100u64)
                    .filter(|&s| forage_gift(pet, bond, s) != ForageGift::Nothing)
                    .count();
                assert!(
                    hits >= prev_hits,
                    "叼回骰面數應隨羈絆不減：{prev_hits} → {hits} @ bond={bond} pet={pet:?}"
                );
                prev_hits = hits;
            }
            // 最熟（滿羈絆）仍不是必中——大半時候只是單純玩得開心。
            let top = (0..100u64)
                .filter(|&s| forage_gift(pet, MAX_BOND, s) != ForageGift::Nothing)
                .count();
            assert!(top > 0 && top < 100, "滿羈絆應常叼回但非必中：{top}/100 pet={pet:?}");
        }
    }

    /// 叼回乙太量：隨羈絆不減、恆 >=1、封頂 MAX_GIFT_ETHER；壞 bond 夾界不 panic。
    #[test]
    fn 乙太量隨羈絆不減且封頂() {
        let mut prev = 0;
        for bond in 1..=MAX_BOND {
            let amt = gift_ether(bond);
            assert!(amt >= 1, "送了就至少 1 乙太：bond={bond} amt={amt}");
            assert!(amt <= MAX_GIFT_ETHER, "乙太量不得超過封頂：bond={bond} amt={amt}");
            assert!(amt >= prev, "乙太量應隨羈絆不減：{prev} → {amt}");
            prev = amt;
        }
        assert!(gift_ether(0) >= 1, "壞 bond 0 仍回合法量");
        assert!(gift_ether(250) <= MAX_GIFT_ETHER, "壞 bond 超界夾在封頂");
    }

    /// 各寵物依出身叼回對的主題小物，且**全是低值尋常物、無任何裝備材料**（經濟守門）。
    #[test]
    fn 各寵物叼回低值主題物() {
        let humble = [
            ItemKind::WildflowerSeed,
            ItemKind::Stone,
            ItemKind::FishSmall,
            ItemKind::WildFlower,
            ItemKind::StarDust,
        ];
        assert_eq!(forage_item(PetKind::FlutterSprite), ItemKind::WildflowerSeed);
        assert_eq!(forage_item(PetKind::CrystalGolem), ItemKind::Stone);
        assert_eq!(forage_item(PetKind::CoralCrab), ItemKind::FishSmall);
        assert_eq!(forage_item(PetKind::JadeWraith), ItemKind::WildFlower);
        assert_eq!(forage_item(PetKind::OriginGuardian), ItemKind::StarDust);
        for pet in ALL_PETS {
            assert!(
                humble.contains(&forage_item(pet)),
                "叼回物必須是低值尋常物、不得是裝備材料：pet={pet:?}"
            );
        }
    }

    /// 確定可重現：同寵物同羈絆同 seed 永遠同結果。
    #[test]
    fn 同輸入同輸出可重現() {
        for pet in ALL_PETS {
            for bond in 0..=MAX_BOND {
                for seed in [0u64, 3, 17, 42, 99, 100, 12345, u64::MAX] {
                    assert_eq!(forage_gift(pet, bond, seed), forage_gift(pet, bond, seed));
                }
            }
        }
    }

    /// 高骰面（接近 99）一律空手回來——大半時候小夥伴只是單純玩得開心。
    #[test]
    fn 高骰面皆空手() {
        for pet in ALL_PETS {
            for bond in 1..=MAX_BOND {
                assert_eq!(forage_gift(pet, bond, 99), ForageGift::Nothing);
                assert_eq!(forage_gift(pet, bond, 95), ForageGift::Nothing);
            }
        }
    }

    /// 壞 bond（超界）夾在界內、不 panic、結果合法。
    #[test]
    fn 壞羈絆保守夾界() {
        for pet in ALL_PETS {
            for seed in 0..100u64 {
                let g = forage_gift(pet, 200, seed); // 遠超 MAX_BOND，等效滿羈絆
                assert!(matches!(
                    g,
                    ForageGift::Nothing | ForageGift::Ether(_) | ForageGift::Item(_)
                ));
                // 等效滿羈絆：與 MAX_BOND 同結果（min 夾住）。
                assert_eq!(g, forage_gift(pet, MAX_BOND, seed));
            }
        }
    }

    /// ForageGift 存取器自洽：ether()／item() 互相一致。
    #[test]
    fn 存取器自洽() {
        assert_eq!(ForageGift::Nothing.ether(), 0);
        assert_eq!(ForageGift::Nothing.item(), None);

        assert_eq!(ForageGift::Ether(2).ether(), 2);
        assert_eq!(ForageGift::Ether(2).item(), None);

        assert_eq!(ForageGift::Item(ItemKind::Stone).ether(), 0);
        assert_eq!(ForageGift::Item(ItemKind::Stone).item(), Some(ItemKind::Stone));
    }
}
