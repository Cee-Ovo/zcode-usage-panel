import { Fragment, useEffect, useRef, useState } from "react";
import { SegmentedControl, Switch, TextField } from "open-glass-ui";
import { motion } from "motion/react";
import { api } from "../lib/ipc";
import { FxButton, useAction } from "../components/fx";
import { store, useStore } from "../lib/store";
import type {
  CredentialsStatusDto,
  DiagnoseDto,
  LauncherStatus,
  OverrideDto,
  PriceEntryDto,
  PricingTableDto,
  ProviderSnapshot,
  Settings,
} from "../lib/types";
import { RANGE_KEYS, RANGE_LABELS } from "../lib/types";
import {
  formatDateTime,
  formatPerM,
  formatRelative,
} from "../lib/format";

/**
 * Settings:分区导航(通用 / ZCode / Codex / Antigravity / 火山引擎 /
 * API Pricing / 通知 / 外观 / 高级)。敏感凭据只进系统凭据管理器,
 * 永远不写入 settings.json。
 */

const SECTIONS = [
  ["general", "通用"],
  ["zcode", "ZCode"],
  ["codex", "Codex"],
  ["antigravity", "Antigravity"],
  ["volcengine", "火山引擎"],
  ["pricing", "API Pricing"],
  ["notifications", "通知"],
  ["appearance", "外观"],
  ["advanced", "高级"],
] as const;

