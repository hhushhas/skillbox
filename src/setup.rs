use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use clap::{ArgAction, Args, ValueEnum};
use serde::Serialize;
use serde_json::{Map, Value, json};

const CLAUDE_HOOK_SOURCE: &str = include_str!("../scripts/claude-skillbox-hook.mjs");
const PI_EXTENSION_SOURCE: &str = include_str!("../assets/pi-skillbox-audit.ts");
const CLAUDE_HOOK_FILE: &str = "claude-skillbox-hook.mjs";
const PI_EXTENSION_FILE: &str = "skillbox-audit.ts";

#[derive(Debug, Args)]
pub struct SetupArgs {
    /// Harnesses to configure; repeat the flag or separate values with commas.
    #[arg(long = "harness", value_enum, value_delimiter = ',', action = ArgAction::Append)]
    harnesses: Vec<Harness>,
    /// Show current harness configuration without changing files.
    #[arg(long, conflicts_with_all = ["dry_run", "yes"])]
    status: bool,
    /// Show what would change without writing files.
    #[arg(long)]
    dry_run: bool,
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,
    /// Skip the interactive selector and configure detected harnesses.
    #[arg(short = 'y', long)]
    yes: bool,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
#[value(rename_all = "kebab-case")]
enum Harness {
    Codex,
    #[value(alias = "claude")]
    ClaudeCode,
    Pi,
}

impl Harness {
    const ALL: [Self; 3] = [Self::Codex, Self::ClaudeCode, Self::Pi];

