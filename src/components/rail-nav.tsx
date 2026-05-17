"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { FaDownload } from "react-icons/fa";
import { FiWifi } from "react-icons/fi";
import { GoGear, GoKebabHorizontal } from "react-icons/go";
import {
  LuCloud,
  LuKeyboard,
  LuPlug,
  LuPuzzle,
  LuUser,
  LuUsers,
} from "react-icons/lu";
import { cn } from "@/lib/utils";
import { Logo } from "./icons/logo";
import { Tooltip, TooltipContent, TooltipTrigger } from "./ui/tooltip";

export type AppPage =
  | "profiles"
  | "proxies"
  | "extensions"
  | "groups"
  | "vpns"
  | "settings"
  | "integrations"
  | "account"
  | "import"
  | "shortcuts";

const CLICK_THRESHOLD = 5;
const CLICK_WINDOW_MS = 2000;
const GRAVITY = 2200;
const BOUNCE_DAMPING = 0.6;
const INITIAL_HORIZONTAL_SPEED = 350;
const SPIN_SPEED = 720;
const MIN_BOUNCE_VELOCITY = 60;
const LOGO_HIDDEN_KEY = "donut-logo-hidden";

function useLogoEasterEgg({
  currentPage,
  onNavigate,
}: {
  currentPage: AppPage;
  onNavigate: (page: AppPage) => void;
}) {
  const clickTimestamps = useRef<number[]>([]);
  const [isPressed, setIsPressed] = useState(false);
  const [wobbleKey, setWobbleKey] = useState(0);
  const [isFalling, setIsFalling] = useState(false);
  /**
   * Click count toward the bounce trigger while the user is on the profiles
   * page. Capped at 4: each click here grows the logo by 25%, so step 4 has
   * doubled the original size. Click 5 fires `triggerFall` and resets.
   */
  const [growStep, setGrowStep] = useState(0);
  const resetTimeoutRef = useRef<number | null>(null);
  const [isHidden, setIsHidden] = useState(() => {
    try {
      return sessionStorage.getItem(LOGO_HIDDEN_KEY) === "1";
    } catch {
      return false;
    }
  });
  const logoRef = useRef<HTMLButtonElement>(null);
  const animFrameRef = useRef<number>(0);

  const triggerFall = useCallback(() => {
    const el = logoRef.current;
    if (!el || isFalling) return;
    setIsFalling(true);

    const rect = el.getBoundingClientRect();
    const startX = rect.left;
    const startY = rect.top;
    const floorY = window.innerHeight;
    const rightWall = window.innerWidth;

    const clone = el.cloneNode(true) as HTMLElement;
    clone.style.position = "fixed";
    clone.style.left = `${startX}px`;
    clone.style.top = `${startY}px`;
    clone.style.zIndex = "9999";
    clone.style.pointerEvents = "none";
    clone.style.margin = "0";
    document.body.appendChild(clone);
    el.style.visibility = "hidden";

    let x = 0;
    let y = 0;
    let vy = -500;
    // Roll right first, bounce off the right wall, then escape the left.
    let vx = INITIAL_HORIZONTAL_SPEED;
    let rotation = 0;
    let lastTime = performance.now();

    const animate = (time: number) => {
      const dt = Math.min((time - lastTime) / 1000, 0.05);
      lastTime = time;

      vy += GRAVITY * dt;
      x += vx * dt;
      y += vy * dt;
      rotation += SPIN_SPEED * dt * (vx > 0 ? 1 : -1);

      const currentBottom = startY + y + rect.height;
      if (currentBottom >= floorY && vy > 0) {
        y = floorY - startY - rect.height;
        vy =
          Math.abs(vy) > MIN_BOUNCE_VELOCITY
            ? -Math.abs(vy) * BOUNCE_DAMPING
            : -MIN_BOUNCE_VELOCITY * 3;
      }

      // Right-wall bounce: hit, reverse horizontal velocity (with a tiny
      // damping), and keep rolling. Left wall has no bounce — the donut
      // exits the window off the left edge.
      const currentRight = startX + x + rect.width;
      if (currentRight >= rightWall && vx > 0) {
        x = rightWall - startX - rect.width;
        vx = -Math.abs(vx) * 0.9;
      }

      clone.style.transform = `translate(${x}px, ${y}px) rotate(${rotation}deg)`;

      const offScreenLeft = startX + x + rect.width < -200;
      const offScreenBottom = startY + y > floorY + 100;
      const offScreenTop = startY + y + rect.height < -200;

      if (offScreenLeft || offScreenBottom || offScreenTop) {
        clone.remove();
        try {
          sessionStorage.setItem(LOGO_HIDDEN_KEY, "1");
        } catch {
          // ignore — sessionStorage unavailable in some Tauri WebViews
        }
        setIsHidden(true);
        setIsFalling(false);
        return;
      }
      animFrameRef.current = requestAnimationFrame(animate);
    };
    animFrameRef.current = requestAnimationFrame(animate);
  }, [isFalling]);

  useEffect(() => {
    return () => {
      if (animFrameRef.current) cancelAnimationFrame(animFrameRef.current);
    };
  }, []);

  const handleClick = useCallback(() => {
    if (isFalling || isHidden) return;

    // First behaviour: any click from elsewhere in the app just routes the
    // user back to the profiles list. Growing the donut requires the user
    // to already be home — that keeps the easter egg from accidentally
    // firing during normal navigation.
    if (currentPage !== "profiles") {
      onNavigate("profiles");
      clickTimestamps.current = [];
      setGrowStep(0);
      if (resetTimeoutRef.current !== null) {
        window.clearTimeout(resetTimeoutRef.current);
        resetTimeoutRef.current = null;
      }
      return;
    }

    const now = Date.now();
    clickTimestamps.current = clickTimestamps.current.filter(
      (t) => now - t < CLICK_WINDOW_MS,
    );
    clickTimestamps.current.push(now);

    if (clickTimestamps.current.length >= CLICK_THRESHOLD) {
      clickTimestamps.current = [];
      setGrowStep(0);
      if (resetTimeoutRef.current !== null) {
        window.clearTimeout(resetTimeoutRef.current);
        resetTimeoutRef.current = null;
      }
      triggerFall();
    } else {
      setGrowStep(
        Math.min(clickTimestamps.current.length, CLICK_THRESHOLD - 1),
      );
      setWobbleKey((k) => k + 1);
      if (resetTimeoutRef.current !== null) {
        window.clearTimeout(resetTimeoutRef.current);
      }
      resetTimeoutRef.current = window.setTimeout(() => {
        clickTimestamps.current = [];
        setGrowStep(0);
        resetTimeoutRef.current = null;
      }, CLICK_WINDOW_MS);
    }
  }, [currentPage, isFalling, isHidden, onNavigate, triggerFall]);

  // Leaving the profiles page mid-streak cancels growth so we never end up
  // with an outsized logo when the user returns later.
  useEffect(() => {
    if (currentPage !== "profiles") {
      clickTimestamps.current = [];
      setGrowStep(0);
      if (resetTimeoutRef.current !== null) {
        window.clearTimeout(resetTimeoutRef.current);
        resetTimeoutRef.current = null;
      }
    }
  }, [currentPage]);

  useEffect(() => {
    return () => {
      if (resetTimeoutRef.current !== null) {
        window.clearTimeout(resetTimeoutRef.current);
      }
    };
  }, []);

  return {
    logoRef,
    isPressed,
    setIsPressed,
    wobbleKey,
    isFalling,
    isHidden,
    growStep,
    handleClick,
  };
}

