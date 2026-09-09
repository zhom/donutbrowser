"use client";

import { motion, useReducedMotion } from "motion/react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { LuCheck, LuMousePointer2 } from "react-icons/lu";
import { Button } from "@/components/ui/button";
import { MOTION_EASE_OUT } from "@/lib/motion";
import type { BrowserProfile } from "@/types";

export function SynchronizerRehearsal({
  leader,
  followers,
  disabled = false,
}: {
  leader: Pick<BrowserProfile, "id" | "name">;
  followers: Pick<BrowserProfile, "id" | "name">[];
  disabled?: boolean;
}) {
  const { t } = useTranslation();
  const reduceMotion = useReducedMotion();
  const [rehearsal, setRehearsal] = useState({ revision: 0, animate: false });
  const hasRehearsed = rehearsal.revision > 0;

  return (
    <section data-slot="synchronizer-rehearsal" className="space-y-3">
      <div className="space-y-1">
        <h3 className="text-sm font-medium">
          {t("synchronizerPreview.title")}
        </h3>
        <p className="text-xs leading-relaxed text-muted-foreground">
          {t("synchronizerPreview.description")}
        </p>
      </div>

      <div className="max-h-48 overflow-y-auto rounded-lg bg-muted/40 p-3">
        <div className="grid grid-cols-[minmax(0,1fr)_2.5rem_minmax(0,1fr)] items-stretch">
          <div className="flex min-w-0 flex-col justify-center gap-1 py-2">
            <span className="text-xs text-muted-foreground">
              {t("profiles.synchronizer.leader")}
            </span>
            <span className="text-sm font-medium break-words">
              {leader.name}
            </span>
          </div>

          <div className="relative min-h-12" aria-hidden="true">
            {followers.length > 0 && (
              <svg
                viewBox="0 0 40 100"
                preserveAspectRatio="none"
                className="absolute inset-0 h-full w-full overflow-visible"
                fill="none"
                aria-hidden="true"
              >
                {followers.map((follower, index) => {
                  const end = ((index + 0.5) / followers.length) * 100;
                  const path = `M 4 50 C 20 50, 20 ${end}, 36 ${end}`;
                  return (
                    <g key={follower.id}>
                      <path
                        d={path}
                        className="stroke-muted-foreground/40"
                        strokeWidth="1.5"
                        strokeLinecap="round"
                        vectorEffect="non-scaling-stroke"
                      />
                      {rehearsal.animate && !reduceMotion && (
                        <motion.path
                          key={rehearsal.revision}
                          d={path}
                          className="stroke-foreground"
                          strokeWidth="2.5"
                          strokeLinecap="round"
                          vectorEffect="non-scaling-stroke"
                          initial={{ pathLength: 0.14, pathOffset: 0 }}
                          animate={{ pathLength: 0.14, pathOffset: 0.86 }}
                          transition={{ duration: 0.22, ease: MOTION_EASE_OUT }}
                        />
                      )}
                    </g>
                  );
                })}
              </svg>
            )}
          </div>

          {followers.length > 0 ? (
            <ul className="grid min-w-0 auto-rows-fr">
              {followers.map((follower) => (
                <li
                  key={follower.id}
                  data-slot="synchronizer-preview-follower"
                  data-profile-id={follower.id}
                  data-received={hasRehearsed || undefined}
                  className="flex min-w-0 items-center gap-2 py-2 text-sm"
                >
                  <span className="min-w-0 flex-1 break-words">
                    {follower.name}
                  </span>
                  {hasRehearsed && (
                    <LuCheck
                      className="size-3.5 shrink-0 text-foreground"
                      aria-label={t("synchronizerPreview.received")}
                      role="img"
                    />
                  )}
                </li>
              ))}
            </ul>
          ) : (
            <p className="self-center py-2 text-xs leading-relaxed text-muted-foreground">
              {t("synchronizerPreview.empty")}
            </p>
          )}
        </div>
      </div>

      <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
        <Button
          type="button"
          variant="secondary"
          size="sm"
          data-slot="synchronizer-preview-send"
          disabled={disabled || followers.length === 0}
          onClick={(event) => {
            setRehearsal((previous) => ({
              revision: previous.revision + 1,
              animate: event.detail > 0,
            }));
          }}
        >
          <LuMousePointer2 className="size-3.5" aria-hidden="true" />
          {t("synchronizerPreview.send")}
        </Button>
        <p
          role="status"
          aria-live="polite"
          aria-atomic="true"
          data-slot="synchronizer-preview-status"
          className="min-w-0 flex-1 text-xs text-muted-foreground"
        >
          {t(
            hasRehearsed
              ? "synchronizerPreview.sent"
              : "synchronizerPreview.ready",
            { count: followers.length },
          )}
        </p>
      </div>
    </section>
  );
}
