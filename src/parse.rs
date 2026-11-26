use crate::effects::{BiquadKind, Effect, FilterWidth, TempoQuality};
use anyhow::{Context, Result, anyhow};

/// Parse a sequence of effect tokens (SoX-style command line arguments)
/// into a vector of `Effect` values.
pub fn parse_effects(tokens: &[String]) -> Result<(Vec<Effect>, bool)> {
    let mut effects = Vec::new();
    let mut stat_json = false;
    let mut i = 0;

    while i < tokens.len() {
        match tokens[i].as_str() {
            "gain" | "vol" => {
                let db = parse_next_f32(tokens, &mut i, "gain requires <db>")?;
                effects.push(Effect::GainDb(db));
            }
            "norm" | "normalize" => {
                let target_db = match tokens.get(i + 1).and_then(|s| finite(s)) {
                    Some(value) => {
                        i += 1;
                        value
                    }
                    None => -1.0,
                };
                effects.push(Effect::Normalize { target_db });
            }
            "trim" => {
                let start = parse_next_f32(tokens, &mut i, "trim requires <start-sec>")?;
                let duration = match tokens.get(i + 1).and_then(|s| finite(s)) {
                    Some(value) => {
                        i += 1;
                        Some(value)
                    }
                    None => None,
                };
                effects.push(Effect::Trim {
                    start_sec: start,
                    duration_sec: duration,
                });
            }
            "fade" => {
                let in_sec = parse_next_f32(tokens, &mut i, "fade requires <in-sec>")?;
                let out_sec = match tokens.get(i + 1).and_then(|s| finite(s)) {
                    Some(value) => {
                        i += 1;
                        value
                    }
                    None => 0.0,
                };
                effects.push(Effect::Fade { in_sec, out_sec });
            }
            "reverse" | "reverse-samples" => effects.push(Effect::Reverse),
            "speed" => {
                let factor = parse_next_f32(tokens, &mut i, "speed requires <factor>")?;
                effects.push(Effect::Speed { factor });
            }
            "stretch" => {
                let factor = parse_next_f32(tokens, &mut i, "stretch requires <factor>")?;
                let window_ms = take_optional_f32(tokens, &mut i).unwrap_or(82.0);
                let search_ms = take_optional_f32(tokens, &mut i).unwrap_or(14.0);
                let overlap_ms = take_optional_f32(tokens, &mut i).unwrap_or(12.0);
                effects.push(Effect::Stretch {
                    factor,
                    window_ms,
                    search_ms,
                    overlap_ms,
                });
            }
            "tempo" => {
                let quality = match tokens.get(i + 1).map(String::as_str) {
                    Some("-q") => {
                        i += 1;
                        TempoQuality::Quick
                    }
                    Some("-m") => {
                        i += 1;
                        TempoQuality::Music
                    }
                    Some("-s") => {
                        i += 1;
                        TempoQuality::Speech
                    }
                    Some("-l") => {
                        i += 1;
                        TempoQuality::Linear
                    }
                    _ => TempoQuality::Music,
                };
                let factor = parse_next_f32(tokens, &mut i, "tempo requires <factor>")?;
                let defaults = match quality {
                    TempoQuality::Quick => (30.0, 5.0, 10.0),
                    TempoQuality::Music => (82.0, 14.0, 12.0),
                    TempoQuality::Speech => (50.0, 10.0, 10.0),
                    TempoQuality::Linear => (30.0, 5.0, 15.0),
                };
                let segment_ms = take_optional_f32(tokens, &mut i).unwrap_or(defaults.0);
                let search_ms = take_optional_f32(tokens, &mut i).unwrap_or(defaults.1);
                let overlap_ms = take_optional_f32(tokens, &mut i).unwrap_or(defaults.2);
                effects.push(Effect::Tempo {
                    factor,
                    quality,
                    segment_ms,
                    search_ms,
                    overlap_ms,
                });
            }
            "dither" => {
                let bits = match tokens
                    .get(i + 1)
                    .and_then(|value| value.parse::<u16>().ok())
                {
                    Some(value) => {
                        i += 1;
                        value
                    }
                    None => 16,
                };
                effects.push(Effect::Dither { bits });
            }
            "compand" => {
                let attack_values = parse_comma_floats(
                    tokens
                        .get(i + 1)
                        .ok_or_else(|| anyhow!("compand requires <attack,decay,...>"))?,
                    "compand attack/decay values",
                    false,
                )?;
                i += 1;
                if attack_values.is_empty() || attack_values.len() % 2 != 0 {
                    return Err(anyhow!("compand attack/decay list must contain time pairs"));
                }
                let attack_decay = attack_values
                    .chunks_exact(2)
                    .map(|pair| (pair[0], pair[1]))
                    .collect::<Vec<_>>();
                let transfer_token = tokens
                    .get(i + 1)
                    .ok_or_else(|| anyhow!("compand requires <transfer-points>"))?;
                let (knee_db, transfer_token) = match transfer_token.split_once(':') {
                    Some((knee, points)) => {
                        (parse_finite(knee).context("invalid compand knee")?, points)
                    }
                    None => (0.0, transfer_token.as_str()),
                };
                let transfer_values =
                    parse_comma_floats(transfer_token, "compand transfer points", true)?;
                i += 1;
                if transfer_values.len() < 4 || transfer_values.len() % 2 != 0 {
                    return Err(anyhow!(
                        "compand transfer list must contain at least two input/output pairs"
                    ));
                }
                let transfer_points = transfer_values
                    .chunks_exact(2)
                    .map(|pair| (pair[0], pair[1]))
                    .collect::<Vec<_>>();
                let gain_db = take_optional_f32(tokens, &mut i).unwrap_or(0.0);
                let initial_volume_db = take_optional_f32(tokens, &mut i).unwrap_or(0.0);
                let delay_sec = take_optional_f32(tokens, &mut i).unwrap_or(0.0);
                effects.push(Effect::Compand {
                    attack_decay,
                    transfer_points,
                    knee_db,
                    gain_db,
                    initial_volume_db,
                    delay_sec,
                });
            }
            "reverb" => {
                let mut wet_only = false;
                while matches!(
                    tokens.get(i + 1).map(String::as_str),
                    Some("-w" | "--wet-only")
                ) {
                    i += 1;
                    wet_only = true;
                }
                let defaults = [50.0, 50.0, 100.0, 100.0, 0.0, 0.0];
                let mut values = defaults;
                for value in &mut values {
                    if let Some(parsed) = take_optional_f32(tokens, &mut i) {
                        *value = parsed;
                    } else {
                        break;
                    }
                }
                effects.push(Effect::Reverb {
                    wet_only,
                    reverberance: values[0],
                    hf_damping: values[1],
                    room_scale: values[2],
                    stereo_depth: values[3],
                    pre_delay_ms: values[4],
                    wet_gain_db: values[5],
                });
            }
            "pad" => {
                let start_sec = parse_next_f32(tokens, &mut i, "pad requires <start-sec>")?;
                let end_sec = match tokens.get(i + 1).and_then(|s| finite(s)) {
                    Some(value) => {
                        i += 1;
                        value
                    }
                    None => 0.0,
                };
                effects.push(Effect::Pad { start_sec, end_sec });
            }
            "silence" => {
                let threshold_db =
                    parse_next_f32(tokens, &mut i, "silence requires <threshold-db>")?;
                let min_duration_sec = match tokens.get(i + 1).and_then(|s| finite(s)) {
                    Some(value) => {
                        i += 1;
                        value
                    }
                    None => 0.1,
                };
                effects.push(Effect::Silence {
                    threshold_db,
                    min_duration_sec,
                });
            }
            "lowpass" => {
                let poles = match tokens.get(i + 1).map(String::as_str) {
                    Some("-1") => {
                        i += 1;
                        1
                    }
                    Some("-2") => {
                        i += 1;
                        2
                    }
                    _ => 2,
                };
                let hz = parse_next_frequency(tokens, &mut i, "lowpass requires <frequency>")?;
                let width = take_optional_width(tokens, &mut i, "qohk", 'q')
                    .unwrap_or(FilterWidth::Q(0.707));
                effects.push(Effect::BiquadFilter {
                    kind: BiquadKind::LowPass,
                    hz,
                    width,
                    gain_db: 0.0,
                    poles,
                });
            }
            "highpass" => {
                let poles = match tokens.get(i + 1).map(String::as_str) {
                    Some("-1") => {
                        i += 1;
                        1
                    }
                    Some("-2") => {
                        i += 1;
                        2
                    }
                    _ => 2,
                };
                let hz = parse_next_frequency(tokens, &mut i, "highpass requires <frequency>")?;
                let width = take_optional_width(tokens, &mut i, "qohk", 'q')
                    .unwrap_or(FilterWidth::Q(0.707));
                effects.push(Effect::BiquadFilter {
                    kind: BiquadKind::HighPass,
                    hz,
                    width,
                    gain_db: 0.0,
                    poles,
                });
            }
            "bass" | "treble" => {
                let name = tokens[i].as_str();
                let gain_db = parse_next_f32(tokens, &mut i, "bass/treble requires <gain-db>")?;
                let default_hz = if name == "bass" { 100.0 } else { 3000.0 };
                let hz = match tokens
                    .get(i + 1)
                    .and_then(|token| parse_filter_frequency(token))
                {
                    Some(value) => {
                        i += 1;
                        value
                    }
                    None => default_hz,
                };
                let width = take_optional_width(tokens, &mut i, "shkqo", 's')
                    .unwrap_or(FilterWidth::Slope(0.5));
                effects.push(Effect::BiquadFilter {
                    kind: if name == "bass" {
                        BiquadKind::LowShelf
                    } else {
                        BiquadKind::HighShelf
                    },
                    hz,
                    width,
                    gain_db,
                    poles: 2,
                });
            }
            "allpass" | "bandpass" | "bandreject" => {
                let name = tokens[i].as_str();
                let constant_skirt =
                    name == "bandpass" && tokens.get(i + 1).is_some_and(|token| token == "-c");
                if constant_skirt {
                    i += 1;
                }
                let hz = parse_next_frequency(tokens, &mut i, "filter requires <frequency>")?;
                let width = take_optional_width(tokens, &mut i, "hkqo", 'h')
                    .ok_or_else(|| anyhow!("{name} requires <width[h|k|q|o]>"))?;
                let kind = match name {
                    "allpass" => BiquadKind::AllPass,
                    "bandpass" if constant_skirt => BiquadKind::BandPassConstantSkirt,
                    "bandpass" => BiquadKind::BandPass,
                    _ => BiquadKind::BandReject,
                };
                effects.push(Effect::BiquadFilter {
                    kind,
                    hz,
                    width,
                    gain_db: 0.0,
                    poles: 2,
                });
            }
            "equalizer" => {
                let hz = parse_next_frequency(tokens, &mut i, "equalizer requires <frequency>")?;
                let width = take_optional_width(tokens, &mut i, "qohk", 'q')
                    .ok_or_else(|| anyhow!("equalizer requires <width[q|o|h|k]>"))?;
                let gain_db = parse_next_f32(tokens, &mut i, "equalizer requires <gain-db>")?;
                effects.push(Effect::BiquadFilter {
                    kind: BiquadKind::Equalizer,
                    hz,
                    width,
                    gain_db,
                    poles: 2,
                });
            }
            "echo" => {
                let input_gain = parse_next_f32(tokens, &mut i, "echo requires <gain-in>")?;
                let output_gain = parse_next_f32(tokens, &mut i, "echo requires <gain-out>")?;
                let mut taps_ms = Vec::new();
                loop {
                    let Some(delay) = tokens.get(i + 1).and_then(|s| finite(s)) else {
                        break;
                    };
                    i += 1;
                    let decay = parse_next_f32(tokens, &mut i, "echo delay requires <decay>")?;
                    taps_ms.push((delay, decay));
                }
                if taps_ms.is_empty() {
                    return Err(anyhow!(
                        "echo requires at least one <delay-ms> <decay> pair"
                    ));
                }
                effects.push(Effect::Echo {
                    input_gain,
                    output_gain,
                    taps_ms,
                });
            }
            "tremolo" => {
                let speed_hz = parse_next_f32(tokens, &mut i, "tremolo requires <speed-hz>")?;
                let depth_percent = match tokens.get(i + 1).and_then(|s| finite(s)) {
                    Some(value) => {
                        i += 1;
                        value
                    }
                    None => 40.0,
                };
                effects.push(Effect::Tremolo {
                    speed_hz,
                    depth_percent,
                });
            }
            "delay" => {
                let mut delays_sec = Vec::new();
                while tokens.get(i + 1).and_then(|s| finite(s)).is_some() {
                    delays_sec.push(parse_next_f32(
                        tokens,
                        &mut i,
                        "delay requires <position-sec>",
                    )?);
                }
                if delays_sec.is_empty() {
                    return Err(anyhow!("delay requires at least one <position-sec>"));
                }
                effects.push(Effect::Delay { delays_sec });
            }
            "dcshift" => {
                let shift = parse_next_f32(tokens, &mut i, "dcshift requires <shift>")?;
                let limiter_gain = tokens.get(i + 1).and_then(|s| finite(s));
                if limiter_gain.is_some() {
                    i += 1;
                }
                effects.push(Effect::DcShift {
                    shift,
                    limiter_gain,
                });
            }
            "downsample" => {
                let factor = match tokens.get(i + 1).and_then(|s| s.parse::<u32>().ok()) {
                    Some(value) => {
                        i += 1;
                        value
                    }
                    None => 2,
                };
                effects.push(Effect::Downsample { factor });
            }
            "upsample" => {
                let factor = match tokens.get(i + 1).and_then(|s| s.parse::<u32>().ok()) {
                    Some(value) => {
                        i += 1;
                        value
                    }
                    None => 2,
                };
                effects.push(Effect::Upsample { factor });
            }
            "repeat" => {
                let count = match tokens.get(i + 1).and_then(|s| s.parse::<u32>().ok()) {
                    Some(value) => {
                        i += 1;
                        value
                    }
                    None => 1,
                };
                effects.push(Effect::Repeat { count });
            }
            "swap" => effects.push(Effect::Swap),
            "limiter" => {
                let threshold = match tokens.get(i + 1).and_then(|s| finite(s)) {
                    Some(value) => {
                        i += 1;
                        value
                    }
                    None => 0.95,
                };
                effects.push(Effect::Limiter { threshold });
            }
            "rate" | "resample" => {
                let sample_rate = parse_next_u32(tokens, &mut i, "rate requires <hz>")?;
                effects.push(Effect::Rate { sample_rate });
            }
            "channels" | "ch" => {
                let channels = parse_next_u16(tokens, &mut i, "channels requires <count>")?;
                effects.push(Effect::Channels { channels });
            }
            "stat" | "stats" => effects.push(Effect::Stats),
            "stat-json" | "stats-json" => {
                stat_json = true;
                effects.push(Effect::Stats);
            }
            unknown => return Err(anyhow!("unsupported effect '{unknown}'")),
        }
        i += 1;
    }

    Ok((effects, stat_json))
}

