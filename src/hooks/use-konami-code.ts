"use client";

import { useEffect, useRef } from "react";

/** Up, up, down, down, left, right, left, right, B, A. */
const SEQUENCE = [
  "ArrowUp",
  "ArrowUp",
  "ArrowDown",
  "ArrowDown",
  "ArrowLeft",
  "ArrowRight",
  "ArrowLeft",
  "ArrowRight",
  "b",
  "a",
];

/** A pause longer than this between keys starts the code over. */
const STEP_WINDOW_MS = 2000;

function isTyping(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  const tag = target.tagName;
  return (
    tag === "INPUT" ||
    tag === "TEXTAREA" ||
    tag === "SELECT" ||
    target.isContentEditable
  );
}

/**
 * Calls `onUnlock` once per completed code. Keys typed into a field never
 * count, and a held modifier never counts, so a shortcut that shares a letter
 * cannot advance the sequence by accident.
 */
export function useKonamiCode(onUnlock: () => void) {
  const unlock = useRef(onUnlock);
  unlock.current = onUnlock;
  const progress = useRef(0);
  const lastKeyAt = useRef(0);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.metaKey || event.ctrlKey || event.altKey) return;
      if (isTyping(event.target)) {
        progress.current = 0;
        return;
      }
      const now = Date.now();
      if (now - lastKeyAt.current > STEP_WINDOW_MS) progress.current = 0;
      lastKeyAt.current = now;

      const key = event.key.length === 1 ? event.key.toLowerCase() : event.key;
      if (key === SEQUENCE[progress.current]) {
        progress.current += 1;
      } else {
        // A wrong key may still be the first key of a fresh attempt.
        progress.current = key === SEQUENCE[0] ? 1 : 0;
      }
      if (progress.current === SEQUENCE.length) {
        progress.current = 0;
        unlock.current();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
    };
  }, []);
}
