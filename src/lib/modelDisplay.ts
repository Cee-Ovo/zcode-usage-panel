/**
 * Display-layer model-name formatting.
 *
 * IMPORTANT: this is presentation-only. Raw model names must keep flowing
 * untouched into queries, IPC calls, Map keys, cost lookups and the store —
 * the source badge exists purely so users can see where a model's numbers
 * came from. Only call with a matching `source` when the data provably
 * originates from that provider snapshot (localUsage / its model rows);
 * never guess from the model name itself.
 *
 * Single-source surfaces pass no source and stay unbadged; multi-source
 * surfaces (the merged Models page) pass every row's source, ZCode included,
 * so the same model name from two agents cannot be confused.
 */

export const ZCODE_BADGE = "（ZCode）";
export const CODEX_BADGE = "（Codex）";
export const DSH_BADGE = "（DSH）";
/** Short form: the dashboard tab and the model rows both label it "CC". */
export const CLAUDE_CODE_BADGE = "（CC）";

export type ModelSource = "codex" | "dsh" | "claude-code" | "zcode" | null;

const BADGES: Partial<Record<Exclude<ModelSource, null>, string>> = {
  zcode: ZCODE_BADGE,
  codex: CODEX_BADGE,
  dsh: DSH_BADGE,
  "claude-code": CLAUDE_CODE_BADGE,
};

/** Append the source badge for provider-sourced models; idempotent. */
export function displayModelName(name: string, source: ModelSource = null): string {
  const { name: bare, badge } = displayModelParts(name, source);
  return badge ? `${bare}${badge}` : bare;
}

/**
 * Same split as `displayModelName`, for layouts that truncate the name but
 * must keep the badge whole (the badge is the disambiguator — losing it to an
 * ellipsis defeats the point).
 */
export function displayModelParts(
  name: string,
  source: ModelSource = null,
): { name: string; badge: string | null } {
  const badge = source ? BADGES[source] : undefined;
  if (!badge) return { name, badge: null };
  return { name: name.endsWith(badge) ? name.slice(0, -badge.length) : name, badge };
}
