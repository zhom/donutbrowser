"use client";

import { motion } from "motion/react";
import {
  Bin,
  bezier,
  Check,
  Cursor,
  Globe,
  Keycap,
  Laptop,
  Padlock,
  Puzzle,
  puzzlePath,
  Scene,
  Server,
  travel,
  useScene,
  Window,
} from "./scene-primitives";

/**
 * Every scene here is decorative: the dialog text carries the meaning and the
 * drawing shows it happening. Scenes loop on their own clock, render their
 * resting frame under reduced motion, and never hide anything a reader needs.
 */

/** Requests leave a profile; the ones bound for ad and tracker hosts stop at the shield. */
export function DnsScene() {
  const { kf, tr } = useScene(3.6);
  return (
    <Scene>
      <Window x={16} y={32} width={92} height={96} />
      <path
        d="M160 42 l22 8 v22 c0 18 -10 30 -22 38 c-12 -8 -22 -20 -22 -38 v-22 z"
        className="text-foreground"
      />
      {[50, 84, 118].map((y) => (
        <g key={y}>
          <path d={`M108 ${y} H236`} strokeDasharray="2 5" />
          <rect x={236} y={y - 12} width={68} height={24} rx={5} />
          <path d={`M248 ${y} h24`} strokeWidth={2} />
        </g>
      ))}
      <motion.circle
        r={3.5}
        cy={50}
        fill="currentColor"
        stroke="none"
        className="text-foreground"
        initial={false}
        animate={{ cx: kf([108, 108, 236, 236]) }}
        transition={tr([0, 0.08, 0.55, 1])}
      />
      {[84, 118].map((y, index) => {
        const hit = 0.34 + index * 0.1;
        return (
          <g key={y} className="text-destructive-text">
            <motion.circle
              cy={y}
              fill="currentColor"
              stroke="none"
              initial={false}
              animate={{
                cx: kf([108, 108, 134, 134, 134]),
                r: kf([3.5, 3.5, 3.5, 0, 0]),
              }}
              transition={tr([0, 0.08, hit, hit + 0.05, 1])}
            />
            <motion.path
              d={`M130 ${y - 4} l8 8 M138 ${y - 4} l-8 8`}
              initial={false}
              animate={{ pathLength: kf([0, 0, 1, 1]) }}
              transition={tr([0, hit, hit + 0.12, 1])}
            />
          </g>
        );
      })}
    </Scene>
  );
}

const ROUTE = travel(
  [
    ...bezier(
      { x: 82, y: 104 },
      { x: 118, y: 104 },
      { x: 128, y: 44 },
      { x: 160, y: 44 },
      8,
    ),
    ...bezier(
      { x: 160, y: 44 },
      { x: 192, y: 44 },
      { x: 204, y: 104 },
      { x: 256, y: 104 },
      8,
    ).slice(1),
  ],
  0.08,
  0.66,
);

/** A check travels this device, the proxy, the exit; the exit is confirmed. */
export function ProxyRouteScene() {
  const { kf, tr } = useScene(3.4);
  return (
    <Scene>
      <Laptop x={24} y={86} />
      <path d="M82 104 C118 104 128 44 160 44 C192 44 204 104 256 104" />
      <motion.circle
        cx={160}
        cy={44}
        r={10}
        className="text-foreground"
        initial={false}
        animate={{ r: kf([10, 10, 13, 10, 10]) }}
        transition={tr([0, 0.3, 0.37, 0.44, 1])}
      />
      <circle
        cx={160}
        cy={44}
        r={2}
        fill="currentColor"
        stroke="none"
        className="text-foreground"
      />
      <Globe cx={272} cy={104} r={16} />
      <motion.circle
        r={3.5}
        fill="currentColor"
        stroke="none"
        className="text-foreground"
        initial={false}
        animate={{ cx: kf(ROUTE.cx), cy: kf(ROUTE.cy) }}
        transition={tr(ROUTE.times, { ease: "linear" })}
      />
      <Check
        x={280}
        y={78}
        className="text-success-text"
        animate={{ pathLength: kf([0, 0, 1, 1]) }}
        transition={tr([0, 0.7, 0.82, 1])}
      />
    </Scene>
  );
}

const COLUMNS = [20, 118, 216];

