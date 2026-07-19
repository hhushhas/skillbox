use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::Local;
use clap::{Args, Parser, Subcommand};
use serde::{Deserialize, Serialize};

const DEFAULT_REGISTRY_OWNER: &str = "hhushhas";
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
    /// Search registries, or search skills.sh with --web.
    Search(SearchArgs),
    /// Print a skill and prepare any support files in temp.
    Fetch(FetchArgs),
    /// Show where a skill lives and how to update it.
    Info(InfoArgs),
    /// Add an external skills.sh skill to the local installed registry.
    Add(AddArgs),
    /// Promote an external skill into the trusted global registry.
    Promote(PromoteArgs),
    /// Remove a skill from the local installed registry.
    Remove(RemoveArgs),
    /// Print short setup and maintenance instructions.
    Guide(GuideArgs),
    /// Remove Skillbox-created temp folders.
    Cleanup,
    /// Check Skillbox configuration, registries, and paths.
    Doctor,
}

#[derive(Args)]
struct ListArgs {
    /// Limit output to a category.
    #[arg(long, value_parser = parse_category)]
    category: Option<String>,
    /// Print only skill names.
    #[arg(long)]
    names: bool,
}

#[derive(Args)]
struct SearchArgs {
    /// Natural-language search query.
    #[arg(required = true)]
    query: Vec<String>,
    /// Limit registry results to a category.
    #[arg(long, value_parser = parse_category)]
    category: Option<String>,
    /// Print only skill names.
    #[arg(long)]
    names: bool,
    /// Search the public skills.sh directory instead of trusted registries.
    #[arg(long)]
    web: bool,
}

#[derive(Args)]
struct FetchArgs {
    /// Skill name from the registry.
    name: String,
}

#[derive(Args)]
struct InfoArgs {
    /// Skill name from the registry.
    name: String,
}

#[derive(Args)]
struct AddArgs {
    /// External skill reference: owner/repo/skillId.
    reference: String,
}

#[derive(Args)]
struct PromoteArgs {
    /// External skill reference owner/repo/skillId, or installed external skill id.
    reference: String,
    /// Trusted registry category.
    #[arg(long, value_parser = parse_promote_category)]
    category: String,
}

