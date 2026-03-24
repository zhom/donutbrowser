import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { FaDownload } from "react-icons/fa";
import { FiWifi } from "react-icons/fi";
import { GoGear, GoKebabHorizontal, GoPlus } from "react-icons/go";
import {
  LuCloud,
  LuPlug,
  LuPuzzle,
  LuSearch,
  LuUsers,
  LuX,
} from "react-icons/lu";
import { cn } from "@/lib/utils";
import { Logo } from "./icons/logo";
import { Button } from "./ui/button";
import { CardTitle } from "./ui/card";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "./ui/dropdown-menu";
import { Input } from "./ui/input";
import { Tooltip, TooltipContent, TooltipTrigger } from "./ui/tooltip";

const CLICK_THRESHOLD = 5;
const CLICK_WINDOW_MS = 2000;
const GRAVITY = 2200;
const BOUNCE_DAMPING = 0.6;
const INITIAL_HORIZONTAL_SPEED = 350;
const SPIN_SPEED = 720;
const MIN_BOUNCE_VELOCITY = 60;
const LOGO_HIDDEN_KEY = "donut-logo-hidden";

function useLogoEasterEgg() {
  const clickTimestamps = useRef<number[]>([]);
  const [isPressed, setIsPressed] = useState(false);
  const [wobbleKey, setWobbleKey] = useState(0);
  const [isFalling, setIsFalling] = useState(false);
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
    const leftWall = 0;
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
    let vx = -INITIAL_HORIZONTAL_SPEED;
    let rotation = 0;
    let lastTime = performance.now();

    const animate = (time: number) => {
      const dt = Math.min((time - lastTime) / 1000, 0.05);
      lastTime = time;

      vy += GRAVITY * dt;
      x += vx * dt;
      y += vy * dt;
      rotation += SPIN_SPEED * dt * (vx > 0 ? 1 : -1);

      // Floor bounce
      const currentBottom = startY + y + rect.height;
      if (currentBottom >= floorY && vy > 0) {
        y = floorY - startY - rect.height;
        if (Math.abs(vy) > MIN_BOUNCE_VELOCITY) {
          vy = -Math.abs(vy) * BOUNCE_DAMPING;
        } else {
          vy = -MIN_BOUNCE_VELOCITY * 3;
        }
      }

      // Left wall bounce only — right wall lets it fly off screen
      const currentLeft = startX + x;
      if (currentLeft <= leftWall && vx < 0) {
        x = leftWall - startX;
        vx = Math.abs(vx) * 1.1;
      }

      clone.style.transform = `translate(${x}px, ${y}px) rotate(${rotation}deg)`;

      // Only end when fully off-screen vertically (bounced out the top or flew off bottom somehow)
      const currentTop = startY + y;
      const offScreenRight = startX + x > rightWall + 50;
      const offScreenBottom = currentTop > floorY + 100;
      const offScreenTop = currentTop + rect.height < -200;

      if (offScreenRight || offScreenBottom || offScreenTop) {
        clone.remove();
        try {
          sessionStorage.setItem(LOGO_HIDDEN_KEY, "1");
        } catch {
          // ignore
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

    const now = Date.now();
    clickTimestamps.current = clickTimestamps.current.filter(
      (t) => now - t < CLICK_WINDOW_MS,
    );
    clickTimestamps.current.push(now);

    if (clickTimestamps.current.length >= CLICK_THRESHOLD) {
      clickTimestamps.current = [];
      triggerFall();
    } else {
      setWobbleKey((k) => k + 1);
    }
  }, [isFalling, isHidden, triggerFall]);

  return {
    logoRef,
    isPressed,
    setIsPressed,
    wobbleKey,
    isFalling,
    isHidden,
    handleClick,
  };
}

type Props = {
  onSettingsDialogOpen: (open: boolean) => void;
  onProxyManagementDialogOpen: (open: boolean) => void;
  onGroupManagementDialogOpen: (open: boolean) => void;
  onImportProfileDialogOpen: (open: boolean) => void;
  onCreateProfileDialogOpen: (open: boolean) => void;
  onSyncConfigDialogOpen: (open: boolean) => void;
  onIntegrationsDialogOpen: (open: boolean) => void;
  onExtensionManagementDialogOpen: (open: boolean) => void;
  searchQuery: string;
  onSearchQueryChange: (query: string) => void;
};

const HomeHeader = ({
  onSettingsDialogOpen,
  onProxyManagementDialogOpen,
  onGroupManagementDialogOpen,
  onImportProfileDialogOpen,
  onCreateProfileDialogOpen,
  onSyncConfigDialogOpen,
  onIntegrationsDialogOpen,
  onExtensionManagementDialogOpen,
  searchQuery,
  onSearchQueryChange,
}: Props) => {
  const { t } = useTranslation();
  const {
    logoRef,
    isPressed,
    setIsPressed,
    wobbleKey,
    isFalling,
    isHidden,
    handleClick,
  } = useLogoEasterEgg();

  return (
    <div className="flex justify-between items-center mt-6">
      <div className="flex gap-3 items-center">
        {!isHidden ? (
          <button
            ref={logoRef}
            type="button"
            className="p-1 cursor-pointer select-none"
            onClick={handleClick}
            onPointerDown={() => setIsPressed(true)}
            onPointerUp={() => setIsPressed(false)}
            onPointerLeave={() => setIsPressed(false)}
          >
            <Logo
              key={wobbleKey}
              className={cn(
                "w-10 h-10 transition-transform duration-300 ease-out will-change-transform hover:scale-110",
                isPressed && "scale-90",
                !isFalling &&
                  !isPressed &&
                  wobbleKey > 0 &&
                  "animate-[wiggle_0.3s_ease-in-out]",
              )}
            />
          </button>
        ) : (
          <div className="p-1 w-10 h-10" />
        )}
        <CardTitle>Donut</CardTitle>
      </div>
      <div className="flex gap-2 items-center">
        <div className="relative">
          <Input
            type="text"
            placeholder={t("header.searchPlaceholder")}
            value={searchQuery}
            onChange={(e) => onSearchQueryChange(e.target.value)}
            className="pr-8 pl-10 w-48"
          />
          <LuSearch className="absolute left-3 top-1/2 w-4 h-4 transform -translate-y-1/2 text-muted-foreground" />
          {searchQuery && (
            <button
              type="button"
              onClick={() => onSearchQueryChange("")}
              className="absolute right-2 top-1/2 p-1 rounded-sm transition-colors transform -translate-y-1/2 hover:bg-accent"
              aria-label={t("header.clearSearch")}
            >
              <LuX className="w-4 h-4 text-muted-foreground hover:text-foreground" />
            </button>
          )}
        </div>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <span>
              <Tooltip>
                <TooltipTrigger asChild>
                  <span>
                    <Button
                      size="sm"
                      variant="outline"
                      className="flex gap-2 items-center h-[36px]"
                    >
                      <GoKebabHorizontal className="w-4 h-4" />
                    </Button>
                  </span>
                </TooltipTrigger>
                <TooltipContent>{t("header.moreActions")}</TooltipContent>
              </Tooltip>
            </span>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem
              onClick={() => {
                onSettingsDialogOpen(true);
              }}
            >
              <GoGear className="mr-2 w-4 h-4" />
              {t("header.menu.settings")}
            </DropdownMenuItem>
            <DropdownMenuItem
              onClick={() => {
                onProxyManagementDialogOpen(true);
              }}
            >
              <FiWifi className="mr-2 w-4 h-4" />
              {t("header.menu.proxies")}
            </DropdownMenuItem>
            <DropdownMenuItem
              onClick={() => {
                onGroupManagementDialogOpen(true);
              }}
            >
              <LuUsers className="mr-2 w-4 h-4" />
              {t("header.menu.groups")}
            </DropdownMenuItem>
            <DropdownMenuItem
              onClick={() => {
                onExtensionManagementDialogOpen(true);
              }}
            >
              <LuPuzzle className="mr-2 w-4 h-4" />
              {t("header.menu.extensions")}
            </DropdownMenuItem>
            <DropdownMenuItem
              onClick={() => {
                onSyncConfigDialogOpen(true);
              }}
            >
              <LuCloud className="mr-2 w-4 h-4" />
              {t("header.menu.syncService")}
            </DropdownMenuItem>
            <DropdownMenuItem
              onClick={() => {
                onIntegrationsDialogOpen(true);
              }}
            >
              <LuPlug className="mr-2 w-4 h-4" />
              {t("header.menu.integrations")}
            </DropdownMenuItem>
            <DropdownMenuItem
              onClick={() => {
                onImportProfileDialogOpen(true);
              }}
            >
              <FaDownload className="mr-2 w-4 h-4" />
              {t("header.menu.importProfile")}
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
        <Tooltip>
          <TooltipTrigger asChild>
            <span>
              <Button
                size="sm"
                onClick={() => {
                  onCreateProfileDialogOpen(true);
                }}
                className="flex gap-2 items-center h-[36px]"
              >
                <GoPlus className="w-4 h-4" />
              </Button>
            </span>
          </TooltipTrigger>
          <TooltipContent
            arrowOffset={-8}
            style={{ transform: "translateX(-8px)" }}
          >
            {t("header.createProfile")}
          </TooltipContent>
        </Tooltip>
      </div>
    </div>
  );
};

export default HomeHeader;
