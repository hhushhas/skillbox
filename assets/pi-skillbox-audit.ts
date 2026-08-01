import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";

const SKILLBOX_COMMAND = /(?:^|[;&|()]+\s*)(?:env\s+)?(?:[A-Za-z_][A-Za-z0-9_]*=[^ \t;&|()]+\s+)*(?:(?:command)\s+|(?:sudo)(?:\s+[^ \t;&|()]+)*\s+)?(?:[^ \t;&|()'\"]+\/)?skillbox(?=$|[\s;&|()])/;

export interface SkillboxAuditMetadata {
  harness: string;
  threadId: string;
  modelId?: string;
  transcriptPath?: string;
}

interface BashToolCallEvent {
  type: "tool_call";
  toolName: "bash";
  input: { command: string };
}

function isBashToolCallEvent(event: unknown): event is BashToolCallEvent {
  if (!event || typeof event !== "object") return false;
  const candidate = event as Partial<BashToolCallEvent>;
  return candidate.type === "tool_call"
    && candidate.toolName === "bash"
    && typeof candidate.input?.command === "string";
}

function nonEmpty(value: string | undefined): string | undefined {
  const trimmed = value?.trim();
  return trimmed || undefined;
}

function shellQuote(value: string): string {
  return `'${value.replaceAll("'", "'\\''")}'`;
}

export function isSkillboxCommand(command: string): boolean {
  return SKILLBOX_COMMAND.test(command);
}

export function buildEnvironmentPrefix(metadata: SkillboxAuditMetadata): string {
  const values: Array<[string, string | undefined]> = [
    ["SKILLBOX_HARNESS", metadata.harness],
    ["SKILLBOX_THREAD_ID", metadata.threadId],
    ["SKILLBOX_MODEL_ID", metadata.modelId],
    ["SKILLBOX_TRANSCRIPT_PATH", metadata.transcriptPath],
  ];
  const assignments = values
    .filter(([, value]) => nonEmpty(value) !== undefined)
    .map(([key, value]) => `${key}=${shellQuote(value as string)}`)
    .join(" ");
  return `export ${assignments};`;
}

export function patchSkillboxCommand(command: string, metadata: SkillboxAuditMetadata): string {
  if (!isSkillboxCommand(command) || command.includes("SKILLBOX_HARNESS=")) return command;
  return `${buildEnvironmentPrefix(metadata)}\n${command}`;
}

export function currentSkillboxMetadata(
  ctx: Pick<ExtensionContext, "sessionManager" | "model">,
): SkillboxAuditMetadata {
  const modelId = ctx.model ? `${ctx.model.provider}/${ctx.model.id}` : undefined;
  return {
    harness: "pi",
    threadId: ctx.sessionManager.getSessionId(),
    modelId,
    transcriptPath: ctx.sessionManager.getSessionFile(),
  };
}

export default function skillboxAudit(pi: ExtensionAPI): void {
  pi.on("tool_call", (event, ctx) => {
    if (!isBashToolCallEvent(event)) return;
    const patchedCommand = patchSkillboxCommand(
      event.input.command,
      currentSkillboxMetadata(ctx),
    );
    if (patchedCommand !== event.input.command) event.input.command = patchedCommand;
  });
}
