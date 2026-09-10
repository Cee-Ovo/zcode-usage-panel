import type { AppState } from "./store";
import type {
  Agg,
  DashboardDto,
  ModelCost,
  ModelRow,
  ProviderSnapshot,
  SessionSummary,
  SessionsPageDto,
  Settings,
  TrendDto,
  UsageUpdateEvent,
  UsageViewDto,
  CostSummaryDto,
  LocalUsageRange,
  ModelUsageRow,
  TokenBreakdown,
} from "./types";

/**
 * DEV-only mock data (dev server / visual iteration without Tauri IPC).
 * Never imported by production builds — guarded by `import.meta.env.DEV`
 * at the call site, and the whole module is tree-shaken in `vite build`.
 */

const now = Date.now();
const hour = 3_600_000;

const mkAgg = (scale: number) => ({
  requests: Math.round(420 * scale),
  input: Math.round(8_400_000 * scale),
  output: Math.round(960_000 * scale),
  reasoning: { sum: Math.round(140_000 * scale), present: Math.round(420 * scale) },
  cacheRead: { sum: Math.round(3_100_000 * scale), present: Math.round(400 * scale) },
  cacheWrite: { sum: Math.round(210_000 * scale), present: Math.round(300 * scale) },
  // Inclusive mock schema: input already contains the cache; total excludes it.
  totalSum: Math.round(9_500_000 * scale),
  hitCached: Math.round(2_600_000 * scale),
  hitInputTotal: Math.round(8_400_000 * scale),
  firstTsMs: now - 12 * hour,
  lastTsMs: now - 5 * 60_000,
});

const agg = mkAgg(1);

const codexToday: TokenBreakdown = {
  requests: 96,
  inputTokens: 2_140_000,
  cachedInputTokens: 1_460_000,
  cacheWriteTokens: 88_000,
  outputTokens: 214_000,
  reasoningTokens: 41_200,
  totalTokens: 3_883_200,
};

const codex7d: TokenBreakdown = {
  requests: 512,
  inputTokens: 9_820_000,
  cachedInputTokens: 6_410_000,
  cacheWriteTokens: 402_000,
  outputTokens: 981_000,
  reasoningTokens: 190_400,
  totalTokens: 17_603_400,
};

const codexAll: TokenBreakdown = {
  requests: 2_240,
  inputTokens: 41_300_000,
  cachedInputTokens: 27_800_000,
  cacheWriteTokens: 1_720_000,
  outputTokens: 4_120_000,
  reasoningTokens: 802_000,
  totalTokens: 75_742_000,
};

const scaleCodexBreakdown = (value: TokenBreakdown, ratio: number): TokenBreakdown => ({
  requests: Math.round(value.requests * ratio),
  inputTokens: Math.round(value.inputTokens * ratio),
  cachedInputTokens: Math.round(value.cachedInputTokens * ratio),
  cacheWriteTokens: Math.round(value.cacheWriteTokens * ratio),
  outputTokens: Math.round(value.outputTokens * ratio),
  reasoningTokens: Math.round(value.reasoningTokens * ratio),
  totalTokens: Math.round(value.totalTokens * ratio),
});

const mockCodexModels = (breakdown: TokenBreakdown): ModelUsageRow[] => [
  { model: "gpt-5.6-sol", breakdown: scaleCodexBreakdown(breakdown, 0.68) },
  { model: "gpt-5.6-luna", breakdown: scaleCodexBreakdown(breakdown, 0.32) },
];

const mockCodexRanges: LocalUsageRange[] = [
  { key: "today", breakdown: codexToday, sessions: 12, models: mockCodexModels(codexToday) },
  {
    key: "60m",
    breakdown: scaleCodexBreakdown(codexToday, 0.08),
    sessions: 2,
    models: mockCodexModels(scaleCodexBreakdown(codexToday, 0.08)),
  },
  {
    key: "24h",
    breakdown: scaleCodexBreakdown(codexToday, 1.18),
    sessions: 15,
    models: mockCodexModels(scaleCodexBreakdown(codexToday, 1.18)),
  },
  { key: "7d", breakdown: codex7d, sessions: 46, models: mockCodexModels(codex7d) },
  {
    key: "30d",
    breakdown: scaleCodexBreakdown(codexAll, 0.54),
    sessions: 91,
    models: mockCodexModels(scaleCodexBreakdown(codexAll, 0.54)),
  },
  { key: "all", breakdown: codexAll, sessions: 132, models: mockCodexModels(codexAll) },
];

// ---- DSH (DeepSeek Harness) 模拟数据 -----------------------------------------
// `?dsh=missing` 把 DSH 快照切换为「未找到数据目录」空态,用于截图验收。

const dshToday: TokenBreakdown = {
  requests: 64,
  inputTokens: 1_180_000,
  cachedInputTokens: 720_000,
  cacheWriteTokens: 52_000,
  outputTokens: 168_000,
  reasoningTokens: 96_400,
  totalTokens: 2_120_000,
};

const dsh7d: TokenBreakdown = {
  requests: 388,
  inputTokens: 6_640_000,
  cachedInputTokens: 4_310_000,
  cacheWriteTokens: 288_000,
  outputTokens: 902_000,
  reasoningTokens: 512_300,
  totalTokens: 12_140_000,
};

