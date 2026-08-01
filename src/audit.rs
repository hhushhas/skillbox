use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration as StdDuration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Duration as ChronoDuration, Local};
use clap::Args;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const AUDIT_SCHEMA_VERSION: u8 = 1;
const DEFAULT_AUDIT_LIMIT: usize = 50;
const MAX_QUERY_CHARS: usize = 4_096;
const LOCK_TIMEOUT: StdDuration = StdDuration::from_millis(500);
const LOCK_RETRY_DELAY: StdDuration = StdDuration::from_millis(5);

static EVENT_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Args)]
pub struct AuditArgs {
    /// Filter by operation: list, search, info, or fetch.
    #[arg(long, value_parser = parse_operation)]
    operation: Option<String>,
    /// Filter by requested or resolved skill name.
    #[arg(long)]
    skill: Option<String>,
    /// Filter by harness name.
    #[arg(long)]
    harness: Option<String>,
    /// Filter by thread or session ID.
    #[arg(long)]
    thread: Option<String>,
    /// Filter by model ID when a harness adapter supplied one.
    #[arg(long)]
    model: Option<String>,
    /// Filter by command status: success or error.
    #[arg(long, value_parser = parse_status)]
    status: Option<String>,
    /// Show events since an RFC3339 timestamp or a duration such as 24h.
    #[arg(long)]
    since: Option<String>,
    /// Maximum number of events to print.
    #[arg(long, default_value_t = DEFAULT_AUDIT_LIMIT)]
    limit: usize,
    /// Print matching events as JSONL.
    #[arg(long)]
    json: bool,
}

#[derive(Debug)]
pub struct Invocation {
    operation: String,
    skill: Option<String>,
    query: Option<String>,
    category: Option<String>,
    web: bool,
    names: bool,
    started: Instant,
    started_at: String,
}

