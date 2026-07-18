"use client";

import { MotionConfig } from "motion/react";
import { useEffect } from "react";
import { I18nProvider } from "@/components/i18n-provider";
import { OnboardingProvider } from "@/components/onboarding-provider";
import { CustomThemeProvider } from "@/components/theme-provider";
import { Toaster } from "@/components/ui/sonner";
import { TooltipProvider } from "@/components/ui/tooltip";
import { WindowDragArea } from "@/components/window-drag-area";
import { setupLogging } from "@/lib/logger";

export function ClientProviders({ children }: { children: React.ReactNode }) {
  useEffect(() => {
    void setupLogging();
  }, []);

  return (
    <I18nProvider>
      <CustomThemeProvider>
        {/* reducedMotion="user" makes every motion/react animation honor the
            OS prefers-reduced-motion setting: transforms are skipped, opacity
            cross-fades are kept. The CSS-side media query in globals.css only
            covers CSS transitions — this covers the JS-driven ones. */}
        <MotionConfig reducedMotion="user">
          <WindowDragArea />
          <TooltipProvider>
            <OnboardingProvider>{children}</OnboardingProvider>
          </TooltipProvider>
          <Toaster />
        </MotionConfig>
      </CustomThemeProvider>
    </I18nProvider>
  );
}
