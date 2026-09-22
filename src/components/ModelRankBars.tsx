import type { CSSProperties, ReactNode } from "react";
import { motion } from "motion/react";
import { listItemVariants, rowGestures, softSpring } from "../lib/motion";

/**
 * 模型排行横向渐变条(参考 Usage panel web 版 K3 排行图):
 * 名称列 + 渐变条 + 条端数值,副行承载明细与成本入口。
 * ZCode 仪表盘与本地源分区的「模型排行」共用,行为由调用方注入。
 */
export function RankBarRow({
  label,
  labelTitle,
  total,
  pct,
  sub,
  onActivate,
  activateTitle,
}: {
  label: ReactNode;
  labelTitle?: string;
  /** 已格式化的总量文案,渲染在条端。 */
  total: string;
  /** 0..1,相对本组最大值;内部留出数值标签空间。 */
  pct: number;
  sub?: ReactNode;
  onActivate?: () => void;
  activateTitle?: string;
}) {
  const width = `${Math.max(2, Math.min(88, pct * 88)).toFixed(1)}%`;
  const interactive = !!onActivate;
  return (
    <motion.div
      variants={listItemVariants}
      initial="initial"
      animate="enter"
      exit="exit"
      {...(interactive ? rowGestures : {})}
      transition={softSpring}
      className={`rank-row ${interactive ? "rank-row--link" : ""}`}
      {...(interactive
        ? {
            role: "button",
            tabIndex: 0,
            onKeyDown: (e: React.KeyboardEvent<HTMLDivElement>) => {
              if (e.target === e.currentTarget && (e.key === "Enter" || e.key === " ")) {
                e.preventDefault();
                onActivate?.();
              }
            },
            onClick: () => onActivate?.(),
            title: activateTitle,
          }
        : {})}
    >
      <div className="rank-line">
        <span className="rank-name" title={labelTitle}>
          {label}
        </span>
        <span className="rank-track" style={{ "--rank-pct": width } as CSSProperties}>
          <span className="rank-fill" style={{ width }} />
          <span className="rank-value">{total}</span>
        </span>
      </div>
      {sub != null && <div className="rank-sub">{sub}</div>}
    </motion.div>
  );
}
