/** TypeScript mirrors of the Rust DTOs (serde camelCase). */

export interface FieldStat {
  sum: number;
  present: number;
}

export interface Agg {
  requests: number;
  input: number;
  output: number;
  reasoning: FieldStat;
  cacheRead: FieldStat;
  cacheWrite: FieldStat;
  /** Σ per-record display totals (backend caliber-aware); the value behind
   * the "总 Token" cards. Absent only in legacy/dev-mock payloads. */
  totalSum?: number;
  hitCached: number;
  hitInputTotal: number;
  firstTsMs: number | null;
  lastTsMs: number | null;
}

/** ZCode 响应速度统计(与 Agg 同一范围的查询期现算,样本为 0 时字段为 null)。 */
export interface SpeedStats {
  ttftAvgMs: number | null;
  ttftP50Ms: number | null;
  ttftP95Ms: number | null;
  ttftSamples: number;
  speedTps: number | null;
  speedP50Tps: number | null;
  speedSamples: number;
  /** true = tps 的窗口为整请求时长(由事件时间戳推导,含首字等待,未记录 TTFT),UI 标注"近似"。 */
  speedApproximate?: boolean;
  completedRequests: number;
  generatedTokens: number;
  generationMs: number;
}

export interface ModelStat {
  name: string;
  agg: Agg;
}

export interface ModelRow {
  name: string;
  agg: Agg;
  share: number;
  /** Per-model speed stats; absent/default on the boot-snapshot path. */
  speed?: SpeedStats | null;
}

export interface ModelSwitch {
  tsMs: number;
  model: string;
}

export interface ActiveSession {
  sessionId: string;
  project: string | null;
  sessionTotalTokens: number;
  sessionAgg: Agg;
  tokensLast5m: number;
  tokensPerMin: number;
  lastRequestMs: number | null;
  activeModel: string | null;
  modelSwitches: ModelSwitch[];
}

export interface DashboardDto {
  rangeKey: string;
  fromMs: number;
  toMs: number;
  agg: Agg;
  models: ModelRow[];
  activeSession: ActiveSession | null;
  speed: SpeedStats;
  restored: boolean;
  dataError: string | null;
}

export interface Bucket {
  startMs: number;
  endMs: number;
  agg: Agg;
  byModel: Record<string, Agg>;
}

export interface TrendDto {
  rangeKey: string;
  fromMs: number;
  toMs: number;
  buckets: Bucket[];
  restored: boolean;
}

export interface SessionSummary {
  id: string;
  /** 真实会话标题（ZCode CLI session 表）；null = 无真实标题,诚实降级为 "—"。 */
  title: string | null;
  /** 项目文件夹名(显示用);记录级 project 优先,回退 session 表 directory 末段。 */
  project: string | null;
  /** 完整工作区路径(悬停提示/详情用);null = 未知。 */
  projectPath: string | null;
  models: string[];
  agg: Agg;
}

export type SessionSort = "recent" | "tokens";

export interface SessionsPageDto {
  items: SessionSummary[];
  total: number;
  page: number;
  pageSize: number;
}

export interface SessionDetailDto {
  summary: SessionSummary;
  buckets: Bucket[];
  models: ModelStat[];
}

export interface HistoryHealth {
  persistent: boolean;
  error: string | null;
  lastSuccessMs: number | null;
}

export interface ModelDetailDto {
  name: string;
  today: Agg;
  last7d: Agg;
  last30d: Agg;
  allTime: Agg;
  avgTokensPerRequest: number;
  hitRate: number | null;
  lastUsedMs: number | null;
  trend30d: Bucket[];
  topSessions: [string, number][];
}

export interface FileStatusDto {
  path: string;
  recordsRead: number;
  linesSkipped: number;
  offset: number;
  watermark: number;
  table: string | null;
  lastError: string | null;
}

export interface DiagnoseDto {
  root: string | null;
  rootSource: string;
  jsonlFiles: FileStatusDto[];
  sqliteFiles: FileStatusDto[];
  untrackedJsonl: number;
  untrackedSqlite: number;
  notes: string[];
  recordCount: number;
  lastRefreshMs: number | null;
  error: string | null;
  recentRecords: {
    tsMs: number;
    model: string;
    inputTokens: number;
    outputTokens: number;
    reasoningTokens: number | null;
    cacheReadTokens: number | null;
    cacheWriteTokens: number | null;
  }[];
}

export interface AlertEvent {
  rule: string;
  severity: number;
  title: string;
  body: string;
  tsMs: number;
}

export interface UsageUpdateEvent {
  recordCount: number;
  lastRefreshMs: number | null;
  lastRecordMs: number | null;
  /** Streak-gated by the backend: only set once failures persist ≥2 cycles. */
  error: string | null;
  errorStreak: number;
  paused: boolean;
  suspended: boolean;
  restoredFromCache: boolean;
}

