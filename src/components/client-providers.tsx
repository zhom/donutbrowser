"use client";

import { useEffect } from "react";
import { I18nProvider } from "@/components/i18n-provider";
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
        <WindowDragArea />
        <TooltipProvider>{children}</TooltipProvider>
        <Toaster />
      </CustomThemeProvider>
    </I18nProvider>
  );
}
