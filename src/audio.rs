use anyhow::{Context, Result, anyhow, bail};
use clap::ValueEnum;
use hound::{SampleFormat, WavReader, WavSpec, WavWriter};
use serde::Serialize;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use symphonia::core::codecs::audio::{AudioDecoderOptions, CODEC_ID_NULL_AUDIO};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::formats::probe::Hint;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

#[derive(Debug, Clone, Copy, Serialize)]
pub struct AudioSpec {
    pub sample_rate: u32,
    pub channels: u16,
}

#[derive(Debug, Clone, Copy, Default, ValueEnum)]
pub enum RawEncoding {
    PcmU8,
    PcmU16le,
    PcmU16be,
    PcmU24le,
    PcmU24be,
    PcmU32le,
    PcmU32be,
    PcmS8,
    #[default]
    PcmS16le,
    PcmS16be,
    PcmS24le,
    PcmS24be,
    PcmS32le,
    PcmS32be,
    Float32le,
    Float32be,
    Float64le,
    Float64be,
    #[value(name = "mu-law")]
    MuLaw,
    #[value(name = "a-law")]
    ALaw,
}

#[derive(Debug, Clone)]
pub struct AudioBuffer {
    pub spec: AudioSpec,
    pub samples: Vec<f32>,
}

impl AudioBuffer {
    pub fn read(path: &Path) -> Result<Self> {
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default();
        if extension.eq_ignore_ascii_case("wav") {
            return Self::read_wav(path);
        }
        if extension.eq_ignore_ascii_case("au") || extension.eq_ignore_ascii_case("snd") {
            return Self::read_au(path);
        }
        if extension.eq_ignore_ascii_case("raw") {
            bail!(
                "RAW input has no header; use `convert --input-raw-rate HZ --input-raw-channels N` to specify its format"
            );
        }
        if extension.eq_ignore_ascii_case("gsm") {
            return crate::codecs::read_gsm(path);
        }
        if extension.eq_ignore_ascii_case("amr")
            || extension.eq_ignore_ascii_case("awb")
            || extension.eq_ignore_ascii_case("amr-wb")
        {
            return crate::codecs::read_amr(path);
        }
        if extension.eq_ignore_ascii_case("wv") || extension.eq_ignore_ascii_case("wavpack") {
            return crate::codecs::read_wavpack(path);
        }

        Self::read_with_symphonia(path)
    }

