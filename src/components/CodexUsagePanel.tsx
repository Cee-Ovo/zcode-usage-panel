import { useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import { Glass, SegmentedControl } from "open-glass-ui";
import { AnimatedNumber } from "./AnimatedNumber";
import { InfoDot } from "./MetricCard";
import { FxChip } from "./fx";
import { ProviderDetailModal } from "./QuotaSection";
import { useStore } from "../lib/store";
import type { LocalUsage, LocalUsageRange, ModelUsageRow } from "../lib/types";
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
  "Codex Token = 本地 Codex 客户端 session 日志中提供的 total_tokens 原值;Cached / Cache Write 作为分项展示,不会重复加到总量中。\n" +
  "它与「ZCode 总 Token」分开统计、互不计入;与服务额度区的 Codex 官方套餐额度(5 小时/周 rate_limits)也是两个独立指标。\n" +
  "本地 Token 统计 ≠ 官方剩余额度 ≠ 实际 Billing。";

const MotionGlass = motion.create(Glass);

const CODEX_RANGE_KEYS = ["today", "60m", "24h", "7d", "30d", "all"] as const;
type CodexRangeKey = (typeof CODEX_RANGE_KEYS)[number];

const CODEX_RANGE_LABELS: Record<CodexRangeKey, string> = {
  today: "今天",
  "60m": "60 分钟",
  "24h": "24 小时",
  "7d": "7 天",
  "30d": "30 天",
  all: "全部",
};

/** Compatibility fallback while a provider snapshot from the old DTO is still in memory. */
function selectedUsageRange(usage: LocalUsage, key: CodexRangeKey): LocalUsageRange | null {
  const exact = usage.ranges?.find((range) => range.key === key);
  if (exact) return exact;
  if (key === "today") {
    return { key, breakdown: usage.today, sessions: usage.sessions, models: usage.models };
  }
  if (key === "7d") {
    return { key, breakdown: usage.last7d, sessions: usage.sessions, models: usage.models };
  }
  if (key === "all") {
    return { key, breakdown: usage.allTime, sessions: usage.sessions, models: usage.models };
  }
  return null;
}

export function CodexUsagePanel() {
  const codex = useStore((s) => s.providers.find((p) => p.provider === "codex") ?? null);
  const loading = useStore((s) => s.providers.length === 0);
  const [expanded, setExpanded] = useState(false);
  const [detail, setDetail] = useState(false);
  const [rangeKey, setRangeKey] = useState<CodexRangeKey>("today");

  const usage = codex?.localUsage ?? null;
  const selected = usage ? selectedUsageRange(usage, rangeKey) : null;

  return (
    <MotionGlass
      className="codex-panel sample-glass"
      renderer="css"
      material="regular"
      interactive={false}
      variants={cardVariants}
      whileHover={{ y: -1 }}
      transition={softSpring}
    >
      <div className="panel-title codex-heading">
        <span className="codex-heading-title">Codex 本地 Token 用量</span>
        <span className="muted codex-heading-subtitle">
          session 日志统计 · 不计入 ZCode 总 Token
        </span>
        <span className="right codex-heading-actions">
          <FxChip className="codex-detail-chip" onClick={() => setDetail(true)} title="查看 Codex 详情(官方额度 / 模型明细)">
            详情 ›
          </FxChip>
        </span>
      </div>

      {loading ? (
        <div className="muted codex-loading">
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
          <div className="codex-range-control">
            <SegmentedControl
              aria-label="Codex 本地 Token 时间范围"
              className="codex-range-tabs"
              value={rangeKey}
              onValueChange={(value) => {
                setRangeKey(value);
                setExpanded(false);
              }}
              items={CODEX_RANGE_KEYS.map((key) => ({
                value: key,
                label: CODEX_RANGE_LABELS[key],
              }))}
            />
          </div>

          <AnimatePresence mode="wait" initial={false}>
            {selected ? (
              <motion.div
                key={rangeKey}
                className="codex-range-content"
                initial={{ opacity: 0, y: 6 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0, y: -4 }}
                transition={{ duration: 0.18, ease: [0.22, 1, 0.36, 1] }}
              >
                <div className="codex-headline">
                  <span className="muted codex-headline-label">
                    {CODEX_RANGE_LABELS[rangeKey]}
                  </span>
                  <span className="big codex-headline-value">
                    <AnimatedNumber value={selected.breakdown.totalTokens} format={formatTokens} />
                  </span>
                  <span className="muted codex-headline-unit">
                    tokens
                  </span>
                  <span className="codex-headline-info">
                    <InfoDot text={CODEX_EXPLAIN} />
                  </span>
                </div>

                <div className="codex-mini">
                  <span className="codex-stat">
                    <span className="k">Sessions </span>
                    {formatFull(selected.sessions)}
                  </span>
                  <span className="codex-stat">
                    <span className="k">请求 </span>
                    {formatFull(selected.breakdown.requests)}
                  </span>
                  <span className="codex-stat">
                    <span className="k">模型 </span>
                    {formatFull(selected.models.length)}
                  </span>
                </div>

                <div className="codex-breakdown">
                  <span className="codex-part">
                    Input {formatTokens(selected.breakdown.inputTokens)}
                  </span>
                  <span className="codex-part">
                    Cached {formatTokens(selected.breakdown.cachedInputTokens)}
                  </span>
                  <span className="codex-part">
                    Cache 写 {formatTokens(selected.breakdown.cacheWriteTokens)}
                  </span>
                  <span className="codex-part">
                    Output {formatTokens(selected.breakdown.outputTokens)}
                  </span>
                  <span className="codex-part">
                    Reasoning {formatTokens(selected.breakdown.reasoningTokens)}
                  </span>
                </div>

                {selected.models.length > 0 && (
                  <CodexModelList
                    models={selected.models}
                    rangeLabel={CODEX_RANGE_LABELS[rangeKey]}
                    expanded={expanded}
                    onToggle={setExpanded}
                  />
                )}
              </motion.div>
            ) : (
              <motion.div
                key={`${rangeKey}-unavailable`}
                initial={{ opacity: 0 }}
                animate={{ opacity: 1 }}
                exit={{ opacity: 0 }}
              >
                <UnavailableLine
                  text={`${CODEX_RANGE_LABELS[rangeKey]}统计 unavailable`}
                  hint="等待 Codex Provider 完成新版范围统计后自动出现。"
                />
              </motion.div>
            )}
          </AnimatePresence>

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
    </MotionGlass>
  );
}

function UnavailableLine({ text, hint }: { text: string; hint?: string }) {
  return (
    <div className="codex-unavailable-line">
      <div className="unavailable codex-unavailable-title">
        {text}
      </div>
      {hint && (
        <div className="muted codex-unavailable-hint" title={hint}>
          {hint}
        </div>
      )}
    </div>
  );
}

/** Codex 模型用量(当前范围,按总量降序)。前 3 行 + 展开;名称统一带（Codex）标记。 */
function CodexModelList({
  models,
  rangeLabel,
  expanded,
  onToggle,
}: {
  models: ModelUsageRow[];
  rangeLabel: string;
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
              `${displayModelName(m.model, "codex")} · ${rangeLabel}\n` +
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
            <span className="codex-model-value">
              {formatTokens(m.breakdown.totalTokens)}
            </span>
          </motion.div>
        ))}
      </AnimatePresence>
      {sorted.length > 3 && (
        <div className="codex-model-toggle">
          <FxChip className="codex-toggle-chip" onClick={() => onToggle(!expanded)}>
            {expanded ? "收起" : `展开全部 (${sorted.length})`}
          </FxChip>
        </div>
      )}
    </div>
  );
}
