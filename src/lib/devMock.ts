import type { AppState } from "./store";
import type {
  DashboardDto,
  ProviderSnapshot,
  SessionSummary,
  SessionsPageDto,
  Settings,
  TrendDto,
  UsageUpdateEvent,
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
      },
      {
        name: "glm-5.3-air",
        agg: mkAgg(0.3),
        share: 0.29,
      },
      {
        name: "deepseek-v4",
        agg: mkAgg(0.08),
        share: 0.08,
      },
      {
        name: "kimi-k2.5",
        agg: mkAgg(0.02),
        share: 0.05,
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
  sessions: [
    {
      id: "a1b2c3d4e5f6-0001",
      project: "zcode-usage-panel",
      models: ["glm-5.3", "glm-5.3-air"],
      agg: mkAgg(0.42),
    },
    {
      id: "b2c3d4e5f6a7-0002",
      project: "codex-usage-docs",
      models: ["glm-5.3"],
      agg: mkAgg(0.2),
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
    return [s.id, s.project ?? "", ...s.models]
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
      return (summary ? { summary, buckets: [], models: [] } : null) as T;
    }
    case "get_model_detail": {
      const name = String(args.name ?? "glm-5.3");
      const model = state.dash?.models?.find((entry) => entry.name === name) ?? state.dash?.models?.[0];
      const modelAgg = model?.agg ?? state.dash?.agg;
      const total = modelAgg ? modelAgg.input + modelAgg.output + modelAgg.reasoning.sum + modelAgg.cacheRead.sum + modelAgg.cacheWrite.sum : 0;
      return {
        name,
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
    case "cost_detail": return { model: String(args.model ?? "unknown"), priced: false, notes: ["浏览器 mock 未配置价格"], totalCny: 0, lines: [] } as T;
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