    pub fn read_with_raw_options(
        path: &Path,
        raw_sample_rate: Option<u32>,
        raw_channels: Option<u16>,
        raw_encoding: Option<RawEncoding>,
    ) -> Result<Self> {
        let is_raw = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("raw"));
        if !is_raw {
            if raw_sample_rate.is_some() || raw_channels.is_some() || raw_encoding.is_some() {
                bail!("--input-raw-* options are only valid for RAW input");
            }
            return Self::read(path);
        }
        let sample_rate =
            raw_sample_rate.ok_or_else(|| anyhow!("RAW input requires --input-raw-rate"))?;
        let channels =
            raw_channels.ok_or_else(|| anyhow!("RAW input requires --input-raw-channels"))?;
        Self::read_raw(
            path,
            sample_rate,
            channels,
            raw_encoding.unwrap_or_default(),
        )
    }

    fn read_raw(
        path: &Path,
        sample_rate: u32,
        channels: u16,
        encoding: RawEncoding,
    ) -> Result<Self> {
        if sample_rate == 0 || channels == 0 {
            bail!("RAW input sample rate and channel count must be greater than zero");
        }
        let mut bytes = Vec::new();
        File::open(path)?.read_to_end(&mut bytes)?;
        let sample_bytes = raw_sample_bytes(encoding);
        if bytes.len() % sample_bytes != 0 {
            bail!("RAW input ends with an incomplete sample for {encoding:?}");
        }
        let samples = match encoding {
            RawEncoding::PcmU8 => bytes
                .into_iter()
                .map(|sample| ((i16::from(sample) - 128) as f32 / 128.0).clamp(-1.0, 1.0))
                .collect(),
            RawEncoding::PcmU16le => decode_raw_unsigned::<2>(&bytes, false),
            RawEncoding::PcmU16be => decode_raw_unsigned::<2>(&bytes, true),
            RawEncoding::PcmU24le => decode_raw_unsigned::<3>(&bytes, false),
            RawEncoding::PcmU24be => decode_raw_unsigned::<3>(&bytes, true),
            RawEncoding::PcmU32le => decode_raw_unsigned::<4>(&bytes, false),
            RawEncoding::PcmU32be => decode_raw_unsigned::<4>(&bytes, true),
            RawEncoding::PcmS8 => bytes
                .into_iter()
                .map(|sample| (sample as i8 as f32 / 128.0).clamp(-1.0, 1.0))
                .collect(),
            RawEncoding::PcmS16le => decode_raw_signed::<2>(&bytes, false),
            RawEncoding::PcmS16be => decode_raw_signed::<2>(&bytes, true),
            RawEncoding::PcmS24le => decode_raw_signed::<3>(&bytes, false),
            RawEncoding::PcmS24be => decode_raw_signed::<3>(&bytes, true),
            RawEncoding::PcmS32le => decode_raw_signed::<4>(&bytes, false),
            RawEncoding::PcmS32be => decode_raw_signed::<4>(&bytes, true),
            RawEncoding::Float32le => decode_raw_float::<4>(&bytes, false)?,
            RawEncoding::Float32be => decode_raw_float::<4>(&bytes, true)?,
            RawEncoding::Float64le => decode_raw_float::<8>(&bytes, false)?,
            RawEncoding::Float64be => decode_raw_float::<8>(&bytes, true)?,
            RawEncoding::MuLaw => bytes
                .into_iter()
                .map(|sample| decode_mulaw(sample) as f32 / 32768.0)
                .collect(),
            RawEncoding::ALaw => bytes
                .into_iter()
                .map(|sample| decode_alaw(sample) as f32 / 32768.0)
                .collect(),
        };
        if samples.len() % usize::from(channels) != 0 {
            bail!("RAW sample data is not aligned to its channel count");
        }
        Ok(Self {
            spec: AudioSpec {
                sample_rate,
                channels,
            },
            samples,
        })
    }

    fn read_au(path: &Path) -> Result<Self> {
        let mut file = File::open(path)?;
        let mut header = [0_u8; 24];
        file.read_exact(&mut header)
            .with_context(|| "truncated AU header".to_string())?;
        if &header[0..4] != b".snd" {
            bail!("invalid AU magic number");
        }

        let data_offset = u32::from_be_bytes(header[4..8].try_into().unwrap());
        let data_size = u32::from_be_bytes(header[8..12].try_into().unwrap());
        let encoding = u32::from_be_bytes(header[12..16].try_into().unwrap());
        let sample_rate = u32::from_be_bytes(header[16..20].try_into().unwrap());
        let channels = u32::from_be_bytes(header[20..24].try_into().unwrap());
        if data_offset < 24 {
            bail!("invalid AU data offset {data_offset}");
        }
        if sample_rate == 0 || channels == 0 || channels > u16::MAX.into() {
            bail!("invalid AU stream format");
        }

        file.seek(SeekFrom::Start(u64::from(data_offset)))?;
        let mut bytes = Vec::new();
        if data_size == u32::MAX {
            file.read_to_end(&mut bytes)?;
        } else {
            file.take(u64::from(data_size)).read_to_end(&mut bytes)?;
            if bytes.len() != data_size as usize {
                bail!("truncated AU audio data");
            }
        }

        let samples = match encoding {
            1 => bytes
                .into_iter()
                .map(|sample| decode_mulaw(sample) as f32 / 32768.0)
                .collect(),
            2 => bytes
                .into_iter()
                .map(|sample| (sample as i8 as f32 / 128.0).clamp(-1.0, 1.0))
                .collect(),
            3 => decode_au_signed(&bytes, 2, 16)?,
            4 => decode_au_signed(&bytes, 3, 24)?,
            5 => decode_au_signed(&bytes, 4, 32)?,
            6 => decode_au_floats::<4>(&bytes)?,
            7 => decode_au_floats::<8>(&bytes)?,
            27 => bytes
                .into_iter()
                .map(|sample| decode_alaw(sample) as f32 / 32768.0)
                .collect(),
            value => bail!("unsupported AU encoding type {value}"),
        };
        let channels = channels as u16;
        if samples.len() % usize::from(channels) != 0 {
            bail!("AU sample data is not aligned to its channel count");
        }
        Ok(Self {
            spec: AudioSpec {
                sample_rate,
                channels,
            },
            samples,
        })
    }

    pub fn read_wav(path: &Path) -> Result<Self> {
        if let Some(audio) = read_float64_wav(path)? {
            return Ok(audio);
        }
        let mut reader = match WavReader::open(path) {
            Ok(reader) => reader,
            Err(hound_error) => {
                return Self::read_with_symphonia(path)
                    .with_context(|| format!("unsupported or invalid WAV file: {hound_error}"));
            }
        };
        let spec = reader.spec();
        if spec.channels == 0 {
            bail!("WAV file has zero channels");
        }

        let samples = match spec.sample_format {
            // Floating-point samples are kept as-is: values above full scale are
            // legitimate headroom and must survive until something quantizes them.
            SampleFormat::Float => {
                let samples = reader
                    .samples::<f32>()
                    .map(|sample| sample.map_err(anyhow::Error::from))
                    .collect::<Result<Vec<_>>>()?;
                ensure_finite_samples(&samples, "32-bit float WAV")?;
                samples
            }
            SampleFormat::Int => read_int_samples(&mut reader, spec.bits_per_sample)?,
        };

        Ok(Self {
            spec: AudioSpec {
                sample_rate: spec.sample_rate,
                channels: spec.channels,
            },
            samples,
        })
    }

    pub fn write_wav(&self, path: &Path) -> Result<()> {
        self.write_wav_with_format(path, 16, false)
    }

    pub fn write_wav_with_format(&self, path: &Path, bits: u16, float: bool) -> Result<()> {
        if !matches!(bits, 8 | 16 | 24 | 32) || (float && bits != 32) {
            bail!("WAV output requires 8, 16, 24, or 32 bits; float requires 32 bits");
        }
        self.validate()?;
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }

        let spec = WavSpec {
            channels: self.spec.channels,
            sample_rate: self.spec.sample_rate,
            bits_per_sample: bits,
            sample_format: if float {
                SampleFormat::Float
            } else {
                SampleFormat::Int
            },
        };
        let mut writer = WavWriter::create(path, spec)?;
        for sample in &self.samples {
            if float {
                writer.write_sample(*sample)?;
            } else {
                let value = quantize_pcm(*sample, u32::from(bits)) as i32;
                match bits {
                    8 => writer.write_sample(value as i8)?,
                    16 => writer.write_sample(value as i16)?,
                    24 | 32 => writer.write_sample(value)?,
                    _ => unreachable!(),
                }
            }
        }
        writer.finalize()?;
        Ok(())
    }

    /// Check that the buffer can be written: a non-zero channel count and sample
    /// rate, interleaved samples aligned to the channel count, and only finite
    /// samples (NaN or infinity is an error rather than silently becoming
    /// silence or full scale). Values above full scale are allowed; integer
    /// writers clamp them when quantizing and float writers preserve them.
    pub fn validate(&self) -> Result<()> {
        if self.spec.channels == 0 || self.spec.sample_rate == 0 {
            bail!("audio stream must have at least one channel and a non-zero sample rate");
        }
        if !self
            .samples
            .len()
            .is_multiple_of(usize::from(self.spec.channels))
        {
            bail!("interleaved sample count is not divisible by the channel count");
        }
        ensure_finite_samples(&self.samples, "audio stream")
    }

    pub fn frames(&self) -> usize {
        self.samples.len() / usize::from(self.spec.channels)
    }

    pub fn duration_seconds(&self) -> f64 {
        self.frames() as f64 / self.spec.sample_rate as f64
    }

    pub fn peak(&self) -> f32 {
        self.samples
            .iter()
            .map(|sample| sample.abs())
            .fold(0.0_f32, f32::max)
    }

    fn read_with_symphonia(path: &Path) -> Result<Self> {
        let source = Box::new(File::open(path)?);
        let media = MediaSourceStream::new(source, Default::default());
        let mut hint = Hint::new();
        if let Some(extension) = path.extension().and_then(|extension| extension.to_str()) {
            hint.with_extension(extension);
        }

        let mut format = symphonia::default::get_probe().probe(
            &hint,
            media,
            FormatOptions::default(),
            MetadataOptions::default(),
        )?;
        let track = format
            .tracks()
            .iter()
            .find(|track| {
                track
                    .codec_params
                    .as_ref()
                    .and_then(|params| params.audio())
                    .is_some_and(|params| params.codec != CODEC_ID_NULL_AUDIO)
            })
            .ok_or_else(|| anyhow!("no supported audio track found"))?
            .clone();
        let codec_params = track
            .codec_params
            .as_ref()
            .and_then(|params| params.audio())
            .ok_or_else(|| anyhow!("track has no audio codec parameters"))?;
        let sample_rate = codec_params
            .sample_rate
            .ok_or_else(|| anyhow!("codec did not report a sample rate"))?;
        let channels = codec_params
            .channels
            .as_ref()
            .ok_or_else(|| anyhow!("codec did not report a channel layout"))?
            .count() as u16;

        let mut decoder = symphonia::default::get_codecs()
            .make_audio_decoder(codec_params, &AudioDecoderOptions::default())?;
        let mut samples = Vec::new();
        let mut decoded_samples = Vec::new();

        loop {
            let packet = match format.next_packet() {
                Ok(Some(packet)) => packet,
                Ok(None) => break,
                Err(SymphoniaError::IoError(err))
                    if err.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    break;
                }
                Err(SymphoniaError::ResetRequired) => bail!("codec reset required"),
                Err(err) => return Err(err.into()),
            };

            if packet.track_id != track.id {
                continue;
            }

            let decoded = match decoder.decode(&packet) {
                Ok(decoded) => decoded,
                Err(SymphoniaError::DecodeError(_)) => continue,
                Err(err) => return Err(err.into()),
            };

            decoded.copy_to_vec_interleaved(&mut decoded_samples);
            samples.extend_from_slice(&decoded_samples);
        }
        ensure_finite_samples(&samples, "decoded audio")?;

        let mut audio = Self {
            spec: AudioSpec {
                sample_rate,
                channels,
            },
            samples,
        };
        if let Some(frame_count) = read_wav_fact_frame_count(path)? {
            let sample_count = frame_count
                .checked_mul(usize::from(channels))
                .ok_or_else(|| anyhow!("WAV fact sample count overflow"))?;
            if sample_count > audio.samples.len() {
                bail!(
                    "WAV fact chunk declares {frame_count} frames, but the decoder produced only {}",
                    audio.frames()
                );
            }
            audio.samples.truncate(sample_count);
        }
        Ok(audio)
    }
}

