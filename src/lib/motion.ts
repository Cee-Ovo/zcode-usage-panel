import type { Transition, Variants } from "motion/react";

/**
 * Shared motion language for the desktop shell.
 *
 * The values intentionally stay close to the surface: small translations,
 * quick exits and restrained springs keep the dashboard feeling native rather
 * than like a marketing page.
 */
export const softSpring: Transition = {
  type: "spring",
  stiffness: 360,
  damping: 32,
  mass: 0.72,
};

export const progressSpring: Transition = {
  type: "spring",
  stiffness: 230,
  damping: 28,
  mass: 0.85,
};

/**
 * Interaction springs (see components/fx.tsx).
 * - pressSpring: snappy down-press with a hint of bounce on release.
 * - titleSpring: faster/flatter for 1-click titlebar controls.
 * - popSpring: small overshoot for success/checked icons.
 */
export const pressSpring: Transition = {
  type: "spring",
  stiffness: 480,
  damping: 26,
  mass: 0.66,
};

export const titleSpring: Transition = {
  type: "spring",
  stiffness: 700,
  damping: 34,
  mass: 0.5,
};

export const popSpring: Transition = {
  type: "spring",
  stiffness: 520,
  damping: 21,
  mass: 0.7,
};

/** Hover/tap gestures for the standard button wrapper (FxButton). */
export const buttonGestures = {
  whileHover: { scale: 1.02, y: -1 },
  whileTap: { scale: 0.968, y: 0.5 },
} as const;

/** Quieter gestures for secondary/quiet buttons. */
export const quietButtonGestures = {
  whileHover: { scale: 1.012, y: -0.5 },
  whileTap: { scale: 0.975 },
} as const;

/** Small controls (link buttons, chips): minimal, fast feedback. */
export const chipGestures = {
  whileHover: { scale: 1.05 },
  whileTap: { scale: 0.92 },
} as const;

export const pageVariants: Variants = {
  initial: { opacity: 0, y: 7 },
  enter: {
    opacity: 1,
    y: 0,
    transition: { duration: 0.22, ease: [0.22, 1, 0.36, 1] },
  },
  exit: {
    opacity: 0,
    y: -4,
    transition: { duration: 0.13, ease: [0.4, 0, 1, 1] },
  },
};

export const backdropVariants: Variants = {
  initial: { opacity: 0 },
  enter: { opacity: 1, transition: { duration: 0.18, ease: "easeOut" } },
  exit: { opacity: 0, transition: { duration: 0.13, ease: "easeIn" } },
};

export const dialogVariants: Variants = {
  initial: { opacity: 0, y: 10, scale: 0.96 },
  enter: { opacity: 1, y: 0, scale: 1, transition: pressSpring },
  exit: {
    opacity: 0,
    y: 5,
    scale: 0.97,
    transition: { duration: 0.12, ease: "easeIn" },
  },
};

export const staggerContainer: Variants = {
  initial: {},
  enter: {
    transition: { staggerChildren: 0.035, delayChildren: 0.035 },
  },
};

export const cardVariants: Variants = {
  initial: { opacity: 0, y: 8 },
  enter: {
    opacity: 1,
    y: 0,
    transition: { duration: 0.24, ease: [0.22, 1, 0.36, 1] },
  },
};

export const listItemVariants: Variants = {
  initial: { opacity: 0, y: 6, scale: 0.992 },
  enter: { opacity: 1, y: 0, scale: 1, transition: softSpring },
  exit: {
    opacity: 0,
    y: -3,
    scale: 0.995,
    transition: { duration: 0.12, ease: "easeIn" },
  },
};

/** Row gesture for clickable table rows: nudge right on hover, sink on press. */
export const rowGestures = {
  whileHover: { x: 2 },
  whileTap: { scale: 0.994, x: 2 },
} as const;