    fn label(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::ClaudeCode => "Claude Code",
            Self::Pi => "Pi",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum SetupStatus {
    BuiltIn,
    AlreadyConfigured,
    Installed,
    WouldInstall,
    NotConfigured,
    NotDetected,
    NeedsDependency,
    Error,
}

#[derive(Debug, Serialize)]
struct HarnessReport {
    harness: Harness,
    detected: bool,
    configured: bool,
    changed: bool,
    status: SetupStatus,
    message: String,
    paths: Vec<String>,
}

#[derive(Debug, Serialize)]
struct SetupReport {
    dry_run: bool,
    status_only: bool,
    harnesses: Vec<HarnessReport>,
}

#[derive(Clone, Debug)]
struct SetupPaths {
    claude_settings: PathBuf,
    claude_hook: PathBuf,
    pi_agent: PathBuf,
    pi_extension: PathBuf,
    pi_settings: PathBuf,
}

impl SetupPaths {
    fn from_environment() -> Result<Self> {
        let home = env::var_os("SKILLBOX_SETUP_HOME")
            .map(PathBuf::from)
            .or_else(dirs::home_dir)
            .ok_or_else(|| anyhow!("failed to locate home directory"))?;
        let pi_agent = env::var_os("PI_CODING_AGENT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".pi").join("agent"));
        Ok(Self::from_home_and_pi(home, pi_agent))
    }

    fn from_home_and_pi(home: PathBuf, pi_agent: PathBuf) -> Self {
        Self {
            claude_settings: home.join(".claude").join("settings.json"),
            claude_hook: home.join(".skillbox").join("hooks").join(CLAUDE_HOOK_FILE),
            pi_extension: pi_agent.join("extensions").join(PI_EXTENSION_FILE),
            pi_settings: pi_agent.join("settings.json"),
            pi_agent,
        }
    }
}

#[derive(Debug)]
struct ClaudeState {
    detected: bool,
    settings: Option<Value>,
    hook_matches: bool,
}

#[derive(Debug)]
struct PiState {
    detected: bool,
    package_registered: bool,
    global_extension_matches: bool,
}

#[derive(Debug, Default)]
struct FileChange {
    changed: bool,
    backup: Option<PathBuf>,
}

pub fn run(args: SetupArgs) -> Result<()> {
    let paths = SetupPaths::from_environment()?;
    let harnesses = if args.status {
        selected_for_status(&args.harnesses)
    } else {
        select_harnesses(&args, &paths)?
    };
    if harnesses.is_empty() {
        bail!("no harnesses selected; use --harness or run this command in a terminal")
    }

    let mut reports = Vec::with_capacity(harnesses.len());
    let mut failed = false;
    for harness in harnesses {
        let result = if args.status {
            status_harness(harness, &paths)
        } else {
            configure_harness(harness, &paths, args.dry_run)
        };
        match result {
            Ok(report) => {
                if !args.status {
                    failed |= matches!(
                        report.status,
                        SetupStatus::NotDetected | SetupStatus::NeedsDependency
                    );
                }
                reports.push(report);
            }
            Err(error) => {
                failed = true;
                reports.push(HarnessReport {
                    harness,
                    detected: harness_detected(harness, &paths),
                    configured: false,
                    changed: false,
                    status: SetupStatus::Error,
                    message: format!("{error:#}"),
                    paths: harness_paths(harness, &paths),
                });
            }
        }
    }

    print_report(
        &SetupReport {
            dry_run: args.dry_run,
            status_only: args.status,
            harnesses: reports,
        },
        args.json,
    )?;
    if failed {
        bail!("one or more selected harnesses could not be configured")
    }
    Ok(())
}

fn selected_for_status(requested: &[Harness]) -> Vec<Harness> {
    if requested.is_empty() {
        Harness::ALL.to_vec()
    } else {
        normalize_harnesses(requested.iter().copied())
    }
}

fn select_harnesses(args: &SetupArgs, paths: &SetupPaths) -> Result<Vec<Harness>> {
    if !args.harnesses.is_empty() {
        return Ok(normalize_harnesses(args.harnesses.iter().copied()));
    }

    let detected = detected_harnesses(paths);
    if args.yes || args.json || !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Ok(detected);
    }

    println!("Select harnesses to configure (press Enter for detected harnesses):");
    for (index, harness) in Harness::ALL.iter().enumerate() {
        let marker = if detected.contains(harness) {
            "detected"
        } else {
            "not detected"
        };
        println!("  {}. {} [{}]", index + 1, harness.label(), marker);
    }
    print!("Selection (numbers, names, all, or none): ");
    io::stdout()
        .flush()
        .context("failed to flush setup selector")?;
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("failed to read setup selection")?;
    if input.trim().is_empty() {
        return Ok(detected);
    }
    parse_selection(&input)
}

fn parse_selection(input: &str) -> Result<Vec<Harness>> {
    let input = input.trim();
    if input.eq_ignore_ascii_case("all") {
        return Ok(Harness::ALL.to_vec());
    }
    if input.eq_ignore_ascii_case("none") {
        return Ok(Vec::new());
    }

    let mut selected = Vec::new();
    for item in input
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        let harness = match item.parse::<usize>() {
            Ok(index) => Harness::ALL
                .get(
                    index
                        .checked_sub(1)
                        .ok_or_else(|| anyhow!("selector numbers start at 1"))?,
                )
                .copied()
                .ok_or_else(|| anyhow!("unknown harness selector '{item}'"))?,
            Err(_) => match item.to_ascii_lowercase().as_str() {
                "codex" => Harness::Codex,
                "claude" | "claude-code" => Harness::ClaudeCode,
                "pi" => Harness::Pi,
                _ => bail!("unknown harness selector '{item}'"),
            },
        };
        selected.push(harness);
    }
    Ok(normalize_harnesses(selected))
}

fn normalize_harnesses<I>(requested: I) -> Vec<Harness>
where
    I: IntoIterator<Item = Harness>,
{
    let requested = requested.into_iter().collect::<Vec<_>>();
    Harness::ALL
        .into_iter()
        .filter(|harness| requested.contains(harness))
        .collect()
}

fn detected_harnesses(paths: &SetupPaths) -> Vec<Harness> {
    Harness::ALL
        .into_iter()
        .filter(|harness| harness_detected(*harness, paths))
        .collect()
}

fn harness_detected(harness: Harness, paths: &SetupPaths) -> bool {
    match harness {
        Harness::Codex => command_exists("codex") || env::var_os("CODEX_THREAD_ID").is_some(),
        Harness::ClaudeCode => command_exists("claude") || paths.claude_settings.exists(),
        Harness::Pi => command_exists("pi") || paths.pi_agent.exists(),
    }
}

fn configure_harness(harness: Harness, paths: &SetupPaths, dry_run: bool) -> Result<HarnessReport> {
    match harness {
        Harness::Codex => configure_codex(paths),
        Harness::ClaudeCode => configure_claude(paths, dry_run),
        Harness::Pi => configure_pi(paths, dry_run),
    }
}

fn status_harness(harness: Harness, paths: &SetupPaths) -> Result<HarnessReport> {
    match harness {
        Harness::Codex => configure_codex(paths),
        Harness::ClaudeCode => {
            let state = claude_state(paths)?;
            claude_report(
                paths,
                state.hook_matches
                    && state
                        .settings
                        .as_ref()
                        .is_some_and(|settings| has_usable_claude_hooks(settings, paths)),
                false,
            )
        }
        Harness::Pi => {
            let state = pi_state(paths)?;
            pi_report(
                paths,
                state.package_registered || state.global_extension_matches,
                false,
            )
        }
    }
}

fn configure_codex(paths: &SetupPaths) -> Result<HarnessReport> {
    let detected = harness_detected(Harness::Codex, paths);
    Ok(HarnessReport {
        harness: Harness::Codex,
        detected,
        configured: detected,
        changed: false,
        status: if detected {
            SetupStatus::BuiltIn
        } else {
            SetupStatus::NotDetected
        },
        message: if detected {
            "thread capture is built in; no files changed (model ID remains unset)".to_string()
        } else {
            "Codex was not detected".to_string()
        },
        paths: Vec::new(),
    })
}

fn configure_claude(paths: &SetupPaths, dry_run: bool) -> Result<HarnessReport> {
    let state = claude_state(paths)?;
    if !state.detected {
        return Ok(not_detected_report(Harness::ClaudeCode, paths));
    }

    let hook_matches = state.hook_matches;
    let node = command_path("node");
    let settings_have_current_hooks = state
        .settings
        .as_ref()
        .is_some_and(|settings| has_usable_claude_hooks(settings, paths));
    if hook_matches && settings_have_current_hooks {
        return claude_report(paths, true, false);
    }

    if node.is_none() {
        return Ok(HarnessReport {
            harness: Harness::ClaudeCode,
            detected: true,
            configured: false,
            changed: false,
            status: SetupStatus::NeedsDependency,
            message: "Node.js is required to install the Claude Code hook".to_string(),
            paths: harness_paths(Harness::ClaudeCode, paths),
        });
    }

    let mut settings = state.settings.unwrap_or_else(|| Value::Object(Map::new()));
    let mut settings_changed = false;
    let node =
        node.ok_or_else(|| anyhow!("Node.js is required to install the Claude Code hook"))?;
    let command = |event: &str| {
        format!(
            "{} {} {}",
            shell_quote(&node),
            shell_quote(&paths.claude_hook),
            event
        )
    };
    settings_changed |= add_command_hook(
        &mut settings,
        "SessionStart",
        None,
        &command("session-start"),
    )?;
    settings_changed |= add_command_hook(
        &mut settings,
        "PreToolUse",
        Some("Bash"),
        &command("pre-tool-use"),
    )?;
    settings_changed |=
        add_command_hook(&mut settings, "SessionEnd", None, &command("session-end"))?;

    if dry_run {
        return Ok(HarnessReport {
            harness: Harness::ClaudeCode,
            detected: true,
            configured: false,
            changed: false,
            status: SetupStatus::WouldInstall,
            message: "would install the Claude Code hook and preserve existing settings"
                .to_string(),
            paths: harness_paths(Harness::ClaudeCode, paths),
        });
    }

    let hook_change = install_embedded_file(&paths.claude_hook, CLAUDE_HOOK_SOURCE)?;
    let settings_backup = if settings_changed {
        let backup = if paths.claude_settings.exists() {
            Some(backup_existing(&paths.claude_settings)?)
        } else {
            None
        };
        write_json_atomic(&paths.claude_settings, &settings)?;
        backup
    } else {
        None
    };
    let mut paths_out = harness_paths(Harness::ClaudeCode, paths);
    append_backup_path(&mut paths_out, hook_change.backup);
    append_backup_path(&mut paths_out, settings_backup);
    let changed = hook_change.changed || settings_changed;
    Ok(HarnessReport {
        harness: Harness::ClaudeCode,
        detected: true,
        configured: true,
        changed,
        status: if changed {
            SetupStatus::Installed
        } else {
            SetupStatus::AlreadyConfigured
        },
        message: if changed {
            "installed the Claude Code SessionStart, PreToolUse, and SessionEnd hooks".to_string()
        } else {
            "Claude Code hooks are already configured".to_string()
        },
        paths: paths_out,
    })
}

fn claude_report(paths: &SetupPaths, configured: bool, changed: bool) -> Result<HarnessReport> {
    let detected = harness_detected(Harness::ClaudeCode, paths);
    Ok(HarnessReport {
        harness: Harness::ClaudeCode,
        detected,
        configured,
        changed,
        status: if !detected {
            SetupStatus::NotDetected
        } else if configured {
            SetupStatus::AlreadyConfigured
        } else {
            SetupStatus::NotConfigured
        },
        message: if !detected {
            "Claude Code was not detected".to_string()
        } else if configured {
            "Claude Code hooks are already configured".to_string()
        } else {
            "Claude Code is detected but the Skillbox hook is not configured".to_string()
        },
        paths: harness_paths(Harness::ClaudeCode, paths),
    })
}

fn configure_pi(paths: &SetupPaths, dry_run: bool) -> Result<HarnessReport> {
    let state = pi_state(paths)?;
    if !state.detected {
        return Ok(not_detected_report(Harness::Pi, paths));
    }
    if state.package_registered {
        return pi_report(paths, true, false);
    }
    if state.global_extension_matches {
        return pi_report(paths, true, false);
    }
    if dry_run {
        return Ok(HarnessReport {
            harness: Harness::Pi,
            detected: true,
            configured: false,
            changed: false,
            status: SetupStatus::WouldInstall,
            message: "would install the Pi global Skillbox extension".to_string(),
            paths: harness_paths(Harness::Pi, paths),
        });
    }

    let change = install_embedded_file(&paths.pi_extension, PI_EXTENSION_SOURCE)?;
    let mut paths_out = harness_paths(Harness::Pi, paths);
    append_backup_path(&mut paths_out, change.backup);
    Ok(HarnessReport {
        harness: Harness::Pi,
        detected: true,
        configured: true,
        changed: change.changed,
        status: if change.changed {
            SetupStatus::Installed
        } else {
            SetupStatus::AlreadyConfigured
        },
        message: if change.changed {
            "installed the Pi global Skillbox extension".to_string()
        } else {
            "Pi Skillbox extension is already configured".to_string()
        },
        paths: paths_out,
    })
}

fn pi_report(paths: &SetupPaths, configured: bool, changed: bool) -> Result<HarnessReport> {
    let detected = harness_detected(Harness::Pi, paths);
    let mut configured_paths = vec![paths.pi_settings.display().to_string()];
    if paths.pi_extension.exists() {
        configured_paths.insert(0, paths.pi_extension.display().to_string());
    }
    Ok(HarnessReport {
        harness: Harness::Pi,
        detected,
        configured,
        changed,
        status: if !detected {
            SetupStatus::NotDetected
        } else if configured {
            SetupStatus::AlreadyConfigured
        } else {
            SetupStatus::NotConfigured
        },
        message: if !detected {
            "Pi was not detected".to_string()
        } else if configured {
            "Pi Skillbox extension is already configured".to_string()
        } else {
            "Pi is detected but the Skillbox extension is not configured".to_string()
        },
        paths: configured_paths,
    })
}

fn not_detected_report(harness: Harness, paths: &SetupPaths) -> HarnessReport {
    HarnessReport {
        harness,
        detected: false,
        configured: false,
        changed: false,
        status: SetupStatus::NotDetected,
        message: format!("{} was not detected", harness.label()),
        paths: harness_paths(harness, paths),
    }
}

fn claude_state(paths: &SetupPaths) -> Result<ClaudeState> {
    let settings = if paths.claude_settings.exists() {
        Some(read_json(&paths.claude_settings)?)
    } else {
        None
    };
    let hook_matches = if paths.claude_hook.exists() {
        fs::read_to_string(&paths.claude_hook)
            .with_context(|| format!("failed to read {}", paths.claude_hook.display()))?
            == CLAUDE_HOOK_SOURCE
    } else {
        false
    };
    Ok(ClaudeState {
        detected: harness_detected(Harness::ClaudeCode, paths),
        settings,
        hook_matches,
    })
}

fn pi_state(paths: &SetupPaths) -> Result<PiState> {
    let package_registered = if paths.pi_settings.exists() {
        json_contains_skillbox_extension(&read_json(&paths.pi_settings)?)
    } else {
        false
    };
    let global_extension_matches = if paths.pi_extension.exists() {
        fs::read_to_string(&paths.pi_extension)
            .with_context(|| format!("failed to read {}", paths.pi_extension.display()))?
            == PI_EXTENSION_SOURCE
    } else {
        false
    };
    Ok(PiState {
        detected: harness_detected(Harness::Pi, paths),
        package_registered,
        global_extension_matches,
    })
}

fn has_usable_claude_hooks(settings: &Value, paths: &SetupPaths) -> bool {
    has_usable_command_hook(settings, "SessionStart", None, "session-start", paths)
        && has_usable_command_hook(settings, "PreToolUse", Some("Bash"), "pre-tool-use", paths)
        && has_usable_command_hook(settings, "SessionEnd", None, "session-end", paths)
}

fn has_usable_command_hook(
    settings: &Value,
    event: &str,
    matcher: Option<&str>,
    event_argument: &str,
    paths: &SetupPaths,
) -> bool {
    let Some(groups) = settings
        .get("hooks")
        .and_then(|hooks| hooks.get(event))
        .and_then(Value::as_array)
    else {
        return false;
    };
    groups.iter().any(|group| {
        let matcher_matches = match matcher {
            Some(matcher) => group.get("matcher").and_then(Value::as_str) == Some(matcher),
            None => group.get("matcher").is_none(),
        };
        matcher_matches
            && group
                .get("hooks")
                .and_then(Value::as_array)
                .is_some_and(|hooks| {
                    hooks.iter().any(|hook| {
                        hook.get("type").and_then(Value::as_str) == Some("command")
                            && hook
                                .get("command")
                                .and_then(Value::as_str)
                                .is_some_and(|command| {
                                    let Some(words) = shell_words(command) else {
                                        return false;
                                    };
                                    words.len() == 3
                                        && command_executable_is_usable(&words[0])
                                        && words[1] == paths.claude_hook.to_string_lossy()
                                        && words[2] == event_argument
                                })
                    })
                })
    })
}

fn command_executable_is_usable(executable: &str) -> bool {
    let path = Path::new(executable);
    if path.is_absolute() {
        path.is_file()
    } else {
        command_exists(executable)
    }
}

fn shell_words(command: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_single_quotes = false;
    let mut escaped = false;
    for character in command.chars() {
        if escaped {
            word.push(character);
            escaped = false;
        } else if character == '\\' && !in_single_quotes {
            escaped = true;
        } else if character == '\'' {
            in_single_quotes = !in_single_quotes;
        } else if character.is_whitespace() && !in_single_quotes {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
        } else {
            word.push(character);
        }
    }
    if escaped || in_single_quotes {
        return None;
    }
    if !word.is_empty() {
        words.push(word);
    }
    Some(words)
}

fn add_command_hook(
    settings: &mut Value,
    event: &str,
    matcher: Option<&str>,
    command: &str,
) -> Result<bool> {
    let root = settings
        .as_object_mut()
        .ok_or_else(|| anyhow!("Claude settings must contain a JSON object"))?;
    let hooks = root
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| anyhow!("Claude settings.hooks must be an object when present"))?;
    let groups = hooks
        .entry(event)
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or_else(|| anyhow!("Claude settings.hooks.{event} must be an array"))?;

