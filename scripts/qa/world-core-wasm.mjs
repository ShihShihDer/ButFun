// QA 共用：載入 world-core 編成的 .wasm，讓 QA 機器人用「伺服器同一份 Rust 實作」
// 判地形，不再維護會過期的 JS 副本（過去 functional-qa 的 isSolid 就漂移過）。
// 找不到 .wasm（還沒跑 scripts/build-wasm.sh）回 null，呼叫端退回各自的 JS 後備。
import { readFileSync } from "fs";
import { fileURLToPath } from "url";
import { dirname, join } from "path";

const REPO = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const CANDIDATES = [
  join(REPO, "web", "wasm", "world_core.wasm"),
  join(REPO, "target", "wasm32-unknown-unknown", "release", "world_core.wasm"),
];

// 整數編碼 → 協定字串名（對齊 world-core TileKind::code / Biome::code，
// Rust 測試 tile_kind_codes_are_stable 守著這份順序）。
export const TILE_NAMES = [
  "empty", "dirt", "stone", "ore", "crystal", "mushroom", "ancient_ruin",
  "coral_reef", "wild_flower", "jade_vine", "lava_rock", "void_crystal",
  "aether_mist", "origin_crystal",
];
export const BIOME_NAMES = ["water", "sand", "meadow", "forest", "rocky"];

/** 回 { tileKindCode, biomeCode }（f64,f64→u32）或 null（沒建置過 wasm）。 */
export async function loadWasmTerrain() {
  for (const p of CANDIDATES) {
    try {
      const buf = readFileSync(p);
      const { instance } = await WebAssembly.instantiate(buf, {});
      const ex = instance.exports;
      if (typeof ex.tile_kind_code === "function" && typeof ex.biome_code === "function") {
        console.log(`[wasm] 地形判定使用 ${p}（伺服器同一份實作）`);
        return { tileKindCode: ex.tile_kind_code, biomeCode: ex.biome_code };
      }
    } catch {
      // 試下一個候選路徑
    }
  }
  console.warn("[wasm] 找不到 world_core.wasm（先跑 scripts/build-wasm.sh），改用 JS 後備地形");
  return null;
}
