use crate::audio::{AudioBuffer, AudioSpec};
use crate::effects::channel_matrix;
use anyhow::{Context, Result, anyhow, bail};
use std::path::{Path, PathBuf};

/// Concatenate audio files in order.
/// All inputs must have the same sample rate and channel count.
pub fn concat_inputs(inputs: &[PathBuf]) -> Result<AudioBuffer> {
    if inputs.is_empty() {
        bail!("concat requires at least one input");
    }

    let mut iter = inputs.iter();
    let first_path = iter.next().expect("checked non-empty");
    let mut output = AudioBuffer::read(first_path)
        .with_context(|| format!("failed to read {}", first_path.display()))?;

    for input in iter {
        let audio = AudioBuffer::read(input)
            .with_context(|| format!("failed to read {}", input.display()))?;
        ensure_same_format(&output, &audio, input)?;
        output
            .samples
            .try_reserve(audio.samples.len())
            .context("not enough memory to concatenate inputs")?;
        output.samples.extend_from_slice(&audio.samples);
    }

    Ok(output)
}

/// Mix audio files by summing samples (no automatic averaging).
/// Files with different sample rates will be rejected.
/// Files with different channel counts are converted to the maximum channel
/// count with the same mapping as the `channels` effect
/// ([`crate::effects::channel_matrix`]): mono is copied to every channel and
/// other layouts map each output channel to the same-index (or last) input
/// channel.
pub fn mix_inputs(inputs: &[PathBuf]) -> Result<AudioBuffer> {
    if inputs.is_empty() {
        bail!("mix requires at least one input");
    }

    let mut buffers = Vec::with_capacity(inputs.len());
    for input in inputs {
        buffers.push(
            AudioBuffer::read(input)
                .with_context(|| format!("failed to read {}", input.display()))?,
        );
    }

    let sample_rate = buffers[0].spec.sample_rate;
    if let Some((index, bad)) = buffers
        .iter()
        .enumerate()
        .find(|(_, audio)| audio.spec.sample_rate != sample_rate)
    {
        bail!(
            "mix input {} has sample rate {}, expected {}",
            inputs[index].display(),
            bad.spec.sample_rate,
            sample_rate
        );
    }

    let channels = buffers
        .iter()
        .map(|audio| audio.spec.channels)
        .max()
        .unwrap_or(1);
    let frames = buffers.iter().map(AudioBuffer::frames).max().unwrap_or(0);
    let channel_count = usize::from(channels);
    let sample_count = frames
        .checked_mul(channel_count)
        .ok_or_else(|| anyhow!("mixed output is too large"))?;
    let mut samples = Vec::new();
    samples
        .try_reserve_exact(sample_count)
        .context("not enough memory to mix inputs")?;
    samples.resize(sample_count, 0.0);

    for audio in &buffers {
        accumulate(&mut samples, channel_count, audio);
    }

    Ok(AudioBuffer {
        spec: AudioSpec {
            sample_rate,
            channels,
        },
        samples,
    })
}

/// Add `audio`, converted to `channel_count` channels with the shared channel
/// matrix, onto the interleaved `output`. Frames past the end of `audio`
/// contribute silence.
fn accumulate(output: &mut [f32], channel_count: usize, audio: &AudioBuffer) {
    let source_channels = usize::from(audio.spec.channels);
    let matrix = channel_matrix(source_channels, channel_count);
    for (out_frame, frame) in output
        .chunks_exact_mut(channel_count)
        .zip(audio.samples.chunks_exact(source_channels))
    {
        for (destination, row) in out_frame.iter_mut().zip(&matrix) {
            *destination += row
                .iter()
                .zip(frame)
                .filter(|(gain, _)| **gain != 0.0)
                .map(|(gain, sample)| gain * sample)
                .sum::<f32>();
        }
    }
}

fn ensure_same_format(reference: &AudioBuffer, audio: &AudioBuffer, path: &Path) -> Result<()> {
    if reference.spec.sample_rate != audio.spec.sample_rate
        || reference.spec.channels != audio.spec.channels
    {
        bail!(
            "{} has {} Hz / {} channel(s), expected {} Hz / {} channel(s)",
            path.display(),
            audio.spec.sample_rate,
            audio.spec.channels,
            reference.spec.sample_rate,
            reference.spec.channels
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::{Effect, EffectChain};

    fn buffer(channels: u16, samples: Vec<f32>) -> AudioBuffer {
        AudioBuffer {
            spec: AudioSpec {
                sample_rate: 8_000,
                channels,
            },
            samples,
        }
    }

    #[test]
    fn mixing_uses_the_same_channel_mapping_as_the_channels_effect() {
        let sources = [
            buffer(1, vec![0.5, -0.25]),
            buffer(2, vec![0.1, 0.2, 0.3, 0.4]),
            buffer(4, vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8]),
        ];
        for target in [2_usize, 6] {
            for source in &sources {
                if usize::from(source.spec.channels) > target {
                    continue;
                }
                let mut mixed = vec![0.0; source.frames() * target];
                accumulate(&mut mixed, target, source);
                let mut converted = source.clone();
                EffectChain::new(vec![Effect::Channels {
                    channels: target as u16,
                }])
                .apply(&mut converted)
                .unwrap();
                assert_eq!(
                    mixed, converted.samples,
                    "{} -> {target}",
                    source.spec.channels
                );
            }
        }
    }

    #[test]
    fn shorter_inputs_contribute_silence_after_their_end() {
        let mut mixed = vec![0.0; 3 * 2];
        accumulate(&mut mixed, 2, &buffer(1, vec![1.0, 2.0, 3.0]));
        accumulate(&mut mixed, 2, &buffer(2, vec![0.5, 0.25]));
        assert_eq!(mixed, vec![1.5, 1.25, 2.0, 2.0, 3.0, 3.0]);
    }
}
