"use client";

import { motion } from "motion/react";
import { FaApple, FaLinux, FaWindows } from "react-icons/fa";
import {
  bezier,
  Check,
  Cloud,
  Cursor,
  Laptop,
  Monitor,
  Padlock,
  Person,
  Scene,
  travel,
  useScene,
  Window,
} from "./scene-primitives";

const UP = travel(
  bezier(
    { x: 82, y: 96 },
    { x: 100, y: 70 },
    { x: 120, y: 56 },
    { x: 140, y: 56 },
    8,
  ),
  0.08,
  0.34,
);
const DOWN = travel(
  bezier(
    { x: 180, y: 56 },
    { x: 200, y: 56 },
    { x: 220, y: 70 },
    { x: 238, y: 96 },
    8,
  ),
  0.44,
  0.7,
);

/** A profile goes up to the cloud from one machine and down to another. */
export function CloudSyncScene() {
  const { kf, tr } = useScene(4.4);
  return (
    <Scene>
      <Laptop x={24} y={78} />
      <Cloud cx={160} cy={52} />
      <Monitor x={238} y={74} />
      <path d="M82 96 C100 70 120 56 140 56" strokeDasharray="2 5" />
      <path d="M180 56 C200 56 220 70 238 96" strokeDasharray="2 5" />
      <motion.circle
        r={3.5}
        fill="currentColor"
        stroke="none"
        className="text-foreground"
        initial={false}
        animate={{ cx: kf(UP.cx), cy: kf(UP.cy) }}
        transition={tr(UP.times, { ease: "linear" })}
      />
      <motion.circle
        r={3.5}
        fill="currentColor"
        stroke="none"
        className="text-foreground"
        initial={false}
        animate={{ cx: kf(DOWN.cx), cy: kf(DOWN.cy) }}
        transition={tr(DOWN.times, { ease: "linear" })}
      />
      <Check
        x={260}
        y={92}
        className="text-success-text"
        animate={{ pathLength: kf([0, 0, 1, 1]) }}
        transition={tr([0, 0.72, 0.84, 1])}
      />
    </Scene>
  );
}

const SKY = travel(
  bezier(
    { x: 40, y: 60 },
    { x: 100, y: -4 },
    { x: 220, y: -4 },
    { x: 280, y: 60 },
    12,
  ),
  0.05,
  0.75,
);

/** The moon crosses the sky while the profile collects cookies and history. */
export function CookieBotScene() {
  const { kf, tr } = useScene(5);
  return (
    <Scene>
      <path d="M24 60 H296" strokeDasharray="2 5" />
      <motion.g
        className="text-foreground"
        initial={false}
        animate={{
          x: kf(SKY.cx.map((x) => x - 40)),
          y: kf(SKY.cy.map((y) => y - 60)),
        }}
        transition={tr(SKY.times, { ease: "linear" })}
      >
        <path d="M40 52 a8 8 0 1 0 6 13 a6 6 0 0 1 -6 -13 z" />
      </motion.g>
      <Window x={96} y={74} width={128} height={62} lines={0}>
        {[0, 1, 2, 3, 4].map((index) => {
          const at = 0.12 + index * 0.13;
          return (
            <motion.circle
              key={index}
              cx={116 + index * 22}
              cy={104}
              initial={false}
              animate={{ r: kf([0, 0, 5, 5]) }}
              transition={tr([0, at, at + 0.08, 1])}
            />
          );
        })}
        <motion.path
          d="M108 122 h104"
          strokeWidth={2}
          className="text-foreground"
          initial={false}
          animate={{ pathLength: kf([0, 0, 1, 1]) }}
          transition={tr([0, 0.08, 0.76, 1], { ease: "linear" })}
        />
      </Window>
    </Scene>
  );
}

const OS_MARKS = [FaApple, FaWindows, FaLinux];
const OS_TIMES = [0, 0.28, 0.34, 0.61, 0.67, 0.94, 1];
const OS_VISIBLE = [
  [1, 1, 0, 0, 0, 0, 1],
  [0, 0, 1, 1, 0, 0, 0],
  [0, 0, 0, 0, 1, 1, 0],
];

/** One profile presents as each operating system in turn. */
export function CrossOsScene() {
  const { kf, tr } = useScene(5.4);
  return (
    <Scene>
      <Window x={40} y={30} width={240} height={100} lines={3} />
      <rect x={214} y={54} width={52} height={52} rx={8} />
      {OS_MARKS.map((Mark, index) => (
        <motion.g
          key={Mark.name}
          className="text-foreground"
          initial={false}
          animate={{ opacity: kf(OS_VISIBLE[index]) }}
          transition={tr(OS_TIMES)}
        >
          <Mark x={226} y={66} width={28} height={28} stroke="none" />
        </motion.g>
      ))}
      <motion.path
        d="M222 114 h12"
        strokeWidth={2}
        className="text-foreground"
        initial={false}
        animate={{ x: kf([0, 0, 12, 12, 24, 24, 0]) }}
        transition={tr(OS_TIMES)}
      />
    </Scene>
  );
}

