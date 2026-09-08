import { useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import { Glass } from "open-glass-ui";
import { AnimatedNumber } from "./AnimatedNumber";
import { InfoDot } from "./MetricCard";
import { FxChip } from "./fx";
import { ProviderDetailModal } from "./QuotaSection";
import { useStore } from "../lib/store";
import type { LocalUsage, LocalUsageRange, ModelUsageRow } from "../lib/types";
import { PROVIDER_STATUS_LABELS, RANGE_LABELS } from "../lib/types";
import { displayModelName, type ModelSource } from "../lib/modelDisplay";
import { formatFull, formatTokens } from "../lib/format";
import { cardVariants, softSpring } from "../lib/motion";

/**
 * 通用「本地 Harness Token 用量」分区面板(Codex / DSH 共用)。
 *
 * 展示口径 —— 三者绝不混算:
 * - ZCode 总 Token:ZCode 分区指标卡(ZCode 本地 usage 记录)。
 * - 本面板 Token:对应客户端本地 session 日志统计(离线读取),
 *   不计入 ZCode 总 Token,也不计入任何官方套餐额度。
 * - 官方套餐额度:服务额度区(如 Codex rate_limits)。
 *
 * 数据不可得时显示明确状态(unavailable / 未启用 / 未安装),绝不伪装成 0;
 * 数据源存在且真实统计为 0 时正常显示 0。
 *
 * 时间范围跟随仪表盘全局选择(单一时间心智,切换分区不换口径)。
 */

const MotionGlass = motion.create(Glass);

export interface LocalUsagePanelProps {
  /** Provider snapshot id ("codex" | "dsh"). */
  provider: "codex" | "dsh";
  /** 分区标题,如「Codex 本地 Token 用量」。 */
  title: string;
  /** 标题旁的口径小字。 */
  subtitle: string;
  /** ⓘ 完整口径说明。 */
  explain: string;
  /** 模型名来源徽标。 */
  modelSource: ModelSource;
  /** 未启用时的提示。 */
  notEnabledHint: string;
  /** 面板底部口径脚注。 */
  footnote: string;
  /** 详情弹窗悬浮提示。 */
  detailTitle: string;
  className?: string;
}

/** Compatibility fallback while a provider snapshot from the old DTO is still in memory. */
function selectedUsageRange(
  usage: LocalUsage,
  key: string,
): LocalUsageRange | null {
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

export function LocalUsagePanel({
  provider,
  title,
  subtitle,
  explain,
  modelSource,
  notEnabledHint,
  footnote,
  detailTitle,
  className = "",
}: LocalUsagePanelProps) {
  const snap = useStore((s) => s.providers.find((p) => p.provider === provider) ?? null);
  const loading = useStore((s) => s.providers.length === 0);
  const rangeKey = useStore((s) => s.rangeKey);
  const [expanded, setExpanded] = useState(false);
  const [detail, setDetail] = useState(false);

  const usage = snap?.localUsage ?? null;
  const selected = usage ? selectedUsageRange(usage, rangeKey) : null;
  const rangeLabel =
    RANGE_LABELS[rangeKey as keyof typeof RANGE_LABELS] ?? rangeKey;

  return (
    <MotionGlass
      className={`codex-panel sample-glass local-usage-panel provider-${provider} ${className}`.trim()}
      renderer="css"
      material="regular"
      interactive={false}
      variants={cardVariants}
      whileHover={{ y: -1 }}
      transition={softSpring}
    >
      <div className="panel-title codex-heading">
        <span className="codex-heading-title">{title}</span>
        <span className="muted codex-heading-subtitle">{subtitle}</span>
        <span className="right codex-heading-actions">
          <FxChip
            className="codex-detail-chip"
            onClick={() => setDetail(true)}
            title={detailTitle}
          >
            详情 ›
          </FxChip>
        </span>
      </div>

      {loading ? (
        <div className="muted codex-loading">正在初始化 Provider…</div>
      ) : !snap ? (
        <UnavailableLine
          text={`${provider === "codex" ? "Codex" : "DSH"} 监控未启用`}
          hint={notEnabledHint}
        />
      ) : !usage ? (
        <UnavailableLine
          text={`本地统计 unavailable(${PROVIDER_STATUS_LABELS[snap.status] ?? snap.status})`}
          hint={snap.error ?? "数据目录中没有可解析的 session 日志。"}
        />
      ) : (
        <>
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
                  <span className="muted codex-headline-label">{rangeLabel}</span>
                  <span className="big codex-headline-value">
                    <AnimatedNumber
                      value={selected.breakdown.totalTokens}
                      format={formatTokens}
                    />
                  </span>
                  <span className="muted codex-headline-unit">tokens</span>
                  <span className="codex-headline-info">
                    <InfoDot text={explain} />
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
                  <LocalModelList
                    models={selected.models}
                    rangeLabel={rangeLabel}
                    modelSource={modelSource}
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
                  text={`${rangeLabel}统计 unavailable`}
                  hint="等待 Provider 完成新版范围统计后自动出现。"
                />
              </motion.div>
            )}
          </AnimatePresence>

          <div className="codex-note">{footnote}</div>
        </>
      )}

      <AnimatePresence>
        {detail && snap && (
          <ProviderDetailModal
            key={`${provider}-detail`}
            provider={provider}
            onClose={() => setDetail(false)}
          />
        )}
      </AnimatePresence>
    </MotionGlass>
  );
}

function UnavailableLine({ text, hint }: { text: string; hint?: string }) {
  return (
    <div className="codex-unavailable-line">
      <div className="unavailable codex-unavailable-title">{text}</div>
      {hint && (
        <div className="muted codex-unavailable-hint" title={hint}>
          {hint}
        </div>
      )}
    </div>
  );
}

/** 模型用量(当前范围,按总量降序)。前 3 行 + 展开;名称统一带来源徽标。 */
function LocalModelList({
  models,
  rangeLabel,
  modelSource,
  expanded,
  onToggle,
}: {
  models: ModelUsageRow[];
  rangeLabel: string;
  modelSource: ModelSource;
  expanded: boolean;
  onToggle: (v: boolean) => void;
}) {
  const sorted = [...models].sort(
    (a, b) => b.breakdown.totalTokens - a.breakdown.totalTokens,
  );
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
              `${displayModelName(m.model, modelSource)} · ${rangeLabel}\n` +
              `Total ${formatFull(m.breakdown.totalTokens)}\n` +
              `Input ${formatFull(m.breakdown.inputTokens)} · Cached ${formatFull(m.breakdown.cachedInputTokens)}\n` +
              `Output ${formatFull(m.breakdown.outputTokens)} · Reasoning ${formatFull(m.breakdown.reasoningTokens)}`
            }
          >
            <span className="name">{displayModelName(m.model, modelSource)}</span>
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
