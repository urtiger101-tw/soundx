use crate::cli::{AgentTarget, IntegrationArgs, IntegrationCommand};
use anyhow::{Context, Result, anyhow, bail};
use jsonc_parser::cst::{CstInputValue, CstRootNode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const SKILL: &str = include_str!("../skills/soundx/SKILL.md");
const TARGETS: [AgentTarget; 4] = [
    AgentTarget::Codex,
    AgentTarget::Claude,
    AgentTarget::Opencode,
    AgentTarget::Agy,
];

struct Layout {
    name: &'static str,
    config: PathBuf,
    group: &'static str,
    skills: Vec<PathBuf>,
    manifest: PathBuf,
}
fn layout(home: &Path, agent: AgentTarget) -> Result<Layout> {
    let (name, config, group, skills) = match agent {
        AgentTarget::Codex => (
            "codex",
            ".codex/config.toml",
            "mcp_servers",
            vec![".agents/skills/soundx/SKILL.md"],
        ),
        AgentTarget::Claude => (
            "claude",
            ".claude.json",
            "mcpServers",
            vec![".claude/skills/soundx/SKILL.md"],
        ),
        AgentTarget::Opencode => {
            let jsonc = home.join(".config/opencode/opencode.jsonc");
            (
                "opencode",
                if jsonc.exists() {
                    ".config/opencode/opencode.jsonc"
                } else {
                    ".config/opencode/opencode.json"
                },
                "mcp",
                vec![".config/opencode/skills/soundx/SKILL.md"],
            )
        }
        AgentTarget::Agy => (
            "agy",
            ".gemini/config/mcp_config.json",
            "mcpServers",
            vec![
                ".gemini/antigravity-cli/skills/soundx/SKILL.md",
                ".gemini/config/skills/soundx/SKILL.md",
            ],
        ),
    };
    Ok(Layout {
        name,
        config: home.join(config),
        group,
        skills: skills.into_iter().map(|path| home.join(path)).collect(),
        manifest: home.join(format!(".soundx/integrations/{name}.json")),
    })
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    config: PathBuf,
    entry: Value,
    config_owned: bool,
    files: BTreeMap<PathBuf, String>,
}
fn manifest(layout: &Layout) -> Result<Option<Manifest>> {
    let Some(bytes) = read_optional(&layout.manifest)? else {
        return Ok(None);
    };
    let manifest: Manifest =
        serde_json::from_slice(&bytes).context("invalid soundx integration manifest")?;
    let migrated_opencode = layout.name == "opencode"
        && manifest.config.parent() == layout.config.parent()
        && matches!(
            manifest.config.file_name().and_then(|name| name.to_str()),
            Some("opencode.json" | "opencode.jsonc")
        );
    if (manifest.config != layout.config && !migrated_opencode)
        || manifest
            .files
            .keys()
            .any(|path| !layout.skills.contains(path))
    {
        bail!("integration manifest paths do not match this agent profile");
    }
    Ok(Some(manifest))
}

fn json_options() -> jsonc_parser::ParseOptions {
    jsonc_parser::ParseOptions {
        allow_comments: true,
        allow_trailing_commas: true,
        allow_loose_object_property_names: false,
        allow_missing_commas: false,
        allow_single_quoted_strings: false,
        allow_hexadecimal_numbers: false,
        allow_unary_plus_numbers: false,
    }
}
fn json_input(value: &Value) -> CstInputValue {
    match value {
        Value::Null => CstInputValue::Null,
        Value::Bool(value) => CstInputValue::Bool(*value),
        Value::Number(value) => CstInputValue::Number(value.to_string()),
        Value::String(value) => CstInputValue::String(value.clone()),
        Value::Array(values) => CstInputValue::Array(values.iter().map(json_input).collect()),
        Value::Object(values) => CstInputValue::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), json_input(value)))
                .collect(),
        ),
    }
}