    let group_index = groups.iter().position(|group| match matcher {
        Some(matcher) => group.get("matcher").and_then(Value::as_str) == Some(matcher),
        None => group.get("matcher").is_none(),
    });
    let group = if let Some(index) = group_index {
        groups[index]
            .as_object_mut()
            .ok_or_else(|| anyhow!("Claude settings.hooks.{event} entries must be objects"))?
    } else {
        let mut group = Map::new();
        if let Some(matcher) = matcher {
            group.insert("matcher".to_string(), Value::String(matcher.to_string()));
        }
        group.insert("hooks".to_string(), Value::Array(Vec::new()));
        groups.push(Value::Object(group));
        groups
            .last_mut()
            .and_then(Value::as_object_mut)
            .ok_or_else(|| anyhow!("failed to create Claude hook group"))?
    };
    let hooks = group
        .entry("hooks")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or_else(|| anyhow!("Claude settings hook group must contain an array"))?;
    if hooks.iter().any(|hook| {
        hook.get("type").and_then(Value::as_str) == Some("command")
            && hook.get("command").and_then(Value::as_str) == Some(command)
    }) {
        return Ok(false);
    }
    hooks.push(json!({ "type": "command", "command": command }));
    Ok(true)
}

fn json_contains_skillbox_extension(value: &Value) -> bool {
    match value {
        Value::String(value) => {
            let selector = value.trim();
            !selector.starts_with('-')
                && !selector.starts_with('!')
                && selector
                    .strip_prefix('+')
                    .unwrap_or(selector)
                    .contains(PI_EXTENSION_FILE)
        }
        Value::Array(values) => values.iter().any(json_contains_skillbox_extension),
        Value::Object(values) => values.values().any(json_contains_skillbox_extension),
        _ => false,
    }
}

fn read_json(path: &Path) -> Result<Value> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("failed to read JSON settings at {}", path.display()))?;
    serde_json::from_str(&contents)
        .with_context(|| format!("failed to parse JSON settings at {}", path.display()))
}

