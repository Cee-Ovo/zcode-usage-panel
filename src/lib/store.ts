/** Minimal reactive store — selector-stable snapshots via useSyncExternalStore,
 *  so a dashboard update never re-renders the settings page. */

import { useSyncExternalStore } from "react";
import type {
  AlertEvent,
  CostSummaryDto,
  DashboardDto,
  DiagnoseDto,
  ModelDetailDto,
  ProviderSnapshot,
  QuotaAlertEvent,
  SessionDetailDto,
  SessionSummary,
  Settings,
  TrendDto,
  UsageUpdateEvent,
} from "./types";

export interface AppState {
  ready: boolean;
  version: string;
  settings: Settings | null;
  page: "dashboard" | "sessions" | "models" | "settings";
  rangeKey: string;
  dash: DashboardDto | null;
  trend: TrendDto | null;
  trendVisibleModels: string[] | null; // null = all
  costSummary: CostSummaryDto | null;
  sessions: SessionSummary[];
  sessionDetail: SessionDetailDto | null;
  modelDetail: ModelDetailDto | null;
  alerts: AlertEvent[];
  update: UsageUpdateEvent | null;
  diagnosis: DiagnoseDto | null;
  providers: ProviderSnapshot[];
  quotaAlerts: QuotaAlertEvent[];
}

type Listener = () => void;

const initial: AppState = {
  ready: false,
  version: "",
  settings: null,
  page: "dashboard",
  rangeKey: "today",
  dash: null,
  trend: null,
  trendVisibleModels: null,
  costSummary: null,
  sessions: [],
  sessionDetail: null,
  modelDetail: null,
  alerts: [],
  update: null,
  diagnosis: null,
  providers: [],
  quotaAlerts: [],
};

let state: AppState = initial;
const listeners = new Set<Listener>();

export const store = {
  get: (): AppState => state,
  set(patch: Partial<AppState>): void {
    state = { ...state, ...patch };
    listeners.forEach((l) => l());
  },
  subscribe(l: Listener): () => void {
    listeners.add(l);
    return () => {
      listeners.delete(l);
    };
  },
};

export function useStore<S>(selector: (s: AppState) => S): S {
  return useSyncExternalStore(store.subscribe, () => selector(store.get()));
}