/// Parse a float, rejecting NaN and infinities.
fn parse_finite(value: &str) -> Result<f32> {
    let parsed = value.parse::<f32>()?;
    if !parsed.is_finite() {
        return Err(anyhow!("{value} is not a finite number"));
    }
    Ok(parsed)
}

fn finite(value: &str) -> Option<f32> {
    parse_finite(value).ok()
}

fn parse_next_frequency(tokens: &[String], i: &mut usize, message: &str) -> Result<f32> {
    *i += 1;
    tokens
        .get(*i)
        .and_then(|token| parse_filter_frequency(token))
        .ok_or_else(|| anyhow!(message.to_string()))
}

fn take_optional_f32(tokens: &[String], i: &mut usize) -> Option<f32> {
    let value = finite(tokens.get(*i + 1)?)?;
    *i += 1;
    Some(value)
}

/// Parse comma-separated floats. NaN is always rejected; infinities are
/// accepted only when `allow_infinite` (SoX compand transfer curves use -inf).
fn parse_comma_floats(value: &str, name: &str, allow_infinite: bool) -> Result<Vec<f32>> {
    value
        .split(',')
        .map(|part| {
            let parsed = part
                .parse::<f32>()
                .with_context(|| format!("invalid {name}: '{part}'"))?;
            if parsed.is_nan() || (!allow_infinite && parsed.is_infinite()) {
                return Err(anyhow!("invalid {name}: '{part}' is not a finite number"));
            }
            Ok(parsed)
        })
        .collect()
}

