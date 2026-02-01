"use client";
import { Geist, Geist_Mono } from "next/font/google";
import "@/styles/globals.css";
import "flag-icons/css/flag-icons.min.css";
import { useEffect } from "react";
import { I18nProvider } from "@/components/i18n-provider";
import { CustomThemeProvider } from "@/components/theme-provider";
import { Toaster } from "@/components/ui/sonner";
import { TooltipProvider } from "@/components/ui/tooltip";
import { WindowDragArea } from "@/components/window-drag-area";
import { setupLogging } from "@/lib/logger";

const geistSans = Geist({
  variable: "--font-geist-sans",
  subsets: ["latin"],
});

const geistMono = Geist_Mono({
  variable: "--font-geist-mono",
  subsets: ["latin"],
});

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  useEffect(() => {
    void setupLogging();
  }, []);

  return (
    <html lang="en" suppressHydrationWarning>
      <body
        className={`${geistSans.variable} ${geistMono.variable} antialiased overflow-hidden bg-background`}
      >
        <I18nProvider>
          <CustomThemeProvider>
            <WindowDragArea />
            <TooltipProvider>{children}</TooltipProvider>
            <Toaster />
          </CustomThemeProvider>
        </I18nProvider>
      </body>
    </html>
  );
}