/** Profiles settle into groups, then the group keys walk the columns. */
export function GroupsScene() {
  const { kf, tr } = useScene(4.4);
  return (
    <Scene>
      {COLUMNS.map((x, index) => {
        const press = 0.5 + index * 0.14;
        return (
          <g key={x}>
            <rect x={x} y={26} width={84} height={104} rx={6} />
            <path d={`M${x + 10} 40 h${28 + index * 10}`} strokeWidth={2} />
            <Keycap
              x={x + 33}
              y={138}
              width={18}
              label={String(index + 1)}
              animate={{ y: kf([0, 0, 2, 0, 0]) }}
              transition={tr([0, press, press + 0.05, press + 0.1, 1])}
            />
          </g>
        );
      })}
      <motion.rect
        x={20}
        y={26}
        width={84}
        height={104}
        rx={6}
        className="text-foreground"
        initial={false}
        animate={{ x: kf([0, 0, 98, 196, 196]) }}
        transition={tr([0, 0.5, 0.64, 0.78, 1])}
      />
      {Array.from({ length: 6 }, (_, index) => {
        const column = Math.floor(index / 2);
        const fromY = 52 + index * 12;
        const toY = 52 + (index % 2) * 12;
        const start = 0.1 + index * 0.05;
        return (
          <motion.rect
            key={index}
            x={32}
            y={toY}
            width={60}
            height={8}
            rx={3}
            strokeWidth={2}
            initial={false}
            animate={{
              x: kf([0, 0, COLUMNS[column] - 20, COLUMNS[column] - 20]),
              y: kf([fromY - toY, fromY - toY, 0, 0]),
            }}
            transition={tr([0, start, start + 0.2, 1])}
          />
        );
      })}
    </Scene>
  );
}

/** The chord opens the palette; two typed letters narrow it to one entry. */
export function PaletteScene({ modLabel }: { modLabel: string }) {
  const { kf, tr } = useScene(4);
  const press = (at: number) => ({
    animate: { y: kf([0, 0, 2, 0, 0]) },
    transition: tr([0, at, at + 0.04, at + 0.1, 1]),
  });
  return (
    <Scene>
      <Keycap x={22} y={118} width={30} label={modLabel} {...press(0.06)} />
      <Keycap x={58} y={118} width={22} label="K" {...press(0.08)} />
      <rect x={110} y={22} width={190} height={116} rx={8} />
      <path d="M110 48 H300" />
      <motion.path
        d="M124 35 h26"
        strokeWidth={2}
        className="text-foreground"
        initial={false}
        animate={{ pathLength: kf([0, 0, 1, 1]) }}
        transition={tr([0, 0.25, 0.4, 1])}
      />
      <motion.path
        d="M154 30 v10"
        className="text-foreground"
        initial={false}
        animate={{ x: kf([-26, -26, 0, 0]) }}
        transition={tr([0, 0.25, 0.4, 1])}
      />
      {[62, 80, 98, 116].map((y, index) => (
        <g key={y}>
          <circle cx={128} cy={y} r={3} />
          <path d={`M140 ${y} h${40 + ((index * 23) % 50)}`} strokeWidth={2} />
        </g>
      ))}
      <motion.rect
        x={116}
        y={54}
        width={178}
        height={16}
        rx={4}
        className="text-foreground"
        initial={false}
        animate={{ y: kf([0, 0, 36, 36]) }}
        transition={tr([0, 0.45, 0.6, 1])}
      />
    </Scene>
  );
}

const CLOCK = { cx: 212, cy: 82, r: 16 };

function hand(deg: number, length: number) {
  return {
    x: CLOCK.cx + Math.sin((deg * Math.PI) / 180) * length,
    y: CLOCK.cy - Math.cos((deg * Math.PI) / 180) * length,
  };
}

/** The profile clock turns to the exit's timezone; the mismatch mark gives way. */
export function ConsistencyScene() {
  const { kf, tr } = useScene(4);
  const wrong = hand(-110, 9);
  const right = hand(60, 9);
  return (
    <Scene>
      <Globe cx={76} cy={82} r={40} />
      <g className="text-foreground">
        <circle cx={98} cy={62} r={3.5} fill="currentColor" stroke="none" />
        <path d="M98 62 v-14" />
      </g>
      <path d="M118 62 C140 46 156 46 176 58" strokeDasharray="2 5" />
      <Window x={176} y={34} width={122} height={94} lines={0}>
        <circle cx={CLOCK.cx} cy={CLOCK.cy} r={CLOCK.r} />
        <path
          d={`M${CLOCK.cx} ${CLOCK.cy} L${CLOCK.cx + 5} ${CLOCK.cy - 11}`}
        />
        <motion.line
          x1={CLOCK.cx}
          y1={CLOCK.cy}
          strokeWidth={2}
          className="text-foreground"
          initial={false}
          animate={{
            x2: kf([wrong.x, wrong.x, right.x, right.x]),
            y2: kf([wrong.y, wrong.y, right.y, right.y]),
          }}
          transition={tr([0, 0.3, 0.55, 1])}
        />
        <path d="M244 118 h40" strokeWidth={2} />
      </Window>
      <motion.g
        className="text-warning-text"
        initial={false}
        animate={{ opacity: kf([1, 1, 0, 0]) }}
        transition={tr([0, 0.4, 0.55, 1])}
      >
        <path d="M268 68 l9 16 h-18 z" />
        <path d="M268 74 v5 M268 82 v0.01" strokeWidth={2} />
      </motion.g>
      <Check
        x={262}
        y={78}
        className="text-success-text"
        animate={{ pathLength: kf([0, 0, 1, 1]) }}
        transition={tr([0, 0.6, 0.75, 1])}
      />
    </Scene>
  );
}

