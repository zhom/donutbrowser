import confetti from "canvas-confetti";

/**
 * Donut-sprinkle confetti: small rounded bars tinted with the active theme's
 * chart colors. Used for celebration moments (e.g. a successful profile
 * import). Callers must skip it under prefers-reduced-motion.
 */

// A 12×6 capsule — reads as a donut sprinkle at small scale.
const SPRINKLE_PATH = "M3 0 h6 a3 3 0 0 1 0 6 h-6 a3 3 0 0 1 0 -6 z";

function themeChartColors(): string[] {
  const styles = getComputedStyle(document.documentElement);
  const colors = [1, 2, 3, 4, 5]
    .map((i) => styles.getPropertyValue(`--chart-${i}`).trim())
    .filter(Boolean);
  return colors.length > 0 ? colors : ["#888888"];
}

export function fireSprinkleConfetti(): void {
  const sprinkle = confetti.shapeFromPath({ path: SPRINKLE_PATH });
  const colors = themeChartColors();

  const fire = (particleCount: number, opts: confetti.Options = {}) => {
    void confetti({
      particleCount,
      spread: 75,
      startVelocity: 42,
      scalar: 0.9,
      ticks: 130,
      shapes: [sprinkle],
      colors,
      origin: { y: 0.65 },
      ...opts,
    });
  };

  fire(70);
  window.setTimeout(() => {
    fire(45, { angle: 60, origin: { x: 0.2, y: 0.7 } });
  }, 180);
  window.setTimeout(() => {
    fire(45, { angle: 120, origin: { x: 0.8, y: 0.7 } });
  }, 360);
}