export function SettingsPage() {
  const settings = useStore((s) => s.settings);
  const version = useStore((s) => s.version);
  const [draft, setDraft] = useState<Settings | null>(null);
  const [diag, setDiag] = useState<DiagnoseDto | null>(null);
  const [table, setTable] = useState<PricingTableDto | null>(null);
  const [refreshMsg, setRefreshMsg] = useState<{ ok: boolean; text: string } | null>(null);
  const [expandedModel, setExpandedModel] = useState<string | null>(null);
  // brief highlight on the section a nav chip jumps to
  const [flash, setFlash] = useState<string | null>(null);
  const flashTimer = useRef<number | undefined>(undefined);
  useEffect(() => () => window.clearTimeout(flashTimer.current), []);

  useEffect(() => {
    if (settings && !draft) setDraft(structuredClone(settings));
  }, [settings, draft]);

  useEffect(() => {
    api.pricingTable().then(setTable).catch(() => {});
  }, []);

  // hooks must run before the early return below (rules of hooks)
  const save = useAction(
    async () => {
      if (!draft) return;
      const applied = await api.saveSettings(draft);
      store.set({ settings: applied });
    },
    { okText: "已保存" },
  );

  const diagnose = useAction(
    async () => {
      setDiag(await api.diagnose());
    },
    { okText: "检测完成" },
  );

  if (!draft) return <div className="empty-state">加载设置…</div>;

  const set = (patch: Partial<Settings>) => setDraft({ ...draft, ...patch });
  const snap = (patch: Partial<Settings["snap"]>) =>
    setDraft({ ...draft, snap: { ...draft.snap, ...patch } });
  const notif = (patch: Partial<Settings["notifications"]>) =>
    setDraft({ ...draft, notifications: { ...draft.notifications, ...patch } });
  const prov = (patch: Partial<Settings["providers"]>) =>
    setDraft({ ...draft, providers: { ...draft.providers, ...patch } });
  const launcher = (patch: Partial<Settings["launcher"]>) =>
    setDraft({ ...draft, launcher: { ...draft.launcher, ...patch } });
  const quota = (patch: Partial<Settings["quotaAlerts"]>) =>
    setDraft({ ...draft, quotaAlerts: { ...draft.quotaAlerts, ...patch } });

  const jump = (id: string) => {
    document.getElementById(`sec-${id}`)?.scrollIntoView({ behavior: "smooth", block: "start" });
    setFlash(id);
    window.clearTimeout(flashTimer.current);
    flashTimer.current = window.setTimeout(() => setFlash(null), 900);
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
        <FxButton
          variant="primary"
          size="small"
          magnetic
          action={save}
          busyLabel="保存中…"
          okText="已保存"
          title="保存全部设置(任意分区)"
        >
          保存设置
        </FxButton>
      </div>

      <div className="settings-nav">
        {SECTIONS.map(([id, label]) => (
          <button
            key={id}
            className={`settings-nav-btn ${flash === id ? "flash" : ""}`}
            onClick={() => jump(id)}
          >
            {label}
          </button>
        ))}
      </div>

      {/* ---------------- 通用 General ---------------- */}
      <section className="panel settings-section" id="sec-general">
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
            <div>开机启动本软件</div>
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

      {/* ---------------- ZCode ---------------- */}
      <section className="panel settings-section" id="sec-zcode">
        <div className="panel-title">ZCode(数据源与快速启动)</div>
        <div className="switch-row">
          <div>
            <div>数据目录自动检测</div>
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
        <div className="switch-row">
          <div>
            <div>ZCode 快速启动</div>
            <div className="desc">仪表盘/托盘显示运行状态,一键启动或聚焦已运行窗口</div>
          </div>
          <Switch
            label={null}
            aria-label="启用 ZCode 快速启动"
            checked={draft.launcher.enabled}
            onCheckedChange={(v) => launcher({ enabled: v })}
          />
        </div>
        <div className="switch-row">
          <div>
            <div>自定义 ZCode 路径</div>
            <div className="desc">留空自动检测常见安装位置与 PATH</div>
          </div>
        </div>
        <TextField
          label={null}
          value={draft.launcher.exePath ?? ""}
          onChange={(e) => launcher({ exePath: e.target.value === "" ? null : e.target.value })}
          placeholder="C:\\Users\\<你>\\AppData\\Local\\Programs\\ZCode\\ZCode.exe"
        />
        <div className="switch-row" style={{ marginTop: 6 }}>
          <div>
            <div>本软件启动时自动启动 ZCode</div>
          </div>
          <Switch
            label={null}
            aria-label="自动启动 ZCode"
            checked={draft.launcher.autostart}
            onCheckedChange={(v) => launcher({ autostart: v })}
          />
        </div>
        <LauncherProbe />
        <div style={{ display: "flex", gap: 8, marginTop: 8 }}>
          <FxButton variant="quiet" size="small" action={diagnose} busyLabel="检测中…">
            检测数据源
          </FxButton>
        </div>
        {diag && <DiagnosePanel diag={diag} />}
      </section>

      {/* ---------------- Codex ---------------- */}
      <section className="panel settings-section" id="sec-codex">
        <div className="panel-title">
          OpenAI Codex
          <span className="badge-note">额度来自 Codex 官方客户端本地数据</span>
        </div>
        <div className="switch-row">
          <div>
            <div>启用 Codex 额度监控</div>
            <div className="desc">
              读取 {`<用户目录>/.codex`} 的官方 session 文件:5 小时窗口 / 周额度 / credits(离线,不联网)
            </div>
          </div>
          <Switch
            label={null}
            aria-label="启用 Codex"
            checked={draft.providers.codexEnabled}
            onCheckedChange={(v) => prov({ codexEnabled: v })}
          />
        </div>
        <div className="switch-row">
          <div>
            <div>CODEX_HOME 覆盖</div>
            <div className="desc">留空使用默认 {`<用户目录>/.codex`} 或 CODEX_HOME 环境变量</div>
          </div>
        </div>
        <TextField
          label={null}
          value={draft.providers.codexHome ?? ""}
          onChange={(e) => prov({ codexHome: e.target.value === "" ? null : e.target.value })}
          placeholder="D:\\codex-data"
        />
        <IntervalRow
          label="额度刷新间隔"
          value={draft.providers.codexRefreshMs}
          onChange={(v) => prov({ codexRefreshMs: v })}
          min={30}
          max={3600}
        />
        <div className="desc">
          本地 Harness Token 统计(Input / Cached / Output / Reasoning)与官方套餐额度分开显示,绝不合并。
        </div>
      </section>

      {/* ---------------- Antigravity ---------------- */}
      <section className="panel settings-section" id="sec-antigravity">
        <div className="panel-title">
          Antigravity / 反重力
          <span className="badge-note">本地官方客户端 RPC · 仅 127.0.0.1</span>
        </div>
        <div className="switch-row">
          <div>
            <div>启用 Antigravity 额度监控</div>
            <div className="desc">
              通过 Antigravity 官方本地服务查询套餐/剩余额度;客户端未运行时显示「未找到运行中的本地服务」
            </div>
          </div>
          <Switch
            label={null}
            aria-label="启用 Antigravity"
            checked={draft.providers.antigravityEnabled}
            onCheckedChange={(v) => prov({ antigravityEnabled: v })}
          />
        </div>
        <IntervalRow
          label="额度刷新间隔"
          value={draft.providers.antigravityRefreshMs}
          onChange={(v) => prov({ antigravityRefreshMs: v })}
          min={60}
          max={3600}
        />
        <div className="desc">
          Antigravity 无公开远程额度 API;数据完全来自本机官方守护进程,失败时自动降级为 unavailable,不猜测。
        </div>
      </section>

      {/* ---------------- 火山引擎 ---------------- */}
      <section className="panel settings-section" id="sec-volcengine">
        <div className="panel-title">
          火山引擎 Token 包
          <span className="badge-note">官方费用中心 OpenAPI</span>
        </div>
        <div className="switch-row">
          <div>
            <div>启用火山引擎 Token 包监控</div>
            <div className="desc">调用 ListResourcePackages(官方接口)查询已购资源包余额与到期时间</div>
          </div>
          <Switch
            label={null}
            aria-label="启用火山引擎"
            checked={draft.providers.volcengineEnabled}
            onCheckedChange={(v) => prov({ volcengineEnabled: v })}
          />
        </div>
        <div className="switch-row">
          <div>
            <div>Region</div>
          </div>
        </div>
        <TextField
          label={null}
          value={draft.providers.volcengineRegion}
          onChange={(e) => prov({ volcengineRegion: e.target.value })}
          placeholder="cn-beijing"
        />
        <div className="switch-row" style={{ marginTop: 6 }}>
          <div>
            <div>资源包过滤(可选)</div>
            <div className="desc">按名称/规格筛选,例如「Token」;留空显示全部资源包</div>
          </div>
        </div>
        <TextField
          label={null}
          value={draft.providers.volcengineFilter}
          onChange={(e) => prov({ volcengineFilter: e.target.value })}
          placeholder="Token"
        />
        <IntervalRow
          label="额度刷新间隔"
          value={draft.providers.volcengineRefreshMs}
          onChange={(v) => prov({ volcengineRefreshMs: v })}
          min={300}
          max={86400}
        />
        <VolcengineCredentials />
      </section>

      {/* ---------------- API pricing ---------------- */}
      <section className="panel settings-section" id="sec-pricing">
        <div className="panel-title">
          API 价格表
          <span className="badge-note">按官方 API 单价估算 · 非实际 Billing</span>
        </div>
        <PriceSection
          table={table}
          setTable={setTable}
          draft={draft}
          set={set}
          refreshMsg={refreshMsg}
          setRefreshMsg={setRefreshMsg}
          expandedModel={expandedModel}
          setExpandedModel={setExpandedModel}
        />
      </section>

      {/* ---------------- 通知 ---------------- */}
      <section className="panel settings-section" id="sec-notifications">
        <div className="panel-title">通知(本地 Windows 通知)</div>

        <div className="panel-title" style={{ fontSize: 12, marginTop: 2 }}>
          额度提醒
        </div>
        <div className="switch-row">
          <div>
            <div>启用额度提醒</div>
            <div className="desc">剩余 50% / 20% / 10%、Token 包到期、数据停更、API 成本阈值</div>
          </div>
          <Switch
            label={null}
            aria-label="启用额度提醒"
            checked={draft.quotaAlerts.enabled}
            onCheckedChange={(v) => quota({ enabled: v })}
          />
        </div>
        <div className="switch-row">
          <div>
            <div>剩余比例阈值</div>
            <div className="desc">低于所选比例时通知(同一事件冷却 6 小时)</div>
          </div>
          <div style={{ display: "flex", gap: 10 }}>
            {[50, 20, 10].map((t) => (
              <label key={t} style={{ display: "flex", gap: 4, alignItems: "center" }}>
                <input
                  type="checkbox"
                  checked={draft.quotaAlerts.thresholds.includes(t)}
                  onChange={(e) => {
                    const cur = draft.quotaAlerts.thresholds;
                    quota({
                      thresholds: e.target.checked
                        ? [...cur, t].sort((a, b) => a - b)
                        : cur.filter((x) => x !== t),
                    });
                  }}
                />
                {t}%
              </label>
            ))}
          </div>
        </div>
        <div className="switch-row">
          <div>
            <div>Token 包到期提前提醒(天)</div>
          </div>
          <input
            type="number"
            min={1}
            max={60}
            value={draft.quotaAlerts.packageExpiryDays}
            onChange={(e) => quota({ packageExpiryDays: Number(e.target.value) })}
            style={{ width: 80, textAlign: "right" }}
          />
        </div>
        <div className="switch-row">
          <div>
            <div>今日 API 等价成本提醒(¥,0 关闭)</div>
            <div className="desc">ZCode 当日估算成本达到阈值时提醒一次</div>
          </div>
          <input
            type="number"
            min={0}
            step={1}
            value={draft.quotaAlerts.dailyCostCny}
            onChange={(e) => quota({ dailyCostCny: Number(e.target.value) })}
            style={{ width: 90, textAlign: "right" }}
          />
        </div>

        <div className="panel-title" style={{ fontSize: 12, marginTop: 14 }}>
          ZCode 用量异常检测
        </div>
        <div className="switch-row">
          <div>
            <div>启用异常提醒</div>
            <div className="desc">所有规则本地计算;每条规则 15 分钟冷却</div>
          </div>
          <Switch
            label={null}
            aria-label="启用异常提醒"
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
              onChange={(e) => notif({ [key]: Number(e.target.value) } as never)}
              style={{ width: 110, textAlign: "right" }}
            />
          </div>
        ))}
      </section>

      {/* ---------------- 外观 ---------------- */}
      <section className="panel settings-section" id="sec-appearance">
        <div className="panel-title">外观</div>
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
      </section>

      {/* ---------------- 高级 ---------------- */}
      <section className="panel settings-section" id="sec-advanced">
        <div className="panel-title">高级(吸附 / 导出 / 关于)</div>

        <div className="panel-title" style={{ fontSize: 12 }}>
          边缘吸附(QQ 式贴边自动隐藏)
        </div>
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

        <div className="panel-title" style={{ fontSize: 12, marginTop: 14 }}>
          数据导出
        </div>
        <div className="desc" style={{ marginBottom: 8 }}>
          通过系统保存对话框导出;文件位置完全由你选择,卸载应用不会删除导出文件。
        </div>
        <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
          <ExportBtn draft={draft} scope="range" format="csv" label="时间范围统计 CSV" />
          <ExportBtn draft={draft} scope="range" format="json" label="时间范围 JSON" />
          <ExportBtn draft={draft} scope="models" format="csv" label="模型统计 CSV" />
          <ExportBtn draft={draft} scope="sessions" format="csv" label="Sessions CSV" />
          <ExportBtn draft={draft} scope="raw" format="json" label="原始记录 JSON" />
        </div>

        <div className="panel-title" style={{ fontSize: 12, marginTop: 14 }}>
          关于
        </div>
        <div className="kv">
          <span className="k">版本</span>
          <span>v{version}</span>
          <span className="k">UI 组件</span>
          <span>open-glass-ui(MIT)— Liquid Glass 设计系统</span>
          <span className="k">数据访问</span>
          <span>只读;绝不写入或修改 ZCode / Codex 数据</span>
          <span className="k">凭据存储</span>
          <span>系统凭据管理器(Windows Credential Manager);不写文件、不进日志</span>
        </div>
        <div style={{ marginTop: 10 }}>
          <FxButton variant="danger" size="small" onClick={() => api.quitApp()}>
            退出应用
          </FxButton>
        </div>
      </section>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Sub-panels