impl Invocation {
    pub fn new(
        operation: &str,
        skill: Option<String>,
        query: Option<String>,
        category: Option<String>,
        web: bool,
        names: bool,
    ) -> Self {
        Self {
            operation: operation.to_string(),
            skill,
            query,
            category,
            web,
            names,
            started: Instant::now(),
            started_at: Local::now().to_rfc3339(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Details {
    pub result_count: Option<usize>,
    pub resolved_skill: Option<String>,
    pub source: Option<String>,
    pub content_hash: Option<String>,
    pub resource_count: Option<usize>,
}

#[derive(Debug)]
struct CommandFailure {
    details: Details,
    error: anyhow::Error,
}

impl fmt::Display for CommandFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if formatter.alternate() {
            write!(formatter, "{:#}", self.error)
        } else {
            write!(formatter, "{}", self.error)
        }
    }
}

impl std::error::Error for CommandFailure {}

pub fn with_details(error: anyhow::Error, details: Details) -> anyhow::Error {
    anyhow::Error::new(CommandFailure { details, error })
}

pub fn details_from_error(error: &anyhow::Error) -> Details {
    error
        .downcast_ref::<CommandFailure>()
        .map(|failure| failure.details.clone())
        .unwrap_or_default()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AuditEvent {
    schema_version: u8,
    event_id: String,
    started_at: String,
    completed_at: String,
    duration_ms: u64,
    cli_version: String,
    operation: String,
    status: String,
    harness: String,
    harness_source: String,
    thread_id: Option<String>,
    thread_id_source: Option<String>,
    model_id: Option<String>,
    model_id_source: Option<String>,
    transcript_path: Option<String>,
    cwd: Option<String>,
    pid: u32,
    skill: Option<String>,
    query: Option<String>,
    query_mode: Option<String>,
    category: Option<String>,
    web: bool,
    names: bool,
    result_count: Option<usize>,
    resolved_skill: Option<String>,
    source: Option<String>,
    content_hash: Option<String>,
    resource_count: Option<usize>,
    error_category: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
struct Metadata {
    harness: String,
    harness_source: String,
    thread_id: Option<String>,
    thread_id_source: Option<String>,
    model_id: Option<String>,
    model_id_source: Option<String>,
    transcript_path: Option<String>,
}

pub fn run(args: AuditArgs) -> Result<()> {
    let path = audit_path()?;
    let since = args.since.as_deref().map(parse_since).transpose()?;
    let mut events = read_events(&path)?;
    events.retain(|event| matches_filters(event, &args, since));
    events.reverse();
    events.truncate(args.limit);

    if args.json {
        for event in events {
            println!(
                "{}",
                serde_json::to_string(&event).context("failed to serialize audit event")?
            );
        }
    } else if events.is_empty() {
        println!("no audit events in {}", path.display());
    } else {
        for event in events {
            println!("{}", format_event(&event));
        }
    }
    Ok(())
}

pub fn record(invocation: Invocation, details: Details, success: bool) -> Result<()> {
    let completed_at = Local::now().to_rfc3339();
    let metadata = metadata_from_env(&environment_values());
    let (query, query_mode) = audit_query(invocation.query.as_deref(), query_mode_from_env()?);
    let event = AuditEvent {
        schema_version: AUDIT_SCHEMA_VERSION,
        event_id: event_id(),
        started_at: invocation.started_at,
        completed_at,
        duration_ms: invocation
            .started
            .elapsed()
            .as_millis()
            .min(u64::MAX as u128) as u64,
        cli_version: env!("CARGO_PKG_VERSION").to_string(),
        operation: invocation.operation,
        status: if success { "success" } else { "error" }.to_string(),
        harness: metadata.harness,
        harness_source: metadata.harness_source,
        thread_id: metadata.thread_id,
        thread_id_source: metadata.thread_id_source,
        model_id: metadata.model_id,
        model_id_source: metadata.model_id_source,
        transcript_path: metadata.transcript_path,
        cwd: std::env::current_dir()
            .ok()
            .map(|path| path.display().to_string()),
        pid: process::id(),
        skill: invocation.skill,
        query,
        query_mode,
        category: invocation.category,
        web: invocation.web,
        names: invocation.names,
        result_count: details.result_count,
        resolved_skill: details.resolved_skill,
        source: details.source,
        content_hash: details.content_hash,
        resource_count: details.resource_count,
        error_category: (!success).then_some("command_failed".to_string()),
    };

    append_event(&audit_path()?, &event)
}

pub fn content_hash(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    let hex = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("sha256:{hex}")
}

fn audit_path() -> Result<PathBuf> {
    if let Some(path) = non_empty_env("SKILLBOX_AUDIT_PATH") {
        return Ok(PathBuf::from(path));
    }

    dirs::home_dir()
        .map(|home| home.join(".skillbox").join("audit.jsonl"))
        .ok_or_else(|| anyhow!("failed to locate home directory for audit log"))
}

fn append_event(path: &Path, event: &AuditEvent) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create audit directory {}", parent.display()))?;
    }

    let mut file = open_locked(path, true)?;
    separate_truncated_tail(&mut file, path)?;
    let mut line = serde_json::to_vec(event).context("failed to serialize audit event")?;
    line.push(b'\n');
    file.write_all(&line)
        .with_context(|| format!("failed to append audit event to {}", path.display()))?;
    file.flush()
        .with_context(|| format!("failed to flush audit event to {}", path.display()))?;
    file.unlock()
        .with_context(|| format!("failed to unlock audit log {}", path.display()))
}

fn separate_truncated_tail(file: &mut File, path: &Path) -> Result<()> {
    if file
        .metadata()
        .with_context(|| format!("failed to inspect audit log {}", path.display()))?
        .len()
        == 0
    {
        return Ok(());
    }

    file.seek(SeekFrom::End(-1))
        .with_context(|| format!("failed to inspect audit log {}", path.display()))?;
    let mut last_byte = [0u8; 1];
    file.read_exact(&mut last_byte)
        .with_context(|| format!("failed to inspect audit log {}", path.display()))?;
    if last_byte[0] != b'\n' {
        file.write_all(b"\n")
            .with_context(|| format!("failed to separate audit events in {}", path.display()))?;
    }
    Ok(())
}

fn read_events(path: &Path) -> Result<Vec<AuditEvent>> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let mut file = open_locked(path, false)?;
    let mut text = String::new();
    file.read_to_string(&mut text)
        .with_context(|| format!("failed to read audit log {}", path.display()))?;
    file.unlock()
        .with_context(|| format!("failed to unlock audit log {}", path.display()))?;

    Ok(text
        .lines()
        .filter_map(|line| serde_json::from_str::<AuditEvent>(line).ok())
        .collect())
}

fn open_locked(path: &Path, exclusive: bool) -> Result<File> {
    let mut options = OpenOptions::new();
    options
        .create(exclusive)
        .read(true)
        .append(exclusive)
        .write(exclusive);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        options.mode(0o600);
    }
    let file = options
        .open(path)
        .with_context(|| format!("failed to open audit log {}", path.display()))?;
    set_user_only_file_permissions(path)?;
    let started = Instant::now();

