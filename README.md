# soundx — Rust 原生音訊處理工具

> **SoX 的精神 · Rust 的安全 · 現代化的設計**

`soundx` 是 SoX 風格的純 Rust 音訊處理器，目標是逐步實現命令列、格式、效果與裝置相容。
目前仍是功能子集，尚未完整相容 SoX；請以本文件列出的支援範圍為準。

---

## 快速開始

```bash
# 從原始碼建置
git clone https://github.com/urtiger101-tw/soundx.git
cd soundx
cargo build --release
./target/release/soundx --help

# 安裝至 Cargo 的 bin 目錄
cargo install --path .
```

---

## 使用範例

```bash
# 檢視音訊資訊
soundx info input.wav
soundx info --json input.wav

# 轉換 + 效果處理
soundx convert input.wav output.wav --gain-db -3 --trim 0 10 --normalize

# headerless RAW requires its sample rate and channel count
soundx convert --input-raw-rate 48000 --input-raw-channels 2 --input-raw-encoding pcm-s24be input.raw output.wav
soundx convert --output-raw-encoding float32be input.wav output.raw

# 合成音訊
soundx synth tone.wav --duration 2 --freq 440 --waveform sine

# 串接與混音
soundx concat -o album.wav intro.wav body.wav outro.wav
soundx mix -o bed.wav voice.wav music.wav --normalize

# 批次處理
soundx batch "samples/*.wav" --out-dir out --normalize --rate 48000

# JSON 處理計畫
soundx run-plan plan.json

# 列舉裝置、多檔/循環播放與錄音
soundx devices
soundx play intro.wav chapter1.wav chapter2.wav
soundx play playlist-a.wav playlist-b.wav --loop
soundx record take.wav --duration 10
soundx record live.wav --continuous

# 純 Rust 壓縮格式
soundx convert input.wav compressed.wav --wav-adpcm ima
soundx convert input.wav speech.gsm
soundx convert input.wav speech.amr
soundx convert input.wav speech-wide.awb
soundx convert input.wav lossless.wv

# SoX 風格命令列（相容模式）
soundx input.wav output.wav gain -6 trim 0 0.01 pad 0.01 0.01 speed 1.5 rate 22050

# 串流處理（適合大檔案）
soundx stream input.wav output.wav --gain-db=-3 --fade-in 0.01 --limiter 0.7

# 響度量測（ITU-R BS.1770-4 / EBU R128）與正規化（0.3）
soundx loudness track.wav
soundx loudness a.wav b.flac --json
soundx convert in.wav out.wav --bits 24 --loudness-target=-18 --true-peak=-1.8 --stat-json
soundx stream huge.wav out.wav --loudness-target=-16 --true-peak=-1
```

### 響度量測與正規化（0.3）

`soundx loudness INPUT... [--json]` 依 ITU-R BS.1770-4 / EBU R128 量測：

| 欄位 | 說明 |
|------|------|
| `integrated_lufs` | 整體響度；絕對閘 -70 LUFS、相對閘 -10 LU，400 ms 區塊、75% 重疊。全靜音時為 `null` |
| `loudness_range_lu` | 響度範圍（EBU Tech 3342）：3 秒視窗、-20 LU 相對閘、第 10 至 95 百分位。無資料時為 0 |
| `true_peak_dbtp` | True peak：4 倍過取樣（每相位 48 tap 的 polyphase FIR）；≥96 kHz 用 2 倍、≥192 kHz 不過取樣 |
| `sample_peak_dbfs`、`momentary_max_lufs`、`short_term_max_lufs` | 取樣峰值、瞬時（400 ms）與短期（3 s）最大值 |

K 加權濾波器係數由類比原型經雙線性轉換**依實際取樣率推導**（不限 48 kHz）。聲道權重：L/R/C 為 1.0、環繞聲道為 1.41、LFE 不計（支援單聲道、立體聲、5.1 等 WAV 聲道順序；未知排列一律 1.0）。
量測器為串流累加器，記憶體用量與長度無關（閘控使用 0.01 LU 細分直方圖），WAV 輸入以串流方式讀取，10 小時檔案亦同。

`convert --loudness-target=LUFS [--true-peak=dBTP]`（與 `--normalize` 互斥；負值請寫成 `=` 形式，或 `--loudness-target -18`）會在其他效果之後：量測 → 線性增益至目標 → 若 true peak 會超過上限（預設 -1.0 dBTP）就套用 true-peak limiter。
Limiter 採 2 ms lookahead，以過取樣峰值計算增益，平滑 attack 與 100 ms release，絕不硬削波；延遲已補償，輸出與輸入音框數相同且逐樣本對齊。
Limiter 會降低整體響度，因此啟動時增益會自動微調（最多 4 次量測）使輸出落在目標 ±0.03 LU 附近。單獨使用 `--true-peak` 只做峰值限制、不改變響度。
`--stat-json` 輸出於原有欄位加上 `loudness`：`input`、`output`、`gain_db`、`limited`、`limiter_gain_reduction_max_db`。