// ---------------------------------------------------------------------------

function IntervalRow({
  label,
  value,
  onChange,
  min,
  max,
}: {
  label: string;
  value: number;
  onChange: (v: number) => void;
  min: number;
  max: number;
}) {
  return (
    <div className="switch-row" style={{ marginTop: 6 }}>
      <div>
        <div>{label}</div>
        <div className="desc">窗口隐藏时自动放缓一倍,降低后台占用</div>
      </div>
      <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
        <input
          type="number"
          min={min}
          max={max}
          value={Math.round(value / 1000)}
          onChange={(e) => onChange(Math.max(min, Number(e.target.value)) * 1000)}
          style={{ width: 80, textAlign: "right" }}
        />
        <span className="muted" style={{ fontSize: 11 }}>
          秒
        </span>
      </div>
    </div>
  );
}

function ExportBtn({
  draft,
  scope,
  format,
  label,
}: {
  draft: Settings;
  scope: string;
  format: string;
  label: string;
}) {
  const exportAction = useAction(
    async () => {
      await api.exportData(scope, format, draft.defaultRange, "");
    },
    { okText: "已导出" },
  );
  return (
    <FxButton variant="quiet" size="small" action={exportAction} busyLabel="导出中…">
      {label}
    </FxButton>
  );
}