const CIPHER_LINES = [64, 78, 92, 106];

/** The padlock drops and the profile's text turns to cipher. */
export function LockScene() {
  const { kf, tr } = useScene(4.2);
  const swap = tr([0, 0.3, 0.42, 1], { repeatDelay: 0.8 });
  return (
    <Scene>
      <Window x={96} y={30} width={128} height={100} lines={0}>
        <motion.g
          initial={false}
          animate={{ opacity: kf([1, 1, 0.15, 0.15]) }}
          transition={swap}
        >
          {CIPHER_LINES.map((y, index) => (
            <path
              key={y}
              d={`M108 ${y} h${70 - (index % 2) * 24}`}
              strokeWidth={2}
            />
          ))}
        </motion.g>
        <motion.g
          initial={false}
          animate={{ opacity: kf([0.15, 0.15, 1, 1]) }}
          transition={swap}
        >
          {CIPHER_LINES.map((y, index) => (
            <path
              key={y}
              d={`M108 ${y} h${70 - (index % 2) * 24}`}
              strokeWidth={2}
              strokeDasharray="3 3"
            />
          ))}
        </motion.g>
      </Window>
      <Padlock
        x={200}
        y={112}
        className="text-foreground"
        shackle={{
          animate: { y: kf([-5, -5, 0, 0]) },
          transition: tr([0, 0.3, 0.4, 1], { repeatDelay: 0.8 }),
        }}
      />
    </Scene>
  );
}

const CRUMBS = [
  { x: 100, y: 72, at: 0.4 },
  { x: 132, y: 66, at: 0.46 },
  { x: 166, y: 76, at: 0.52 },
];

/** The window closes and its cookies and storage fall away. */
export function SweepScene() {
  const { kf, tr } = useScene(4);
  const fall = (at: number) => tr([0, at, at + 0.22, 1]);
  return (
    <Scene>
      <Window x={70} y={26} width={150} height={108} lines={0}>
        <motion.path
          d="M203 40 l8 8 M211 40 l-8 8"
          className="text-foreground"
          initial={false}
          animate={{ pathLength: kf([0, 0, 1, 1]) }}
          transition={tr([0, 0.28, 0.36, 1])}
        />
      </Window>
      {CRUMBS.map((crumb) => (
        <motion.g
          key={crumb.x}
          initial={false}
          animate={{ y: kf([0, 0, 90, 90]), opacity: kf([1, 1, 0, 0]) }}
          transition={fall(crumb.at)}
        >
          <circle cx={crumb.x} cy={crumb.y} r={6} />
          <circle
            cx={crumb.x - 2}
            cy={crumb.y - 2}
            r={0.8}
            fill="currentColor"
            stroke="none"
          />
          <circle
            cx={crumb.x + 2.5}
            cy={crumb.y + 1.5}
            r={0.8}
            fill="currentColor"
            stroke="none"
          />
        </motion.g>
      ))}
      {[
        { x: 96, at: 0.5 },
        { x: 150, at: 0.56 },
      ].map((store) => (
        <motion.rect
          key={store.x}
          x={store.x}
          y={98}
          width={30}
          height={12}
          rx={3}
          initial={false}
          animate={{ y: kf([0, 0, 70, 70]), opacity: kf([1, 1, 0, 0]) }}
          transition={fall(store.at)}
        />
      ))}
    </Scene>
  );
}

