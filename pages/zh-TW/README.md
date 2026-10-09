# soundx — Rust 原生音訊處理工具

**Rust 原生編解碼與 DSP · SoX 風格 CLI**

`soundx` 是 Rust 原生音訊處理器，提供 SoX 風格的命令列。目標是逐步相容 SoX 的格式、效果與裝置；
目前仍是部分功能，尚未完整相容 SoX。

> **狀態：** 持續開發中；SoX 支援的部分編碼格式、效果、參數與裝置行為仍未實作。

---

## Agent 整合與 Windows 音訊控制（v0.2）

安裝程式可分別選擇安裝 Codex、Claude Code、OpenCode、AGY / Antigravity 的
Skill 與 MCP 設定。內建 stdio server 以 `soundx mcp` 啟動；可攜版可執行
`soundx integrate install --agent codex` 安裝整合。
`soundx windows devices`、`endpoint`、`sessions` 以 JSON 提供 Windows 原生
音訊裝置、主音量／靜音與個別程式的音訊工作階段。
完整指令見 [Agent 安裝與使用文件](../../docs/AGENT_INTEGRATION.md)。

## 為什麼選擇 soundx？

- **Rust 原生 DSP 與編解碼器** — 系統音訊 I/O 透過 CPAL 使用作業系統音訊後端。
- **確定性管線** — 所有效果都作用於交錯 `f32` 取樣緩衝區，訊號路徑可預測且可測試。
- **SoX 風格 CLI 子集** — 支援部分 `input output effect...` 語法及子命令模式。
- **串流處理** — 逐步處理大型檔案，無需全部載入記憶體。
- **JSON 計畫** — 可重複、可腳本化的處理計畫。
- **平行批次** — 一個指令在多核心上批次轉換。
- **內建合成** — 透過 `synth` 產生波形。

---

## 快速開始

```bash
# 從原始碼建置
git clone https://github.com/urtiger101-tw/sox-rs.git
cd sox-rs
cargo build --release
./target/release/soundx --help
# 或安裝至 Cargo 的 bin 目錄
cargo install --path .
```

```bash
# 查看音檔資訊
soundx info input.wav
soundx info --json input.wav

# 基本轉換與效果
soundx convert input.wav out.wav --gain-db -3 --trim 0 10 --normalize

# SoX 風格傳統語法
soundx input.wav out.wav gain -3 trim 0 10 norm rate 48000 stat

# 串接多檔
soundx concat -o album.wav intro.wav body.wav outro.wav

# 混音
soundx mix -o bed.wav voice.wav music.wav --normalize

# 音訊合成
soundx synth tone.wav --duration 2 --freq 440 --waveform sine --fade 0.05

# 串流處理（低記憶體，適合大型檔案）
soundx stream huge.wav processed.wav --gain-db=-3 --fade-in 0.5 --fade-out 0.5

# 響度量測（BS.1770-4 / EBU R128）與正規化至 -18 LUFS、true peak ≤ -1.8 dBTP
soundx loudness track.wav --json
soundx convert in.wav out.wav --loudness-target=-18 --true-peak=-1.8
soundx stream huge.wav out.wav --loudness-target=-18 --true-peak=-1.8

# 列出支援的編解碼器
soundx formats

# 平行批次轉換
soundx batch "samples/*.wav" --out-dir out --normalize --rate 48000

# 執行 JSON 計畫
soundx run-plan plan.json

# 列出裝置、多檔/循環播放及錄音
soundx devices
soundx play intro.wav chapter1.wav chapter2.wav --loop
soundx record take.wav --duration 10
soundx record live.wav --continuous

# 其他 Rust 原生編碼格式
soundx convert input.wav speech.gsm
soundx convert input.wav speech.amr
soundx convert input.wav speech-wide.awb
soundx convert input.wav lossless.wv
```

---

## 安裝方式

### 預編譯二進位檔

