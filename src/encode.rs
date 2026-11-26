use crate::audio::{AudioBuffer, RawEncoding, quantize_pcm};
use anyhow::{Context, Result, bail};
use clap::ValueEnum;
use ogg::{PacketWriteEndInfo, PacketWriter};
use rusty_aac::{AacEncoder, AacEncoderConfig, AdtsHeader, Error as AacError};
use rusty_flac::Encoder as FlacEncoder;
use rusty_mp3::{Error as Mp3Error, Mp3Encoder, Mp3EncoderConfig};
use rusty_vorbis::{Error as VorbisError, VorbisEncoder, VorbisEncoderConfig};
use std::borrow::Cow;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, Default, ValueEnum)]
pub enum AuEncoding {
    Pcm8,
    #[default]
    Pcm16,
    Pcm24,
    Pcm32,
    Float32,
    Float64,
    #[value(name = "mu-law")]
    MuLaw,
    #[value(name = "a-law")]
    ALaw,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct EncodeOptions {
    pub wav_bits: Option<u16>,
    pub wav_float: bool,
    pub wav_adpcm: Option<crate::codecs::WavAdpcmEncoding>,
    pub aiff_bits: Option<u16>,
    pub bitrate_kbps: u32,
    pub au_encoding: Option<AuEncoding>,
    pub raw_encoding: Option<RawEncoding>,
}

/// Write audio using the format selected by the output extension.
pub fn write_audio(
    audio: &AudioBuffer,
    output: &Path,
    wav_bits: Option<u16>,
    wav_float: bool,
    bitrate_kbps: u32,
) -> Result<()> {
    write_audio_with_au_encoding(audio, output, wav_bits, wav_float, bitrate_kbps, None)
}

/// Write audio, optionally selecting one of the AU/SND encodings.
pub fn write_audio_with_au_encoding(
    audio: &AudioBuffer,
    output: &Path,
    wav_bits: Option<u16>,
    wav_float: bool,
    bitrate_kbps: u32,
    au_encoding: Option<AuEncoding>,
) -> Result<()> {
    write_audio_with_options(
        audio,
        output,
        EncodeOptions {
            wav_bits,
            wav_float,
            bitrate_kbps,
            au_encoding,
            ..EncodeOptions::default()
        },
    )
}

pub fn write_audio_with_options(
    audio: &AudioBuffer,
    output: &Path,
    options: EncodeOptions,
) -> Result<()> {
    let extension = output
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if extension != "wav" && (options.wav_bits.is_some() || options.wav_float) {
        bail!("--bits and --float are only valid for WAV output");
    }
    if options.wav_adpcm.is_some() && extension != "wav" {
        bail!("--wav-adpcm is only valid for WAV output");
    }
    if options.wav_adpcm.is_some() && (options.wav_bits.is_some() || options.wav_float) {
        bail!("--wav-adpcm cannot be combined with --bits or --float");
    }
    if extension == "wav" {
        let bits = options.wav_bits.unwrap_or(16);
        if !matches!(bits, 8 | 16 | 24 | 32 | 64) {
            bail!("WAV output supports 8, 16, 24, 32, or 64 bits");
        }
        if options.wav_float && !matches!(bits, 32 | 64) {
            bail!("floating-point WAV output requires --bits 32 or --bits 64");
        }
        if bits == 64 && !options.wav_float {
            bail!("64-bit WAV output requires --float");
        }
    }
    if options.au_encoding.is_some() && extension != "au" && extension != "snd" {
        bail!("--au-encoding is only valid for AU/SND output");
    }
    if options.raw_encoding.is_some() && extension != "raw" {
        bail!("--output-raw-encoding is only valid for RAW output");
    }
    if options.aiff_bits.is_some() && extension != "aif" && extension != "aiff" {
        bail!("--aiff-bits is only valid for AIFF output");
    }

    ensure_parent(output)?;
    match extension.as_str() {
        "wav" => {
            if let Some(encoding) = options.wav_adpcm {
                crate::codecs::write_wav_adpcm(audio, output, encoding)
            } else if options.wav_float && options.wav_bits == Some(64) {
                write_wav_float64(audio, output)
            } else if options.wav_bits.is_none() && !options.wav_float {
                audio.write_wav(output)
            } else {
                audio.write_wav_with_format(
                    output,
                    options.wav_bits.unwrap_or(16),
                    options.wav_float,
                )
            }
        }
        "flac" => write_flac(audio, output),
        "mp3" => write_mp3(audio, output, options.bitrate_kbps),
        "ogg" | "oga" => write_vorbis(audio, output),
        "aac" => write_aac(audio, output, options.bitrate_kbps),
        "aif" | "aiff" => write_aiff(audio, output, options.aiff_bits.unwrap_or(16)),
        "au" | "snd" => write_au(audio, output, options.au_encoding.unwrap_or_default()),
        "raw" => write_raw(audio, output, options.raw_encoding.unwrap_or_default()),
        "gsm" => crate::codecs::write_gsm(audio, output),
        "wv" | "wavpack" => crate::codecs::write_wavpack(audio, output),
        "amr" => crate::codecs::write_amr(
            audio,
            output,
            codec_core::codecs::amr::AmrVariant::NarrowBand,
        ),
        "awb" | "amr-wb" => crate::codecs::write_amr(
            audio,
            output,
            codec_core::codecs::amr::AmrVariant::WideBand,
        ),
        _ => bail!(
            "unsupported output format '{}'; supported extensions: wav, flac, mp3, ogg, aac, aiff, au, raw, gsm, amr, awb, wv",
            extension
        ),
    }
    .with_context(|| format!("failed to write {}", output.display()))
}

fn write_wav_float64(audio: &AudioBuffer, output: &Path) -> Result<()> {
    validate_audio(audio)?;
    let data_size = u64::try_from(audio.samples.len())
        .context("WAV sample count is too large")?
        .checked_mul(8)
        .ok_or_else(|| anyhow::anyhow!("WAV data size overflow"))?;
    let data_size =
        u32::try_from(data_size).context("64-bit float WAV exceeds the RIFF 4 GiB size limit")?;
    let riff_size = 36_u32
        .checked_add(data_size)
        .context("64-bit float WAV exceeds the RIFF 4 GiB size limit")?;
    let block_align = audio
        .spec
        .channels
        .checked_mul(8)
        .context("WAV channel count is too large for 64-bit samples")?;
    let byte_rate = audio
        .spec
        .sample_rate
        .checked_mul(u32::from(block_align))
        .context("WAV byte rate overflow")?;

    let mut writer = BufWriter::new(File::create(output)?);
    writer.write_all(b"RIFF")?;
    writer.write_all(&riff_size.to_le_bytes())?;
    writer.write_all(b"WAVEfmt ")?;
    writer.write_all(&16_u32.to_le_bytes())?;
    writer.write_all(&3_u16.to_le_bytes())?;
    writer.write_all(&audio.spec.channels.to_le_bytes())?;
    writer.write_all(&audio.spec.sample_rate.to_le_bytes())?;
    writer.write_all(&byte_rate.to_le_bytes())?;
    writer.write_all(&block_align.to_le_bytes())?;
    writer.write_all(&64_u16.to_le_bytes())?;
    writer.write_all(b"data")?;
    writer.write_all(&data_size.to_le_bytes())?;
    for sample in &audio.samples {
        writer.write_all(&f64::from(*sample).to_le_bytes())?;
    }
    writer.flush()?;
    Ok(())
}

fn write_raw(audio: &AudioBuffer, output: &Path, encoding: RawEncoding) -> Result<()> {
    validate_audio(audio)?;
    let mut file = BufWriter::new(File::create(output)?);
    // Integer encodings clamp inside `quantize_pcm`; float encodings keep values
    // above full scale.
    for &sample in &audio.samples {
        match encoding {
            RawEncoding::PcmU8 => write_raw_unsigned(&mut file, sample, 1, false)?,
            RawEncoding::PcmU16le => write_raw_unsigned(&mut file, sample, 2, false)?,
            RawEncoding::PcmU16be => write_raw_unsigned(&mut file, sample, 2, true)?,
            RawEncoding::PcmU24le => write_raw_unsigned(&mut file, sample, 3, false)?,
            RawEncoding::PcmU24be => write_raw_unsigned(&mut file, sample, 3, true)?,
            RawEncoding::PcmU32le => write_raw_unsigned(&mut file, sample, 4, false)?,
            RawEncoding::PcmU32be => write_raw_unsigned(&mut file, sample, 4, true)?,
            RawEncoding::PcmS8 => {
                let value = quantize_pcm(sample, 8) as i8;
                file.write_all(&[value as u8])?;
            }
            RawEncoding::PcmS16le => {
                let value = raw_signed_integer(sample, 16) as i16;
                file.write_all(&value.to_le_bytes())?;
            }
            RawEncoding::PcmS16be => {
                let value = raw_signed_integer(sample, 16) as i16;
                file.write_all(&value.to_be_bytes())?;
            }
            RawEncoding::PcmS24le | RawEncoding::PcmS24be => {
                let value = raw_signed_integer(sample, 24) as i32;
                let bytes = if matches!(encoding, RawEncoding::PcmS24le) {
                    value.to_le_bytes()
                } else {
                    value.to_be_bytes()
                };
                if matches!(encoding, RawEncoding::PcmS24le) {
                    file.write_all(&bytes[..3])?;
                } else {
                    file.write_all(&bytes[1..])?;
                }
            }
            RawEncoding::PcmS32le => {
                let value = raw_signed_integer(sample, 32) as i32;
                file.write_all(&value.to_le_bytes())?;
            }
            RawEncoding::PcmS32be => {
                let value = raw_signed_integer(sample, 32) as i32;
                file.write_all(&value.to_be_bytes())?;
            }
            RawEncoding::Float32le => file.write_all(&sample.to_le_bytes())?,
            RawEncoding::Float32be => file.write_all(&sample.to_be_bytes())?,
            RawEncoding::Float64le => file.write_all(&f64::from(sample).to_le_bytes())?,
            RawEncoding::Float64be => file.write_all(&f64::from(sample).to_be_bytes())?,
            RawEncoding::MuLaw => file.write_all(&[encode_mulaw(to_i16(sample))])?,
            RawEncoding::ALaw => file.write_all(&[encode_alaw(to_i16(sample))])?,
        }
    }
    file.flush()?;
    Ok(())
}

fn write_raw_unsigned(
    file: &mut impl Write,
    sample: f32,
    bytes: usize,
    big_endian: bool,
) -> Result<()> {
    let bits = bytes * 8;
    let midpoint = 1_u64 << (bits - 1);
    let signed = quantize_pcm(sample, bits as u32) as u64;
    let value = (signed ^ midpoint) & ((1_u64 << bits) - 1);
    for index in 0..bytes {
        let shift = if big_endian {
            (bytes - index - 1) * 8
        } else {
            index * 8
        };
        file.write_all(&[((value >> shift) & 0xff) as u8])?;
    }
    Ok(())
}

fn raw_signed_integer(sample: f32, bits: u32) -> i64 {
    quantize_pcm(sample, bits)
}

fn ensure_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    Ok(())
}

