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
  initial: { opacity: 0, y: 7, scale: 0.985 },
  enter: { opacity: 1, y: 0, scale: 1, transition: softSpring },
  exit: {
    opacity: 0,
    y: 4,
    scale: 0.99,
    transition: { duration: 0.13, ease: "easeIn" },
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
