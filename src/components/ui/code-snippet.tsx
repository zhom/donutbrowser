"use client";

import type { ReactNode } from "react";
import { CopyToClipboard } from "@/components/ui/copy-to-clipboard";
import { cn } from "@/lib/utils";

function escapeRegExp(text: string) {
  return text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/** Marks the parts of a snippet the reader has to swap for their own value. */
function withHighlights(code: string, highlights: readonly string[]) {
  const tokens = [...new Set(highlights.filter(Boolean))];
  if (tokens.length === 0) return code;
  const parts = code.split(
    new RegExp(`(${tokens.map(escapeRegExp).join("|")})`, "g"),
  );
  const nodes: ReactNode[] = [];
  let offset = 0;
  parts.forEach((part, index) => {
    if (part) {
      nodes.push(
        index % 2 === 1 ? (
          <span
            key={offset}
            data-slot="code-placeholder"
            className="rounded-sm bg-primary/10 px-0.5 font-medium text-foreground"
          >
            {part}
          </span>
        ) : (
          part
        ),
      );
    }
    offset += part.length;
  });
  return nodes;
}

interface CodeSnippetProps {
  code: string;
  /** What the copy button writes, when it differs from what is shown. */
  copyText?: string;
  highlights?: readonly string[];
  /** Draws a shell prompt that is never copied. */
  prompt?: boolean;
  multiline?: boolean;
  /** Wraps long lines, for prose rather than code. */
  wrap?: boolean;
  copyable?: boolean;
  copyLabel?: string;
  successMessage?: string;
  className?: string;
  "data-testid"?: string;
}

export function CodeSnippet({
  code,
  copyText,
  highlights = [],
  prompt = false,
  multiline = false,
  wrap = false,
  copyable = true,
  copyLabel,
  successMessage,
  className,
  "data-testid": testId,
}: CodeSnippetProps) {
  return (
    <div
      data-slot="code-snippet"
      className={cn(
        "group/snippet relative min-w-0 rounded-lg bg-foreground/4 font-mono text-[11px] leading-5 text-foreground",
        className,
      )}
    >
      <pre
        data-testid={testId}
        className={cn(
          "[font-family:inherit] select-text",
          wrap
            ? "break-words whitespace-pre-wrap"
            : "scrollbar-none overflow-x-auto whitespace-pre",
          multiline ? "max-h-72 overflow-y-auto p-3" : "px-3 py-1.5",
          copyable && "pr-10",
        )}
      >
        {prompt && (
          <span
            aria-hidden="true"
            className="text-muted-foreground select-none"
          >
            ${" "}
          </span>
        )}
        <code className="[font-family:inherit]">
          {withHighlights(code, highlights)}
        </code>
      </pre>
      {copyable && (
        <CopyToClipboard
          text={copyText ?? code}
          variant="subtle"
          aria-label={copyLabel}
          successMessage={successMessage}
          className={cn(
            "absolute right-1 size-7 rounded-md bg-background/60 opacity-0 backdrop-blur-sm transition-opacity duration-150 group-focus-within/snippet:opacity-100 group-hover/snippet:opacity-100 focus-visible:opacity-100 data-[copied=true]:opacity-100 [@media(hover:none)]:opacity-100",
            multiline ? "top-1.5" : "top-1/2 -translate-y-1/2",
          )}
        />
      )}
    </div>
  );
}