    loop {
        let result = if exclusive {
            FileExt::try_lock_exclusive(&file)
        } else {
            FileExt::try_lock_shared(&file)
        };
        match result {
            Ok(()) => return Ok(file),
            Err(error) if is_lock_contended(&error) => {
                if started.elapsed() >= LOCK_TIMEOUT {
                    bail!("timed out waiting for audit log lock {}", path.display());
                }
                thread::sleep(LOCK_RETRY_DELAY);
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to lock audit log {}", path.display()));
            }
        }
    }
}

fn is_lock_contended(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::WouldBlock
        || error.raw_os_error() == fs2::lock_contended_error().raw_os_error()
}

fn set_user_only_file_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = fs::metadata(path)
            .with_context(|| format!("failed to inspect audit log {}", path.display()))?
            .permissions();
        permissions.set_mode(0o600);
        fs::set_permissions(path, permissions)
            .with_context(|| format!("failed to protect audit log {}", path.display()))?;
    }
    Ok(())
}

fn matches_filters(event: &AuditEvent, args: &AuditArgs, since: Option<DateTime<Local>>) -> bool {
    args.operation
        .as_ref()
        .is_none_or(|operation| operation == &event.operation)
        && args.skill.as_ref().is_none_or(|skill| {
            event.skill.as_ref() == Some(skill) || event.resolved_skill.as_ref() == Some(skill)
        })
        && args
            .harness
            .as_ref()
            .is_none_or(|harness| harness == &event.harness)
        && args
            .thread
            .as_ref()
            .is_none_or(|thread| event.thread_id.as_ref() == Some(thread))
        && args
            .model
            .as_ref()
            .is_none_or(|model| event.model_id.as_ref() == Some(model))
        && args
            .status
            .as_ref()
            .is_none_or(|status| status == &event.status)
        && since.is_none_or(|since| event_after(event, since))
}

fn event_after(event: &AuditEvent, since: DateTime<Local>) -> bool {
    DateTime::parse_from_rfc3339(&event.started_at)
        .map(|timestamp| timestamp.with_timezone(&Local) >= since)
        .unwrap_or(false)
}

fn parse_since(value: &str) -> Result<DateTime<Local>> {
    if let Some((amount, unit)) = value.split_at_checked(value.len().saturating_sub(1))
        && let Ok(amount) = amount.parse::<i64>()
    {
        let seconds_per_unit = match unit {
            "s" => 1,
            "m" => 60,
            "h" => 60 * 60,
            "d" => 24 * 60 * 60,
            _ => return parse_rfc3339(value),
        };
        let seconds = amount
            .checked_mul(seconds_per_unit)
            .ok_or_else(|| anyhow!("--since duration '{value}' is out of range"))?;
        let duration = ChronoDuration::try_seconds(seconds)
            .ok_or_else(|| anyhow!("--since duration '{value}' is out of range"))?;
        return Local::now()
            .checked_sub_signed(duration)
            .ok_or_else(|| anyhow!("--since duration '{value}' is out of range"));
    }
    parse_rfc3339(value)
}

fn parse_rfc3339(value: &str) -> Result<DateTime<Local>> {
    DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.with_timezone(&Local))
        .with_context(|| {
            format!("invalid --since value '{value}'; use RFC3339 or a duration such as 24h")
        })
}