/// Change only the soundx entry. TOML and JSONC comments remain intact.
fn edit_config(
    text: &str,
    layout: &Layout,
    replacement: Option<&Value>,
) -> Result<(Option<Value>, String)> {
    if layout.name == "codex" {
        let mut doc = text
            .parse::<toml_edit::DocumentMut>()
            .context("invalid Codex TOML; left unchanged")?;
        if doc.get(layout.group).is_some_and(|item| !item.is_table()) {
            bail!("mcp_servers must be a TOML table");
        }
        let current = match doc.get(layout.group).and_then(|item| item.get("soundx")) {
            None => None,
            Some(item) => {
                // Compare every field, so customized env/permissions are never silently replaced.
                let mut wrapper = toml_edit::DocumentMut::new();
                wrapper["entry"] = item.clone();
                Some(toml_edit::de::from_str::<Value>(&wrapper.to_string())?["entry"].clone())
            }
        };
        if let Some(value) = replacement {
            if doc.get(layout.group).is_none() {
                doc[layout.group] = toml_edit::Item::Table(toml_edit::Table::new());
            }
            let mut table = toml_edit::Table::new();
            table["command"] =
                toml_edit::value(value["command"].as_str().context("missing MCP command")?);
            let mut args = toml_edit::Array::new();
            for arg in value["args"].as_array().context("missing MCP args")? {
                args.push(arg.as_str().context("invalid MCP argument")?);
            }
            table["args"] = toml_edit::value(args);
            doc[layout.group]["soundx"] = toml_edit::Item::Table(table);
        } else if let Some(table) = doc
            .get_mut(layout.group)
            .and_then(toml_edit::Item::as_table_mut)
        {
            table.remove("soundx");
        }
        Ok((current, doc.to_string()))
    } else {
        let text = if text.trim().is_empty() {
            "{}"
        } else {
            text.trim_start_matches('\u{feff}')
        };
        let root = CstRootNode::parse(text, &json_options())
            .context("invalid agent JSON/JSONC; left unchanged")?;
        let object = root
            .object_value()
            .context("agent configuration must be a JSON object")?;
        for key in [layout.group] {
            if object
                .properties()
                .iter()
                .filter(|prop| {
                    prop.name()
                        .and_then(|name| name.decoded_value().ok())
                        .as_deref()
                        == Some(key)
                })
                .count()
                > 1
            {
                bail!("duplicate agent configuration key: {key}");
            }
        }
        let group = object
            .object_value_or_create(layout.group)
            .context("MCP configuration must be an object")?;
        if group
            .properties()
            .iter()
            .filter(|prop| {
                prop.name()
                    .and_then(|name| name.decoded_value().ok())
                    .as_deref()
                    == Some("soundx")
            })
            .count()
            > 1
        {
            bail!("duplicate soundx MCP entries");
        }
        let current = group
            .get("soundx")
            .map(|prop| -> Result<Value> {
                let value = prop.value().context("empty MCP value")?;
                Ok(jsonc_parser::parse_to_serde_value(
                    &value.to_string(),
                    &json_options(),
                )?)
            })
            .transpose()?;
        if let Some(value) = replacement {
            if let Some(prop) = group.get("soundx") {
                prop.set_value(json_input(value));
            } else {
                group.append("soundx", json_input(value));
            }
        } else if let Some(prop) = group.get("soundx") {
            prop.remove();
        }
        Ok((current, root.to_string()))
    }
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("failed to read {}", path.display())),
    }
}
fn nonce() -> Result<u128> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos())
}
/// Follow symlinks to the final path (relative targets resolve against the
/// link's directory). A dangling link yields the path it points at, so the
/// file is created there and the link survives.
fn resolve_link(path: &Path) -> PathBuf {
    let mut current = path.to_path_buf();
    for _ in 0..40 {
        let is_link = std::fs::symlink_metadata(&current)
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false);
        if !is_link {
            break;
        }
        let Ok(target) = std::fs::read_link(&current) else {
            break;
        };
        current = if target.is_absolute() {
            target
        } else {
            current
                .parent()
                .map_or_else(|| target.clone(), |parent| parent.join(&target))
        };
    }
    current
}
/// Write through symlinks: temp file + rename happen next to the real target,
/// so a symlinked config stays a symlink.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let path = &resolve_link(path);
    let parent = path.parent().context("missing parent directory")?;
    std::fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".soundx-{}-{}.tmp", std::process::id(), nonce()?));
    let result = (|| -> Result<()> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        if let Ok(metadata) = std::fs::metadata(path) {
            file.set_permissions(metadata.permissions())?;
        }
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result.with_context(|| format!("failed to update {}", path.display()))
}

