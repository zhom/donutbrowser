"use client";

import { useCallback, useState } from "react";
import { useTranslation } from "react-i18next";
import { LuCheck, LuCopy } from "react-icons/lu";
import { Button } from "@/components/ui/button";
import { showSuccessToast } from "@/lib/toast-utils";

interface CopyToClipboardProps {
  text: string;
  variant?:
    | "default"
    | "destructive"
    | "outline"
    | "secondary"
    | "ghost"
    | "link";
  size?: "default" | "sm" | "lg" | "icon";
  className?: string;
  successMessage?: string;
}

export function CopyToClipboard({
  text,
  variant = "outline",
  size = "icon",
  className,
  successMessage = "Copied to clipboard",
}: CopyToClipboardProps) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);

  const copyToClipboard = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      showSuccessToast(successMessage);
      setTimeout(() => {
        setCopied(false);
      }, 2000);
    } catch (error) {
      console.error("Failed to copy to clipboard:", error);
    }
  }, [text, successMessage]);

  return (
    <Button
      variant={variant}
      size={size}
      className={`relative ${className ?? ""}`}
      onClick={copyToClipboard}
      aria-label={copied ? t("common.aria.copied") : t("common.aria.copy")}
    >
      <span className="sr-only">
        {copied ? t("common.srOnly.copied") : t("common.srOnly.copy")}
      </span>
      <LuCopy
        className={`h-4 w-4 transition-all duration-300 ${
          copied ? "scale-0" : "scale-100"
        }`}
      />
      <LuCheck
        className={`absolute inset-0 m-auto h-4 w-4 text-foreground transition-all duration-300 ${
          copied ? "scale-100" : "scale-0"
        }`}
      />
    </Button>
  );
}
