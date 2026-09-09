"use client";

import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { LuChevronLeft, LuChevronRight } from "react-icons/lu";
import { TipScene } from "@/components/tips/scene-for";
import { AnimatedSwitch } from "@/components/ui/animated-switch";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogTitle } from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import { useInputModality } from "@/hooks/use-input-modality";
import { MOTION_EASE_OUT, MOTION_SPRING_POSITION } from "@/lib/motion";
import { isMacOS } from "@/lib/platform";
import {
  isPlanTip,
  type TipAction,
  type TipDefinition,
  type TipId,
  tipTextKeys,
} from "@/lib/tips";
import { cn } from "@/lib/utils";

export type TipsDialogMode = "browse" | "single";

interface TipsDialogProps {
  open: boolean;
  /** `browse` shows the whole catalog beside the tip; `single` is one card. */
  mode: TipsDialogMode;
  tips: TipDefinition[];
  seen: string[];
  initialTipId: TipId | null;
  /** True when the automatic flow opened the dialog, not the user. */
  auto: boolean;
  autoShow: boolean;
  onOpenChange: (open: boolean) => void;
  onTipShown: (id: TipId, auto: boolean) => void;
  onAutoShowChange: (enabled: boolean) => void;
  onAction: (action: TipAction) => void;
}

function isTypingTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return (
    target.isContentEditable ||
    ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName)
  );
}

/**
 * Feature tips: a drawing of the feature in motion, a few lines on what it
 * does for the user, and a button into the place it lives. The catalog on
 * the left is only there in browse mode; the automatic flow is one card.
 */
