"use client";

import confetti from "canvas-confetti";
import { motion, useReducedMotion } from "motion/react";
import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { LuChevronRight } from "react-icons/lu";
import { Logo } from "@/components/icons/logo";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogTitle } from "@/components/ui/dialog";
import { useInputModality } from "@/hooks/use-input-modality";
import { MOTION_EASE_OUT } from "@/lib/motion";
import { isMacOS } from "@/lib/platform";
import { type TipDefinition, type TipId, tipTextKeys } from "@/lib/tips";

const spring = { type: "spring", stiffness: 240, damping: 22 } as const;

/** "pro" reads as "Pro" in the title; an unknown plan keeps its own spelling. */
function displayPlan(plan: string): string {
  return plan ? plan.charAt(0).toLocaleUpperCase() + plan.slice(1) : plan;
}

/**
 * The first thing a freshly paid account sees: what the plan unlocked, each
 * item opening the tip that shows it working. Shown once per account.
 */
export function PaidWelcomeDialog({
  open,
  plan,
  tips,
  onOpenChange,
  onOpenTip,
}: {
  open: boolean;
  plan: string;
  /** The plan tips this account is entitled to, in catalog order. */
  tips: TipDefinition[];
  onOpenChange: (open: boolean) => void;
  onOpenTip: (id: TipId) => void;
}) {
  const { t } = useTranslation();
  const reduceMotion = useReducedMotion();
  const modality = useInputModality();
  const animate = !reduceMotion && modality === "pointer";
  const mod = isMacOS() ? "⌘" : "Ctrl";

  useEffect(() => {
    if (!open || reduceMotion || document.hidden) return;
    const fire = (options: confetti.Options) => {
      if (document.hidden) return;
      void confetti({
        origin: { y: 0.65 },
        disableForReducedMotion: true,
        ...options,
      });
    };
    fire({ particleCount: 80, spread: 66, startVelocity: 42 });
    const second = window.setTimeout(
      () => fire({ particleCount: 40, spread: 100, decay: 0.92 }),
      220,
    );
    return () => window.clearTimeout(second);
  }, [open, reduceMotion]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        data-slot="paid-welcome"
        className="p-5 sm:max-w-md sm:p-6"
      >
        <div className="flex min-w-0 flex-col gap-5">
          <div className="flex flex-col items-center gap-3 text-center">
            <motion.div
              initial={animate ? { scale: 0.92, rotate: -6 } : false}
              animate={{ scale: 1, rotate: 0 }}
              transition={animate ? spring : { duration: 0 }}
              className="text-foreground"
            >
              <Logo className="size-12" />
            </motion.div>
            <DialogTitle className="text-2xl font-semibold tracking-tight text-balance">
              {t("paidWelcome.title", { plan: displayPlan(plan) })}
            </DialogTitle>
            <p className="max-w-[40ch] text-sm/6 text-pretty text-muted-foreground">
              {t("paidWelcome.body")}
            </p>
          </div>

          {tips.length > 0 && (
            <ul
              data-slot="paid-welcome-list"
              className="flex min-w-0 flex-col gap-0.5"
            >
              {tips.map((tip, index) => {
                const keys = tipTextKeys(tip.id);
                return (
                  <li key={tip.id} className="min-w-0">
                    <motion.button
                      type="button"
                      data-slot="paid-welcome-item"
                      data-tip-id={tip.id}
                      onClick={() => onOpenTip(tip.id)}
                      initial={animate ? { y: 8 } : false}
                      animate={{ y: 0 }}
                      transition={{
                        delay: animate ? 0.05 * index : 0,
                        duration: animate ? 0.3 : 0,
                        ease: MOTION_EASE_OUT,
                      }}
                      className="flex w-full min-w-0 cursor-pointer items-center justify-between gap-3 rounded-md px-3 py-2 text-left transition-colors duration-100 hover:bg-accent hover:text-accent-foreground focus-visible:outline-2 focus-visible:outline-ring"
                    >
                      <span className="flex min-w-0 flex-1 flex-col">
                        <span className="truncate text-sm font-medium">
                          {t(keys.title, { mod })}
                        </span>
                        <span className="truncate text-xs text-muted-foreground">
                          {t(keys.body, { mod })}
                        </span>
                      </span>
                      <LuChevronRight
                        aria-hidden="true"
                        className="size-4 shrink-0 text-muted-foreground"
                      />
                    </motion.button>
                  </li>
                );
              })}
            </ul>
          )}

          <div className="flex flex-wrap items-center justify-between gap-2">
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="text-muted-foreground hover:text-foreground"
              onClick={() => onOpenChange(false)}
            >
              {t("paidWelcome.later")}
            </Button>
            <Button
              type="button"
              size="sm"
              data-slot="paid-welcome-cta"
              disabled={tips.length === 0}
              onClick={() => {
                if (tips[0]) onOpenTip(tips[0].id);
              }}
            >
              {t("paidWelcome.cta")}
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
