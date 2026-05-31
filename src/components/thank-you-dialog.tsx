"use client";

import confetti from "canvas-confetti";
import { motion } from "motion/react";
import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { Logo } from "@/components/icons/logo";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogTitle } from "@/components/ui/dialog";

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

  useEffect(() => {
    if (!isOpen) return;
    const fire = (options: confetti.Options) => {
      void confetti({ origin: { y: 0.7 }, ...options });
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
  }, [isOpen]);

  return (
    <Dialog
      open={isOpen}
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <DialogContent className="sm:max-w-md">
        <div className="flex flex-col items-center gap-6 text-center">
          <motion.div
            initial={{ opacity: 0, scale: 0.6, rotate: -12 }}
            animate={{ opacity: 1, scale: 1, rotate: 0 }}
            transition={{ ...spring, delay: 0.05 }}
            className="text-foreground"
          >
            <Logo className="size-14" />
          </motion.div>

          <div className="flex flex-col gap-2">
            <DialogTitle className="text-2xl font-semibold tracking-tight text-balance">
              {t("onboarding.thankYou.title")}
            </DialogTitle>
            <motion.p
              initial={{ opacity: 0, y: 8 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ ...spring, delay: 0.15 }}
              className="mx-auto max-w-[46ch] text-sm leading-6 text-pretty text-muted-foreground"
            >
              {t("onboarding.thankYou.body")}
            </motion.p>
          </div>

          <Button size="sm" onClick={onClose}>
            {t("onboarding.thankYou.cta")}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
