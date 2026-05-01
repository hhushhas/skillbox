use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use chrono::Local;
use clap::{Args, Parser, Subcommand};
use serde::Deserialize;

const DEFAULT_REGISTRY_OWNER: &str = "hhushhas";
const DEFAULT_REGISTRY_REPO: &str = "skillbox-registry";
const DEFAULT_REGISTRY_REF: &str = "main";
const ALLOWED_CATEGORIES: [&str; 7] = [
    "frontend", "backend", "ai", "cloud", "design", "browser", "project",
];

#[derive(Parser)]
#[command(name = "skillbox")]
#[command(version)]
#[command(about = "List and fetch trusted coding-agent skills on demand.")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// List available skills.
    List(ListArgs),
    /// Fetch a skill as markdown or a temporary folder.
    Fetch(FetchArgs),
    /// Remove Skillbox-created temp folders.
    Cleanup,
}

#[derive(Args)]
struct ListArgs {
    /// Limit output to a category.
    #[arg(long, value_parser = parse_category)]
    category: Option<String>,
}

#[derive(Args)]
struct FetchArgs {
    /// Skill name from the registry.
    name: String,
    /// Print only SKILL.md.
    #[arg(long, conflicts_with = "to_temp")]
    print: bool,
    /// Copy or download the full skill folder to temp and print its path.
    #[arg(long)]
    to_temp: bool,
}

#[derive(Debug, Deserialize)]
struct Config {
    registries: Option<Vec<RemoteRegistry>>,
}

#[derive(Debug, Clone, Deserialize)]
struct RemoteRegistry {
    owner: Option<String>,
    repo: String,
    #[serde(rename = "ref")]
    reference: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RegistryFile {
    version: u8,
    skills: BTreeMap<String, SkillEntry>,
}

#[derive(Debug, Clone, Deserialize)]
struct SkillEntry {
    category: String,
    description: String,
    path: String,
}

#[derive(Debug, Clone)]
struct ResolvedSkill {
    name: String,
    entry: SkillEntry,
    source: Source,
}

#[derive(Debug, Clone)]
enum Source {
    Project { root: PathBuf },
    Remote { registry: RemoteRegistry },
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli
        .command
        .unwrap_or(Command::List(ListArgs { category: None }))
    {
        Command::List(args) => list(args),
        Command::Fetch(args) => fetch(args),
        Command::Cleanup => cleanup(),
    }
}

fn list(args: ListArgs) -> Result<()> {
    let skills = load_skills()?;
    let filtered_by_category = args.category.is_some();
    for skill in skills {
        if args
            .category
            .as_deref()
            .is_some_and(|category| category != skill.entry.category)
        {
            continue;
        }
        println!("{}", format_list_line(&skill, filtered_by_category));
    }
    Ok(())
}

fn format_list_line(skill: &ResolvedSkill, filtered_by_category: bool) -> String {
    if filtered_by_category {
        format!("{}: {}", skill.name, skill.entry.description)
    } else {
        format!(
            "{} [{}]: {}",
            skill.name, skill.entry.category, skill.entry.description
        )
    }
}

fn fetch(args: FetchArgs) -> Result<()> {
    if !args.print && !args.to_temp {
        bail!("choose one: --print or --to-temp");
    }

    let skill = load_skills()?
        .into_iter()
        .find(|skill| skill.name == args.name)
        .ok_or_else(|| anyhow!("unknown skill '{}'", args.name))?;

    if args.print {
        let markdown = read_skill_markdown(&skill)?;
        print!("{markdown}");
        return Ok(());
    }

    let destination = copy_skill_to_temp(&skill)?;
    println!("{}", destination.display());
    Ok(())
}

fn cleanup() -> Result<()> {
    let root = temp_root();
    if root.exists() {
        fs::remove_dir_all(&root)
            .with_context(|| format!("failed to remove {}", root.display()))?;
    }
    fs::create_dir_all(&root).with_context(|| format!("failed to recreate {}", root.display()))?;
    println!("cleaned {}", root.display());
    Ok(())
}

fn parse_category(value: &str) -> std::result::Result<String, String> {
    if ALLOWED_CATEGORIES.contains(&value) {
        Ok(value.to_string())
    } else {
        Err(format!(
            "unknown category '{}'; expected one of: {}",
            value,
            ALLOWED_CATEGORIES.join(", ")
        ))
    }
}

fn load_skills() -> Result<Vec<ResolvedSkill>> {
    let mut merged = BTreeMap::<String, ResolvedSkill>::new();

    if let Some((root, registry)) = find_project_registry()? {
        for (name, entry) in registry.skills {
            validate_entry(&name, &entry)?;
            merged.insert(
                name.clone(),
                ResolvedSkill {
                    name,
                    entry,
                    source: Source::Project { root: root.clone() },
                },
            );
        }
    }

    for remote in configured_registries()? {
        let registry = load_remote_registry(&remote)?;
        for (name, entry) in registry.skills {
            validate_entry(&name, &entry)?;
            merged.entry(name.clone()).or_insert(ResolvedSkill {
                name,
                entry,
                source: Source::Remote {
                    registry: remote.clone(),
                },
            });
        }
    }

    Ok(merged.into_values().collect())
}

fn validate_entry(name: &str, entry: &SkillEntry) -> Result<()> {
    if !ALLOWED_CATEGORIES.contains(&entry.category.as_str()) {
        bail!(
            "skill '{}' uses invalid category '{}'",
            name,
            entry.category
        );
    }
    if entry.description.trim().is_empty() {
        bail!("skill '{}' has an empty description", name);
    }
    if entry.path.trim().is_empty() {
        bail!("skill '{}' has an empty path", name);
    }
    Ok(())
}

fn configured_registries() -> Result<Vec<RemoteRegistry>> {
    let Some(path) = config_path() else {
        return Ok(vec![default_remote_registry()]);
    };
    if !path.exists() {
        return Ok(vec![default_remote_registry()]);
    }

    let config = read_yaml::<Config>(&path)?;
    let registries = config
        .registries
        .filter(|registries| !registries.is_empty())
        .unwrap_or_else(|| vec![default_remote_registry()]);
    Ok(registries)
}

fn default_remote_registry() -> RemoteRegistry {
    RemoteRegistry {
        owner: Some(DEFAULT_REGISTRY_OWNER.to_string()),
        repo: DEFAULT_REGISTRY_REPO.to_string(),
        reference: Some(DEFAULT_REGISTRY_REF.to_string()),
    }
}

fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("skillbox").join("config.yaml"))
}

