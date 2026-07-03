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

Homebrew:

```bash
brew install hhushhas/tap/skillbox
```

Cargo:

```bash
cargo install --git https://github.com/hhushhas/skillbox
```

Prebuilt binaries are on the [releases page](https://github.com/hhushhas/skillbox/releases). For local development:

```bash
cargo build --release
cp target/release/skillbox ~/bin/skillbox
```

## Usage

```bash
skillbox list                        # everything
skillbox list "design for chatbot"   # natural-language query
skillbox list --category frontend
skillbox search effect               # keyword search
skillbox info frontend               # where a skill comes from
skillbox fetch frontend --print      # print SKILL.md to stdout
skillbox fetch frontend --to-temp    # pull skill + support files into a temp dir
skillbox guide                       # agent-facing usage guide
skillbox cleanup
skillbox doctor
```

## skills.sh on demand

The registry stays the primary, trusted path — but you can reach the whole [skills.sh](https://skills.sh) directory without leaving the CLI:

```bash
skillbox search react --web                                      # search the skills.sh directory
skillbox fetch vercel-labs/agent-skills/vercel-react-best-practices --print   # use once, right now
skillbox add vercel-labs/agent-skills/vercel-react-best-practices             # keep it
skillbox remove vercel-react-best-practices                      # drop it again
```

`--web` is always explicit, and external content is always marked unverified — nothing from skills.sh enters a conversation unless you (or your agent, at your instruction) ask for it by full `owner/repo/skill` id. `add` records the skill in `~/.config/skillbox/installed.yaml`; from then on it resolves by short name like any other skill, listed under the `external` category, fetched fresh from its source repo on demand.

External skills are unvetted third-party instructions. Skim `skillbox fetch <ref> --print` before letting an agent load one, and promote skills you rely on into your registry.

## Registries

Skillbox resolves skills in order:

1. The nearest project registry: `.agents/skillbox.yaml`
2. Installed external skills: `~/.config/skillbox/installed.yaml`
3. Remote registries from `~/.config/skillbox/config.yaml`
4. The default shared registry: `hhushhas/skillbox-registry`

For reusable skills, work in a local checkout of the registry repo rather than adding one-off local config.

Registry entries can include aliases to improve natural-language search:

```yaml
skills:
  react:
    category: frontend
    description: "Work with React; use for components, Next.js, performance, and React Native guidance."
    path: skills/react
    aliases: [react, nextjs, hooks, components]
```

Entries can also include resource markers, which appear in `skillbox list` so agents know when to prefer `--to-temp`:

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

Reusable skills live in the public registry, `hhushhas/skillbox-registry`. Project skills live with the project:

```text
repo/.agents/skillbox.yaml
repo/.agents/skills.available/<skill-name>/SKILL.md
```

To add a reusable skill:

1. Add `skills/<skill-name>/SKILL.md` to the registry repo, with any support files (`references/`, `scripts/`, `assets/`, `agents/`) in the same folder.
2. Add the skill to `registry.yaml`: pick one category from the fixed list, write a canonical one-line description, and add aliases for natural-language search.
3. Add `resources` when the skill has support folders.
4. Verify with `skillbox doctor`, `skillbox search "<query>"`, and `skillbox fetch <skill-name> --print` or `--to-temp`.

To update an existing skill, run `skillbox info <skill-name>` to find its registry, edit the entry and skill folder, then run the same verification commands.

### Importing from skills.sh

For personal use, `skillbox add owner/repo/skill` is enough. To promote a skill into a shared registry, treat the `skills.sh` URL as a discovery page, not the canonical source — never scrape rendered `skills.sh` HTML as the skill artifact.

1. Find the backing GitHub repo/path on the `skills.sh` page (or via `skillbox search <query> --web`).
2. Fetch the raw GitHub `SKILL.md` and copy it (plus sibling support folders) into `skills/<skill-name>/`.
3. Add a normalized entry to `registry.yaml`, then verify and publish:

```bash
ruby -e 'require "yaml"; YAML.load_file("registry.yaml"); puts "ok"'
skillbox search "<natural query>"
skillbox fetch <skill-name> --print
git add registry.yaml skills/<skill-name>
git commit -m "Add <skill-name> skill"
git push
```

### Canonical descriptions

`registry.yaml` descriptions are the model-facing discovery text. One line, 70–150 characters, starting with a practical capability — no hype ("comprehensive", "ultimate"), no implementation history or install notes. Preferred shape:

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