`stream IN.wav OUT.wav --loudness-target/--true-peak` 以有界記憶體做相同處理（量測一趟、輸出一趟；limiter 啟動時另需微調量測趟），輸出預設保留輸入位元深度，可用 `--bits 8|16|24|32`、`--float` 指定；不可與 `--gain-db`、`--fade-*`、`--limiter` 併用。

限制：多聲道只內建上述常見排列；取樣率 ≥192 kHz 的 true peak 等同取樣峰值；數值僅在 Windows 驗證（演算法為純 Rust、與平台無關，Linux／macOS 未測）。

### 支援的效果

| 效果 | 說明 |
|------|------|
| `gain` / `vol` | 音量增益（dB） |
| `normalize` | 正規化至目標音量 |
| `trim` | 裁剪音訊區段 |
| `fade` | 淡入淡出 |
| `reverse` | 反轉音訊 |
| `speed` | 變速（線性插值） |
| `stretch` / `tempo` | WSOLA 時間伸縮，保留近似音高 |
| `dither` | TPDF 抖動並量化到指定位元深度 |
| `compand` | attack/decay 包絡與 dB transfer curve 壓縮 |
| `reverb` | Freeverb 風格混響，可輸出 wet-only |
| `pad` | 填補靜音 |
| `silence` | 移除前後靜音 |
| `lowpass` | 低通濾波器 |
| `highpass` | 高通濾波器 |
| `bass` / `treble` | 二階低架／高架音調濾波器 |
| `allpass` / `bandpass` / `bandreject` | 二階全通、帶通與帶阻濾波器 |
| `equalizer` | 二階參數式峰值等化器 |
| `echo` | 一組或多組毫秒延遲/衰減 tap |
| `tremolo` | 正弦振幅調變 |
| `delay` | 依秒數延後全部聲道或分別延後聲道 |
| `dcshift` | 加入或移除直流偏移 |
| `downsample` / `upsample` | 抽取樣本或插入零樣本並調整取樣率 |
| `repeat` | 重複完整輸入音訊指定次數 |
| `swap` | 交換前兩個聲道 |
| `limiter` | 限制器（hard-clamp） |
| `rate` | 重取樣 |
| `channels` | 聲道轉換 |

### 支援的格式

| 方向 | 格式 |
|------|------|
| 讀取 | WAV（PCM/float、IMA/MS ADPCM）、GSM 06.10、AMR-NB/WB、WavPack v5、AIFF、AU/SND、RAW、FLAC、MP3、Ogg/Vorbis、Opus、AAC、ALAC、CAF、MKV/WebM、MP4/M4A |
| 寫入 | WAV（PCM/float、IMA/MS ADPCM）、GSM 06.10、AMR-NB/WB、WavPack v5、FLAC、MP3、Ogg/Vorbis、AAC/ADTS、AIFF、AU/SND、RAW |
| 串流 | WAV → WAV（gain, fade, limiter, 響度正規化） |
| 裝置 | 系統裝置列舉、多檔播放、循環播放、定時/連續 WAV 錄音；可選裝置名稱、取樣率與聲道數 |

格式與效果是 SoX 風格子集，細節、參數和樣本結果不保證與 SoX 完全一致；請參閱 [SoX 相容性矩陣](docs/SOX_COMPATIBILITY.md)。WavPack 僅支援 v5 lossless mono/stereo；AMR 使用 `.amr`/`.awb` 單聲道 storage frames。多檔播放會先在記憶體中串接；連續錄音以 bounded queue 寫 16-bit PCM WAV。錄音需要作業系統授權及可用輸入裝置；播放/錄音使用 CPAL 對接作業系統音訊後端。
RAW 輸入沒有檔頭，需在 `convert` 提供 `--input-raw-rate` 與 `--input-raw-channels`；樣本格式預設為 `pcm-s16le`。RAW 輸出預設 `pcm-s16le`，可用 `--output-raw-encoding` 選擇格式。AU/SND 可用 `--au-encoding pcm8|pcm16|pcm24|pcm32|float32|float64|mu-law|a-law` 選擇編碼。
WAV 可用 `convert --bits 64 --float` 輸出 64-bit float；目前處理管線使用 32-bit float 樣本。AIFF 輸出預設為 16-bit PCM，可用 `convert --aiff-bits 8|16|24|32` 選擇位元深度。

---

## 架構

