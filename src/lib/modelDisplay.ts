/**
 * Display-layer model-name formatting.
 *
 * IMPORTANT: this is presentation-only. Raw model names must keep flowing
 * untouched into queries, IPC calls, Map keys, cost lookups and the store —
 * the source badge exists purely so users can see where a model's numbers
 * came from. Only call with a matching `source` when the data provably
 * originates from that provider snapshot (localUsage / its model rows);
 * never guess from the model name itself.
 */

export const CODEX_BADGE = "（Codex）";
export const DSH_BADGE = "（DSH）";
export const CLAUDE_CODE_BADGE = "（Claude Code）";

export type ModelSource = "codex" | "dsh" | "claude-code" | "zcode" | null;

const BADGES: Partial<Record<Exclude<ModelSource, null>, string>> = {
  codex: CODEX_BADGE,
  dsh: DSH_BADGE,
  "claude-code": CLAUDE_CODE_BADGE,
};

/** Append the source badge for provider-sourced models; idempotent. */
export function displayModelName(name: string, source: ModelSource = null): string {
  const badge = source ? BADGES[source] : undefined;
  if (!badge) return name;
  if (name.endsWith(badge)) return name;
  return `${name}${badge}`;
}
