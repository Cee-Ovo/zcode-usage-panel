/** Typed wrappers around Tauri IPC. */

import type {
  BootstrapDto,
  CostDetailDto,
  CostSummaryDto,
  DashboardDto,
  DiagnoseDto,
  LauncherActionDto,
  ModelDetailDto,
  OverrideDto,
  PricingRefreshResultDto,
  PricingTableDto,
  ProviderSnapshot,
  SessionDetailDto,
  SessionSummary,
  SessionsPageDto,
  SessionSort,
  Settings,
  TrendDto,
  UsageUpdateEvent,
  UsageViewDto,
} from "./types";

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
    const { mockInvoke } = await import("./devMock");
    return mockInvoke<T>(cmd, args);
  }
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(cmd, args);
}

export const api = {
  bootstrap: () => invoke<BootstrapDto>("get_bootstrap"),
  dashboard: (rangeKey: string) => invoke<DashboardDto>("get_dashboard", { rangeKey }),
  usageView: (rangeKey: string, includeTrend = true) =>
    invoke<UsageViewDto>("get_usage_view", { rangeKey, includeTrend }),
  localUsageView: (provider: string, rangeKey: string, includeTrend = true) =>
    invoke<UsageViewDto>("get_local_usage_view", { provider, rangeKey, includeTrend }),
  trend: (rangeKey: string) => invoke<TrendDto>("get_trend", { rangeKey }),
  sessions: () => invoke<SessionSummary[]>("get_sessions"),
  sessionsPage: (query = "", sort: SessionSort = "recent", page = 0, pageSize = 50) =>
    invoke<SessionsPageDto>("get_sessions_page", { query, sort, page, pageSize }),
  sessionDetail: (sessionId: string) =>
    invoke<SessionDetailDto | null>("get_session_detail", { sessionId }),
  /** `provider` scopes the lookup to one source (`zcode` by default). */
  modelDetail: (name: string, provider?: string) =>
    invoke<ModelDetailDto | null>("get_model_detail", { name, provider }),
  activeModels: () => invoke<string[]>("get_active_models"),
  saveSettings: (settings: Settings) => invoke<Settings>("set_settings", { newSettings: settings }),
  diagnose: () => invoke<DiagnoseDto>("diagnose"),
  refreshNow: () => invoke<void>("refresh_now"),
  hideMainWindow: () => invoke<void>("hide_main_window"),
  exportData: (scope: string, format: string, rangeKey: string, suggestedName: string) =>
    invoke<string>("export_data", { scope, format, rangeKey, suggestedName }),
  dockHover: (inside: boolean) => invoke<void>("dock_hover", { inside }),
  dockInteract: (active: boolean) => invoke<void>("dock_interact", { active }),
  popupClose: () => invoke<void>("popup_close"),
  quitApp: () => invoke<void>("quit_app"),

  // ---- official-API cost estimation ----
  costSummary: (rangeKey: string) =>
    invoke<CostSummaryDto>("cost_summary", { range: rangeKey }),
  costDetail: (rangeKey: string, model: string, provider?: string) =>
    invoke<CostDetailDto>("cost_detail", { range: rangeKey, model, provider: provider ?? null }),
  pricingTable: () => invoke<PricingTableDto>("pricing_table"),
  pricingRefresh: () => invoke<PricingRefreshResultDto>("pricing_refresh"),
  pricingOverride: (model: string, o: OverrideDto | null) =>
    invoke<PricingTableDto>("pricing_override", { model, o }),

  // ---- provider snapshots (drives local-source refresh nudges) ----
  providersOverview: () => invoke<ProviderSnapshot[]>("providers_overview"),
  zcodeStatus: () => invoke<ProviderSnapshot>("zcode_status"),
  zcodeLaunch: () => invoke<LauncherActionDto>("zcode_launch"),
  zcodeReveal: () => invoke<LauncherActionDto>("zcode_reveal"),
};

/** Subscribe to a backend event; returns an unlisten function. */
export async function onEvent<T>(
  name: string,
  handler: (payload: T) => void,
): Promise<() => void> {
  if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) return () => {};
  const { listen } = await import("@tauri-apps/api/event");
  const un = await listen<T>(name, (e) => handler(e.payload));
  return un;
}
