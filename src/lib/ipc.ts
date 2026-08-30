/** Typed wrappers around Tauri IPC. */

import type {
  AlertEvent,
  BootstrapDto,
  CostDetailDto,
  CostSummaryDto,
  CredentialsStatusDto,
  DashboardDto,
  DiagnoseDto,
  HistoryPointDto,
  LauncherActionDto,
  ModelDetailDto,
  OverrideDto,
  PricingRefreshResultDto,
  PricingTableDto,
  ProviderSnapshot,
  QuotaAlertEvent,
  SessionDetailDto,
  SessionSummary,
  Settings,
  TrendDto,
  UsageUpdateEvent,
} from "./types";

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(cmd, args);
}

export const api = {
  bootstrap: () => invoke<BootstrapDto>("get_bootstrap"),
  dashboard: (rangeKey: string) => invoke<DashboardDto>("get_dashboard", { rangeKey }),
  trend: (rangeKey: string) => invoke<TrendDto>("get_trend", { rangeKey }),
  sessions: () => invoke<SessionSummary[]>("get_sessions"),
  sessionDetail: (sessionId: string) =>
    invoke<SessionDetailDto | null>("get_session_detail", { sessionId }),
  modelDetail: (name: string) => invoke<ModelDetailDto | null>("get_model_detail", { name }),
  alerts: () => invoke<AlertEvent[]>("get_alerts"),
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
  costDetail: (rangeKey: string, model: string) =>
    invoke<CostDetailDto>("cost_detail", { range: rangeKey, model }),
  pricingTable: () => invoke<PricingTableDto>("pricing_table"),
  pricingRefresh: () => invoke<PricingRefreshResultDto>("pricing_refresh"),
  pricingOverride: (model: string, o: OverrideDto | null) =>
    invoke<PricingTableDto>("pricing_override", { model, o }),

  // ---- multi-provider quota dashboard ----
  providersOverview: () => invoke<ProviderSnapshot[]>("providers_overview"),
  providersRefresh: (provider?: string) =>
    invoke<void>("providers_refresh", { provider: provider ?? null }),
  quotaAlertsList: () => invoke<QuotaAlertEvent[]>("quota_alerts_list"),
  providersHistory: (provider: string, window: string, range: string) =>
    invoke<HistoryPointDto[]>("providers_history", { provider, window, range }),
  providersConsumption: (provider: string, window: string, days: number) =>
    invoke<[number, number][]>("providers_consumption", { provider, window, days }),
  zcodeStatus: () => invoke<ProviderSnapshot>("zcode_status"),
  zcodeLaunch: () => invoke<LauncherActionDto>("zcode_launch"),
  zcodeReveal: () => invoke<LauncherActionDto>("zcode_reveal"),
  volcengineCredentialsStatus: () =>
    invoke<CredentialsStatusDto>("volcengine_credentials_status"),
  volcengineCredentialsSet: (ak: string, sk: string) =>
    invoke<void>("volcengine_credentials_set", { ak, sk }),
  volcengineCredentialsClear: () =>
    invoke<void>("volcengine_credentials_clear"),
  volcengineTest: () => invoke<string>("volcengine_test"),
};

/** Subscribe to a backend event; returns an unlisten function. */
export async function onEvent<T>(
  name: string,
  handler: (payload: T) => void,
): Promise<() => void> {
  const { listen } = await import("@tauri-apps/api/event");
  const un = await listen<T>(name, (e) => handler(e.payload));
  return un;
}
