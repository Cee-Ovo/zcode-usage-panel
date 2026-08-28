import { Fragment, useEffect, useState } from "react";
import { Button, SegmentedControl, Switch, TextField } from "open-glass-ui";
import { api } from "../lib/ipc";
import { store, useStore } from "../lib/store";
import type {
  DiagnoseDto,
  OverrideDto,
  PriceEntryDto,
  PricingTableDto,
  Settings,
} from "../lib/types";
import { RANGE_KEYS, RANGE_LABELS } from "../lib/types";
import {
  formatDateTime,
  formatPerM,
  formatRelative,
} from "../lib/format";

export function SettingsPage() {
  const settings = useStore((s) => s.settings);
  const version = useStore((s) => s.version);
  const [draft, setDraft] = useState<Settings | null>(null);
  const [diag, setDiag] = useState<DiagnoseDto | null>(null);
  const [saving, setSaving] = useState(false);
  const [table, setTable] = useState<PricingTableDto | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [refreshMsg, setRefreshMsg] = useState<{ ok: boolean; text: string } | null>(null);
  const [expandedModel, setExpandedModel] = useState<string | null>(null);

  useEffect(() => {
    if (settings && !draft) setDraft(structuredClone(settings));
  }, [settings, draft]);

  useEffect(() => {
    api.pricingTable().then(setTable).catch(() => {});
  }, []);

  if (!draft) return <div className="empty-state">加载设置…</div>;

  const set = (patch: Partial<Settings>) => setDraft({ ...draft, ...patch });
  const snap = (patch: Partial<Settings["snap"]>) =>
    setDraft({ ...draft, snap: { ...draft.snap, ...patch } });
  const notif = (patch: Partial<Settings["notifications"]>) =>
    setDraft({ ...draft, notifications: { ...draft.notifications, ...patch } });

  const save = async () => {
    setSaving(true);
    try {
      const applied = await api.saveSettings(draft);
      store.set({ settings: applied });
    } finally {
      setSaving(false);
    }
  };

  const refreshPrices = async () => {
    setRefreshing(true);
    setRefreshMsg(null);
    try {
      const r = await api.pricingRefresh();
      if (r.ok && r.fxOk) {
        setRefreshMsg({ ok: true, text: `已更新 · ${r.refreshedAt}` });
      } else if (r.ok) {
        setRefreshMsg({
          ok: true,
          text: `价格表已更新,但汇率获取失败${r.error ? ` · ${r.error}` : ""}`,
        });
      } else {
        setRefreshMsg({ ok: false, text: r.error ?? "更新失败" });
      }
    } catch (e) {
      setRefreshMsg({ ok: false, text: String(e) });
    } finally {
      setRefreshing(false);
      try {
        setTable(await api.pricingTable());
      } catch {
        /* ignore */
      }
    }
  };

  const runDiagnose = async () => {
    setDiag(await api.diagnose());
  };

  const exportAs = async (scope: string, format: string) => {
    try {
      const path = await api.exportData(scope, format, draft.defaultRange, "");
      if (path && path !== "cancelled") {
        console.info("exported:", path);
      }
    } catch (e) {
      console.warn("export cancelled or failed:", e);
    }
  };

  return (
    <div style={{ paddingTop: 6, maxWidth: 860 }}>
      <div
        style={{
          position: "sticky",
          top: 0,
          zIndex: 40,
          display: "flex",
          alignItems: "center",
          gap: 10,
          padding: "8px 0",
          backdropFilter: "blur(12px)",
        }}
      >
        <strong style={{ fontSize: 13 }}>设置</strong>
        <span style={{ marginLeft: "auto" }} />
        <Button onClick={save} disabled={saving}>
          {saving ? "保存中…" : "保存设置"}
        </Button>
      </div>

      {/* ---------------- data source ---------------- */}
      <section className="panel settings-section">
        <div className="panel-title">数据目录</div>
        <div className="switch-row">
          <div>
            <div>自动检测</div>
            <div className="desc">优先 ZCODE_HOME 环境变量,否则 {`<用户目录>/.zcode`}</div>
          </div>
          <Switch
            label={null}
            aria-label="自动检测数据目录"
            checked={draft.dataDir === null}
            onCheckedChange={(v) => set({ dataDir: v ? null : "" })}
          />
        </div>
        {draft.dataDir !== null && (
          <div style={{ padding: "4px 2px" }}>
            <TextField
              label={null}
              value={draft.dataDir ?? ""}
              onChange={(e) => set({ dataDir: e.target.value })}
              placeholder="D:\\zcode-data(留空则恢复自动检测)"
            />
            <div className="desc" style={{ marginTop: 4 }}>
              支持中文与 Unicode 路径;目录变更后自动重新扫描(全程只读)。
            </div>
          </div>
        )}
        <div style={{ display: "flex", gap: 8, marginTop: 8 }}>
          <Button variant="quiet" onClick={runDiagnose}>
            检测数据源
          </Button>
        </div>
        {diag && (
          <div style={{ marginTop: 10, fontSize: 12 }}>
            <div className="kv">
              <span className="k">数据根目录</span>
              <span>
                {diag.root ?? "(未找到)"} <span className="muted">({diag.rootSource})</span>
              </span>
              <span className="k">JSONL 文件</span>
              <span>{diag.jsonlFiles.length} 个已跟踪</span>
              <span className="k">SQLite 文件</span>
              <span>{diag.sqliteFiles.length} 个已跟踪</span>
              <span className="k">累计记录</span>
              <span>{diag.recordCount} 条</span>
              <span className="k">最近刷新</span>
              <span>
                {diag.lastRefreshMs ? formatRelative(diag.lastRefreshMs) : "—"}
              </span>
              {diag.error && (
                <>
                  <span className="k">错误</span>
                  <span style={{ color: "var(--zup-danger)" }}>{diag.error}</span>
                </>
              )}
            </div>
            {diag.jsonlFiles.slice(0, 6).map((f) => (
              <div key={f.path} className="muted" style={{ fontSize: 11, marginTop: 2 }}>
                {f.path} · {f.recordsRead} 条 · offset {f.offset}
                {f.linesSkipped > 0 ? ` · 跳过 ${f.linesSkipped} 行` : ""}
                {f.lastError ? ` · ${f.lastError}` : ""}
              </div>
            ))}
            {diag.sqliteFiles.slice(0, 4).map((f) => (
              <div key={f.path} className="muted" style={{ fontSize: 11, marginTop: 2 }}>
                {f.path} · 表 {f.table ?? "?"} · {f.recordsRead} 条
              </div>
            ))}
            {diag.recentRecords.length > 0 && (
              <>
                <div className="panel-title" style={{ marginTop: 10 }}>
                  最近记录抽样(用于与 ZCode Usage 页核对)
                </div>
                {diag.recentRecords.map((r, i) => (
                  <div className="muted" style={{ fontSize: 11 }} key={i}>
                    {formatDateTime(r.tsMs)} · {r.model} · in {r.inputTokens} · out{" "}
                    {r.outputTokens} · reasoning{" "}
                    {r.reasoningTokens === null ? "unavailable" : r.reasoningTokens} · cache{" "}
                    {r.cacheReadTokens === null ? "unavailable" : r.cacheReadTokens}
                  </div>
                ))}
              </>
            )}
          </div>
        )}
      </section>

      {/* ---------------- general ---------------- */}
      <section className="panel settings-section">
        <div className="panel-title">通用</div>
        <div className="switch-row">
          <div>
            <div>默认时间范围</div>
          </div>
          <SegmentedControl
            aria-label="默认时间范围"
            value={draft.defaultRange}
            onValueChange={(v) => set({ defaultRange: v })}
            items={RANGE_KEYS.map((k) => ({ value: k, label: RANGE_LABELS[k] }))}
          />
        </div>
        <div className="switch-row">
          <div>
            <div>主题</div>
            <div className="desc">浅色为默认;跟随系统时实时响应 Windows 深色模式</div>
          </div>
          <SegmentedControl
            aria-label="主题"
            value={draft.theme}
            onValueChange={(v) => set({ theme: v })}
            items={[
              { value: "light", label: "浅色" },
              { value: "dark", label: "深色" },
              { value: "system", label: "跟随系统" },
            ]}
          />
        </div>
        <div className="switch-row">
          <div>
            <div>Always on Top</div>
          </div>
          <Switch
            label={null}
            aria-label="Always on Top"
            checked={draft.alwaysOnTop}
            onCheckedChange={(v) => set({ alwaysOnTop: v })}
          />
        </div>
        <div className="switch-row">
          <div>
            <div>关闭窗口时最小化到托盘</div>
            <div className="desc">关闭后仍实时监控;托盘菜单可退出</div>
          </div>
          <Switch
            label={null}
            aria-label="关闭窗口时最小化到托盘"
            checked={draft.closeToTray}
            onCheckedChange={(v) => set({ closeToTray: v })}
          />
        </div>
        <div className="switch-row">
          <div>
            <div>开机启动</div>
          </div>
          <Switch
            label={null}
            aria-label="开机启动"
            checked={draft.autostart}
            onCheckedChange={(v) => set({ autostart: v })}
          />
        </div>
        <div className="switch-row">
          <div>
            <div>刷新去抖(毫秒)</div>
            <div className="desc">文件变化事件合并窗口;增大可降低高频写入时的 CPU</div>
          </div>
          <input
            type="number"
            min={200}
            max={5000}
            step={100}
            value={draft.refreshDebounceMs}
            onChange={(e) => set({ refreshDebounceMs: Number(e.target.value) })}
            style={{ width: 90, textAlign: "right" }}
          />
        </div>
      </section>

      {/* ---------------- docking ---------------- */}
      <section className="panel settings-section">
        <div className="panel-title">边缘吸附(QQ 式贴边自动隐藏)</div>
        <div className="switch-row">
          <div>
            <div>启用边缘吸附</div>
            <div className="desc">拖动窗口靠近屏幕边缘时自动贴边</div>
          </div>
          <Switch
            label={null}
            aria-label="启用边缘吸附"
            checked={draft.snap.enabled}
            onCheckedChange={(v) => snap({ enabled: v })}
          />
        </div>
        <div className="switch-row">
          <div>
            <div>自动隐藏</div>
            <div className="desc">贴边后鼠标离开窗口片刻,窗口滑出屏幕仅留 4px 触发条</div>
          </div>
          <Switch
            label={null}
            aria-label="自动隐藏"
            checked={draft.snap.autoHide}
            onCheckedChange={(v) => snap({ autoHide: v })}
          />
        </div>
        <div className="switch-row">
          <div>
            <div>吸附方向</div>
          </div>
          <div style={{ display: "flex", gap: 10 }}>
            {(["left", "right", "top"] as const).map((side) => (
              <label key={side} style={{ display: "flex", gap: 4, alignItems: "center" }}>
                <input
                  type="checkbox"
                  checked={draft.snap.sides[side]}
                  onChange={(e) =>
                    snap({ sides: { ...draft.snap.sides, [side]: e.target.checked } })
                  }
                />
                {side === "left" ? "左" : side === "right" ? "右" : "上"}
              </label>
            ))}
          </div>
        </div>
        {(
          [
            ["thresholdPx", "吸附阈值(px,逻辑像素)", 6, 80, 1, 24],
            ["hideDelayMs", "自动隐藏延迟(ms)", 100, 3000, 50, 600],
            ["animMs", "动画时长(ms)", 120, 300, 10, 200],
          ] as const
        ).map(([key, label, min, max, step, def]) => (
          <div className="switch-row" key={key}>
            <div>
              <div>{label}</div>
            </div>
            <input
              type="range"
              min={min}
              max={max}
              step={step}
              value={draft.snap[key]}
              onChange={(e) => snap({ [key]: Number(e.target.value) } as never)}
            />
            <span style={{ width: 44, textAlign: "right", fontVariantNumeric: "tabular-nums" }}>
              {draft.snap[key]}
            </span>
          </div>
        ))}
      </section>

      {/* ---------------- alerts ---------------- */}
      <section className="panel settings-section">
        <div className="panel-title">异常检测(本地 Windows 通知)</div>
        <div className="switch-row">
          <div>
            <div>启用异常提醒</div>
            <div className="desc">所有规则本地计算;每条规则 15 分钟冷却</div>
          </div>
          <Switch
            label={null}
            checked={draft.notifications.enabled}
            onCheckedChange={(v) => notif({ enabled: v })}
          />
        </div>
        {(
          [
            ["spikeMultiplier", "激增倍数(10 分钟 vs 前一小时均值)", 2, 50, 1],
            ["spikeMinTokens", "激增最低 Token", 100_000, 50_000_000, 100_000],
            ["sessionTotalTokens", "单 Session 阈值", 1_000_000, 500_000_000, 1_000_000],
            ["cacheMinRequests", "命中率下降最少请求数", 5, 500, 5],
            ["modelBurstPer5m", "模型 5 分钟连调次数", 50, 5000, 50],
            ["stalenessMinutes", "数据停滞提醒(分钟)", 15, 720, 15],
          ] as const
        ).map(([key, label, min, max, step]) => (
          <div className="switch-row" key={key}>
            <div>
              <div>{label}</div>
            </div>
            <input
              type="number"
              min={min}
              max={max}
              step={step}
              value={draft.notifications[key]}
              onChange={(e) =>
                notif({ [key]: Number(e.target.value) } as never)
              }
              style={{ width: 110, textAlign: "right" }}
            />
          </div>
        ))}
      </section>

      {/* ---------------- API pricing ---------------- */}
      <section className="panel settings-section">
        <div className="panel-title">
          API 价格表
          <span className="badge-note">按官方 API 单价估算 · 非实际 Billing</span>
        </div>
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: 10,
            flexWrap: "wrap",
            marginBottom: 10,
          }}
        >
          <Button onClick={refreshPrices} disabled={refreshing}>
            {refreshing ? "更新中…" : "更新价格"}
          </Button>
          {refreshMsg && (
            <span
              style={{
                fontSize: 11,
                color: refreshMsg.ok ? "var(--zup-text-3)" : "var(--zup-danger)",
              }}
            >
              {refreshMsg.text}
            </span>
          )}
          <span className="muted" style={{ fontSize: 11, marginLeft: "auto" }}>
            {table
              ? `汇率 1 USD = ${table.fx.usdCny.toFixed(4)} CNY · 更新于 ${table.fx.updatedAt} · 来源 ${table.fx.source}`
              : ""}
          </span>
        </div>
        {table?.lastError && (
          <div className="desc" style={{ color: "var(--zup-danger)", marginBottom: 8 }}>
            上次刷新错误:{table.lastError}
          </div>
        )}
        <div style={{ marginBottom: 10 }}>
          <TextField
            label={null}
            value={draft.pricingRemoteUrl ?? ""}
            onChange={(e) => set({ pricingRemoteUrl: e.target.value === "" ? null : e.target.value })}
            placeholder="价格源 URL(可选,留空使用内置价格表)"
          />
          <div className="desc" style={{ marginTop: 4 }}>
            自定义官方价格表 JSON 地址(格式与内置 prices_builtin.json 一致),随「保存设置」一起保存,更新价格时使用。
          </div>
        </div>

        {!table ? (
          <div className="empty-state" style={{ padding: 20 }}>
            正在加载价格表…
          </div>
        ) : (
          <>
            <div className="price-row price-head">
              <span>模型</span>
              <span>Provider</span>
              <span className="num">Input</span>
              <span className="num">Cache Hit</span>
              <span className="num">Cache Write</span>
              <span className="num">Storage</span>
              <span className="num">Output</span>
              <span className="num">币种</span>
              <span className="num">更新时间</span>
              <span>来源</span>
            </div>
            {table.entries.map((e) => (
              <Fragment key={e.model}>
                <div
                  className="price-row"
                  onClick={() => setExpandedModel(expandedModel === e.model ? null : e.model)}
                  title="点击覆盖价格"
                >
                  <span>
                    <span className="name">{e.model}</span>
                    {e.promo?.currentIsPromo && (
                      <span className="price-badge promo" title={e.promo.note}>
                        促销
                      </span>
                    )}
                    {e.overridden && <span className="price-badge ov">已覆盖</span>}
                  </span>
                  <span className="muted">{e.displayName}</span>
                  <span className="num">{tierCell(e, "input")}</span>
                  <span className="num">{tierCell(e, "cacheHit")}</span>
                  <span className="num">{formatPerM(e.cacheWritePerM)}</span>
                  <span className="num">{formatPerM(e.cacheStoragePerM)}</span>
                  <span className="num">{tierCell(e, "output")}</span>
                  <span className="num">{e.currency}</span>
                  <span className="num muted">{e.updatedAt}</span>
                  <span>
                    {e.sourceUrl ? (
                      <a
                        href={e.sourceUrl}
                        target="_blank"
                        rel="noreferrer"
                        onClick={(ev) => ev.stopPropagation()}
                      >
                        链接
                      </a>
                    ) : (
                      "—"
                    )}
                  </span>
                </div>
                {expandedModel === e.model && (
                  <div className="price-override">
                    <OverrideForm
                      model={e.model}
                      initial={{
                        currency: e.currency,
                        input: e.inputPerM != null ? String(e.inputPerM) : "",
                        hit: e.cacheHitPerM != null ? String(e.cacheHitPerM) : "",
                        write: e.cacheWritePerM != null ? String(e.cacheWritePerM) : "",
                        write1h: e.cacheWrite1hPerM != null ? String(e.cacheWrite1hPerM) : "",
                        storage:
                          e.cacheStoragePerM != null ? String(e.cacheStoragePerM) : "",
                        output: e.outputPerM != null ? String(e.outputPerM) : "",
                        sourceUrl: e.sourceUrl,
                      }}
                      overridden={e.overridden}
                      onSaved={(t) => {
                        setTable(t);
                        setExpandedModel(null);
                      }}
                    />
                  </div>
                )}
              </Fragment>
            ))}

            {table.unknownModels.length > 0 && (
              <div style={{ marginTop: 12 }}>
                <div className="panel-title">价格未知的模型</div>
                {table.unknownModels.map((m) => (
                  <div key={m}>
                    <div className="switch-row">
                      <div>
                        <div>{m}</div>
                        <div className="desc">
                          没有官方价格表条目,手动覆盖后可参与成本估算
                        </div>
                      </div>
                      <Button
                        variant="quiet"
                        onClick={() => setExpandedModel(expandedModel === m ? null : m)}
                      >
                        手动覆盖
                      </Button>
                    </div>
                    {expandedModel === m && (
                      <div className="price-override">
                        <OverrideForm
                          model={m}
                          initial={{
                            currency: "CNY",
                            input: "",
                            hit: "",
                            write: "",
                            write1h: "",
                            storage: "",
                            output: "",
                            sourceUrl: "",
                          }}
                          overridden={false}
                          onSaved={(t) => {
                            setTable(t);
                            setExpandedModel(null);
                          }}
                        />
                      </div>
                    )}
                  </div>
                ))}
              </div>
            )}

            <div className="muted" style={{ fontSize: 10.5, marginTop: 12 }}>
              按官方 API 单价估算 · 非实际 Billing
            </div>
          </>
        )}
      </section>

      {/* ---------------- export ---------------- */}
      <section className="panel settings-section">
        <div className="panel-title">数据导出</div>
        <div className="desc" style={{ marginBottom: 8 }}>
          通过系统保存对话框导出;文件位置完全由你选择,卸载应用不会删除导出文件。
        </div>
        <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
          <Button variant="quiet" onClick={() => exportAs("range", "csv")}>
            时间范围统计 CSV
          </Button>
          <Button variant="quiet" onClick={() => exportAs("range", "json")}>
            时间范围 JSON
          </Button>
          <Button variant="quiet" onClick={() => exportAs("models", "csv")}>
            模型统计 CSV
          </Button>
          <Button variant="quiet" onClick={() => exportAs("sessions", "csv")}>
            Sessions CSV
          </Button>
          <Button variant="quiet" onClick={() => exportAs("raw", "json")}>
            原始记录 JSON
          </Button>
        </div>
      </section>

      {/* ---------------- about ---------------- */}
      <section className="panel settings-section">
        <div className="panel-title">关于</div>
        <div className="kv">
          <span className="k">版本</span>
          <span>v{version}</span>
          <span className="k">UI 组件</span>
          <span>open-glass-ui(MIT)— Liquid Glass 设计系统</span>
          <span className="k">数据访问</span>
          <span>只读;绝不写入或修改 ZCode 数据</span>
        </div>
        <div style={{ marginTop: 10 }}>
          <Button
            variant="quiet"
            onClick={() => {
              api.quitApp();
            }}
          >
            退出应用
          </Button>
        </div>
      </section>
    </div>
  );
}

