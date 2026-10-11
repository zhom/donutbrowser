"use client";

import { invoke } from "@tauri-apps/api/core";
import { MotionConfig } from "motion/react";
import { useEffect } from "react";
import { I18nProvider } from "@/components/i18n-provider";
import { OnboardingProvider } from "@/components/onboarding-provider";
import { CustomThemeProvider } from "@/components/theme-provider";
import { Toaster } from "@/components/ui/sonner";
import { TooltipProvider } from "@/components/ui/tooltip";
import { WindowDragArea } from "@/components/window-drag-area";
import { InputModalityProvider } from "@/hooks/use-input-modality";
import { LaunchActivityProvider } from "@/hooks/use-launch-activity";
import { setupLogging } from "@/lib/logger";
import { installUiSounds, setUiSoundsEnabled } from "@/lib/ui-sounds";

export function ClientProviders({ children }: { children: React.ReactNode }) {
  useEffect(() => {
    void setupLogging();
  }, []);

  useEffect(() => {
    void invoke<{ ui_sounds?: boolean }>("get_app_settings")
      .then((settings) => setUiSoundsEnabled(settings.ui_sounds ?? true))
      .catch(() => {});
    return installUiSounds();
  }, []);

  return (
    <I18nProvider>
      <CustomThemeProvider>
        {/* reducedMotion="user" makes every motion/react animation honor the
            OS prefers-reduced-motion setting: transforms are skipped, opacity
            cross-fades are kept. The CSS-side media query in globals.css only
            covers CSS transitions — this covers the JS-driven ones. */}
        <MotionConfig reducedMotion="user">
          <InputModalityProvider>
            <WindowDragArea />
            <TooltipProvider>
              <LaunchActivityProvider>
                <OnboardingProvider>{children}</OnboardingProvider>
              </LaunchActivityProvider>
            </TooltipProvider>
            <Toaster />
          </InputModalityProvider>
        </MotionConfig>
      </CustomThemeProvider>
    </I18nProvider>
  );
}
