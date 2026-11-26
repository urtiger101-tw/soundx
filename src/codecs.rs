use crate::audio::{AudioBuffer, AudioSpec};
use crate::effects::{Effect, EffectChain};
use anyhow::{Context, Result, anyhow, bail};
use clap::ValueEnum;
use codec_core::codecs::amr::{AmrCodec, AmrFrameType, AmrMode, AmrPayloadFrame, AmrVariant};
use codec_core::types::{CodecConfig, CodedFrame, FrameKind, VariableRateCodec};
use oxideav_gsm::{DecoderState, EncoderState, FRAME_SAMPLES, GSM_BYTE_FRAME_LEN, UnpackedFrame};
use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::Path;

const GSM_SAMPLE_RATE: u32 = 8_000;
const ADPCM_BLOCK_BYTES_PER_CHANNEL: usize = 256;

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum WavAdpcmEncoding {
    Ima,
    Ms,
}

pub fn read_gsm(path: &Path) -> Result<AudioBuffer> {
    let mut bytes = Vec::new();
    File::open(path)
        .with_context(|| format!("failed to open {}", path.display()))?
        .read_to_end(&mut bytes)?;
    decode_gsm(&bytes).with_context(|| format!("invalid GSM 06.10 file {}", path.display()))
}

pub fn read_amr(path: &Path) -> Result<AudioBuffer> {
    let bytes =
        std::fs::read(path).with_context(|| format!("failed to open {}", path.display()))?;
    decode_amr(&bytes).with_context(|| format!("invalid AMR file {}", path.display()))
}

fn decode_amr(bytes: &[u8]) -> Result<AudioBuffer> {
    let (variant, frames) = codec_core::codecs::amr::storage::read(bytes)
        .map_err(|error| anyhow!(error.to_string()))?;
    let config = match variant {
        AmrVariant::NarrowBand => CodecConfig::amr_nb().with_amr_dtx(false),
        AmrVariant::WideBand => CodecConfig::amr_wb().with_amr_dtx(false),
    };
    let mut decoder = AmrCodec::new(&config).map_err(|error| anyhow!(error.to_string()))?;
    let capacity = frames
        .len()
        .checked_mul(variant.frame_samples())
        .ok_or_else(|| anyhow!("AMR decoded sample count overflow"))?;
    let mut samples = Vec::new();
    samples.try_reserve_exact(capacity)?;
    for frame in frames {
        let (kind, mode) = match frame.frame_type {
            AmrFrameType::Speech(mode) => (FrameKind::Speech, mode.index()),
            AmrFrameType::Sid(_) => (FrameKind::ComfortNoise, 0),
            AmrFrameType::NoData => (FrameKind::NoData, 0),
            AmrFrameType::SpeechLost => (FrameKind::Lost, 0),
        };
        let decoded = decoder
            .decode_frame(&CodedFrame {
                kind,
                mode,
                quality_ok: frame.quality_ok,
                data: frame.data,
            })
            .map_err(|error| anyhow!(error.to_string()))?;
        samples.extend(
            decoded
                .into_iter()
                .map(|sample| f32::from(sample) / 32768.0),
        );
    }
    Ok(AudioBuffer {
        spec: AudioSpec {
            sample_rate: variant.sample_rate(),
            channels: 1,
        },
        samples,
    })
}

