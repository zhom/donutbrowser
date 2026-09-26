"use client";

import { useReducedMotion } from "motion/react";
import {
  type HTMLAttributes,
  type PointerEvent,
  type ReactNode,
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";
import { LuChevronLeft, LuChevronRight } from "react-icons/lu";
import { cn } from "@/lib/utils";

/** How far a pressed pointer travels before a press becomes a drag. */
const DRAG_THRESHOLD_PX = 4;
/** Room kept between the active item and a faded edge. */
const EDGE_ROOM_PX = 28;
/** Clears a 2px focus outline: a row that scrolls sideways also clips
 * vertically, so a badge's focus ring would be cut at every edge. */
const RING_ROOM_PX = 2;

interface ScrollStripProps extends HTMLAttributes<HTMLDivElement> {
  children: ReactNode;
  /** Changes whenever the item marked `aria-current` changes. */
  activeKey?: string | null;
  scrollLeftLabel: string;
  scrollRightLabel: string;
  /** Classes for the scrolling row itself; `className` styles the frame. */
  stripClassName?: string;
  /** Scroll without animation, e.g. while something is dragged over it. */
  instant?: boolean;
}

/**
 * One row of badges that never wraps. When the badges do not fit, the row
 * fades out at the side that has more, shows a chevron there, and scrolls
 * with the wheel, a drag or the chevrons. The badge marked `aria-current` is
 * kept in view whenever `activeKey` changes.
 */
export function ScrollStrip({
  children,
  activeKey,
  scrollLeftLabel,
  scrollRightLabel,
  className,
  stripClassName,
  instant = false,
  onPointerDown,
  onClickCapture,
  ...rest
}: ScrollStripProps) {
  const ref = useRef<HTMLDivElement | null>(null);
  const reducedMotion = useReducedMotion();
  const [fadeLeft, setFadeLeft] = useState(false);
  const [fadeRight, setFadeRight] = useState(false);
  const [dragging, setDragging] = useState(false);
  const dragRef = useRef<{
    pointerId: number;
    startX: number;
    startLeft: number;
    moved: boolean;
  } | null>(null);
  const suppressClickRef = useRef(false);
  const behavior: ScrollBehavior =
    instant || reducedMotion ? "instant" : "smooth";

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const update = () => {
      setFadeLeft(el.scrollLeft > 1);
      setFadeRight(el.scrollWidth - el.clientWidth - el.scrollLeft > 1);
    };
    update();
    el.addEventListener("scroll", update, { passive: true });
    const resize = new ResizeObserver(update);
    resize.observe(el);
    const mutations = new MutationObserver(update);
    mutations.observe(el, {
      childList: true,
      subtree: true,
      characterData: true,
    });

    // A vertical wheel has nowhere to go in a single row, so turn it
    // sideways. At either end the event is left alone.
    const onWheel = (event: WheelEvent) => {
      if (event.ctrlKey) return;
      if (Math.abs(event.deltaY) <= Math.abs(event.deltaX)) return;
      const room = el.scrollWidth - el.clientWidth;
      if (room <= 1) return;
      const delta = event.deltaMode === 1 ? event.deltaY * 16 : event.deltaY;
      const next = Math.min(room, Math.max(0, el.scrollLeft + delta));
      if (next === el.scrollLeft) return;
      event.preventDefault();
      el.scrollLeft = next;
    };
    el.addEventListener("wheel", onWheel, { passive: false });

    return () => {
      el.removeEventListener("scroll", update);
      el.removeEventListener("wheel", onWheel);
      resize.disconnect();
      mutations.disconnect();
    };
  }, []);

  // biome-ignore lint/correctness/useExhaustiveDependencies: activeKey is the trigger; the active badge is read from the DOM
  useEffect(() => {
    const el = ref.current;
    if (!el || dragRef.current?.moved) return;
    const item = el.querySelector<HTMLElement>(
      '[aria-current]:not([aria-current="false"])',
    );
    if (!item) return;
    const start = item.offsetLeft;
    const end = start + item.offsetWidth;
    let target: number | null = null;
    if (start < el.scrollLeft + EDGE_ROOM_PX) {
      target = start - EDGE_ROOM_PX;
    } else if (end > el.scrollLeft + el.clientWidth - EDGE_ROOM_PX) {
      target = end - el.clientWidth + EDGE_ROOM_PX;
    }
    if (target !== null) {
      el.scrollTo({ left: Math.max(0, target), behavior });
    }
  }, [activeKey]);

  const page = useCallback(
    (direction: -1 | 1) => {
      const el = ref.current;
      if (el) {
        el.scrollBy({ left: direction * el.clientWidth * 0.6, behavior });
      }
    },
    [behavior],
  );

  const endDrag = (event: PointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    if (drag.moved) {
      // The click that ends a drag arrives in this same task. If none comes,
      // the next real click must not be swallowed.
      suppressClickRef.current = true;
      window.setTimeout(() => {
        suppressClickRef.current = false;
      }, 0);
      if (event.currentTarget.hasPointerCapture(event.pointerId)) {
        event.currentTarget.releasePointerCapture(event.pointerId);
      }
    }
    dragRef.current = null;
    setDragging(false);
  };

  const overflowing = fadeLeft || fadeRight;
  const chevron =
    "absolute top-1/2 z-10 grid size-6 -translate-y-1/2 place-items-center rounded-sm text-muted-foreground transition-colors duration-100 hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring";

  return (
    <div className={cn("relative flex min-w-0 items-center", className)}>
      {fadeLeft && (
        <button
          type="button"
          aria-label={scrollLeftLabel}
          onClick={() => page(-1)}
          className={cn(chevron, "left-0")}
        >
          <LuChevronLeft className="size-3.5" />
        </button>
      )}
      <div
        ref={ref}
        data-fade-left={fadeLeft ? "true" : "false"}
        data-fade-right={fadeRight ? "true" : "false"}
        {...rest}
        onPointerDown={(event) => {
          onPointerDown?.(event);
          const el = ref.current;
          if (
            !el ||
            event.pointerType !== "mouse" ||
            event.button !== 0 ||
            el.scrollWidth - el.clientWidth <= 1
          ) {
            return;
          }
          // The strip owns this press. Without this the title bar under it
          // would start moving the window on a long press.
          event.stopPropagation();
          suppressClickRef.current = false;
          dragRef.current = {
            pointerId: event.pointerId,
            startX: event.clientX,
            startLeft: el.scrollLeft,
            moved: false,
          };
        }}
        onPointerMove={(event) => {
          const drag = dragRef.current;
          const el = ref.current;
          if (!drag || !el || drag.pointerId !== event.pointerId) return;
          const dx = event.clientX - drag.startX;
          if (!drag.moved) {
            if (Math.abs(dx) < DRAG_THRESHOLD_PX) return;
            drag.moved = true;
            // Keep receiving moves when the pointer leaves the strip. A
            // pointer the browser no longer tracks cannot be captured, and
            // the drag still works without it while the pointer stays over.
            try {
              el.setPointerCapture(event.pointerId);
            } catch {}
            setDragging(true);
          }
          el.scrollLeft = drag.startLeft - dx;
        }}
        onPointerUp={endDrag}
        onPointerCancel={endDrag}
        onClickCapture={(event) => {
          // The press that ended a drag is not a click on the badge under it.
          if (suppressClickRef.current) {
            suppressClickRef.current = false;
            event.preventDefault();
            event.stopPropagation();
            return;
          }
          onClickCapture?.(event);
        }}
        className={cn(
          "scroll-fade-x relative flex scrollbar-none items-center overflow-x-auto py-0.5 [-ms-overflow-style:none] [&::-webkit-scrollbar]:hidden",
          overflowing && "cursor-grab",
          dragging && "cursor-grabbing select-none",
          stripClassName,
        )}
        style={{
          paddingLeft: fadeLeft ? 22 : RING_ROOM_PX,
          paddingRight: fadeRight ? 22 : RING_ROOM_PX,
        }}
      >
        {children}
      </div>
      {fadeRight && (
        <button
          type="button"
          aria-label={scrollRightLabel}
          onClick={() => page(1)}
          className={cn(chevron, "right-0")}
        >
          <LuChevronRight className="size-3.5" />
        </button>
      )}
    </div>
  );
}
