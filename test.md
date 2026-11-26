# soundx 測試改善計畫

> 原計畫基於 CBM 分析。本版數字為 0.3.0 worktree 的實際統計：行數以 `wc -l` 計算，測試數為 `#[test]` 出現次數（Grep 計數）。
> 未執行覆蓋率工具，因此不列覆蓋率百分比。

---

## 現狀分析

### 原始碼（`src/*.rs`）

| 模組 | 行數 | `#[test]` 數 | 備註 |
|------|------|--------------|------|
| `src/audio.rs` | 793 | 0 | WAV / symphonia / AU 讀取；測試在 `tests/cli.rs` 與 `tests/effects.rs` |
| `src/cli.rs` | 599 | 0 | 參數定義 |
| `src/codecs.rs` | 495 | 5 | |
| `src/device.rs` | 1076 | 9 | |
| `src/effects.rs` | 1535 | 0 | 效果測試位於 `tests/effects.rs` |
| `src/encode.rs` | 603 | 0 | |
| `src/integrations.rs` | 746 | 7 | |
| `src/io.rs` | 69 | 0 | |
| `src/lib.rs` | 7 | 0 | |
| `src/limiter.rs` | 432 | 0 | |
| `src/loudness.rs` | 697 | 3 | |
| `src/main.rs` | 332 | 0 | |
| `src/mcp.rs` | 652 | 1 | |
| `src/mix.rs` | 114 | 0 | |
| `src/parse.rs` | 559 | 0 | |
| `src/stats.rs` | 58 | 0 | |
| `src/streaming.rs` | 354 | 0 | |
| `src/synth.rs` | 79 | 0 | |
| `src/util.rs` | 4 | 0 | |
| `src/windows_audio.rs` | 704 | 4 | |
| **合計** | **9908** | **29** | |

### 整合測試與測試輔助（`tests/`）

| 檔案 | 行數 | `#[test]` 數 | 內容 |
|------|------|--------------|------|
| `tests/effects.rs` | 1358 | 81 | 效果單元測試 |
| `tests/cli.rs` | 917 | 24 | CLI 整合測試 |
| `tests/loudness.rs` | 802 | 21 | 響度測試（含選用的 ffmpeg 對照） |
| `tests/mcp.rs` | 200 | 3 | MCP stdio 測試 |
| `tests/agent_integration.rs` | 62 | 1 | agent 安裝 / 卸載 |
| `tests/common/mod.rs` | 49 | 0 | 共用輔助函式 |
| **合計** | | **130** | |

`tests/fixtures/` 目前只有 `.gitkeep`，沒有預先產生的 WAV 測試檔。

全部 `#[test]` 數為 159（tests 130 + src 29）。

---

## 測試計畫進度

### Phase 1 — 效果單元測試

**檔案**: `tests/effects.rs`（已存在，81 個測試）

**狀態**: 部分完成。計畫中的 gain、normalize、trim、fade、reverse、speed、pad、trim silence、lowpass/highpass、limiter、rate、channels 與 effect chain 大多已有測試。另有計畫外的 dither、compand、reverb、stretch/tempo、echo、delay、tremolo、dcshift、biquad 等測試。

未見的計畫項目：
- `convert_channels` 的 stereo → 6ch（5.1）映射
- lowpass / highpass 的 impulse response 穩定性測試

### Phase 2 — 音訊 I/O 單元測試

**檔案**: `tests/audio.rs`（未建立）

**狀態**: 未開始（獨立檔案）。WAV 讀寫已由 CLI 層級測試覆蓋：`converts_wav_roundtrip_without_external_deps`、`converts_to_selectable_wav_depths`、`writes_and_reads_float64_wav`（`tests/cli.rs`）。計畫中的各位元深度 `read_wav` 獨立測試與 frame / duration 獨立測試尚未建立。

### Phase 3 — 串流模組測試

**檔案**: `tests/streaming.rs`（未建立）

**狀態**: 未開始（獨立檔案）。`streams_wav_without_full_buffer_pipeline`（`tests/cli.rs`）為 CLI 層級的串流測試。計畫中的串流 gain / fade / limiter 單元測試尚未建立。

### Phase 4 — 整合測試強化

**檔案**: `tests/cli.rs`（已擴充至 24 個測試，原計畫 8 → 15+）

**狀態**: 部分完成。測試數已超過目標，但計畫列出的內容多數未見：
- edge cases：空檔案、不存在的檔案、不支援格式、0 秒 synth 均未見
- `missing_required_arg`、batch（多檔、glob）未見
- `run_plan`：僅有 `runs_json_plan`；`plan_concat`、`plan_mix` 未見
- `legacy_style` 大致對應 `converts_with_sox_style_effects`

### Phase 5 — 屬性測試（Property-based Testing）

**檔案**: `tests/property.rs`（未建立）

**狀態**: 未開始。`Cargo.toml` 未宣告 `proptest`。

### Phase 6 — 回歸測試

**檔案**: `tests/regression.rs`（未建立）

**狀態**: 未開始。

---

## 測試基礎設施

**狀態**: 部分完成。
- `tests/common/mod.rs` 提供 `test_buffer`、`test_buffer_with_samples`、`approx_eq`、`assert_buffer_eq`。計畫中的 `create_test_wav`、`create_sine_buffer`、`create_test_dir`、`assert_peak` 尚未加入。
- 尚未建立 WAV 測試檔，也沒有 `include_bytes!` 嵌入（`tests/fixtures/` 僅有 `.gitkeep`）。
- `tests/loudness.rs` 的 ffmpeg 對照測試在找不到 ffmpeg 時會跳過；可用環境變數 `SOUNDX_FFMPEG` 指定路徑。

---

## 測試執行計畫

| Phase | 檔案 | 狀態 | 目前測試數 |
|-------|------|------|------------|
| 1 | `tests/effects.rs` | 部分完成 | 81 |
| 2 | `tests/audio.rs` | 未開始 | 0（相關 CLI 測試見上） |
| 3 | `tests/streaming.rs` | 未開始 | 0（相關 CLI 測試見上） |
| 4 | `tests/cli.rs` | 部分完成 | 24 |
| 5 | `tests/property.rs` | 未開始 | 0 |
| 6 | `tests/regression.rs` | 未開始 | 0 |

---

## 優先執行建議

1. **Phase 1 缺口**：補上 stereo → 5.1 映射與 lowpass / highpass 的 impulse response 測試。
2. **Phase 4 缺口**：補上 edge cases（空檔案、不存在的檔案、不支援格式、0 秒 synth）與 batch、缺少必要參數的測試。
3. **Phase 2 + Phase 3**：為 I/O 與串流建立獨立測試檔。
4. **Phase 5 + Phase 6**：屬性測試與回歸測試（需先在 `Cargo.toml` 加入 `proptest`）。

```bash
# 執行所有測試
cargo test

# 執行特定測試檔
cargo test --test effects
cargo test --test cli -- converts_with_sox_style_effects
```
