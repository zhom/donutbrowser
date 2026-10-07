"use client";

import { useCallback, useState } from "react";
import { useTranslation } from "react-i18next";
import { LuCheck, LuCopy } from "react-icons/lu";
import { Button, type ButtonProps } from "@/components/ui/button";
import { showSuccessToast } from "@/lib/toast-utils";
import { cn } from "@/lib/utils";

interface CopyToClipboardProps {
  text: string;
  variant?: ButtonProps["variant"];
  size?: "default" | "sm" | "lg" | "icon";
  className?: string;
  successMessage?: string;
  "aria-label"?: string;
}

export function CopyToClipboard({
  text,
  variant = "outline",
  size = "icon",
  className,
  successMessage,
  "aria-label": ariaLabel,
}: CopyToClipboardProps) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);

  const copyToClipboard = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      showSuccessToast(successMessage ?? t("toasts.success.copied"));
      setTimeout(() => {
        setCopied(false);
      }, 2000);
    } catch (error) {
      console.error("Failed to copy to clipboard:", error);
    }
  }, [text, successMessage, t]);

  return (
    <Button
      variant={variant}
      size={size}
      // Lets a control that is only shown on hover stay up while it confirms.
      data-copied={copied}
      className={cn("relative", className)}
      onClick={copyToClipboard}
      aria-label={
        copied ? t("common.aria.copied") : (ariaLabel ?? t("common.aria.copy"))
      }
    >
      <span className="sr-only">
        {copied ? t("common.srOnly.copied") : t("common.srOnly.copy")}
      </span>
      <LuCopy
        className={cn(
          "size-4 transition-all duration-150",
          copied ? "scale-0" : "scale-100",
        )}
      />
      <LuCheck
        className={cn(
          "absolute inset-0 m-auto size-4 text-foreground transition-all duration-150",
          copied ? "scale-100" : "scale-0",
        )}
      />
    </Button>
  );
}