/** A link from another app passes the chooser and opens in the chosen profile. */
export function LinkRouteScene() {
  const { kf, tr } = useScene(4.2);
  const pop = tr([0, 0.55, 0.7, 1]);
  return (
    <Scene>
      <rect x={16} y={56} width={64} height={48} rx={5} />
      <path d="M26 70 h44 M26 82 h30" strokeWidth={2} />
      <path d="M26 94 h24" strokeWidth={2} className="text-foreground" />
      <path d="M80 94 H112" strokeDasharray="2 5" />
      <motion.circle
        r={3}
        cy={94}
        fill="currentColor"
        stroke="none"
        className="text-foreground"
        initial={false}
        animate={{ cx: kf([80, 80, 112, 112]) }}
        transition={tr([0, 0.1, 0.3, 1])}
      />
      <Window x={112} y={30} width={104} height={100} lines={0}>
        {[52, 76, 100].map((y, index) => (
          <g key={y}>
            <circle cx={128} cy={y + 8} r={5} />
            <path d={`M140 ${y + 8} h${28 + index * 12}`} strokeWidth={2} />
          </g>
        ))}
      </Window>
      <motion.rect
        x={118}
        y={52}
        width={92}
        height={16}
        rx={4}
        className="text-foreground"
        initial={false}
        animate={{ y: kf([0, 0, 24, 24]) }}
        transition={tr([0, 0.35, 0.5, 1])}
      />
      <path d="M216 84 H240" strokeDasharray="2 5" />
      <motion.g
        initial={false}
        animate={{ x: kf([12, 12, 0, 0]), y: kf([10, 10, 0, 0]) }}
        transition={pop}
      >
        <motion.rect
          x={240}
          y={44}
          rx={6}
          initial={false}
          animate={{
            width: kf([40, 40, 64, 64]),
            height: kf([48, 48, 80, 80]),
          }}
          transition={pop}
        />
        <motion.path
          d="M240 58 H304"
          initial={false}
          animate={{ pathLength: kf([0.6, 0.6, 1, 1]) }}
          transition={pop}
        />
        <path d="M250 74 h40 M250 86 h28" strokeWidth={2} />
      </motion.g>
    </Scene>
  );
}

/** One extension group; each profile that uses it receives the pieces. */
export function ExtensionsScene() {
  const { kf, tr } = useScene(4);
  return (
    <Scene>
      <rect x={20} y={40} width={88} height={80} rx={8} />
      <Puzzle x={50} y={66} size={28} className="text-foreground" />
      {[28, 66, 104].map((y, index) => {
        const at = 0.25 + index * 0.18;
        return (
          <g key={y}>
            <path
              d={`M108 80 C130 80 130 ${y + 15} 148 ${y + 15}`}
              strokeDasharray="2 5"
            />
            <rect x={148} y={y} width={152} height={30} rx={6} />
            <path
              d={`M162 ${y + 15} h${50 + (index % 2) * 20}`}
              strokeWidth={2}
            />
            <motion.path
              d={puzzlePath(268, y + 8, 14)}
              className="text-foreground"
              initial={false}
              animate={{ pathLength: kf([0, 0, 1, 1]) }}
              transition={tr([0, at, at + 0.15, 1])}
            />
          </g>
        );
      })}
    </Scene>
  );
}

/** A packet is sealed on its way to the server; the server confirms it. */
export function SyncScene() {
  const { kf, tr } = useScene(4.2);
  return (
    <Scene>
      <Laptop x={24} y={62} />
      <path d="M86 80 H244" />
      <Server x={244} y={52} />
      <motion.rect
        x={90}
        y={74}
        width={12}
        height={9}
        rx={2}
        className="text-foreground"
        initial={false}
        animate={{ x: kf([0, 0, 140, 140]) }}
        transition={tr([0, 0.1, 0.6, 1], { ease: "linear" })}
      />
      <Padlock
        x={158}
        y={96}
        className="text-foreground"
        shackle={{
          animate: { y: kf([-5, -5, 0, 0]) },
          transition: tr([0, 0.3, 0.38, 1]),
        }}
      />
      <motion.circle
        cx={276}
        cy={61}
        r={1.5}
        className="text-success-text"
        fill="currentColor"
        stroke="none"
        initial={false}
        animate={{ r: kf([1.5, 1.5, 3, 3]) }}
        transition={tr([0, 0.62, 0.7, 1])}
      />
    </Scene>
  );
}

/** A profile goes to the bin, the retention ring drains, and it comes back. */
export function TrashScene() {
  const { kf, tr } = useScene(5);
  const move = tr([0, 0.12, 0.34, 0.72, 0.94, 1]);
  return (
    <Scene>
      <Bin x={240} y={70} />
      <motion.circle
        cx={251}
        cy={78}
        r={22}
        className="text-foreground"
        initial={false}
        animate={{ pathLength: kf([1, 1, 1, 0.12, 1, 1]) }}
        transition={tr([0, 0.34, 0.4, 0.68, 0.8, 1])}
      />
      <motion.g
        initial={false}
        animate={{
          x: kf([0, 0, 150, 150, 0, 0]),
          opacity: kf([1, 1, 0.3, 0.3, 1, 1]),
        }}
        transition={move}
      >
        <rect x={28} y={66} width={104} height={24} rx={6} />
        <circle cx={44} cy={78} r={6} />
        <path d="M58 74 h48 M58 82 h30" strokeWidth={2} />
      </motion.g>
    </Scene>
  );
}

