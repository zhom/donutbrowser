// Lightweight module-level flag for whether the first-run onboarding is in
// progress. The welcome dialog shows its own browser-setup progress, so the
// global browser-download toasts (from use-browser-download) are suppressed
// while this is true to avoid a competing toast during onboarding.
let active = false;

export function setOnboardingActive(value: boolean): void {
  active = value;
}

export function isOnboardingActive(): boolean {
  return active;
}

// Dispatched on `window` when the product tour reaches its end and the user
// clicks "Finish" (not when they skip early). The page listens for it to show
// the celebratory thank-you dialog.
export const ONBOARDING_TOUR_FINISHED_EVENT = "donut:onboarding-tour-finished";

// Dispatched when the product tour is either finished or explicitly skipped.
// This is separate from the finished event because both outcomes complete the
// one-shot onboarding, while only a full finish earns the celebration.
export const ONBOARDING_TOUR_CLOSED_EVENT = "donut:onboarding-tour-closed";