const dshAll: TokenBreakdown = {
  requests: 1_460,
  inputTokens: 24_900_000,
  cachedInputTokens: 16_800_000,
  cacheWriteTokens: 1_120_000,
  outputTokens: 3_360_000,
  reasoningTokens: 1_902_000,
  totalTokens: 47_380_000,
};

const mockDshModels = (breakdown: TokenBreakdown): ModelUsageRow[] => [
  { model: "deepseek-chat", breakdown: scaleCodexBreakdown(breakdown, 0.61) },
  { model: "deepseek-reasoner", breakdown: scaleCodexBreakdown(breakdown, 0.39) },
];

const mockDshRanges: LocalUsageRange[] = [
  { key: "today", breakdown: dshToday, sessions: 7, models: mockDshModels(dshToday) },
  {
    key: "60m",
    breakdown: scaleCodexBreakdown(dshToday, 0.12),
    sessions: 1,
    models: mockDshModels(scaleCodexBreakdown(dshToday, 0.12)),
  },
  {
    key: "24h",
    breakdown: scaleCodexBreakdown(dshToday, 1.34),
    sessions: 9,
    models: mockDshModels(scaleCodexBreakdown(dshToday, 1.34)),
  },
  { key: "7d", breakdown: dsh7d, sessions: 31, models: mockDshModels(dsh7d) },
  {
    key: "30d",
    breakdown: scaleCodexBreakdown(dshAll, 0.47),
    sessions: 58,
    models: mockDshModels(scaleCodexBreakdown(dshAll, 0.47)),
  },
  { key: "all", breakdown: dshAll, sessions: 96, models: mockDshModels(dshAll) },
];

const dshMissing = (() => {
  try {
    return new URLSearchParams(window.location.search).get("dsh") === "missing";
  } catch {
    return false;
  }
})();

const ccMissing = (() => {
  try {
    return new URLSearchParams(window.location.search).get("cc") === "missing";
  } catch {
    return false;
  }
})();

// ---- 本地源(Codex / DSH / Claude Code)统一视图 mock --------------------------
// 与后端 get_local_usage_view 同构:同一套 ZCode 密度结构,数据按各源口径
// 造数(Codex inclusive、DSH/CC exclusive),速度字段恒为 null(诚实不可用)。

interface LocalSourceFixture {
  /** [model, share] — 模型与总量占比。 */
  models: [string, number][];
  /** 全量基数,按时间范围比例缩放。 */
  base: TokenBreakdown;
  sessions: number;
  /** 最近活跃 session(带前缀 id,与 mockState.sessions 对应)。 */
  latest: SessionSummary;
}

const RANGE_SCALE: Record<string, number> = {
  today: 0.14,
  "60m": 0.015,
  "24h": 0.22,
  "7d": 0.55,
  "30d": 0.82,
  all: 1,
};

const scaleBreakdown = (b: TokenBreakdown, ratio: number): TokenBreakdown => ({
  requests: Math.max(1, Math.round(b.requests * ratio)),
  inputTokens: Math.round(b.inputTokens * ratio),
  cachedInputTokens: Math.round(b.cachedInputTokens * ratio),
  cacheWriteTokens: Math.round(b.cacheWriteTokens * ratio),
  outputTokens: Math.round(b.outputTokens * ratio),
  reasoningTokens: Math.round(b.reasoningTokens * ratio),
  totalTokens: Math.round(b.totalTokens * ratio),
});

const mkLocalAgg = (b: TokenBreakdown): Agg => ({
  requests: b.requests,
  input: b.inputTokens,
  output: b.outputTokens,
  reasoning: { sum: b.reasoningTokens, present: b.reasoningTokens > 0 ? b.requests : 0 },
  cacheRead: { sum: b.cachedInputTokens, present: b.cachedInputTokens > 0 ? b.requests : 0 },
  cacheWrite: { sum: b.cacheWriteTokens, present: b.cacheWriteTokens > 0 ? b.requests : 0 },
  totalSum: b.totalTokens,
  hitCached: b.cachedInputTokens,
  hitInputTotal: b.inputTokens + b.cachedInputTokens + b.cacheWriteTokens,
  firstTsMs: now - 30 * 24 * hour,
  lastTsMs: now - 8 * 60_000,
});

const codexFixture: LocalSourceFixture = {
  models: [["gpt-5.6-sol", 0.68], ["gpt-5.6-luna", 0.32]],
  base: codexAll,
  sessions: 132,
  latest: {
    id: "cx-rollout-2026-09-09T21-04-33-a1b2c3d4",
    title: "Refactor quota cards with typed windows",
    project: "zcode-usage-panel",
    projectPath: "C:\\Users\\27632\\Desktop\\zcode-usage-panel",
    models: ["gpt-5.6-sol"],
    agg: mkLocalAgg(scaleCodexBreakdown(codexToday, 0.2)),
  },
};

const dshFixture: LocalSourceFixture = {
  models: [["deepseek-chat", 0.61], ["deepseek-reasoner", 0.39]],
  base: dshAll,
  sessions: 96,
  latest: {
    id: "dsh-sess-88f2ec41",
    title: null,
    project: null,
    projectPath: null,
    models: ["deepseek-reasoner"],
    agg: mkLocalAgg(scaleCodexBreakdown(dshToday, 0.25)),
  },
};

