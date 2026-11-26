# soundx 優化改善清單

> 基於 CBM 架構分析與原始碼審查結果，按優先級排列。
> 狀態已依 0.3.0 的 `src/`、`tests/`、`Cargo.toml` 與 `.github/workflows/ci.yml` 逐項比對更新。
> 標記：`[完成]` 已實作；`（部分完成）` 只完成部分內容；`（未完成）` 尚未實作。

---

## P0 — 關鍵缺陷（必須修復）

### [完成] 重複的 `seconds_to_frames` 函式

- **完成**: 已移至 `src/util.rs`（`pub fn seconds_to_frames(seconds: f32, sample_rate: u32) -> usize`），`src/effects.rs` 與 `src/streaming.rs` 皆改為引用同一函式。

### [完成] `main.rs` 過大

- **完成**: `src/main.rs` 已降至 332 行。`parse_effects` 與 `parse_next_f32/u32/u16` 位於 `src/parse.rs`；`expand_inputs`、`output_for_batch`、`convert_one`、`write_output` 位於 `src/io.rs`；`concat_inputs`、`mix_inputs`、`sample_at`、`ensure_same_format` 位於 `src/mix.rs`。

### [P0] WAV 位元深度處理重複（未完成）

- **狀態**: 未完成。`src/audio.rs` 的 `read_int_samples`（約 L563）與 `src/streaming.rs` 的 `stream_wav`（約 L77 起的 `bits_per_sample` match）、`replay_wav`（約 L189 起）各自實作整數轉 `f32`，`src/util.rs` 目前沒有共用的 `wav_to_f32`。

### [P0] 三倍重複的 `effect_chain()` 模式（部分完成）

- **狀態**: 部分完成。`src/effects.rs` 的 `EffectChain` 已提供 builder 式的 `with` / `extend`（約 L169–176）。但 `src/cli.rs` 仍有三個各自實作的 `effect_chain()`（約 L467、L527、L560），尚未改為共用建構邏輯。

---

## P1 — 重要改善

### [完成] 新增效果單元測試（effects.rs 原 403 行零測試）

- **完成**: 效果測試位於 `tests/effects.rs`（81 個 `#[test]`），涵蓋 gain、normalize、trim、fade、reverse、speed、pad、trim silence、lowpass/highpass、limiter、rate、channels 與 effect chain。`src/effects.rs` 本身（現 1535 行）仍沒有 `#[test]`。

### [完成] Mix 行為修正（平均 → 疊加）

- **完成**: `src/mix.rs` 的 `mix_inputs` 改為疊加（函式註解為 "summing samples (no automatic averaging)"）。`mix` 子命令提供 `--normalize`（`MixArgs`，`src/cli.rs`）。`tests/cli.rs` 的 `concatenates_and_mixes_inputs` 覆蓋基本行為。

### [P1] 僅輸出 16-bit WAV — 支援更多格式（部分完成）

- **狀態**: 部分完成。`--bits`（8/16/24/32）與 `--float` 已實作（`src/cli.rs`；`--float` 需搭配 `--bits 32` 或 `64`），測試見 `tests/cli.rs` 的 `converts_to_selectable_wav_depths` 與 `writes_and_reads_float64_wav`。建議中的 `--format`（wav/f32）參數尚未實作。

### [P1] Stream 模式僅支援 WAV（未完成）

- **狀態**: 未完成。`stream` 子命令仍只處理 WAV（`src/streaming.rs` 的 `stream_wav` / `replay_wav`；`src/cli.rs` 說明為 "Process WAV in a streaming path"）。

### [P1] 無最大檔案保護（OOM 風險）（未完成）

- **狀態**: 未完成。`src/audio.rs` 的 `AudioBuffer::read`（約 L55）未檢查檔案大小，也沒有 `--max-size` 參數或大檔案警告。

---

## P2 — 品質與 DX

### [完成] 固定 `rust-toolchain.toml`

- **完成**: `rust-toolchain.toml` 固定 Rust `1.94.1`，`Cargo.toml` 宣告 `rust-version = "1.91"`。
- **驗證**: `cargo test --locked --all-targets`、`cargo clippy --locked --all-targets -- -D warnings` 及 release build 通過。

