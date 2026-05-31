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
    <div className="relative p-4 w-80 max-w-[90vw] rounded-lg border shadow-lg bg-popover text-popover-foreground">
      <div className="flex gap-2 items-start justify-between">
        <h3 className="text-sm font-semibold leading-tight">{step.title}</h3>
        <span className="shrink-0 text-[11px] tabular-nums text-muted-foreground">
          {currentStep + 1}/{totalSteps}
        </span>
      </div>

      <div className="mt-2 text-xs leading-relaxed text-muted-foreground">
        {step.content}
      </div>

      <div className="flex gap-2 items-center justify-between mt-4">
        {isLast ? (
          <span />
        ) : (
          <Button
            variant="ghost"
            size="sm"
            className="text-xs h-7 px-2 text-muted-foreground hover:text-foreground"
            onClick={() => {
              closeOnborda();
            }}
          >
            {t("onboarding.buttons.skip")}
          </Button>
        )}

        <div className="flex gap-2 items-center">
          {!isFirst && !isLast && (
            <Button
              variant="outline"
              size="sm"
              className="text-xs h-7 px-2.5"
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
              className="text-xs h-7 px-3"
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
              className="text-xs h-7 px-3"
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