export interface SnapSides {
  left: boolean;
  right: boolean;
  top: boolean;
}

export interface SnapSettings {
  enabled: boolean;
  autoHide: boolean;
  thresholdPx: number;
  hideDelayMs: number;
  animMs: number;
  sides: SnapSides;
}

export interface AlertRuleState {
  enabled: boolean;
  spikeMultiplier: number;
  spikeMinTokens: number;
  sessionTotalTokens: number;
  cacheHitDrop: number;
  cacheMinRequests: number;
  modelBurstPer5m: number;
  stalenessMinutes: number;
}

export interface WindowState {
  x: number;
  y: number;
  width: number;
  height: number;
  maximized: boolean;
  dockSide: string | null;
  dockHidden: boolean;
}

export interface Settings {
  dataDir: string | null;
  refreshDebounceMs: number;
  defaultRange: string;
  theme: string;
  alwaysOnTop: boolean;
  monitoringPaused: boolean;
  closeToTray: boolean;
  autostart: boolean;
  pricingRemoteUrl: string | null;
  snap: SnapSettings;
  notifications: AlertRuleState;
  window: WindowState;
  providers: ProviderSettings;
  launcher: LauncherSettings;
  quotaAlerts: QuotaAlertRules;
}

export interface ProviderSettings {
  codexEnabled: boolean;
  codexHome: string | null;
  codexRefreshMs: number;
  dshEnabled: boolean;
  dshHome: string | null;
  dshRefreshMs: number;
  claudeCodeEnabled: boolean;
  claudeCodeHome: string | null;
  claudeCodeRefreshMs: number;
  antigravityEnabled: boolean;
  antigravityRefreshMs: number;
  volcengineEnabled: boolean;
  volcengineRefreshMs: number;
  volcengineRegion: string;
  volcengineFilter: string;
}

export interface LauncherSettings {
  enabled: boolean;
  exePath: string | null;
  autostart: boolean;
}

export interface QuotaAlertRules {
  enabled: boolean;
  thresholds: number[];
  packageExpiryDays: number;
  dailyCostCny: number;
}

export interface BootstrapDto {
  settings: Settings;
  version: string;
  configDir: string | null;
  cacheDir: string | null;
}

// ---- official-API cost estimation (cost_summary / cost_detail / pricing_*) ----

export interface FxInfo {
  usdCny: number;
  updatedAt: string;
  source: string;
}

export interface ModelCost {
  name: string;
  costCny: number;
  priced: boolean;
}

export interface CostSummaryDto {
  range: string;
  totalTokens: number;
  totalCostCny: number;
  fullyPriced: boolean;
  models: ModelCost[];
  unknownModels: string[];
  fx: FxInfo;
  priceUpdatedAt: string;
  disclaimer: string;
}

export interface CostLine {
  key: string;
  label: string;
  tokens: number;
  perM: number | null;
  currency: "CNY" | "USD";
  tier: "peak" | "offpeak" | null;
  costCny: number;
  includedIn: string | null;
}

export interface CostDetailDto {
  model: string;
  priced: boolean;
  notes: string[];
  totalCny: number;
  lines: CostLine[];
}

export interface PromoDto {
  activeUntil: string;
  note: string;
  currentIsPromo: boolean;
}

export interface TierDto {
  name: "peak" | "offpeak";
  inputPerM: number;
  cacheHitPerM: number;
  outputPerM: number;
}

export interface PriceEntryDto {
  provider: string;
  displayName: string;
  model: string;
  currency: "CNY" | "USD";
  inputPerM: number | null;
  cacheHitPerM: number | null;
  cacheWritePerM: number | null;
  cacheWrite1hPerM: number | null;
  cacheStoragePerM: number | null;
  outputPerM: number | null;
  tiers: TierDto[] | null;
  reasoningPolicy: string;
  sourceUrl: string;
  updatedAt: string;
  promo: PromoDto | null;
  overridden: boolean;
  notes: string[];
}

export interface PricingTableDto {
  entries: PriceEntryDto[];
  unknownModels: string[];
  fx: FxInfo;
  remoteUrl: string | null;
  lastRefresh: string | null;
  lastError: string | null;
}

export interface PricingRefreshResultDto {
  ok: boolean;
  fxOk: boolean;
  error: string | null;
  refreshedAt: string;
}

export interface OverrideDto {
  currency: "CNY" | "USD";
  inputPerM: number;
  cacheHitPerM: number;
  cacheWritePerM: number | null;
  cacheWrite1hPerM: number | null;
  cacheStoragePerM: number | null;
  outputPerM: number;
  sourceUrl: string | null;
  note: string | null;
}


// ---- multi-provider quota dashboard (providers/*) ---------------------------