interface RailNavProps {
  currentPage: AppPage;
  onNavigate: (page: AppPage) => void;
}

interface RailItem {
  page: AppPage;
  Icon: React.ComponentType<{ className?: string }>;
  labelKey: string;
}

const TOP_ITEMS: RailItem[] = [
  { page: "profiles", Icon: LuUser, labelKey: "rail.profiles" },
  { page: "proxies", Icon: FiWifi, labelKey: "rail.network" },
  { page: "extensions", Icon: LuPuzzle, labelKey: "rail.extensions" },
  { page: "groups", Icon: LuUsers, labelKey: "rail.groups" },
  { page: "integrations", Icon: LuPlug, labelKey: "rail.integrations" },
  { page: "account", Icon: LuCloud, labelKey: "rail.account" },
];

interface MoreMenuItem {
  page: AppPage;
  Icon: React.ComponentType<{ className?: string }>;
  labelKey: string;
  hintKey: string;
}

const MORE_ITEMS: MoreMenuItem[] = [
  {
    page: "import",
    Icon: FaDownload,
    labelKey: "rail.more.importProfile",
    hintKey: "rail.more.importProfileHint",
  },
  {
    page: "shortcuts",
    Icon: LuKeyboard,
    labelKey: "rail.more.keyboardShortcuts",
    hintKey: "rail.more.keyboardShortcutsHint",
  },
];

