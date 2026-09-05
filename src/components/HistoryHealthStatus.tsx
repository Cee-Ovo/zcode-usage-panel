import { useEffect, useState } from "react";
import { api, onEvent } from "../lib/ipc";
import type { HistoryHealth } from "../lib/types";

/** Persistence health is independent of quota freshness: failed storage must
 * not turn valid provider readings into an empty/zero quota. */
export function HistoryHealthStatus() {
  const [health, setHealth] = useState<HistoryHealth | null>(null);
  const [failed, setFailed] = useState(false);
  const [retry, setRetry] = useState(0);
  useEffect(() => {
    let disposed = false;
    let running = false;
    let unsubscribe: (() => void) | undefined;
    const refresh = async () => {
      if (disposed || running || document.hidden) return;
      running = true;
      try {
        const result = await api.historyHealth();
        if (!disposed) { setHealth(result); setFailed(false); }
      } catch {
        if (!disposed) setFailed(true);
      } finally { running = false; }
    };
    void refresh();
    onEvent("provider-update", () => { void refresh(); }).then((u) => {
      if (disposed) u(); else unsubscribe = u;
    }).catch(() => { if (!disposed) setFailed(true); });
    document.addEventListener("visibilitychange", refresh);
    return () => {
      disposed = true;
      unsubscribe?.();
      document.removeEventListener("visibilitychange", refresh);
    };
  }, [retry]);

  const warning = failed || !!health?.error || health?.persistent === false;
  return (
    <div role="status" style={{ marginTop: 6, fontSize: 10.5 }}>
      {failed ? "历史保存状态未知" : health?.error ? "历史保存异常" :
        health ? (health.persistent ? "历史记录 · 本地保存" : "历史记录 · 仅本次运行") : "正在检查历史保存…"}
      {warning && <div>
        <span>{failed ? "暂时无法检查。" : health?.error || "退出后不保留历史。"}</span>
        <button type="button" onClick={() => setRetry((n) => n + 1)} aria-label="重新检查历史保存状态">重新检查</button>
      </div>}
    </div>
  );
}