export type ProviderStatus =
  | "ok"
  | "not_configured"
  | "not_installed"
  | "disabled"
  | "stale"
  | "error";

export interface Forecast {
  etaMs: number;
  ratePerDay: number;
  samples: number;
  confidence: "low" | "medium" | "high" | string;
}

export interface QuotaWindow {
  key: string;
  label: string;
  usedPercent: number | null;
  totalQuota: number | null;
  usedQuota: number | null;
  remainingQuota: number | null;
  unit: string | null;
  resetAtMs: number | null;
  windowMinutes: number | null;
  forecast: Forecast | null;
}

export interface PackageInfo {
  instanceNo: string;
  name: string;
  configuration: string;
  product: string;
  totalAmount: number;
  availableAmount: number;
  usedAmount: number;
  unit: string;
  unitMultiplier: number;
  effectiveMs: number | null;
  expiryMs: number | null;
  status: string;
  usagePercent: number | null;
}

export interface TokenBreakdown {
  requests: number;
  inputTokens: number;
  cachedInputTokens: number;
  cacheWriteTokens: number;
  outputTokens: number;
  reasoningTokens: number;
  totalTokens: number;
}

export interface ModelUsageRow {
  model: string;
  breakdown: TokenBreakdown;
}

export interface LocalUsageRange {
  key: "today" | "60m" | "24h" | "7d" | "30d" | "all" | string;
  breakdown: TokenBreakdown;
  sessions: number;
  models: ModelUsageRow[];
}

export interface LocalUsage {
  today: TokenBreakdown;
  last7d: TokenBreakdown;
  allTime: TokenBreakdown;
  sessions: number;
  models: ModelUsageRow[];
  ranges: LocalUsageRange[];
}

export interface LauncherStatus {
  state: "not_installed" | "not_running" | "starting" | "running" | string;
  exePath: string | null;
  version: string | null;
  detectedVia: string | null;
}

export interface ProviderSnapshot {
  provider: string;
  status: ProviderStatus;
  account: string | null;
  planName: string | null;
  windows: QuotaWindow[];
  packages: PackageInfo[];
  localUsage: LocalUsage | null;
  launcher: LauncherStatus | null;
  source: string;
  sourceUrl: string | null;
  notes: string[];
  error: string | null;
  updatedAtMs: number;
  nextPollMs: number;
}

export interface QuotaAlertEvent {
  rule: string;
  severity: number;
  title: string;
  body: string;
  tsMs: number;
}

export interface HistoryPointDto {
  tsMs: number;
  usedPercent: number | null;
  used: number | null;
  remaining: number | null;
}

export interface LauncherActionDto {
  result: string;
  snapshot: ProviderSnapshot;
}

export interface CredentialsStatusDto {
  configured: boolean;
  backend: string;
  akHint: string | null;
}

export const PROVIDER_LABELS: Record<string, string> = {
  zcode: "ZCode",
  codex: "Codex",
  dsh: "DSH",
  "claude-code": "Claude Code",
  antigravity: "Antigravity",
  volcengine: "火山引擎",
};

export const PROVIDER_STATUS_LABELS: Record<ProviderStatus, string> = {
  ok: "正常",
  not_configured: "未配置",
  not_installed: "未安装",
  disabled: "已禁用",
  stale: "数据过期",
  error: "查询失败",
};

export const RANGE_KEYS = ["today", "60m", "24h", "7d", "30d", "all"] as const;
export type RangeKey = (typeof RANGE_KEYS)[number];

export const RANGE_LABELS: Record<RangeKey, string> = {
  today: "今天",
  "60m": "60 分钟",
  "24h": "24 小时",
  "7d": "7 天",
  "30d": "30 天",
  all: "全部",
};

// ---- derived helpers -------------------------------------------------------

export function totalTokens(agg: Agg): number {
  // Backend-caliber total (inclusive schemas don't re-add cache; reasoning
  // may be nested in output). Fallback recombinces for legacy/mock payloads.
  if (typeof agg.totalSum === "number") return agg.totalSum;
  return agg.input + agg.output + agg.reasoning.sum + agg.cacheRead.sum + agg.cacheWrite.sum;
}

/** Unified cache-hit-rate convention (see README §统计口径). */
export function cacheHitRate(agg: Agg): number | null {
  if (agg.hitInputTotal > 0) return agg.hitCached / agg.hitInputTotal;
  return null;
}

export function coverage(stat: FieldStat, requests: number): number {
  if (requests <= 0) return 0;
  return stat.present / requests;
}
/** Atomic dashboard/trend/cost response from one ingestion revision. */
export interface UsageViewDto {
  dash: DashboardDto;
  trend: TrendDto | null;
  costSummary: CostSummaryDto;
  revision: number;
}