```
soundx/            # ── crate root
├── src/
│   ├── main.rs    # 入口、命令分發
│   ├── cli.rs     # CLI 參數解析
│   ├── audio.rs   # AudioBuffer、WAV/AU/RAW I/O、Symphonia 解碼
│   ├── encode.rs  # WAV、FLAC、MP3、Vorbis、AAC、AIFF、AU、RAW 編碼
│   ├── device.rs  # 裝置列舉、播放、定時錄音
│   ├── effects.rs # Effect 枚舉、EffectChain、DSP 實作
│   ├── parse.rs   # 效果 token 解析器
│   ├── io.rs      # 檔案 I/O 公用函式
│   ├── mix.rs     # 串接與混音
│   ├── streaming.rs # 增量 WAV 管線、串流響度正規化
│   ├── loudness.rs  # BS.1770-4／EBU R128 量測：K 加權、閘控、LRA、true peak
│   ├── limiter.rs   # lookahead true-peak limiter 與響度正規化流程
│   ├── synth.rs   # 波形合成
│   ├── stats.rs   # 音訊統計報告
│   └── util.rs    # 共用工具函式
├── tests/
│   ├── cli.rs     # CLI 整合測試
│   ├── effects.rs # 效果單元測試
│   ├── loudness.rs # 響度、limiter、CLI／MCP 整合測試（可選 FFmpeg 比對）
│   └── common/    # 測試輔助工具
└── docs/
    ├── ARCHITECTURE.md
    ├── KNOWLEDGE_MAP.md
    └── ROADMAP.md
```

四條處理路徑：
1. **AudioBuffer 路徑**：將輸入解碼為 `f32` 交錯樣本，套用效果鏈，依副檔名編碼
2. **Streaming 路徑**：增量處理 WAV 樣本，適合大檔案
3. **Synth 路徑**：生成內建波形，重複使用效果鏈與格式編碼器
4. **Device 路徑**：CPAL 提供輸入/輸出裝置與音訊串流

---

## 安裝包

### Agent 與 Windows 音訊控制（0.2）

Windows 安裝程式可勾選安裝 Codex、Claude Code、OpenCode、AGY / Antigravity
使用的 soundx Skill 與 MCP 設定。既有設定會保留並備份，同名衝突不會被覆寫。
可攜版可執行 `soundx integrate install --agent codex`（或 `claude`、`opencode`、`agy`）。

`soundx mcp` 直接提供 stdio MCP，讓 agent 轉檔、分析、合成與管理播放／錄音工作；
`soundx windows devices`、`soundx windows endpoint` 與 `soundx windows sessions`
提供 Windows Core Audio 裝置及程式音量／靜音控制。所有原生控制回傳 JSON。
詳細設定、工具參數、移除方式與限制請見 [Agent 整合說明](docs/AGENT_INTEGRATION.md)。

發行流程會產生以下平台的執行檔封裝；裝置功能使用各作業系統的音訊後端：

| 平台 | 格式 |
|------|------|
| Windows x86_64 | `soundx-x.y.z-x86_64-pc-windows-msvc.zip`、`soundx-x.y.z-setup.exe` |
| Linux x86_64 | `soundx-x.y.z-x86_64-unknown-linux-gnu.tar.gz`（`soundx`） |
| macOS x86_64 | `soundx-x.y.z-x86_64-apple-darwin.tar.gz`（`soundx`） |

### 從原始碼建置

```bash
cargo build --release
# 執行檔位於 target/release/soundx (或 soundx.exe)
```

Windows 安裝程式會將安裝目錄加入 PATH；預設安裝至目前使用者，安裝精靈亦可選擇系統層級安裝（需管理員權限）。請在安裝後開啟新的終端機，執行 `soundx --version` 驗證。

以 `pwsh -File scripts/package.ps1` 建立 ZIP 與 Inno Setup 安裝程式。Inno Setup 6 或 7 須已安裝。

Linux 建置需安裝 `pkg-config` 與 ALSA 開發套件（Debian/Ubuntu：`sudo apt-get install pkg-config libasound2-dev`）。封裝包含相容性說明、中英文使用文件及第三方授權聲明。

`play --loop` 持續播放到 Ctrl+C；播放清單先載入記憶體，第一個音軌直接移轉緩衝區以減少複製。連續錄音遇到 RIFF WAV 的 4 GiB 上限會停止並回報錯誤，保存完整音框與可讀取的 WAV 標頭；目前不輸出 RF64。

目前提供 SoX 風格的部分命令與效果，尚未達到完整 SoX 格式、效果及參數相容；請以 `soundx --help` 與 `soundx formats` 檢查已實作範圍。

---

## 授權

**LGPL-2.1-or-later** 或 **MIT**（任選其一）。

---

## 相關連結

- [English Documentation](pages/en/README.md)
- [繁體中文說明](pages/zh-TW/README.md)
- [架構文件](docs/ARCHITECTURE.md)
- [知識圖譜](docs/KNOWLEDGE_MAP.md)
- [開發藍圖](docs/ROADMAP.md)
- [SoX 相容性現況與差距](docs/SOX_COMPATIBILITY.md)
