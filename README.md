# Skillbox

Skillbox is a small, agent-agnostic CLI for listing and fetching trusted coding-agent skills on demand.

## Commands

```bash
skillbox list
skillbox list --category frontend
skillbox list --names --category frontend
skillbox list "react guidelines"
skillbox list "design for chatbot"
skillbox search effect
skillbox search ai
skillbox search --names tokens
skillbox fetch frontend --print
skillbox fetch frontend --to-temp
skillbox info frontend
skillbox cleanup
skillbox doctor
```

## Install

After the repository is public:

```bash
cargo install --git https://github.com/hhushhas/skillbox
```

For local development:

```bash
cargo build --release
cp target/release/skillbox ~/bin/skillbox
```

## Download

GitHub releases are at:

```text
https://github.com/hhushhas/skillbox/releases
```

macOS Apple Silicon v0.1.5:

```bash
curl -L https://github.com/hhushhas/skillbox/releases/download/v0.1.5/skillbox-v0.1.5-aarch64-apple-darwin.tar.gz -o skillbox.tar.gz
tar -xzf skillbox.tar.gz
install -m 0755 skillbox-aarch64-apple-darwin/skillbox ~/bin/skillbox
```

## Registries

Skillbox reads the nearest project registry first:

```text
.agents/skillbox.yaml
```

Then it reads configured remote registries from:

```text
~/.config/skillbox/config.yaml
```

If no config exists, Skillbox defaults to:

```text
hhushhas/skillbox-registry
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

Registry entries can also include resource markers. These appear in `skillbox list` so agents know when to prefer `--to-temp`:

```yaml
skills:
  git-merge-report:
    category: project
    description: "Prepare merge reports; use for non-destructive branch comparison and conflict planning."
    path: skills/git-merge-report
    aliases: [git, merge, conflict, branch, diff, report]
    resources: [refs, scripts]
```

## Adding Skills

Reusable skills live in the public registry:

```text
hhushhas/skillbox-registry
```

Project skills live with the project:

```text
repo/.agents/skillbox.yaml
repo/.agents/skills.available/<skill-name>/SKILL.md
```

To add a reusable skill:

1. Add `skills/<skill-name>/SKILL.md` to the registry repo.
2. Keep optional support files inside the same skill folder, such as `references/`, `scripts/`, `assets/`, or `agents/`.
3. Add the skill to `registry.yaml`.
4. Pick one category from the fixed list below.
5. Write a canonical one-line description.
6. Add aliases for natural-language search.
7. Add `resources` when the skill has support folders such as `references/`, `scripts/`, `assets/`, `agents/`, `templates/`, `tests/`, or `evals/`.
8. Run `skillbox doctor`, `skillbox search "<query>"`, and `skillbox fetch <skill-name> --print` or `--to-temp`.

To update an existing skill, run `skillbox info <skill-name>` to find whether it comes from the project registry or a remote registry, edit the shown registry entry and skill folder, then run the same verification commands.

## Adding From skills.sh

Treat a `skills.sh` URL as a discovery page, not as the canonical source.

1. Open the `skills.sh` page and find the backing GitHub repo/path.
2. Fetch the raw GitHub `SKILL.md`.
3. Copy `SKILL.md` and any sibling `references/`, `scripts/`, `assets/`, or `agents/` into `skills/<skill-name>/`.
4. Add a normalized entry to `registry.yaml`.
5. Add `resources` when the skill has support folders such as `references/`, `scripts/`, `assets/`, `agents/`, `templates/`, `tests/`, or `evals/`.
6. Verify with:

```bash
ruby -e 'require "yaml"; YAML.load_file("registry.yaml"); puts "ok"'
skillbox list "<natural query>"
skillbox search "<natural query>"
skillbox fetch <skill-name> --print
skillbox fetch <skill-name> --to-temp
```

Then publish:

```bash
git add registry.yaml skills/<skill-name>
git commit -m "Add <skill-name> skill"
git push
```

Do not scrape rendered `skills.sh` HTML as the skill artifact. Use the raw GitHub files, then normalize the registry description and aliases.

## Canonical Descriptions

`registry.yaml` descriptions are the model-facing discovery text. Keep them consistent:

```text
- one line only
- 70-150 characters preferred
- start with a practical capability, not marketing language
- include the main trigger or use case
- name important scope only when it changes routing
- avoid hype such as "comprehensive", "ultimate", or "powerful"
- avoid implementation history, repo paths, and install notes
```

Preferred shape:

```text
<capability/action> for <domain/task>; use when <trigger or situation>.
```

Examples:

```text
Build and polish frontend UI; use for layouts, components, accessibility, and visual QA.
Work with shadcn/ui; use for component installs, registry usage, styling, and composition.
Run browser automation; use for web flows, local app QA, screenshots, and form interaction.
```

## Categories

```text
frontend
backend
ai
cloud
design
browser
project
```
