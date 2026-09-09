/** Display-layer model naming: the （来源） badge must be suffix-only,
 *  idempotent, and never applied to unattributed (e.g. ZCode) models —
 *  raw names must stay untouched for queries/IPC/map keys. */

import { describe, expect, it } from "vitest";
import {
  CLAUDE_CODE_BADGE,
  CODEX_BADGE,
  DSH_BADGE,
  displayModelName,
} from "../src/lib/modelDisplay";

describe("displayModelName", () => {
  it("appends the Codex badge for Codex-sourced models", () => {
    expect(displayModelName("gpt-5.6-sol", "codex")).toBe("gpt-5.6-sol（Codex）");
    expect(displayModelName("gpt-5.6-luna", "codex")).toBe("gpt-5.6-luna（Codex）");
  });

  it("never double-appends the badge", () => {
    expect(displayModelName("gpt-5.6-sol（Codex）", "codex")).toBe("gpt-5.6-sol（Codex）");
  });

  it("passes ZCode and unattributed models through unchanged", () => {
    expect(displayModelName("gpt-5.6-sol", "zcode")).toBe("gpt-5.6-sol");
    expect(displayModelName("gpt-5.6-sol", null)).toBe("gpt-5.6-sol");
    // no guessing from the name itself — "codex" substring is not a source
    expect(displayModelName("codex-fast", "zcode")).toBe("codex-fast");
    expect(displayModelName("codex-fast", null)).toBe("codex-fast");
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
});