fn install_embedded_file(path: &Path, contents: &str) -> Result<FileChange> {
    let existing = if path.exists() {
        Some(fs::read(path).with_context(|| format!("failed to read {}", path.display()))?)
    } else {
        None
    };
    if existing.as_deref() == Some(contents.as_bytes()) {
        return Ok(FileChange::default());
    }

    if let Some(parent) = path.parent() {
        let parent_missing = !parent.exists();
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
        if parent_missing {
            set_mode(parent, 0o700)?;
        }
    }
    let backup = if existing.is_some() {
        Some(backup_existing(path)?)
    } else {
        None
    };
    let mode = existing
        .as_ref()
        .map(|_| file_mode(path, 0o600))
        .unwrap_or(0o600);
    write_atomic(path, contents.as_bytes(), mode)?;
    Ok(FileChange {
        changed: true,
        backup,
    })
}

fn write_json_atomic(path: &Path, value: &Value) -> Result<()> {
    let mut contents = serde_json::to_vec_pretty(value).context("failed to serialize settings")?;
    contents.push(b'\n');
    let mode = if path.exists() {
        file_mode(path, 0o600)
    } else {
        0o600
    };
    write_atomic(path, &contents, mode)
}

fn write_atomic(path: &Path, contents: &[u8], mode: u32) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("path {} has no parent", path.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| anyhow!("path {} has no file name", path.display()))?
        .to_string_lossy();
    let temporary = parent.join(format!(".{file_name}.{}.tmp", unique_suffix()));
    let result = (|| -> Result<()> {
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        let mut file = options
            .open(&temporary)
            .with_context(|| format!("failed to create {}", temporary.display()))?;
        set_mode(&temporary, mode)?;
        file.write_all(contents)
            .with_context(|| format!("failed to write {}", temporary.display()))?;
        file.sync_all()
            .with_context(|| format!("failed to flush {}", temporary.display()))?;
        drop(file);
        fs::rename(&temporary, path)
            .with_context(|| format!("failed to replace {}", path.display()))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn backup_existing(path: &Path) -> Result<PathBuf> {
    if !path.exists() {
        bail!("cannot back up missing file {}", path.display())
    }
    let file_name = path
        .file_name()
        .ok_or_else(|| anyhow!("path {} has no file name", path.display()))?
        .to_string_lossy();
    let backup = path.with_file_name(format!("{file_name}.bak-skillbox-{}", unique_suffix()));
    fs::copy(path, &backup).with_context(|| {
        format!(
            "failed to back up {} to {}",
            path.display(),
            backup.display()
        )
    })?;
    Ok(backup)
}

fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    format!("{}-{nanos}", process::id())
}

fn file_mode(path: &Path, default: u32) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fs::metadata(path)
            .map(|metadata| metadata.permissions().mode() & 0o777)
            .unwrap_or(default)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        default
    }
}