pub fn write_amr(audio: &AudioBuffer, output: &Path, variant: AmrVariant) -> Result<()> {
    validate_audio(audio)?;
    let mut converted = audio.clone();
    EffectChain::new(vec![
        Effect::Rate {
            sample_rate: variant.sample_rate(),
        },
        Effect::Channels { channels: 1 },
    ])
    .apply(&mut converted)
    .with_context(|| format!("failed to convert audio for {variant}"))?;

    let config = match variant {
        AmrVariant::NarrowBand => CodecConfig::amr_nb().with_amr_dtx(false),
        AmrVariant::WideBand => CodecConfig::amr_wb().with_amr_dtx(false),
    };
    let mut encoder = AmrCodec::new(&config).map_err(|error| anyhow!(error.to_string()))?;
    let frame_samples = variant.frame_samples();
    let frame_count = converted.frames().div_ceil(frame_samples);
    let mut pcm = vec![0_i16; frame_samples];
    let mut frames = Vec::new();
    frames.try_reserve_exact(frame_count)?;
    for frame_index in 0..frame_count {
        pcm.fill(0);
        let first = frame_index * frame_samples;
        for (destination, sample) in pcm.iter_mut().zip(&converted.samples[first..]) {
            *destination = normalized_to_i16(*sample)?;
        }
        let coded = encoder
            .encode_frame(&pcm)
            .map_err(|error| anyhow!(error.to_string()))?;
        let frame_type = match coded.kind {
            FrameKind::Speech => AmrFrameType::Speech(
                AmrMode::new(variant, coded.mode).map_err(|error| anyhow!(error.to_string()))?,
            ),
            FrameKind::ComfortNoise => AmrFrameType::Sid(variant),
            FrameKind::NoData => AmrFrameType::NoData,
            FrameKind::Lost => bail!("AMR encoder unexpectedly emitted a lost frame"),
        };
        frames.push(
            AmrPayloadFrame::new(frame_type, coded.quality_ok, coded.data)
                .map_err(|error| anyhow!(error.to_string()))?,
        );
    }
    let encoded = codec_core::codecs::amr::storage::write(variant, &frames)
        .map_err(|error| anyhow!(error.to_string()))?;
    File::create(output)?.write_all(&encoded)?;
    Ok(())
}

fn decode_gsm(bytes: &[u8]) -> Result<AudioBuffer> {
    if !bytes.len().is_multiple_of(GSM_BYTE_FRAME_LEN) {
        bail!(
            "GSM 06.10 input must contain whole {GSM_BYTE_FRAME_LEN}-byte frames; got {} bytes",
            bytes.len()
        );
    }
    let mut decoder = DecoderState::new();
    let mut samples = Vec::with_capacity(bytes.len() / GSM_BYTE_FRAME_LEN * FRAME_SAMPLES);
    for frame in bytes.chunks_exact(GSM_BYTE_FRAME_LEN) {
        let unpacked = UnpackedFrame::from_gsm_byte_frame(frame)?;
        samples.extend(
            decoder
                .decode_frame(&unpacked)
                .into_iter()
                .map(|sample| f32::from(sample) / 32768.0),
        );
    }
    Ok(AudioBuffer {
        spec: AudioSpec {
            sample_rate: GSM_SAMPLE_RATE,
            channels: 1,
        },
        samples,
    })
}

pub fn write_gsm(audio: &AudioBuffer, output: &Path) -> Result<()> {
    validate_audio(audio)?;
    let mut converted = audio.clone();
    EffectChain::new(vec![
        Effect::Rate {
            sample_rate: GSM_SAMPLE_RATE,
        },
        Effect::Channels { channels: 1 },
    ])
    .apply(&mut converted)
    .context("failed to convert audio to mono 8 kHz for GSM 06.10")?;

    let mut encoder = EncoderState::new();
    let mut writer = BufWriter::new(File::create(output)?);
    for input in converted.samples.chunks(FRAME_SAMPLES) {
        let mut frame_samples = [0_i16; FRAME_SAMPLES];
        for (destination, sample) in frame_samples.iter_mut().zip(input) {
            *destination = normalized_to_i16(*sample)?;
        }
        writer.write_all(&encoder.encode_frame(&frame_samples).to_gsm_byte_frame())?;
    }
    writer.flush()?;
    Ok(())
}

pub fn read_wavpack(path: &Path) -> Result<AudioBuffer> {
    let bytes =
        std::fs::read(path).with_context(|| format!("failed to open {}", path.display()))?;
    decode_wavpack(&bytes).with_context(|| format!("invalid WavPack file {}", path.display()))
}