fn write_flac(audio: &AudioBuffer, output: &Path) -> Result<()> {
    validate_audio(audio)?;
    let bits = 24_u32;
    let pcm: Vec<i32> = audio
        .samples
        .iter()
        .map(|sample| quantize_pcm(*sample, bits) as i32)
        .collect();
    let mut encoder =
        FlacEncoder::new(audio.spec.sample_rate, u32::from(audio.spec.channels), bits)
            .context("invalid FLAC stream format")?;
    encoder.set_compression_level(5);
    encoder
        .push_interleaved(&pcm)
        .context("failed to pass PCM samples to FLAC encoder")?;
    std::fs::write(output, encoder.finish())?;
    Ok(())
}

fn write_mp3(audio: &AudioBuffer, output: &Path, bitrate_kbps: u32) -> Result<()> {
    validate_audio(audio)?;
    if audio.spec.channels > 2 {
        bail!("MP3 output supports mono or stereo; downmix to two channels first");
    }
    let mut encoder = Mp3Encoder::new(Mp3EncoderConfig {
        bitrate_kbps,
        vbr_quality: None,
    });
    encoder
        .push_pcm_f32(
            &normalized_pcm(&audio.samples),
            audio.spec.channels,
            audio.spec.sample_rate,
        )
        .context("MP3 encoder rejected the stream format")?;
    encoder.finish();

    let mut file = BufWriter::new(File::create(output)?);
    loop {
        match encoder.next_packet() {
            Ok(packet) => file.write_all(&packet)?,
            Err(Mp3Error::Eof) => break,
            Err(err) => return Err(err).context("failed to encode MP3 packet"),
        }
    }
    file.flush()?;
    Ok(())
}

