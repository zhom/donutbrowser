"use client";

import * as TabsPrimitive from "@radix-ui/react-tabs";
import { motion, useReducedMotion } from "motion/react";
import { type ReactNode, useId, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  AnimatedTabs,
  AnimatedTabsContent,
  AnimatedTabsList,
  AnimatedTabsTrigger,
} from "@/components/ui/animated-tabs";
import { CodeSnippet } from "@/components/ui/code-snippet";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { ProBadge } from "@/components/ui/pro-badge";
import { SECTION_LABEL_CLASS } from "@/components/ui/settings-panel";
import { useInputModality } from "@/hooks/use-input-modality";
import type { SnippetLanguage } from "@/lib/api-examples";
import { MOTION_EASE_OUT } from "@/lib/motion";
import { cn } from "@/lib/utils";

export interface CodeExample {
  id: string;
  title: string;
  description: string;
  method?: string;
  paid?: boolean;
  snippets: Partial<Record<SnippetLanguage, { display: string; copy: string }>>;
  response?: string;
}

const LANGUAGE_ORDER: SnippetLanguage[] = [
  "curl",
  "javascript",
  "python",
  "json",
];

function languagesOf(example: CodeExample) {
  return LANGUAGE_ORDER.filter((language) => example.snippets[language]);
}

export function CodeExamplesDialog({
  open,
  onOpenChange,
  title,
  description,
  examples,
  highlights,
  note,
  copyMessage,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  description: string;
  examples: CodeExample[];
  highlights: readonly string[];
  note?: ReactNode;
  copyMessage: string;
}) {
  const { t } = useTranslation();
  const reduceMotion = useReducedMotion();
  const inputModality = useInputModality();
  const animate = !reduceMotion && inputModality === "pointer";
  const markerId = useId();
  const [selectedId, setSelectedId] = useState(examples[0]?.id ?? "");
  // The language is the reader's, not the example's: it stays as they move
  // between examples, and an example without it falls back to its own first.
  const [language, setLanguage] = useState<SnippetLanguage>("curl");

  const selected =
    examples.find((example) => example.id === selectedId) ?? examples[0];
  if (!selected) return null;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        data-testid="code-examples-dialog"
        className="flex max-h-[85vh] max-w-4xl flex-col gap-4"
      >
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>{description}</DialogDescription>
        </DialogHeader>

        <div className="@container flex min-h-0 flex-1 flex-col">
          <TabsPrimitive.Root
            value={selected.id}
            onValueChange={setSelectedId}
            orientation="vertical"
            className="flex min-h-0 flex-1 flex-col gap-4 @2xl:flex-row"
          >
            <TabsPrimitive.List
              aria-label={title}
              className="isolate flex shrink-0 scrollbar-none gap-1 overflow-x-auto @2xl:w-52 @2xl:flex-col @2xl:overflow-visible"
            >
              {examples.map((example) => {
                const active = example.id === selected.id;
                return (
                  <TabsPrimitive.Trigger
                    key={example.id}
                    value={example.id}
                    data-testid={`code-example-${example.id}`}
                    className={cn(
                      "relative flex shrink-0 cursor-pointer items-center gap-2 rounded-lg px-2.5 py-1.5 text-left text-xs whitespace-nowrap transition-colors duration-150 outline-none focus-visible:ring-2 focus-visible:ring-foreground/15",
                      active
                        ? "font-medium text-foreground"
                        : "text-muted-foreground hover:bg-foreground/4 hover:text-foreground",
                    )}
                  >
                    {active && (
                      <motion.span
                        layoutId={
                          animate ? `example-marker-${markerId}` : undefined
                        }
                        initial={false}
                        aria-hidden="true"
                        className="absolute inset-0 -z-10 rounded-lg bg-foreground/7"
                        transition={
                          animate
                            ? { type: "spring", stiffness: 420, damping: 36 }
                            : { duration: 0 }
                        }
                      />
                    )}
                    {example.method && (
                      <span className="w-9 shrink-0 font-mono text-[10px] text-muted-foreground">
                        {example.method}
                      </span>
                    )}
                    <span className="min-w-0 flex-1 truncate">
                      {example.title}
                    </span>
                    {example.paid && <ProBadge />}
                  </TabsPrimitive.Trigger>
                );
              })}
            </TabsPrimitive.List>

            {examples.map((example) => {
              const languages = languagesOf(example);
              const shown = languages.includes(language)
                ? language
                : (languages[0] ?? "curl");
              return (
                <TabsPrimitive.Content
                  key={example.id}
                  value={example.id}
                  className="flex min-h-0 min-w-0 flex-1 flex-col gap-3 overflow-y-auto outline-none"
                >
                  <motion.div
                    initial={animate ? { x: 6 } : false}
                    animate={{ x: 0 }}
                    transition={{ duration: 0.18, ease: MOTION_EASE_OUT }}
                    className="flex flex-col gap-1"
                  >
                    <h3 className="flex items-center gap-2 text-sm font-medium">
                      {example.title}
                      {example.paid && (
                        <span className="text-xs font-normal text-muted-foreground">
                          {t("integrations.examples.paid")}
                        </span>
                      )}
                    </h3>
                    <p className="text-xs leading-relaxed text-muted-foreground">
                      {example.description}
                    </p>
                  </motion.div>

                  <AnimatedTabs
                    value={shown}
                    onValueChange={(value) => {
                      setLanguage(value as SnippetLanguage);
                    }}
                    className="flex min-w-0 flex-col gap-2"
                  >
                    <AnimatedTabsList
                      aria-label={t("integrations.examples.language")}
                    >
                      {languages.map((item) => (
                        <AnimatedTabsTrigger
                          key={item}
                          value={item}
                          className="h-6 px-2 text-xs"
                        >
                          {t(`integrations.examples.languages.${item}`)}
                        </AnimatedTabsTrigger>
                      ))}
                    </AnimatedTabsList>
                    {languages.map((item) => {
                      const snippet = example.snippets[item];
                      if (!snippet) return null;
                      return (
                        <AnimatedTabsContent key={item} value={item}>
                          <motion.div
                            initial={animate ? { y: 4 } : false}
                            animate={{ y: 0 }}
                            transition={{
                              duration: 0.16,
                              ease: MOTION_EASE_OUT,
                            }}
                          >
                            <CodeSnippet
                              multiline
                              code={snippet.display}
                              copyText={snippet.copy}
                              highlights={highlights}
                              successMessage={copyMessage}
                              className="text-xs leading-relaxed"
                            />
                          </motion.div>
                        </AnimatedTabsContent>
                      );
                    })}
                  </AnimatedTabs>

                  {example.response && (
                    <div className="flex min-w-0 flex-col gap-2">
                      <p className={SECTION_LABEL_CLASS}>
                        {t("integrations.examples.response")}
                      </p>
                      <CodeSnippet
                        multiline
                        copyable={false}
                        code={example.response}
                        className="text-xs leading-relaxed text-muted-foreground"
                      />
                    </div>
                  )}
                </TabsPrimitive.Content>
              );
            })}
          </TabsPrimitive.Root>
        </div>

        {note && (
          <p className="shrink-0 text-xs leading-relaxed text-muted-foreground">
            {note}
          </p>
        )}
      </DialogContent>
    </Dialog>
  );
}
