use crate::audio::AudioBuffer;
use crate::effects::EffectChain;
use crate::encode;
use crate::stats;
use anyhow::{Context, Result, bail};
use glob::glob;
use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Read an input file, apply the effect chain, and write the output.
pub fn convert_one(
    input: &Path,
    output: &Path,
    chain: &EffectChain,
    stat_json: bool,
) -> Result<()> {
    let mut audio =
        AudioBuffer::read(input).with_context(|| format!("failed to read {}", input.display()))?;
    chain.apply(&mut audio)?;
    write_output(&audio, output, stat_json || chain.wants_stats(), stat_json)
}

/// Write an `AudioBuffer` using the format chosen by the output extension,
/// optionally printing stats. The destination is replaced atomically.
pub fn write_output(
    audio: &AudioBuffer,
    output: &Path,
    print_stats: bool,
    stat_json: bool,
) -> Result<()> {
    write_audio_atomic(
        audio,
        output,
        encode::EncodeOptions {
            bitrate_kbps: 192,
            ..encode::EncodeOptions::default()
        },
    )?;

    if print_stats {
        let report = stats::Report::from_audio(output, audio);
        if stat_json {
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            println!("{report}");
        }
    }

    Ok(())
}

/// Encode `audio` into a temporary sibling of `output` and rename it over the
/// destination only after the write fully succeeded.
pub fn write_audio_atomic(
    audio: &AudioBuffer,
    output: &Path,
    options: encode::EncodeOptions,
) -> Result<()> {
    let temp = TempOutput::new(output)?;
    encode::write_audio_with_options(audio, temp.path(), options)
        .with_context(|| format!("failed to write {}", output.display()))?;
    temp.commit()
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A temporary file next to a destination. `commit` renames it over the
/// destination; dropping it without committing removes it. Declare it before
/// any writer that targets `path()` so the writer is closed first.
pub struct TempOutput {
    temp: PathBuf,
    dest: PathBuf,
    committed: bool,
}

impl TempOutput {
    /// Create the destination directory if needed and reserve a temp name that
    /// keeps the destination's extension.
    pub fn new(dest: &Path) -> Result<Self> {
        if let Some(parent) = dest
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        if dest.is_dir() {
            bail!("output {} is a directory", dest.display());
        }
        let nonce = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let clock = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.subsec_nanos())
            .unwrap_or(0);
        let mut name = OsString::from(".");
        name.push(dest.file_stem().unwrap_or_else(|| OsStr::new("output")));
        name.push(format!(
            ".soundx-tmp-{}-{nonce}-{clock}",
            std::process::id()
        ));
        if let Some(extension) = dest.extension() {
            name.push(".");
            name.push(extension);
        }
        Ok(Self {
            temp: dest.with_file_name(name),
            dest: dest.to_path_buf(),
            committed: false,
        })
    }

    pub fn path(&self) -> &Path {
        &self.temp
    }

    /// Replace the destination with the finished temp file.
    pub fn commit(mut self) -> Result<()> {
        std::fs::rename(&self.temp, &self.dest).with_context(|| {
            format!(
                "failed to move finished output into place at {}",
                self.dest.display()
            )
        })?;
        self.committed = true;
        Ok(())
    }
}

impl Drop for TempOutput {
    fn drop(&mut self) {
        if !self.committed {
            let _ = std::fs::remove_file(&self.temp);
        }
    }
}

/// Expand glob patterns into a sorted, deduplicated list of file paths.
/// A path that exists literally is used as-is; a pattern matching nothing is
/// an error.
pub fn expand_inputs(patterns: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut inputs = Vec::new();
    for pattern in patterns {
        let text = pattern.to_string_lossy();
        let is_pattern = text.contains('*') || text.contains('?') || text.contains('[');
        if !is_pattern || pattern.exists() {
            inputs.push(pattern.clone());
            continue;
        }
        let before = inputs.len();
        for entry in glob(&text).with_context(|| format!("invalid glob {text}"))? {
            inputs.push(entry.with_context(|| format!("failed to expand glob {text}"))?);
        }
        if inputs.len() == before {
            bail!("no input files matched '{text}'");
        }
    }
    inputs.sort();
    inputs.dedup();
    Ok(inputs)
}

/// Build the output path for a batch item: `out_dir/{stem}.{extension}`
pub fn output_for_batch(input: &Path, out_dir: &Path, extension: &str) -> PathBuf {
    let stem = input
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or("output");
    out_dir.join(format!("{stem}.{extension}"))
}

fn comparable(path: PathBuf) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(path.to_string_lossy().to_lowercase())
    } else {
        path
    }
}

/// Compute every batch output path and fail when two inputs share an output
/// or an output would overwrite one of the inputs.
pub fn plan_batch_outputs(
    inputs: &[PathBuf],
    out_dir: &Path,
    extension: &str,
) -> Result<Vec<PathBuf>> {
    let canonical_dir = out_dir.canonicalize().ok();
    let key_of = |output: &Path| -> PathBuf {
        let resolved = match (&canonical_dir, output.file_name()) {
            (Some(dir), Some(name)) => dir.join(name),
            _ => output
                .canonicalize()
                .unwrap_or_else(|_| output.to_path_buf()),
        };
        comparable(resolved)
    };
    let canonical_inputs: Vec<PathBuf> = inputs
        .iter()
        .map(|input| comparable(input.canonicalize().unwrap_or_else(|_| input.clone())))
        .collect();

    let outputs: Vec<PathBuf> = inputs
        .iter()
        .map(|input| output_for_batch(input, out_dir, extension))
        .collect();
    let keys: Vec<PathBuf> = outputs.iter().map(|output| key_of(output)).collect();
    let mut by_output: HashMap<&PathBuf, Vec<usize>> = HashMap::new();
    for (index, key) in keys.iter().enumerate() {
        by_output.entry(key).or_default().push(index);
    }

    let mut problems = Vec::new();
    for (index, key) in keys.iter().enumerate() {
        let group = &by_output[key];
        if group[0] == index && group.len() > 1 {
            let names: Vec<String> = group
                .iter()
                .map(|&i| inputs[i].display().to_string())
                .collect();
            problems.push(format!(
                "{} would all be written to {}",
                names.join(", "),
                outputs[index].display()
            ));
        }
        if let Some(position) = canonical_inputs.iter().position(|input| input == key) {
            problems.push(format!(
                "{} would overwrite input {}",
                outputs[index].display(),
                inputs[position].display()
            ));
        }
    }
    if !problems.is_empty() {
        bail!("batch output collision: {}", problems.join("; "));
    }
    Ok(outputs)
}
