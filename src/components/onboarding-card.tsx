"use client";

import { motion, useReducedMotion } from "motion/react";
import type { CardComponentProps } from "onborda";
import { useOnborda } from "onborda";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import {
  ONBOARDING_TOUR_CLOSED_EVENT,
  ONBOARDING_TOUR_FINISHED_EVENT,
} from "@/lib/onboarding-signal";

// Custom Onborda card, themed with the app's CSS variables. Closing always
// persists completion; finishing the last step also asks the page to celebrate.
export function OnboardingCard({
  step,
  currentStep,
  totalSteps,
  nextStep,
  prevStep,
  arrow,
}: CardComponentProps) {
  const { t } = useTranslation();
  const { closeOnborda } = useOnborda();
  const reduceMotion = useReducedMotion();

  const isFirst = currentStep === 0;
  const isLast = currentStep === totalSteps - 1;
  // This step is completed by clicking the highlighted element (the "New"
  // button), not by a "Next" button — advancing manually would jump to a step
  // whose target doesn't exist yet and block the button. So hide "Next" here.
  const requiresAction = step.selector === '[data-onborda="create-profile"]';
  const closeTour = () => {
    closeOnborda();
    window.dispatchEvent(new Event(ONBOARDING_TOUR_CLOSED_EVENT));
  };

  return (
    <motion.div
      initial={{ opacity: 0, scale: reduceMotion ? 1 : 0.96 }}
      animate={{ opacity: 1, scale: 1 }}
      transition={
        reduceMotion
          ? { duration: 0.15 }
          : { type: "spring", stiffness: 300, damping: 30 }
      }
      className="relative flex w-80 max-w-[calc(100vw-2rem)] flex-col gap-4 rounded-lg border bg-popover p-4 text-popover-foreground shadow-lg"
    >
      <div className="flex items-start justify-between gap-2">
        <h3 className="text-base font-semibold text-balance sm:text-sm">
          {step.title}
        </h3>
        <span className="shrink-0 text-[11px] text-muted-foreground tabular-nums">
          {currentStep + 1}/{totalSteps}
        </span>
      </div>

      <div className="text-base/7 text-pretty text-muted-foreground sm:text-sm/6">
        {step.content}
      </div>

      <div className="flex items-center justify-between gap-2">
        {isLast ? (
          <span />
        ) : (
          <Button
            variant="ghost"
            size="sm"
            className="text-muted-foreground hover:text-foreground"
            onClick={closeTour}
          >
            {t("onboarding.buttons.skip")}
          </Button>
        )}

        <div className="flex items-center gap-2">
          {!isFirst && !isLast && (
            <Button
              variant="outline"
              size="sm"
              onClick={() => {
                prevStep();
              }}
            >
              {t("onboarding.buttons.back")}
            </Button>
          )}
          {isLast ? (
            <Button
              size="sm"
              onClick={() => {
                closeTour();
                window.dispatchEvent(new Event(ONBOARDING_TOUR_FINISHED_EVENT));
              }}
            >
              {t("onboarding.buttons.finish")}
            </Button>
          ) : requiresAction ? null : (
            <Button
              size="sm"
              onClick={() => {
                nextStep();
              }}
            >
              {t("onboarding.buttons.next")}
            </Button>
          )}
        </div>
      </div>

      <span className="text-popover">{arrow}</span>
    </motion.div>
  );
}