export function RailNav({ currentPage, onNavigate }: RailNavProps) {
  const { t } = useTranslation();
  const [moreOpen, setMoreOpen] = useState(false);
  const {
    logoRef,
    isPressed,
    setIsPressed,
    wobbleKey,
    isFalling,
    isHidden,
    growStep,
    handleClick,
  } = useLogoEasterEgg({ currentPage, onNavigate });

  return (
    <nav className="flex flex-col items-center w-10 py-2 gap-1 bg-background border-r border-border shrink-0 relative">
      {!isHidden ? (
        <button
          ref={logoRef}
          type="button"
          aria-label={t("header.donutLogo")}
          className="grid place-items-center size-7 rounded-md cursor-pointer select-none text-foreground bg-transparent"
          onClick={handleClick}
          onPointerDown={() => {
            setIsPressed(true);
          }}
          onPointerUp={() => {
            setIsPressed(false);
          }}
          onPointerLeave={() => {
            setIsPressed(false);
          }}
        >
          {/* Inner wrapper survives clicks (no `key`) so the scale change
              animates smoothly across the wiggle layer's remounts. */}
          <span
            style={{
              transform: isPressed
                ? `scale(${(1 + growStep * 0.25) * 0.9})`
                : `scale(${1 + growStep * 0.25})`,
            }}
            className="inline-grid place-items-center transition-transform duration-300 ease-out will-change-transform"
          >
            <span
              key={wobbleKey}
              className={cn(
                "inline-grid place-items-center",
                !isFalling &&
                  !isPressed &&
                  wobbleKey > 0 &&
                  "animate-[wiggle_0.3s_ease-in-out]",
              )}
            >
              <Logo className="size-5 will-change-transform" />
            </span>
          </span>
        </button>
      ) : (
        <div className="size-7" />
      )}

      <div className="w-5 h-px bg-border my-1" />

      {TOP_ITEMS.map(({ page, Icon, labelKey }) => {
        const active = currentPage === page;
        return (
          <Tooltip key={page} delayDuration={300}>
            <TooltipTrigger asChild>
              <button
                type="button"
                onClick={() => {
                  onNavigate(page);
                }}
                aria-label={t(labelKey)}
                aria-current={active ? "page" : undefined}
                className={cn(
                  "relative grid place-items-center size-7 rounded-md transition-colors duration-100",
                  active
                    ? "text-foreground bg-accent"
                    : "text-muted-foreground hover:text-card-foreground hover:bg-accent/50",
                )}
              >
                {active && (
                  <span
                    aria-hidden="true"
                    className="absolute left-[-7px] top-1.5 bottom-1.5 w-[2px] rounded-full bg-foreground"
                  />
                )}
                <Icon className="size-3.5" />
              </button>
            </TooltipTrigger>
            <TooltipContent side="right">{t(labelKey)}</TooltipContent>
          </Tooltip>
        );
      })}

      <div className="flex-1" />

      <Tooltip delayDuration={300}>
        <TooltipTrigger asChild>
          <button
            type="button"
            onClick={() => {
              setMoreOpen((v) => !v);
            }}
            aria-label={t("rail.more.label")}
            aria-expanded={moreOpen}
            className={cn(
              "grid place-items-center size-7 rounded-md transition-colors duration-100",
              moreOpen
                ? "text-foreground bg-accent"
                : "text-muted-foreground hover:text-card-foreground hover:bg-accent/50",
            )}
          >
            <GoKebabHorizontal className="size-3.5" />
          </button>
        </TooltipTrigger>
        <TooltipContent side="right">{t("rail.more.label")}</TooltipContent>
      </Tooltip>

      <Tooltip delayDuration={300}>
        <TooltipTrigger asChild>
          <button
            type="button"
            onClick={() => {
              onNavigate("settings");
            }}
            aria-label={t("rail.settings")}
            aria-current={currentPage === "settings" ? "page" : undefined}
            className={cn(
              "relative grid place-items-center size-7 rounded-md transition-colors duration-100",
              currentPage === "settings"
                ? "text-foreground bg-accent"
                : "text-muted-foreground hover:text-card-foreground hover:bg-accent/50",
            )}
          >
            {currentPage === "settings" && (
              <span
                aria-hidden="true"
                className="absolute left-[-7px] top-1.5 bottom-1.5 w-[2px] rounded-full bg-foreground"
              />
            )}
            <GoGear className="size-3.5" />
          </button>
        </TooltipTrigger>
        <TooltipContent side="right">{t("rail.settings")}</TooltipContent>
      </Tooltip>

      {moreOpen && (
        <>
          <button
            type="button"
            aria-label={t("rail.more.closeAriaLabel")}
            className="fixed inset-0 z-30 bg-transparent cursor-default"
            onClick={() => {
              setMoreOpen(false);
            }}
          />
          <div className="absolute bottom-14 left-11 w-56 bg-card border border-border rounded-lg shadow-2xl p-1 z-40 animate-in fade-in-0 slide-in-from-bottom-1 duration-100">
            {MORE_ITEMS.map(({ page, Icon, labelKey, hintKey }) => (
              <button
                key={page}
                type="button"
                onClick={() => {
                  setMoreOpen(false);
                  onNavigate(page);
                }}
                className="flex items-center gap-2 w-full px-2 py-1.5 rounded-md hover:bg-accent transition-colors duration-100 text-left"
              >
                <span className="grid place-items-center size-5 rounded bg-muted text-muted-foreground shrink-0">
                  <Icon className="size-3" />
                </span>
                <span className="flex flex-col min-w-0">
                  <span className="text-xs font-medium text-foreground truncate">
                    {t(labelKey)}
                  </span>
                  <span className="text-[10px] text-muted-foreground truncate">
                    {t(hintKey)}
                  </span>
                </span>
              </button>
            ))}
          </div>
        </>
      )}
    </nav>
  );
}