const KEEP_BACKUPS: usize = 5;
/// Keep only the newest backups per agent; they can contain credentials.
fn prune_backups(home: &Path, agent: &str) {
    let directory = home.join(".soundx/backups");
    let Ok(entries) = std::fs::read_dir(&directory) else {
        return;
    };
    let prefix = format!("{agent}-");
    let mut backups: Vec<(u128, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let stem = name.strip_suffix(".bak")?;
            stem.strip_prefix(&prefix)?;
            let stamp = stem.rsplit('-').next()?.parse().ok()?;
            Some((stamp, entry.path()))
        })
        .collect();
    backups.sort();
    let excess = backups.len().saturating_sub(KEEP_BACKUPS);
    for (_, path) in backups.into_iter().take(excess) {
        let _ = std::fs::remove_file(path);
    }
}

struct Change {
    path: PathBuf,
    before: Option<Vec<u8>>,
    after: Option<Vec<u8>>,
}
fn apply(changes: &[Change]) -> Result<()> {
    for (index, change) in changes.iter().enumerate() {
        let result = (|| -> Result<()> {
            if read_optional(&change.path)? != change.before {
                bail!(
                    "{} changed during integration; retry after other writers finish",
                    change.path.display()
                );
            }
            match &change.after {
                Some(bytes) => write_atomic(&change.path, bytes),
                None => {
                    if change.before.is_some() {
                        // `remove_file` removes a symlink itself, never its target.
                        std::fs::remove_file(&change.path)?;
                    }
                    Ok(())
                }
            }
        })();
        if let Err(error) = result {
            let mut recovery = Vec::new();
            for previous in changes[..index].iter().rev() {
                let restored = (|| -> Result<()> {
                    if read_optional(&previous.path)? != previous.after {
                        bail!("concurrent change");
                    }
                    if let Some(bytes) = &previous.before {
                        write_atomic(&previous.path, bytes)?;
                    } else if previous.after.is_some() {
                        // We created the (possibly link-resolved) file; remove that one.
                        std::fs::remove_file(resolve_link(&previous.path))?;
                    }
                    Ok(())
                })();
                if let Err(problem) = restored {
                    recovery.push(format!("{}: {problem:#}", previous.path.display()));
                }
            }
            if !recovery.is_empty() {
                bail!(
                    "{error:#}; rollback requires attention: {}",
                    recovery.join("; ")
                );
            }
            return Err(error);
        }
    }
    Ok(())
}

fn install(home: &Path, agent: AgentTarget, executable: &Path) -> Result<Value> {
    let layout = layout(home, agent)?;
    if layout.name == "opencode"
        && home.join(".config/opencode/opencode.json").exists()
        && home.join(".config/opencode/opencode.jsonc").exists()
    {
        bail!("both opencode.json and opencode.jsonc exist; choose one before installing");
    }
    let previous = manifest(&layout)?;
    let executable = executable
        .to_str()
        .context("executable path is not valid Unicode")?;
    let entry = if layout.name == "opencode" {
        json!({"type":"local","command":[executable,"mcp"],"enabled":true})
    } else {
        json!({"command":executable,"args":["mcp"]})
    };
    let config = read_optional(&layout.config)?;
    let text = std::str::from_utf8(config.as_deref().unwrap_or_default())
        .context("agent config is not UTF-8")?;
    let (current, updated) = edit_config(text, &layout, Some(&entry))?;
    let owned = match (&current, &previous) {
        (None, _) => true,
        (Some(current), Some(old)) if old.config_owned && current == &old.entry => true,
        (Some(current), _) if current == &entry => {
            previous.as_ref().is_some_and(|old| old.config_owned)
        }
        _ => bail!(
            "{} already has a different soundx MCP entry; existing configuration was preserved",
            layout.config.display()
        ),
    };
    let mut changes = Vec::new();
    let mut files = BTreeMap::new();
    for path in &layout.skills {
        let before = read_optional(path)?;
        if let Some(content) = &before {
            let previously_managed = previous
                .as_ref()
                .and_then(|old| old.files.get(path))
                .is_some_and(|text| text.as_bytes() == content);
            if content != SKILL.as_bytes() && !previously_managed {
                bail!(
                    "{} already exists with different content; left unchanged",
                    path.display()
                );
            }
            // Identical pre-existing user files are usable but are not ours to uninstall.
            if !previous
                .as_ref()
                .is_some_and(|old| old.files.contains_key(path))
                && content == SKILL.as_bytes()
            {
                continue;
            }
        }
        files.insert(path.clone(), SKILL.to_owned());
        changes.push(Change {
            path: path.clone(),
            before,
            after: Some(SKILL.as_bytes().to_vec()),
        });
    }
    let backup = if config.is_some() && text != updated && owned {
        let path = home.join(format!(".soundx/backups/{}-{}.bak", layout.name, nonce()?));
        write_atomic(&path, config.as_deref().unwrap())?;
        Some(path)
    } else {
        None
    };
    if owned {
        changes.push(Change {
            path: layout.config.clone(),
            before: config,
            after: Some(updated.into_bytes()),
        });
    }
    let record = Manifest {
        config: layout.config.clone(),
        entry,
        config_owned: owned,
        files,
    };
    changes.push(Change {
        path: layout.manifest.clone(),
        before: read_optional(&layout.manifest)?,
        after: Some(serde_json::to_vec_pretty(&record)?),
    });
    apply(&changes)?;
    prune_backups(home, layout.name);
    Ok(
        json!({"agent":layout.name,"status":"installed","config":layout.config,"skills":layout.skills,"backup":backup,"restart_agent":true}),
    )
}

