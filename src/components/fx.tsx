import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type ButtonHTMLAttributes,
  type CSSProperties,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
} from "react";
import { Button } from "open-glass-ui";
import { motion, useReducedMotion } from "motion/react";
import {
  buttonGestures,
  popSpring,
  pressSpring,
  quietButtonGestures,
  softSpring,
  titleSpring,
} from "../lib/motion";

/**
 * Unified interaction layer ("fx") for the whole shell.
 *
 * Every pressable surface funnels through here so hover / press / release,
 * ripples, busy/success/error feedback and reduced-motion behavior stay
 * consistent instead of being re-tuned per page. Visuals still come from
 * open-glass-ui + global.css — this layer only adds motion semantics:
 *
 * - hover: small lift, springy
 * - press: clear compression + sink, quick spring
 * - release: soft elastic settle (pressSpring)
 * - primary buttons: one-shot light sweep on hover + tap ripple
 * - async actions: busy spinner → ok pop / error shake
 *
 * All effects respect prefers-reduced-motion (hook guards + MotionConfig
 * "user"), ripples self-remove on animationend (no DOM build-up), and
 * keyboard focus rings / disabled states are preserved untouched.
 */

// ---------------------------------------------------------------------------
// useAction — async action state machine with unmount-safe setState
// ---------------------------------------------------------------------------

export type ActionPhase = "idle" | "ok" | "error";

export function useAction<A extends unknown[]>(
  fn: (...args: A) => Promise<unknown>,
  opts?: { okText?: string; resetMs?: number },
) {
  const [busy, setBusy] = useState(false);
  const [phase, setPhase] = useState<ActionPhase>("idle");
  const busyRef = useRef(false);
  const alive = useRef(true);
  const timer = useRef<number | undefined>(undefined);
  // Always invoke the latest closure (callers pass inline arrows capturing
  // fresh state like the settings draft) while keeping `run` referentially
  // stable.
  const fnRef = useRef(fn);
  fnRef.current = fn;

  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      window.clearTimeout(timer.current);
    };
  }, []);

  const run = useCallback(
    async (...args: A) => {
      if (busyRef.current) return;
      busyRef.current = true;
      setBusy(true);
      setPhase("idle");
      window.clearTimeout(timer.current);
      try {
        await fnRef.current(...args);
        if (!alive.current) return;
        setPhase("ok");
        timer.current = window.setTimeout(
          () => alive.current && setPhase("idle"),
          opts?.resetMs ?? 1500,
        );
      } catch (e) {
        console.warn("[fx] action failed:", e);
        if (!alive.current) return;
        setPhase("error");
        timer.current = window.setTimeout(
          () => alive.current && setPhase("idle"),
          opts?.resetMs ?? 2400,
        );
      } finally {
        busyRef.current = false;
        if (alive.current) setBusy(false);
      }
    },
    [opts?.resetMs],
  );

  return { run, busy, phase };
}

// ---------------------------------------------------------------------------
// useRipple — self-cleaning tap ripple (Material-style, CSS keyframe)
// ---------------------------------------------------------------------------

interface Ripple {
  id: number;
  x: number;
  y: number;
  size: number;
}

const MAX_RIPPLES = 4;

export function useRipple(light = false) {
  const [ripples, setRipples] = useState<Ripple[]>([]);
  const idRef = useRef(0);
  const reduce = useReducedMotion();

  const onPointerDown = useCallback(
    (e: ReactPointerEvent<HTMLElement>) => {
      if (reduce) return;
      const rect = e.currentTarget.getBoundingClientRect();
      if (rect.width === 0 && rect.height === 0) return;
      const size = Math.max(rect.width, rect.height) * 1.5;
      const id = ++idRef.current;
      const r: Ripple = {
        id,
        x: e.clientX - rect.left - size / 2,
        y: e.clientY - rect.top - size / 2,
        size,
      };
      // keep at most MAX_RIPPLES live; animationend removes each one.
      setRipples((list) => [...list.slice(-(MAX_RIPPLES - 1)), r]);
    },
    [reduce],
  );

  const remove = useCallback((id: number) => {
    setRipples((list) => list.filter((r) => r.id !== id));
  }, []);

  const elements =
    ripples.length > 0 ? (
      <>
        {ripples.map((r) => (
          <span
            key={r.id}
            className={`fx-ripple ${light ? "fx-ripple--light" : ""}`}
            style={{ left: r.x, top: r.y, width: r.size, height: r.size }}
            onAnimationEnd={() => remove(r.id)}
          />
        ))}
      </>
    ) : null;

  return { onPointerDown, elements };
}

