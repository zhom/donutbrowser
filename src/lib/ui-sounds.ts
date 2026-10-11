/**
 * Interface sounds: short, quiet, synthesized at play time. Nothing is
 * recorded or loaded, and a sound only plays while the window is in front.
 *
 * Each sound is a few voices: a sine, triangle or noise burst with a soft
 * attack (no click at the start) and an exponential decay, like a small
 * physical switch. Every play shifts the pitch and level a little, so a run
 * of presses never sounds like a machine repeating one sample.
 */

export type UiSound = "tap" | "toggleOn" | "toggleOff" | "success" | "error";

export interface SoundVoice {
  wave: "sine" | "triangle" | "noise";
  /** Hz at the start. Ignored for noise. */
  frequency: number;
  /** Hz at the end of the decay, for a slight glide. */
  endFrequency?: number;
  /** Seconds after the sound starts. */
  delay: number;
  attack: number;
  decay: number;
  /** Peak level before the master level. */
  peak: number;
  /** A band-pass that shapes a noise burst into a soft tick. */
  bandPass?: { frequency: number; q: number };
}

export const SOUNDS: Record<UiSound, SoundVoice[]> = {
  tap: [
    {
      wave: "sine",
      frequency: 1750,
      delay: 0,
      attack: 0.002,
      decay: 0.03,
      peak: 0.07,
    },
    {
      wave: "noise",
      frequency: 0,
      delay: 0,
      attack: 0.001,
      decay: 0.012,
      peak: 0.035,
      bandPass: { frequency: 3600, q: 1.1 },
    },
  ],
  toggleOn: [
    {
      wave: "sine",
      frequency: 1100,
      delay: 0,
      attack: 0.002,
      decay: 0.035,
      peak: 0.06,
    },
    {
      wave: "sine",
      frequency: 1650,
      delay: 0.045,
      attack: 0.002,
      decay: 0.05,
      peak: 0.06,
    },
  ],
  toggleOff: [
    {
      wave: "sine",
      frequency: 1650,
      delay: 0,
      attack: 0.002,
      decay: 0.035,
      peak: 0.055,
    },
    {
      wave: "sine",
      frequency: 1100,
      delay: 0.045,
      attack: 0.002,
      decay: 0.05,
      peak: 0.055,
    },
  ],
  success: [
    {
      wave: "sine",
      frequency: 880,
      delay: 0,
      attack: 0.004,
      decay: 0.16,
      peak: 0.05,
    },
    {
      wave: "sine",
      frequency: 1318.5,
      delay: 0.07,
      attack: 0.004,
      decay: 0.22,
      peak: 0.045,
    },
  ],
  error: [
    {
      wave: "triangle",
      frequency: 330,
      endFrequency: 247,
      delay: 0,
      attack: 0.004,
      decay: 0.14,
      peak: 0.08,
    },
    {
      wave: "triangle",
      frequency: 262,
      endFrequency: 196,
      delay: 0.09,
      attack: 0.004,
      decay: 0.16,
      peak: 0.06,
    },
  ],
};

/** The shortest time between two plays of one sound, in ms. */
export const SOUND_GAP_MS: Record<UiSound, number> = {
  tap: 35,
  toggleOn: 60,
  toggleOff: 60,
  success: 1200,
  error: 1200,
};

/** What the sound picker needs to know about the control that was used. */
export interface ControlInfo {
  role: string | null;
  tag: string;
  /** `aria-checked` after the change, for switches and checkboxes. */
  checked: string | null;
  disabled: boolean;
  /** `data-sound="off"` on the control or a parent. */
  silenced: boolean;
}

const PRESSABLE_ROLES = new Set([
  "button",
  "menuitem",
  "menuitemcheckbox",
  "menuitemradio",
  "option",
  "tab",
  "link",
]);
const TOGGLE_ROLES = new Set(["switch", "checkbox", "radio"]);

/**
 * `press` is a pointer going down: a button sounds at once, as a real one
 * does. `activate` is the click that follows, or a key press: a toggle sounds
 * then, once it holds its new state, and a button sounds only when a key
 * pressed it (a pointer already made it sound).
 */
export function soundForControl(
  control: ControlInfo,
  phase: "press" | "activate",
  byKeyboard: boolean,
): UiSound | null {
  if (control.disabled || control.silenced) return null;
  const role = control.role ?? implicitRole(control.tag);
  if (role && TOGGLE_ROLES.has(role)) {
    if (phase !== "activate") return null;
    return control.checked === "true" ? "toggleOn" : "toggleOff";
  }
  if (role && PRESSABLE_ROLES.has(role)) {
    if (phase === "press") return "tap";
    return byKeyboard ? "tap" : null;
  }
  return null;
}

function implicitRole(tag: string): string | null {
  switch (tag) {
    case "button":
    case "summary":
      return "button";
    case "a":
      return "link";
    default:
      return null;
  }
}

/** The elements that can make a sound; the closest one to the event wins. */
export const SOUND_CONTROL_SELECTOR = [
  "button",
  "summary",
  "a[href]",
  ...[...PRESSABLE_ROLES, ...TOGGLE_ROLES].map((role) => `[role="${role}"]`),
].join(",");

