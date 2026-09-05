import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { GlassSystemProvider, OrganicFilterDefinition, Switch } from "open-glass-ui";
import { AnimatePresence, LayoutGroup, MotionConfig, motion } from "motion/react";
import { api, onEvent } from "./lib/ipc";
import { pageVariants, popSpring, softSpring } from "./lib/motion";
import { store, useStore } from "./lib/store";
import type { RangeKey } from "./lib/types";
import { RANGE_KEYS, RANGE_LABELS } from "./lib/types";
import { TitleBar, WindowFrame } from "./components/WindowFrame";
import { HistoryHealthStatus } from "./components/HistoryHealthStatus";
import { DashboardPage } from "./pages/Dashboard";
import { SessionsPage } from "./pages/Sessions";
import { ModelsPage } from "./pages/Models";
import { SettingsPage } from "./pages/Settings";
import { formatClock } from "./lib/format";
import { createQueryCoordinator, type QueryCoordinator } from "./lib/queryCoordinator";
import type { AppState } from "./lib/store";

type DataPage = "dashboard" | "models";
type QueryPayload = Awaited<ReturnType<typeof api.usageView>>;

function createAppQueryCoordinator() {
  return createQueryCoordinator<DataPage, QueryPayload>({
    fetch: ({ page, rangeKey: key }) => api.usageView(key, page === "dashboard"),
    apply: (result) => {
      const patch: Partial<AppState> = {
        dash: result.dash,
        costSummary: result.costSummary,
      };
      if (result.trend !== null) patch.trend = result.trend;
      store.set(patch);
    },
    onStateChange: (next) => store.set({ refresh: next }),
  });
}

/**
 * Application shell: glass theme provider, custom frame (title bar +
 * resize edges), navigation rail, and the backend data pump.
 *
 * Data flow: backend "usage-update" events (throttled ≥500 ms server-side)
 * → store.set(dash/trend) → each page re-renders only its own slice via
 * selector-stable `useStore` subscriptions.
 */
