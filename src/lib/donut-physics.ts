/**
 * Shared physics for the Donut logo easter eggs: clones an element into a
 * fixed-position layer, drops it with gravity, bounces it off the floor and
 * right wall, and lets the user grab it (1:1, respecting the grab offset) and
 * throw it — the sim continues at the pointer's release velocity. Used by the
 * rail logo (5-click trigger) and the About dialog flywheel escape.
 */

const GRAVITY = 2200;
const BOUNCE_DAMPING = 0.6;
const DEFAULT_HORIZONTAL_SPEED = 350;
const DEFAULT_SPIN_SPEED = 720;
const MIN_BOUNCE_VELOCITY = 60;

export interface DonutLaunchOptions {
  /** Initial horizontal velocity in px/s. Defaults to a rightward roll. */
  initialVX?: number;
  /** Initial vertical velocity in px/s (negative = upward). */
  initialVY?: number;
  /** Spin speed in deg/s. */
  spinSpeed?: number;
  /** Called once the clone has left the screen and been removed. */
  onExit?: () => void;
}

/**
 * Launch a physics clone of `el`. The source element is hidden (visibility)
 * and stays hidden — the caller decides when/if to restore it. Returns a
 * cancel function that removes the clone and stops the sim.
 */
export function launchDonutClone(
  el: HTMLElement,
  options: DonutLaunchOptions = {},
): () => void {
  // getBoundingClientRect measures the *transformed* box. A caller that rotates
  // the element before launching (the About dialog spins it up to escape
  // velocity) would otherwise hand us the rotated bounding box: ~37% too large
  // at 45°, with an offset origin — so the clone jumps at launch and bounces off
  // the floor and walls early. Suppress the transform for the measurement to get
  // the layout box, then put it back.
  const previousTransform = el.style.transform;
  if (previousTransform) {
    el.style.transform = "none";
  }
  const rect = el.getBoundingClientRect();
  if (previousTransform) {
    el.style.transform = previousTransform;
  }
  const startX = rect.left;
  const startY = rect.top;

  const clone = el.cloneNode(true) as HTMLElement;
  clone.style.position = "fixed";
  clone.style.left = `${startX}px`;
  clone.style.top = `${startY}px`;
  clone.style.zIndex = "9999";
  clone.style.margin = "0";
  // The fallen donut is a toy: it can be grabbed 1:1 and thrown, inheriting
  // the pointer's release velocity.
  clone.style.pointerEvents = "auto";
  clone.style.cursor = "grab";
  clone.style.touchAction = "none";
  document.body.appendChild(clone);
  el.style.visibility = "hidden";

  let x = 0;
  let y = 0;
  let vy = options.initialVY ?? -500;
  // Roll right first, bounce off the right wall, then escape the left.
  let vx = options.initialVX ?? DEFAULT_HORIZONTAL_SPEED;
  const spinSpeed = options.spinSpeed ?? DEFAULT_SPIN_SPEED;
  let rotation = 0;
  let lastTime = performance.now();
  let grabbed = false;
  let grabDX = 0;
  let grabDY = 0;
  let animFrame = 0;
  let cancelled = false;
  // Recent pointer positions (≤100ms) for release-velocity estimation.
  let history: { t: number; x: number; y: number }[] = [];

  // Progressive resistance past a window edge — follows less the further out.
  const rubberband = (overshoot: number, dimension: number, c = 0.55) =>
    (overshoot * dimension * c) / (dimension + c * Math.abs(overshoot));

  const applyTransform = () => {
    clone.style.transform = `translate(${x}px, ${y}px) rotate(${rotation}deg)`;
  };

  const onPointerDown = (e: PointerEvent) => {
    if (grabbed) return;
    grabbed = true;
    clone.setPointerCapture(e.pointerId);
    clone.style.cursor = "grabbing";
    // Respect where the donut was grabbed — no snap to center.
    grabDX = e.clientX - (startX + x);
    grabDY = e.clientY - (startY + y);
    vx = 0;
    vy = 0;
    history = [{ t: performance.now(), x, y }];
  };

  const onPointerMove = (e: PointerEvent) => {
    if (!grabbed) return;
    let nx = e.clientX - grabDX - startX;
    let ny = e.clientY - grabDY - startY;
    const minX = -startX;
    const maxX = window.innerWidth - rect.width - startX;
    const minY = -startY;
    const maxY = window.innerHeight - rect.height - startY;
    if (nx > maxX) nx = maxX + rubberband(nx - maxX, rect.width);
    if (nx < minX) nx = minX + rubberband(nx - minX, rect.width);
    if (ny > maxY) ny = maxY + rubberband(ny - maxY, rect.height);
    if (ny < minY) ny = minY + rubberband(ny - minY, rect.height);
    x = nx;
    y = ny;
    const now = performance.now();
    history.push({ t: now, x, y });
    while (history.length > 1 && now - history[0].t > 100) history.shift();
    applyTransform();
  };

  const onPointerUp = (e: PointerEvent) => {
    if (!grabbed) return;
    grabbed = false;
    clone.style.cursor = "grab";
    try {
      clone.releasePointerCapture(e.pointerId);
    } catch {
      // capture already gone
    }
    // Velocity handoff: the sim continues at the finger's speed so a flick
    // throws the donut instead of dropping it.
    const now = performance.now();
    const oldest = history[0];
    const dt = oldest ? (now - oldest.t) / 1000 : 0;
    if (oldest && dt > 0.016) {
      vx = (x - oldest.x) / dt;
      vy = (y - oldest.y) / dt;
    }
    history = [];
    lastTime = now;
  };

  clone.addEventListener("pointerdown", onPointerDown);
  clone.addEventListener("pointermove", onPointerMove);
  clone.addEventListener("pointerup", onPointerUp);
  clone.addEventListener("pointercancel", onPointerUp);

  const animate = (time: number) => {
    if (cancelled) return;
    const dt = Math.min((time - lastTime) / 1000, 0.05);
    lastTime = time;

    // Read live so a mid-animation window resize moves the floor/wall.
    const floorY = window.innerHeight;
    const rightWall = window.innerWidth;

    if (!grabbed) {
      vy += GRAVITY * dt;
      x += vx * dt;
      y += vy * dt;
      rotation += spinSpeed * dt * (vx > 0 ? 1 : -1);

      const currentBottom = startY + y + rect.height;
      if (currentBottom >= floorY && vy > 0) {
        y = floorY - startY - rect.height;
        vy =
          Math.abs(vy) > MIN_BOUNCE_VELOCITY
            ? -Math.abs(vy) * BOUNCE_DAMPING
            : -MIN_BOUNCE_VELOCITY * 3;
        // A donut dropped with no sideways speed would hop in place forever —
        // nudge it toward its left-edge exit.
        if (Math.abs(vx) < 40) {
          vx = -DEFAULT_HORIZONTAL_SPEED * 0.5;
        }
      }

      // Right-wall bounce: hit, reverse horizontal velocity (with a tiny
      // damping), and keep rolling. Left wall has no bounce — the donut
      // exits the window off the left edge.
      const currentRight = startX + x + rect.width;
      if (currentRight >= rightWall && vx > 0) {
        x = rightWall - startX - rect.width;
        vx = -Math.abs(vx) * 0.9;
      }

      applyTransform();
    }

    const offScreenLeft = startX + x + rect.width < -200;
    const offScreenBottom = startY + y > floorY + 100;
    const offScreenTop = startY + y + rect.height < -200;

    if (!grabbed && (offScreenLeft || offScreenBottom || offScreenTop)) {
      clone.remove();
      options.onExit?.();
      return;
    }
    animFrame = requestAnimationFrame(animate);
  };
  animFrame = requestAnimationFrame(animate);

  return () => {
    cancelled = true;
    cancelAnimationFrame(animFrame);
    clone.remove();
  };
}