fn parse_filter_frequency(token: &str) -> Option<f32> {
    let (value, multiplier) = if let Some(value) = token.strip_suffix('k') {
        (value, 1000.0)
    } else {
        (token, 1.0)
    };
    let hz = value.parse::<f32>().ok()? * multiplier;
    (hz.is_finite() && hz > 0.0).then_some(hz)
}

fn take_optional_width(
    tokens: &[String],
    i: &mut usize,
    allowed_types: &str,
    default_type: char,
) -> Option<FilterWidth> {
    let width = tokens
        .get(*i + 1)
        .and_then(|token| parse_filter_width(token, allowed_types, default_type))?;
    *i += 1;
    Some(width)
}

fn parse_filter_width(token: &str, allowed_types: &str, default_type: char) -> Option<FilterWidth> {
    let (number, width_type) = match token.chars().last() {
        Some(unit) if unit.is_ascii_alphabetic() => (
            &token[..token.len().checked_sub(unit.len_utf8())?],
            unit.to_ascii_lowercase(),
        ),
        _ => (token, default_type),
    };
    if !allowed_types.contains(width_type) {
        return None;
    }
    let value = number.parse::<f32>().ok()?;
    if !value.is_finite() || value <= 0.0 {
        return None;
    }
    Some(match width_type {
        'h' => FilterWidth::Hertz(value),
        'k' => FilterWidth::Kilohertz(value),
        'b' => FilterWidth::HertzNoWarp(value),
        'o' => FilterWidth::Octaves(value),
        'q' => FilterWidth::Q(value),
        's' => FilterWidth::Slope(value),
        _ => return None,
    })
}

fn parse_next_f32(tokens: &[String], i: &mut usize, message: &str) -> Result<f32> {
    *i += 1;
    tokens
        .get(*i)
        .ok_or_else(|| anyhow!(message.to_string()))
        .and_then(|token| parse_finite(token))
        .with_context(|| message.to_string())
}

fn parse_next_u32(tokens: &[String], i: &mut usize, message: &str) -> Result<u32> {
    *i += 1;
    tokens
        .get(*i)
        .ok_or_else(|| anyhow!(message.to_string()))?
        .parse::<u32>()
        .with_context(|| message.to_string())
}

fn parse_next_u16(tokens: &[String], i: &mut usize, message: &str) -> Result<u16> {
    *i += 1;
    tokens
        .get(*i)
        .ok_or_else(|| anyhow!(message.to_string()))?
        .parse::<u16>()
        .with_context(|| message.to_string())
}
