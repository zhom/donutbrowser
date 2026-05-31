"use client";

import { Onborda, type OnbordaProps, OnbordaProvider } from "onborda";
import { useTranslation } from "react-i18next";
import { OnboardingCard } from "@/components/onboarding-card";

// Name of the first-run product tour. Referenced by the trigger logic in
// `src/app/page.tsx` via `startOnborda(ONBOARDING_TOUR)`.
export const ONBOARDING_TOUR = "donut-onboarding";

export function OnboardingProvider({
  children,
}: {
  children: React.ReactNode;
}) {
  const { t } = useTranslation();

  const tours: OnbordaProps["steps"] = [
    {
      tour: ONBOARDING_TOUR,
      steps: [
        {
          icon: null,
          title: t("onboarding.steps.createProfile.title"),
          content: t("onboarding.steps.createProfile.content"),
          selector: '[data-onborda="create-profile"]',
          // The "New" button sits in the top-right corner; "bottom-right"
          // anchors the card's right edge to it so the card extends left/down
          // and stays on-screen instead of overflowing the right viewport edge.
          side: "bottom-right",
          showControls: true,
          pointerPadding: 8,
          pointerRadius: 10,
        },
        {
          icon: null,
          title: t("onboarding.steps.dnsBlocking.title"),
          content: t("onboarding.steps.dnsBlocking.content"),
          selector: '[data-onborda="dns-blocklist"]',
          // The DNS dropdown sits in the right-hand columns. A centered "bottom"
          // card runs off the right edge; "bottom-right" anchors the card's right
          // edge to the dropdown and extends it left/down, keeping it fully
          // on-screen with its arrow pointing up at the option.
          side: "bottom-right",
          showControls: true,
          pointerPadding: 6,
          pointerRadius: 8,
        },
      ],
    },
  ];

  return (
    <OnbordaProvider>
      <Onborda
        steps={tours}
        cardComponent={OnboardingCard}
        interact
        shadowRgb="0,0,0"
        shadowOpacity="0.6"
      >
        {children}
      </Onborda>
    </OnbordaProvider>
  );
}