fn write_vorbis(audio: &AudioBuffer, output: &Path) -> Result<()> {
    validate_audio(audio)?;
    if audio.spec.channels > 2 || !matches!(audio.spec.sample_rate, 44_100 | 48_000) {
        bail!("Ogg Vorbis output currently supports 1–2 channels at 44100 or 48000 Hz");
    }
    let mut encoder = VorbisEncoder::new(VorbisEncoderConfig::default());
    encoder
        .push_pcm_f32(
            &normalized_pcm(&audio.samples),
            audio.spec.channels,
            audio.spec.sample_rate,
        )
        .context("Vorbis encoder rejected the stream format")?;
    encoder.finish();

    let serial = stream_serial();
    let mut writer = PacketWriter::new(BufWriter::new(File::create(output)?));
    let mut first = true;
    let mut last_packet = None;
    loop {
        match encoder.next_packet() {
            Ok(packet) => {
                if let Some(previous) = last_packet.replace(packet) {
                    let end = if first {
                        first = false;
                        PacketWriteEndInfo::EndPage
                    } else {
                        PacketWriteEndInfo::NormalPacket
                    };
                    writer.write_packet(previous.data, serial, end, previous.pts.max(0) as u64)?;
                }
            }
            Err(VorbisError::Eof) => break,
            Err(err) => return Err(err).context("failed to encode Vorbis packet"),
        }
    }

    if let Some(packet) = last_packet {
        writer.write_packet(
            packet.data,
            serial,
            PacketWriteEndInfo::EndStream,
            packet.pts.max(0) as u64,
        )?;
    }
    writer.into_inner().flush()?;
    Ok(())
}