fn set_mode(path: &Path, mode: u32) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let permissions = fs::Permissions::from_mode(mode);
        fs::set_permissions(path, permissions)
            .with_context(|| format!("failed to set permissions on {}", path.display()))?;
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
    }
    Ok(())
}

fn command_path(name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    let mut names = vec![name.to_string()];
    if cfg!(windows) {
        names.extend([format!("{name}.exe"), format!("{name}.cmd")]);
    }
    env::split_paths(&path)
        .flat_map(|directory| names.iter().map(move |candidate| directory.join(candidate)))
        .find(|candidate| candidate.is_file())
}

fn command_exists(name: &str) -> bool {
    command_path(name).is_some()
}

fn shell_quote(value: &Path) -> String {
    shell_quote_string(&value.to_string_lossy())
}

fn shell_quote_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn harness_paths(harness: Harness, paths: &SetupPaths) -> Vec<String> {
    match harness {
        Harness::Codex => Vec::new(),
        Harness::ClaudeCode => vec![
            paths.claude_settings.display().to_string(),
            paths.claude_hook.display().to_string(),
        ],
        Harness::Pi => vec![
            paths.pi_extension.display().to_string(),
            paths.pi_settings.display().to_string(),
        ],
    }
}

fn append_backup_path(paths: &mut Vec<String>, backup: Option<PathBuf>) {
    if let Some(backup) = backup {
        paths.push(backup.display().to_string());
    }
}

