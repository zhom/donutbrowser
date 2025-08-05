"use client";
import { Geist, Geist_Mono } from "next/font/google";
import "@/styles/globals.css";
import { CustomThemeProvider } from "@/components/theme-provider";
import { Toaster } from "@/components/ui/sonner";
import { TooltipProvider } from "@/components/ui/tooltip";
import { WindowDragArea } from "@/components/window-drag-area";

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
  return (
    <html lang="en" suppressHydrationWarning>
      <body
        className={`${geistSans.variable} ${geistMono.variable} antialiased overflow-hidden`}
      >
        <CustomThemeProvider>
          <TooltipProvider>{children}</TooltipProvider>
          <Toaster className="pointer-events-none" />
          <WindowDragArea />
        </CustomThemeProvider>
      </body>
    </html>
  );
}
