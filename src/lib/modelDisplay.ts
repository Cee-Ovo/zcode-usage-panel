/**
 * Display-layer model-name formatting.
 *
 * IMPORTANT: this is presentation-only. Raw model names must keep flowing
 * untouched into queries, IPC calls, Map keys, cost lookups and the store —
 * the (Codex) suffix exists purely so users can see where a model's numbers
 * came from. Only call with `source: "codex"` when the data provably
 * originates from the Codex provider snapshot (localUsage / its model rows);
 * never guess from the model name itself.
 */

export const CODEX_BADGE = "（Codex）";

export type ModelSource = "codex" | "zcode" | null;

/** Append the (Codex) badge for Codex-sourced models; idempotent. */
export function displayModelName(name: string, source: ModelSource = null): string {
  if (source !== "codex") return name;
  if (name.endsWith(CODEX_BADGE)) return name;
  return `${name}${CODEX_BADGE}`;
}