export function TipsDialog({
  open,
  mode,
  tips,
  seen,
  initialTipId,
  auto,
  autoShow,
  onOpenChange,
  onTipShown,
  onAutoShowChange,
  onAction,
}: TipsDialogProps) {
  const { t } = useTranslation();
  const reduceMotion = useReducedMotion();
  const modality = useInputModality();
  const animate = !reduceMotion && modality === "pointer";
  const listId = useId();
  const autoShowId = useId();
  const total = tips.length;
  const initialIndex = Math.max(
    0,
    initialTipId ? tips.findIndex((tip) => tip.id === initialTipId) : 0,
  );
  const [index, setIndex] = useState(initialIndex);
  const [direction, setDirection] = useState<1 | -1>(1);
  const tip = tips[Math.min(index, Math.max(0, total - 1))];
  const mod = isMacOS() ? "⌘" : "Ctrl";

  // Each tip is reported once as it comes on screen, so the automatic flow
  // never repeats one and the catalog can tell seen from new.
  const reportedRef = useRef<string | null>(null);
  useEffect(() => {
    if (!open || !tip || reportedRef.current === tip.id) return;
    reportedRef.current = tip.id;
    onTipShown(tip.id, auto && tip.id === initialTipId);
  }, [open, tip, auto, initialTipId, onTipShown]);

  if (!tip) return null;

  const keys = tipTextKeys(tip.id);
  const isLast = index >= total - 1;
  const go = (next: number) => {
    const clamped = Math.max(0, Math.min(total - 1, next));
    if (clamped === index) return;
    setDirection(clamped > index ? 1 : -1);
    setIndex(clamped);
  };

  const essentials = tips
    .map((item, itemIndex) => ({ item, itemIndex }))
    .filter(({ item }) => !isPlanTip(item));
  const planTips = tips
    .map((item, itemIndex) => ({ item, itemIndex }))
    .filter(({ item }) => isPlanTip(item));
  const sections = [
    { key: "essentials", label: t("tips.essentials"), entries: essentials },
    ...(planTips.length > 0
      ? [{ key: "plan", label: t("tips.planSection"), entries: planTips }]
      : []),
  ];

  const detail = (
    <section
      data-slot="tip-detail"
      data-tip-id={tip.id}
      className="flex min-h-0 min-w-0 flex-col gap-4 overflow-y-auto p-5"
    >
      <DialogTitle className="pr-6 text-lg font-semibold tracking-tight text-balance">
        {t(keys.title, { mod })}
      </DialogTitle>

      <div
        data-slot="tip-scene-panel"
        className="relative h-40 overflow-hidden rounded-lg bg-muted/35"
      >
        <AnimatePresence mode="popLayout" initial={false}>
          <motion.div
            key={tip.id}
            className="absolute inset-0 p-3"
            initial={animate ? { x: direction * 24 } : false}
            animate={{ x: 0 }}
            exit={animate ? { x: -direction * 24, opacity: 0 } : { opacity: 0 }}
            transition={{ duration: 0.22, ease: MOTION_EASE_OUT }}
          >
            <TipScene id={tip.id} />
          </motion.div>
        </AnimatePresence>
      </div>

      <div className="flex flex-col gap-1.5">
        {isPlanTip(tip) && (
          <p className="text-xs text-muted-foreground">
            {t("tips.planSection")}
          </p>
        )}
        <p className="text-sm/6 text-pretty text-muted-foreground">
          {t(keys.body, { mod })}
        </p>
      </div>

      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex items-center gap-1">
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="size-8 p-0 text-muted-foreground hover:text-foreground"
            aria-label={t("tips.previous")}
            data-slot="tip-previous"
            disabled={index === 0}
            onClick={() => go(index - 1)}
          >
            <LuChevronLeft className="size-4" />
          </Button>
          <span
            className="min-w-[5ch] text-center text-xs text-muted-foreground tabular-nums"
            aria-live="polite"
          >
            {t("tips.count", { current: index + 1, total })}
          </span>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="size-8 p-0 text-muted-foreground hover:text-foreground"
            aria-label={t("tips.next")}
            data-slot="tip-next"
            disabled={isLast}
            onClick={() => go(index + 1)}
          >
            <LuChevronRight className="size-4" />
          </Button>
        </div>
        <div className="flex items-center gap-2">
          <Button
            type="button"
            variant="ghost"
            size="sm"
            data-slot="tip-action"
            onClick={() => onAction(tip.action)}
          >
            {t(keys.action)}
          </Button>
          <Button
            type="button"
            size="sm"
            data-slot="tip-advance"
            onClick={() => {
              if (isLast) onOpenChange(false);
              else go(index + 1);
            }}
          >
            {t(isLast ? "tips.done" : "tips.next")}
          </Button>
        </div>
      </div>

      <div className="flex items-center gap-2">
        <AnimatedSwitch
          id={autoShowId}
          data-slot="tips-auto-show"
          checked={autoShow}
          onCheckedChange={onAutoShowChange}
        />
        <Label
          htmlFor={autoShowId}
          className="cursor-pointer text-xs font-normal text-muted-foreground"
        >
          {t("tips.autoShow")}
        </Label>
      </div>
    </section>
  );

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        data-slot="tips-dialog"
        data-mode={mode}
        className={cn(
          "max-h-[calc(100vh-3rem)] gap-0 overflow-hidden p-0",
          mode === "browse" ? "sm:max-w-2xl" : "sm:max-w-md",
        )}
        onKeyDown={(event) => {
          if (isTypingTarget(event.target)) return;
          if (event.key === "ArrowRight") {
            event.preventDefault();
            go(index + 1);
          } else if (event.key === "ArrowLeft") {
            event.preventDefault();
            go(index - 1);
          }
        }}
      >
        {mode === "browse" ? (
          <div className="grid max-h-[calc(100vh-3rem)] min-h-0 sm:grid-cols-[11rem_minmax(0,1fr)]">
            <aside className="hidden min-h-0 overflow-hidden border-r sm:flex sm:flex-col">
              <h2 className="px-4 pt-5 pb-1 text-sm font-semibold">
                {t("tips.title")}
              </h2>
              <nav
                aria-label={t("tips.title")}
                className="min-h-0 flex-1 overflow-y-auto px-2 pb-3"
              >
                {sections.map((section) => (
                  <div key={section.key}>
                    <p className="px-2 pt-3 pb-1 text-[11px] font-medium text-muted-foreground">
                      {section.label}
                    </p>
                    <ul data-slot="tips-list">
                      {section.entries.map(({ item, itemIndex }) => {
                        const active = itemIndex === index;
                        return (
                          <li key={item.id}>
                            <button
                              type="button"
                              data-slot="tips-list-item"
                              data-tip-id={item.id}
                              data-seen={seen.includes(item.id)}
                              aria-current={active ? "true" : undefined}
                              onClick={() => go(itemIndex)}
                              className={cn(
                                "relative isolate flex w-full cursor-pointer items-center rounded-md px-2 py-1.5 text-left text-sm transition-colors duration-150 focus-visible:outline-2 focus-visible:outline-ring",
                                active
                                  ? "text-accent-foreground"
                                  : seen.includes(item.id)
                                    ? "text-muted-foreground hover:text-foreground"
                                    : "text-foreground",
                              )}
                            >
                              {active && (
                                <motion.span
                                  aria-hidden="true"
                                  data-slot="tips-list-indicator"
                                  layoutId={
                                    animate ? `${listId}-indicator` : undefined
                                  }
                                  initial={false}
                                  transition={
                                    animate
                                      ? MOTION_SPRING_POSITION
                                      : { duration: 0 }
                                  }
                                  className="absolute inset-0 -z-10 rounded-md bg-accent"
                                />
                              )}
                              <span className="truncate">
                                {t(tipTextKeys(item.id).label)}
                              </span>
                            </button>
                          </li>
                        );
                      })}
                    </ul>
                  </div>
                ))}
              </nav>
            </aside>
            {detail}
          </div>
        ) : (
          <div className="grid max-h-[calc(100vh-3rem)] min-h-0">{detail}</div>
        )}
      </DialogContent>
    </Dialog>
  );
}
