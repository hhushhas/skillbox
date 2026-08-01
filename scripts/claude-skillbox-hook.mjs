#!/usr/bin/env node

import { createHash } from "node:crypto";
import {
  chmodSync,
  mkdirSync,
  readFileSync,
  renameSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const CLAUDE_HARNESS = "claude-code";
const SKILLBOX_COMMAND = /(?:^|[;&|()]+\s*)(?:env\s+)?(?:[A-Za-z_][A-Za-z0-9_]*=[^ \t;&|()]+\s+)*(?:(?:command)\s+|(?:sudo)(?:\s+[^ \t;&|()]+)*\s+)?(?:[^ \t;&|()'\"]+\/)?skillbox(?=$|[\s;&|()])/;

function nonEmptyString(value) {
  return typeof value === "string" && value.trim() ? value.trim() : undefined;
}

function shellQuote(value) {
  return `'${String(value).replaceAll("'", "'\\''")}'`;
}

function metadataDirectory() {
  return nonEmptyString(process.env.SKILLBOX_CLAUDE_METADATA_DIR)
    ?? join(homedir(), ".skillbox", "claude-sessions");
}

export function sessionMetadataPath(sessionId) {
  const digest = createHash("sha256").update(sessionId).digest("hex");
  return join(metadataDirectory(), `${digest}.json`);
}

function readSessionModel(sessionId) {
  try {
    const metadata = JSON.parse(readFileSync(sessionMetadataPath(sessionId), "utf8"));
    return nonEmptyString(metadata?.model);
  } catch {
    return undefined;
  }
}

function rememberSessionModel(payload) {
  const sessionId = nonEmptyString(payload?.session_id);
  if (!sessionId) return;

  const target = sessionMetadataPath(sessionId);
  const temporary = `${target}.${process.pid}.${Date.now()}.tmp`;
  mkdirSync(metadataDirectory(), { recursive: true, mode: 0o700 });
  chmodSync(metadataDirectory(), 0o700);

  try {
    writeFileSync(
      temporary,
      `${JSON.stringify({ model: nonEmptyString(payload?.model) })}\n`,
      { mode: 0o600 },
    );
    chmodSync(temporary, 0o600);
    renameSync(temporary, target);
  } finally {
    rmSync(temporary, { force: true });
  }
}

function forgetSessionModel(payload) {
  const sessionId = nonEmptyString(payload?.session_id);
  if (sessionId) rmSync(sessionMetadataPath(sessionId), { force: true });
}

export function isSkillboxCommand(command) {
  return SKILLBOX_COMMAND.test(command);
}

export function buildEnvironmentPrefix(metadata) {
  const values = [
    ["SKILLBOX_HARNESS", metadata.harness],
    ["SKILLBOX_THREAD_ID", metadata.threadId],
    ["SKILLBOX_MODEL_ID", metadata.modelId],
    ["SKILLBOX_TRANSCRIPT_PATH", metadata.transcriptPath],
  ].filter(([, value]) => nonEmptyString(value));

  return `export ${values.map(([key, value]) => `${key}=${shellQuote(value)}`).join(" ")};`;
}

export function patchSkillboxCommand(command, metadata) {
  if (!isSkillboxCommand(command) || command.includes("SKILLBOX_HARNESS=")) return command;
  return `${buildEnvironmentPrefix(metadata)}\n${command}`;
}

function patchPreToolUse(payload) {
  if (payload?.tool_name !== "Bash" || typeof payload?.tool_input?.command !== "string") {
    return {};
  }

  const sessionId = nonEmptyString(payload.session_id);
  if (!sessionId) return {};

  const command = payload.tool_input.command;
  const patchedCommand = patchSkillboxCommand(command, {
    harness: CLAUDE_HARNESS,
    threadId: sessionId,
    modelId: readSessionModel(sessionId),
    transcriptPath: nonEmptyString(payload.transcript_path),
  });
  if (patchedCommand === command) return {};

  return {
    hookSpecificOutput: {
      hookEventName: "PreToolUse",
      updatedInput: { ...payload.tool_input, command: patchedCommand },
    },
  };
}

export function handleHook(event, payload) {
  if (event === "session-start") {
    rememberSessionModel(payload);
    return {};
  }
  if (event === "session-end") {
    forgetSessionModel(payload);
    return {};
  }
  if (event === "pre-tool-use") return patchPreToolUse(payload);
  return {};
}

async function readStdin() {
  const chunks = [];
  for await (const chunk of process.stdin) chunks.push(chunk);
  return Buffer.concat(chunks).toString("utf8");
}

export async function main(argv = process.argv) {
  const event = argv[2];
  let payload = {};
  try {
    payload = JSON.parse(await readStdin());
  } catch {
    // Hook failures must not interrupt the original Claude command.
  }

  try {
    process.stdout.write(JSON.stringify(handleHook(event, payload)));
  } catch {
    process.stdout.write("{}");
  }
}

if (process.argv[1]) {
  try {
    if (import.meta.url === pathToFileURL(realpathSync(process.argv[1])).href) {
      await main();
    }
  } catch {
    // A missing entry path cannot be the main module.
  }
}
