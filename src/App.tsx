import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { GlassSystemProvider, Switch } from "open-glass-ui";
import { AnimatePresence, LayoutGroup, MotionConfig, motion } from "motion/react";
import { api, onEvent } from "./lib/ipc";
import { pageVariants, softSpring } from "./lib/motion";
import { store, useStore } from "./lib/store";
import type { RangeKey } from "./lib/types";
import { RANGE_KEYS, RANGE_LABELS } from "./lib/types";
import { TitleBar, WindowFrame } from "./components/WindowFrame";
import { DashboardPage } from "./pages/Dashboard";
import { SessionsPage } from "./pages/Sessions";
import { ModelsPage } from "./pages/Models";
import { SettingsPage } from "./pages/Settings";
import { formatClock } from "./lib/format";

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
  const [resolvedTheme, setResolvedTheme] = useState<"light" | "dark">(
    theme === "dark" ? "dark" : "light",
  );

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

  // ---- bootstrap + data pump -------------------------------------------------
  const rangeRef = useRef(rangeKey);
  rangeRef.current = rangeKey;

  const refreshQueries = useCallback(async () => {
    const key = rangeRef.current;
    const [dash, trend, cost] = await Promise.all([
      api.dashboard(key),
      api.trend(key),
      api.costSummary(key),
    ]);
    store.set({ dash, trend, costSummary: cost });
  }, []);

  const refreshAll = useCallback(() => {
    api.refreshNow().catch(() => {});
    refreshQueries().catch(() => {});
    api.sessions().then((sessions) => store.set({ sessions })).catch(() => {});
  }, [refreshQueries]);

  useEffect(() => {
    let disposed = false;
    (async () => {
      const boot = await api.bootstrap();
      if (disposed) return;
      store.set({
        ready: true,
        version: boot.version,
        settings: boot.settings,
        rangeKey: (RANGE_KEYS as readonly string[]).includes(boot.settings.defaultRange)
          ? (boot.settings.defaultRange as RangeKey)
          : "today",
      });
      await refreshQueries();
      const sessions = await api.sessions();
      const alerts = await api.alerts();
      const providers = await api.providersOverview();
      const quotaAlerts = await api.quotaAlertsList();
      if (!disposed) store.set({ sessions, alerts, providers, quotaAlerts });
    })().catch(console.error);

    const unsubs: (() => void)[] = [];
    onEvent<import("./lib/types").UsageUpdateEvent>("usage-update", (e) => {
      store.set({ update: e });
      // Refresh the visible queries — Rust only emits at most ~2×/s.
      refreshQueries().catch(() => {});
    }).then((u) => unsubs.push(u));

    onEvent<import("./lib/types").ProviderSnapshot[]>("provider-update", (snaps) => {
      store.set({ providers: snaps ?? [] });
    }).then((u) => unsubs.push(u));

    onEvent<import("./lib/types").QuotaAlertEvent>("quota-alert", (ev) => {
      const current = store.get().quotaAlerts;
      store.set({ quotaAlerts: [ev, ...current].slice(0, 50) });
    }).then((u) => unsubs.push(u));

    onEvent<boolean>("ui-visibility", (visible) => {
      if (visible) {
        api.refreshNow().catch(() => {});
        refreshQueries().catch(() => {});
      }
    }).then((u) => unsubs.push(u));

    onEvent<import("./lib/types").AlertEvent>("alert", (ev) => {
      const current = store.get().alerts;
      store.set({ alerts: [ev, ...current].slice(0, 50) });
    }).then((u) => unsubs.push(u));

    onEvent<import("./lib/types").Settings>("settings-changed", (s) => {
      store.set({ settings: s });
    }).then((u) => unsubs.push(u));

    onEvent<string>("navigate", (target) => {
      if (target === "settings") store.set({ page: "settings" });
    }).then((u) => unsubs.push(u));

    // model visibility toggles from chart chips
    const onToggle = (e: Event) => {
      const detail = (e as CustomEvent).detail as string[] | null;
      store.set({ trendVisibleModels: detail });
    };
    window.addEventListener("zup-toggle-model", onToggle);

    return () => {
      disposed = true;
      unsubs.forEach((u) => u());
      window.removeEventListener("zup-toggle-model", onToggle);
    };
  }, [refreshQueries]);

  // Sessions list refreshes only while its page is visible (cheap either
  // way: the backend serves a memoized slice) — no idle polling.
  useEffect(() => {
    const refresh = () => {
      if (store.get().page === "sessions") {
        api.sessions().then((sessions) => store.set({ sessions })).catch(() => {});
      }
    };
    const t = setInterval(refresh, 15_000);
    window.addEventListener("zup-page-sessions", refresh);
    return () => {
      clearInterval(t);
      window.removeEventListener("zup-page-sessions", refresh);
    };
  }, []);

  const setRange = useCallback(
    (key: string) => {
      store.set({ rangeKey: key });
      Promise.all([api.dashboard(key), api.trend(key), api.costSummary(key)])
        .then(([dash, trend, cost]) => store.set({ dash, trend, costSummary: cost }))
        .catch(() => {});
    },
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
          whileTap={{ scale: 0.975 }}
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
            />
          )}
          <span className="zup-nav-content" aria-hidden style={{ opacity: 0.7 }}>
            {icon}
          </span>
          <span className="zup-nav-content">{label}</span>
        </motion.button>
      )),
    [page],
  );

  return (
    <MotionConfig reducedMotion="user">
      <GlassSystemProvider theme={{ appearance: resolvedTheme }}>
        <div className="zup-shell zup-frame">
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
