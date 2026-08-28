import { useEffect, useRef, useState } from "react";

/**
 * Tweened numeric display: animates value changes over ~220 ms with an
 * ease-out curve. Used for dashboard figures so live updates feel alive
 * without full-page re-renders (the component re-renders itself only).
 */
export function AnimatedNumber({
  value,
  format,
  className,
  duration = 220,
}: {
  value: number;
  format: (n: number) => string;
  className?: string;
  duration?: number;
}) {
  const [display, setDisplay] = useState(value);
  const fromRef = useRef(value);
  const startRef = useRef(0);
  const rafRef = useRef<number>(0);

  useEffect(() => {
    const reduce =
      typeof matchMedia !== "undefined" &&
      matchMedia("(prefers-reduced-motion: reduce)").matches;
    if (reduce || !isFinite(value)) {
      setDisplay(value);
      fromRef.current = value;
      return;
    }
    fromRef.current = display;
    startRef.current = performance.now();
    cancelAnimationFrame(rafRef.current);

    const step = (now: number) => {
      const t = Math.min(1, (now - startRef.current) / duration);
      const eased = 1 - Math.pow(1 - t, 3);
      const current = fromRef.current + (value - fromRef.current) * eased;
      setDisplay(current);
      if (t < 1) {
        rafRef.current = requestAnimationFrame(step);
      } else {
        fromRef.current = value;
      }
    };
    rafRef.current = requestAnimationFrame(step);
    return () => cancelAnimationFrame(rafRef.current);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [value, duration]);

  return <span className={className}>{format(display)}</span>;
}