fn write_aac(audio: &AudioBuffer, output: &Path, bitrate_kbps: u32) -> Result<()> {
    validate_audio(audio)?;
    if audio.spec.channels > 6 {
        bail!("AAC-LC output supports up to six channels");
    }
    let mut encoder = AacEncoder::new(AacEncoderConfig {
        bitrate_bps: bitrate_kbps.saturating_mul(1_000),
        ..AacEncoderConfig::default()
    });
    encoder
        .push_pcm(
            &normalized_pcm(&audio.samples),
            audio.spec.channels,
            audio.spec.sample_rate,
        )
        .context("AAC encoder rejected the stream format")?;
    encoder.finish();

    let mut file = BufWriter::new(File::create(output)?);
    loop {
        match encoder.next_packet() {
            Ok(packet) => {
                let header = AdtsHeader {
                    object_type: 2,
                    sample_rate: audio.spec.sample_rate,
                    channels: audio.spec.channels,
                    frame_length: 7 + packet.data.len(),
                    header_len: 7,
                };
                file.write_all(&rusty_aac::write_adts_header(&header))?;
                file.write_all(&packet.data)?;
            }
            Err(AacError::Eof) => break,
            Err(err) => return Err(err).context("failed to encode AAC packet"),
        }
    }
    file.flush()?;
    Ok(())
}

