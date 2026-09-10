/** The Models page merges four sources into one list; every row must keep
 *  the source it came from (badge + detail routing) and the list must stay
 *  ordered by display tokens. */

import { describe, expect, it } from "vitest";
import { enabledModelProviders, mergeModelRows } from "../src/lib/modelRows";
import type { Agg, ModelRow, UsageViewDto } from "../src/lib/types";

const agg = (tokens: number) =>
  ({ requests: 1, input: tokens, totalSum: tokens }) as unknown as Agg;

const row = (name: string, tokens: number): ModelRow => ({
  name,
  agg: agg(tokens),
  share: 0,
});

const view = (models: ModelRow[]) =>
  ({ dash: { models } }) as unknown as UsageViewDto;

describe("mergeModelRows", () => {
  it("keeps ZCode rows when no local source contributed", () => {
    const rows = mergeModelRows({ models: [row("glm-5.3", 10)] }, {});
    expect(rows).toHaveLength(1);
    expect(rows[0].source).toBe("zcode");
  });

  it("merges the enabled local sources and sorts by display tokens", () => {
    const rows = mergeModelRows({ models: [row("glm-5.3", 10)] }, {
      codex: view([row("gpt-5.6-sol", 300)]),
      dsh: view([row("deepseek-chat", 50)]),
    });
    expect(rows.map((r) => [r.source, r.row.name])).toEqual([
      ["codex", "gpt-5.6-sol"],
      ["dsh", "deepseek-chat"],
      ["zcode", "glm-5.3"],
    ]);
  });

  it("keeps the same model name from two agents as two distinct rows", () => {
    const rows = mergeModelRows({ models: [row("deepseek-chat", 20)] }, {
      dsh: view([row("deepseek-chat", 90)]),
    });
    expect(rows).toHaveLength(2);
    expect(rows.map((r) => r.source)).toEqual(["dsh", "zcode"]);
    expect(rows[0].row).not.toBe(rows[1].row);
  });

  it("tolerates missing dash / view payloads", () => {
    expect(mergeModelRows(null, {})).toEqual([]);
    expect(mergeModelRows(undefined, { claude: null } as never)).toEqual([]);
  });
});

describe("enabledModelProviders", () => {
  it("maps the settings flags, in section order", () => {
    expect(
      enabledModelProviders({ codexEnabled: true, dshEnabled: false, claudeCodeEnabled: true }),
    ).toEqual(["codex", "claude-code"]);
    expect(enabledModelProviders({})).toEqual([]);
    expect(enabledModelProviders(null)).toEqual([]);
  });
});
