# Skillbox

Skillbox is a small, agent-agnostic CLI for listing and fetching trusted coding-agent skills on demand.

## Commands

```bash
skillbox list
skillbox list --category frontend
skillbox list "react guidelines"
skillbox list "design for chatbot"
skillbox fetch frontend --print
skillbox fetch frontend --to-temp
skillbox cleanup
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