// ---------------------------------------------------------------------------
// useMagnetic — very restrained pointer pull for primary actions
// ---------------------------------------------------------------------------

export function useMagnetic(strengthPx = 2) {
  const reduce = useReducedMotion();
  const [pos, setPos] = useState({ x: 0, y: 0 });

  const onPointerMove = useCallback(
    (e: ReactPointerEvent<HTMLElement>) => {
      if (reduce || e.pointerType === "touch") return;
      const rect = e.currentTarget.getBoundingClientRect();
      const dx = ((e.clientX - rect.left) / rect.width - 0.5) * 2;
      const dy = ((e.clientY - rect.top) / rect.height - 0.5) * 2;
      setPos({ x: dx * strengthPx, y: dy * strengthPx });
    },
    [reduce, strengthPx],
  );

  const onPointerLeave = useCallback(() => setPos({ x: 0, y: 0 }), []);

  return { pos, onPointerMove, onPointerLeave };
}

// ---------------------------------------------------------------------------
// FxButton — motion-wrapped open-glass-ui Button
// ---------------------------------------------------------------------------

type FxButtonProps = Omit<ButtonHTMLAttributes<HTMLButtonElement>, "children"> & {
  variant?: "primary" | "secondary" | "quiet" | "danger";
  size?: "small" | "medium" | "large";
  children?: ReactNode;
  /** Result of useAction(): drives busy spinner / ok pop / error shake. */
  action?: { run: (...args: unknown[]) => void; busy: boolean; phase: ActionPhase };
  /** Text shown while busy (default keeps the original label). */
  busyLabel?: ReactNode;
  /** Text flashed on success (defaults to the label). */
  okText?: ReactNode;
  /** Tap ripple + hover light sweep. Default: only on primary. */
  ripple?: boolean;
  /** Subtle magnetic pull toward the cursor (primary actions only). */
  magnetic?: boolean;
};

export function FxButton({
  variant = "secondary",
  size,
  children,
  onClick,
  disabled,
  action,
  busyLabel,
  okText,
  ripple,
  magnetic = false,
  className,
  ...rest
}: FxButtonProps) {
  const showRipple = ripple ?? variant === "primary";
  const { onPointerDown, elements } = useRipple(variant === "primary" || variant === "danger");
  const mag = useMagnetic(2);
  const reduce = useReducedMotion();

  const gestures = disabled
    ? {}
    : magnetic
      ? { whileHover: { scale: 1.02 }, whileTap: { scale: 0.968 } }
      : variant === "quiet"
        ? quietButtonGestures
        : buttonGestures;

  const busy = action?.busy ?? false;
  const phase = action?.phase ?? "idle";

  const handleClick = (e: React.MouseEvent<HTMLButtonElement>) => {
    if (action) {
      action.run();
      onClick?.(e);
    } else {
      onClick?.(e);
    }
  };

  return (
    <motion.div
      className={`fx-press ${phase === "error" ? "fx-shake" : ""} ${magnetic && !reduce ? "fx-magnet" : ""}`}
      style={{ display: "inline-flex", borderRadius: "var(--ogui-radius-control, 0.8rem)" }}
      {...gestures}
      animate={magnetic && !reduce ? { x: mag.pos.x, y: mag.pos.y } : undefined}
      onPointerMove={magnetic ? mag.onPointerMove : undefined}
      onPointerLeave={magnetic ? mag.onPointerLeave : undefined}
      onPointerDown={showRipple ? onPointerDown : undefined}
      transition={pressSpring}
    >
      <Button
        variant={variant}
        size={size}
        className={`fx-btn ${className ?? ""}`}
        disabled={disabled || busy}
        onClick={handleClick}
        {...rest}
      >
        {busy ? (
          <>
            <span className="fx-spinner" aria-hidden />
            {busyLabel ?? children}
          </>
        ) : phase === "ok" ? (
          <motion.span
            className="fx-ok"
            initial={reduce ? false : { scale: 0.5, opacity: 0 }}
            animate={{ scale: 1, opacity: 1 }}
            transition={popSpring}
          >
            ✓ {okText ?? children}
          </motion.span>
        ) : (
          children
        )}
      </Button>
      {showRipple && (
        <span className={`fx-overlay ${variant === "primary" ? "fx-sweep" : ""}`} aria-hidden>
          {elements}
        </span>
      )}
    </motion.div>
  );
}

