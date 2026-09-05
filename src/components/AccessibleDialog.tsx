import { useEffect, useRef, type ReactNode } from "react";
import { AnimatePresence, motion } from "motion/react";
import { Glass } from "open-glass-ui";
import { createPortal } from "react-dom";
import { backdropVariants, dialogVariants } from "../lib/motion";
const MotionGlass = motion.create(Glass);

interface AccessibleDialogProps {
  label: string;
  onClose: () => void;
  children: ReactNode;
  className?: string;
  glass?: boolean;
}

/** A glass modal with keyboard dismissal, focus trapping, and focus restore. */
export function AccessibleDialog({ label, onClose, children, className = "", glass = false }: AccessibleDialogProps) {
  const panelRef = useRef<HTMLDivElement>(null);
  const restoreRef = useRef<HTMLElement | null>(null);
  const Surface = glass ? MotionGlass : motion.div;

  useEffect(() => {
    restoreRef.current = document.activeElement as HTMLElement | null;
    const frame = window.requestAnimationFrame(() => {
      const first = panelRef.current?.querySelector<HTMLElement>(
        "button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex='-1'])",
      );
      (first ?? panelRef.current)?.focus();
    });
    return () => {
      window.cancelAnimationFrame(frame);
      restoreRef.current?.focus?.();
      restoreRef.current = null;
    };
  }, []);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      }
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  const trapFocus = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "Tab") return;
    const focusable = Array.from(
      event.currentTarget.querySelectorAll<HTMLElement>(
        "button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex='-1'])",
      ),
    );
    if (focusable.length === 0) {
      event.preventDefault();
      event.currentTarget.focus();
      return;
    }
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  };

  const content = (
    <AnimatePresence>
      <motion.div
        className="overlay-backdrop"
        variants={backdropVariants}
        initial="initial"
        animate="enter"
        exit="exit"
        onClick={(event) => {
          if (event.target === event.currentTarget) onClose();
        }}
      >
        <Surface
          {...(glass ? { material: "frosted" as const, renderer: "css" as const, interactive: false } : {})}
          ref={panelRef}
          className={`panel overlay-card ${glass ? "sample-glass" : ""} ${className}`.trim()}
          variants={dialogVariants}
          role="dialog"
          aria-modal="true"
          aria-label={label}
          tabIndex={-1}
          onKeyDown={trapFocus}
        >
          {children}
        </Surface>
      </motion.div>
    </AnimatePresence>
  );
  // Backdrop-filter establishes a containing block for fixed descendants.
  // Keep glass dialogs outside all glass cards so the overlay covers the window.
  const host = glass ? document.querySelector(".zup-shell") : null;
  return host ? createPortal(content, host) : content;
}
