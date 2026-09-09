"use client";

import { motion, useReducedMotion } from "motion/react";
import { useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { LuCookie, LuRotateCcw, LuUserRound } from "react-icons/lu";
import { Button } from "@/components/ui/button";
import { MOTION_EASE_OUT } from "@/lib/motion";
import { cn } from "@/lib/utils";

type ExampleId = "research" | "shopping";
type ExampleState = {
  signedIn: boolean;
  proxy: boolean;
  cookieRevision: number;
  animateCookie: boolean;
  animateRoute: boolean;
};

function freshExamples(): Record<ExampleId, ExampleState> {
  return {
    research: {
      signedIn: false,
      proxy: false,
      cookieRevision: 0,
      animateCookie: false,
      animateRoute: false,
    },
    shopping: {
      signedIn: false,
      proxy: false,
      cookieRevision: 0,
      animateCookie: false,
      animateRoute: false,
    },
  };
}

const EXAMPLE_IDS: ExampleId[] = ["research", "shopping"];

export function ProfileIsolationDemo({
  showHeading = true,
}: {
  showHeading?: boolean;
}) {
  const { t } = useTranslation();
  const reduceMotion = useReducedMotion();
  const id = useId();
  const [selected, setSelected] = useState<ExampleId>("research");
  const [examples, setExamples] = useState(freshExamples);
  const [feedback, setFeedback] = useState<ExampleId | "reset" | null>(null);

  const changeExample = (field: "signedIn" | "proxy", pointer: boolean) => {
    setExamples((previous) => ({
      ...previous,
      [selected]: {
        ...previous[selected],
        [field]: !previous[selected][field],
        cookieRevision:
          previous[selected].cookieRevision + (field === "signedIn" ? 1 : 0),
        animateCookie: field === "signedIn" && pointer,
        animateRoute: field === "proxy" && pointer,
      },
    }));
    setFeedback(selected);
  };

  return (
    <section data-slot="profile-isolation-demo" className="space-y-4">
      {showHeading && (
        <div className="space-y-1">
          <h3 className="text-base font-semibold">
            {t("isolationDemo.title")}
          </h3>
          <p className="text-xs leading-relaxed text-muted-foreground">
            {t("isolationDemo.description")}
          </p>
        </div>
      )}

      <div className="grid grid-cols-2 gap-3 rounded-xl bg-muted/35 p-2 sm:gap-5 sm:p-3">
        {EXAMPLE_IDS.map((profileId) => {
          const profile = examples[profileId];
          const active = selected === profileId;
          const name = t(`isolationDemo.${profileId}`);
          return (
            <div
              key={profileId}
              data-slot="isolation-example-profile"
              data-profile-id={profileId}
              data-signed-in={profile.signedIn}
              data-route={profile.proxy ? "proxy" : "direct"}
              className="row-span-4 grid min-w-0 grid-rows-subgrid gap-3"
            >
              <Button
                id={`${id}-${profileId}`}
                type="button"
                variant="ghost"
                aria-pressed={active}
                data-slot="isolation-select-profile"
                onClick={() => setSelected(profileId)}
                className={cn(
                  "h-auto min-h-9 w-full justify-start gap-2 px-2 py-2 whitespace-normal",
                  active && "bg-accent text-accent-foreground",
                )}
              >
                <LuUserRound className="size-4 shrink-0" aria-hidden="true" />
                <span className="min-w-0 text-start break-words">{name}</span>
              </Button>

              <div className="space-y-1.5 px-2">
                <p className="text-xs text-muted-foreground">
                  {t("isolationDemo.cookies")}
                </p>
                <div className="flex items-start gap-2">
                  <motion.span
                    key={profile.cookieRevision}
                    initial={
                      profile.animateCookie && !reduceMotion
                        ? { rotate: 0 }
                        : false
                    }
                    animate={{
                      rotate:
                        profile.animateCookie && !reduceMotion
                          ? [0, -16, 0]
                          : 0,
                    }}
                    transition={{ duration: 0.28, ease: MOTION_EASE_OUT }}
                    className="mt-0.5 shrink-0"
                    aria-hidden="true"
                  >
                    <LuCookie
                      className={cn(
                        "size-4",
                        profile.signedIn
                          ? "text-foreground"
                          : "text-muted-foreground",
                      )}
                    />
                  </motion.span>
                  <p
                    data-slot="isolation-cookie-value"
                    className="text-sm leading-5"
                  >
                    {t(
                      profile.signedIn
                        ? "isolationDemo.savedCookie"
                        : "isolationDemo.noCookie",
                    )}
                  </p>
                </div>
              </div>

              <div className="space-y-1.5 px-2">
                <p className="text-xs text-muted-foreground">
                  {t("isolationDemo.route")}
                </p>
                <p
                  data-slot="isolation-route-value"
                  className="text-sm leading-5"
                >
                  {t(
                    profile.proxy
                      ? "isolationDemo.proxy"
                      : "isolationDemo.direct",
                  )}
                </p>
              </div>
              <div className="px-2 pb-1">
                <svg
                  viewBox="0 0 128 40"
                  preserveAspectRatio="none"
                  className="h-10 w-full text-muted-foreground"
                  fill="none"
                  aria-hidden="true"
                >
                  <motion.path
                    initial={false}
                    animate={{
                      d: profile.proxy
                        ? "M 32 28 C 44 28 52 8 64 8 C 76 8 84 28 96 28"
                        : "M 32 28 C 44 28 52 28 64 28 C 76 28 84 28 96 28",
                    }}
                    transition={{
                      duration:
                        profile.animateRoute && !reduceMotion ? 0.32 : 0,
                      ease: MOTION_EASE_OUT,
                    }}
                    stroke="currentColor"
                    strokeWidth="1.5"
                    strokeLinecap="round"
                    vectorEffect="non-scaling-stroke"
                  />
                  <path
                    d="M 32 28 h 0.001 M 96 28 h 0.001"
                    stroke="currentColor"
                    strokeWidth="6"
                    strokeLinecap="round"
                    vectorEffect="non-scaling-stroke"
                    className="text-foreground"
                  />
                  <motion.path
                    initial={false}
                    animate={{
                      d: profile.proxy ? "M 64 8 h 0.001" : "M 64 28 h 0.001",
                      strokeWidth: profile.proxy ? 8 : 0,
                    }}
                    transition={{
                      duration:
                        profile.animateRoute && !reduceMotion ? 0.32 : 0,
                      ease: MOTION_EASE_OUT,
                    }}
                    stroke="currentColor"
                    strokeLinecap="round"
                    vectorEffect="non-scaling-stroke"
                    className="text-foreground"
                  />
                </svg>
                <div className="grid grid-cols-2 text-center text-xs text-muted-foreground">
                  <span>{t("isolationDemo.device")}</span>
                  <span>{t("isolationDemo.site")}</span>
                </div>
              </div>
            </div>
          );
        })}
      </div>

      <div
        role="group"
        aria-labelledby={`${id}-${selected}`}
        className="flex flex-wrap items-center gap-2"
      >
        <Button
          type="button"
          variant="secondary"
          size="sm"
          data-slot="isolation-toggle-cookie"
          onClick={(event) => changeExample("signedIn", event.detail > 0)}
        >
          {t(
            examples[selected].signedIn
              ? "isolationDemo.signOut"
              : "isolationDemo.signIn",
          )}
        </Button>
        <Button
          type="button"
          variant="secondary"
          size="sm"
          data-slot="isolation-toggle-route"
          onClick={(event) => changeExample("proxy", event.detail > 0)}
        >
          {t("isolationDemo.switchRoute")}
        </Button>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          data-slot="isolation-reset"
          className="ms-auto text-muted-foreground hover:text-foreground"
          onClick={() => {
            setExamples(freshExamples());
            setFeedback("reset");
          }}
        >
          <LuRotateCcw className="size-3.5" aria-hidden="true" />
          {t("isolationDemo.reset")}
        </Button>
      </div>

      <p
        role="status"
        aria-live="polite"
        aria-atomic="true"
        data-slot="isolation-feedback"
        className="min-h-8 text-xs leading-relaxed text-muted-foreground"
      >
        {feedback === "reset"
          ? t("isolationDemo.resetDone")
          : feedback
            ? t("isolationDemo.changed", {
                name: t(`isolationDemo.${feedback}`),
              })
            : t("isolationDemo.hint")}
      </p>
    </section>
  );
}