export function App() {
  const page = useStore((s) => s.page);
  const theme = useStore((s) => s.settings?.theme ?? "light");
  const paused = useStore((s) => s.settings?.monitoringPaused ?? false);
  const update = useStore((s) => s.update);
  const suspended = update?.suspended ?? false;
  const rangeKey = useStore((s) => s.rangeKey);
  const ready = useStore((s) => s.ready);
  const initializationError = useStore((s) => s.initializationError);
  const refresh = useStore((s) => s.refresh);
  const [resolvedTheme, setResolvedTheme] = useState<"light" | "dark">(
    theme === "dark" ? "dark" : "light",
  );
  const coordinatorRef = useRef<QueryCoordinator<DataPage, QueryPayload> | null>(null);
  const bootstrapRetryRef = useRef<(() => void) | null>(null);
  const initializationFailedRef = useRef(false);

  // ---- theme ---------------------------------------------------------------
  useEffect(() => {
    const apply = (t: string) => {
      const resolved =
        t === "system"
          ? matchMedia("(prefers-color-scheme: dark)").matches
            ? "dark"
            : "light"
          : t;
      document.documentElement.setAttribute("data-theme", resolved);
      localStorage.setItem("zup.theme", resolved);
      setResolvedTheme(resolved as "light" | "dark");
    };
    apply(theme);
    if (theme === "system") {
      const mq = matchMedia("(prefers-color-scheme: dark)");
      const fn = () => apply("system");
      mq.addEventListener("change", fn);
      return () => mq.removeEventListener("change", fn);
    }
  }, [theme]);

  // Pause the ambient material while the webview is hidden. The liquid
  // background is CSS-only, but there is no reason to keep compositing it
  // when the desktop window is not visible.
  useEffect(() => {
    const syncAmbientState = () => {
      document.documentElement.toggleAttribute("data-ambient-paused", document.hidden);
    };
    syncAmbientState();
    document.addEventListener("visibilitychange", syncAmbientState);
    return () => document.removeEventListener("visibilitychange", syncAmbientState);
  }, []);

  const requestVisiblePage = useCallback(() => {
    const queryCoordinator = coordinatorRef.current;
    if (!queryCoordinator) return;
    const current = store.get();
    if (!current.ready || document.hidden || (current.page !== "dashboard" && current.page !== "models")) {
      queryCoordinator.setVisible(false);
      return;
    }
    queryCoordinator.request({
      page: current.page,
      rangeKey: current.rangeKey,
      visible: true,
    });
  }, []);

  const refreshAll = useCallback(() => {
    api.refreshNow().catch(() => {});
    const current = store.get();
    if (current.page === "sessions") {
      window.dispatchEvent(new Event("zup-page-sessions"));
    } else {
      requestVisiblePage();
    }
  }, [requestVisiblePage]);

  const retryRefresh = useCallback(() => {
    if (!store.get().ready || initializationFailedRef.current || store.get().initializationError) {
      bootstrapRetryRef.current?.();
    }
    else requestVisiblePage();
  }, [requestVisiblePage]);

  useEffect(() => {
    const queryCoordinator = createAppQueryCoordinator();
    coordinatorRef.current = queryCoordinator;
    let disposed = false;
    const bootstrap = async () => {
      try {
        const boot = await api.bootstrap();
        if (disposed) return;
        initializationFailedRef.current = false;
        store.set({
          ready: true,
          initializationError: null,
          version: boot.version,
          settings: boot.settings,
          rangeKey: (RANGE_KEYS as readonly string[]).includes(boot.settings.defaultRange)
            ? (boot.settings.defaultRange as RangeKey)
            : "today",
        });
        const alerts = await api.alerts();
        const providers = await api.providersOverview();
        const quotaAlerts = await api.quotaAlertsList();
        if (!disposed) store.set({ alerts, providers, quotaAlerts });
      } catch {
        if (!disposed) {
          initializationFailedRef.current = true;
          store.set({
            initializationError: "初始化失败，请重试",
          });
        }
      }
    };
    bootstrapRetryRef.current = () => void bootstrap();
    void bootstrap();

    const registerEvent = <T,>(name: string, handler: (payload: T) => void) => {
      let cancelled = false;
      let unlisten: (() => void) | null = null;
      onEvent<T>(name, (payload) => {
        if (!cancelled && !disposed) handler(payload);
      })
        .then((u) => {
          if (cancelled || disposed) u();
          else unlisten = u;
        })
        .catch(() => {});
      const cleanup = () => {
        cancelled = true;
        if (unlisten) {
          unlisten();
          unlisten = null;
        }
      };
      eventCleanups.push(cleanup);
    };
    const eventCleanups: (() => void)[] = [];

    registerEvent<import("./lib/types").UsageUpdateEvent>("usage-update", (e) => {
      store.set({ update: e });
      // Refresh the visible queries — Rust only emits at most ~2×/s.
      requestVisiblePage();
    });

    registerEvent<import("./lib/types").ProviderSnapshot[]>("provider-update", (snaps) => {
      store.set({ providers: snaps ?? [] });
    });

    registerEvent<import("./lib/types").QuotaAlertEvent>("quota-alert", (ev) => {
      const current = store.get().quotaAlerts;
      store.set({ quotaAlerts: [ev, ...current].slice(0, 50) });
    });

    registerEvent<boolean>("ui-visibility", (visible) => {
      if (visible) {
        api.refreshNow().catch(() => {});
        requestVisiblePage();
      } else {
        coordinatorRef.current?.setVisible(false);
      }
    });

    registerEvent<import("./lib/types").AlertEvent>("alert", (ev) => {
      const current = store.get().alerts;
      store.set({ alerts: [ev, ...current].slice(0, 50) });
    });

    registerEvent<import("./lib/types").Settings>("settings-changed", (s) => {
      store.set({ settings: s });
    });

    registerEvent<string>("navigate", (target) => {
      if (target === "settings") store.set({ page: "settings" });
    });

    // model visibility toggles from chart chips
    const onToggle = (e: Event) => {
      const detail = (e as CustomEvent).detail as string[] | null;
      store.set({ trendVisibleModels: detail });
    };
    window.addEventListener("zup-toggle-model", onToggle);

    return () => {
      disposed = true;
      bootstrapRetryRef.current = null;
      queryCoordinator.dispose();
      if (coordinatorRef.current === queryCoordinator) coordinatorRef.current = null;
      eventCleanups.forEach((cleanup) => cleanup());
      window.removeEventListener("zup-toggle-model", onToggle);
    };
  }, [requestVisiblePage]);

  useEffect(() => {
    if (ready) requestVisiblePage();
    else coordinatorRef.current?.setVisible(false);
  }, [ready, page, rangeKey, requestVisiblePage]);

  useEffect(() => {
    const onVisibilityChange = () => {
      if (document.hidden) coordinatorRef.current?.setVisible(false);
      else requestVisiblePage();
    };
    document.addEventListener("visibilitychange", onVisibilityChange);
    return () => document.removeEventListener("visibilitychange", onVisibilityChange);
  }, [requestVisiblePage]);

  const setRange = useCallback(
    (key: string) => store.set({ rangeKey: key }),
    [],
  );

  const nav = useMemo(
    () =>
      (
        [
          ["dashboard", "仪表盘", "◱"],
          ["sessions", "Sessions", "⧉"],
          ["models", "模型", "◈"],
          ["settings", "设置", "⚙"],
        ] as const
      ).map(([id, label, icon]) => (
        <motion.button
          key={id}
          layout
          whileHover={{ x: 2 }}
          whileTap={{ scale: 0.965 }}
          transition={softSpring}
          className={`zup-nav-item ${page === id ? "active" : ""}`}
          onClick={() => {
            store.set({ page: id });
            if (id === "sessions") {
              window.dispatchEvent(new Event("zup-page-sessions"));
            }
          }}
        >
          {page === id && (
            <motion.span
              layoutId="zup-nav-active"
              className="zup-nav-active"
              transition={softSpring}
            >
              <span className="zup-nav-liquid-light" />
            </motion.span>
          )}
          {/* icon pops once when its item becomes the active page */}
          <motion.span
            key={page === id ? "on" : "off"}
            className="zup-nav-content"
            aria-hidden
            style={{ opacity: 0.7 }}
            initial={{ scale: 0.75 }}
            animate={{ scale: 1 }}
            transition={popSpring}
          >
            {icon}
          </motion.span>
          <span className="zup-nav-content">{label}</span>
        </motion.button>
      )),
    [page],
  );

  return (
    <MotionConfig reducedMotion="user">
      <GlassSystemProvider
        renderer="auto"
        quality="auto"
        motion="system"
        theme={{ appearance: resolvedTheme }}
      >
        <OrganicFilterDefinition
          id="zup-liquid-refraction"
          frequency={0.012}
          turbulence={0.58}
          scale={9}
          seed={4}
          animate={false}
        />
        <div className="zup-shell zup-frame">
          <div className="liquid-ambient" aria-hidden>
            <span className="liquid-ambient__orb liquid-ambient__orb--blue" />
            <span className="liquid-ambient__orb liquid-ambient__orb--cyan" />
            <span className="liquid-ambient__orb liquid-ambient__orb--violet" />
            <span className="liquid-ambient__grain" />
          </div>
          <WindowFrame />
          <TitleBar title="ZCode Usage Panel" onRefresh={refreshAll} />
          <div className="zup-body">
            <nav className="zup-nav">
              <LayoutGroup id="primary-navigation">{nav}</LayoutGroup>
              <div className="footnote">
                <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                  <span
                    className={`status-dot ${paused || suspended ? "paused" : update?.error ? "error" : "live"}`}
                  />
                  {paused ? "已暂停监控" : suspended ? "窗口隐藏·挂起监控" : "实时监控中"}
                </div>
                {update?.lastRefreshMs ? (
                  <div>更新 {formatClock(update.lastRefreshMs)}</div>
                ) : null}
                {update?.restoredFromCache && <div>显示缓存统计,同步中…</div>}
                {initializationError && (
                  <div role="alert" style={{ color: "var(--zup-red-600, #b42318)" }}>
                    初始化失败 · {initializationError}{" "}
                    <button
                      type="button"
                      className="zup-nav-item"
                      onClick={retryRefresh}
                      style={{ padding: "1px 5px", marginLeft: 2 }}
                    >
                      重试
                    </button>
                  </div>
                )}
                {refresh.loading && <div role="status">数据刷新中…</div>}
                {refresh.error && !initializationError && (
                  <div role="alert" style={{ color: "var(--zup-red-600, #b42318)" }}>
                    刷新失败 · {refresh.error}{" "}
                    <button
                      type="button"
                      className="zup-nav-item"
                      onClick={retryRefresh}
                      style={{ padding: "1px 5px", marginLeft: 2 }}
                    >
                      重试
                    </button>
                  </div>
                )}
                {refresh.lastSuccessMs !== null && (
                  <div>最近成功 {formatClock(refresh.lastSuccessMs)}</div>
                )}
                <HistoryHealthStatus />
                <div style={{ display: "flex", alignItems: "center", gap: 6, marginTop: 6 }}>
                  <span>实时</span>
                  <Switch
                    label={null}
                    aria-label="实时监控开关"
                    checked={!paused}
                    onCheckedChange={(v) => {
                      const s = store.get().settings;
                      if (s) api.saveSettings({ ...s, monitoringPaused: !v }).catch(() => {});
                    }}
                  />
                </div>
              </div>
            </nav>
            <main className="zup-content">
              <AnimatePresence mode="wait" initial={false}>
                <motion.div
                  key={page}
                  className="motion-page"
                  variants={pageVariants}
                  initial="initial"
                  animate="enter"
                  exit="exit"
                >
                  {page === "dashboard" && <DashboardPage onRangeChange={setRange} />}
                  {page === "sessions" && <SessionsPage />}
                  {page === "models" && <ModelsPage />}
                  {page === "settings" && <SettingsPage />}
                </motion.div>
              </AnimatePresence>
            </main>
          </div>
        </div>
      </GlassSystemProvider>
    </MotionConfig>
  );
}
