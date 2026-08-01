# Changelog

## 0.4.0 — 2026-08-01

- Record local audit events for `list`, `search`, `info`, and `fetch`, including Codex thread detection, resolved skill metadata, and queryable JSONL history.
- Add `skillbox audit` filters, JSONL output, bounded query retention modes, and generic `SKILLBOX_*` metadata overrides for other harnesses.
- Add a Claude Code hook installer and Pi extension that inject audit metadata automatically at the harness tool boundary.
- Add `skillbox setup` with interactive, non-interactive, status, dry-run, and JSON harness configuration for Codex, Claude Code, and Pi, including idempotent stale-hook repair and disabled-extension handling.
- Add a checksum-verified npm launcher for `@hhushhas/skillbox`, which downloads the matching native release binary on first use.

## 0.3.0 — 2026-07-19

- Make `skillbox fetch <skill>` print `SKILL.md` without requiring `--print` or `--to-temp`.
- Automatically prepare resource-bearing skills in the Skillbox temp directory and report support-file counts and their exact path.
- Keep `list` for browsing and move all query behavior, including explicit skills.sh discovery with `--web`, to `search`.
- Refresh the CLI guide, README, website examples, and agent router instructions for the simplified API.

## 0.2.0 — 2026-07-04

- Add promotion of external skills into the trusted local registry.
- Add multi-platform release builds for macOS, Linux, and Windows.