const claudeBase: TokenBreakdown = {
  // Claude 官方口径:input 不含 cache(读/写单列),无独立 reasoning 字段。
  requests: 1_840,
  inputTokens: 18_600_000,
  cachedInputTokens: 52_400_000,
  cacheWriteTokens: 3_120_000,
  outputTokens: 2_940_000,
  reasoningTokens: 0,
  totalTokens: 77_060_000,
};

const claudeFixture: LocalSourceFixture = {
  models: [["claude-sonnet-5", 0.71], ["claude-opus-5", 0.22], ["claude-haiku-5", 0.07]],
  base: claudeBase,
  sessions: 158,
  latest: {
    id: "cc-9f3a2b71-1111-2222-3333-444455556666",
    title: "会话压缩摘要标题(来自 summary 行)",
    project: "zcode-usage-panel",
    projectPath: "C:\\Users\\27632\\Desktop\\zcode-usage-panel",
    models: ["claude-sonnet-5"],
    agg: mkLocalAgg(scaleCodexBreakdown(claudeBase, 0.18)),
  },
};

const LOCAL_SOURCES: Record<string, LocalSourceFixture> = {
  codex: codexFixture,
  dsh: dshFixture,
  "claude-code": claudeFixture,
};

function mockLocalUsageView(provider: string, rangeKey: string, includeTrend: boolean): UsageViewDto {
  const fixture = LOCAL_SOURCES[provider] ?? codexFixture;
  const scale = RANGE_SCALE[rangeKey] ?? 1;
  const breakdown = scaleBreakdown(fixture.base, scale);
  const nullSpeed = {
    ttftAvgMs: null, ttftP50Ms: null, ttftP95Ms: null, ttftSamples: 0,
    speedTps: null, speedP50Tps: null, speedSamples: 0,
    completedRequests: breakdown.requests, generatedTokens: 0, generationMs: 0,
  };
  // Codex rollouts carry per-line timestamps → approximate tok/s (whole-
  // request window, includes first-token wait); TTFT stays honestly null.
  const codexApproxSpeed = provider === "codex"
    ? {
        ...nullSpeed,
        speedTps: 21.6, speedP50Tps: 19.4,
        speedSamples: Math.max(1, Math.round(breakdown.requests * 0.82)),
        speedApproximate: true,
        generatedTokens: Math.round(breakdown.outputTokens * 0.9),
        generationMs: Math.round((breakdown.outputTokens * 0.9) / 21.6) * 1000,
      }
    : nullSpeed;
  const mkRows = (b: TokenBreakdown): ModelRow[] =>
    fixture.models.map(([name, share]) => ({
      name,
      agg: {
        ...mkLocalAgg(scaleBreakdown(b, share)),
        requests: Math.max(1, Math.round(b.requests * share)),
      },
      share,
      speed: codexApproxSpeed,
    }));
  const latestAgg = mkLocalAgg(scaleBreakdown(fixture.base, 0.18));
  const dash: DashboardDto = {
    rangeKey,
    fromMs: now - 24 * hour,
    toMs: now,
    agg: mkLocalAgg(breakdown),
    models: mkRows(breakdown),
    activeSession: {
      sessionId: fixture.latest.id,
      project: fixture.latest.project,
      sessionTotalTokens: latestAgg.totalSum ?? 0,
      sessionAgg: latestAgg,
      tokensLast5m: Math.round(breakdown.totalTokens * 0.01),
      tokensPerMin: Math.round(breakdown.totalTokens * 0.01) / 5,
      lastRequestMs: now - 8 * 60_000,
      activeModel: fixture.latest.models[0],
      modelSwitches: [
        { tsMs: now - 2 * hour, model: fixture.models[0][0] },
        { tsMs: now - 0.6 * hour, model: fixture.models[1][0] },
      ],
    },
    speed: codexApproxSpeed,
    restored: false,
    dataError: null,
  };
  const bucketCount = rangeKey === "60m" || rangeKey === "today" || rangeKey === "24h" ? 12 : 16;
  const trend: TrendDto | null = includeTrend
    ? {
        rangeKey,
        fromMs: now - 24 * hour,
        toMs: now,
        buckets: Array.from({ length: bucketCount }, (_, i) => {
          const wobble = 0.4 + 0.6 * Math.abs(Math.sin(i * 1.3));
          const per = scaleBreakdown(breakdown, (wobble / bucketCount) * 1.6);
          const byModel: Record<string, Agg> = {};
          for (const [name, share] of fixture.models) {
            byModel[name] = mkLocalAgg(scaleBreakdown(per, share));
          }
          return {
            startMs: now - 24 * hour + (i * 24 * hour) / bucketCount,
            endMs: now - 24 * hour + ((i + 1) * 24 * hour) / bucketCount,
            agg: mkLocalAgg(per),
            byModel,
          };
        }),
        restored: false,
      }
    : null;
  const models: ModelCost[] = fixture.models.map(([name], i) => ({
    name,
    costCny: Math.round((breakdown.totalTokens / 1e6) * [22, 66, 3][i % 3] * 100) / 100,
    priced: true,
  }));
  const costSummary: CostSummaryDto = {
    range: rangeKey,
    totalTokens: breakdown.totalTokens,
    totalCostCny: Math.round(models.reduce((s, m) => s + m.costCny, 0) * 100) / 100,
    fullyPriced: true,
    models,
    unknownModels: [],
    fx: { usdCny: 7.16, updatedAt: "2026-09-01", source: "frankfurter.dev" },
    priceUpdatedAt: "2026-08-30",
    disclaimer: "按官方 API 单价估算 · 非实际 Billing",
  };
  return { dash, trend, costSummary, revision: breakdown.requests };
}