interface OverrideFormState {
  currency: "CNY" | "USD";
  input: string;
  hit: string;
  write: string;
  write1h: string;
  storage: string;
  output: string;
  sourceUrl: string;
}

function OverrideForm({
  model,
  initial,
  overridden,
  onSaved,
}: {
  model: string;
  initial: OverrideFormState;
  overridden: boolean;
  onSaved: (t: PricingTableDto) => void;
}) {
  const [form, setForm] = useState<OverrideFormState>(initial);
  const [busy, setBusy] = useState(false);

  const set = (patch: Partial<OverrideFormState>) => setForm((f) => ({ ...f, ...patch }));

  // Required fields (input / cacheHit / output) coerce empty → 0 (免费);
  // optional fields coerce empty → null (不计费). Backend requires the
  // required three to be real numbers.
  const toRequired = (s: string): number => {
    const n = Number(s.trim());
    return Number.isFinite(n) ? n : 0;
  };
  const toOptional = (s: string): number | null => {
    if (s.trim() === "") return null;
    const n = Number(s.trim());
    return Number.isFinite(n) ? n : null;
  };

  const save = async () => {
    setBusy(true);
    try {
      const dto: OverrideDto = {
        currency: form.currency,
        inputPerM: toRequired(form.input),
        cacheHitPerM: toRequired(form.hit),
        cacheWritePerM: toOptional(form.write),
        cacheWrite1hPerM: toOptional(form.write1h),
        cacheStoragePerM: toOptional(form.storage),
        outputPerM: toRequired(form.output),
        sourceUrl: form.sourceUrl.trim() === "" ? null : form.sourceUrl.trim(),
        note: null,
      };
      const t = await api.pricingOverride(model, dto);
      onSaved(t);
    } catch (e) {
      console.warn("pricing override failed:", e);
    } finally {
      setBusy(false);
    }
  };

  const clear = async () => {
    setBusy(true);
    try {
      const t = await api.pricingOverride(model, null);
      onSaved(t);
    } catch (e) {
      console.warn("pricing override clear failed:", e);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div>
      <div className="switch-row" style={{ alignItems: "center" }}>
        <div style={{ fontWeight: 600, fontSize: 12 }}>{model}</div>
        <div style={{ display: "flex", gap: 8, alignItems: "center", marginLeft: "auto" }}>
          <SegmentedControl
            aria-label="币种"
            value={form.currency}
            onValueChange={(v) => set({ currency: v as "CNY" | "USD" })}
            items={[
              { value: "CNY", label: "CNY" },
              { value: "USD", label: "USD" },
            ]}
          />
          <Button
            variant="quiet"
            onClick={clear}
            disabled={busy}
            title={overridden ? "恢复官方价,清除覆盖" : "清除该模型的覆盖(若存在)"}
          >
            {overridden ? "恢复官方价" : "清除覆盖"}
          </Button>
          <Button onClick={save} disabled={busy}>
            {busy ? "保存中…" : "保存"}
          </Button>
        </div>
      </div>
      <div className="override-grid">
        <Field label="Input /M" value={form.input} onChange={(v) => set({ input: v })} />
        <Field label="Cache Hit /M" value={form.hit} onChange={(v) => set({ hit: v })} />
        <Field label="Cache Write /M" value={form.write} onChange={(v) => set({ write: v })} />
        <Field
          label="Cache Write 1h /M"
          value={form.write1h}
          onChange={(v) => set({ write1h: v })}
        />
        <Field label="Storage /M" value={form.storage} onChange={(v) => set({ storage: v })} />
        <Field label="Output /M" value={form.output} onChange={(v) => set({ output: v })} />
        <Field
          label="来源 URL"
          value={form.sourceUrl}
          onChange={(v) => set({ sourceUrl: v })}
          wide
        />
      </div>
      <div className="desc" style={{ fontSize: 10.5, margin: "4px 2px 0" }}>
        Input / Cache Hit / Output 留空按 0(免费)计;其余留空为不计费。
      </div>
    </div>
  );
}

function Field({
  label,
  value,
  onChange,
  wide,
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  wide?: boolean;
}) {
  return (
    <div className={wide ? "override-field wide" : "override-field"}>
      <div className="muted" style={{ fontSize: 10.5, marginBottom: 2 }}>
        {label}
      </div>
      <TextField label={null} value={value} onChange={(e) => onChange(e.target.value)} placeholder="—" />
    </div>
  );
}

function tierCell(e: PriceEntryDto, kind: "input" | "cacheHit" | "output") {
  if (e.tiers) {
    const peak = e.tiers.find((t) => t.name === "peak");
    const off = e.tiers.find((t) => t.name === "offpeak");
    const pick = (t: (typeof e.tiers)[number] | undefined) =>
      kind === "input"
        ? t?.inputPerM
        : kind === "cacheHit"
          ? t?.cacheHitPerM
          : t?.outputPerM;
    return (
      <span
        style={{
          display: "inline-flex",
          flexDirection: "column",
          alignItems: "flex-end",
          lineHeight: 1.5,
        }}
      >
        <span>高 {formatPerM(pick(peak))}</span>
        <span>空 {formatPerM(pick(off))}</span>
      </span>
    );
  }
  const v = kind === "input" ? e.inputPerM : kind === "cacheHit" ? e.cacheHitPerM : e.outputPerM;
  return <>{formatPerM(v)}</>;
}
