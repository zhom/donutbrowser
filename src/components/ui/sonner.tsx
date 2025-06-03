"use client";

import { useTheme } from "next-themes";
import { Toaster as Sonner, type ToasterProps } from "sonner";

const Toaster = ({ ...props }: ToasterProps) => {
  const { theme = "system" } = useTheme();

  return (
    <Sonner
      theme={theme as ToasterProps["theme"]}
      className="toaster group"
      style={
        {
          "--normal-bg": "var(--popover)",
          "--normal-text": "var(--popover-foreground)",
          "--normal-border": "var(--border)",
          zIndex: 99999,
        } as React.CSSProperties
      }
      toastOptions={{
        style: {
          zIndex: 99999,
          pointerEvents: "auto",
        },
      }}
      {...props}
    />
  );
};

export { Toaster };