export const mockState: Partial<AppState> = {
  ready: true,
  version: "1.2.0-dev",
  page: "dashboard",
  rangeKey: "today",
  dash: {
    rangeKey: "today",
    fromMs: now - 12 * hour,
    toMs: now,
    agg,
    models: [
      {
        name: "glm-5.3",
        agg: mkAgg(0.6),
        share: 0.58,
        speed: {
          ttftAvgMs: 1_240,
          ttftP50Ms: 980,
          ttftP95Ms: 2_410,
          ttftSamples: 96,
          speedTps: 82.4,
          speedP50Tps: 86.1,
          speedSamples: 96,
          completedRequests: 118,
          generatedTokens: 46_200,
          generationMs: 561_000,
        },
      },
      {
        name: "glm-5.3-air",
        agg: mkAgg(0.3),
        share: 0.29,
        speed: {
          ttftAvgMs: 2_080,
          ttftP50Ms: 1_740,
          ttftP95Ms: 4_320,
          ttftSamples: 48,
          speedTps: 54.7,
          speedP50Tps: 52.2,
          speedSamples: 48,
          completedRequests: 61,
          generatedTokens: 18_400,
          generationMs: 336_000,
        },
      },
      {
        name: "deepseek-v4",
        agg: mkAgg(0.08),
        share: 0.08,
        speed: {
          ttftAvgMs: 860,
          ttftP50Ms: 720,
          ttftP95Ms: 1_630,
          ttftSamples: 22,
          speedTps: 121.5,
          speedP50Tps: 118.6,
          speedSamples: 22,
          completedRequests: 27,
          generatedTokens: 8_200,
          generationMs: 67_490,
        },
      },
      {
        name: "kimi-k2.5",
        agg: mkAgg(0.02),
        share: 0.05,
        speed: {
          ttftAvgMs: null,
          ttftP50Ms: null,
          ttftP95Ms: null,
          ttftSamples: 0,
          speedTps: null,
          speedP50Tps: null,
          speedSamples: 0,
          completedRequests: 4,
          generatedTokens: 0,
          generationMs: 0,
        },
      },
    ],
    activeSession: {
      sessionId: "dev-session",
      project: "zcode-usage-panel",
      sessionTotalTokens: 412_300,
      sessionAgg: mkAgg(0.05),
      tokensLast5m: 8_400,
      tokensPerMin: 1_680,
      lastRequestMs: now - 40_000,
      activeModel: "glm-5.3",
      modelSwitches: [
        { tsMs: now - 3 * hour, model: "glm-5.3-air" },
        { tsMs: now - 1.2 * hour, model: "glm-5.3" },
      ],
    },
    restored: false,
    dataError: null,
    speed: {
      ttftAvgMs: 1_873,
      ttftP50Ms: 1_420,
      ttftP95Ms: 2_633,
      ttftSamples: 112,
      speedTps: 74.6,
      speedP50Tps: 76.2,
      speedSamples: 112,
      completedRequests: 164,
      generatedTokens: 71_240,
      generationMs: 954_000,
    },
  } satisfies DashboardDto,
  trend: {
    rangeKey: "today",
    fromMs: now - 12 * hour,
    toMs: now,
    buckets: Array.from({ length: 12 }, (_, i) => ({
      startMs: now - 12 * hour + i * hour,
      endMs: now - 11 * hour + i * hour,
      agg: mkAgg(0.04 + 0.02 * Math.sin(i)),
      byModel: { "glm-5.3": mkAgg(0.03), "glm-5.3-air": mkAgg(0.012) },
    })),
    restored: false,
  } satisfies TrendDto,
  costSummary: {
    range: "today",
    totalTokens: 12_810_000,
    totalCostCny: 43.21,
    fullyPriced: true,
    models: [
      { name: "glm-5.3", costCny: 32.1, priced: true },
      { name: "glm-5.3-air", costCny: 9.8, priced: true },
      { name: "deepseek-v4", costCny: 1.31, priced: true },
      { name: "kimi-k2.5", costCny: 0, priced: false },
    ],
    unknownModels: ["kimi-k2.5"],
    fx: { usdCny: 7.16, updatedAt: "2026-09-01", source: "frankfurter.dev" },
    priceUpdatedAt: "2026-08-30",
    disclaimer: "按官方 API 单价估算 · 非实际 Billing",
  } satisfies CostSummaryDto,
  // 会话名/项目来自 ZCode CLI session 表的真实样例(标题为实际生成标题);
  // 其余三源带显示前缀:cx-Codex、cc-Claude Code、dsh-DSH。
  sessions: [
    {
      id: "sess_6db49f3e-4cac-4298-b3e3-6a713d3a3356",
      title: "Sessions 页表格改造与会话数据补全",
      project: "zcode-usage-panel",
      projectPath: "/home/cee/projects/zcode-usage-panel",
      models: ["glm-5.3", "glm-5.3-air"],
      agg: mkAgg(0.42),
    },
    {
      id: "cc-9f3a2b71-1111-2222-3333-444455556666",
      title: "会话压缩摘要标题(来自 summary 行)",
      project: "zcode-usage-panel",
      projectPath: "C:\\Users\\27632\\Desktop\\zcode-usage-panel",
      models: ["claude-sonnet-5"],
      agg: mkLocalAgg(scaleCodexBreakdown(claudeBase, 0.18)),
    },
    {
      id: "cx-rollout-2026-09-09T21-04-33-a1b2c3d4",
      title: "Refactor quota cards with typed windows",
      project: "zcode-usage-panel",
      projectPath: "C:\\Users\\27632\\Desktop\\zcode-usage-panel",
      models: ["gpt-5.6-sol"],
      agg: mkLocalAgg(scaleCodexBreakdown(codexToday, 0.2)),
    },
    {
      id: "sess_ccdd18d8-81c4-4647-bed4-a7a98dbe4df3",
      title: "节点小宝远程屏幕控制连接失败排查",
      project: "default",
      projectPath: "/home/cee/.zcode/workspace/default",
      models: ["glm-5.3"],
      agg: mkAgg(0.2),
    },
    {
      id: "dsh-sess-88f2ec41",
      // DSH 日志无可核实的标题/项目数据 → 诚实降级为 "—"。
      title: null,
      project: null,
      projectPath: null,
      models: ["deepseek-reasoner"],
      agg: mkLocalAgg(scaleCodexBreakdown(dshToday, 0.25)),
    },
    {
      id: "cc-1a2b3c4d-5678-90ab-cdef-1234567890ab",
      title: "帮我把爬虫改成异步并加限流重试",
      project: "crawler_Xianyu",
      projectPath: "C:\\Users\\27632\\Desktop\\crawler_Xianyu",
      models: ["claude-opus-5", "claude-haiku-5"],
      agg: mkLocalAgg(scaleCodexBreakdown(claudeBase, 0.07)),
    },
    {
      id: "sess_0b7b195c-0a19-42be-b946-c63711cb4db0",
      title: "ZCode仪表盘速度指标与DSH三分区改造",
      project: "zcode-usage-panel",
      projectPath: "/home/cee/projects/zcode-usage-panel",
      models: ["glm-5.3", "glm-5.3-flash"],
      agg: mkAgg(0.31),
    },
    {
      id: "cx-rollout-2026-08-30T09-15-02-9988776655",
      // archived_sessions 里的历史 Codex 会话:无用户消息 → 无标题,诚实降级。
      title: null,
      project: "qwen_agent",
      projectPath: "C:\\Users\\27632\\Desktop\\qwen_agent",
      models: ["gpt-5.6-luna"],
      agg: mkLocalAgg(scaleCodexBreakdown(codexAll, 0.05)),
    },
    {
      id: "sess_9c21e0e9-47b8-4528-b9ba-caee37ab5552",
      title: "GTX 1650 部署优化 4B 模型达到 40 tok/s",
      project: "qwen_agent",
      projectPath: "/home/cee/projects/qwen_agent",
      models: ["glm-5.3-flash"],
      agg: mkAgg(0.14),
    },
    {
      // 无 session 元数据行的会话:诚实降级为 "—",禁止编造。
      id: "sess_0cfc7b4a-50c1-435f-aec3-29de1fb62e2d",
      title: null,
      project: null,
      projectPath: null,
      models: ["deepseek-v4-flash"],
      agg: mkAgg(0.06),
    },
  ] satisfies SessionSummary[],
  alerts: [
    {
      rule: "spike",
      severity: 1,
      title: "10 分钟激增",
      body: "近 10 分钟消耗 3.2M tokens(基线均值的 4.1 倍)",
      tsMs: now - 20 * 60_000,
    },
  ],
  update: {
    recordCount: 18_402,
    lastRefreshMs: now - 4_000,
    lastRecordMs: now - 40_000,
    error: null,
    errorStreak: 0,
    paused: false,
    suspended: false,
    restoredFromCache: false,
  } satisfies UsageUpdateEvent,
  providers: [
    {
      provider: "zcode",
      status: "ok",
      account: null,
      planName: null,
      windows: [
        {
          key: "today_tokens",
          label: "今日",
          usedPercent: null,
          totalQuota: null,
          usedQuota: 12_810_000,
          remainingQuota: null,
          unit: "tokens",
          resetAtMs: null,
          windowMinutes: null,
          forecast: null,
        },
      ],
      packages: [],
      localUsage: null,
      launcher: {
        state: "running",
        exePath: "C:\\Users\\dev\\AppData\\Local\\Programs\\ZCode\\ZCode.exe",
        version: "1.8.2",
        detectedVia: "registry",
      },
      source: "ZCode 本地 usage 记录",
      sourceUrl: null,
      notes: [],
      error: null,
      updatedAtMs: now - 4_000,
      nextPollMs: now + 60_000,
    },
    {
      provider: "codex",
      status: "ok",
      account: "dev@example.com",
      planName: "ChatGPT Plus",
      windows: [
        {
          key: "5h",
          label: "5 小时窗口",
          usedPercent: 41,
          totalQuota: null,
          usedQuota: null,
          remainingQuota: null,
          unit: "% 套餐额度",
          resetAtMs: now + 2.2 * hour,
          windowMinutes: 300,
          forecast: null,
        },
        {
          key: "weekly",
          label: "周额度",
          usedPercent: 63,
          totalQuota: null,
          usedQuota: null,
          remainingQuota: null,
          unit: "% 套餐额度",
          resetAtMs: now + 3.4 * 24 * hour,
          windowMinutes: null,
          forecast: {
            etaMs: 2.6 * 24 * hour,
            ratePerDay: 11.4,
            samples: 6,
            confidence: "low",
          },
        },
      ],
      packages: [],
      localUsage: {
        today: codexToday,
        last7d: codex7d,
        allTime: codexAll,
        sessions: 132,
        models: mockCodexModels(codexAll),
        ranges: mockCodexRanges,
      },
      launcher: null,
      source: "Codex 本地 session 文件(离线)",
      sourceUrl: "https://developers.openai.com/codex/rate-limits",
      notes: ["额度与本地用量来自不同数据源,相互独立。"],
      error: null,
      updatedAtMs: now - 12_000,
      nextPollMs: now + 120_000,
    },
    {
      provider: "dsh",
      status: "ok",
      account: null,
      planName: null,
      windows: [],
      packages: [],
      localUsage: dshMissing
        ? null
        : {
            today: dshToday,
            last7d: dsh7d,
            allTime: dshAll,
            sessions: 96,
            models: mockDshModels(dshAll),
            ranges: mockDshRanges,
          },
      launcher: null,
      source: "DeepSeek Harness 本地 session 日志(离线读取)",
      sourceUrl: "https://www.deepseek.com/harness/",
      notes: dshMissing
        ? []
        : ["reasoning 已含在 Output 中,总量不重复累计。", "session 日志统计 · 不计入 ZCode 总 Token"],
      error: dshMissing
        ? "未检测到 DeepSeek Harness 数据目录(默认 ~/.dsh;可在「设置 → DSH」指定路径)"
        : null,
      updatedAtMs: now - 18_000,
      nextPollMs: now + 60_000,
    },
    {
      // Claude Code:无本地可查的官方套餐额度 → 只有本地日志统计,不编造额度。
      provider: "claude-code",
      status: ccMissing ? "not_installed" : "ok",
      account: null,
      planName: null,
      windows: [],
      packages: [],
      localUsage: ccMissing
        ? null
        : {
            today: scaleCodexBreakdown(claudeBase, 0.14),
            last7d: scaleCodexBreakdown(claudeBase, 0.55),
            allTime: claudeBase,
            sessions: 158,
            models: [
              { model: "claude-sonnet-5", breakdown: scaleCodexBreakdown(claudeBase, 0.71) },
              { model: "claude-opus-5", breakdown: scaleCodexBreakdown(claudeBase, 0.22) },
              { model: "claude-haiku-5", breakdown: scaleCodexBreakdown(claudeBase, 0.07) },
            ],
            ranges: (["today", "60m", "24h", "7d", "30d", "all"] as const).map((key) => ({
              key,
              breakdown: scaleCodexBreakdown(claudeBase, RANGE_SCALE[key]),
              sessions: Math.max(1, Math.round(158 * RANGE_SCALE[key])),
              models: [
                { model: "claude-sonnet-5", breakdown: scaleCodexBreakdown(claudeBase, 0.71 * RANGE_SCALE[key]) },
                { model: "claude-opus-5", breakdown: scaleCodexBreakdown(claudeBase, 0.22 * RANGE_SCALE[key]) },
                { model: "claude-haiku-5", breakdown: scaleCodexBreakdown(claudeBase, 0.07 * RANGE_SCALE[key]) },
              ],
            })),
          },
      launcher: null,
      source: "Claude Code 本地 session 转写(离线读取)",
      sourceUrl: "https://code.claude.com/docs/",
      notes: ccMissing
        ? []
        : [
            "input 不含 cache(读/写单列),总量 = Input + Output + Cache 读 + Cache 写。",
            "同一 message.id 的流式重复行按最后一条计数。",
            "session 转写统计 · 不计入 ZCode 总 Token",
          ],
      error: ccMissing
        ? "未检测到 Claude Code 数据目录(默认 ~/.claude/projects;可在「设置 → Claude Code」指定路径或设置 CLAUDE_CONFIG_DIR)"
        : null,
      updatedAtMs: now - 14_000,
      nextPollMs: now + 60_000,
    },
    {
      provider: "antigravity",
      status: "not_configured",
      account: null,
      planName: null,
      windows: [],
      packages: [],
      localUsage: null,
      launcher: null,
      source: "Antigravity 本地 RPC",
      sourceUrl: null,
      notes: [],
      error: null,
      updatedAtMs: now - 60_000,
      nextPollMs: 0,
    },
  ] satisfies ProviderSnapshot[],
  settings: {
    dataDir: null,
    refreshDebounceMs: 800,
    defaultRange: "today",
    theme: "light",
    alwaysOnTop: false,
    monitoringPaused: false,
    closeToTray: true,
    autostart: false,
    pricingRemoteUrl: null,
    snap: {
      enabled: true,
      autoHide: true,
      thresholdPx: 24,
      hideDelayMs: 600,
      animMs: 200,
      sides: { left: true, right: true, top: false },
    },
    notifications: {
      enabled: true,
      spikeMultiplier: 4,
      spikeMinTokens: 1_000_000,
      sessionTotalTokens: 10_000_000,
      cacheHitDrop: 0.2,
      cacheMinRequests: 20,
      modelBurstPer5m: 100,
      stalenessMinutes: 30,
    },
    window: { x: 80, y: 80, width: 1180, height: 760, maximized: false, dockSide: null, dockHidden: false },
    providers: {
      codexEnabled: true,
      codexHome: null,
      codexRefreshMs: 300_000,
      dshEnabled: true,
      dshHome: null,
      dshRefreshMs: 300_000,
      claudeCodeEnabled: true,
      claudeCodeHome: null,
      claudeCodeRefreshMs: 300_000,
      antigravityEnabled: true,
      antigravityRefreshMs: 600_000,
      volcengineEnabled: false,
      volcengineRefreshMs: 1_800_000,
      volcengineRegion: "cn-beijing",
      volcengineFilter: "Token",
    },
    launcher: { enabled: true, exePath: null, autostart: false },
    quotaAlerts: { enabled: true, thresholds: [50, 20, 10], packageExpiryDays: 7, dailyCostCny: 50 },
  } satisfies Settings,
};