fn decode_wavpack(bytes: &[u8]) -> Result<AudioBuffer> {
    let decoded = wavicle::decode_stream(bytes).map_err(|error| anyhow!(error.to_string()))?;
    if !(1..=2).contains(&decoded.channels) {
        bail!(
            "WavPack v5 supports mono or stereo in this build; file has {} channels",
            decoded.channels
        );
    }
    if decoded.sample_rate == 0 || decoded.samples.len() % decoded.channels as usize != 0 {
        bail!("WavPack stream has invalid sample-rate or channel alignment");
    }

    let samples = if decoded.is_float {
        // Float WavPack keeps values above full scale; only NaN/Inf is rejected.
        let samples: Vec<f32> = decoded
            .samples
            .into_iter()
            .map(|bits| f32::from_bits(bits as u32))
            .collect();
        crate::audio::ensure_finite_samples(&samples, "WavPack float stream")?;
        samples
    } else {
        if !(8..=32).contains(&decoded.bits_per_sample) {
            bail!(
                "WavPack integer depth {} is unsupported; expected 8 to 32 bits",
                decoded.bits_per_sample
            );
        }
        let scale = 2_f64.powi(decoded.bits_per_sample as i32 - 1);
        decoded
            .samples
            .into_iter()
            .map(|sample| (f64::from(sample) / scale) as f32)
            .collect()
    };

    Ok(AudioBuffer {
        spec: AudioSpec {
            sample_rate: decoded.sample_rate,
            channels: decoded.channels as u16,
        },
        samples,
    })
}

pub fn write_wavpack(audio: &AudioBuffer, output: &Path) -> Result<()> {
    validate_audio(audio)?;
    if !(1..=2).contains(&audio.spec.channels) {
        bail!("WavPack v5 output supports mono or stereo audio");
    }
    let bytes = wavicle::encode_float(
        u32::from(audio.spec.channels),
        audio.spec.sample_rate,
        &audio.samples,
    )
    .map_err(|error| anyhow!(error.to_string()))?;
    File::create(output)?.write_all(&bytes)?;
    Ok(())
}