fn find_project_registry() -> Result<Option<(PathBuf, RegistryFile)>> {
    let mut current = std::env::current_dir().context("failed to read current directory")?;
    loop {
        let candidate = current.join(".agents").join("skillbox.yaml");
        if candidate.exists() {
            let registry = read_yaml::<RegistryFile>(&candidate)?;
            return Ok(Some((current, registry)));
        }
        if !current.pop() {
            return Ok(None);
        }
    }
}

fn load_remote_registry(remote: &RemoteRegistry) -> Result<RegistryFile> {
    let url = remote_raw_url(remote, "registry.yaml");
    let body = fetch_text(&url)?;
    let registry = serde_yaml::from_str::<RegistryFile>(&body)
        .with_context(|| format!("failed to parse registry {}", url))?;
    if registry.version != 1 {
        bail!(
            "unsupported registry version {} from {}",
            registry.version,
            url
        );
    }
    Ok(registry)
}

fn read_skill_markdown(skill: &ResolvedSkill) -> Result<String> {
    match &skill.source {
        Source::Project { root } => {
            let path = root.join(&skill.entry.path).join("SKILL.md");
            fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))
        }
        Source::Remote { registry } => {
            let path = format!("{}/SKILL.md", skill.entry.path.trim_end_matches('/'));
            fetch_text(&remote_raw_url(registry, &path))
        }
    }
}

fn copy_skill_to_temp(skill: &ResolvedSkill) -> Result<PathBuf> {
    let destination = temp_root().join(format!(
        "{}-{}",
        sanitize_name(&skill.name),
        Local::now().format("%Y%m%d%H%M%S")
    ));

    match &skill.source {
        Source::Project { root } => {
            let source = root.join(&skill.entry.path);
            copy_dir(&source, &destination)?;
        }
        Source::Remote { registry } => {
            copy_remote_skill(registry, &skill.entry.path, &destination)?;
        }
    }

    Ok(destination)
}

fn copy_remote_skill(
    registry: &RemoteRegistry,
    skill_path: &str,
    destination: &Path,
) -> Result<()> {
    let archive = fetch_bytes(&archive_url(registry))?;
    let decoder = flate2::read::GzDecoder::new(&archive[..]);
    let mut archive = tar::Archive::new(decoder);
    let mut copied = false;
    let trimmed = skill_path.trim_matches('/');

    fs::create_dir_all(destination)
        .with_context(|| format!("failed to create {}", destination.display()))?;

    for entry in archive
        .entries()
        .context("failed to read registry archive")?
    {
        let mut entry = entry.context("failed to read archive entry")?;
        let path = entry.path().context("failed to read archive path")?;
        let mut components = path.components();
        components.next();
        let relative = components.as_path();
        if !relative.starts_with(trimmed) {
            continue;
        }

        let inside_skill = relative
            .strip_prefix(trimmed)
            .context("failed to strip skill path")?;
        if inside_skill.as_os_str().is_empty() {
            continue;
        }
        let output = destination.join(inside_skill);
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        entry
            .unpack(&output)
            .with_context(|| format!("failed to unpack {}", output.display()))?;
        copied = true;
    }

    if !copied {
        bail!(
            "skill folder '{}' was not found in remote archive",
            skill_path
        );
    }
    Ok(())
}

