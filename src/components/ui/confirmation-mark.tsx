"use client";

import { motion, useReducedMotion } from "motion/react";
import { LuCheck } from "react-icons/lu";
import { useInputModality } from "@/hooks/use-input-modality";
import { MOTION_EASE_OUT } from "@/lib/motion";

export function ConfirmationMark() {
  const reduced = useReducedMotion();
  const modality = useInputModality();
  return (
    <motion.span
      aria-hidden="true"
      initial={reduced || modality === "keyboard" ? false : { scale: 0.9 }}
      animate={{ scale: 1 }}
      transition={{ duration: 0.14, ease: MOTION_EASE_OUT }}
      className="inline-flex shrink-0 text-success-text"
    >
      <LuCheck className="size-3.5" />
    </motion.span>
  );
}