#[derive(Args)]
struct RemoveArgs {
    /// Installed skill id to remove.
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

#[derive(Debug, Deserialize, Serialize)]
struct RegistryFile {
    version: u8,
    skills: BTreeMap<String, SkillEntry>,
}

#[derive(Debug, Deserialize)]
struct GitHubContentResponse {
    content: String,
    encoding: String,
}

#[derive(Debug, Deserialize)]
struct WebSearchResponse {
    skills: Vec<WebSearchSkill>,
}

#[derive(Debug, Deserialize)]
struct WebSearchSkill {
    id: String,
    installs: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct SkillEntry {
    category: String,
    description: String,
    path: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    resources: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct InstalledSkillEntry {
    source: String,
    path: String,
    description: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct InstalledFile {
    version: u8,
    #[serde(default)]
    skills: BTreeMap<String, InstalledSkillEntry>,
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
    Global { root: PathBuf },
    Remote { registry: RemoteRegistry },
    Installed { source: String, path: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExternalRef {
    owner: String,
    repo: String,
    skill_id: String,
}

#[derive(Debug)]
struct ExternalSkill {
    reference: ExternalRef,
    branch: String,
    path: String,
    markdown: String,
    archive: Vec<u8>,
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
        category: None,
        names: false,
    })) {
        Command::List(args) => list(args),
        Command::Search(args) => search(args),
        Command::Fetch(args) => fetch(args),
        Command::Info(args) => info(args),
        Command::Add(args) => add(args),
        Command::Promote(args) => promote(args),
        Command::Remove(args) => remove(args),
        Command::Guide(args) => guide(args),
        Command::Cleanup => cleanup(),
        Command::Doctor => doctor(),
    }
}

fn list(args: ListArgs) -> Result<()> {
    let skills = load_skills()?;
    let no_skills_loaded = skills.is_empty();
    let filtered_by_category = args.category.is_some();

    let matches = skills
        .into_iter()
        .filter(|skill| category_matches(skill, args.category.as_deref()))
        .collect::<Vec<_>>();

    if args.names {
        println!(
            "{}",
            matches
                .iter()
                .map(|skill| skill.name.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        );
    } else {
        for skill in matches {
            println!("{}", format_list_line(&skill, filtered_by_category));
        }
    }
    if no_skills_loaded {
        print_empty_registry_hint();
    }
    Ok(())
}

fn search(args: SearchArgs) -> Result<()> {
    if args.web {
        return search_web(args);
    }

    let skills = load_skills()?;
    let no_skills_loaded = skills.is_empty();
    let filtered_by_category = args.category.is_some();
    let query = args.query.join(" ");
    let query_tokens = query_tokens(&query);
    let mut matches = skills
        .into_iter()
        .filter(|skill| category_matches(skill, args.category.as_deref()))
        .filter_map(|skill| {
            let score = search_score(&skill, &query, &query_tokens);
            (score > 0).then_some((score, skill))
        })
        .collect::<Vec<_>>();

    matches.sort_by(|(left_score, left_skill), (right_score, right_skill)| {
        right_score
            .cmp(left_score)
            .then_with(|| left_skill.name.cmp(&right_skill.name))
    });
    matches.truncate(MAX_SEARCH_RESULTS);

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
    if no_skills_loaded {
        print_empty_registry_hint();
    }
    Ok(())
}

fn search_web(args: SearchArgs) -> Result<()> {
    let query = args.query.join(" ");
    let url = format!("https://skills.sh/api/search?q={}", percent_encode(&query));
    let body = fetch_text(&url)?;
    let response = serde_json::from_str::<WebSearchResponse>(&body)
        .with_context(|| format!("failed to parse skills.sh response from {}", url))?;

    eprintln!(
        "note: unvetted third-party content from skills.sh; load with `skillbox fetch <owner/repo/skill>`."
    );

    let skills = response
        .skills
        .into_iter()
        .take(MAX_SEARCH_RESULTS)
        .collect::<Vec<_>>();

    if args.names {
        println!(
            "{}",
            skills
                .iter()
                .map(|skill| skill.id.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        );
        return Ok(());
    }

    for skill in skills {
        println!("{} [unverified, {} installs]", skill.id, skill.installs);
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
    if let Some(reference) = parse_external_ref(&args.name)? {
        return fetch_external(reference);
    }

    let skill = load_skills()?
        .into_iter()
        .find(|skill| skill.name == args.name)
        .ok_or_else(|| anyhow!("unknown skill '{}'", args.name))?;

    if let Source::Installed { source, path } = &skill.source {
        let external = resolve_installed_skill(&skill.name, source, path)?;
        warn_unverified_if_external(&skill);
        return fetch_archived_skill(external);
    }

    warn_unverified_if_external(&skill);
    let markdown = read_skill_markdown(&skill)?;
    if let Some((destination, resources)) = prepare_support_files(&skill)? {
        print_support_suggestion(&destination, &resources)?;
    }
    print_skill_markdown(&markdown);
    Ok(())
}

fn fetch_external(reference: ExternalRef) -> Result<()> {
    let skill = resolve_external_skill(&reference)?;
    eprintln!("warning: external skill from skills.sh — unverified third-party content");
    fetch_archived_skill(skill)
}

fn fetch_archived_skill(skill: ExternalSkill) -> Result<()> {
    let destination = create_skill_temp_dir(&skill.reference.skill_id)?;
    copy_external_skill_to_temp(&skill, destination.path())?;
    let resources = resource_entries(destination.path())?;
    if !resources.is_empty() {
        let destination = destination.keep();
        print_support_suggestion(&destination, &resources)?;
    }
    print_skill_markdown(&skill.markdown);
    Ok(())
}

fn prepare_support_files(skill: &ResolvedSkill) -> Result<Option<(PathBuf, Vec<String>)>> {
    if skill.entry.resources.is_empty() {
        return Ok(None);
    }

    let destination = copy_skill_to_temp(skill)?;
    let resources = skill.entry.resources.clone();
    Ok(Some((destination, resources)))
}

fn print_support_suggestion(destination: &Path, resources: &[String]) -> Result<()> {
    let summary = format_resource_summary(&resource_file_counts(destination, resources)?);
    eprintln!(
        "suggestion: this skill includes {summary}; read the support files from {}.",
        destination.display()
    );
    Ok(())
}

fn format_resource_summary(resources: &[(String, usize)]) -> String {
    resources
        .iter()
        .map(|(name, count)| format!("{count} {} in {name}", pluralize("file", *count)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn print_skill_markdown(markdown: &str) {
    eprintln!(
        "tokens: ~{} ({} chars)",
        approximate_tokens(markdown),
        markdown.chars().count()
    );
    print!("{markdown}");
}

fn info(args: InfoArgs) -> Result<()> {
    if let Some(reference) = parse_external_ref(&args.name)? {
        return info_external(reference);
    }

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
        Source::Global { root } => {
            println!("source: global");
            println!("registry: {}", root.join("skillbox.yaml").display());
            println!("skill: {}", root.join(&skill.entry.path).display());
            println!(
                "update: edit the global registry and skill folder, then run doctor/search/fetch"
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
        Source::Installed { source, path } => {
            println!("source: installed external");
            println!("registry: {}", installed_path_display());
            println!("skill: {}", installed_tree_url(source, path));
            println!("note: unverified third-party content");
            println!(
                "update: run skillbox add {}/{} to refresh this installed entry",
                source, skill.name
            );
        }
    }

    Ok(())
}

fn info_external(reference: ExternalRef) -> Result<()> {
    let skill = resolve_external_skill(&reference)?;
    println!("name: {}", skill.reference.skill_id);
    println!("category: external");
    println!(
        "description: {}",
        description_from_markdown(&skill.markdown)
    );
    println!(
        "tokens: ~{} ({} chars, SKILL.md)",
        approximate_tokens(&skill.markdown),
        skill.markdown.chars().count()
    );
    println!("source: external");
    println!("skill: {}", external_tree_url(&skill));
    println!("note: unverified third-party content");
    Ok(())
}

fn add(args: AddArgs) -> Result<()> {
    let reference = parse_external_ref(&args.reference)?
        .ok_or_else(|| anyhow!("expected external skill reference owner/repo/skillId"))?;
    let skill = resolve_external_skill(&reference)?;
    let path = installed_path().ok_or_else(|| anyhow!("failed to locate config directory"))?;
    let mut installed = read_installed_file_at(&path)?;
    let existed = installed.skills.contains_key(&skill.reference.skill_id);

    installed.skills.insert(
        skill.reference.skill_id.clone(),
        InstalledSkillEntry {
            source: format!("{}/{}", skill.reference.owner, skill.reference.repo),
            path: skill.path.clone(),
            description: description_from_markdown(&skill.markdown),
        },
    );
    write_installed_file_at(&path, &installed)?;

    if existed {
        println!("updated {} in {}", skill.reference.skill_id, path.display());
    } else {
        println!("added {} to {}", skill.reference.skill_id, path.display());
    }
    eprintln!("warning: installed skill is unverified third-party content");
    Ok(())
}

fn promote(args: PromoteArgs) -> Result<()> {
    let root = skillbox_config_dir().ok_or_else(|| anyhow!("failed to locate config directory"))?;
    let registry_path = root.join("skillbox.yaml");
    let skill = resolve_promoted_skill(&args.reference)?;
    let name = skill.reference.skill_id.clone();
    let destination = root.join("skills").join(&name);
    let mut registry = read_registry_file_at(&registry_path)?;

    remove_existing_path(&destination)?;
    copy_external_skill_to_temp(&skill, &destination)?;

    let entry = promoted_skill_entry(
        &args.category,
        &skill.markdown,
        &name,
        resource_entries(&destination)?,
    );
    validate_entry(&name, &entry)?;
    registry.skills.insert(name.clone(), entry);
    write_registry_file_at(&registry_path, &registry)?;
    remove_installed_skill_if_present(&skill)?;

    println!("promoted {} to {}", name, destination.display());
    println!(
        "hint: edit aliases in {} to improve search",
        registry_path.display()
    );
    Ok(())
}

fn remove(args: RemoveArgs) -> Result<()> {
    let path = installed_path().ok_or_else(|| anyhow!("failed to locate config directory"))?;
    let mut installed = read_installed_file_at(&path)?;

    if installed.skills.remove(&args.name).is_none() {
        bail!("installed skill '{}' was not found", args.name);
    }

    write_installed_file_at(&path, &installed)?;
    println!("removed {} from {}", args.name, path.display());
    Ok(())
}

fn guide(args: GuideArgs) -> Result<()> {
    match args.topic.as_deref() {
        None => {
            print_agent_guide();
            println!("topics: {}", GUIDE_TOPICS.join(" "));
            println!("usage: skillbox guide <topic>");
        }
        Some("agent") => print_agent_guide(),
        Some("onboarding") => {
            println!("1. Run: skillbox doctor");
            println!(
                "2. Add personal skills to ~/.skillbox/skillbox.yaml with folders under ~/.skillbox/skills/"
            );
            println!("3. Explore: skillbox list; skillbox search \"<task>\"");
            println!(
                "4. Layer project skills with .agents/skillbox.yaml; add remote registries only via ~/.skillbox/config.yaml"
            );
            println!(
                "5. Load only when useful: skillbox fetch <skill>; support files are prepared in temp automatically"
            );
            println!(
                "6. Search skills.sh explicitly with skillbox search \"<task>\" --web; external skills are unverified"
            );
        }
        Some("registry") => {
            println!("global registry: ~/.skillbox/skillbox.yaml");
            println!("global skills: ~/.skillbox/skills/<skill>/SKILL.md");
            println!("project registry: .agents/skillbox.yaml");
            println!("project skills: .agents/skills.available/<skill>/SKILL.md");
            println!("remote config: ~/.skillbox/config.yaml (opt-in)");
            println!("remote example: registries: [{{owner: hhushhas, repo: skillbox-registry}}]");
            println!(
                "external installs: skillbox add owner/repo/skillId records ~/.skillbox/installed.yaml; promote with skillbox promote <skillId> --category <category>; remove with skillbox remove <skillId>"
            );
        }
        Some("add-skill") => {
            println!("1. For reusable personal skills, edit ~/.skillbox/skillbox.yaml");
            println!(
                "2. Add ~/.skillbox/skills/<skill>/SKILL.md or project .agents/skills.available/<skill>/SKILL.md"
            );
            println!(
                "3. Add registry entry: category, description, path, aliases, optional resources"
            );
            println!("4. Keep description one line; say what it does and when to use it");
            println!(
                "5. Verify: skillbox doctor; skillbox search \"<query>\"; skillbox fetch <skill>"
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

fn print_agent_guide() {
    println!("1. Browse: skillbox list or skillbox list --category <category>");
    println!("2. Search: skillbox search \"<task>\"");
    println!("3. Inspect when needed: skillbox info <skill>");
    println!("4. Load: skillbox fetch <skill>");
    println!(
        "   Fetch prints SKILL.md; when support files exist, it also reports their counts and temp path."
    );
    println!("5. Search the public directory only when needed: skillbox search \"<task>\" --web");
    println!("   skills.sh results are unverified; fetch them by full owner/repo/skill id.");
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
                " (missing, no remotes configured)"
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

    match load_global_registry()? {
        Some((root, registry)) => {
            println!(
                "global: {} ({} skills)",
                root.join("skillbox.yaml").display(),
                registry.skills.len()
            );
        }
        None => println!("global: none"),
    }

    match installed_path() {
        Some(path) if path.exists() => {
            let installed = read_installed_file_at(&path)?;
            println!(
                "installed: {} ({} skills)",
                path.display(),
                installed.skills.len()
            );
        }
        _ => println!("installed: none"),
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
    if value == "external" || ALLOWED_CATEGORIES.contains(&value) {
        Ok(value.to_string())
    } else {
        Err(format!(
            "unknown category '{}'; expected one of: {}, external",
            value,
            ALLOWED_CATEGORIES.join(", ")
        ))
    }
}

fn parse_promote_category(value: &str) -> std::result::Result<String, String> {
    let category = parse_category(value)?;
    if category == "external" {
        Err(format!(
            "category 'external' cannot be promoted; expected one of: {}",
            ALLOWED_CATEGORIES.join(", ")
        ))
    } else {
        Ok(category)
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
        merge_registry_skills(&mut merged, registry, Source::Project { root })?;
    }

    if let Some((root, registry)) = load_global_registry()? {
        merge_registry_skills(&mut merged, registry, Source::Global { root })?;
    }

    if let Some(path) = installed_path()
        && path.exists()
    {
        let installed = read_installed_file_at(&path)?;
        for (name, entry) in installed.skills {
            merged.entry(name.clone()).or_insert(ResolvedSkill {
                name,
                entry: SkillEntry {
                    category: "external".to_string(),
                    description: entry.description.clone(),
                    path: entry.path.clone(),
                    aliases: Vec::new(),
                    resources: Vec::new(),
                },
                source: Source::Installed {
                    source: entry.source,
                    path: entry.path,
                },
            });
        }
    }

    for remote in configured_registries()? {
        let registry = match load_remote_registry(&remote) {
            Ok(registry) => registry,
            Err(error) => {
                eprintln!(
                    "warning: skipping registry {}: {:#}",
                    remote_registry_label(&remote),
                    error
                );
                continue;
            }
        };
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

fn merge_registry_skills(
    merged: &mut BTreeMap<String, ResolvedSkill>,
    registry: RegistryFile,
    source: Source,
) -> Result<()> {
    for (name, entry) in registry.skills {
        validate_entry(&name, &entry)?;
        merged.entry(name.clone()).or_insert(ResolvedSkill {
            name,
            entry,
            source: source.clone(),
        });
    }
    Ok(())
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
        return Ok(Vec::new());
    };
    if !path.exists() {
        return Ok(Vec::new());
    }

    let text =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }

    let config = serde_yaml::from_str::<Config>(&text)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(registries_from_config(Some(config)))
}

fn registries_from_config(config: Option<Config>) -> Vec<RemoteRegistry> {
    config
        .and_then(|config| config.registries)
        .unwrap_or_default()
}

fn skillbox_config_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".skillbox"))
}

fn config_path() -> Option<PathBuf> {
    skillbox_config_dir().map(|dir| dir.join("config.yaml"))
}

fn installed_path() -> Option<PathBuf> {
    skillbox_config_dir().map(|dir| dir.join("installed.yaml"))
}

fn installed_path_display() -> String {
    installed_path()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "~/.skillbox/installed.yaml".to_string())
}

fn load_global_registry() -> Result<Option<(PathBuf, RegistryFile)>> {
    let Some(root) = dirs::home_dir().map(|home| home.join(".skillbox")) else {
        return Ok(None);
    };
    let path = root.join("skillbox.yaml");
    if !path.exists() {
        return Ok(None);
    }
    let registry = read_yaml::<RegistryFile>(&path)?;
    Ok(Some((root, registry)))
}

fn read_registry_file_at(path: &Path) -> Result<RegistryFile> {
    if !path.exists() {
        return Ok(empty_registry_file());
    }

    let registry = read_yaml::<RegistryFile>(path)?;
    if registry.version != 1 {
        bail!(
            "unsupported registry version {} from {}",
            registry.version,
            path.display()
        );
    }
    Ok(registry)
}

fn write_registry_file_at(path: &Path, registry: &RegistryFile) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let text = serde_yaml::to_string(registry)
        .with_context(|| format!("failed to serialize {}", path.display()))?;
    fs::write(path, text).with_context(|| format!("failed to write {}", path.display()))
}

fn empty_registry_file() -> RegistryFile {
    RegistryFile {
        version: 1,
        skills: BTreeMap::new(),
    }
}

fn read_installed_file_at(path: &Path) -> Result<InstalledFile> {
    if !path.exists() {
        return Ok(empty_installed_file());
    }

    let installed = read_yaml::<InstalledFile>(path)?;
    if installed.version != 1 {
        bail!(
            "unsupported installed registry version {} from {}",
            installed.version,
            path.display()
        );
    }
    Ok(installed)
}

fn write_installed_file_at(path: &Path, installed: &InstalledFile) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let text = serde_yaml::to_string(installed)
        .with_context(|| format!("failed to serialize {}", path.display()))?;
    fs::write(path, text).with_context(|| format!("failed to write {}", path.display()))
}

fn empty_installed_file() -> InstalledFile {
    InstalledFile {
        version: 1,
        skills: BTreeMap::new(),
    }
}

fn find_project_registry() -> Result<Option<(PathBuf, RegistryFile)>> {
    let mut current = std::env::current_dir().context("failed to read current directory")?;
    let home = dirs::home_dir();
    loop {
        if home.as_ref().is_some_and(|home| current == *home) {
            return Ok(None);
        }
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
        Source::Project { root } | Source::Global { root } => {
            let path = root.join(&skill.entry.path).join("SKILL.md");
            fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))
        }
        Source::Remote { registry } => {
            let path = format!("{}/SKILL.md", skill.entry.path.trim_end_matches('/'));
            fetch_github_content(registry, &path)
                .or_else(|_| fetch_text(&remote_raw_url(registry, &path)))
        }
        Source::Installed { source, path } => read_installed_skill_markdown(source, path),
    }
}

fn copy_skill_to_temp(skill: &ResolvedSkill) -> Result<PathBuf> {
    let destination = create_skill_temp_dir(&skill.name)?;

    match &skill.source {
        Source::Project { root } | Source::Global { root } => {
            let source = root.join(&skill.entry.path);
            copy_dir(&source, destination.path())?;
        }
        Source::Remote { registry } => {
            copy_remote_skill(registry, &skill.entry.path, destination.path())?;
        }
        Source::Installed { source, path } => {
            copy_installed_skill(source, path, destination.path())?;
        }
    }

    Ok(destination.keep())
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
        validate_archive_relative_path(inside_skill)?;
        if !archive_entry_is_regular(&entry) {
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

fn parse_external_ref(value: &str) -> Result<Option<ExternalRef>> {
    if !value.contains('/') {
        return Ok(None);
    }

    let parts = value.split('/').collect::<Vec<_>>();
    if parts.len() != 3 || parts.iter().any(|part| part.trim().is_empty()) {
        bail!("external skill references must use owner/repo/skillId");
    }

    Ok(Some(ExternalRef {
        owner: parts[0].to_string(),
        repo: parts[1].to_string(),
        skill_id: parts[2].to_string(),
    }))
}

fn resolve_external_skill(reference: &ExternalRef) -> Result<ExternalSkill> {
    let (branch, archive) = fetch_repo_archive(&reference.owner, &reference.repo)?;
    let folders = skill_folders_in_archive(&archive)?;
    let Some(path) = matching_skill_folder(&reference.skill_id, &folders) else {
        let names = available_skill_folder_names(&folders);
        bail!(
            "skill '{}' was not found in {}/{}; available skills: {}",
            reference.skill_id,
            reference.owner,
            reference.repo,
            names.join(", ")
        );
    };
    let markdown_path = format!("{}/SKILL.md", path.trim_end_matches('/'));
    let markdown = read_archive_file(&archive, &markdown_path)?;

    Ok(ExternalSkill {
        reference: reference.clone(),
        branch,
        path,
        markdown,
        archive,
    })
}

fn resolve_promoted_skill(reference: &str) -> Result<ExternalSkill> {
    if let Some(reference) = parse_external_ref(reference)? {
        return resolve_external_skill(&reference);
    }

    let path = installed_path().ok_or_else(|| anyhow!("failed to locate config directory"))?;
    let installed = read_installed_file_at(&path)?;
    let entry = installed
        .skills
        .get(reference)
        .ok_or_else(|| anyhow!("installed external skill '{}' was not found", reference))?;

    resolve_installed_skill(reference, &entry.source, &entry.path)
}

fn resolve_installed_skill(name: &str, source: &str, path: &str) -> Result<ExternalSkill> {
    let (owner, repo) = parse_installed_source(source)?;
    let (branch, archive) = fetch_repo_archive(owner, repo)?;
    let markdown_path = format!("{}/SKILL.md", path.trim_end_matches('/'));
    let markdown = read_archive_file(&archive, &markdown_path)?;

    Ok(ExternalSkill {
        reference: ExternalRef {
            owner: owner.to_string(),
            repo: repo.to_string(),
            skill_id: name.to_string(),
        },
        branch,
        path: path.to_string(),
        markdown,
        archive,
    })
}

fn fetch_repo_archive(owner: &str, repo: &str) -> Result<(String, Vec<u8>)> {
    let main_url = repo_archive_url(owner, repo, "main");
    match fetch_bytes(&main_url) {
        Ok(bytes) => Ok(("main".to_string(), bytes)),
        Err(main_error) => {
            let master_url = repo_archive_url(owner, repo, "master");
            fetch_bytes(&master_url)
                .map(|bytes| ("master".to_string(), bytes))
                .with_context(|| {
                    format!(
                        "failed to download {} on main ({:#}) or master",
                        repo_label(owner, repo),
                        main_error
                    )
                })
        }
    }
}

fn copy_external_skill_to_temp(skill: &ExternalSkill, destination: &Path) -> Result<()> {
    copy_skill_from_archive(&skill.archive, &skill.path, destination)
}

fn copy_installed_skill(source: &str, path: &str, destination: &Path) -> Result<()> {
    let (owner, repo) = parse_installed_source(source)?;
    let (_, archive) = fetch_repo_archive(owner, repo)?;
    copy_skill_from_archive(&archive, path, destination)
}

fn copy_skill_from_archive(archive: &[u8], skill_path: &str, destination: &Path) -> Result<()> {
    let decoder = flate2::read::GzDecoder::new(archive);
    let mut archive = tar::Archive::new(decoder);
    let mut copied = false;
    let trimmed = Path::new(skill_path.trim_matches('/'));

    fs::create_dir_all(destination)
        .with_context(|| format!("failed to create {}", destination.display()))?;

    for entry in archive
        .entries()
        .context("failed to read repository archive")?
    {
        let mut entry = entry.context("failed to read archive entry")?;
        let path = entry
            .path()
            .context("failed to read archive path")?
            .into_owned();
        let Some(relative) = path_without_archive_root(&path) else {
            continue;
        };
        let Ok(inside_skill) = relative.strip_prefix(trimmed) else {
            continue;
        };
        if inside_skill.as_os_str().is_empty() {
            continue;
        }
        validate_archive_relative_path(inside_skill)?;
        if !archive_entry_is_regular(&entry) {
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

fn validate_archive_relative_path(path: &Path) -> Result<()> {
    if path
        .components()
        .all(|component| matches!(component, std::path::Component::Normal(_)))
    {
        Ok(())
    } else {
        bail!("archive contains unsafe path '{}'", path.display())
    }
}

fn archive_entry_is_regular<R: Read>(entry: &tar::Entry<'_, R>) -> bool {
    let entry_type = entry.header().entry_type();
    entry_type.is_file() || entry_type.is_dir()
}

fn read_installed_skill_markdown(source: &str, path: &str) -> Result<String> {
    let (owner, repo) = parse_installed_source(source)?;
    let (_, archive) = fetch_repo_archive(owner, repo)?;
    read_archive_file(
        &archive,
        &format!("{}/SKILL.md", path.trim_end_matches('/')),
    )
}

fn read_archive_file(archive: &[u8], wanted: &str) -> Result<String> {
    let decoder = flate2::read::GzDecoder::new(archive);
    let mut archive = tar::Archive::new(decoder);
    let wanted = Path::new(wanted.trim_matches('/'));

    for entry in archive
        .entries()
        .context("failed to read repository archive")?
    {
        let mut entry = entry.context("failed to read archive entry")?;
        let path = entry
            .path()
            .context("failed to read archive path")?
            .into_owned();
        let Some(relative) = path_without_archive_root(&path) else {
            continue;
        };
        if relative != wanted {
            continue;
        }

        let mut text = String::new();
        entry
            .read_to_string(&mut text)
            .with_context(|| format!("failed to read {}", wanted.display()))?;
        return Ok(text);
    }

    bail!("{} was not found in remote archive", wanted.display())
}

fn skill_folders_in_archive(archive: &[u8]) -> Result<Vec<String>> {
    let decoder = flate2::read::GzDecoder::new(archive);
    let mut archive = tar::Archive::new(decoder);
    let mut folders = Vec::new();

    for entry in archive
        .entries()
        .context("failed to read repository archive")?
    {
        let entry = entry.context("failed to read archive entry")?;
        let path = entry
            .path()
            .context("failed to read archive path")?
            .into_owned();
        let Some(relative) = path_without_archive_root(&path) else {
            continue;
        };
        if relative.file_name().is_some_and(|name| name == "SKILL.md")
            && let Some(parent) = relative.parent()
        {
            folders.push(parent.to_string_lossy().to_string());
        }
    }

    folders.sort();
    folders.dedup();
    Ok(folders)
}

fn path_without_archive_root(path: &Path) -> Option<PathBuf> {
    let mut components = path.components();
    components.next()?;
    Some(components.as_path().to_path_buf())
}

fn matching_skill_folder(skill_id: &str, folders: &[String]) -> Option<String> {
    folders
        .iter()
        .find(|folder| folder_name(folder).is_some_and(|name| name == skill_id))
        .cloned()
        .or_else(|| {
            folders
                .iter()
                .find(|folder| {
                    folder_name(folder)
                        .is_some_and(|name| name.ends_with(skill_id) || skill_id.ends_with(name))
                })
                .cloned()
        })
}

fn folder_name(path: &str) -> Option<&str> {
    path.trim_matches('/').rsplit('/').next()
}

fn available_skill_folder_names(folders: &[String]) -> Vec<String> {
    folders
        .iter()
        .filter_map(|folder| folder_name(folder).map(str::to_string))
        .collect()
}

fn parse_installed_source(source: &str) -> Result<(&str, &str)> {
    let parts = source.split('/').collect::<Vec<_>>();
    if parts.len() != 2 || parts.iter().any(|part| part.trim().is_empty()) {
        bail!("installed skill source '{}' must use owner/repo", source);
    }
    Ok((parts[0], parts[1]))
}

fn promoted_skill_entry(
    category: &str,
    markdown: &str,
    name: &str,
    resources: Vec<String>,
) -> SkillEntry {
    SkillEntry {
        category: category.to_string(),
        description: description_from_markdown(markdown),
        path: format!("skills/{name}"),
        aliases: Vec::new(),
        resources,
    }
}

fn resource_entries(path: &Path) -> Result<Vec<String>> {
    let mut resources = Vec::new();
    for entry in fs::read_dir(path).with_context(|| format!("failed to read {}", path.display()))? {
        let entry = entry.with_context(|| format!("failed to read {}", path.display()))?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name != "SKILL.md" {
            resources.push(name);
        }
    }
    resources.sort();
    Ok(resources)
}

fn resource_file_counts(path: &Path, resources: &[String]) -> Result<Vec<(String, usize)>> {
    resources
        .iter()
        .map(|resource| {
            let resource_path = path.join(resource);
            count_files(&resource_path).map(|count| (resource.clone(), count))
        })
        .collect()
}

fn count_files(path: &Path) -> Result<usize> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to inspect {}", path.display()));
        }
    };
    if metadata.file_type().is_symlink() {
        return Ok(0);
    }
    if metadata.is_file() {
        return Ok(1);
    }
    if !metadata.is_dir() {
        return Ok(0);
    }

    let mut count = 0;
    for entry in fs::read_dir(path).with_context(|| format!("failed to read {}", path.display()))? {
        let entry = entry.with_context(|| format!("failed to read {}", path.display()))?;
        count += count_files(&entry.path())?;
    }
    Ok(count)
}

fn pluralize(word: &str, count: usize) -> String {
    if count == 1 {
        word.to_string()
    } else {
        format!("{word}s")
    }
}

fn remove_installed_skill_if_present(skill: &ExternalSkill) -> Result<()> {
    let Some(path) = installed_path() else {
        return Ok(());
    };
    if !path.exists() {
        return Ok(());
    }

    let mut installed = read_installed_file_at(&path)?;
    let name = &skill.reference.skill_id;
    let source = format!("{}/{}", skill.reference.owner, skill.reference.repo);
    let Some(entry) = installed.skills.get(name) else {
        return Ok(());
    };
    if entry.source != source {
        return Ok(());
    }

    installed.skills.remove(name);
    write_installed_file_at(&path, &installed)
}

fn remove_existing_path(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    if path.is_dir() {
        fs::remove_dir_all(path).with_context(|| format!("failed to remove {}", path.display()))
    } else {
        fs::remove_file(path).with_context(|| format!("failed to remove {}", path.display()))
    }
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
    let metadata = fs::symlink_metadata(source)
        .with_context(|| format!("failed to inspect {}", source.display()))?;
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    if metadata.is_dir() {
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

fn percent_encode(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn description_from_markdown(markdown: &str) -> String {
    if let Some(description) = frontmatter_description(markdown) {
        return description;
    }

    let mut lines = markdown.lines().peekable();
    if lines.peek().is_some_and(|line| line.trim() == "---") {
        lines.next();
        for line in lines.by_ref() {
            if line.trim() == "---" {
                break;
            }
        }
    }

    lines
        .find_map(|line| {
            let trimmed = line.trim().trim_start_matches('#').trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(one_line(trimmed))
            }
        })
        .unwrap_or_else(|| "External skill".to_string())
}

fn frontmatter_description(markdown: &str) -> Option<String> {
    let mut lines = markdown.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }

    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            return None;
        }
        if let Some(value) = trimmed.strip_prefix("description:") {
            let description = value.trim().trim_matches(['"', '\'']);
            if !description.is_empty() {
                return Some(one_line(description));
            }
        }
    }
    None
}

fn one_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn warn_unverified_if_external(skill: &ResolvedSkill) {
    if matches!(skill.source, Source::Installed { .. }) {
        eprintln!("warning: external skill from skills.sh — unverified third-party content");
    }
}

fn print_empty_registry_hint() {
    eprintln!(
        "hint: no skills loaded; global skills live in ~/.skillbox/skillbox.yaml with folders under ~/.skillbox/skills/; remote registries are opt-in via ~/.skillbox/config.yaml; see skillbox guide onboarding."
    );
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

fn repo_archive_url(owner: &str, repo: &str, reference: &str) -> String {
    format!(
        "https://github.com/{}/{}/archive/refs/heads/{}.tar.gz",
        owner, repo, reference
    )
}

fn repo_label(owner: &str, repo: &str) -> String {
    format!("{owner}/{repo}")
}

fn external_tree_url(skill: &ExternalSkill) -> String {
    format!(
        "https://github.com/{}/{}/tree/{}/{}",
        skill.reference.owner,
        skill.reference.repo,
        skill.branch,
        skill.path.trim_start_matches('/')
    )
}

fn installed_tree_url(source: &str, path: &str) -> String {
    match parse_installed_source(source) {
        Ok((owner, repo)) => format!(
            "https://github.com/{}/{}/tree/main/{}",
            owner,
            repo,
            path.trim_start_matches('/')
        ),
        Err(_) => format!("{}:{}", source, path),
    }
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

fn create_skill_temp_dir(name: &str) -> Result<tempfile::TempDir> {
    let root = temp_root();
    fs::create_dir_all(&root).with_context(|| format!("failed to create {}", root.display()))?;
    tempfile::Builder::new()
        .prefix(&format!(
            "{}-{}-",
            sanitize_name(name),
            Local::now().format("%Y%m%d%H%M%S")
        ))
        .tempdir_in(&root)
        .with_context(|| format!("failed to create skill temp folder in {}", root.display()))
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

    fn test_remote_registry() -> RemoteRegistry {
        RemoteRegistry {
            owner: Some("hhushhas".to_string()),
            repo: "skillbox-registry".to_string(),
            reference: Some("main".to_string()),
        }
    }

    fn test_registry_skill(description: &str, path: &str) -> SkillEntry {
        SkillEntry {
            category: "frontend".to_string(),
            description: description.to_string(),
            path: path.to_string(),
            aliases: Vec::new(),
            resources: Vec::new(),
        }
    }

    #[test]
    fn external_ref_parser_accepts_three_segments_only() {
        assert_eq!(
            parse_external_ref("owner/repo/skill-id").expect("parse external"),
            Some(ExternalRef {
                owner: "owner".to_string(),
                repo: "repo".to_string(),
                skill_id: "skill-id".to_string(),
            })
        );
        assert_eq!(parse_external_ref("plain-skill").expect("local"), None);
        assert!(parse_external_ref("owner/repo").is_err());
        assert!(parse_external_ref("owner/repo/skill/extra").is_err());
        assert!(parse_external_ref("owner//skill").is_err());
    }

    #[test]
    fn skill_folder_matching_prefers_exact_then_suffix() {
        let folders = vec![
            "skills/react-best-practices".to_string(),
            "skills/vercel-react-best-practices".to_string(),
            "skills/database".to_string(),
        ];

        assert_eq!(
            matching_skill_folder("vercel-react-best-practices", &folders),
            Some("skills/vercel-react-best-practices".to_string())
        );

        let suffix_only = vec![
            "skills/react-best-practices".to_string(),
            "skills/database".to_string(),
        ];
        assert_eq!(
            matching_skill_folder("vercel-react-best-practices", &suffix_only),
            Some("skills/react-best-practices".to_string())
        );
        assert_eq!(matching_skill_folder("python", &suffix_only), None);
    }

    #[test]
    fn installed_file_round_trips() {
        let root = tempfile::tempdir().expect("tempdir");
        let path = root.path().join("installed.yaml");
        let mut installed = empty_installed_file();
        installed.skills.insert(
            "vercel-react-best-practices".to_string(),
            InstalledSkillEntry {
                source: "vercel-labs/agent-skills".to_string(),
                path: "skills/react-best-practices".to_string(),
                description: "React best practices".to_string(),
            },
        );

        write_installed_file_at(&path, &installed).expect("write installed");
        let round_tripped = read_installed_file_at(&path).expect("read installed");

        assert_eq!(round_tripped.version, 1);
        assert_eq!(
            round_tripped
                .skills
                .get("vercel-react-best-practices")
                .expect("installed skill")
                .path,
            "skills/react-best-practices"
        );
    }

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
    fn command_parser_keeps_list_search_and_fetch_distinct() {
        assert!(Cli::try_parse_from(["skillbox", "list", "--category", "frontend"]).is_ok());
        assert!(Cli::try_parse_from(["skillbox", "list", "react"]).is_err());
        assert!(Cli::try_parse_from(["skillbox", "search"]).is_err());
        assert!(Cli::try_parse_from(["skillbox", "search", "react", "performance"]).is_ok());
        assert!(Cli::try_parse_from(["skillbox", "search", "react", "--web"]).is_ok());
        assert!(Cli::try_parse_from(["skillbox", "fetch", "react"]).is_ok());
        assert!(Cli::try_parse_from(["skillbox", "fetch", "react", "--print"]).is_err());
        assert!(Cli::try_parse_from(["skillbox", "fetch", "react", "--to-temp"]).is_err());
    }

    #[test]
    fn resource_summary_counts_nested_support_files() {
        let root = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(root.path().join("references").join("nested")).expect("references dir");
        fs::create_dir(root.path().join("scripts")).expect("scripts dir");
        fs::write(root.path().join("references").join("one.md"), "one").expect("reference file");
        fs::write(
            root.path().join("references").join("nested").join("two.md"),
            "two",
        )
        .expect("nested reference file");
        fs::write(root.path().join("scripts").join("run.sh"), "run").expect("script file");

        let resources = vec!["references".to_string(), "scripts".to_string()];
        let counts = resource_file_counts(root.path(), &resources).expect("resource counts");

        assert_eq!(
            counts,
            vec![("references".to_string(), 2), ("scripts".to_string(), 1)]
        );
        assert_eq!(
            format_resource_summary(&counts),
            "2 files in references, 1 file in scripts"
        );
    }

    #[cfg(unix)]
    #[test]
    fn resource_count_skips_symlinks() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().expect("tempdir");
        let destination = tempfile::tempdir().expect("destination tempdir");
        let references = root.path().join("references");
        fs::create_dir(&references).expect("references dir");
        fs::write(references.join("notes.md"), "notes").expect("reference file");
        symlink("..", references.join("loop")).expect("loop symlink");
        symlink("notes.md", references.join("notes-link.md")).expect("file symlink");

        assert_eq!(count_files(&references).expect("resource count"), 1);
        copy_dir(root.path(), destination.path()).expect("copy skill");
        assert!(!destination.path().join("references").join("loop").exists());
        assert!(
            !destination
                .path()
                .join("references")
                .join("notes-link.md")
                .exists()
        );
    }

    #[test]
    fn archive_paths_must_stay_relative() {
        assert!(validate_archive_relative_path(Path::new("references/notes.md")).is_ok());
        assert!(validate_archive_relative_path(Path::new("../escape.md")).is_err());
        assert!(validate_archive_relative_path(Path::new("references/../escape.md")).is_err());
    }

    #[test]
    fn category_parser_rejects_unknown_categories() {
        assert_eq!(
            parse_category("frontend").expect("known category"),
            "frontend"
        );
        assert_eq!(
            parse_category("external").expect("external category"),
            "external"
        );
        assert!(parse_category("sales").is_err());
    }

    #[test]
    fn promote_category_parser_rejects_external() {
        assert_eq!(
            parse_promote_category("backend").expect("known category"),
            "backend"
        );
        assert!(parse_promote_category("external").is_err());
        assert!(parse_promote_category("sales").is_err());
    }

    #[test]
    fn promoted_entry_uses_markdown_description_and_resource_entries() {
        let root = tempfile::tempdir().expect("tempdir");
        fs::create_dir(root.path().join("scripts")).expect("scripts dir");
        fs::create_dir(root.path().join("references")).expect("references dir");
        fs::write(root.path().join("SKILL.md"), "# Example").expect("skill file");
        fs::write(root.path().join("template.html"), "template").expect("support file");

        let resources = resource_entries(root.path()).expect("resources");
        let entry = promoted_skill_entry(
            "ai",
            "---\ndescription: Use for careful agent orchestration.\n---\n# Fallback",
            "agent-orchestration",
            resources,
        );

        assert_eq!(entry.category, "ai");
        assert_eq!(entry.description, "Use for careful agent orchestration.");
        assert_eq!(entry.path, "skills/agent-orchestration");
        assert!(entry.aliases.is_empty());
        assert_eq!(
            entry.resources,
            vec![
                "references".to_string(),
                "scripts".to_string(),
                "template.html".to_string()
            ]
        );
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
    fn config_without_registries_yields_no_remote_registries() {
        let config = serde_yaml::from_str::<Config>("{}").expect("parse config");
        assert!(registries_from_config(Some(config)).is_empty());

        let config = serde_yaml::from_str::<Config>("registries: []").expect("parse config");

        assert!(registries_from_config(Some(config)).is_empty());
        assert!(registries_from_config(None).is_empty());
    }

    #[test]
    fn project_registry_shadows_global_registry() {
        let project_root = PathBuf::from("/project");
        let global_root = PathBuf::from("/home/user/.skillbox");
        let mut project_skills = BTreeMap::new();
        project_skills.insert(
            "frontend".to_string(),
            test_registry_skill(
                "Project frontend guidance",
                ".agents/skills.available/frontend",
            ),
        );
        let mut global_skills = BTreeMap::new();
        global_skills.insert(
            "frontend".to_string(),
            test_registry_skill("Global frontend guidance", "skills/frontend"),
        );
        let mut merged = BTreeMap::<String, ResolvedSkill>::new();

        merge_registry_skills(
            &mut merged,
            RegistryFile {
                version: 1,
                skills: project_skills,
            },
            Source::Project { root: project_root },
        )
        .expect("merge project");
        merge_registry_skills(
            &mut merged,
            RegistryFile {
                version: 1,
                skills: global_skills,
            },
            Source::Global { root: global_root },
        )
        .expect("merge global");

        let skill = merged.get("frontend").expect("merged skill");
        assert_eq!(skill.entry.description, "Project frontend guidance");
        assert!(matches!(skill.source, Source::Project { .. }));
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
                registry: test_remote_registry(),
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
                registry: test_remote_registry(),
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
                registry: test_remote_registry(),
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
                registry: test_remote_registry(),
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
                registry: test_remote_registry(),
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