/** A command in a terminal opens a real profile; `run` also drives it. */
export function ApiScene({ variant }: { variant: "api" | "run" }) {
  const { kf, tr } = useScene(4.4);
  const pop = tr([0, 0.5, 0.64, 1]);
  return (
    <Scene>
      <Window x={16} y={30} width={140} height={100} lines={0}>
        <path d="M30 60 l6 5 l-6 5" className="text-foreground" />
        <motion.path
          d="M44 65 h72"
          strokeWidth={2}
          className="text-foreground"
          initial={false}
          animate={{ pathLength: kf([0, 0, 1, 1]) }}
          transition={tr([0, 0.1, 0.36, 1])}
        />
        <motion.g
          initial={false}
          animate={{ opacity: kf([0.2, 0.2, 1, 1]) }}
          transition={tr([0, 0.42, 0.5, 1])}
        >
          <path d="M30 84 h50 M30 96 h34" strokeWidth={2} />
        </motion.g>
      </Window>
      <path d="M156 80 H196" strokeDasharray="2 5" />
      <motion.circle
        r={3}
        cy={80}
        fill="currentColor"
        stroke="none"
        className="text-foreground"
        initial={false}
        animate={{ cx: kf([156, 156, 196, 196]) }}
        transition={tr([0, 0.38, 0.5, 1])}
      />
      {variant === "run" && (
        <rect
          x={212}
          y={36}
          width={92}
          height={70}
          rx={6}
          strokeDasharray="3 3"
        />
      )}
      <motion.g
        initial={false}
        animate={{ x: kf([8, 8, 0, 0]), y: kf([8, 8, 0, 0]) }}
        transition={pop}
      >
        <motion.rect
          x={196}
          y={44}
          rx={6}
          initial={false}
          animate={{
            width: kf([60, 60, 108, 108]),
            height: kf([44, 44, 76, 76]),
          }}
          transition={pop}
        />
        <motion.path
          d="M196 58 H304"
          initial={false}
          animate={{ pathLength: kf([0.55, 0.55, 1, 1]) }}
          transition={pop}
        />
        <path d="M208 74 h48 M208 88 h30" strokeWidth={2} />
        {variant === "run" && (
          <Cursor
            className="text-foreground"
            animate={{
              x: kf([232, 232, 262, 262]),
              y: kf([92, 92, 76, 76]),
            }}
            transition={tr([0, 0.7, 0.9, 1])}
          />
        )}
      </motion.g>
      <Check
        x={274}
        y={100}
        className="text-success-text"
        animate={{ pathLength: kf([0, 0, 1, 1]) }}
        transition={tr([0, 0.66, 0.78, 1])}
      />
    </Scene>
  );
}

const IMPORT_ARC = bezier(
  { x: 126, y: 84 },
  { x: 160, y: 26 },
  { x: 200, y: 26 },
  { x: 263, y: 84 },
  12,
);

/** Cookies, logins and extensions cross from another browser into a profile. */
export function ImportScene() {
  const { kf, tr } = useScene(4.6);
  return (
    <Scene>
      <Window x={16} y={38} width={110} height={92} lines={4} />
      <path d="M126 84 C160 26 200 26 263 84" strokeDasharray="2 5" />
      <rect x={222} y={38} width={82} height={92} rx={6} />
      <circle cx={263} cy={84} r={18} className="text-foreground" />
      <circle cx={263} cy={84} r={7} className="text-foreground" />
      {[0.1, 0.26, 0.42].map((at) => {
        const trip = travel(IMPORT_ARC, at, at + 0.3);
        return (
          <motion.circle
            key={at}
            r={4}
            fill="currentColor"
            stroke="none"
            className="text-foreground"
            initial={false}
            animate={{ cx: kf(trip.cx), cy: kf(trip.cy) }}
            transition={tr(trip.times, { ease: "linear" })}
          />
        );
      })}
      <Check
        x={286}
        y={50}
        className="text-success-text"
        animate={{ pathLength: kf([0, 0, 1, 1]) }}
        transition={tr([0, 0.76, 0.86, 1])}
      />
    </Scene>
  );
}
