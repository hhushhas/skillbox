# Skillbox

**Your skills. Loaded only when you ask.**

Install a skill globally and every coding agent starts loading it on its own — wanted or not. Skillbox keeps skills out of the agent's context until you say the word.

## The problem

You find a skill you like, so you install it globally to reuse it across projects. Then the agent starts invoking it by itself. Sometimes that's what you want; often it isn't, and now you're pruning, moving, and renaming skill folders so the agent doesn't grab the wrong thing.

The root cause: Claude Code, Codex, opencode, Cursor, Pi — every harness implements skills differently, but they share one behavior. Once a skill's frontmatter is loaded into context, the agent is bound to it.

## The fix

Skillbox keeps skills outside the harness entirely. No frontmatter is preloaded, so nothing fires by accident. When you say "use this skill", the agent runs `skillbox search`, surfaces the skill, and fetches it — right then, and only then. You decide when a skill enters the conversation.

One CLI, every agent, zero babysitting.

## Install

npm:

```bash
npm install --global @hhushhas/skillbox
skillbox search "browser automation"
npx @hhushhas/skillbox search "browser automation"
```

The npm launcher downloads and verifies the matching native release binary on first use. The unscoped npm name `skillbox` belongs to another project, so the supported package is `@hhushhas/skillbox`.

Homebrew:

```bash
brew install hhushhas/tap/skillbox
```

For source-tree development, install directly from Git:

```bash
cargo install --git https://github.com/hhushhas/skillbox
```