/** Launcher detection feedback(不保存也即时探测)。 */
function LauncherProbe() {
  const [status, setStatus] = useState<LauncherStatus | null>(null);
  const launchAction = useAction(
    async () => {
      const r = await api.zcodeLaunch();
      setStatus(r.snapshot.launcher ?? null);
    },
    { okText: "已唤醒" },
  );
  useEffect(() => {
    api
      .zcodeStatus()
      .then((s: ProviderSnapshot) => setStatus(s.launcher ?? null))
      .catch(() => {});
  }, []);
  const label = status
    ? status.state === "running"
      ? `运行中${status.version ? ` · ${status.version}` : ""}`
      : status.state === "not_installed"
        ? "未检测到 ZCode"
        : "未运行"
    : "检测中…";
  return (
    <div className="switch-row">
      <div>
        <div>当前状态:{label}</div>
        <div className="desc">
          {status?.exePath ? `${status.exePath}(${status.detectedVia ?? "auto"})` : "自动检测常见安装位置"}
        </div>
      </div>
      <span style={{ display: "flex", gap: 8 }}>
        <FxButton
          variant="quiet"
          size="small"
          action={launchAction}
          disabled={status?.state === "not_installed"}
          busyLabel="启动中…"
        >
          {status?.state === "running" ? "聚焦窗口" : "启动 ZCode"}
        </FxButton>
      </span>
    </div>
  );
}