#[cfg(test)]
fn remove(home: &Path, agent: AgentTarget) -> Result<Value> {
    remove_owned(home, agent, None)
}

fn remove_owned(home: &Path, agent: AgentTarget, executable: Option<&Path>) -> Result<Value> {
    let layout = layout(home, agent)?;
    let Some(manifest) = manifest(&layout)? else {
        return Ok(json!({"agent":layout.name,"status":"not_installed"}));
    };
    if let Some(executable) = executable {
        let installed = manifest.entry["command"]
            .as_str()
            .or_else(|| manifest.entry["command"][0].as_str())
            .context("invalid managed executable")?;
        let normalize = |path: &Path| {
            let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
            let text = path.to_string_lossy().into_owned();
            if cfg!(windows) {
                text.to_lowercase()
            } else {
                text
            }
        };
        if normalize(Path::new(installed)) != normalize(executable) {
            return Ok(json!({"agent":layout.name,"status":"belongs_to_other_installation"}));
        }
    }
    let mut changes = Vec::new();
    let mut preserved = Vec::new();
    let mut config_paths = vec![layout.config.clone()];
    if manifest.config != layout.config {
        config_paths.push(manifest.config.clone());
    }
    for config_path in config_paths {
        if !manifest.config_owned {
            continue;
        }
        let Some(config) = read_optional(&config_path)? else {
            continue;
        };
        let text = std::str::from_utf8(&config)?;
        let (current, updated) = edit_config(text, &layout, None)?;
        if current.as_ref() == Some(&manifest.entry) {
            let backup = home.join(format!(
                ".soundx/backups/{}-remove-{}.bak",
                layout.name,
                nonce()?
            ));
            write_atomic(&backup, &config)?;
            changes.push(Change {
                path: config_path.clone(),
                before: Some(config),
                after: Some(updated.into_bytes()),
            });
        } else if current.is_some() {
            preserved.push(config_path);
        }
    }
    for (path, expected) in &manifest.files {
        if let Some(content) = read_optional(path)? {
            if content == expected.as_bytes() {
                changes.push(Change {
                    path: path.clone(),
                    before: Some(content),
                    after: None,
                });
            } else {
                preserved.push(path.clone());
            }
        }
    }
    if preserved.is_empty() {
        changes.push(Change {
            path: layout.manifest.clone(),
            before: read_optional(&layout.manifest)?,
            after: None,
        });
    }
    apply(&changes)?;
    prune_backups(home, layout.name);
    for path in &layout.skills {
        if let Some(parent) = path.parent() {
            let _ = std::fs::remove_dir(parent);
        }
    }
    Ok(
        json!({"agent":layout.name,"status":if preserved.is_empty() {"removed"} else {"modified_files_preserved"},"preserved":preserved}),
    )
}

/// `create` is only for install; list/remove must not invent directories.
fn home_path(override_path: Option<PathBuf>, create: bool) -> Result<PathBuf> {
    let path = override_path
        .or_else(|| {
            std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
        })
        .context("user home unavailable; pass --home")?;
    let path = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()?.join(path)
    };
    if create {
        std::fs::create_dir_all(&path)?;
    } else if !path.is_dir() {
        bail!("integration home does not exist: {}", path.display());
    }
    std::fs::canonicalize(path).context("could not resolve integration home")
}