Prebuilt binaries for macOS, Linux, and Windows are attached to tagged releases on the [releases page](https://github.com/hhushhas/skillbox/releases). For local development:

```bash
cargo build --release
cp target/release/skillbox ~/bin/skillbox
```

## Usage

```bash
skillbox list                        # everything
skillbox list --category frontend
skillbox search "design for chatbot" # natural-language registry search
skillbox search react --web          # explicit skills.sh search (unverified)
skillbox info frontend               # where a skill comes from
skillbox fetch frontend              # print SKILL.md; prepare support files in temp
skillbox setup                       # select and configure detected agent harnesses
skillbox setup --harness claude-code,pi --yes
skillbox setup --status
skillbox audit                       # inspect recent list/search/info/fetch activity
skillbox audit --operation fetch --since 24h --json
skillbox guide                       # agent-facing usage guide
skillbox cleanup
skillbox doctor

# The npm launcher accepts the same commands:
npx @hhushhas/skillbox setup --status
npx @hhushhas/skillbox search "browser automation"
```

## Usage audit

Skillbox records `list`, `search`, `info`, and `fetch` invocations in `~/.skillbox/audit.jsonl`. Each event includes the operation, outcome, timestamp, working directory, requested or resolved skill, and available source, skill hash, search result count, and harness metadata. Audit logging is local and does not block the original command; if the log cannot be written, Skillbox keeps the command result and prints a warning.

Run `skillbox setup` to select the detected harnesses in an interactive terminal. Use `skillbox setup --harness codex,claude-code,pi --yes` for non-interactive setup, or `skillbox setup --status` to inspect the current wiring. The command is idempotent, preserves existing Claude settings and Pi files with timestamped backups when it replaces them, and does not contact a remote service.

Codex thread IDs are detected automatically from `CODEX_THREAD_ID`, so Codex agents do not need a configuration file or extra argument. Codex model IDs are intentionally left unset. Claude Code and Pi capture the same metadata automatically through their harness integration; other harnesses can set these once in their launcher or session hook, and every later Skillbox command inherits them:

```text
Codex:       built in
Claude Code: skillbox setup --harness claude-code --yes
Pi:          skillbox setup --harness pi --yes
```

The Claude adapter writes a maintained hook to `~/.skillbox/hooks/`, adds `SessionStart`, `PreToolUse` for Bash, and `SessionEnd` entries to `~/.claude/settings.json`, and keeps a timestamped settings backup when replacing an existing file. The hook remembers Claude's model from `SessionStart`, then patches only Bash commands that invoke Skillbox so the child shell receives the session ID, model ID, and transcript path. The Pi adapter writes the conventional global extension to `~/.pi/agent/extensions/skillbox-audit.ts`; an existing package-managed Skillbox extension is recognized without adding a duplicate. The extension reads Pi's live session manager and current model at the mutable `tool_call` boundary and applies the same child-shell environment. Neither integration changes the agent prompt or requires agent-supplied flags.

For a harness without an integration, set the metadata explicitly:

```bash
export SKILLBOX_HARNESS=claude-code
export SKILLBOX_THREAD_ID=<session-id>
export SKILLBOX_MODEL_ID=<model-id>
export SKILLBOX_TRANSCRIPT_PATH=<transcript-path>
```

Use `skillbox audit --json` for JSONL output, or combine filters such as `--skill`, `--harness`, `--thread`, `--status`, `--operation`, `--model`, and `--since 24h`. Search queries are stored locally in raw form by default because they explain why a skill was discovered. Set `SKILLBOX_AUDIT_QUERY_MODE=hash` or `omit` when those queries should not be retained. Set `SKILLBOX_AUDIT_PATH` to use a different local log path.

## skills.sh on demand

Local registries stay the trusted path, but you can reach the whole [skills.sh](https://skills.sh) directory without leaving the CLI:

```bash
skillbox search react --web                                            # discover unverified skills
skillbox fetch vercel-labs/agent-skills/vercel-react-best-practices    # use once, right now
skillbox add vercel-labs/agent-skills/vercel-react-best-practices      # keep it
skillbox promote vercel-react-best-practices --category frontend       # trust it locally
skillbox remove vercel-react-best-practices                            # drop it again
```

`--web` is always explicit, and external content is always marked unverified — nothing from skills.sh enters a conversation unless you (or your agent, at your instruction) ask for it by full `owner/repo/skill` id. `add` records the skill in `~/.skillbox/installed.yaml`; from then on it resolves by short name like any other skill, listed under the `external` category, fetched fresh from its source repo on demand.

External skills are unvetted third-party instructions. Skim the output of `skillbox fetch <ref>` before following it, and promote skills you rely on into your local registry with `skillbox promote <ref> --category <category>`.

## Registries

Skillbox resolves skills in order:

1. The nearest project registry: `.agents/skillbox.yaml`
2. The global local registry: `~/.skillbox/skillbox.yaml`
3. Installed external skills: `~/.skillbox/installed.yaml`
4. Remote registries from `~/.skillbox/config.yaml`

The global registry is the default home for personal skills: offline, private, and resolved from `~/.skillbox`. A conventional entry points at `~/.skillbox/skills/<skill-name>/SKILL.md`.

Remote registries are explicit opt-in. To add the shared Skillbox registry, create `~/.skillbox/config.yaml`:

```yaml
registries:
  - owner: hhushhas
    repo: skillbox-registry
```

Registry entries can include aliases to improve natural-language search:

```yaml
skills:
  react:
    category: frontend
    description: "Work with React; use for components, Next.js, performance, and React Native guidance."
    path: skills/react
    aliases: [react, nextjs, hooks, components]
```

Entries can also include resource markers for support files or directories, which appear in `skillbox list`. When `skillbox fetch <skill>` sees these markers, it copies the full skill folder to Skillbox's temp directory, prints `SKILL.md`, and writes a suggestion to stderr with the file counts and exact support-file path:

```yaml
skills:
  git-merge-report:
    category: project
    description: "Prepare merge reports; use for non-destructive branch comparison and conflict planning."
    path: skills/git-merge-report
    aliases: [git, merge, conflict, branch, diff, report]
    resources: [refs, scripts]
```

## Adding skills

Reusable personal skills live in the global local registry:

```text
~/.skillbox/skillbox.yaml
~/.skillbox/skills/<skill-name>/SKILL.md
```

Project skills live with the project and shadow global skills with the same name:

```text
repo/.agents/skillbox.yaml
repo/.agents/skills.available/<skill-name>/SKILL.md
```

To add a reusable skill:

1. Add `~/.skillbox/skills/<skill-name>/SKILL.md`, with any support files (`references/`, `scripts/`, `assets/`, `agents/`) in the same folder.
2. Add the skill to `~/.skillbox/skillbox.yaml`: pick one category from the fixed list, write a canonical one-line description, and add aliases for natural-language search.
3. Add `resources` when the skill has support files or folders.
4. Verify with `skillbox doctor`, `skillbox search "<query>"`, and `skillbox fetch <skill-name>`.

To update an existing skill, run `skillbox info <skill-name>` to find its registry, edit the entry and skill folder, then run the same verification commands.

### Importing from skills.sh

For personal use, `skillbox add owner/repo/skill` is enough. To promote a skill into your global registry, run `skillbox promote <skill> --category <category>`. For a shared remote registry, treat the `skills.sh` URL as a discovery page, not the canonical source — never scrape rendered `skills.sh` HTML as the skill artifact.

1. Find the backing GitHub repo/path on the `skills.sh` page (or via `skillbox search <query> --web`).
2. For local trust, run `skillbox promote owner/repo/skill --category <category>`; for a shared registry, fetch the raw GitHub `SKILL.md` and copy it (plus sibling support folders) into the shared registry's skills folder.
3. Add or review the normalized entry, then verify:

```bash
ruby -e 'require "yaml"; YAML.load_file(File.expand_path("~/.skillbox/skillbox.yaml")); puts "ok"'
skillbox search "<natural query>"
skillbox fetch <skill-name>
```

### Canonical descriptions

Registry descriptions are the model-facing discovery text. One line, 70–150 characters, starting with a practical capability — no hype ("comprehensive", "ultimate"), no implementation history or install notes. Preferred shape:

```text
<capability/action> for <domain/task>; use when <trigger or situation>.
```

Examples:

```text
Build and polish frontend UI; use for layouts, components, accessibility, and visual QA.
Run browser automation; use for web flows, local app QA, screenshots, and form interaction.
```

### Categories

`frontend` · `backend` · `ai` · `cloud` · `design` · `browser` · `project`

## License

MIT