fn write_aiff(audio: &AudioBuffer, output: &Path, bits: u16) -> Result<()> {
    validate_audio(audio)?;
    if !matches!(bits, 8 | 16 | 24 | 32) {
        bail!("AIFF output requires 8, 16, 24, or 32 bits per sample");
    }
    let bytes_per_sample = usize::from(bits / 8);
    let data_bytes = audio
        .samples
        .len()
        .checked_mul(bytes_per_sample)
        .ok_or_else(|| anyhow::anyhow!("AIFF output size overflow"))?;
    let frames = u32::try_from(audio.frames()).context("AIFF output exceeds 2^32 frames")?;
    let data_size = u32::try_from(data_bytes).context("AIFF output exceeds 4 GiB")?;
    let ssnd_size = 8_u32
        .checked_add(data_size)
        .ok_or_else(|| anyhow::anyhow!("AIFF output exceeds 4 GiB"))?;
    let form_size = 46_u32
        .checked_add(data_size)
        .ok_or_else(|| anyhow::anyhow!("AIFF output exceeds 4 GiB"))?;

    let mut file = BufWriter::new(File::create(output)?);
    file.write_all(b"FORM")?;
    file.write_all(&form_size.to_be_bytes())?;
    file.write_all(b"AIFFCOMM")?;
    file.write_all(&18_u32.to_be_bytes())?;
    file.write_all(&audio.spec.channels.to_be_bytes())?;
    file.write_all(&frames.to_be_bytes())?;
    file.write_all(&bits.to_be_bytes())?;
    file.write_all(&extended_sample_rate(audio.spec.sample_rate))?;
    file.write_all(b"SSND")?;
    file.write_all(&ssnd_size.to_be_bytes())?;
    file.write_all(&0_u32.to_be_bytes())?; // offset
    file.write_all(&0_u32.to_be_bytes())?; // block size
    for sample in &audio.samples {
        let value = quantize_pcm(*sample, u32::from(bits)) as i32;
        match bits {
            8 => file.write_all(&[value as i8 as u8])?,
            16 => file.write_all(&(value as i16).to_be_bytes())?,
            24 => file.write_all(&value.to_be_bytes()[1..])?,
            32 => file.write_all(&value.to_be_bytes())?,
            _ => unreachable!(),
        }
    }
    file.flush()?;
    Ok(())
}

fn write_au(audio: &AudioBuffer, output: &Path, encoding: AuEncoding) -> Result<()> {
    validate_audio(audio)?;
    let (encoding_code, bytes_per_sample) = match encoding {
        AuEncoding::Pcm8 => (2_u32, 1_usize),
        AuEncoding::Pcm16 => (3, 2),
        AuEncoding::Pcm24 => (4, 3),
        AuEncoding::Pcm32 => (5, 4),
        AuEncoding::Float32 => (6, 4),
        AuEncoding::Float64 => (7, 8),
        AuEncoding::MuLaw => (1, 1),
        AuEncoding::ALaw => (27, 1),
    };
    let data_bytes = audio
        .samples
        .len()
        .checked_mul(bytes_per_sample)
        .ok_or_else(|| anyhow::anyhow!("AU output size overflow"))?;
    let data_size = u32::try_from(data_bytes).context("AU output exceeds 4 GiB")?;

    let mut file = BufWriter::new(File::create(output)?);
    file.write_all(b".snd")?;
    file.write_all(&28_u32.to_be_bytes())?; // data offset
    file.write_all(&data_size.to_be_bytes())?;
    file.write_all(&encoding_code.to_be_bytes())?;
    file.write_all(&audio.spec.sample_rate.to_be_bytes())?;
    file.write_all(&u32::from(audio.spec.channels).to_be_bytes())?;
    file.write_all(&[0_u8; 4])?; // annotation padding (keeps strict AU readers happy)
    for &sample in &audio.samples {
        match encoding {
            AuEncoding::Pcm8 => {
                file.write_all(&(quantize_pcm(sample, 8) as i8 as u8).to_be_bytes())?
            }
            AuEncoding::Pcm16 => file.write_all(&to_i16(sample).to_be_bytes())?,
            AuEncoding::Pcm24 => {
                let value = quantize_pcm(sample, 24) as i32;
                let bytes = value.to_be_bytes();
                file.write_all(&bytes[1..])?;
            }
            AuEncoding::Pcm32 => {
                let value = quantize_pcm(sample, 32) as i32;
                file.write_all(&value.to_be_bytes())?;
            }
            AuEncoding::Float32 => file.write_all(&sample.to_be_bytes())?,
            AuEncoding::Float64 => file.write_all(&(f64::from(sample)).to_be_bytes())?,
            AuEncoding::MuLaw => file.write_all(&[encode_mulaw(to_i16(sample))])?,
            AuEncoding::ALaw => file.write_all(&[encode_alaw(to_i16(sample))])?,
        }
    }
    file.flush()?;
    Ok(())
}