fn print_report(report: &SetupReport, json_output: bool) -> Result<()> {
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(report).context("failed to serialize setup report")?
        );
        return Ok(());
    }

    if report.status_only {
        println!("Skillbox harness setup status:");
    } else if report.dry_run {
        println!("Skillbox harness setup (dry run):");
    } else {
        println!("Skillbox harness setup:");
    }
    for harness in &report.harnesses {
        println!("- {}: {}", harness.harness.label(), harness.message);
        for path in &harness.paths {
            println!("  {path}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    use tempfile::tempdir;

    #[test]
    fn selection_accepts_numbers_names_and_all() {
        assert_eq!(
            parse_selection("3,claude,3").unwrap(),
            vec![Harness::ClaudeCode, Harness::Pi]
        );
        assert_eq!(parse_selection("all").unwrap(), Harness::ALL.to_vec());
        assert!(parse_selection("0").is_err());
    }

    #[test]
    fn claude_setup_is_idempotent_and_preserves_settings() {
        let Some(_node) = command_path("node") else {
            return;
        };
        let home = tempdir().unwrap();
        let paths =
            SetupPaths::from_home_and_pi(home.path().to_path_buf(), home.path().join("pi-agent"));
        fs::create_dir_all(paths.claude_settings.parent().unwrap()).unwrap();
        fs::write(&paths.claude_settings, r#"{"model":"test-model"}"#).unwrap();

        let first = configure_claude(&paths, false).unwrap();
        assert!(first.changed);
        assert!(
            first
                .paths
                .iter()
                .any(|path| path.contains(".bak-skillbox-"))
        );
        let settings = read_json(&paths.claude_settings).unwrap();
        assert_eq!(settings["model"], "test-model");
        assert!(has_usable_claude_hooks(&settings, &paths));
        assert_eq!(
            fs::read_to_string(&paths.claude_hook).unwrap(),
            CLAUDE_HOOK_SOURCE
        );

        let second = configure_claude(&paths, false).unwrap();
        assert!(!second.changed);
        assert!(matches!(second.status, SetupStatus::AlreadyConfigured));
    }

    #[test]
    fn claude_setup_repairs_stale_hook_commands() {
        let Some(node) = command_path("node") else {
            return;
        };
        let home = tempdir().unwrap();
        let paths =
            SetupPaths::from_home_and_pi(home.path().to_path_buf(), home.path().join("pi-agent"));
        fs::create_dir_all(paths.claude_settings.parent().unwrap()).unwrap();
        fs::write(&paths.claude_settings, "{}").unwrap();

        configure_claude(&paths, false).unwrap();
        let mut settings = read_json(&paths.claude_settings).unwrap();
        for event in ["SessionStart", "PreToolUse", "SessionEnd"] {
            for group in settings["hooks"][event].as_array_mut().unwrap() {
                for hook in group["hooks"].as_array_mut().unwrap() {
                    if let Some(command) = hook["command"].as_str() {
                        hook["command"] = Value::String(
                            command.replace(&shell_quote(&node), "'/missing/skillbox-node'"),
                        );
                    }
                }
            }
        }
        fs::write(
            &paths.claude_settings,
            serde_json::to_vec_pretty(&settings).unwrap(),
        )
        .unwrap();

        let repaired = configure_claude(&paths, false).unwrap();
        assert!(repaired.changed);
        let settings = read_json(&paths.claude_settings).unwrap();
        assert!(has_usable_claude_hooks(&settings, &paths));
    }

    #[test]
    fn pi_setup_writes_conventional_global_extension_and_is_idempotent() {
        let home = tempdir().unwrap();
        let pi_agent = home.path().join("pi-agent");
        fs::create_dir_all(&pi_agent).unwrap();
        let paths = SetupPaths::from_home_and_pi(home.path().to_path_buf(), pi_agent);

        let first = configure_pi(&paths, false).unwrap();
        assert!(first.changed);
        assert_eq!(
            fs::read_to_string(&paths.pi_extension).unwrap(),
            PI_EXTENSION_SOURCE
        );

        let second = configure_pi(&paths, false).unwrap();
        assert!(!second.changed);
        assert!(matches!(second.status, SetupStatus::AlreadyConfigured));
    }

    #[test]
    fn pi_package_registration_counts_as_configured_without_duplicate_global_file() {
        let home = tempdir().unwrap();
        let pi_agent = home.path().join("pi-agent");
        fs::create_dir_all(&pi_agent).unwrap();
        let paths = SetupPaths::from_home_and_pi(home.path().to_path_buf(), pi_agent);
        fs::write(
            &paths.pi_settings,
            r#"{"packages":[{"extensions":["+extensions/skillbox-audit.ts"]}]}"#,
        )
        .unwrap();

        let report = configure_pi(&paths, false).unwrap();
        assert!(!report.changed);
        assert!(report.configured);
        assert!(!paths.pi_extension.exists());
    }

    #[test]
    fn disabled_pi_package_selector_does_not_count_as_configured() {
        let home = tempdir().unwrap();
        let pi_agent = home.path().join("pi-agent");
        fs::create_dir_all(&pi_agent).unwrap();
        let paths = SetupPaths::from_home_and_pi(home.path().to_path_buf(), pi_agent);
        fs::write(
            &paths.pi_settings,
            r#"{"packages":[{"extensions":["-extensions/skillbox-audit.ts"]}]}"#,
        )
        .unwrap();

        let report = configure_pi(&paths, false).unwrap();
        assert!(report.changed);
        assert!(paths.pi_extension.exists());
    }

    #[test]
    fn status_matches_existing_claude_and_pi_configuration() {
        let home = tempdir().unwrap();
        let pi_agent = home.path().join("pi-agent");
        fs::create_dir_all(&pi_agent).unwrap();
        let paths = SetupPaths::from_home_and_pi(home.path().to_path_buf(), pi_agent);
        fs::create_dir_all(paths.claude_settings.parent().unwrap()).unwrap();
        fs::write(&paths.claude_settings, "{}\n").unwrap();
        configure_claude(&paths, false).unwrap();
        configure_pi(&paths, false).unwrap();

        let claude = status_harness(Harness::ClaudeCode, &paths).unwrap();
        let pi = status_harness(Harness::Pi, &paths).unwrap();
        assert!(claude.configured);
        assert!(pi.configured);
        assert!(matches!(claude.status, SetupStatus::AlreadyConfigured));
        assert!(matches!(pi.status, SetupStatus::AlreadyConfigured));
    }
}
