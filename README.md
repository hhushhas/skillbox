# Skillbox

Skillbox is a small, agent-agnostic CLI for listing and fetching trusted coding-agent skills on demand.

## Commands

```bash
skillbox list
skillbox list --category frontend
skillbox list --names --category frontend
skillbox list "react guidelines"
skillbox list "design for chatbot"
skillbox fetch frontend --print
skillbox fetch frontend --to-temp
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
