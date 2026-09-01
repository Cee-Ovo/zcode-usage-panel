import { useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import { AnimatedNumber } from "./AnimatedNumber";
import { InfoDot } from "./MetricCard";
import { FxChip } from "./fx";
import { ProviderDetailModal } from "./QuotaSection";
import { useStore } from "../lib/store";
import type { ModelUsageRow } from "../lib/types";
import { PROVIDER_STATUS_LABELS } from "../lib/types";
import { displayModelName } from "../lib/modelDisplay";
import { formatFull, formatTokens } from "../lib/format";
import { cardVariants, softSpring } from "../lib/motion";

/**
 * Codex 本地 Token 用量(仪表盘主要指标区)。
 *
 * 展示口径 — 三者绝不混算:
 * - ZCode 总 Token:上方指标卡(ZCode 本地 usage 记录)。
 * - Codex Token(本面板):Codex 客户端本地 session 日志统计
 *   (<CODEX_HOME>/sessions/.../rollout-*.jsonl,离线读取)。
 * - Codex 官方套餐额度:服务额度区 rate_limits(5 小时窗口/周额度)。
 *
 * 数据不可得时显示明确状态(unavailable / 未启用),绝不伪装成 0;
 * 数据源存在且真实统计为 0 时正常显示 0。
 */

const CODEX_EXPLAIN =
  "Codex Token = 本地 Codex 客户端 session 日志统计(Input+Cached+CacheWrite+Output+Reasoning,离线读取)。\n" +
  "它与「ZCode 总 Token」分开统计、互不计入;与服务额度区的 Codex 官方套餐额度(5 小时/周 rate_limits)也是两个独立指标。\n" +
  "本地 Token 统计 ≠ 官方剩余额度 ≠ 实际 Billing。";

export function CodexUsagePanel() {
  const codex = useStore((s) => s.providers.find((p) => p.provider === "codex") ?? null);
  const loading = useStore((s) => s.providers.length === 0);
  const [expanded, setExpanded] = useState(false);
  const [detail, setDetail] = useState(false);

  const usage = codex?.localUsage ?? null;

  return (
    <motion.div className="codex-panel" variants={cardVariants}>
      <div className="panel-title" style={{ marginBottom: 0 }}>
        Codex 本地 Token 用量
        <span className="muted" style={{ fontWeight: 500, fontSize: 10.5 }}>
          session 日志统计 · 不计入 ZCode 总 Token
        </span>
        <span className="right">
          <FxChip onClick={() => setDetail(true)} title="查看 Codex 详情(官方额度 / 模型明细)">
            详情 ›
          </FxChip>
        </span>
      </div>

      {loading ? (
        <div className="muted" style={{ fontSize: 11.5, padding: "10px 0" }}>
          正在初始化 Provider…
        </div>
      ) : !codex ? (
        <UnavailableLine
          text="Codex 监控未启用"
          hint="在「设置 → Codex」中开启后,这里会显示本地 session 日志统计。"
        />
      ) : !usage ? (
        <UnavailableLine
          text={`本地统计 unavailable(${PROVIDER_STATUS_LABELS[codex.status] ?? codex.status})`}
          hint={codex.error ?? "Codex 数据目录中没有可解析的 session 日志。"}
        />
      ) : (
        <>
          <div className="codex-headline">
            <span className="muted" style={{ fontSize: 12 }}>
              今日
            </span>
            <span className="big">
              <AnimatedNumber value={usage.today.totalTokens} format={formatTokens} />
            </span>
            <span className="muted" style={{ fontSize: 11 }}>
              tokens
            </span>
            <span style={{ marginLeft: "auto" }}>
              <InfoDot text={CODEX_EXPLAIN} />
            </span>
          </div>

          <div className="codex-mini">
            <span>
              <span className="k">7 天 </span>
              {formatTokens(usage.last7d.totalTokens)}
            </span>
            <span>
              <span className="k">累计 </span>
              {formatTokens(usage.allTime.totalTokens)}
            </span>
            <span>
              <span className="k">Sessions </span>
              {formatFull(usage.sessions)}
            </span>
            <span>
              <span className="k">今日请求 </span>
              {formatFull(usage.today.requests)}
            </span>
          </div>

          <div className="codex-breakdown">
            <span className="codex-part">Input {formatTokens(usage.today.inputTokens)}</span>
            <span className="codex-part">Cached {formatTokens(usage.today.cachedInputTokens)}</span>
            <span className="codex-part">Cache 写 {formatTokens(usage.today.cacheWriteTokens)}</span>
            <span className="codex-part">Output {formatTokens(usage.today.outputTokens)}</span>
            <span className="codex-part">Reasoning {formatTokens(usage.today.reasoningTokens)}</span>
          </div>

          {usage.models.length > 0 && (
            <CodexModelList models={usage.models} expanded={expanded} onToggle={setExpanded} />
          )}

          <div className="codex-note">
            来自本地 Codex session 日志 · 与官方套餐额度(5 小时/周)分开统计 · 不等于实际
            Billing · 不计入上方 ZCode 总 Token
          </div>
        </>
      )}

      <AnimatePresence>
        {detail && codex && (
          <ProviderDetailModal key="codex-detail" provider="codex" onClose={() => setDetail(false)} />
        )}
      </AnimatePresence>
    </motion.div>
  );
}

function UnavailableLine({ text, hint }: { text: string; hint?: string }) {
  return (
    <div style={{ padding: "8px 0 2px" }}>
      <div className="unavailable" style={{ fontSize: 12.5 }}>
        {text}
      </div>
      {hint && (
        <div className="muted" style={{ fontSize: 10.5, marginTop: 3 }} title={hint}>
          {hint}
        </div>
      )}
    </div>
  );
}

/** Codex 模型用量(累计,按总量降序)。前 3 行 + 展开;名称统一带（Codex）标记。 */
function CodexModelList({
  models,
  expanded,
  onToggle,
}: {
  models: ModelUsageRow[];
  expanded: boolean;
  onToggle: (v: boolean) => void;
}) {
  const sorted = [...models].sort((a, b) => b.breakdown.totalTokens - a.breakdown.totalTokens);
  const top = expanded ? sorted : sorted.slice(0, 3);
  const max = sorted[0]?.breakdown.totalTokens || 1;
  return (
    <div className="codex-models">
      <AnimatePresence initial={false}>
        {top.map((m) => (
          <motion.div
            key={m.model}
            className="codex-model-row"
            layout="position"
            initial={{ opacity: 0, y: 4 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0 }}
            transition={softSpring}
            title={
              `${displayModelName(m.model, "codex")} · 累计\n` +
              `Total ${formatFull(m.breakdown.totalTokens)}\n` +
              `Input ${formatFull(m.breakdown.inputTokens)} · Cached ${formatFull(m.breakdown.cachedInputTokens)}\n` +
              `Output ${formatFull(m.breakdown.outputTokens)} · Reasoning ${formatFull(m.breakdown.reasoningTokens)}`
            }
          >
            <span className="name">{displayModelName(m.model, "codex")}</span>
            <div className="share-track" style={{ flex: 1, marginTop: 0 }}>
              <div
                className="share-fill"
                style={{
                  width: `${
                    m.breakdown.totalTokens > 0
                      ? Math.max(2, (m.breakdown.totalTokens / max) * 100)
                      : 0
                  }%`,
                }}
              />
            </div>
            <span style={{ width: 64, textAlign: "right", fontWeight: 600 }}>
              {formatTokens(m.breakdown.totalTokens)}
            </span>
          </motion.div>
        ))}
      </AnimatePresence>
      {sorted.length > 3 && (
        <div style={{ marginTop: 4 }}>
          <FxChip onClick={() => onToggle(!expanded)}>
            {expanded ? "收起" : `展开全部 (${sorted.length})`}
          </FxChip>
        </div>
      )}
    </div>
  );
}
