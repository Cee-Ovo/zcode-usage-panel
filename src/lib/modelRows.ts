/**
 * Merge the four sources' model rows into the single list the Models page
 * renders. ZCode rows come from the dashboard slice; each enabled local
 * source contributes the rows of its own usage view. Every row keeps the
 * source it came from so the UI can tag it and route the detail lookup back
 * to the right backend store.
 */

import type { ModelRow, UsageViewDto } from "./types";
import { totalTokens } from "./types";
import type { ModelSource } from "./modelDisplay";

export const LOCAL_MODEL_PROVIDERS = ["codex", "dsh", "claude-code"] as const;
export type LocalModelProvider = (typeof LOCAL_MODEL_PROVIDERS)[number];

export type SourcedModelRow = {
  source: Exclude<ModelSource, null>;
  row: ModelRow;
};

/** Enabled-state per local source, straight off the settings payload. */
export function enabledModelProviders(providers: {
  codexEnabled?: boolean;
  dshEnabled?: boolean;
  claudeCodeEnabled?: boolean;
} | null | undefined): LocalModelProvider[] {
  if (!providers) return [];
  return LOCAL_MODEL_PROVIDERS.filter((p) =>
    p === "codex"
      ? providers.codexEnabled
      : p === "dsh"
        ? providers.dshEnabled
        : providers.claudeCodeEnabled,
  );
}

/** All sources' rows, ordered by display tokens (descending). */
export function mergeModelRows(
  dash: { models: ModelRow[] } | null | undefined,
  localViews: Partial<Record<LocalModelProvider, UsageViewDto | null | undefined>>,
): SourcedModelRow[] {
  const merged: SourcedModelRow[] = [];
  for (const row of dash?.models ?? []) merged.push({ source: "zcode", row });
  for (const provider of LOCAL_MODEL_PROVIDERS) {
    for (const row of localViews[provider]?.dash.models ?? []) {
      merged.push({ source: provider, row });
    }
  }
  return merged.sort((a, b) => totalTokens(b.row.agg) - totalTokens(a.row.agg));
}
