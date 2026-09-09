"use client";

import confetti from "canvas-confetti";
import { motion, useReducedMotion } from "motion/react";
import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { Logo } from "@/components/icons/logo";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogTitle } from "@/components/ui/dialog";
import { useInputModality } from "@/hooks/use-input-modality";

const spring = { type: "spring", stiffness: 240, damping: 22 } as const;

// Celebratory close-out of the first-run onboarding: thanks the user and fires
// confetti. Shown once the product tour is finished.
export function ThankYouDialog({
  isOpen,
  onClose,
}: {
  isOpen: boolean;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const reduceMotion = useReducedMotion();
  const inputModality = useInputModality();

  useEffect(() => {
    if (!isOpen || reduceMotion || document.hidden) return;
    const fire = (options: confetti.Options) => {
      if (document.hidden) return;
      void confetti({
        origin: { y: 0.7 },
        disableForReducedMotion: true,
        ...options,
      });
    };
    fire({ particleCount: 110, spread: 70, startVelocity: 48 });
    const t1 = setTimeout(
      () => fire({ particleCount: 70, spread: 100, decay: 0.92 }),
      200,
    );
    const t2 = setTimeout(
      () => fire({ particleCount: 50, spread: 120, scalar: 0.9 }),
      420,
    );
    return () => {
      clearTimeout(t1);
      clearTimeout(t2);
    };
  }, [isOpen, reduceMotion]);

  return (
    <Dialog
      open={isOpen}
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <DialogContent className="p-4 sm:max-w-md sm:p-6">
        <div className="flex flex-col items-center gap-6 text-center">
          <motion.div
            initial={
              reduceMotion || inputModality === "keyboard"
                ? false
                : { scale: 0.92, rotate: -6 }
            }
            animate={{ scale: 1, rotate: 0 }}
            transition={
              reduceMotion || inputModality === "keyboard"
                ? { duration: 0 }
                : spring
            }
            className="text-foreground"
          >
            <Logo className="size-14" />
          </motion.div>

          <div className="flex flex-col gap-2">
            <DialogTitle className="text-2xl font-semibold tracking-tight text-balance">
              {t("onboarding.thankYou.title")}
            </DialogTitle>
            <p className="mx-auto max-w-[46ch] text-base/7 text-pretty text-muted-foreground sm:text-sm/6">
              {t("onboarding.thankYou.body")}
            </p>
          </div>

          <Button size="sm" onClick={onClose}>
            {t("onboarding.thankYou.cta")}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