/** Browser-only equivalent of the paged sessions query. */
export function mockSessionsPage(
  query = "",
  sort: "recent" | "tokens" = "recent",
  page = 0,
  pageSize = 50,
): SessionsPageDto {
  const normalized = query.trim().toLocaleLowerCase();
  const all = (mockState.sessions ?? []).filter((s) => {
    if (!normalized) return true;
    return [s.id, s.title ?? "", s.project ?? "", s.projectPath ?? "", ...s.models]
      .join("\u0000")
      .toLocaleLowerCase()
      .includes(normalized);
  });
  all.sort((a, b) => {
    const primary = sort === "tokens"
      ? totalSessionTokens(b) - totalSessionTokens(a)
      : (b.agg.lastTsMs ?? -Infinity) - (a.agg.lastTsMs ?? -Infinity);
    return primary || a.id.localeCompare(b.id);
  });
  const size = Math.min(100, Math.max(1, Math.floor(pageSize) || 50));
  const safePage = Math.max(0, Math.floor(page) || 0);
  return {
    items: all.slice(safePage * size, (safePage + 1) * size),
    total: all.length,
    page: safePage,
    pageSize: size,
  };
}

export const mockHistoryHealth = {
  persistent: true,
  error: null,
  lastSuccessMs: now - 4_000,
};

