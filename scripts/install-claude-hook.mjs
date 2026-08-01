#!/usr/bin/env node

import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  renameSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const hookPath = join(dirname(fileURLToPath(import.meta.url)), "claude-skillbox-hook.mjs");
const args = process.argv.slice(2);

function valueAfter(flag) {
  const index = args.indexOf(flag);
  return index >= 0 ? args[index + 1] : undefined;
}

function shellQuote(value) {
  return `'${String(value).replaceAll("'", "'\\''")}'`;
}

function hookCommand(event, installedHookPath) {
  return `${shellQuote(process.execPath)} ${shellQuote(installedHookPath)} ${event}`;
}

function getSettingsPath() {
  return valueAfter("--settings") ?? join(homedir(), ".claude", "settings.json");
}

function getInstalledHookPath() {
  return valueAfter("--hook") ?? join(homedir(), ".skillbox", "hooks", "claude-skillbox-hook.mjs");
}

function findOrCreateGroup(groups, matcher) {
  const existing = groups.find((group) => {
    if (!group || typeof group !== "object" || !Array.isArray(group.hooks)) return false;
    if (matcher === undefined) return !("matcher" in group);
    return group.matcher === matcher;
  });
  if (existing) return existing;

  const group = { ...(matcher === undefined ? {} : { matcher }), hooks: [] };
  groups.push(group);
  return group;
}

function addCommandHook(settings, event, matcher, command) {
  if (settings.hooks === undefined) settings.hooks = {};
  if (!settings.hooks || typeof settings.hooks !== "object" || Array.isArray(settings.hooks)) {
    throw new Error("Claude settings.hooks must be an object when present");
  }

  const groups = settings.hooks[event] ?? [];
  if (!Array.isArray(groups)) throw new Error(`Claude settings.hooks.${event} must be an array`);
  settings.hooks[event] = groups;

  const group = findOrCreateGroup(groups, matcher);
  if (group.hooks.some((hook) => hook?.type === "command" && hook.command === command)) return false;
  group.hooks.push({ type: "command", command });
  return true;
}

function writeSettings(path, settings) {
  const directory = dirname(path);
  mkdirSync(directory, { recursive: true });
  const temporary = `${path}.${process.pid}.${Date.now()}.tmp`;
  const mode = existsSync(path) ? statSync(path).mode & 0o777 : 0o600;
  writeFileSync(temporary, `${JSON.stringify(settings, null, 2)}\n`, { mode });
  chmodSync(temporary, mode);
  renameSync(temporary, path);
}

function installHook(source, destination) {
  if (source === destination) return undefined;
  const directory = dirname(destination);
  mkdirSync(directory, { recursive: true, mode: 0o700 });
  chmodSync(directory, 0o700);

  const sourceContents = readFileSync(source);
  const existingContents = existsSync(destination) ? readFileSync(destination) : undefined;
  if (existingContents?.equals(sourceContents)) return undefined;

  const backup = existsSync(destination)
    ? `${destination}.bak-skillbox-${new Date().toISOString().replaceAll(/[:.]/g, "-")}`
    : undefined;
  if (backup) copyFileSync(destination, backup);

  const temporary = `${destination}.${process.pid}.${Date.now()}.tmp`;
  writeFileSync(temporary, sourceContents, { mode: 0o600 });
  chmodSync(temporary, 0o600);
  renameSync(temporary, destination);
  return backup;
}

export function installClaudeHook(
  settingsPath = getSettingsPath(),
  installedHookPath = getInstalledHookPath(),
) {
  const existed = existsSync(settingsPath);
  const original = existed ? readFileSync(settingsPath, "utf8") : "{}\n";
  let settings;
  try {
    settings = JSON.parse(original);
  } catch (error) {
    throw new Error(`Cannot parse Claude settings at ${settingsPath}: ${error.message}`);
  }
  if (!settings || typeof settings !== "object" || Array.isArray(settings)) {
    throw new Error(`Claude settings at ${settingsPath} must contain a JSON object`);
  }

  const changed = [
    addCommandHook(settings, "SessionStart", undefined, hookCommand("session-start", installedHookPath)),
    addCommandHook(settings, "PreToolUse", "Bash", hookCommand("pre-tool-use", installedHookPath)),
    addCommandHook(settings, "SessionEnd", undefined, hookCommand("session-end", installedHookPath)),
  ].some(Boolean);
  const hookBackup = installHook(hookPath, installedHookPath);
  if (!changed) return { changed: false, backup: undefined, hookBackup, settingsPath, installedHookPath };

  const backup = existed
    ? `${settingsPath}.bak-skillbox-${new Date().toISOString().replaceAll(/[:.]/g, "-")}`
    : undefined;
  if (backup) copyFileSync(settingsPath, backup);
  writeSettings(settingsPath, settings);
  return { changed: true, backup, hookBackup, settingsPath, installedHookPath };
}

function main() {
  if (args.includes("--help")) {
    console.log("Usage: node scripts/install-claude-hook.mjs [--settings <path>] [--hook <path>]");
    return;
  }
  for (let index = 0; index < args.length; index += 1) {
    if (args[index] !== "--settings" && args[index] !== "--hook") {
      throw new Error("Usage: node scripts/install-claude-hook.mjs [--settings <path>] [--hook <path>]");
    }
    if (!args[index + 1]) {
      throw new Error("Usage: node scripts/install-claude-hook.mjs [--settings <path>] [--hook <path>]");
    }
    index += 1;
  }

  const result = installClaudeHook();
  console.log(`installed Claude Code Skillbox hook at ${result.installedHookPath}`);
  if (result.hookBackup) console.log(`hook backup: ${result.hookBackup}`);
  if (result.changed) {
    console.log(`installed Claude Code Skillbox hooks in ${result.settingsPath}`);
    if (result.backup) console.log(`backup: ${result.backup}`);
  } else {
    console.log(`Claude Code Skillbox hooks already installed in ${result.settingsPath}`);
  }
}

try {
  main();
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error));
  process.exitCode = 1;
}