fn format_event(event: &AuditEvent) -> String {
    let target = event
        .skill
        .as_deref()
        .or(event.resolved_skill.as_deref())
        .unwrap_or("-");
    let query = event
        .query
        .as_deref()
        .map(|query| format!(" query={query:?}"))
        .unwrap_or_default();
    let thread = event.thread_id.as_deref().unwrap_or("-");
    format!(
        "{} {} skill={}{} harness={} thread={} status={}{}",
        event.started_at,
        event.operation,
        target,
        query,
        event.harness,
        thread,
        event.status,
        event
            .result_count
            .map(|count| format!(" results={count}"))
            .unwrap_or_default()
    )
}

fn event_id() -> String {
    let counter = EVENT_COUNTER.fetch_add(1, Ordering::Relaxed);
    let timestamp = Local::now().timestamp_nanos_opt().unwrap_or_default();
    format!("{timestamp}-{}-{counter}", process::id())
}

fn query_mode_from_env() -> Result<&'static str> {
    let Some(value) = std::env::var_os("SKILLBOX_AUDIT_QUERY_MODE") else {
        return Ok("raw");
    };
    let value = value
        .to_str()
        .ok_or_else(|| anyhow!("SKILLBOX_AUDIT_QUERY_MODE must be valid UTF-8"))?;
    query_mode_from_value((!value.trim().is_empty()).then_some(value))
}

fn query_mode_from_value(value: Option<&str>) -> Result<&'static str> {
    match value {
        None | Some("raw") => Ok("raw"),
        Some("hash") => Ok("hash"),
        Some("omit") => Ok("omit"),
        Some(value) => bail!("invalid SKILLBOX_AUDIT_QUERY_MODE '{value}'; use raw, hash, or omit"),
    }
}

fn audit_query(query: Option<&str>, mode: &str) -> (Option<String>, Option<String>) {
    let Some(query) = query else {
        return (None, None);
    };
    let bounded = query.chars().take(MAX_QUERY_CHARS).collect::<String>();
    match mode {
        "hash" => (Some(content_hash(&bounded)), Some("hash".to_string())),
        "omit" => (None, Some("omit".to_string())),
        "raw" => (Some(bounded), Some("raw".to_string())),
        _ => (None, Some("omit".to_string())),
    }
}

fn metadata_from_env(values: &BTreeMap<String, String>) -> Metadata {
    let harness_override = non_empty_value(values, "SKILLBOX_HARNESS");
    let thread_override = non_empty_value(values, "SKILLBOX_THREAD_ID");
    let codex_thread = non_empty_value(values, "CODEX_THREAD_ID");
    let model_override = non_empty_value(values, "SKILLBOX_MODEL_ID");
    let transcript_path = non_empty_value(values, "SKILLBOX_TRANSCRIPT_PATH");

    let (harness, harness_source) = match harness_override {
        Some(harness) => (harness, "SKILLBOX_HARNESS".to_string()),
        None if codex_thread.is_some() => ("codex".to_string(), "CODEX_THREAD_ID".to_string()),
        None => ("unknown".to_string(), "unknown".to_string()),
    };
    let codex_thread_source = codex_thread.as_ref().map(|_| "CODEX_THREAD_ID".to_string());
    let (thread_id, thread_id_source) = match thread_override {
        Some(thread) => (Some(thread), Some("SKILLBOX_THREAD_ID".to_string())),
        None => (codex_thread, codex_thread_source),
    };

    Metadata {
        harness,
        harness_source,
        thread_id,
        thread_id_source,
        model_id: model_override,
        model_id_source: non_empty_value(values, "SKILLBOX_MODEL_ID")
            .map(|_| "SKILLBOX_MODEL_ID".to_string()),
        transcript_path,
    }
}

fn environment_values() -> BTreeMap<String, String> {
    std::env::vars_os()
        .filter_map(|(key, value)| Some((key.to_str()?.to_string(), value.to_str()?.to_string())))
        .collect()
}

fn non_empty_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn non_empty_value(values: &BTreeMap<String, String>, key: &str) -> Option<String> {
    values
        .get(key)
        .filter(|value| !value.trim().is_empty())
        .cloned()
}