fn read_wav_fact_frame_count(path: &Path) -> Result<Option<usize>> {
    let mut file = File::open(path)?;
    let file_len = file.metadata()?.len();
    let mut header = [0_u8; 12];
    if file.read_exact(&mut header).is_err() || &header[..4] != b"RIFF" || &header[8..] != b"WAVE" {
        return Ok(None);
    }
    let riff_end = u64::from(u32::from_le_bytes(header[4..8].try_into().unwrap())) + 8;
    if riff_end > file_len || riff_end < 12 {
        bail!("invalid RIFF/WAVE container length");
    }
    while file.stream_position()? + 8 <= riff_end {
        let mut chunk = [0_u8; 8];
        file.read_exact(&mut chunk)?;
        let chunk_len = u64::from(u32::from_le_bytes(chunk[4..].try_into().unwrap()));
        let content = file.stream_position()?;
        let chunk_end = content
            .checked_add(chunk_len)
            .ok_or_else(|| anyhow!("WAV chunk length overflow"))?;
        if chunk_end > riff_end {
            bail!("WAV chunk extends beyond the RIFF container");
        }
        if &chunk[..4] == b"fact" {
            if chunk_len < 4 {
                bail!("WAV fact chunk is shorter than its sample-count field");
            }
            let mut frames = [0_u8; 4];
            file.read_exact(&mut frames)?;
            return Ok(Some(u32::from_le_bytes(frames) as usize));
        }
        file.seek(SeekFrom::Start(chunk_end + (chunk_len & 1)))?;
    }
    Ok(None)
}