### [P2] 降低 symphonia 編譯負擔（未完成）

- **狀態**: 未完成。`Cargo.toml` 仍為 `symphonia = { version = "0.6.1", features = ["all"] }`。

### [完成] 新增 CI 配置

- **完成**: `.github/workflows/ci.yml` 的 `check` job 以 `ubuntu-latest`、`windows-latest`、`macos-latest` 矩陣執行 `cargo build --locked`、`cargo test --locked --all-targets`、`cargo clippy --locked --all-targets -- -D warnings` 與 `cargo fmt --check`。另有 `package`、`release` job 於 `v*` tag 推送時執行。

### [完成] Hermetic FLAC 測試

- **完成**: `tests/cli.rs` 已不依賴 ffmpeg（檔案中沒有 ffmpeg 相關呼叫）。FLAC 寫入與讀回由 `encodes_and_reads_native_rust_output_formats` 的格式迴圈涵蓋。未採用 `include_bytes!` 嵌入檔案，因 `tests/fixtures/` 目前只有 `.gitkeep`。
- 註：`tests/loudness.rs` 中與 ffmpeg 對照的測試仍是選用項目，未安裝 ffmpeg 時會跳過（可用 `SOUNDX_FFMPEG` 指定路徑）。

### [P2] Linear interpolation 提升品質（未完成）

- **狀態**: 未完成。`src/effects.rs` 的 `speed`（約 L817）與 `rate`（約 L1438）仍使用線性插值 `a + (b - a) * fraction`。

---

## P3 — 新增功能

### [P3] 尚未實作的 SoX 效果（部分完成）

- **狀態**: 部分完成。`dither`、`compand`、`reverb`、`stretch`、`tempo` 已在 `src/parse.rs` 解析，`echo`、`delay` 亦已實作；上述效果於 `tests/effects.rs` 與 `tests/cli.rs` 有測試。`phaser`、`flanger`、`pitch` 在 `src/` 中仍未實作。

### [P3] `--guard` Clipping 保護（未完成）

- **狀態**: 未完成。`src/cli.rs` 沒有 `--guard` 參數。已有的 `--limiter`（true-peak limiter）並非建議中的全域 soft-knee guard。

### [P3] 屬性測試（Property-based Testing）（未完成）

- **狀態**: 未完成。`Cargo.toml` 未宣告 `proptest` 或 `quickcheck`。

---

## 優先級摘要

| 優先級 | 數量 | 完成 | 部分完成 | 未完成 | 類型 |
|--------|------|------|----------|--------|------|
| P0 | 4 | 2 | 1 | 1 | 重複程式碼、職責拆分、bug |
| P1 | 5 | 2 | 1 | 2 | 測試覆蓋、行為修正、功能增強 |
| P2 | 5 | 3 | 0 | 2 | 開發體驗、CI、編譯優化 |
| P3 | 3 | 0 | 1 | 2 | 新功能、進階測試 |
| 合計 | 17 | 7 | 3 | 7 | |

---

## 風險說明

- **P0 未完成項目** 為 WAV 位元深度重複與 `effect_chain()` 三處重複，建議在下一輪開發優先處理。
- **main.rs 拆分** 已完成，原先關於 `pub(crate)` 引用路徑的風險已不適用。
- **Mix 行為變更** 可能影響現有使用者的混音結果，需在 changelog 標註 breaking change。
- **Symphonia feature 調整** 需確認常用格式不受影響。

---

## 已完成：響度（0.3.0）

- [x] `soundx loudness`：ITU-R BS.1770-4 / EBU R128（integrated、LRA、true peak、momentary／short-term）
- [x] `convert --loudness-target/--true-peak` 與 `stream` 版本；lookahead true-peak limiter
- [x] MCP `soundx_loudness` 工具與 Agent Skill 文件
- [ ] 其他聲道排列（如 7.1.4、自訂 channel mask）之權重；目前依 WAV 聲道數推定
- [ ] `batch`／`run-plan` 支援 `--loudness-target`