fn parse_operation(value: &str) -> std::result::Result<String, String> {
    match value {
        "list" | "search" | "info" | "fetch" => Ok(value.to_string()),
        _ => Err("expected one of: list, search, info, fetch".to_string()),
    }
}

fn parse_status(value: &str) -> std::result::Result<String, String> {
    match value {
        "success" | "error" => Ok(value.to_string()),
        _ => Err("expected one of: success, error".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn env(values: &[(&str, &str)]) -> BTreeMap<String, String> {
        values
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    fn test_event() -> AuditEvent {
        AuditEvent {
            schema_version: 1,
            event_id: "event".to_string(),
            started_at: "2026-08-01T10:00:00+00:00".to_string(),
            completed_at: "2026-08-01T10:00:01+00:00".to_string(),
            duration_ms: 1,
            cli_version: "0.3.0".to_string(),
            operation: "fetch".to_string(),
            status: "success".to_string(),
            harness: "codex".to_string(),
            harness_source: "CODEX_THREAD_ID".to_string(),
            thread_id: Some("thread".to_string()),
            thread_id_source: Some("CODEX_THREAD_ID".to_string()),
            model_id: None,
            model_id_source: None,
            transcript_path: None,
            cwd: None,
            pid: 1,
            skill: Some("browser".to_string()),
            query: None,
            query_mode: None,
            category: None,
            web: false,
            names: false,
            result_count: None,
            resolved_skill: Some("browser".to_string()),
            source: Some("global".to_string()),
            content_hash: Some("sha256:test".to_string()),
            resource_count: Some(2),
            error_category: None,
        }
    }

    #[test]
    fn codex_thread_is_detected_without_model_id() {
        let metadata = metadata_from_env(&env(&[("CODEX_THREAD_ID", "thread")]));

        assert_eq!(metadata.harness, "codex");
        assert_eq!(metadata.thread_id.as_deref(), Some("thread"));
        assert_eq!(metadata.model_id, None);
        assert_eq!(
            metadata.thread_id_source.as_deref(),
            Some("CODEX_THREAD_ID")
        );
    }

    #[test]
    fn explicit_metadata_overrides_detected_values() {
        let metadata = metadata_from_env(&env(&[
            ("CODEX_THREAD_ID", "codex-thread"),
            ("SKILLBOX_HARNESS", "claude-code"),
            ("SKILLBOX_THREAD_ID", "claude-thread"),
            ("SKILLBOX_MODEL_ID", "claude-model"),
        ]));

        assert_eq!(metadata.harness, "claude-code");
        assert_eq!(metadata.thread_id.as_deref(), Some("claude-thread"));
        assert_eq!(metadata.model_id.as_deref(), Some("claude-model"));
        assert_eq!(metadata.harness_source, "SKILLBOX_HARNESS");
        assert_eq!(
            metadata.thread_id_source.as_deref(),
            Some("SKILLBOX_THREAD_ID")
        );
    }

    #[test]
    fn empty_overrides_do_not_hide_codex_metadata() {
        let metadata = metadata_from_env(&env(&[
            ("SKILLBOX_HARNESS", " "),
            ("SKILLBOX_THREAD_ID", ""),
            ("CODEX_THREAD_ID", "thread"),
        ]));

        assert_eq!(metadata.harness, "codex");
        assert_eq!(metadata.thread_id.as_deref(), Some("thread"));
    }

    #[test]
    fn query_modes_bound_and_transform_queries() {
        let query = "a".repeat(MAX_QUERY_CHARS + 1);
        let (raw, raw_mode) = audit_query(Some(&query), "raw");
        let (hashed, hash_mode) = audit_query(Some(&query), "hash");
        let (omitted, omit_mode) = audit_query(Some(&query), "omit");

        assert_eq!(raw.expect("raw query").chars().count(), MAX_QUERY_CHARS);
        assert_eq!(raw_mode.as_deref(), Some("raw"));
        assert_eq!(hashed.expect("hashed query").len(), 71);
        assert_eq!(hash_mode.as_deref(), Some("hash"));
        assert_eq!(omitted, None);
        assert_eq!(omit_mode.as_deref(), Some("omit"));
    }

    #[test]
    fn invalid_query_modes_fail_closed() {
        assert!(query_mode_from_value(Some("hashh")).is_err());

        let (query, mode) = audit_query(Some("private query"), "invalid");
        assert_eq!(query, None);
        assert_eq!(mode.as_deref(), Some("omit"));
    }

    #[test]
    fn command_failure_retains_audit_details() {
        let details = Details {
            resolved_skill: Some("browser".to_string()),
            source: Some("global".to_string()),
            content_hash: Some("sha256:test".to_string()),
            ..Default::default()
        };
        let error = with_details(anyhow!("late failure"), details.clone());

        assert_eq!(
            details_from_error(&error).resolved_skill,
            details.resolved_skill
        );
        assert_eq!(details_from_error(&error).source, details.source);
        assert_eq!(
            details_from_error(&error).content_hash,
            details.content_hash
        );
    }

    #[test]
    fn malformed_or_truncated_lines_are_skipped() {
        let root = tempfile::tempdir().expect("tempdir");
        let path = root.path().join("audit.jsonl");
        let event = test_event();
        fs::write(
            &path,
            format!(
                "{}\n{{\"schema_version\":1",
                serde_json::to_string(&event).expect("event JSON")
            ),
        )
        .expect("audit log");

        let events = read_events(&path).expect("read events");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_id, "event");
    }

    #[test]
    fn appends_after_a_truncated_tail_on_a_new_line() {
        let root = tempfile::tempdir().expect("tempdir");
        let path = root.path().join("audit.jsonl");
        fs::write(&path, b"{\"schema_version\":1").expect("truncated audit log");

        append_event(&path, &test_event()).expect("append event");

        let text = fs::read_to_string(&path).expect("audit log");
        assert_eq!(text.lines().count(), 2);
        assert_eq!(read_events(&path).expect("read events").len(), 1);
    }

    #[test]
    fn concurrent_appends_remain_complete_json_lines() {
        let root = tempfile::tempdir().expect("tempdir");
        let path = Arc::new(root.path().join("audit.jsonl"));
        let event = Arc::new(test_event());
        let workers = (0..8)
            .map(|_| {
                let path = Arc::clone(&path);
                let event = Arc::clone(&event);
                std::thread::spawn(move || append_event(&path, &event))
            })
            .collect::<Vec<_>>();

        for worker in workers {
            worker.join().expect("append worker").expect("append event");
        }

        assert_eq!(read_events(&path).expect("read events").len(), 8);
    }

    #[test]
    fn fs2_contention_errors_are_retryable() {
        assert!(is_lock_contended(&fs2::lock_contended_error()));
    }

    #[cfg(unix)]
    #[test]
    fn audit_log_is_user_only() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().expect("tempdir");
        let path = root.path().join("audit.jsonl");
        append_event(&path, &test_event()).expect("append event");

        let mode = fs::metadata(path)
            .expect("audit metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn filters_match_skill_thread_and_status() {
        let event = test_event();
        let args = AuditArgs {
            operation: Some("fetch".to_string()),
            skill: Some("browser".to_string()),
            harness: Some("codex".to_string()),
            thread: Some("thread".to_string()),
            model: None,
            status: Some("success".to_string()),
            since: None,
            limit: 50,
            json: false,
        };

        assert!(matches_filters(
            &event,
            &args,
            Some(Local::now() - ChronoDuration::days(1))
        ));
    }

    #[test]
    fn duration_since_values_are_supported() {
        let since = parse_since("24h").expect("duration");
        assert!(since < Local::now());
    }

    #[test]
    fn oversized_duration_since_values_return_errors() {
        assert!(parse_since("9223372036854775807d").is_err());
    }

    #[test]
    fn content_hash_is_stable() {
        assert_eq!(
            content_hash("skillbox"),
            "sha256:3375b63adc3d31e70f6849a22f0fae677490b3532e60a374719f2eac5bc9cfee".to_string()
        );
    }
}
