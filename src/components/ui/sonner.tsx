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
          "--normal-bg": "var(--card)",
          "--normal-text": "var(--card-foreground)",
          "--normal-border": "var(--border)",
          zIndex: 99999,
        } as React.CSSProperties
      }
      toastOptions={{
        style: {
          zIndex: 99999,
          pointerEvents: "auto",
          backdropFilter: "saturate(1.2)",
        },
      }}
      {...props}
    />
  );
};

export { Toaster };
