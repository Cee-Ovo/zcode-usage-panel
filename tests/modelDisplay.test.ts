/** Display-layer model naming: the （来源） badge must be suffix-only,
 *  idempotent, and only applied when the caller knows the source —
 *  raw names must stay untouched for queries/IPC/map keys. */

import { describe, expect, it } from "vitest";
import {
  CLAUDE_CODE_BADGE,
  CODEX_BADGE,
  DSH_BADGE,
  ZCODE_BADGE,
  displayModelName,
  displayModelParts,
} from "../src/lib/modelDisplay";

describe("displayModelName", () => {
  it("appends the Codex badge for Codex-sourced models", () => {
    expect(displayModelName("gpt-5.6-sol", "codex")).toBe("gpt-5.6-sol（Codex）");
    expect(displayModelName("gpt-5.6-luna", "codex")).toBe("gpt-5.6-luna（Codex）");
  });

  it("never double-appends the badge", () => {
    expect(displayModelName("gpt-5.6-sol（Codex）", "codex")).toBe("gpt-5.6-sol（Codex）");
  });

  it("tags ZCode rows on multi-source surfaces", () => {
    expect(displayModelName("glm-5.3", "zcode")).toBe(`glm-5.3${ZCODE_BADGE}`);
    expect(displayModelName(`glm-5.3${ZCODE_BADGE}`, "zcode")).toBe(`glm-5.3${ZCODE_BADGE}`);
  });

  it("passes unattributed models through unchanged", () => {
    expect(displayModelName("gpt-5.6-sol", null)).toBe("gpt-5.6-sol");
    // no guessing from the name itself — "codex" substring is not a source
    expect(displayModelName("codex-fast", null)).toBe("codex-fast");
    expect(displayModelName("codex-fast")).toBe("codex-fast");
  });

  it("appends the DSH and Claude Code badges for their sources", () => {
    expect(displayModelName("deepseek-chat", "dsh")).toBe(`deepseek-chat${DSH_BADGE}`);
    expect(displayModelName("deepseek-reasoner", "dsh")).not.toContain(CODEX_BADGE);
    expect(displayModelName("claude-sonnet-5", "claude-code")).toBe(
      `claude-sonnet-5${CLAUDE_CODE_BADGE}`,
    );
    // idempotent for the new badge as well
    expect(displayModelName(`claude-sonnet-5${CLAUDE_CODE_BADGE}`, "claude-code")).toBe(
      `claude-sonnet-5${CLAUDE_CODE_BADGE}`,
    );
    // a claude- model name alone is NOT a source attribution
    expect(displayModelName("claude-sonnet-5", null)).toBe("claude-sonnet-5");
  });

  it("default source is unattributed (no badge)", () => {
    expect(displayModelName("gpt-5.6-sol")).toBe("gpt-5.6-sol");
  });

  it("badge constant matches the visible suffix", () => {
    expect(displayModelName("m", "codex").endsWith(CODEX_BADGE)).toBe(true);
  });

  it("splits name and badge for layouts that truncate only the name", () => {
    expect(displayModelParts("gpt-5.6-sol", "codex")).toEqual({
      name: "gpt-5.6-sol",
      badge: CODEX_BADGE,
    });
    expect(displayModelParts("gpt-5.6-sol", null)).toEqual({ name: "gpt-5.6-sol", badge: null });
    // already-badged names must not nest a second badge
    expect(displayModelParts(`gpt-5.6-sol${CODEX_BADGE}`, "codex")).toEqual({
      name: "gpt-5.6-sol",
      badge: CODEX_BADGE,
    });
    expect(displayModelParts("glm-5.3", "zcode")).toEqual({
      name: "glm-5.3",
      badge: ZCODE_BADGE,
    });
  });
});