function totalSessionTokens(s: SessionSummary): number {
  return s.agg.input + s.agg.output + s.agg.reasoning.sum + s.agg.cacheRead.sum + s.agg.cacheWrite.sum;
}

/** Minimal browser IPC adapter used only by the Vite development preview. */
export async function mockInvoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  const state = mockState;
  switch (cmd) {
    case "get_usage_view": return {
      dash: { ...state.dash, rangeKey: args.rangeKey ?? "today" },
      trend: args.includeTrend ? { ...state.trend, rangeKey: args.rangeKey ?? "today" } : null,
      costSummary: { ...state.costSummary, range: args.rangeKey ?? "today" },
      revision: state.update?.recordCount ?? 0,
    } as T;
    case "get_local_usage_view":
      return mockLocalUsageView(
        String(args.provider ?? "codex"),
        String(args.rangeKey ?? "today"),
        args.includeTrend !== false,
      ) as T;
    case "get_bootstrap":
      return { settings: state.settings, version: state.version ?? "1.2.0-dev", configDir: null, cacheDir: null } as T;
    case "get_dashboard": return {
      ...state.dash,
      rangeKey: String(args.rangeKey ?? state.dash?.rangeKey ?? "today"),
    } as T;
    case "get_trend": return {
      ...state.trend,
      rangeKey: String(args.rangeKey ?? state.trend?.rangeKey ?? "today"),
    } as T;
    case "get_sessions": return (state.sessions ?? []) as T;
    case "get_sessions_page": return mockSessionsPage(
      String(args.query ?? ""),
      args.sort === "tokens" ? "tokens" : "recent",
      Number(args.page ?? 0),
      Number(args.pageSize ?? 50),
    ) as T;
    case "get_session_detail": {
      const summary = state.sessions?.find((s) => s.id === String(args.sessionId));
      if (!summary) return null as T;
      // 8 个时间桶的趋势,便于视觉验证详情弹窗。
      const from = summary.agg.firstTsMs ?? now - 2 * hour;
      const to = summary.agg.lastTsMs ?? now;
      const span = Math.max(1, to - from);
      const buckets = Array.from({ length: 8 }, (_, i) => {
        const share = 0.05 + 0.2 * Math.abs(Math.sin(i * 1.7));
        const agg = { ...summary.agg, requests: Math.max(1, Math.round(summary.agg.requests * share)) };
        const byModel: Record<string, Agg> = {};
        summary.models.forEach((m, j) => {
          byModel[m] = { ...agg, requests: Math.max(1, Math.round((agg.requests / summary.models.length) * (j === 0 ? 1.4 : 0.6))) };
        });
        return {
          startMs: from + (i * span) / 8,
          endMs: from + ((i + 1) * span) / 8,
          agg,
          byModel,
        };
      });
      const models = summary.models.map((m) => ({
        name: m,
        agg: { ...summary.agg, requests: Math.max(1, Math.round(summary.agg.requests / summary.models.length)) },
      }));
      return { summary, buckets, models } as T;
    }
    case "get_model_detail": {
      const name = String(args.name ?? "glm-5.3");
      const source = typeof args.provider === "string" && args.provider ? args.provider : "zcode";
      // Local sources resolve against their own mock view, mirroring the
      // backend's provider-scoped lookup.
      const pool =
        source === "zcode"
          ? state.dash?.models
          : mockLocalUsageView(source, String(args.rangeKey ?? "today"), false).dash.models;
      const model = pool?.find((entry) => entry.name === name) ?? pool?.[0] ?? state.dash?.models?.[0];
      const modelAgg = model?.agg ?? state.dash?.agg;
      const total = modelAgg ? modelAgg.input + modelAgg.output + modelAgg.reasoning.sum + modelAgg.cacheRead.sum + modelAgg.cacheWrite.sum : 0;
      return {
        name,
        source,
        today: modelAgg,
        last7d: modelAgg,
        last30d: modelAgg,
        allTime: modelAgg,
        avgTokensPerRequest: modelAgg && modelAgg.requests > 0 ? total / modelAgg.requests : 0,
        hitRate: modelAgg && modelAgg.hitInputTotal > 0 ? modelAgg.hitCached / modelAgg.hitInputTotal : null,
        lastUsedMs: modelAgg?.lastTsMs ?? null,
        trend30d: state.trend?.buckets ?? [],
        topSessions: (state.sessions ?? []).slice(0, 10).map((session) => [session.id, totalSessionTokens(session)]),
      } as T;
    }
    case "get_alerts": return (state.alerts ?? []) as T;
    case "get_active_models": return ["glm-5.3", "glm-5.3-air", "deepseek-v4"] as T;
    case "cost_summary": return { ...state.costSummary, range: String(args.range ?? state.costSummary?.range ?? "today") } as T;
    case "providers_overview": return (state.providers ?? []) as T;
    case "quota_alerts_list": return (state.quotaAlerts ?? []) as T;
    case "providers_history": return [] as T;
    case "providers_consumption": return [] as T;
    case "history_health": return mockHistoryHealth as T;
    case "set_settings": return args.newSettings as T;
    case "refresh_now":
    case "providers_refresh":
    case "pricing_refresh": return { ok: true, fxOk: true, error: null, refreshedAt: new Date().toISOString() } as T;
    case "diagnose": return {
      root: null, rootSource: "mock", jsonlFiles: [], sqliteFiles: [], untrackedJsonl: 0,
      untrackedSqlite: 0, notes: [], recordCount: state.update?.recordCount ?? 0,
      lastRefreshMs: state.update?.lastRefreshMs ?? null, error: null, recentRecords: [],
    } as T;
    case "pricing_table": return {
      entries: [], unknownModels: [], fx: { usdCny: 7.16, updatedAt: "2026-09-01", source: "mock" },
      remoteUrl: null, lastRefresh: null, lastError: null,
    } as T;
    case "cost_detail": {
      if (args.provider) {
        const view = mockLocalUsageView(String(args.provider), String(args.range ?? "today"), false);
        const model = String(args.model ?? "");
        const cost = view.costSummary.models.find((m) => m.name === model);
        const input = Math.round(view.dash.agg.input / Math.max(1, view.costSummary.models.length));
        return {
          model,
          priced: !!cost?.priced,
          notes: ["本地源费用按官方 API 单价估算 · 非实际 Billing"],
          totalCny: cost?.costCny ?? 0,
          lines: [
            { key: "input", label: "Input", tokens: input, perM: 3, currency: "USD" as const, tier: null, costCny: Math.round((cost?.costCny ?? 0) * 0.42 * 100) / 100, includedIn: null },
            { key: "cache_read", label: "Cache 读", tokens: Math.round(input * 2.6), perM: 0.3, currency: "USD" as const, tier: null, costCny: Math.round((cost?.costCny ?? 0) * 0.08 * 100) / 100, includedIn: null },
            { key: "output", label: "Output", tokens: Math.round(input * 0.16), perM: 15, currency: "USD" as const, tier: null, costCny: Math.round((cost?.costCny ?? 0) * 0.5 * 100) / 100, includedIn: null },
          ],
        } as T;
      }
      return { model: String(args.model ?? "unknown"), priced: false, notes: ["浏览器 mock 未配置价格"], totalCny: 0, lines: [] } as T;
    }
    case "zcode_status": return state.providers?.find((p) => p.provider === "zcode") as T;
    case "volcengine_credentials_status": return { configured: false, backend: "mock", akHint: null } as T;
    case "hide_main_window":
    case "export_data":
    case "dock_hover":
    case "dock_interact":
    case "popup_close":
    case "quit_app":
    case "pricing_override":
    case "zcode_launch":
    case "zcode_reveal":
    case "volcengine_credentials_set":
    case "volcengine_credentials_clear":
    case "volcengine_test":
      throw new Error(`${cmd} is unavailable in browser mock`);
    default:
      throw new Error(`Unsupported browser mock command: ${cmd}`);
  }
}
