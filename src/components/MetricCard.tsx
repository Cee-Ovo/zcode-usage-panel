import type { ReactNode } from "react";
import { motion } from "motion/react";
import { cardVariants, softSpring } from "../lib/motion";

/** Metric card with an optional tooltip (title attr keeps it dependency-free). */
export function MetricCard({
  label,
  value,
  sub,
  hint,
  unavailable = false,
}: {
  label: string;
  value: ReactNode;
  sub?: ReactNode;
  hint?: string;
  unavailable?: boolean;
}) {
  return (
    <motion.div
      layout
      variants={cardVariants}
      whileHover={{ y: -2 }}
      transition={softSpring}
      className="metric-card"
      title={hint}
    >
      <div className="label">
        {label}
        {hint && <InfoDot text={hint} />}
      </div>
      <div className="value" style={unavailable ? { fontStyle: "italic", fontSize: 15 } : undefined}>
        {value}
      </div>
      {sub && <div className="sub">{sub}</div>}
    </motion.div>
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