pub fn write_wav_adpcm(
    audio: &AudioBuffer,
    output: &Path,
    encoding: WavAdpcmEncoding,
) -> Result<()> {
    validate_audio(audio)?;
    if audio.spec.sample_rate == 0 {
        bail!("WAV ADPCM sample rate must be greater than zero");
    }
    let channels = usize::from(audio.spec.channels);
    let channel_limit = match encoding {
        WavAdpcmEncoding::Ima => 8,
        WavAdpcmEncoding::Ms => 2,
    };
    if channels == 0 || channels > channel_limit {
        bail!("WAV {encoding:?} ADPCM supports 1 to {channel_limit} channels");
    }

    let block_align = ADPCM_BLOCK_BYTES_PER_CHANNEL
        .checked_mul(channels)
        .ok_or_else(|| anyhow!("ADPCM block size overflow"))?;
    let (format_tag, samples_per_block, extra) = match encoding {
        WavAdpcmEncoding::Ima => {
            let body_bytes = block_align - 4 * channels;
            let samples = 1 + (body_bytes * 2) / channels;
            (0x0011_u16, samples, (samples as u16).to_le_bytes().to_vec())
        }
        WavAdpcmEncoding::Ms => {
            let body_bytes = block_align - 7 * channels;
            let samples = 2 + (body_bytes * 2) / channels;
            let samples_u16 = u16::try_from(samples)
                .context("MS ADPCM samples per block exceeds the WAV field limit")?;
            let extra = oxideav_adpcm::ms::build_extradata(
                samples_u16,
                &oxideav_adpcm::ms::STANDARD_COEFFS,
            )?;
            (0x0002_u16, samples, extra)
        }
    };
    let block_align_u16 = u16::try_from(block_align).context("ADPCM block is too large")?;
    let samples_per_block_u32 = u32::try_from(samples_per_block)?;
    let byte_rate = u64::from(audio.spec.sample_rate)
        .checked_mul(block_align as u64)
        .ok_or_else(|| anyhow!("ADPCM byte rate overflow"))?
        / u64::from(samples_per_block_u32);
    let byte_rate = u32::try_from(byte_rate).context("ADPCM byte rate exceeds the WAV limit")?;
    let frames = u32::try_from(audio.frames())
        .context("ADPCM WAV exceeds the 32-bit fact sample-count limit")?;
    let blocks = audio.frames().div_ceil(samples_per_block);
    let data_size = block_align
        .checked_mul(blocks)
        .ok_or_else(|| anyhow!("ADPCM data size overflow"))?;
    let fmt_size = 18_u32
        .checked_add(u32::try_from(extra.len())?)
        .ok_or_else(|| anyhow!("ADPCM fmt chunk size overflow"))?;
    let riff_size = 4_u64 + 8 + u64::from(fmt_size) + 12 + 8 + data_size as u64;
    let riff_size = u32::try_from(riff_size).context("ADPCM WAV exceeds the RIFF 4 GiB limit")?;
    let extra_len = u16::try_from(extra.len()).context("ADPCM fmt extension is too large")?;

    let mut writer = BufWriter::new(File::create(output)?);
    writer.write_all(b"RIFF")?;
    writer.write_all(&riff_size.to_le_bytes())?;
    writer.write_all(b"WAVEfmt ")?;
    writer.write_all(&fmt_size.to_le_bytes())?;
    writer.write_all(&format_tag.to_le_bytes())?;
    writer.write_all(&audio.spec.channels.to_le_bytes())?;
    writer.write_all(&audio.spec.sample_rate.to_le_bytes())?;
    writer.write_all(&byte_rate.to_le_bytes())?;
    writer.write_all(&block_align_u16.to_le_bytes())?;
    writer.write_all(&4_u16.to_le_bytes())?;
    writer.write_all(&extra_len.to_le_bytes())?;
    writer.write_all(&extra)?;
    writer.write_all(b"fact")?;
    writer.write_all(&4_u32.to_le_bytes())?;
    writer.write_all(&frames.to_le_bytes())?;
    writer.write_all(b"data")?;
    writer.write_all(
        &u32::try_from(data_size)
            .context("ADPCM data exceeds the RIFF 4 GiB limit")?
            .to_le_bytes(),
    )?;

    let mut pcm = vec![0_i16; samples_per_block * channels];
    for block in 0..blocks {
        let first_frame = block * samples_per_block;
        pcm.fill(0);
        for frame in 0..samples_per_block {
            let source_frame = first_frame + frame;
            if source_frame >= audio.frames() {
                break;
            }
            for channel in 0..channels {
                pcm[frame * channels + channel] =
                    normalized_to_i16(audio.samples[source_frame * channels + channel])?;
            }
        }
        let encoded = match encoding {
            WavAdpcmEncoding::Ima => {
                oxideav_adpcm::encoder::ima_encode_block(&pcm, channels, block_align)?
            }
            WavAdpcmEncoding::Ms => {
                oxideav_adpcm::encoder::encode_block(&pcm, channels, block_align)?
            }
        };
        if encoded.len() != block_align {
            bail!(
                "ADPCM encoder returned {} bytes for a {block_align}-byte block",
                encoded.len()
            );
        }
        writer.write_all(&encoded)?;
    }
    writer.flush()?;
    Ok(())
}

fn normalized_to_i16(sample: f32) -> Result<i16> {
    if !sample.is_finite() {
        bail!("GSM/ADPCM encoders require finite audio samples");
    }
    Ok((sample.clamp(-1.0, 1.0) * 32768.0)
        .round()
        .clamp(-32768.0, 32767.0) as i16)
}