fn read_float64_wav(path: &Path) -> Result<Option<AudioBuffer>> {
    let mut file = File::open(path)?;
    let file_len = file.metadata()?.len();
    let mut container = [0_u8; 12];
    if file.read_exact(&mut container).is_err()
        || &container[..4] != b"RIFF"
        || &container[8..] != b"WAVE"
    {
        return Ok(None);
    }
    let riff_end = u64::from(u32::from_le_bytes(container[4..8].try_into().unwrap())) + 8;
    if riff_end > file_len {
        return Ok(None);
    }

    let mut format = None;
    let mut data_chunk = None;
    while file.stream_position()? + 8 <= riff_end {
        let mut chunk_header = [0_u8; 8];
        file.read_exact(&mut chunk_header)?;
        let chunk_size = u64::from(u32::from_le_bytes(chunk_header[4..8].try_into().unwrap()));
        let chunk_data = file.stream_position()?;
        let chunk_end = chunk_data
            .checked_add(chunk_size)
            .ok_or_else(|| anyhow!("WAV chunk offset overflow"))?;
        let next_chunk = chunk_end
            .checked_add(chunk_size & 1)
            .ok_or_else(|| anyhow!("WAV chunk offset overflow"))?;
        if next_chunk > riff_end {
            return Ok(None);
        }

        match &chunk_header[..4] {
            b"fmt " if chunk_size >= 16 => {
                let mut fmt = [0_u8; 16];
                file.read_exact(&mut fmt)?;
                let format_tag = u16::from_le_bytes(fmt[0..2].try_into().unwrap());
                let channels = u16::from_le_bytes(fmt[2..4].try_into().unwrap());
                let sample_rate = u32::from_le_bytes(fmt[4..8].try_into().unwrap());
                let block_align = u16::from_le_bytes(fmt[12..14].try_into().unwrap());
                let bits_per_sample = u16::from_le_bytes(fmt[14..16].try_into().unwrap());
                if format_tag != 3 || bits_per_sample != 64 {
                    return Ok(None);
                }
                if channels == 0 || sample_rate == 0 {
                    bail!("WAV file has an invalid channel count or sample rate");
                }
                let expected_block_align = channels
                    .checked_mul(8)
                    .ok_or_else(|| anyhow!("64-bit float WAV channel count is too large"))?;
                if block_align != expected_block_align {
                    bail!("64-bit float WAV has an invalid block alignment");
                }
                format = Some((channels, sample_rate, block_align));
            }
            b"data" => data_chunk = Some((chunk_data, chunk_size)),
            _ => {}
        }
        file.seek(SeekFrom::Start(next_chunk))?;
        if let (Some((channels, sample_rate, block_align)), Some((data_start, data_size))) =
            (format, data_chunk)
        {
            if data_size % u64::from(block_align) != 0 {
                bail!("64-bit float WAV data is not aligned to its channel count");
            }
            let sample_count = usize::try_from(data_size / 8)
                .context("64-bit float WAV is too large for this platform")?;
            let mut samples = Vec::new();
            samples
                .try_reserve_exact(sample_count)
                .context("not enough memory to read 64-bit float WAV")?;
            file.seek(SeekFrom::Start(data_start))?;
            for _ in 0..sample_count {
                let mut bytes = [0_u8; 8];
                file.read_exact(&mut bytes)?;
                // No clamping: values above full scale are preserved. A finite
                // f64 beyond the f32 range would become infinite, so check after
                // the conversion.
                let sample = f64::from_le_bytes(bytes) as f32;
                if !sample.is_finite() {
                    bail!("64-bit float WAV contains NaN, infinite or out-of-range samples");
                }
                samples.push(sample);
            }
            return Ok(Some(AudioBuffer {
                spec: AudioSpec {
                    sample_rate,
                    channels,
                },
                samples,
            }));
        }
    }
    if format.is_some() {
        bail!("64-bit float WAV is missing a data chunk");
    }
    Ok(None)
}