const AGENT_BUTTONS = [
  { x: 36, y: 56 },
  { x: 36, y: 82 },
  { x: 108, y: 108 },
];
const AGENT_CLICKS = [0.18, 0.42, 0.66];

/** The agent clicks through a page and writes each step into a recipe. */
export function AgentScene() {
  const { kf, tr } = useScene(5.2);
  return (
    <Scene>
      <Window x={16} y={26} width={168} height={108} lines={0}>
        {AGENT_BUTTONS.map((button, index) => {
          const at = AGENT_CLICKS[index];
          return (
            <g key={button.y}>
              <rect x={button.x} y={button.y} width={52} height={14} rx={4} />
              <motion.rect
                x={button.x}
                y={button.y}
                width={52}
                height={14}
                rx={4}
                fill="currentColor"
                stroke="none"
                className="text-foreground"
                initial={false}
                animate={{ opacity: kf([0, 0, 0.9, 0.9]) }}
                transition={tr([0, at, at + 0.05, 1])}
              />
              <Check
                x={button.x + 58}
                y={button.y + 6}
                size={9}
                className="text-success-text"
                animate={{ pathLength: kf([0, 0, 1, 1]) }}
                transition={tr([0, at + 0.04, at + 0.12, 1])}
              />
            </g>
          );
        })}
      </Window>
      <Cursor
        className="text-foreground"
        animate={{
          x: kf([150, 150, 58, 58, 58, 58, 130, 130]),
          y: kf([120, 120, 60, 60, 86, 86, 112, 112]),
        }}
        transition={tr([
          0,
          0.06,
          AGENT_CLICKS[0],
          AGENT_CLICKS[0] + 0.1,
          AGENT_CLICKS[1],
          AGENT_CLICKS[1] + 0.1,
          AGENT_CLICKS[2],
          1,
        ])}
      />
      <Window x={204} y={26} width={100} height={108} lines={0}>
        {AGENT_CLICKS.map((at, index) => (
          <g key={at}>
            <circle
              cx={214}
              cy={58 + index * 22}
              r={2}
              fill="currentColor"
              stroke="none"
            />
            <motion.path
              d={`M222 ${58 + index * 22} h${60 - index * 10}`}
              strokeWidth={2}
              initial={false}
              animate={{ pathLength: kf([0, 0, 1, 1]) }}
              transition={tr([0, at + 0.08, at + 0.2, 1])}
            />
          </g>
        ))}
      </Window>
    </Scene>
  );
}

/** One teammate holds the profile lock, releases it, and the other takes it. */
export function TeamScene() {
  const { kf, tr } = useScene(5);
  return (
    <Scene>
      <Person cx={44} cy={84} />
      <Person cx={276} cy={84} />
      <Window x={112} y={48} width={96} height={64} lines={2} />
      <path d="M58 80 H112 M208 80 H262" />
      <motion.path
        d="M58 80 H112"
        className="text-foreground"
        strokeWidth={2.5}
        initial={false}
        animate={{ pathLength: kf([1, 1, 0, 0, 0]) }}
        transition={tr([0, 0.4, 0.5, 0.52, 1])}
      />
      <motion.path
        d="M262 80 H208"
        className="text-foreground"
        strokeWidth={2.5}
        initial={false}
        animate={{ pathLength: kf([0, 0, 0, 1, 1]) }}
        transition={tr([0, 0.55, 0.6, 0.72, 1])}
      />
      <Padlock
        x={152}
        y={116}
        className="text-foreground"
        shackle={{
          animate: { y: kf([0, 0, -5, -5, 0, 0]) },
          transition: tr([0, 0.42, 0.5, 0.62, 0.7, 1]),
        }}
      />
    </Scene>
  );
}

/** A request from the website crosses the bridge, drives the desktop, and reports back. */
export function RemoteScene() {
  const { kf, tr } = useScene(4.8);
  return (
    <Scene>
      <Cloud cx={64} cy={64} />
      <Monitor x={232} y={62} />
      <path d="M92 82 H232" strokeDasharray="2 5" />
      <motion.rect
        x={96}
        y={78}
        width={12}
        height={8}
        rx={2}
        className="text-foreground"
        initial={false}
        animate={{ x: kf([0, 0, 124, 124, 0, 0]) }}
        transition={tr([0, 0.08, 0.34, 0.56, 0.82, 1], { ease: "linear" })}
      />
      <Cursor
        className="text-foreground"
        animate={{ x: kf([246, 246, 268, 268]), y: kf([88, 88, 74, 74]) }}
        transition={tr([0, 0.36, 0.5, 1])}
      />
      <motion.circle
        cx={274}
        cy={80}
        className="text-foreground"
        initial={false}
        animate={{ r: kf([0, 0, 4, 7, 0, 0]) }}
        transition={tr([0, 0.5, 0.53, 0.58, 0.62, 1])}
      />
    </Scene>
  );
}