fn validate_audio(audio: &AudioBuffer) -> Result<()> {
    // Rejects NaN/infinite samples too, so the float WavPack writer cannot
    // store them; the integer GSM/AMR/ADPCM paths clamp in `normalized_to_i16`.
    audio.validate()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

    fn test_path(extension: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "soundx-codec-test-{}-{}.{}",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed),
            extension
        ))
    }

    fn sine_audio(frames: usize, channels: u16, sample_rate: u32) -> AudioBuffer {
        let samples = (0..frames)
            .flat_map(|frame| {
                let sample = (std::f32::consts::TAU * 440.0 * frame as f32 / sample_rate as f32)
                    .sin()
                    * 0.55;
                std::iter::repeat_n(sample, usize::from(channels))
            })
            .collect();
        AudioBuffer {
            spec: AudioSpec {
                sample_rate,
                channels,
            },
            samples,
        }
    }

    #[test]
    fn wavpack_float_stream_round_trips_bit_exactly() {
        let audio = sine_audio(257, 2, 44_100);
        let encoded = wavicle::encode_float(2, 44_100, &audio.samples).unwrap();
        let decoded = decode_wavpack(&encoded).unwrap();
        assert_eq!(decoded.spec.channels, 2);
        assert_eq!(decoded.spec.sample_rate, 44_100);
        assert_eq!(
            decoded
                .samples
                .iter()
                .map(|sample| sample.to_bits())
                .collect::<Vec<_>>(),
            audio
                .samples
                .iter()
                .map(|sample| sample.to_bits())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn gsm_frames_encode_and_decode_at_standard_frame_boundaries() {
        let audio = sine_audio(FRAME_SAMPLES, 1, GSM_SAMPLE_RATE);
        let path = test_path("gsm");
        write_gsm(&audio, &path).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let decoded = decode_gsm(&bytes).unwrap();
        let _ = std::fs::remove_file(path);
        assert_eq!(bytes.len(), GSM_BYTE_FRAME_LEN);
        assert_eq!(decoded.spec.sample_rate, GSM_SAMPLE_RATE);
        assert_eq!(decoded.samples.len(), FRAME_SAMPLES);
        assert!(decoded.samples.iter().all(|sample| sample.is_finite()));
        assert!(decode_gsm(&bytes[..bytes.len() - 1]).is_err());
    }

    #[test]
    fn amr_narrowband_and_wideband_storage_round_trip() {
        for (variant, rate, extension) in [
            (AmrVariant::NarrowBand, 8_000, "amr"),
            (AmrVariant::WideBand, 16_000, "awb"),
        ] {
            let audio = sine_audio(variant.frame_samples() * 5, 1, rate);
            let path = test_path(extension);
            write_amr(&audio, &path, variant).unwrap();
            let bytes = std::fs::read(&path).unwrap();
            assert!(bytes.starts_with(variant.storage_magic()));
            let decoded = AudioBuffer::read(&path).unwrap();
            let _ = std::fs::remove_file(path);
            assert_eq!(decoded.spec.sample_rate, rate);
            assert_eq!(decoded.spec.channels, 1);
            assert_eq!(decoded.frames(), audio.frames());
            assert!(decoded.samples.iter().all(|sample| sample.is_finite()));
            assert!(decoded.samples.iter().any(|sample| sample.abs() > 0.01));
        }
    }

    #[test]
    fn amr_rejects_truncated_storage_records() {
        assert!(decode_amr(b"#!AMR\n\x3c\x00").is_err());
        assert!(decode_amr(b"#!AMR-WB\n\x3c\x00").is_err());
    }

    #[test]
    fn ima_and_ms_adpcm_wav_round_trip_through_symphonia() {
        let audio = sine_audio(1_337, 2, 22_050);
        for encoding in [WavAdpcmEncoding::Ima, WavAdpcmEncoding::Ms] {
            let path = test_path("wav");
            write_wav_adpcm(&audio, &path, encoding).unwrap();
            let decoded = AudioBuffer::read(&path).unwrap();
            let _ = std::fs::remove_file(path);
            assert_eq!(decoded.spec.sample_rate, audio.spec.sample_rate);
            assert_eq!(decoded.spec.channels, audio.spec.channels);
            assert_eq!(decoded.frames(), audio.frames());
            assert!(decoded.samples.iter().all(|sample| sample.is_finite()));
            assert!(decoded.samples.iter().any(|sample| sample.abs() > 0.01));
        }
    }
}
