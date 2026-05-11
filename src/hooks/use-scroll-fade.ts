import { type RefObject, useEffect } from "react";

/**
 * Track scroll position on a vertical scroll container and write the result
 * to `data-fade-top` / `data-fade-bottom` attributes on the element. The
 * `.scroll-fade` CSS utility in `globals.css` reads these attributes and
 * shows fade gradients only in directions that are actually scrollable.
 *
 * A ResizeObserver watches the container AND its direct children so internal
 * content height changes (e.g. virtualizer padding rows growing/shrinking
 * as the user scrolls) recompute the fade state automatically.
 */
export function useScrollFade<T extends HTMLElement>(
  ref: RefObject<T | null>,
): void {
  useEffect(() => {
    const el = ref.current;
    if (!el) return;

    const update = () => {
      const fadeTop = el.scrollTop > 1;
      const fadeBottom = el.scrollHeight - el.clientHeight - el.scrollTop > 1;
      el.setAttribute("data-fade-top", fadeTop ? "true" : "false");
      el.setAttribute("data-fade-bottom", fadeBottom ? "true" : "false");
    };

    update();
    el.addEventListener("scroll", update, { passive: true });

    const ro = new ResizeObserver(update);
    ro.observe(el);
    for (const child of Array.from(el.children)) {
      ro.observe(child);
    }

    // MutationObserver picks up DOM additions (virtualizer mounts new rows)
    // and re-attaches the ResizeObserver to the new children. Without this,
    // newly inserted rows wouldn't trigger a fade recompute.
    const mo = new MutationObserver(() => {
      ro.disconnect();
      ro.observe(el);
      for (const child of Array.from(el.children)) {
        ro.observe(child);
      }
      update();
    });
    mo.observe(el, { childList: true, subtree: true });

    return () => {
      el.removeEventListener("scroll", update);
      ro.disconnect();
      mo.disconnect();
    };
  }, [ref]);
}
