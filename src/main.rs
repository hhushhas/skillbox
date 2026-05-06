use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::Local;
use clap::{Args, Parser, Subcommand};
use serde::Deserialize;

const DEFAULT_REGISTRY_OWNER: &str = "hhushhas";
const DEFAULT_REGISTRY_REPO: &str = "skillbox-registry";
const DEFAULT_REGISTRY_REF: &str = "main";
const MAX_SEARCH_RESULTS: usize = 8;
const APPROX_CHARS_PER_TOKEN: usize = 4;
const GUIDE_TOPICS: [&str; 5] = [
    "agent",
    "onboarding",
    "registry",
    "add-skill",
    "update-skill",
];
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
    /// Search skills with a natural-language query.
    Search(ListArgs),
    /// Fetch a skill as markdown or a temporary folder.
    Fetch(FetchArgs),
    /// Show where a skill lives and how to update it.
    Info(InfoArgs),
    /// Print short setup and maintenance instructions.
    Guide(GuideArgs),
    /// Remove Skillbox-created temp folders.
    Cleanup,
    /// Check Skillbox configuration, registries, and paths.
    Doctor,
}

#[derive(Args)]
struct ListArgs {
    /// Optional natural-language search query.
    query: Vec<String>,
    /// Limit output to a category.
    #[arg(long, value_parser = parse_category)]
    category: Option<String>,
    /// Print only skill names.
    #[arg(long)]
    names: bool,
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

#[derive(Args)]
struct InfoArgs {
    /// Skill name from the registry.
    name: String,
}

#[derive(Args)]
struct GuideArgs {
    /// Guide topic.
    #[arg(value_parser = parse_guide_topic)]
    topic: Option<String>,
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

#[derive(Debug, Deserialize)]
struct GitHubContentResponse {
    content: String,
    encoding: String,
}

#[derive(Debug, Clone, Deserialize)]
struct SkillEntry {
    category: String,
    description: String,
    path: String,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    resources: Vec<String>,
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
    match cli.command.unwrap_or(Command::List(ListArgs {
        query: Vec::new(),
        category: None,
        names: false,
    })) {
        Command::List(args) => list(args),
        Command::Search(args) => list(args),
        Command::Fetch(args) => fetch(args),
        Command::Info(args) => info(args),
        Command::Guide(args) => guide(args),
        Command::Cleanup => cleanup(),
        Command::Doctor => doctor(),
    }
}

fn list(args: ListArgs) -> Result<()> {
    let skills = load_skills()?;
    let filtered_by_category = args.category.is_some();

    let mut matches = Vec::new();
    let query = args.query.join(" ");
    let query_tokens = query_tokens(&query);

    for skill in skills
        .into_iter()
        .filter(|skill| category_matches(skill, args.category.as_deref()))
    {
        if query_tokens.is_empty() {
            matches.push((0, skill));
            continue;
        }

        let score = search_score(&skill, &query, &query_tokens);
        if score > 0 {
            matches.push((score, skill));
        }
    }

    if !query_tokens.is_empty() {
        matches.sort_by(|(left_score, left_skill), (right_score, right_skill)| {
            right_score
                .cmp(left_score)
                .then_with(|| left_skill.name.cmp(&right_skill.name))
        });
        matches.truncate(MAX_SEARCH_RESULTS);
    }

    if args.names {
        println!(
            "{}",
            matches
                .iter()
                .map(|(_, skill)| skill.name.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        );
    } else {
        for (_, skill) in matches {
            println!("{}", format_list_line(&skill, filtered_by_category));
        }
    }
    Ok(())
}

fn category_matches(skill: &ResolvedSkill, category: Option<&str>) -> bool {
    category.is_none_or(|category| category == skill.entry.category)
}

fn format_list_line(skill: &ResolvedSkill, filtered_by_category: bool) -> String {
    let resources = format_resource_hint(skill);
    if filtered_by_category {
        format!("{}{}: {}", skill.name, resources, skill.entry.description)
    } else {
        format!(
            "{}{} [{}]: {}",
            skill.name, resources, skill.entry.category, skill.entry.description
        )
    }
}

fn format_resource_hint(skill: &ResolvedSkill) -> String {
    if skill.entry.resources.is_empty() {
        String::new()
    } else {
        format!("[{}]", skill.entry.resources.join(","))
    }
}

fn search_score(skill: &ResolvedSkill, raw_query: &str, query_tokens: &[String]) -> u16 {
    let haystack = format!(
        "{} {} {}",
        skill.name.to_lowercase(),
        skill.entry.category.to_lowercase(),
        skill.entry.description.to_lowercase()
    );
    let haystack = format!(
        "{} {}",
        haystack,
        skill.entry.aliases.join(" ").to_lowercase()
    );
    let haystack_tokens = query_tokens_from_text(&haystack);
    let normalized_query = raw_query.to_lowercase();
    let mut score = 0;

    if !normalized_query.trim().is_empty() && haystack.contains(normalized_query.trim()) {
        score += 12;
    }

    for token in query_tokens {
        if token == "skills" || token == "skill" || token == "for" {
            continue;
        }
        let name = skill.name.to_lowercase();
        let name_tokens = query_tokens_from_text(&name);

        if name == *token {
            score += 18;
        } else if name_tokens.iter().any(|candidate| candidate == token) {
            score += 12;
        } else if token.len() >= 3 && name.contains(token) {
            score += 10;
        }
        if skill.entry.category == *token {
            score += 8;
        }
        if skill.entry.aliases.iter().any(|alias| {
            query_tokens_from_text(alias)
                .iter()
                .any(|candidate| candidate == token)
        }) {
            score += 10;
        }
        if haystack_tokens.iter().any(|candidate| candidate == token) {
            score += 4;
        } else if token.len() >= 3 && haystack.contains(token) {
            score += 2;
        }
    }

    score
}

fn query_tokens(query: &str) -> Vec<String> {
    let mut tokens = query_tokens_from_text(query);
    let mut expanded = Vec::new();
    for token in &tokens {
        expanded.extend(token_synonyms(token).into_iter().map(str::to_string));
    }
    tokens.extend(expanded);
    tokens.sort();
    tokens.dedup();
    tokens
}

fn query_tokens_from_text(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            current.push(ch.to_ascii_lowercase());
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn token_synonyms(token: &str) -> Vec<&'static str> {
    match token {
        "chatbot" | "chatbots" => vec!["chat", "ai", "agent"],
        "guideline" | "guidelines" => vec!["practice", "practices", "rules"],
        "transcription" | "transcripts" => vec!["transcript"],
        "ui" => vec!["frontend", "design"],
        "ux" => vec!["design"],
        _ => Vec::new(),
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
        if !skill.entry.resources.is_empty() {
            eprintln!(
                "note: {} has {}; use `skillbox fetch {} --to-temp` for the full skill folder.",
                skill.name,
                skill.entry.resources.join(","),
                skill.name
            );
        }
        let markdown = read_skill_markdown(&skill)?;
        eprintln!(
            "tokens: ~{} ({} chars)",
            approximate_tokens(&markdown),
            markdown.chars().count()
        );
        print!("{markdown}");
        return Ok(());
    }

    let destination = copy_skill_to_temp(&skill)?;
    println!("{}", destination.display());
    Ok(())
}

fn info(args: InfoArgs) -> Result<()> {
    let skill = load_skills()?
        .into_iter()
        .find(|skill| skill.name == args.name)
        .ok_or_else(|| anyhow!("unknown skill '{}'", args.name))?;

    println!("name: {}", skill.name);
    println!("category: {}", skill.entry.category);
    println!("description: {}", skill.entry.description);
    if !skill.entry.aliases.is_empty() {
        println!("aliases: {}", skill.entry.aliases.join(","));
    }
    if !skill.entry.resources.is_empty() {
        println!("resources: {}", skill.entry.resources.join(","));
    }
    let markdown = read_skill_markdown(&skill)?;
    println!(
        "tokens: ~{} ({} chars, SKILL.md)",
        approximate_tokens(&markdown),
        markdown.chars().count()
    );

    match &skill.source {
        Source::Project { root } => {
            println!("source: project");
            println!("registry: {}", root.join(".agents/skillbox.yaml").display());
            println!("skill: {}", root.join(&skill.entry.path).display());
            println!(
                "update: edit the project registry and skill folder, then run doctor/search/fetch"
            );
        }
        Source::Remote { registry } => {
            println!("source: remote");
            println!("registry: {}", remote_registry_label(registry));
            println!("skill: {}", remote_tree_url(registry, &skill.entry.path));
            println!(
                "update: edit registry.yaml and the skill folder in the source repo, then run doctor/search/fetch"
            );
        }
    }

    Ok(())
}

fn guide(args: GuideArgs) -> Result<()> {
    match args.topic.as_deref() {
        None => {
            println!("topics: {}", GUIDE_TOPICS.join(" "));
            println!("usage: skillbox guide <topic>");
            println!("start: skillbox guide onboarding");
        }
        Some("agent") => {
            println!("1. Find: skillbox search \"<task>\" or skillbox list --category <category>");
            println!("2. Inspect: skillbox info <skill>");
            println!("3. Load: skillbox fetch <skill> --print");
            println!("4. Full folder: use --to-temp when resources are listed");
        }
        Some("onboarding") => {
            println!("1. Run: skillbox doctor");
            println!("2. Explore: skillbox list; skillbox search \"<task>\"");
            println!("3. Migrate useful global skills from ~/.agents/skills or ~/.codex/skills");
            println!(
                "4. Add shared skills to a registry; add project skills to .agents/skillbox.yaml"
            );
            println!("5. Load only when useful: skillbox fetch <skill> --print or --to-temp");
        }
        Some("registry") => {
            println!("project registry: .agents/skillbox.yaml");
            println!("project skills: .agents/skills.available/<skill>/SKILL.md");
            println!("remote config: ~/.config/skillbox/config.yaml");
            println!("default remote: hhushhas/skillbox-registry/main");
        }
        Some("add-skill") => {
            println!("1. Add skills/<skill>/SKILL.md or .agents/skills.available/<skill>/SKILL.md");
            println!(
                "2. Add registry entry: category, description, path, aliases, optional resources"
            );
            println!("3. Keep description one line; say what it does and when to use it");
            println!(
                "4. Verify: skillbox doctor; skillbox search \"<query>\"; skillbox fetch <skill> --print"
            );
        }
        Some("update-skill") => {
            println!("1. Locate: skillbox info <skill>");
            println!("2. Edit the shown registry entry and skill folder");
            println!("3. Keep SKILL.md lean; move support material into resources");
            println!("4. Verify token change with skillbox info <skill> and fetch output");
        }
        Some(topic) => bail!(
            "unknown guide topic '{}'; expected one of: {}",
            topic,
            GUIDE_TOPICS.join(", ")
        ),
    }
    Ok(())
}

fn approximate_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(APPROX_CHARS_PER_TOKEN)
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

fn doctor() -> Result<()> {
    println!("skillbox: {}", env!("CARGO_PKG_VERSION"));
    println!("binary: {}", current_exe_display());
    if let Some(path) = config_path() {
        println!(
            "config: {}{}",
            path.display(),
            if path.exists() {
                ""
            } else {
                " (missing, using default)"
            }
        );
    }
    println!("temp: {}", temp_root().display());

    match find_project_registry()? {
        Some((root, registry)) => {
            println!(
                "project: {} ({} skills)",
                root.display(),
                registry.skills.len()
            );
        }
        None => println!("project: none"),
    }

    for remote in configured_registries()? {
        let label = remote_registry_label(&remote);
        match load_remote_registry(&remote) {
            Ok(registry) => println!("remote: {} ok ({} skills)", label, registry.skills.len()),
            Err(error) => println!("remote: {} error ({:#})", label, error),
        }
    }

    println!("skills: {}", load_skills()?.len());
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

fn parse_guide_topic(value: &str) -> std::result::Result<String, String> {
    if GUIDE_TOPICS.contains(&value) {
        Ok(value.to_string())
    } else {
        Err(format!(
            "unknown topic '{}'; expected one of: {}",
            value,
            GUIDE_TOPICS.join(", ")
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
    for alias in &entry.aliases {
        if alias.trim().is_empty() {
            bail!("skill '{}' has an empty alias", name);
        }
    }
    for resource in &entry.resources {
        if resource.trim().is_empty() {
            bail!("skill '{}' has an empty resource marker", name);
        }
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
    let body = fetch_github_content(remote, "registry.yaml").or_else(|_| {
        let url = remote_raw_url(remote, "registry.yaml");
        fetch_text(&url)
    })?;
    let registry = serde_yaml::from_str::<RegistryFile>(&body)
        .with_context(|| format!("failed to parse registry {}", remote_registry_label(remote)))?;
    if registry.version != 1 {
        bail!(
            "unsupported registry version {} from {}",
            registry.version,
            remote_registry_label(remote)
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
            fetch_github_content(registry, &path)
                .or_else(|_| fetch_text(&remote_raw_url(registry, &path)))
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
    let response = http_client()
        .get(url)
        .send()
        .with_context(|| format!("failed to GET {}", url))?;
    if !response.status().is_success() {
        bail!("GET {} returned {}", url, response.status());
    }
    response
        .text()
        .with_context(|| format!("failed to read response from {}", url))
}

fn fetch_bytes(url: &str) -> Result<Vec<u8>> {
    let mut response = http_client()
        .get(url)
        .send()
        .with_context(|| format!("failed to GET {}", url))?;
    if !response.status().is_success() {
        bail!("GET {} returned {}", url, response.status());
    }
    let mut bytes = Vec::new();
    response
        .read_to_end(&mut bytes)
        .with_context(|| format!("failed to read response from {}", url))?;
    Ok(bytes)
}

fn fetch_github_content(registry: &RemoteRegistry, path: &str) -> Result<String> {
    let url = github_contents_url(registry, path);
    let response = http_client()
        .get(&url)
        .send()
        .with_context(|| format!("failed to GET {}", url))?;
    if !response.status().is_success() {
        bail!("GET {} returned {}", url, response.status());
    }
    let body = response
        .text()
        .with_context(|| format!("failed to read response from {}", url))?;
    let content = serde_json::from_str::<GitHubContentResponse>(&body)
        .with_context(|| format!("failed to parse GitHub content response from {}", url))?;
    if content.encoding != "base64" {
        bail!("unsupported GitHub content encoding '{}'", content.encoding);
    }
    let compact = content.content.replace(['\n', '\r'], "");
    let decoded = BASE64
        .decode(compact)
        .with_context(|| format!("failed to decode GitHub content from {}", url))?;
    String::from_utf8(decoded).with_context(|| format!("GitHub content is not UTF-8: {}", url))
}

fn http_client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .user_agent("skillbox")
        .build()
        .expect("build HTTP client")
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

fn remote_tree_url(registry: &RemoteRegistry, path: &str) -> String {
    let owner = registry.owner.as_deref().unwrap_or(DEFAULT_REGISTRY_OWNER);
    let reference = registry
        .reference
        .as_deref()
        .unwrap_or(DEFAULT_REGISTRY_REF);
    format!(
        "https://github.com/{}/{}/tree/{}/{}",
        owner,
        registry.repo,
        reference,
        path.trim_start_matches('/')
    )
}

fn github_contents_url(registry: &RemoteRegistry, path: &str) -> String {
    let owner = registry.owner.as_deref().unwrap_or(DEFAULT_REGISTRY_OWNER);
    let reference = registry
        .reference
        .as_deref()
        .unwrap_or(DEFAULT_REGISTRY_REF);
    format!(
        "https://api.github.com/repos/{}/{}/contents/{}?ref={}",
        owner,
        registry.repo,
        path.trim_start_matches('/'),
        reference
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

fn remote_registry_label(registry: &RemoteRegistry) -> String {
    let owner = registry.owner.as_deref().unwrap_or(DEFAULT_REGISTRY_OWNER);
    let reference = registry
        .reference
        .as_deref()
        .unwrap_or(DEFAULT_REGISTRY_REF);
    format!("{}/{}/{}", owner, registry.repo, reference)
}

fn current_exe_display() -> String {
    std::env::current_exe()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| "unknown".to_string())
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
    fn guide_topic_parser_rejects_unknown_topics() {
        assert_eq!(
            parse_guide_topic("add-skill").expect("known topic"),
            "add-skill"
        );
        assert!(parse_guide_topic("publish").is_err());
    }

    #[test]
    fn list_line_omits_category_when_category_filter_is_active() {
        let skill = ResolvedSkill {
            name: "frontend".to_string(),
            entry: SkillEntry {
                category: "frontend".to_string(),
                description: "Build and polish frontend UI".to_string(),
                path: "skills/frontend".to_string(),
                aliases: Vec::new(),
                resources: Vec::new(),
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

    #[test]
    fn list_line_mentions_resource_markers() {
        let skill = ResolvedSkill {
            name: "agent-browser".to_string(),
            entry: SkillEntry {
                category: "browser".to_string(),
                description: "Run browser automation".to_string(),
                path: "skills/agent-browser".to_string(),
                aliases: Vec::new(),
                resources: vec!["refs".to_string(), "scripts".to_string()],
            },
            source: Source::Remote {
                registry: default_remote_registry(),
            },
        };

        assert_eq!(
            format_list_line(&skill, true),
            "agent-browser[refs,scripts]: Run browser automation"
        );
    }

    #[test]
    fn natural_language_search_scores_synonyms() {
        let skill = ResolvedSkill {
            name: "vercel-react-best-practices".to_string(),
            entry: SkillEntry {
                category: "frontend".to_string(),
                description: "Optimize React and Next.js; use for rendering and performance."
                    .to_string(),
                path: "skills/vercel-react-best-practices".to_string(),
                aliases: vec!["guidelines".to_string()],
                resources: vec!["rules".to_string()],
            },
            source: Source::Remote {
                registry: default_remote_registry(),
            },
        };

        let tokens = query_tokens("react guidelines");
        assert!(tokens.contains(&"react".to_string()));
        assert!(tokens.contains(&"practices".to_string()));
        assert!(search_score(&skill, "react guidelines", &tokens) > 0);
    }

    #[test]
    fn natural_language_search_scores_registry_aliases() {
        let skill = ResolvedSkill {
            name: "react".to_string(),
            entry: SkillEntry {
                category: "frontend".to_string(),
                description: "Work with React".to_string(),
                path: "skills/react".to_string(),
                aliases: vec!["nextjs".to_string(), "hooks".to_string()],
                resources: Vec::new(),
            },
            source: Source::Remote {
                registry: default_remote_registry(),
            },
        };

        let tokens = query_tokens("nextjs hooks");
        assert!(search_score(&skill, "nextjs hooks", &tokens) > 0);
    }

    #[test]
    fn short_tokens_do_not_match_inside_unrelated_words() {
        let skill = ResolvedSkill {
            name: "client-comms-studio".to_string(),
            entry: SkillEntry {
                category: "project".to_string(),
                description: "Draft client communications".to_string(),
                path: "skills/client-comms-studio".to_string(),
                aliases: Vec::new(),
                resources: Vec::new(),
            },
            source: Source::Remote {
                registry: default_remote_registry(),
            },
        };

        let tokens = query_tokens("ai skills");
        assert_eq!(search_score(&skill, "ai skills", &tokens), 0);
    }

    #[test]
    fn query_tokenizer_normalizes_punctuation() {
        assert_eq!(
            query_tokens_from_text("design for chatbot"),
            vec!["design", "for", "chatbot"]
        );
        assert!(query_tokens("chatbot").contains(&"chat".to_string()));
    }

    #[test]
    fn approximate_tokens_rounds_up_by_four_chars() {
        assert_eq!(approximate_tokens(""), 0);
        assert_eq!(approximate_tokens("abcd"), 1);
        assert_eq!(approximate_tokens("abcde"), 2);
    }
}