fn read_int_samples(
    reader: &mut WavReader<std::io::BufReader<std::fs::File>>,
    bits: u16,
) -> Result<Vec<f32>> {
    match bits {
        0 => bail!("WAV file has zero bits per sample"),
        1..=8 => {
            let scale = (1_i32 << (bits - 1)) as f32;
            reader
                .samples::<i8>()
                .map(|sample| {
                    sample
                        .map(|value| value as f32 / scale)
                        .map_err(anyhow::Error::from)
                })
                .collect()
        }
        9..=16 => {
            let scale = (1_i32 << (bits - 1)) as f32;
            reader
                .samples::<i16>()
                .map(|sample| {
                    sample
                        .map(|value| value as f32 / scale)
                        .map_err(anyhow::Error::from)
                })
                .collect()
        }
        17..=24 => {
            let scale = (1_i64 << (bits - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|sample| {
                    sample
                        .map(|value| value as f32 / scale)
                        .map_err(anyhow::Error::from)
                })
                .collect()
        }
        25..=32 => {
            let scale = (1_i64 << (bits - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|sample| {
                    sample
                        .map(|value| value as f32 / scale)
                        .map_err(anyhow::Error::from)
                })
                .collect()
        }
        _ => Err(anyhow!("unsupported integer WAV depth: {bits}")),
    }
}

fn decode_au_signed(bytes: &[u8], bytes_per_sample: usize, bits: u32) -> Result<Vec<f32>> {
    if !bytes.len().is_multiple_of(bytes_per_sample) {
        bail!("truncated AU PCM sample");
    }
    let full_scale = (1_i64 << (bits - 1)) as f32;
    Ok(bytes
        .chunks_exact(bytes_per_sample)
        .map(|chunk| {
            let value = match bytes_per_sample {
                2 => i16::from_be_bytes([chunk[0], chunk[1]]) as i64,
                3 => {
                    let sign = if chunk[0] & 0x80 == 0 { 0 } else { 0xff };
                    i32::from_be_bytes([sign, chunk[0], chunk[1], chunk[2]]) as i64
                }
                4 => i32::from_be_bytes(chunk.try_into().unwrap()) as i64,
                _ => unreachable!(),
            };
            value as f32 / full_scale
        })
        .collect())
}

fn decode_au_floats<const N: usize>(bytes: &[u8]) -> Result<Vec<f32>> {
    if !bytes.len().is_multiple_of(N) {
        bail!("truncated AU floating-point sample");
    }
    let samples: Vec<f32> = bytes
        .chunks_exact(N)
        .map(|chunk| {
            if N == 4 {
                f32::from_be_bytes(chunk.try_into().unwrap())
            } else {
                f64::from_be_bytes(chunk.try_into().unwrap()) as f32
            }
        })
        .collect();
    ensure_finite_samples(&samples, "AU floating-point input")?;
    Ok(samples)
}

/// Reject NaN and infinite samples with a readable error. Floating-point
/// readers call this instead of clamping, so over-full-scale values survive.
pub(crate) fn ensure_finite_samples(samples: &[f32], what: &str) -> Result<()> {
    if let Some(index) = samples.iter().position(|sample| !sample.is_finite()) {
        bail!(
            "{what} contains a NaN, infinite or out-of-range sample (sample index {index}, value {})",
            samples[index]
        );
    }
    Ok(())
}

fn decode_mulaw(sample: u8) -> i16 {
    let value = !sample;
    let magnitude = ((((value & 0x0f) as i32) << 3) + 0x84) << ((value >> 4) & 0x07);
    let decoded = if value & 0x80 != 0 {
        0x84 - magnitude
    } else {
        magnitude - 0x84
    };
    decoded as i16
}

fn decode_alaw(sample: u8) -> i16 {
    let value = sample ^ 0x55;
    let segment = (value & 0x70) >> 4;
    let mut magnitude = ((value & 0x0f) as i32) << 4;
    magnitude += if segment == 0 { 8 } else { 0x108 };
    if segment > 1 {
        magnitude <<= segment - 1;
    }
    if value & 0x80 != 0 {
        magnitude as i16
    } else {
        -(magnitude as i16)
    }
}

fn raw_sample_bytes(encoding: RawEncoding) -> usize {
    match encoding {
        RawEncoding::PcmU8 | RawEncoding::PcmS8 | RawEncoding::MuLaw | RawEncoding::ALaw => 1,
        RawEncoding::PcmU16le
        | RawEncoding::PcmU16be
        | RawEncoding::PcmS16le
        | RawEncoding::PcmS16be => 2,
        RawEncoding::PcmU24le
        | RawEncoding::PcmU24be
        | RawEncoding::PcmS24le
        | RawEncoding::PcmS24be => 3,
        RawEncoding::PcmU32le
        | RawEncoding::PcmU32be
        | RawEncoding::PcmS32le
        | RawEncoding::PcmS32be
        | RawEncoding::Float32le
        | RawEncoding::Float32be => 4,
        RawEncoding::Float64le | RawEncoding::Float64be => 8,
    }
}

/// Quantize a sample to a signed `bits`-bit integer. This is where samples
/// above full scale are clamped to [-1, 1]; floating-point paths never clamp.
pub(crate) fn quantize_pcm(sample: f32, bits: u32) -> i64 {
    let midpoint = 1_i64 << (bits - 1);
    let min = -midpoint;
    let max = midpoint - 1;
    let internal = (f64::from(sample.clamp(-1.0, 1.0)) * 2_147_483_648.0).round() as i64;
    if bits == 32 {
        internal.clamp(min, max)
    } else {
        let shift = 32 - bits;
        let rounding = 1_i64 << (shift - 1);
        ((internal + rounding) >> shift).clamp(min, max)
    }
}

fn decode_raw_signed<const N: usize>(bytes: &[u8], big_endian: bool) -> Vec<f32> {
    let bits = N * 8;
    let full_scale = (1_i64 << (bits - 1)) as f64;
    bytes
        .chunks_exact(N)
        .map(|chunk| {
            let mut value = 0_i64;
            if big_endian {
                for byte in chunk {
                    value = (value << 8) | i64::from(*byte);
                }
            } else {
                for (shift, byte) in chunk.iter().enumerate() {
                    value |= i64::from(*byte) << (shift * 8);
                }
            }
            let sign_bit = 1_i64 << (bits - 1);
            if value & sign_bit != 0 {
                value -= 1_i64 << bits;
            }
            (value as f64 / full_scale) as f32
        })
        .collect()
}

fn decode_raw_unsigned<const N: usize>(bytes: &[u8], big_endian: bool) -> Vec<f32> {
    let bits = N * 8;
    let midpoint = 1_u64 << (bits - 1);
    let full_scale = midpoint as f64;
    bytes
        .chunks_exact(N)
        .map(|chunk| {
            let mut value = 0_u64;
            if big_endian {
                for byte in chunk {
                    value = (value << 8) | u64::from(*byte);
                }
            } else {
                for (shift, byte) in chunk.iter().enumerate() {
                    value |= u64::from(*byte) << (shift * 8);
                }
            }
            ((value as f64 - midpoint as f64) / full_scale).clamp(-1.0, 1.0) as f32
        })
        .collect()
}

fn decode_raw_float<const N: usize>(bytes: &[u8], big_endian: bool) -> Result<Vec<f32>> {
    let samples: Vec<f32> = bytes
        .chunks_exact(N)
        .map(|chunk| {
            if N == 4 {
                let value: [u8; 4] = chunk.try_into().unwrap();
                if big_endian {
                    f32::from_be_bytes(value)
                } else {
                    f32::from_le_bytes(value)
                }
            } else {
                let value: [u8; 8] = chunk.try_into().unwrap();
                let float = if big_endian {
                    f64::from_be_bytes(value)
                } else {
                    f64::from_le_bytes(value)
                };
                float as f32
            }
        })
        .collect();
    ensure_finite_samples(&samples, "RAW floating-point input")?;
    Ok(samples)
}