let enabled = true;
let context: AudioContext | null = null;
let output: GainNode | null = null;
let noise: AudioBuffer | null = null;
const lastPlayed = new Map<UiSound, number>();

export function setUiSoundsEnabled(on: boolean) {
  enabled = on;
}

function audio(): AudioContext | null {
  if (context) return context;
  const Context = window.AudioContext;
  if (!Context) return null;
  context = new Context({ latencyHint: "interactive" });
  // A gentle low-pass takes the edge off every voice; the master level keeps
  // the whole set well under anything else a computer plays.
  const soften = context.createBiquadFilter();
  soften.type = "lowpass";
  soften.frequency.value = 7000;
  output = context.createGain();
  output.gain.value = 0.6;
  output.connect(soften);
  soften.connect(context.destination);
  noise = context.createBuffer(
    1,
    Math.ceil(context.sampleRate * 0.05),
    context.sampleRate,
  );
  const samples = noise.getChannelData(0);
  for (let i = 0; i < samples.length; i++) samples[i] = Math.random() * 2 - 1;
  return context;
}

function schedule(
  ctx: AudioContext,
  voice: SoundVoice,
  at: number,
  pitch: number,
  level: number,
) {
  if (!output) return;
  const start = at + voice.delay;
  const end = start + voice.attack + voice.decay;
  const envelope = ctx.createGain();
  envelope.gain.setValueAtTime(0, start);
  envelope.gain.linearRampToValueAtTime(
    voice.peak * level,
    start + voice.attack,
  );
  envelope.gain.exponentialRampToValueAtTime(0.0001, end);

  let source: AudioScheduledSourceNode;
  if (voice.wave === "noise") {
    const buffer = ctx.createBufferSource();
    buffer.buffer = noise;
    source = buffer;
  } else {
    const oscillator = ctx.createOscillator();
    oscillator.type = voice.wave;
    oscillator.frequency.setValueAtTime(voice.frequency * pitch, start);
    if (voice.endFrequency) {
      oscillator.frequency.exponentialRampToValueAtTime(
        voice.endFrequency * pitch,
        end,
      );
    }
    source = oscillator;
  }
  let last: AudioNode = source;
  if (voice.bandPass) {
    const band = ctx.createBiquadFilter();
    band.type = "bandpass";
    band.frequency.value = voice.bandPass.frequency * pitch;
    band.Q.value = voice.bandPass.q;
    last.connect(band);
    last = band;
  }
  last.connect(envelope);
  envelope.connect(output);
  source.start(start);
  source.stop(end + 0.02);
}

export function playUiSound(sound: UiSound) {
  if (!enabled || typeof window === "undefined") return;
  if (document.visibilityState !== "visible" || !document.hasFocus()) return;
  const now = performance.now();
  if (
    now - (lastPlayed.get(sound) ?? Number.NEGATIVE_INFINITY) <
    SOUND_GAP_MS[sound]
  ) {
    return;
  }
  lastPlayed.set(sound, now);
  try {
    const ctx = audio();
    if (!ctx) return;
    if (ctx.state === "suspended") void ctx.resume();
    const pitch = 1 + (Math.random() - 0.5) * 0.08;
    const level = 1 + (Math.random() - 0.5) * 0.2;
    const at = ctx.currentTime + 0.004;
    for (const voice of SOUNDS[sound]) schedule(ctx, voice, at, pitch, level);
  } catch (error) {
    console.debug("UI sound not played:", error);
  }
}

function controlInfo(element: Element): ControlInfo {
  return {
    role: element.getAttribute("role"),
    tag: element.tagName.toLowerCase(),
    checked: element.getAttribute("aria-checked"),
    disabled:
      element.hasAttribute("disabled") ||
      element.getAttribute("aria-disabled") === "true" ||
      element.getAttribute("data-disabled") === "true",
    silenced: element.closest('[data-sound="off"]') !== null,
  };
}

/** Listen for presses and toggles across the whole window. */
export function installUiSounds(): () => void {
  const control = (target: EventTarget | null) =>
    target instanceof Element ? target.closest(SOUND_CONTROL_SELECTOR) : null;

  const onPointerDown = (event: PointerEvent) => {
    if (event.button !== 0) return;
    const element = control(event.target);
    if (!element) return;
    const sound = soundForControl(controlInfo(element), "press", false);
    if (sound) playUiSound(sound);
  };
  const onClick = (event: MouseEvent) => {
    const element = control(event.target);
    if (!element) return;
    // A click with no pointer behind it (detail 0) came from Enter or Space.
    const byKeyboard = event.detail === 0;
    // Read a toggle's state once the app has drawn the change.
    requestAnimationFrame(() => {
      const sound = soundForControl(
        controlInfo(element),
        "activate",
        byKeyboard,
      );
      if (sound) playUiSound(sound);
    });
  };

  window.addEventListener("pointerdown", onPointerDown, {
    capture: true,
    passive: true,
  });
  window.addEventListener("click", onClick, { capture: true, passive: true });
  return () => {
    window.removeEventListener("pointerdown", onPointerDown, { capture: true });
    window.removeEventListener("click", onClick, { capture: true });
  };
}