/** 火山凭据:保存到系统 keyring,值永不出现在 UI/文件。 */
function VolcengineCredentials() {
  const [status, setStatus] = useState<CredentialsStatusDto | null>(null);
  const [ak, setAk] = useState("");
  const [sk, setSk] = useState("");
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null);

  const refresh = () => api.volcengineCredentialsStatus().then(setStatus).catch(() => {});
  useEffect(() => {
    refresh();
  }, []);

  const save = useAction(
    async () => {
      try {
        await api.volcengineCredentialsSet(ak.trim(), sk.trim());
        setAk("");
        setSk("");
        setMsg({ ok: true, text: "已保存到系统凭据管理器" });
        refresh();
      } catch (e) {
        setMsg({ ok: false, text: String(e) });
        throw e; // let the button show its error phase
      }
    },
    { okText: "已保存" },
  );

  const test = useAction(
    async () => {
      try {
        const text = await api.volcengineTest();
        setMsg({ ok: true, text });
      } catch (e) {
        setMsg({ ok: false, text: String(e) });
        throw e;
      }
    },
    { okText: "连接正常" },
  );

  const clear = useAction(
    async () => {
      try {
        await api.volcengineCredentialsClear();
        setMsg({ ok: true, text: "已清除凭据" });
        refresh();
      } catch (e) {
        setMsg({ ok: false, text: String(e) });
        throw e;
      }
    },
    { okText: "已清除" },
  );

  return (
    <div style={{ marginTop: 12 }}>
      <div className="panel-title" style={{ fontSize: 12 }}>
        凭据(AccessKey / SecretKey)
      </div>
      <div className="desc" style={{ marginBottom: 6 }}>
        {status?.configured
          ? `已配置(${status.akHint ?? "***"}) · 存储:${status.backend}。Secret 永远不写入任何文件或日志。`
          : "未配置。需要具备费用中心只读权限(BillingCenterReadOnlyAccess)的 IAM AccessKey。"}
      </div>
      <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 8 }}>
        <TextField label={null} value={ak} onChange={(e) => setAk(e.target.value)} placeholder="AccessKey ID" />
        <TextField
          label={null}
          value={sk}
          onChange={(e) => setSk(e.target.value)}
          placeholder="Secret Access Key"
          type="password"
        />
      </div>
      <div style={{ display: "flex", gap: 8, marginTop: 8, alignItems: "center" }}>
        <FxButton
          variant="primary"
          size="small"
          action={save}
          busyLabel="保存中…"
          disabled={ak.trim() === "" || sk.trim() === ""}
        >
          保存凭据
        </FxButton>
        <FxButton variant="quiet" size="small" action={test} busyLabel="测试中…">
          测试连接
        </FxButton>
        <FxButton
          variant="danger"
          size="small"
          action={clear}
          busyLabel="清除中…"
          disabled={!status?.configured}
        >
          清除
        </FxButton>
        {msg && (
          <motion.span
            key={msg.text}
            initial={{ opacity: 0, y: -3 }}
            animate={{ opacity: 1, y: 0 }}
            style={{ fontSize: 11, color: msg.ok ? "var(--zup-text-3)" : "var(--zup-danger)" }}
          >
            {msg.text}
          </motion.span>
        )}
      </div>
    </div>
  );
}

