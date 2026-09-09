"use client";

import {
  animate,
  motion,
  type PanInfo,
  useMotionValue,
  useReducedMotion,
} from "motion/react";
import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { MOTION_SPRING_POSITION } from "@/lib/motion";
import { cn } from "@/lib/utils";
import type { BrowserProfile } from "@/types";

export const UNGROUPED_DROP_ID = "__ungrouped__";

interface DragSource {
  profiles: BrowserProfile[];
  element: HTMLElement;
  rect: DOMRect;
  canMove: (profiles: BrowserProfile[]) => boolean;
}

interface DragSession extends DragSource {
  token: number;
  phase: "dragging" | "saving" | "settling";
  travelComplete?: boolean;
  decorationHidden?: boolean;
}

interface DragContextValue {
  active: boolean;
  saving: boolean;
  targetId: string | null;
  movingIds: Set<string>;
  canDrop: (groupId: string) => boolean;
  start: (source: DragSource, info: PanInfo) => void;
  move: (info: PanInfo) => void;
  end: (info: PanInfo) => void;
  cancelSource: (element: HTMLElement | null) => void;
}

const DragContext = createContext<DragContextValue | null>(null);

export function useProfileGroupDrag() {
  return useContext(DragContext);
}

export function ProfileGroupDragProvider({
  enabled,
  onAssign,
  children,
}: {
  enabled: boolean;
  onAssign: (profileIds: string[], groupId: string | null) => Promise<boolean>;
  children: ReactNode;
}) {
  const { t } = useTranslation();
  const reducedMotion = useReducedMotion();
  const x = useMotionValue(0);
  const y = useMotionValue(0);
  const [session, setSession] = useState<DragSession | null>(null);
  const [targetId, setTargetId] = useState<string | null>(null);
  const [announcement, setAnnouncement] = useState("");
  const activeRef = useRef<DragSession | null>(null);
  const tokenRef = useRef(0);
  const mountedRef = useRef(true);
  const frameRef = useRef<number | null>(null);
  const animationEpochRef = useRef(0);
  const animationTimeoutRef = useRef<number | null>(null);
  const pointerRef = useRef({ x: 0, y: 0 });

  const stopScrolling = useCallback(() => {
    if (frameRef.current !== null) cancelAnimationFrame(frameRef.current);
    frameRef.current = null;
  }, []);

  const stopTravel = useCallback(() => {
    animationEpochRef.current += 1;
    if (animationTimeoutRef.current !== null) {
      window.clearTimeout(animationTimeoutRef.current);
      animationTimeoutRef.current = null;
    }
    x.stop();
    y.stop();
  }, [x, y]);

  const finish = useCallback(
    (token: number) => {
      if (!mountedRef.current || activeRef.current?.token !== token) return;
      stopTravel();
      activeRef.current = null;
      setSession(null);
      setTargetId(null);
    },
    [stopTravel],
  );

  const travel = useCallback(
    (
      current: DragSession,
      target: { x: number; y: number },
      velocity: PanInfo["velocity"],
      onComplete: () => void,
    ) => {
      stopTravel();
      const epoch = animationEpochRef.current;
      const complete = () => {
        if (
          !mountedRef.current ||
          activeRef.current?.token !== current.token ||
          animationEpochRef.current !== epoch
        )
          return;
        stopTravel();
        x.set(target.x);
        y.set(target.y);
        onComplete();
      };
      if (reducedMotion || document.hidden || current.decorationHidden) {
        complete();
        return;
      }
      const horizontal = animate(x, target.x, {
        ...MOTION_SPRING_POSITION,
        velocity: velocity.x,
      });
      const vertical = animate(y, target.y, {
        ...MOTION_SPRING_POSITION,
        velocity: velocity.y,
      });
      // The gesture's decoration must finish even if the animation clock stalls.
      // This never resolves or cancels the backend assignment itself.
      animationTimeoutRef.current = window.setTimeout(complete, 800);
      void Promise.all([horizontal, vertical]).then(complete);
    },
    [reducedMotion, stopTravel, x, y],
  );

  const settle = useCallback(
    (
      current: DragSession,
      target: { x: number; y: number },
      velocity: PanInfo["velocity"],
    ) => {
      if (activeRef.current?.token !== current.token) return;
      const next = { ...current, phase: "settling" as const };
      activeRef.current = next;
      setSession(next);
      travel(next, target, velocity, () => finish(current.token));
    },
    [finish, travel],
  );

  const cancel = useCallback(() => {
    const current = activeRef.current;
    if (current?.phase !== "dragging") return;
    stopScrolling();
    setTargetId(null);
    setAnnouncement(t("profileMotion.dragCancelled"));
    settle(
      current,
      { x: current.rect.left, y: current.rect.top },
      { x: x.getVelocity(), y: y.getVelocity() },
    );
  }, [settle, stopScrolling, t, x, y]);
  const cancelRef = useRef(cancel);
  cancelRef.current = cancel;

  const canDrop = useCallback((groupId: string) => {
    const current = activeRef.current;
    if (current?.phase !== "dragging" || !current.canMove(current.profiles))
      return false;
    const target = groupId === UNGROUPED_DROP_ID ? null : groupId;
    return current.profiles.some(
      (profile) => (profile.group_id ?? null) !== target,
    );
  }, []);

  const targetAtPointer = useCallback(() => {
    const element = document
      .elementFromPoint(pointerRef.current.x, pointerRef.current.y)
      ?.closest<HTMLElement>("[data-profile-group-drop]");
    const id = element?.dataset.profileGroupDrop;
    return element && id && canDrop(id) ? { element, id } : null;
  }, [canDrop]);

  const trackTarget = useCallback(() => {
    const target = targetAtPointer();
    setTargetId(target?.id ?? null);
  }, [targetAtPointer]);

  const scrollGroups = useCallback(() => {
    frameRef.current = null;
    if (activeRef.current?.phase !== "dragging") return;
    const strip = document.querySelector<HTMLElement>(
      "[data-slot='profile-group-strip']",
    );
    if (!strip) return;
    const rect = strip.getBoundingClientRect();
    const point = pointerRef.current;
    if (point.y < rect.top - 12 || point.y > rect.bottom + 12) return;
    const edge = Math.min(40, rect.width / 4);
    const step =
      point.x < rect.left + edge && point.x >= rect.left - 16
        ? -Math.min(12, (rect.left + edge - point.x) / 3)
        : point.x > rect.right - edge && point.x <= rect.right + 16
          ? Math.min(12, (point.x - rect.right + edge) / 3)
          : 0;
    if (!step) return;
    const before = strip.scrollLeft;
    strip.scrollBy({ left: step, behavior: "instant" });
    trackTarget();
    if (strip.scrollLeft !== before)
      frameRef.current = requestAnimationFrame(scrollGroups);
  }, [trackTarget]);

  const move = useCallback(
    (info: PanInfo) => {
      const current = activeRef.current;
      if (current?.phase !== "dragging") return;
      if (!current.canMove(current.profiles)) {
        cancel();
        return;
      }
      x.set(current.rect.left + info.offset.x);
      y.set(current.rect.top + info.offset.y);
      pointerRef.current = {
        x: info.point.x - window.scrollX,
        y: info.point.y - window.scrollY,
      };
      trackTarget();
      if (frameRef.current === null)
        frameRef.current = requestAnimationFrame(scrollGroups);
    },
    [cancel, scrollGroups, trackTarget, x, y],
  );

  const start = useCallback(
    (source: DragSource, info: PanInfo) => {
      if (
        !enabled ||
        activeRef.current?.phase === "saving" ||
        !source.canMove(source.profiles)
      )
        return;
      stopScrolling();
      stopTravel();
      const next: DragSession = {
        ...source,
        token: ++tokenRef.current,
        phase: "dragging",
      };
      activeRef.current = next;
      setSession(next);
      setTargetId(null);
      setAnnouncement(
        t("profileMotion.dragPickedUp", { count: source.profiles.length }),
      );
      move(info);
    },
    [enabled, move, stopScrolling, stopTravel, t],
  );

  const end = useCallback(
    (info: PanInfo) => {
      const current = activeRef.current;
      if (current?.phase !== "dragging") return;
      pointerRef.current = {
        x: info.point.x - window.scrollX,
        y: info.point.y - window.scrollY,
      };
      stopScrolling();
      const target = targetAtPointer();
      if (!target) {
        setTargetId(null);
        setAnnouncement(t("profileMotion.dragCancelled"));
        settle(
          current,
          { x: current.rect.left, y: current.rect.top },
          info.velocity,
        );
        return;
      }
      const rect = target.element.getBoundingClientRect();
      const next = {
        ...current,
        phase: "saving" as const,
        travelComplete: false,
      };
      activeRef.current = next;
      setSession(next);
      setAnnouncement(t("profileMotion.dragSaving"));
      travel(next, { x: rect.left, y: rect.bottom + 4 }, info.velocity, () => {
        const active = activeRef.current;
        if (!active || active.token !== current.token) return;
        if (active.phase === "settling") {
          finish(current.token);
        } else {
          const stopped = { ...active, travelComplete: true };
          activeRef.current = stopped;
          setSession(stopped);
        }
      });
      const groupId = target.id === UNGROUPED_DROP_ID ? null : target.id;
      const changedProfiles = current.profiles.filter(
        (profile) => (profile.group_id ?? null) !== groupId,
      );
      const resolveAssignment = (success: boolean) => {
        const active = activeRef.current;
        if (!mountedRef.current || !active || active.token !== current.token)
          return;
        setTargetId(null);
        setAnnouncement(success ? "" : t("profileMotion.dragCancelled"));
        if (active.decorationHidden) {
          finish(current.token);
        } else if (success) {
          if (active.travelComplete) {
            finish(current.token);
          } else {
            const settling = { ...active, phase: "settling" as const };
            activeRef.current = settling;
            setSession(settling);
          }
        } else {
          settle(
            active,
            { x: current.rect.left, y: current.rect.top },
            { x: x.getVelocity(), y: y.getVelocity() },
          );
        }
      };
      void onAssign(
        changedProfiles.map((profile) => profile.id),
        groupId,
      )
        .then(resolveAssignment)
        .catch(() => resolveAssignment(false));
    },
    [finish, onAssign, settle, stopScrolling, t, targetAtPointer, travel, x, y],
  );

  const cancelSource = useCallback((element: HTMLElement | null) => {
    if (activeRef.current?.element === element) cancelRef.current();
  }, []);

  const dismissDecoration = useCallback(() => {
    const current = activeRef.current;
    if (!current) return;
    stopScrolling();
    stopTravel();
    setTargetId(null);
    if (current.phase === "saving") {
      const hidden = {
        ...current,
        decorationHidden: true,
        travelComplete: true,
      };
      activeRef.current = hidden;
      setSession(hidden);
    } else {
      if (current.phase === "dragging")
        setAnnouncement(t("profileMotion.dragCancelled"));
      finish(current.token);
    }
  }, [finish, stopScrolling, stopTravel, t]);
  const dismissDecorationRef = useRef(dismissDecoration);
  dismissDecorationRef.current = dismissDecoration;

  useEffect(() => {
    if (!enabled) dismissDecorationRef.current();
  }, [enabled]);

  useEffect(() => {
    mountedRef.current = true;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || activeRef.current?.phase !== "dragging")
        return;
      event.preventDefault();
      event.stopImmediatePropagation();
      cancelRef.current();
    };
    const onCancel = () => cancelRef.current();
    const onBlur = () => dismissDecorationRef.current();
    const onVisibility = () => {
      if (document.hidden) dismissDecorationRef.current();
    };
    window.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("blur", onBlur);
    window.addEventListener("pointercancel", onCancel);
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      mountedRef.current = false;
      activeRef.current = null;
      stopScrolling();
      stopTravel();
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("blur", onBlur);
      window.removeEventListener("pointercancel", onCancel);
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [stopScrolling, stopTravel]);

  return (
    <DragContext.Provider
      value={{
        active: session?.phase === "dragging",
        saving: session?.phase === "saving",
        targetId,
        movingIds: new Set(session?.profiles.map((profile) => profile.id)),
        canDrop,
        start,
        move,
        end,
        cancelSource,
      }}
    >
      {children}
      <span
        className="sr-only"
        role="status"
        aria-live="polite"
        aria-atomic="true"
      >
        {announcement}
      </span>
      {session &&
        !session.decorationHidden &&
        createPortal(
          <motion.div
            data-slot="profile-drag-preview"
            data-phase={session.phase}
            aria-hidden="true"
            className="pointer-events-none fixed top-0 left-0 z-[100] flex min-h-9 max-w-64 items-center gap-2 rounded-md bg-accent px-3 py-2 text-xs font-medium text-accent-foreground shadow-sm"
            style={{ x, y }}
          >
            <ProfileGrip />
            <span className="min-w-0 break-words">
              {session.phase === "saving"
                ? t("profileMotion.dragSaving")
                : session.profiles.length === 1
                  ? session.profiles[0].name
                  : t("profileMotion.dragSelection", {
                      count: session.profiles.length,
                    })}
            </span>
          </motion.div>,
          document.body,
        )}
    </DragContext.Provider>
  );
}

