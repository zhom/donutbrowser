"use client";

import { motion, useReducedMotion } from "motion/react";
import { useCallback, useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { Logo } from "@/components/icons/logo";
import { useInputModality } from "@/hooks/use-input-modality";
import { MOTION_EASE_OUT } from "@/lib/motion";

/** A tiny snack drawer, unlocked with Shift + the About logo. */
export function DonutSnack() {
  const { t } = useTranslation();
  const reduced = useReducedMotion();
  const modality = useInputModality();
  const maskId = useId();
  const [bites, setBites] = useState(0);
  const still = reduced || modality === "keyboard";
  const focusBite = useCallback((button: HTMLButtonElement | null) => {
    button?.focus({ preventScroll: true });
  }, []);

  return (
    <div
      data-slot="donut-snack"
      data-bites={bites}
      className="flex flex-col items-center gap-2"
    >
      <button
        type="button"
        ref={focusBite}
        data-slot="donut-snack-bite"
        onClick={() => setBites((count) => (count + 1) % 4)}
        className="group grid justify-items-center gap-1 rounded-md px-3 py-1 text-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        <motion.svg
          aria-hidden="true"
          viewBox="0 0 100 100"
          className="size-24"
          initial={false}
          animate={{
            rotate: still ? 0 : bites === 1 ? -4 : bites === 2 ? 3 : 0,
          }}
          transition={{ duration: still ? 0 : 0.18, ease: MOTION_EASE_OUT }}
        >
          <defs>
            <mask
              id={maskId}
              maskUnits="userSpaceOnUse"
              x="0"
              y="0"
              width="100"
              height="100"
            >
              <rect width="100" height="100" fill="white" />
              {bites >= 1 && (
                <g fill="black">
                  <circle cx="77" cy="18" r="17" />
                  <circle cx="88" cy="31" r="14" />
                  <circle cx="67" cy="24" r="12" />
                </g>
              )}
              {bites >= 2 && (
                <g fill="black">
                  <circle cx="16" cy="36" r="20" />
                  <circle cx="28" cy="25" r="16" />
                  <circle cx="27" cy="44" r="12" />
                </g>
              )}
            </mask>
          </defs>
          {bites < 3 ? (
            <g mask={`url(#${maskId})`}>
              <Logo width="100" height="100" />
            </g>
          ) : (
            <g fill="currentColor">
              <path d="m38 54 6-3 4 5-3 4-7-2Z" />
              <path d="m57 60 5-2 3 4-5 3-4-1Z" />
              <rect
                x="51"
                y="45"
                width="3"
                height="6"
                rx="1.5"
                transform="rotate(28 52.5 48)"
              />
              <circle cx="34" cy="65" r="1.5" />
              <circle cx="65" cy="48" r="1.5" />
            </g>
          )}
        </motion.svg>
        <span className="text-xs font-medium group-hover:text-muted-foreground">
          {t(bites === 3 ? "about.snack.again" : "about.snack.takeBite")}
        </span>
      </button>
      <p
        role="status"
        className="max-w-64 text-center text-xs text-muted-foreground"
      >
        {t(bites === 3 ? "about.snack.finished" : "about.snack.found")}
      </p>
    </div>
  );
}
