import assert from "node:assert/strict";
import { chmodSync, mkdtempSync, readFileSync, rmSync, statSync } from "node:fs";
import test from "node:test";
import { tmpdir } from "node:os";
import { join } from "node:path";

import {
  buildEnvironmentPrefix,
  handleHook,
  isSkillboxCommand,
  patchSkillboxCommand,
  sessionMetadataPath,
} from "./claude-skillbox-hook.mjs";

test("Claude hook patches Skillbox Bash input with automatic session metadata", () => {
  const metadataRoot = mkdtempSync(join(tmpdir(), "skillbox-claude-hook-"));
  const previous = process.env.SKILLBOX_CLAUDE_METADATA_DIR;
  process.env.SKILLBOX_CLAUDE_METADATA_DIR = metadataRoot;

  try {
    const sessionId = "claude-session-test";
    handleHook("session-start", { session_id: sessionId, model: "claude-test-model" });
    const metadataPath = sessionMetadataPath(sessionId);
    assert.equal(statSync(metadataPath).mode & 0o777, 0o600);
    assert.deepEqual(JSON.parse(readFileSync(metadataPath, "utf8")), { model: "claude-test-model" });

    const result = handleHook("pre-tool-use", {
      session_id: sessionId,
      transcript_path: "/tmp/claude transcript.jsonl",
      tool_name: "Bash",
      tool_input: { command: "skillbox fetch browser", description: "fetch" },
    });
    const command = result.hookSpecificOutput.updatedInput.command;
    assert.match(command, /^export SKILLBOX_HARNESS='claude-code'/);
    assert.match(command, /SKILLBOX_THREAD_ID='claude-session-test'/);
    assert.match(command, /SKILLBOX_MODEL_ID='claude-test-model'/);
    assert.match(command, /SKILLBOX_TRANSCRIPT_PATH='\/tmp\/claude transcript\.jsonl'/);
    assert.match(command, /\nskillbox fetch browser$/);
    assert.equal(result.hookSpecificOutput.updatedInput.description, "fetch");

    handleHook("session-end", { session_id: sessionId });
    assert.throws(() => statSync(metadataPath), { code: "ENOENT" });
  } finally {
    if (previous === undefined) delete process.env.SKILLBOX_CLAUDE_METADATA_DIR;
    else process.env.SKILLBOX_CLAUDE_METADATA_DIR = previous;
    chmodSync(metadataRoot, 0o700);
    rmSync(metadataRoot, { recursive: true, force: true });
  }
});

test("Claude hook leaves unrelated or already bridged commands unchanged", () => {
  assert.equal(isSkillboxCommand("printf '%s\\n' 'skillbox'"), false);
  assert.equal(isSkillboxCommand("/Users/macmini/bin/skillbox list"), true);
  assert.equal(
    patchSkillboxCommand("export SKILLBOX_HARNESS='existing'; skillbox list", {
      harness: "claude-code",
      threadId: "thread",
    }),
    "export SKILLBOX_HARNESS='existing'; skillbox list",
  );
  assert.equal(
    buildEnvironmentPrefix({ harness: "pi", threadId: "thread's id" }),
    "export SKILLBOX_HARNESS='pi' SKILLBOX_THREAD_ID='thread'\\''s id';",
  );
});