fn copy_dir(source: &Path, destination: &Path) -> Result<()> {
    if !source.exists() {
        bail!("skill folder does not exist: {}", source.display());
    }
    fs::create_dir_all(destination)
        .with_context(|| format!("failed to create {}", destination.display()))?;
    for entry in
        fs::read_dir(source).with_context(|| format!("failed to read {}", source.display()))?
    {
        let entry = entry.with_context(|| format!("failed to read {}", source.display()))?;
        copy_path(&entry.path(), &destination.join(entry.file_name()))?;
    }
    Ok(())
}

fn copy_path(source: &Path, destination: &Path) -> Result<()> {
    if source.is_dir() {
        fs::create_dir_all(destination)
            .with_context(|| format!("failed to create {}", destination.display()))?;
        for entry in
            fs::read_dir(source).with_context(|| format!("failed to read {}", source.display()))?
        {
            let entry = entry.with_context(|| format!("failed to read {}", source.display()))?;
            copy_path(&entry.path(), &destination.join(entry.file_name()))?;
        }
    } else {
        fs::copy(source, destination).with_context(|| {
            format!(
                "failed to copy {} to {}",
                source.display(),
                destination.display()
            )
        })?;
    }
    Ok(())
}

fn read_yaml<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let text =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_yaml::from_str(&text).with_context(|| format!("failed to parse {}", path.display()))
}

fn fetch_text(url: &str) -> Result<String> {
    let response = reqwest::blocking::get(url).with_context(|| format!("failed to GET {}", url))?;
    if !response.status().is_success() {
        bail!("GET {} returned {}", url, response.status());
    }
    response
        .text()
        .with_context(|| format!("failed to read response from {}", url))
}

fn fetch_bytes(url: &str) -> Result<Vec<u8>> {
    let mut response =
        reqwest::blocking::get(url).with_context(|| format!("failed to GET {}", url))?;
    if !response.status().is_success() {
        bail!("GET {} returned {}", url, response.status());
    }
    let mut bytes = Vec::new();
    response
        .read_to_end(&mut bytes)
        .with_context(|| format!("failed to read response from {}", url))?;
    Ok(bytes)
}

fn remote_raw_url(registry: &RemoteRegistry, path: &str) -> String {
    let owner = registry.owner.as_deref().unwrap_or(DEFAULT_REGISTRY_OWNER);
    let reference = registry
        .reference
        .as_deref()
        .unwrap_or(DEFAULT_REGISTRY_REF);
    format!(
        "https://raw.githubusercontent.com/{}/{}/{}/{}",
        owner,
        registry.repo,
        reference,
        path.trim_start_matches('/')
    )
}

fn archive_url(registry: &RemoteRegistry) -> String {
    let owner = registry.owner.as_deref().unwrap_or(DEFAULT_REGISTRY_OWNER);
    let reference = registry
        .reference
        .as_deref()
        .unwrap_or(DEFAULT_REGISTRY_REF);
    format!(
        "https://github.com/{}/{}/archive/refs/heads/{}.tar.gz",
        owner, registry.repo, reference
    )
}

fn temp_root() -> PathBuf {
    std::env::temp_dir().join("skillbox")
}

fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_dir_places_skill_contents_at_destination_root() {
        let source_root = tempfile::tempdir().expect("source tempdir");
        let destination_root = tempfile::tempdir().expect("destination tempdir");
        let source = source_root.path().join("example-skill");
        fs::create_dir_all(source.join("references")).expect("references dir");
        fs::write(source.join("SKILL.md"), "# Example").expect("skill file");
        fs::write(source.join("references").join("notes.md"), "notes").expect("reference file");

        copy_dir(&source, destination_root.path()).expect("copy skill");

        assert!(destination_root.path().join("SKILL.md").exists());
        assert!(
            destination_root
                .path()
                .join("references")
                .join("notes.md")
                .exists()
        );
        assert!(!destination_root.path().join("example-skill").exists());
    }

    #[test]
    fn category_parser_rejects_unknown_categories() {
        assert_eq!(
            parse_category("frontend").expect("known category"),
            "frontend"
        );
        assert!(parse_category("sales").is_err());
    }

    #[test]
    fn list_line_omits_category_when_category_filter_is_active() {
        let skill = ResolvedSkill {
            name: "frontend".to_string(),
            entry: SkillEntry {
                category: "frontend".to_string(),
                description: "Build and polish frontend UI".to_string(),
                path: "skills/frontend".to_string(),
            },
            source: Source::Remote {
                registry: default_remote_registry(),
            },
        };

        assert_eq!(
            format_list_line(&skill, true),
            "frontend: Build and polish frontend UI"
        );
        assert_eq!(
            format_list_line(&skill, false),
            "frontend [frontend]: Build and polish frontend UI"
        );
    }
}
