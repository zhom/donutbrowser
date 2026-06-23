"use client";

import type { CardComponentProps } from "onborda";
import { useOnborda } from "onborda";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { ONBOARDING_TOUR_FINISHED_EVENT } from "@/lib/onboarding-signal";

// Custom Onborda card, themed with the app's CSS variables. Finishing the last
// step emits ONBOARDING_TOUR_FINISHED_EVENT so the page can show the celebratory
// thank-you dialog (skipping early does not emit it).
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

  const isFirst = currentStep === 0;
  const isLast = currentStep === totalSteps - 1;
  // This step is completed by clicking the highlighted element (the "New"
  // button), not by a "Next" button — advancing manually would jump to a step
  // whose target doesn't exist yet and block the button. So hide "Next" here.
  const requiresAction = step.selector === '[data-onborda="create-profile"]';

  return (
    <div className="relative w-80 max-w-[90vw] rounded-lg border bg-popover p-4 text-popover-foreground shadow-lg">
      <div className="flex items-start justify-between gap-2">
        <h3 className="text-sm/tight font-semibold">{step.title}</h3>
        <span className="shrink-0 text-[11px] text-muted-foreground tabular-nums">
          {currentStep + 1}/{totalSteps}
        </span>
      </div>

      <div className="mt-2 text-xs/relaxed text-muted-foreground">
        {step.content}
      </div>

      <div className="mt-4 flex items-center justify-between gap-2">
        {isLast ? (
          <span />
        ) : (
          <Button
            variant="ghost"
            size="sm"
            className="h-7 px-2 text-xs text-muted-foreground hover:text-foreground"
            onClick={() => {
              closeOnborda();
            }}
          >
            {t("onboarding.buttons.skip")}
          </Button>
        )}

        <div className="flex items-center gap-2">
          {!isFirst && !isLast && (
            <Button
              variant="outline"
              size="sm"
              className="h-7 px-2.5 text-xs"
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
              className="h-7 px-3 text-xs"
              onClick={() => {
                closeOnborda();
                window.dispatchEvent(new Event(ONBOARDING_TOUR_FINISHED_EVENT));
              }}
            >
              {t("onboarding.buttons.finish")}
            </Button>
          ) : requiresAction ? null : (
            <Button
              size="sm"
              className="h-7 px-3 text-xs"
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
    </div>
  );
}