從 [GitHub Releases](https://github.com/stevenke1981/sox-rs/releases) 下載：

| 平台 | 套件 | 執行檔 |
|------|------|--------|
| Windows x86_64 | `soundx-<version>-setup.exe`（安裝程式）或 `soundx-<version>-x86_64-pc-windows-msvc.zip` | `soundx.exe` |
| Linux x86_64 | `soundx-<version>-x86_64-unknown-linux-gnu.tar.gz` | `soundx` |
| macOS x86_64 | `soundx-<version>-x86_64-apple-darwin.tar.gz` | `soundx` |

Windows 安裝程式預設會將安裝目錄加入目前使用者的 `PATH`。安裝後開啟新終端機，執行 `soundx --version` 驗證。

### 從原始碼建置

```bash
cargo install --path .
# 或
cargo build --release
# 執行檔位於 target/release/soundx (或 soundx.exe)
```

---

## 支援的效果

| 效果 | 語法 | 說明 |
|------|------|------|
| `gain` | `gain <db>` | 增益/衰減（分貝） |
| `norm` | `norm [target-db]` | 峰值正規化（預設 −1 dBFS） |
| `trim` | `trim <start-sec> [duration-sec]` | 從指定時間點裁剪 |
| `fade` | `fade <in-sec> [out-sec]` | 線性淡入/淡出 |
| `reverse` | `reverse` | 反轉取樣 |
| `speed` | `speed <factor>` | 改變播放速度 |
| `stretch` / `tempo` | `stretch <factor>` / `tempo [-q|-m|-s|-l] <factor>` | WSOLA 時間伸縮，保留近似音高 |
| `dither` | `dither [bits]` | 確定性 TPDF 抖動與量化 |
| `compand` | `compand attack,decay,... transfer-points [gain [initial-volume [delay]]]` | 依包絡進行動態範圍處理 |
| `reverb` | `reverb [-w] [reverberance [HF-damping [room-scale ...]]]` | Freeverb 風格混響，可只輸出 wet 聲 |
| `pad` | `pad <start-sec> [end-sec]` | 開頭/結尾補靜音 |
| `silence` | `silence <threshold-db> [min-sec]` | 移除前後靜音 |
| `lowpass` | `lowpass [-1|-2] <hz> [width[q|o|h|k]]` | 低通濾波（一階或二階） |
| `highpass` | `highpass [-1|-2] <hz> [width[q|o|h|k]]` | 高通濾波（一階或二階） |
| `bass` / `treble` | `bass|treble <gain-db> [hz [width[s|h|k|q|o]]]` | 低架／高架音調濾波 |
| `allpass` / `bandpass` / `bandreject` | `<effect> <hz> <width[h|k|q|o]>` | 二階全通、帶通與帶阻濾波 |
| `equalizer` | `equalizer <hz> <width[q|o|h|k]> <gain-db>` | 二階參數式峰值等化 |
| `echo` | `echo <gain-in> <gain-out> <delay-ms> <decay> [...]` | 加入一個或多個延遲 tap |
| `tremolo` | `tremolo <speed-hz> [depth-percent]` | 正弦振幅調變 |
| `delay` | `delay <position-sec> [...]` | 延遲全部聲道或逐聲道指定延遲 |
| `dcshift` | `dcshift <shift> [limitergain]` | 加入或移除直流偏移 |
| `downsample` / `upsample` | `[factor，預設 2]` | 抽取樣本或插入零樣本並調整取樣率 |
| `repeat` | `repeat [count，預設 1]` | 重複完整輸入音訊 |
| `swap` | `swap` | 交換前兩個聲道 |
| `limiter` | `limiter [threshold]` | 硬限制器（預設 0.95） |
| `rate` | `rate <hz>` | 重新取樣 |
| `channels` | `channels <count>` | 聲道數量轉換 |
| `stat` | `stat` | 列印音訊統計 |

## 合成波形

- 波形：`sine`（正弦波）、`square`（方波）、`triangle`（三角波）、`saw`（鋸齒波）、`noise`（雜訊）、`silence`（靜音）
- 後處理：`--gain-db`、`--normalize`、`--fade`

## 編解碼支援

| 方向 | 支援格式 |
|------|---------|
| **讀取** | WAV（PCM/float、IMA/MS ADPCM）、GSM 06.10、AMR-NB/WB、WavPack v5、AIFF、AU/SND、RAW、FLAC、MP3、Ogg/Vorbis、Opus、AAC、ALAC、CAF、MKV/WebM、M4A |
| **寫入** | WAV（PCM/float、IMA/MS ADPCM）、GSM 06.10、AMR-NB/WB、WavPack v5、FLAC、MP3、Ogg/Vorbis、AAC/ADTS、AIFF、AU/SND、RAW |
| **串流** | WAV → WAV（gain、fade、limiter） |
| **裝置** | 列出系統裝置、多檔/循環播放、定時/連續 WAV 錄音，可選裝置、取樣率與聲道數 |

WavPack 目前支援 v5 lossless 單聲道/立體聲；GSM 與 AMR 使用單聲道語音影格。多檔播放會先將播放清單載入記憶體；連續錄音透過 bounded queue 寫入 16-bit WAV。此專案仍是 SoX 風格子集，格式與效果限制請見[相容性矩陣](../../docs/SOX_COMPATIBILITY.md)。錄音需要可用輸入裝置及作業系統授權。

`play --loop` 會持續到 Ctrl+C，不受單次播放逾時限制。連續錄音抵達 RIFF WAV 約 4 GiB 的上限時會停止、回報錯誤並完成已錄音部分的標頭；目前不支援 RF64。

RAW 輸入需以 `convert --input-raw-rate HZ --input-raw-channels N` 指定取樣率及聲道數，預設使用 `pcm-s16le`；輸出可用 `--output-raw-encoding` 選格式。AU/SND 可用 `convert --au-encoding pcm8|pcm16|pcm24|pcm32|float32|float64|mu-law|a-law` 選擇編碼，預設為 16-bit PCM。
AIFF 預設 16-bit PCM，可用 `convert --aiff-bits 8|16|24|32` 選擇位元深度。

---

## 專案結構

```
soundx/                    # crate 根目錄
├── Cargo.toml
├── README.md
├── src/
│   ├── main.rs            # 入口、命令分發
│   ├── cli.rs             # Clap 參數定義
│   ├── audio.rs           # AudioBuffer、WAV/AU/RAW 與 Symphonia 輸入
│   ├── encode.rs          # 容器與音訊編碼器
│   ├── codecs.rs          # ADPCM、GSM、AMR、WavPack 編解碼器
│   ├── device.rs          # 裝置列舉、播放、定時/連續錄音
│   ├── effects.rs         # Effect 列舉、EffectChain、DSP
│   ├── parse.rs           # 效果 token 解析器
│   ├── io.rs              # 檔案 I/O 工具
│   ├── mix.rs             # 串接與混音
│   ├── streaming.rs       # 增量 WAV 管線
│   ├── synth.rs           # 波形合成
│   ├── stats.rs           # 音訊統計
│   └── util.rs            # 共用工具函式
├── tests/
│   ├── cli.rs             # CLI 整合測試
│   ├── effects.rs         # 效果單元測試
│   └── common/mod.rs      # 測試輔助工具
├── docs/
│   ├── ARCHITECTURE.md
│   ├── KNOWLEDGE_MAP.md
│   └── ROADMAP.md
└── pages/
    ├── en/README.md
    └── zh-TW/README.md
```

---

## 建置與安裝

```powershell
# Debug 建置
cargo build

# Release 建置
cargo build --release

# 執行檔位於 target/release/soundx (或 soundx.exe)
./target/release/soundx --help
```

**前置需求：** [Rust](https://www.rust-lang.org/tools/install) 1.94.1（由 `rust-toolchain.toml` 固定；AMR 相依套件要求 Rust 1.91+）。

---

## 授權條款

雙重授權：**LGPL-2.1-or-later** 或 **MIT**（任選其一）。
