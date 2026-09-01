import { useId } from "react";
import { LayoutGroup, motion, useReducedMotion } from "motion/react";
import type { ReactNode } from "react";
import "../styles/liquid-segments.css";

export interface LiquidSegmentItem<T extends string = string> {
  value: T;
  label: ReactNode;
  disabled?: boolean;
}

export interface LiquidSegmentedControlProps<T extends string = string> {
  "aria-label": string;
  items: readonly LiquidSegmentItem<T>[];
  value: T;
  onValueChange: (value: T) => void;
  className?: string;
  id?: string;
}

/**
 * A compact, controlled segmented control with a shared-layout glass lens.
 *
 * The lens lives inside the active button so its bounds follow the label, but
 * `layoutId` lets Motion interpolate that element between buttons when the
 * controlled value changes.
 */
export function LiquidSegmentedControl<T extends string = string>({
  "aria-label": ariaLabel,
  items,
  value,
  onValueChange,
  className,
  id,
}: LiquidSegmentedControlProps<T>) {
  const instanceId = useId();
  const reduceMotion = useReducedMotion();
  const classes = ["liquid-segments", className].filter(Boolean).join(" ");
  const layoutId = `liquid-segments-lens-${instanceId}`;

  return (
    <LayoutGroup id={`liquid-segments-group-${instanceId}`}>
      <fieldset id={id} className={classes} aria-label={ariaLabel}>
        <legend className="liquid-segments__sr-only">{ariaLabel}</legend>
        {items.map((item) => {
          const selected = item.value === value;

          return (
            <button
              key={item.value}
              type="button"
              className="liquid-segments__item"
              aria-pressed={selected}
              disabled={item.disabled}
              onClick={() => onValueChange(item.value)}
            >
              {selected ? (
                <motion.span
                  aria-hidden="true"
                  className="liquid-segments__lens"
                  layoutId={layoutId}
                  initial={reduceMotion ? false : { scaleX: 0.93, scaleY: 0.96 }}
                  animate={
                    reduceMotion
                      ? { scaleX: 1, scaleY: 1 }
                      : { scaleX: [0.93, 1.045, 1], scaleY: [0.96, 1.015, 1] }
                  }
                  transition={
                    reduceMotion
                      ? { duration: 0 }
                      : {
                          layout: { type: "spring", stiffness: 500, damping: 34, mass: 0.65 },
                          default: { duration: 0.28, ease: [0.22, 1, 0.36, 1] },
                        }
                  }
                />
              ) : null}
              <span className="liquid-segments__content">{item.label}</span>
            </button>
          );
        })}
      </fieldset>
    </LayoutGroup>
  );
}
