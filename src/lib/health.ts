/**
 * Unified monitoring-health derivation for the status card, the dashboard
 * error pill and any surface that used to read raw refresh errors.
 *
 * Why this exists: the engine reports per-cycle errors and the query
 * coordinator reports per-request IPC errors. Feeding either straight into
 * the UI made the status dot flip red/green on every transient blip. The
 * tracker applies deliberate hysteresis instead:
 *
 * - downgrade slowly — an error becomes visible only after
 *   `QUERY_ERROR_VISIBILITY` consecutive query failures, after the engine's
 *   own streak gating (≥2 cycles, see engine.rs `gate_error`), or when a
 *   query error persists with no successful refresh for
 *   `STALE_ERROR_AFTER_MS`;
 * - recover quickly — any success returns to ok immediately;
 * - deterministic failures (bootstrap/initialization) surface at once.
 *
 * All inputs and outputs are plain data so the rules are unit-testable with
 * a fake clock.
 */

import type { UsageUpdateEvent } from "./types";

export type HealthLevel = "ok" | "paused" | "suspended" | "error";

export interface HealthView {
  level: HealthLevel;
  /** Always-visible status label, e.g. 「监控中」 / 「刷新异常」. */
  statusText: string;
  /** Honest detail for 运行详情 / tooltips; null when healthy. */
  detail: string | null;
  /** True when the error is persistent (drove `level === "error"`). */
  persistent: boolean;
}

/** Consecutive failed query cycles before the coordinator path shows red. */
export const QUERY_ERROR_VISIBILITY = 2;
/** A lone query error only matters once refreshes have been failing this long. */
export const STALE_ERROR_AFTER_MS = 60_000;

const OK_VIEW: HealthView = { level: "ok", statusText: "监控中", detail: null, persistent: false };

function errorView(statusText: string, detail: string): HealthView {
  return { level: "error", statusText, detail, persistent: true };
}

export interface HealthQueryState {
  loading: boolean;
  error: string | null;
  lastSuccessMs: number | null;
}

export class HealthTracker {
  private queryErrors = 0;
  /** True while the last seen coordinator state carried an error — a single
   * failed request publishes its error twice (catch + finally), so counting
   * raw publications would double-count every failure. */
  private inQueryError = false;
  private lastQuerySuccessMs: number | null = null;
  private lastQueryError: string | null = null;
  private engine: UsageUpdateEvent | null = null;
  private initializationError: string | null = null;
  private paused = false;
  private suspended = false;

  /** Feed the query-coordinator state (called on every state change). */
  onQueryState(state: HealthQueryState): void {
    if (state.error !== null) {
      if (!this.inQueryError) {
        this.queryErrors += 1;
        this.lastQueryError = state.error;
        this.inQueryError = true;
      }
    } else {
      this.inQueryError = false;
      if (state.lastSuccessMs !== null && state.lastSuccessMs !== this.lastQuerySuccessMs) {
        this.queryErrors = 0;
        this.lastQueryError = null;
        this.lastQuerySuccessMs = state.lastSuccessMs;
      }
    }
  }

  /** Feed the engine's `usage-update` event. */
  onEngineEvent(update: UsageUpdateEvent): void {
    this.engine = update;
    this.paused = update.paused;
    this.suspended = update.suspended;
  }

  /** Bootstrap outcome — a failure here is deterministic, never transient. */
  onInitialization(error: string | null): void {
    this.initializationError = error;
  }

  /** Pause flag from settings (the authoritative UI source — engine events
   * can be quiet in dev/mock sessions and right after toggling). */
  onPaused(paused: boolean): void {
    this.paused = paused;
  }

  derive(now: number): HealthView {
    if (this.initializationError !== null) {
      return errorView("初始化失败", this.initializationError);
    }
    if (this.paused) {
      return { level: "paused", statusText: "已暂停", detail: null, persistent: false };
    }
    if (this.suspended) {
      return { level: "suspended", statusText: "已挂起", detail: null, persistent: false };
    }

    // Engine path: the backend already streak-gates this error (≥2 cycles).
    const engineError = this.engine?.error ?? null;
    if (engineError !== null) {
      const streak = this.engine?.errorStreak ?? 0;
      return errorView(
        "刷新异常",
        `数据源连续 ${Math.max(streak, 2)} 个刷新周期失败:${engineError}`,
      );
    }

    // Query path: repeated failures, or one failure while refreshes have
    // been stale long enough, are persistent; a lone blip right after a
    // success stays green (it is recorded for 运行详情 via transientDetail).
    const stale =
      this.lastQuerySuccessMs === null ||
      now - this.lastQuerySuccessMs > STALE_ERROR_AFTER_MS;
    if (this.queryErrors >= QUERY_ERROR_VISIBILITY || (this.lastQueryError !== null && stale)) {
      return errorView(
        "刷新异常",
        this.queryErrors >= QUERY_ERROR_VISIBILITY
          ? `查询连续失败 ${this.queryErrors} 次:${this.lastQueryError ?? "未知错误"}`
          : `刷新失败且超过 ${Math.round(STALE_ERROR_AFTER_MS / 1000)} 秒无成功刷新`,
      );
    }

    return OK_VIEW;
  }

  /** Transient blips that never downgraded the level — for 运行详情 only. */
  transientDetail(): string | null {
    if (this.queryErrors > 0) {
      return `最近 ${this.queryErrors} 次查询失败(未影响监控):${this.lastQueryError ?? "未知错误"}`;
    }
    return null;
  }
}
