# 待 Review PR

## PR #143 — ROADMAP 49 農田地塊作物種植

- **分支**：`feat/farm-crops`
- **PR**：https://github.com/ShihShihDer/ButFun/pull/143
- **狀態**：等待 review

### 重點確認項目

1. `src/farm_crops.rs` 純邏輯 + 9 個單元測試是否符合設計
2. `src/crafting.rs` 新增 `farm_croppable` 來源判斷是否完整
3. `web/game.js` 農作面板 UX 是否清楚（種植/收割按鈕狀態）
4. `web/index.html` T 鍵快捷鍵是否與其他快捷鍵衝突
5. 3 種食物回血量平衡（12/10/15 HP）是否合理