function ProfileGrip() {
  return (
    <svg
      aria-hidden="true"
      viewBox="0 0 16 16"
      className="size-3.5 shrink-0 fill-current"
    >
      <path d="M4 3h2v4H4zm0 6h2v4H4zm6-6h2v4h-2zm0 6h2v4h-2z" />
    </svg>
  );
}

export function ProfileGroupDragHandle({
  profile,
  profiles,
  disabled,
  canMove,
  onChooseGroup,
}: {
  profile: BrowserProfile;
  profiles: BrowserProfile[];
  disabled: boolean;
  canMove: (profiles: BrowserProfile[]) => boolean;
  onChooseGroup: () => void;
}) {
  const { t } = useTranslation();
  const drag = useProfileGroupDrag();
  const elementRef = useRef<HTMLButtonElement | null>(null);
  const sourceRef = useRef<DragSource | null>(null);
  const pointerStartRef = useRef<{ x: number; y: number; id: number } | null>(
    null,
  );
  const didPanRef = useRef(false);
  const canMoveRef = useRef(canMove);
  canMoveRef.current = canMove;
  const cancelSource = drag?.cancelSource;
  useEffect(() => {
    const element = elementRef.current;
    return () => cancelSource?.(element);
  }, [cancelSource]);
  if (!drag) return null;
  return (
    <motion.button
      ref={elementRef}
      type="button"
      data-slot="profile-drag-handle"
      data-profile-id={profile.id}
      aria-label={t("profileMotion.dragHandle", { name: profile.name })}
      title={
        drag.saving
          ? t("profileMotion.dragSaving")
          : disabled
            ? t("profileMotion.dragUnavailable")
            : t("profileMotion.dragHint")
      }
      disabled={disabled || drag.saving}
      aria-busy={drag.saving && drag.movingIds.has(profile.id)}
      className={cn(
        "grid size-6 shrink-0 touch-none place-items-center rounded-sm text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring disabled:cursor-not-allowed disabled:opacity-40",
        !disabled && "cursor-grab active:cursor-grabbing",
      )}
      onPointerDown={(event) => {
        if (event.button !== 0) return;
        event.stopPropagation();
        event.currentTarget.setPointerCapture(event.pointerId);
        didPanRef.current = false;
        pointerStartRef.current = {
          x: event.clientX,
          y: event.clientY,
          id: event.pointerId,
        };
        sourceRef.current = {
          profiles,
          element: event.currentTarget,
          rect: event.currentTarget.getBoundingClientRect(),
          canMove: (captured) => canMoveRef.current(captured),
        };
      }}
      onPointerMove={(event) => {
        const point = pointerStartRef.current;
        if (event.buttons !== 1 || !point || point.id !== event.pointerId)
          return;
        if (Math.hypot(event.clientX - point.x, event.clientY - point.y) > 3)
          didPanRef.current = true;
      }}
      onPanStart={(_, info) => {
        didPanRef.current = true;
        if (sourceRef.current) drag.start(sourceRef.current, info);
      }}
      onPan={(_, info) => drag.move(info)}
      onPanEnd={(_, info) => drag.end(info)}
      onClick={(event) => {
        event.stopPropagation();
        if (!didPanRef.current || event.detail === 0) onChooseGroup();
        didPanRef.current = false;
      }}
    >
      <ProfileGrip />
    </motion.button>
  );
}