function DiagnosePanel({ diag }: { diag: DiagnoseDto }) {
  return (
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
        <span>{diag.lastRefreshMs ? formatRelative(diag.lastRefreshMs) : "—"}</span>
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
              {formatDateTime(r.tsMs)} · {r.model} · in {r.inputTokens} · out {r.outputTokens} ·
              reasoning {r.reasoningTokens === null ? "unavailable" : r.reasoningTokens} · cache{" "}
              {r.cacheReadTokens === null ? "unavailable" : r.cacheReadTokens}
            </div>
          ))}
        </>
      )}
    </div>
  );
}

/** API 价格表主体(从原设置页抽出,行为不变)。 */
function PriceSection({
  table,
  setTable,
  draft,
  set,
  refreshMsg,
  setRefreshMsg,
  expandedModel,
  setExpandedModel,
}: {
  table: PricingTableDto | null;
  setTable: (t: PricingTableDto | null) => void;
  draft: Settings;
  set: (patch: Partial<Settings>) => void;
  refreshMsg: { ok: boolean; text: string } | null;
  setRefreshMsg: (m: { ok: boolean; text: string } | null) => void;
  expandedModel: string | null;
  setExpandedModel: (m: string | null) => void;
}) {
  const refreshAction = useAction(
    async () => {
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
          throw new Error(r.error ?? "更新失败");
        }
      } catch (e) {
        if (!(e instanceof Error)) setRefreshMsg({ ok: false, text: String(e) });
        throw e; // propagate so the button enters its error phase
      } finally {
        try {
          setTable(await api.pricingTable());
        } catch {
          /* ignore */
        }
      }
    },
    { okText: "已更新" },
  );

  return (
    <>
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 10,
          flexWrap: "wrap",
          marginBottom: 10,
        }}
      >
        <FxButton action={refreshAction} busyLabel="更新中…">
          更新价格
        </FxButton>
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
                      storage: e.cacheStoragePerM != null ? String(e.cacheStoragePerM) : "",
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
                      <div className="desc">没有官方价格表条目,手动覆盖后可参与成本估算</div>
                    </div>
                    <FxButton
                      variant="quiet"
                      size="small"
                      onClick={() => setExpandedModel(expandedModel === m ? null : m)}
                    >
                      手动覆盖
                    </FxButton>
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
            按官方 API 单价估算 · 非实际 Billing · Reasoning 不重复计费
          </div>
        </>
      )}
    </>
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

  const save = useAction(
    async () => {
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
    },
    { okText: "已保存" },
  );

  const clear = useAction(
    async () => {
      const t = await api.pricingOverride(model, null);
      onSaved(t);
    },
    { okText: "已恢复" },
  );

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
          <FxButton
            variant="quiet"
            size="small"
            action={clear}
            busyLabel="处理中…"
            title={overridden ? "恢复官方价,清除覆盖" : "清除该模型的覆盖(若存在)"}
          >
            {overridden ? "恢复官方价" : "清除覆盖"}
          </FxButton>
          <FxButton variant="primary" size="small" action={save} busyLabel="保存中…">
            保存
          </FxButton>
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