fn to_i16(sample: f32) -> i16 {
    quantize_pcm(sample, 16) as i16
}

fn encode_mulaw(sample: i16) -> u8 {
    // SoX quantizes linear samples to 14 bits before its u-law lookup table.
    let mut pcm = ((i32::from(sample) + 2) >> 2) << 2;
    let mask = if pcm < 0 {
        pcm = -pcm;
        0x7f
    } else {
        0xff
    };
    pcm = pcm.min(32_635) + 0x84;
    let mut exponent = 7_u8;
    let mut segment = 0x4000_i32;
    while exponent > 0 && pcm & segment == 0 {
        exponent -= 1;
        segment >>= 1;
    }
    let mantissa = ((pcm >> (u32::from(exponent) + 3)) & 0x0f) as u8;
    ((exponent << 4) | mantissa) ^ mask
}

fn encode_alaw(sample: i16) -> u8 {
    // SoX quantizes linear samples to 13 bits before its A-law lookup table.
    let mut pcm = ((i32::from(sample) + 4) >> 3) << 3;
    let mask = if pcm >= 0 {
        0xd5
    } else {
        pcm = -pcm - 1;
        0x55
    };
    pcm = pcm.min(32_767);
    let (segment, mantissa) = if pcm < 256 {
        (0_u8, ((pcm >> 4) & 0x0f) as u8)
    } else {
        let mut exponent = 1_u8;
        let mut boundary = 0x200_i32;
        while exponent < 7 && pcm >= boundary {
            exponent += 1;
            boundary <<= 1;
        }
        (exponent, ((pcm >> (u32::from(exponent) + 3)) & 0x0f) as u8)
    };
    ((segment << 4) | mantissa) ^ mask
}

fn validate_audio(audio: &AudioBuffer) -> Result<()> {
    audio.validate()
}

/// Samples for encoders whose input contract is normalized PCM in [-1, 1]
/// (the MP3, Vorbis and AAC encoders scale by 32768 internally and are only
/// specified for that range). Borrows the buffer when nothing exceeds full
/// scale; otherwise returns a clamped copy.
fn normalized_pcm(samples: &[f32]) -> Cow<'_, [f32]> {
    if samples.iter().all(|sample| (-1.0..=1.0).contains(sample)) {
        Cow::Borrowed(samples)
    } else {
        Cow::Owned(
            samples
                .iter()
                .map(|sample| sample.clamp(-1.0, 1.0))
                .collect(),
        )
    }
}

fn stream_serial() -> u32 {
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.subsec_nanos());
    time ^ std::process::id()
}

fn extended_sample_rate(rate: u32) -> [u8; 10] {
    let value = rate as f64;
    let bits = value.to_bits();
    let sign = (bits >> 63) as u8;
    let exponent = ((bits >> 52) & 0x7ff) as i32 - 1023 + 16383;
    let mantissa = (bits & 0x000f_ffff_ffff_ffff) | 0x0010_0000_0000_0000;
    let mut bytes = [0; 10];
    bytes[0] = (sign << 7) | ((exponent >> 8) as u8 & 0x7f);
    bytes[1] = exponent as u8;
    bytes[2..].copy_from_slice(&(mantissa << 11).to_be_bytes());
    bytes
}
