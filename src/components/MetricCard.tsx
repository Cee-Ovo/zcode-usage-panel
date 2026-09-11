import type { ReactNode } from "react";
import { motion } from "motion/react";
import { Glass } from "open-glass-ui";
import { cardVariants, softSpring } from "../lib/motion";
import { formatTps } from "../lib/format";
import type { SpeedWindowStats } from "../lib/types";
const MotionGlass = motion.create(Glass);

/** Metric card with an optional tooltip (title attr keeps it dependency-free). */
export function MetricCard({
  label,
  value,
  sub,
  hint,
  unavailable = false,
  className = "",
  glass = false,
  layoutEnabled = true,
}: {
  label: string;
  value: ReactNode;
  sub?: ReactNode;
  hint?: string;
  unavailable?: boolean;
  className?: string;
  glass?: boolean;
  layoutEnabled?: boolean;
}) {
  const Surface = glass ? MotionGlass : motion.div;
  return (
    <Surface
      {...(glass ? { material: "regular" as const, renderer: "css" as const, interactive: false } : {})}
      layout={layoutEnabled}
      variants={layoutEnabled ? cardVariants : undefined}
      whileHover={{ y: -1 }}
      transition={layoutEnabled ? softSpring : { duration: 0.12 }}
      className={`metric-card ${glass ? "sample-glass" : "liquid-metric"} ${className}`.trim()}
      title={hint}
      onPointerMove={(event) => {
        if (glass) return;
        const rect = event.currentTarget.getBoundingClientRect();
        event.currentTarget.style.setProperty("--liquid-x", `${event.clientX - rect.left}px`);
        event.currentTarget.style.setProperty("--liquid-y", `${event.clientY - rect.top}px`);
      }}
      onPointerLeave={(event) => {
        if (glass) return;
        event.currentTarget.style.setProperty("--liquid-x", "50%");
        event.currentTarget.style.setProperty("--liquid-y", "0px");
      }}
    >
      <div className="label">
        {label}
        {hint && <InfoDot text={hint} />}
      </div>
      <div className="value" style={unavailable ? { fontStyle: "italic", fontSize: 15 } : undefined}>
        {value}
      </div>
      {sub && <div className="sub">{sub}</div>}
    </Surface>
  );
}

/** Small "i" affordance for statistical-convention tooltips. */
export function InfoDot({ text }: { text: string }) {
  return (
    <span
      className="muted"
      style={{
        display: "inline-flex",
        alignItems: "center",
        justifyContent: "center",
        width: 13,
        height: 13,
        borderRadius: "50%",
        border: "1px solid currentColor",
        fontSize: 9,
        cursor: "help",
        flex: "none",
      }}
      title={text}
    >
      i
    </span>
  );
}

const DAY_MS = 86_400_000;

/** 速度卡第二行:近 24h 与近 7 天的加权 tps 对比。继承父级 .sub 的字号与
 * 颜色,不加彩色强调,与卡片原有辅助行同一密度;7 天窗口无样本时整行隐藏。 */
export function SpeedTrendLine({ windows }: { windows: SpeedWindowStats[] | undefined }) {
  const h24 = windows?.find((w) => w.windowMs === DAY_MS);
  const d7 = windows?.find((w) => w.windowMs === 7 * DAY_MS);
  if (!d7 || d7.speedSamples === 0 || d7.speedTps === null) return null;
  const h24Ready = !!h24 && h24.speedSamples > 0 && h24.speedTps !== null;
  const delta =
    h24Ready && d7.speedTps > 0 ? (h24!.speedTps! - d7.speedTps) / d7.speedTps : null;
  return (
    <div
      title={`近 24 小时 / 近 7 天各自的加权 tps(固定窗口,与当前所选时间范围无关)。\n样本 ${h24?.speedSamples ?? 0} / ${d7.speedSamples} 条请求。`}
    >
      近24h {h24Ready ? formatTps(h24!.speedTps) : "—"}
      <span className="muted"> · </span>
      近7天 {formatTps(d7.speedTps)}
      {delta !== null && (
        <span className="muted">
          {" · "}
          {Math.abs(delta) < 0.03 ? "持平" : `${delta > 0 ? "↑" : "↓"}${Math.round(Math.abs(delta) * 100)}%`}
        </span>
      )}
    </div>
  );
}