pub fn run(args: IntegrationArgs) -> Result<()> {
    let value = match args.command {
        IntegrationCommand::List { home } => {
            let home = home_path(home, false)?;
            let mut targets = Vec::new();
            for agent in TARGETS {
                let layout = layout(&home, agent)?;
                targets.push(json!({"agent":layout.name,"managed":layout.manifest.is_file(),"config":layout.config,"skills":layout.skills}));
            }
            json!(targets)
        }
        IntegrationCommand::Install { agent, home } => {
            install(&home_path(home, true)?, agent, &std::env::current_exe()?)?
        }
        IntegrationCommand::Remove {
            agent,
            all,
            only_executable,
            home,
        } => {
            let home = home_path(home, false)?;
            if all {
                let mut results = Vec::new();
                for agent in TARGETS {
                    results.push(match remove_owned(&home, agent, only_executable.as_deref()) {
                        Ok(value) => value,
                        Err(error) => json!({"agent":layout(&home, agent)?.name,"status":"failed","error":format!("{error:#}")}),
                    });
                }
                json!(results)
            } else {
                remove_owned(
                    &home,
                    agent.ok_or_else(|| anyhow!("specify --agent or --all"))?,
                    only_executable.as_deref(),
                )?
            }
        }
    };
    println!("{}", serde_json::to_string_pretty(&value)?);
    let incomplete = |value: &Value| {
        matches!(
            value["status"].as_str(),
            Some("modified_files_preserved" | "failed")
        )
    };
    if incomplete(&value)
        || value
            .as_array()
            .is_some_and(|values| values.iter().any(incomplete))
    {
        bail!("some integration files require attention; see the JSON report");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn root() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "soundx-integration-test-{}-{}",
            std::process::id(),
            nonce().unwrap()
        ));
        std::fs::create_dir(&path).unwrap();
        path
    }
    #[test]
    fn every_agent_preserves_unrelated_settings_and_roundtrips() {
        for agent in TARGETS {
            let home = root();
            let layout = layout(&home, agent).unwrap();
            let initial = if layout.name == "codex" {
                "# user comment\nmodel = 'custom'\n[mcp_servers.other]\ncommand = 'other'\n"
            } else if layout.name == "opencode" {
                "{\n// user comment\n\"theme\":\"dark\",\"mcp\":{\"other\":{\"type\":\"remote\",\"url\":\"https://example.invalid\"}}\n}"
            } else {
                "{\n// user comment\n\"theme\":\"dark\",\"mcpServers\":{\"other\":{\"command\":\"other\"}}\n}"
            };
            write_atomic(&layout.config, initial.as_bytes()).unwrap();
            let exe = home.join("Program Files/音訊/soundx.exe");
            install(&home, agent, &exe).unwrap();
            install(&home, agent, &exe).unwrap();
            let installed = std::fs::read_to_string(&layout.config).unwrap();
            assert!(installed.contains("user comment"));
            assert!(installed.contains("other"));
            assert!(edit_config(&installed, &layout, None).unwrap().0.is_some());
            assert_eq!(remove(&home, agent).unwrap()["status"], "removed");
            let remaining = std::fs::read_to_string(&layout.config).unwrap();
            assert!(remaining.contains("user comment"));
            assert!(remaining.contains("other"));
            assert!(edit_config(&remaining, &layout, None).unwrap().0.is_none());
            assert!(layout.skills.iter().all(|path| !path.exists()));
            std::fs::remove_dir_all(home).unwrap();
        }
    }
    #[test]
    fn conflicting_and_modified_files_are_preserved() {
        let home = root();
        let agent = AgentTarget::Claude;
        let layout = layout(&home, agent).unwrap();
        let original = br#"{"mcpServers":{"soundx":{"command":"user-owned"}}}"#;
        write_atomic(&layout.config, original).unwrap();
        assert!(install(&home, agent, &home.join("soundx.exe")).is_err());
        assert_eq!(std::fs::read(&layout.config).unwrap(), original);
        assert!(!layout.skills[0].exists());
        write_atomic(&layout.config, b"{}").unwrap();
        install(&home, agent, &home.join("soundx.exe")).unwrap();
        write_atomic(&layout.skills[0], b"user customization").unwrap();
        assert_eq!(
            remove(&home, agent).unwrap()["status"],
            "modified_files_preserved"
        );
        assert_eq!(
            std::fs::read(&layout.skills[0]).unwrap(),
            b"user customization"
        );
        std::fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn jsonc_comments_and_trailing_commas_survive() {
        let home = root();
        let config = home.join(".config/opencode/opencode.jsonc");
        write_atomic(&config, b"{// retain\n\"mcp\":{},\"theme\":\"dark\",}").unwrap();
        install(&home, AgentTarget::Opencode, &home.join("soundx.exe")).unwrap();
        assert!(
            std::fs::read_to_string(&config)
                .unwrap()
                .contains("// retain")
        );
        remove(&home, AgentTarget::Opencode).unwrap();
        assert!(!home.join(".config/opencode/opencode.json").exists());
        std::fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn opencode_config_migration_and_duplicate_copy_can_be_removed() {
        for keep_original in [false, true] {
            let home = root();
            let agent = AgentTarget::Opencode;
            install(&home, agent, &home.join("soundx.exe")).unwrap();
            let json = home.join(".config/opencode/opencode.json");
            let jsonc = json.with_extension("jsonc");
            if keep_original {
                std::fs::copy(&json, &jsonc).unwrap();
                assert!(install(&home, agent, &home.join("soundx.exe")).is_err());
            } else {
                std::fs::rename(&json, &jsonc).unwrap();
            }
            assert_eq!(remove(&home, agent).unwrap()["status"], "removed");
            let layout = layout(&home, agent).unwrap();
            for path in [json, jsonc] {
                if path.exists() {
                    let content = std::fs::read_to_string(path).unwrap();
                    assert!(edit_config(&content, &layout, None).unwrap().0.is_none());
                }
            }
            std::fs::remove_dir_all(home).unwrap();
        }
    }
    #[test]
    fn identical_preexisting_files_are_not_claimed_after_reinstall() {
        let home = root();
        let agent = AgentTarget::Claude;
        let layout = layout(&home, agent).unwrap();
        let exe = home.join("soundx.exe");
        let entry = json!({"command":exe,"args":["mcp"]});
        let (_, config) = edit_config("{}", &layout, Some(&entry)).unwrap();
        write_atomic(&layout.config, config.as_bytes()).unwrap();
        write_atomic(&layout.skills[0], SKILL.as_bytes()).unwrap();
        install(&home, agent, &exe).unwrap();
        install(&home, agent, &exe).unwrap();
        assert_eq!(remove(&home, agent).unwrap()["status"], "removed");
        assert_eq!(std::fs::read_to_string(&layout.config).unwrap(), config);
        assert_eq!(std::fs::read_to_string(&layout.skills[0]).unwrap(), SKILL);
        std::fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn old_uninstaller_cannot_remove_newer_installation() {
        let home = root();
        let agent = AgentTarget::Codex;
        install(&home, agent, &home.join("new/soundx.exe")).unwrap();
        assert_eq!(
            remove_owned(&home, agent, Some(&home.join("old/soundx.exe"))).unwrap()["status"],
            "belongs_to_other_installation"
        );
        assert!(layout(&home, agent).unwrap().manifest.exists());
        assert_eq!(
            remove_owned(&home, agent, Some(&home.join("new/soundx.exe"))).unwrap()["status"],
            "removed"
        );
        std::fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn transaction_rolls_back_earlier_writes_on_late_conflict() {
        let home = root();
        let first = home.join("first");
        let second = home.join("second");
        write_atomic(&first, b"original").unwrap();
        write_atomic(&second, b"concurrent change").unwrap();
        let changes = [
            Change {
                path: first.clone(),
                before: Some(b"original".to_vec()),
                after: Some(b"new".to_vec()),
            },
            Change {
                path: second.clone(),
                before: None,
                after: Some(b"new".to_vec()),
            },
        ];
        assert!(apply(&changes).is_err());
        assert_eq!(std::fs::read(first).unwrap(), b"original");
        assert_eq!(std::fs::read(second).unwrap(), b"concurrent change");
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn symlinked_config_stays_a_link_and_target_is_updated() {
        let home = root();
        let agent = AgentTarget::Claude;
        let layout = layout(&home, agent).unwrap();
        let repo = home.join("dotfiles");
        std::fs::create_dir(&repo).unwrap();
        let target = repo.join("claude.json");
        std::fs::write(&target, "{\"theme\":\"dark\"}").unwrap();
        // Relative link target, as dotfile managers commonly create.
        if !try_symlink(&Path::new("dotfiles").join("claude.json"), &layout.config) {
            std::fs::remove_dir_all(home).unwrap();
            return;
        }
        install(&home, agent, &home.join("soundx.exe")).unwrap();
        assert!(
            std::fs::symlink_metadata(&layout.config)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        let text = std::fs::read_to_string(&target).unwrap();
        assert!(text.contains("theme") && text.contains("soundx"));
        assert_eq!(remove(&home, agent).unwrap()["status"], "removed");
        assert!(
            std::fs::symlink_metadata(&layout.config)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(!std::fs::read_to_string(&target).unwrap().contains("soundx"));
        std::fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn dangling_symlink_is_created_in_place_and_kept() {
        let home = root();
        let target = home.join("dotfiles/new.json");
        let link = home.join("link.json");
        if !try_symlink(&Path::new("dotfiles").join("new.json"), &link) {
            std::fs::remove_dir_all(home).unwrap();
            return;
        }
        write_atomic(&link, b"hello").unwrap();
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"hello");
        std::fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn removing_a_symlinked_skill_removes_only_the_link() {
        let home = root();
        let agent = AgentTarget::Claude;
        let layout = layout(&home, agent).unwrap();
        install(&home, agent, &home.join("soundx.exe")).unwrap();
        let skill = &layout.skills[0];
        let kept = home.join("kept-skill.md");
        std::fs::write(&kept, SKILL).unwrap();
        std::fs::remove_file(skill).unwrap();
        if !try_symlink(&kept, skill) {
            std::fs::remove_dir_all(home).unwrap();
            return;
        }
        assert_eq!(remove(&home, agent).unwrap()["status"], "removed");
        assert!(std::fs::symlink_metadata(skill).is_err());
        assert_eq!(std::fs::read_to_string(&kept).unwrap(), SKILL);
        std::fs::remove_dir_all(home).unwrap();
    }
    /// Create a file symlink; false when the platform denies it (Windows
    /// without privilege), so the test is skipped gracefully.
    fn try_symlink(target: &Path, link: &Path) -> bool {
        #[cfg(unix)]
        let result = std::os::unix::fs::symlink(target, link);
        #[cfg(windows)]
        let result = std::os::windows::fs::symlink_file(target, link);
        match result {
            Ok(()) => true,
            Err(error) if cfg!(windows) => {
                eprintln!("skipping symlink test: {error}");
                false
            }
            Err(error) => panic!("symlink failed: {error}"),
        }
    }
    #[test]
    fn only_newest_backups_are_kept_per_agent() {
        let home = root();
        let backups = home.join(".soundx/backups");
        std::fs::create_dir_all(&backups).unwrap();
        for stamp in 1..=9u32 {
            for name in ["claude", "claude-remove", "codex"] {
                std::fs::write(backups.join(format!("{name}-{stamp:03}.bak")), "x").unwrap();
            }
        }
        std::fs::write(backups.join("claude-notes.txt"), "keep").unwrap();
        prune_backups(&home, "claude");
        let names = |prefix: &str| {
            let mut found: Vec<String> = std::fs::read_dir(&backups)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().into_string().unwrap())
                .filter(|name| name.starts_with(prefix) && name.ends_with(".bak"))
                .collect();
            found.sort();
            found
        };
        // claude- and claude-remove- share one budget of five (newest wins).
        assert_eq!(names("claude-").len(), 5);
        assert!(names("claude-").iter().all(|n| {
            ["007.bak", "008.bak", "009.bak"]
                .iter()
                .any(|t| n.ends_with(t))
        }));
        assert_eq!(names("codex-").len(), 9);
        assert!(backups.join("claude-notes.txt").exists());
        std::fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn repeated_installs_do_not_accumulate_backups() {
        let home = root();
        let agent = AgentTarget::Claude;
        let layout = layout(&home, agent).unwrap();
        for round in 0..8 {
            write_atomic(&layout.config, format!("{{\"round\":{round}}}").as_bytes()).unwrap();
            install(&home, agent, &home.join(format!("soundx-{round}.exe"))).ok();
            let _ = remove(&home, agent);
        }
        let count = std::fs::read_dir(home.join(".soundx/backups"))
            .unwrap()
            .count();
        assert!(count <= KEEP_BACKUPS, "{count}");
        std::fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn home_path_only_creates_when_asked() {
        let base = root();
        let missing = base.join("typo/nested");
        assert!(home_path(Some(missing.clone()), false).is_err());
        assert!(!missing.exists());
        assert!(!base.join("typo").exists());
        assert!(home_path(Some(missing.clone()), true).is_ok());
        assert!(missing.is_dir());
        std::fs::remove_dir_all(base).unwrap();
    }
}