// ---------------------------------------------------------------------------
// FxTitleButton — titlebar control (fast, precise; no magnet, per spec)
// ---------------------------------------------------------------------------

export function FxTitleButton({
  onClick,
  title,
  children,
}: {
  onClick?: () => void;
  title: string;
  children: ReactNode;
}) {
  return (
    <motion.button
      type="button"
      className="zup-nav-item titlebar-btn"
      title={title}
      aria-label={title}
      onClick={onClick}
      whileHover={{ scale: 1.07 }}
      whileTap={{ scale: 0.86, y: 0.5 }}
      transition={titleSpring}
      style={{ padding: "4px 10px" }}
    >
      {children}
    </motion.button>
  );
}

/**
 * Refresh affordance: replays one 360° sweep of its icon per click
 * (semantic spin, not an infinite loop).
 */
export function FxSpinOnClick({
  onClick,
  title,
  className,
  disabled,
  stopPropagation = false,
  children,
}: {
  onClick?: () => void;
  title?: string;
  className?: string;
  disabled?: boolean;
  /** Stop the click from bubbling (e.g. nested inside a clickable card). */
  stopPropagation?: boolean;
  children: ReactNode;
}) {
  const [key, setKey] = useState(0);
  const reduce = useReducedMotion();
  return (
    <motion.button
      type="button"
      className={className}
      title={title}
      aria-label={title}
      disabled={disabled}
      onClick={(e) => {
        if (stopPropagation) e.stopPropagation();
        setKey((k) => k + 1);
        onClick?.();
      }}
      whileHover={{ scale: 1.08 }}
      whileTap={{ scale: 0.9 }}
      transition={titleSpring}
    >
      <motion.span
        key={key}
        style={{ display: "inline-flex" }}
        initial={false}
        animate={key > 0 && !reduce ? { rotate: 360 } : undefined}
        transition={{ duration: 0.45, ease: [0.35, 0.6, 0.35, 1] }}
      >
        {children}
      </motion.span>
    </motion.button>
  );
}

// ---------------------------------------------------------------------------
// FxChip — small inline control (link-btn style) with pop feedback
// ---------------------------------------------------------------------------

export function FxChip({
  className = "",
  onClick,
  title,
  disabled,
  style,
  children,
}: {
  className?: string;
  onClick?: () => void;
  title?: string;
  disabled?: boolean;
  style?: CSSProperties;
  children?: ReactNode;
}) {
  return (
    <motion.button
      type="button"
      className={`link-btn ${className}`}
      title={title}
      aria-label={title}
      disabled={disabled}
      onClick={onClick}
      style={style}
      whileHover={{ scale: 1.06 }}
      whileTap={{ scale: 0.92 }}
      transition={softSpring}
    >
      {children}
    </motion.button>
  );
}

/** Modal close button ("model-chip" pill) with a clear press. */
export function FxCloseChip({ onClick, label = "关闭" }: { onClick: () => void; label?: string }) {
  return (
    <motion.button
      type="button"
      className="model-chip fx-close"
      onClick={onClick}
      whileHover={{ scale: 1.06 }}
      whileTap={{ scale: 0.9 }}
      transition={softSpring}
      title={label}
      aria-label={label}
    >
      ✕
    </motion.button>
  );
}
