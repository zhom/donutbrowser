export interface ThemeColors extends Record<string, string> {
  "--background": string;
  "--foreground": string;
  "--card": string;
  "--card-foreground": string;
  "--popover": string;
  "--popover-foreground": string;
  "--primary": string;
  "--primary-foreground": string;
  "--secondary": string;
  "--secondary-foreground": string;
  "--muted": string;
  "--muted-foreground": string;
  "--accent": string;
  "--accent-foreground": string;
  "--destructive": string;
  "--destructive-foreground": string;
  "--success": string;
  "--success-foreground": string;
  "--warning": string;
  "--warning-foreground": string;
  "--border": string;
  "--chart-1": string;
  "--chart-2": string;
  "--chart-3": string;
  "--chart-4": string;
  "--chart-5": string;
}

export interface Theme {
  id: string;
  name: string;
  mode: "light" | "dark";
  colors: ThemeColors;
}

export const THEMES: Theme[] = [
  {
    id: "donut-mono",
    name: "Donut Mono",
    mode: "dark",
    colors: {
      "--background": "#070707",
      "--foreground": "#ffffff",
      "--card": "#0e0e0e",
      "--card-foreground": "#e4e4e4",
      "--popover": "#0e0e0e",
      "--popover-foreground": "#e4e4e4",
      "--primary": "#ffffff",
      "--primary-foreground": "#070707",
      "--secondary": "#161616",
      "--secondary-foreground": "#e4e4e4",
      "--muted": "#161616",
      "--muted-foreground": "#a0a0a0",
      "--accent": "#1f1f1f",
      "--accent-foreground": "#ffffff",
      "--destructive": "#ec6a5e",
      "--destructive-foreground": "#070707",
      "--success": "#61c554",
      "--success-foreground": "#070707",
      "--warning": "#f4be4f",
      "--warning-foreground": "#070707",
      "--border": "rgba(255,255,255,0.06)",
      "--chart-1": "#a0a0a0",
      "--chart-2": "#6b6b6b",
      "--chart-3": "#444444",
      "--chart-4": "#e4e4e4",
      "--chart-5": "#ffffff",
    },
  },
  {
    id: "tokyo-night",
    name: "Tokyo Night",
    mode: "dark",
    colors: {
      "--background": "#1a1b26",
      "--foreground": "#c0caf5",
      "--card": "#24283b",
      "--card-foreground": "#c0caf5",
      "--popover": "#24283b",
      "--popover-foreground": "#c0caf5",
      "--primary": "#7aa2f7",
      "--primary-foreground": "#1a1b26",
      "--secondary": "#2ac3de",
      "--secondary-foreground": "#1a1b26",
      "--muted": "#3b4261",
      "--muted-foreground": "#a9b1d6",
      "--accent": "#bb9af7",
      "--accent-foreground": "#1a1b26",
      "--destructive": "#f7768e",
      "--destructive-foreground": "#1a1b26",
      "--success": "#9ece6a",
      "--success-foreground": "#1a1b26",
      "--warning": "#e0af68",
      "--warning-foreground": "#1a1b26",
      "--border": "#3b4261",
      "--chart-1": "#7aa2f7",
      "--chart-2": "#9ece6a",
      "--chart-3": "#bb9af7",
      "--chart-4": "#2ac3de",
      "--chart-5": "#ff9e64",
    },
  },
  {
    id: "dracula",
    name: "Dracula",
    mode: "dark",
    colors: {
      "--background": "#282a36",
      "--foreground": "#fcfcfa",
      "--card": "#44475a",
      "--card-foreground": "#fcfcfa",
      "--popover": "#44475a",
      "--popover-foreground": "#fcfcfa",
      "--primary": "#bd93f9",
      "--primary-foreground": "#282a36",
      "--secondary": "#8be9fd",
      "--secondary-foreground": "#282a36",
      "--muted": "#6272a4",
      "--muted-foreground": "#fcfcfa",
      "--accent": "#ff79c6",
      "--accent-foreground": "#282a36",
      "--destructive": "#ff5555",
      "--destructive-foreground": "#282a36",
      "--success": "#50fa7b",
      "--success-foreground": "#282a36",
      "--warning": "#ffb86c",
      "--warning-foreground": "#282a36",
      "--border": "#6272a4",
      "--chart-1": "#bd93f9",
      "--chart-2": "#50fa7b",
      "--chart-3": "#ff79c6",
      "--chart-4": "#8be9fd",
      "--chart-5": "#ffb86c",
    },
  },
  {
    id: "matchalk",
    name: "Matchalk",
    mode: "dark",
    colors: {
      "--background": "#273136",
      "--foreground": "#D1DED3",
      "--card": "#1c2427",
      "--card-foreground": "#D1DED3",
      "--popover": "#323e45",
      "--popover-foreground": "#D1DED3",
      "--primary": "#7eb08a",
      "--primary-foreground": "#273136",
      "--secondary": "#d2b48c",
      "--secondary-foreground": "#273136",
      "--muted": "#323e45",
      "--muted-foreground": "#8badb8",
      "--accent": "#d2b48c",
      "--accent-foreground": "#273136",
      "--destructive": "#ff819f",
      "--destructive-foreground": "#273136",
      "--success": "#a8c97f",
      "--success-foreground": "#273136",
      "--warning": "#e6c07b",
      "--warning-foreground": "#273136",
      "--border": "#304e37",
      "--chart-1": "#7eb08a",
      "--chart-2": "#d2b48c",
      "--chart-3": "#7ea4b0",
      "--chart-4": "#a8c97f",
      "--chart-5": "#e6c07b",
    },
  },
  {
    id: "houston",
    name: "Houston",
    mode: "dark",
    colors: {
      "--background": "#17191e",
      "--foreground": "#f7f7f8",
      "--card": "#21252e",
      "--card-foreground": "#f7f7f8",
      "--popover": "#21252e",
      "--popover-foreground": "#f7f7f8",
      "--primary": "#5755d9",
      "--primary-foreground": "#f7f7f8",
      "--secondary": "#f25f4c",
      "--secondary-foreground": "#17191e",
      "--muted": "#2a2e39",
      "--muted-foreground": "#9ca3af",
      "--accent": "#0ea5e9",
      "--accent-foreground": "#17191e",
      "--destructive": "#ef4444",
      "--destructive-foreground": "#17191e",
      "--success": "#22c55e",
      "--success-foreground": "#17191e",
      "--warning": "#f59e0b",
      "--warning-foreground": "#17191e",
      "--border": "#2a2e39",
      "--chart-1": "#5755d9",
      "--chart-2": "#0ea5e9",
      "--chart-3": "#f25f4c",
      "--chart-4": "#22c55e",
      "--chart-5": "#f59e0b",
    },
  },
  {
    id: "ayu-dark",
    name: "Ayu Dark",
    mode: "dark",
    colors: {
      "--background": "#0a0e14",
      "--foreground": "#b3b1ad",
      "--card": "#11151c",
      "--card-foreground": "#b3b1ad",
      "--popover": "#11151c",
      "--popover-foreground": "#b3b1ad",
      "--primary": "#39bae6",
      "--primary-foreground": "#0a0e14",
      "--secondary": "#ffb454",
      "--secondary-foreground": "#0a0e14",
      "--muted": "#1f2430",
      "--muted-foreground": "#858d96",
      "--accent": "#d2a6ff",
      "--accent-foreground": "#0a0e14",
      "--destructive": "#f07178",
      "--destructive-foreground": "#0a0e14",
      "--success": "#c2d94c",
      "--success-foreground": "#0a0e14",
      "--warning": "#ffb454",
      "--warning-foreground": "#0a0e14",
      "--border": "#1f2430",
      "--chart-1": "#39bae6",
      "--chart-2": "#c2d94c",
      "--chart-3": "#d2a6ff",
      "--chart-4": "#ffb454",
      "--chart-5": "#f07178",
    },
  },
  {
    id: "ayu-light",
    name: "Ayu Light",
    mode: "light",
    // Source: ayu-theme/ayu-colors light.yaml. Primary uses the iconic
    // Ayu orange instead of blue — that's the colour the theme is known for.
    colors: {
      "--background": "#f8f9fa",
      "--foreground": "#5c6166",
      "--card": "#fcfcfc",
      "--card-foreground": "#5c6166",
      "--popover": "#ffffff",
      "--popover-foreground": "#5c6166",
      "--primary": "#f29718",
      "--primary-foreground": "#0a0e14",
      "--secondary": "#399ee6",
      "--secondary-foreground": "#0a0e14",
      "--muted": "#ebeef0",
      "--muted-foreground": "#626b78",
      "--accent": "#a37acc",
      "--accent-foreground": "#0a0e14",
      "--destructive": "#e65050",
      "--destructive-foreground": "#0a0e14",
      "--success": "#86b300",
      "--success-foreground": "#0a0e14",
      "--warning": "#fa8532",
      "--warning-foreground": "#0a0e14",
      "--border": "#c8d0d6",
      "--chart-1": "#f29718",
      "--chart-2": "#86b300",
      "--chart-3": "#a37acc",
      "--chart-4": "#399ee6",
      "--chart-5": "#4cbf99",
    },
  },
  {
    id: "catppuccin-latte",
    name: "Catppuccin Latte",
    mode: "light",
    // Source: github.com/catppuccin/palette/blob/main/palette.json
    // Primary uses mauve (purple) — the colour Catppuccin is most known
    // for — instead of blue, to differentiate from the many blue themes.
    // Frappé and Macchiato variants intentionally omitted; they're tonal
    // mid-points between Latte and Mocha and added little variety.
    colors: {
      "--background": "#eff1f5",
      "--foreground": "#484b64",
      "--card": "#ccd0da",
      "--card-foreground": "#484b64",
      "--popover": "#ccd0da",
      "--popover-foreground": "#484b64",
      "--primary": "#8839ef",
      "--primary-foreground": "#eff1f5",
      "--secondary": "#1e66f5",
      "--secondary-foreground": "#ffffff",
      "--muted": "#bcc0cc",
      "--muted-foreground": "#4b4d5c",
      "--accent": "#ea76cb",
      "--accent-foreground": "#353637",
      "--destructive": "#d20f39",
      "--destructive-foreground": "#eff1f5",
      "--success": "#40a02b",
      "--success-foreground": "#242525",
      "--warning": "#df8e1d",
      "--warning-foreground": "#363637",
      "--border": "#9ca0b0",
      "--chart-1": "#8839ef",
      "--chart-2": "#40a02b",
      "--chart-3": "#ea76cb",
      "--chart-4": "#04a5e5",
      "--chart-5": "#fe640b",
    },
  },
  {
    id: "catppuccin-mocha",
    name: "Catppuccin Mocha",
    mode: "dark",
    // Source: github.com/catppuccin/palette/blob/main/palette.json
    // Primary uses mauve (purple) — Catppuccin's signature colour.
    colors: {
      "--background": "#1e1e2e",
      "--foreground": "#cdd6f4",
      "--card": "#313244",
      "--card-foreground": "#cdd6f4",
      "--popover": "#313244",
      "--popover-foreground": "#cdd6f4",
      "--primary": "#cba6f7",
      "--primary-foreground": "#1e1e2e",
      "--secondary": "#89b4fa",
      "--secondary-foreground": "#1e1e2e",
      "--muted": "#45475a",
      "--muted-foreground": "#bac2de",
      "--accent": "#f5c2e7",
      "--accent-foreground": "#1e1e2e",
      "--destructive": "#f38ba8",
      "--destructive-foreground": "#1e1e2e",
      "--success": "#a6e3a1",
      "--success-foreground": "#1e1e2e",
      "--warning": "#f9e2af",
      "--warning-foreground": "#1e1e2e",
      "--border": "#585b70",
      "--chart-1": "#cba6f7",
      "--chart-2": "#a6e3a1",
      "--chart-3": "#f5c2e7",
      "--chart-4": "#89dceb",
      "--chart-5": "#fab387",
    },
  },
  {
    id: "nord",
    name: "Nord",
    mode: "dark",
    // Source: nordtheme.com/docs/colors-and-palettes (Polar Night / Snow Storm / Frost / Aurora)
    colors: {
      "--background": "#2e3440",
      "--foreground": "#d8dee9",
      "--card": "#3b4252",
      "--card-foreground": "#d8dee9",
      "--popover": "#3b4252",
      "--popover-foreground": "#d8dee9",
      "--primary": "#81a1c1",
      "--primary-foreground": "#2e3440",
      "--secondary": "#88c0d0",
      "--secondary-foreground": "#2e3440",
      "--muted": "#434c5e",
      "--muted-foreground": "#d8dee9",
      "--accent": "#b48ead",
      "--accent-foreground": "#2b313c",
      "--destructive": "#bf616a",
      "--destructive-foreground": "#111112",
      "--success": "#a3be8c",
      "--success-foreground": "#2e3440",
      "--warning": "#ebcb8b",
      "--warning-foreground": "#2e3440",
      "--border": "#4c566a",
      "--chart-1": "#81a1c1",
      "--chart-2": "#a3be8c",
      "--chart-3": "#b48ead",
      "--chart-4": "#88c0d0",
      "--chart-5": "#d08770",
    },
  },
  {
    id: "gruvbox-dark",
    name: "Gruvbox Dark",
    mode: "dark",
    // Source: github.com/morhetz/gruvbox medium-contrast dark palette.
    // Primary uses the iconic Gruvbox orange instead of blue.
    colors: {
      "--background": "#282828",
      "--foreground": "#ebdbb2",
      "--card": "#3c3836",
      "--card-foreground": "#ebdbb2",
      "--popover": "#3c3836",
      "--popover-foreground": "#ebdbb2",
      "--primary": "#fe8019",
      "--primary-foreground": "#282828",
      "--secondary": "#83a598",
      "--secondary-foreground": "#282828",
      "--muted": "#504945",
      "--muted-foreground": "#d5c4a1",
      "--accent": "#d3869b",
      "--accent-foreground": "#282828",
      "--destructive": "#fb4934",
      "--destructive-foreground": "#222222",
      "--success": "#b8bb26",
      "--success-foreground": "#282828",
      "--warning": "#fabd2f",
      "--warning-foreground": "#282828",
      "--border": "#665c54",
      "--chart-1": "#fe8019",
      "--chart-2": "#b8bb26",
      "--chart-3": "#d3869b",
      "--chart-4": "#83a598",
      "--chart-5": "#8ec07c",
    },
  },
  {
    id: "gruvbox-light",
    name: "Gruvbox Light",
    mode: "light",
    // Source: github.com/morhetz/gruvbox medium-contrast light palette.
    // Primary uses the iconic Gruvbox orange instead of blue.
    colors: {
      "--background": "#fbf1c7",
      "--foreground": "#3c3836",
      "--card": "#ebdbb2",
      "--card-foreground": "#3c3836",
      "--popover": "#ebdbb2",
      "--popover-foreground": "#3c3836",
      "--primary": "#af3a03",
      "--primary-foreground": "#fbf1c7",
      "--secondary": "#076678",
      "--secondary-foreground": "#fbf1c7",
      "--muted": "#d5c4a1",
      "--muted-foreground": "#595048",
      "--accent": "#8f3f71",
      "--accent-foreground": "#fbf1c7",
      "--destructive": "#9d0006",
      "--destructive-foreground": "#fbf1c7",
      "--success": "#79740e",
      "--success-foreground": "#fdf9e6",
      "--warning": "#b57614",
      "--warning-foreground": "#1b1a16",
      "--border": "#a89984",
      "--chart-1": "#af3a03",
      "--chart-2": "#79740e",
      "--chart-3": "#8f3f71",
      "--chart-4": "#076678",
      "--chart-5": "#427b58",
    },
  },
  {
    id: "solarized-dark",
    name: "Solarized Dark",
    mode: "dark",
    // Source: ethanschoonover.com/solarized — base03 / base02 / base01 / base00 / base0 / base1
    colors: {
      "--background": "#002b36",
      "--foreground": "#8d9d9f",
      "--card": "#073642",
      "--card-foreground": "#8d9d9f",
      "--popover": "#073642",
      "--popover-foreground": "#8d9d9f",
      "--primary": "#268bd2",
      "--primary-foreground": "#002028",
      "--secondary": "#2aa198",
      "--secondary-foreground": "#002b36",
      "--muted": "#073642",
      "--muted-foreground": "#93a1a1",
      "--accent": "#6c71c4",
      "--accent-foreground": "#070706",
      "--destructive": "#dc322f",
      "--destructive-foreground": "#fffefd",
      "--success": "#859900",
      "--success-foreground": "#002b36",
      "--warning": "#b58900",
      "--warning-foreground": "#002b36",
      "--border": "#586e75",
      "--chart-1": "#268bd2",
      "--chart-2": "#859900",
      "--chart-3": "#6c71c4",
      "--chart-4": "#2aa198",
      "--chart-5": "#cb4b16",
    },
  },
  {
    id: "solarized-light",
    name: "Solarized Light",
    mode: "light",
    // Source: ethanschoonover.com/solarized — same accents, inverted base scale
    colors: {
      "--background": "#fdf6e3",
      "--foreground": "#576a71",
      "--card": "#eee8d5",
      "--card-foreground": "#576a71",
      "--popover": "#eee8d5",
      "--popover-foreground": "#576a71",
      "--primary": "#268bd2",
      "--primary-foreground": "#1d1d1a",
      "--secondary": "#2aa198",
      "--secondary-foreground": "#2a2926",
      "--muted": "#eee8d5",
      "--muted-foreground": "#606969",
      "--accent": "#6c71c4",
      "--accent-foreground": "#070706",
      "--destructive": "#dc322f",
      "--destructive-foreground": "#fffefd",
      "--success": "#859900",
      "--success-foreground": "#292825",
      "--warning": "#b58900",
      "--warning-foreground": "#292825",
      "--border": "#cdc7b3",
      "--chart-1": "#268bd2",
      "--chart-2": "#859900",
      "--chart-3": "#6c71c4",
      "--chart-4": "#2aa198",
      "--chart-5": "#cb4b16",
    },
  },
  {
    id: "one-dark",
    name: "One Dark",
    mode: "dark",
    // Source: github.com/atom/atom one-dark-syntax/styles/colors.less (mono-1, hue-1..6)
    colors: {
      "--background": "#282c34",
      "--foreground": "#abb2bf",
      "--card": "#21252b",
      "--card-foreground": "#abb2bf",
      "--popover": "#21252b",
      "--popover-foreground": "#abb2bf",
      "--primary": "#61afef",
      "--primary-foreground": "#282c34",
      "--secondary": "#56b6c2",
      "--secondary-foreground": "#282c34",
      "--muted": "#3e4451",
      "--muted-foreground": "#abb2bf",
      "--accent": "#c678dd",
      "--accent-foreground": "#282c34",
      "--destructive": "#e06c75",
      "--destructive-foreground": "#252830",
      "--success": "#98c379",
      "--success-foreground": "#282c34",
      "--warning": "#e5c07b",
      "--warning-foreground": "#282c34",
      "--border": "#3e4451",
      "--chart-1": "#61afef",
      "--chart-2": "#98c379",
      "--chart-3": "#c678dd",
      "--chart-4": "#56b6c2",
      "--chart-5": "#d19a66",
    },
  },
  {
    id: "monokai-pro",
    name: "Monokai Pro",
    mode: "dark",
    // Source: classic Monokai filter (monokai-pro.nvim palette/classic.lua).
    // Primary uses Monokai's signature green instead of cyan.
    colors: {
      "--background": "#272822",
      "--foreground": "#fdfff1",
      "--card": "#1d1e19",
      "--card-foreground": "#fdfff1",
      "--popover": "#1d1e19",
      "--popover-foreground": "#fdfff1",
      "--primary": "#a6e22e",
      "--primary-foreground": "#272822",
      "--secondary": "#66d9ef",
      "--secondary-foreground": "#272822",
      "--muted": "#3b3c35",
      "--muted-foreground": "#a6a79f",
      "--accent": "#ae81ff",
      "--accent-foreground": "#272822",
      "--destructive": "#f92672",
      "--destructive-foreground": "#1a1a19",
      "--success": "#a6e22e",
      "--success-foreground": "#272822",
      "--warning": "#e6db74",
      "--warning-foreground": "#272822",
      "--border": "#57584f",
      "--chart-1": "#a6e22e",
      "--chart-2": "#66d9ef",
      "--chart-3": "#ae81ff",
      "--chart-4": "#e6db74",
      "--chart-5": "#fd971f",
    },
  },
  {
    id: "rose-pine",
    name: "Rosé Pine",
    mode: "dark",
    // Source: github.com/rose-pine/palette/blob/main/palette.json.
    // Primary uses iris (purple) — the iconic Rosé Pine accent — and
    // success uses pine. Destructive stays love (pink), which is correct
    // for the palette's red role.
    colors: {
      "--background": "#191724",
      "--foreground": "#e0def4",
      "--card": "#1f1d2e",
      "--card-foreground": "#e0def4",
      "--popover": "#1f1d2e",
      "--popover-foreground": "#e0def4",
      "--primary": "#c4a7e7",
      "--primary-foreground": "#191724",
      "--secondary": "#9ccfd8",
      "--secondary-foreground": "#191724",
      "--muted": "#26233a",
      "--muted-foreground": "#908caa",
      "--accent": "#ebbcba",
      "--accent-foreground": "#191724",
      "--destructive": "#eb6f92",
      "--destructive-foreground": "#191724",
      "--success": "#31748f",
      "--success-foreground": "#f1f0fa",
      "--warning": "#f6c177",
      "--warning-foreground": "#191724",
      "--border": "#403d52",
      "--chart-1": "#c4a7e7",
      "--chart-2": "#9ccfd8",
      "--chart-3": "#ebbcba",
      "--chart-4": "#eb6f92",
      "--chart-5": "#f6c177",
    },
  },
  {
    id: "rose-pine-dawn",
    name: "Rosé Pine Dawn",
    mode: "light",
    // Source: github.com/rose-pine/palette/blob/main/palette.json (dawn variant).
    // Primary uses iris (purple) for parity with the dark variant.
    colors: {
      "--background": "#faf4ed",
      "--foreground": "#575279",
      "--card": "#fffaf3",
      "--card-foreground": "#575279",
      "--popover": "#fffaf3",
      "--popover-foreground": "#575279",
      "--primary": "#907aa9",
      "--primary-foreground": "#1a1a19",
      "--secondary": "#56949f",
      "--secondary-foreground": "#242322",
      "--muted": "#f2e9e1",
      "--muted-foreground": "#696680",
      "--accent": "#d7827e",
      "--accent-foreground": "#32302f",
      "--destructive": "#b4637a",
      "--destructive-foreground": "#0e0e0e",
      "--success": "#286983",
      "--success-foreground": "#faf4ed",
      "--warning": "#ea9d34",
      "--warning-foreground": "#42403e",
      "--border": "#cecacd",
      "--chart-1": "#907aa9",
      "--chart-2": "#56949f",
      "--chart-3": "#d7827e",
      "--chart-4": "#b4637a",
      "--chart-5": "#ea9d34",
    },
  },
  {
    id: "github-dark",
    name: "GitHub Dark",
    mode: "dark",
    // Source: github.com/primer/primitives base color tokens (dark default)
    colors: {
      "--background": "#0d1117",
      "--foreground": "#f0f6fc",
      "--card": "#151b23",
      "--card-foreground": "#f0f6fc",
      "--popover": "#151b23",
      "--popover-foreground": "#f0f6fc",
      "--primary": "#1f6feb",
      "--primary-foreground": "#fefeff",
      "--secondary": "#58a6ff",
      "--secondary-foreground": "#0d1117",
      "--muted": "#212830",
      "--muted-foreground": "#9198a1",
      "--accent": "#8957e5",
      "--accent-foreground": "#ffffff",
      "--destructive": "#da3633",
      "--destructive-foreground": "#ffffff",
      "--success": "#238636",
      "--success-foreground": "#fefeff",
      "--warning": "#d29922",
      "--warning-foreground": "#0d1117",
      "--border": "#3d444d",
      "--chart-1": "#1f6feb",
      "--chart-2": "#238636",
      "--chart-3": "#8957e5",
      "--chart-4": "#58a6ff",
      "--chart-5": "#db6d28",
    },
  },
  {
    id: "github-light",
    name: "GitHub Light",
    mode: "light",
    // Source: github.com/primer/primitives base color tokens (light default)
    colors: {
      "--background": "#ffffff",
      "--foreground": "#25292e",
      "--card": "#f6f8fa",
      "--card-foreground": "#25292e",
      "--popover": "#f6f8fa",
      "--popover-foreground": "#25292e",
      "--primary": "#0969da",
      "--primary-foreground": "#ffffff",
      "--secondary": "#54aeff",
      "--secondary-foreground": "#25292e",
      "--muted": "#eff2f5",
      "--muted-foreground": "#59636e",
      "--accent": "#8250df",
      "--accent-foreground": "#ffffff",
      "--destructive": "#cf222e",
      "--destructive-foreground": "#ffffff",
      "--success": "#1a7f37",
      "--success-foreground": "#ffffff",
      "--warning": "#bf8700",
      "--warning-foreground": "#25292e",
      "--border": "#d1d9e0",
      "--chart-1": "#0969da",
      "--chart-2": "#1a7f37",
      "--chart-3": "#8250df",
      "--chart-4": "#54aeff",
      "--chart-5": "#bc4c00",
    },
  },
];

const LEGACY_PRESET_OVERRIDES: Record<string, Partial<ThemeColors>> = {
  dracula: {
    "--foreground": "#f8f8f2",
    "--card-foreground": "#f8f8f2",
    "--popover-foreground": "#f8f8f2",
    "--muted-foreground": "#f8f8f2",
    "--destructive-foreground": "#f8f8f2",
  },
  matchalk: {
    "--muted-foreground": "#7ea4b0",
  },
  houston: {
    "--secondary-foreground": "#f7f7f8",
    "--accent-foreground": "#f7f7f8",
    "--destructive-foreground": "#f7f7f8",
  },
  "ayu-dark": {
    "--muted-foreground": "#5c6773",
    "--destructive-foreground": "#b3b1ad",
  },
  "ayu-light": {
    "--primary-foreground": "#ffffff",
    "--secondary-foreground": "#ffffff",
    "--muted-foreground": "#828e9f",
    "--accent-foreground": "#ffffff",
    "--destructive-foreground": "#ffffff",
    "--success-foreground": "#ffffff",
    "--warning-foreground": "#ffffff",
  },
  "catppuccin-latte": {
    "--foreground": "#4c4f69",
    "--card-foreground": "#4c4f69",
    "--popover-foreground": "#4c4f69",
    "--secondary-foreground": "#eff1f5",
    "--muted-foreground": "#6c6f85",
    "--accent-foreground": "#eff1f5",
    "--success-foreground": "#eff1f5",
    "--warning-foreground": "#eff1f5",
  },
  "catppuccin-mocha": {
    "--muted-foreground": "#a6adc8",
  },
  nord: {
    "--accent-foreground": "#2e3440",
    "--destructive-foreground": "#eceff4",
  },
  "gruvbox-dark": {
    "--muted-foreground": "#a89984",
    "--destructive-foreground": "#282828",
  },
  "gruvbox-light": {
    "--muted-foreground": "#7c6f64",
    "--success-foreground": "#fbf1c7",
    "--warning-foreground": "#fbf1c7",
  },
  "solarized-dark": {
    "--foreground": "#839496",
    "--card-foreground": "#839496",
    "--popover-foreground": "#839496",
    "--primary-foreground": "#002b36",
    "--accent-foreground": "#fdf6e3",
    "--destructive-foreground": "#fdf6e3",
  },
  "solarized-light": {
    "--foreground": "#657b83",
    "--card-foreground": "#657b83",
    "--popover-foreground": "#657b83",
    "--primary-foreground": "#fdf6e3",
    "--secondary-foreground": "#fdf6e3",
    "--muted-foreground": "#93a1a1",
    "--accent-foreground": "#fdf6e3",
    "--destructive-foreground": "#fdf6e3",
    "--success-foreground": "#fdf6e3",
    "--warning-foreground": "#fdf6e3",
  },
  "one-dark": {
    "--muted-foreground": "#7d8590",
    "--destructive-foreground": "#282c34",
  },
  "monokai-pro": {
    "--muted-foreground": "#919288",
    "--destructive-foreground": "#fdfff1",
  },
  "rose-pine": {
    "--success-foreground": "#e0def4",
  },
  "rose-pine-dawn": {
    "--primary-foreground": "#faf4ed",
    "--secondary-foreground": "#faf4ed",
    "--muted-foreground": "#797593",
    "--accent-foreground": "#faf4ed",
    "--destructive-foreground": "#faf4ed",
    "--warning-foreground": "#faf4ed",
  },
  "github-dark": {
    "--primary-foreground": "#f0f6fc",
    "--accent-foreground": "#f0f6fc",
    "--destructive-foreground": "#f0f6fc",
    "--success-foreground": "#f0f6fc",
  },
  "github-light": {
    "--secondary-foreground": "#ffffff",
    "--warning-foreground": "#ffffff",
  },
};

export const THEME_VARIABLES: Array<{ key: keyof ThemeColors; label: string }> =
  [
    { key: "--background", label: "Background" },
    { key: "--foreground", label: "Foreground" },
    { key: "--card", label: "Card" },
    { key: "--card-foreground", label: "Card FG" },
    { key: "--popover", label: "Popover" },
    { key: "--popover-foreground", label: "Popover FG" },
    { key: "--primary", label: "Primary" },
    { key: "--primary-foreground", label: "Primary FG" },
    { key: "--secondary", label: "Secondary" },
    { key: "--secondary-foreground", label: "Secondary FG" },
    { key: "--muted", label: "Muted" },
    { key: "--muted-foreground", label: "Muted FG" },
    { key: "--accent", label: "Accent" },
    { key: "--accent-foreground", label: "Accent FG" },
    { key: "--destructive", label: "Destructive" },
    { key: "--destructive-foreground", label: "Destructive FG" },
    { key: "--success", label: "Success" },
    { key: "--success-foreground", label: "Success FG" },
    { key: "--warning", label: "Warning" },
    { key: "--warning-foreground", label: "Warning FG" },
    { key: "--border", label: "Border" },
    { key: "--chart-1", label: "Chart 1" },
    { key: "--chart-2", label: "Chart 2" },
    { key: "--chart-3", label: "Chart 3" },
    { key: "--chart-4", label: "Chart 4" },
    { key: "--chart-5", label: "Chart 5" },
  ];

export function getThemeById(id: string): Theme | undefined {
  return THEMES.find((theme) => theme.id === id);
}

export function getThemeByColors(
  colors: Record<string, string>,
): Theme | undefined {
  const matches = (expected: Record<string, string>) =>
    THEME_VARIABLES.every(({ key }) => expected[key] === colors[key]);
  const current = THEMES.find((theme) => matches(theme.colors));
  if (current) return current;

  return THEMES.find((theme) => {
    const overrides = LEGACY_PRESET_OVERRIDES[theme.id];
    if (!overrides) return false;
    const legacyColors: Record<string, string> = { ...theme.colors };
    for (const [key, value] of Object.entries(overrides)) {
      if (value !== undefined) legacyColors[key] = value;
    }
    return matches(legacyColors);
  });
}

export function normalizeThemeColors(
  colors: Record<string, string>,
): Record<string, string> {
  return getThemeByColors(colors)?.colors ?? colors;
}

interface RgbaColor {
  r: number;
  g: number;
  b: number;
  a: number;
}

export const DERIVED_THEME_VARIABLES = [
  "--primary-text",
  "--destructive-text",
  "--success-text",
  "--warning-text",
  "--input",
  "--ring",
  "--sidebar",
  "--sidebar-foreground",
  "--sidebar-primary",
  "--sidebar-primary-foreground",
  "--sidebar-accent",
  "--sidebar-accent-foreground",
  "--sidebar-border",
  "--sidebar-ring",
] as const;

function parseCssColor(value: string | undefined): RgbaColor | null {
  if (!value) return null;
  const normalized = value.trim().toLowerCase();
  const hex = normalized.match(/^#([\da-f]{3,8})$/)?.[1];
  if (hex) {
    if (hex.length === 3 || hex.length === 4) {
      return {
        r: Number.parseInt(`${hex[0]}${hex[0]}`, 16),
        g: Number.parseInt(`${hex[1]}${hex[1]}`, 16),
        b: Number.parseInt(`${hex[2]}${hex[2]}`, 16),
        a:
          hex.length === 4
            ? Number.parseInt(`${hex[3]}${hex[3]}`, 16) / 255
            : 1,
      };
    }
    if (hex.length === 6 || hex.length === 8) {
      return {
        r: Number.parseInt(hex.slice(0, 2), 16),
        g: Number.parseInt(hex.slice(2, 4), 16),
        b: Number.parseInt(hex.slice(4, 6), 16),
        a: hex.length === 8 ? Number.parseInt(hex.slice(6, 8), 16) / 255 : 1,
      };
    }
  }

  const rgb = normalized.match(
    /^rgba?\(\s*([\d.]+)\s*[, ]\s*([\d.]+)\s*[, ]\s*([\d.]+)(?:\s*[,/]\s*([\d.]+%?))?\s*\)$/,
  );
  if (!rgb) return null;
  const alpha = rgb[4]?.endsWith("%")
    ? Number.parseFloat(rgb[4]) / 100
    : Number.parseFloat(rgb[4] ?? "1");
  const channels = rgb.slice(1, 4).map(Number);
  if (
    channels.some((channel) => !Number.isFinite(channel)) ||
    !Number.isFinite(alpha)
  ) {
    return null;
  }
  return {
    r: Math.min(255, Math.max(0, channels[0])),
    g: Math.min(255, Math.max(0, channels[1])),
    b: Math.min(255, Math.max(0, channels[2])),
    a: Math.min(1, Math.max(0, alpha)),
  };
}

function mixColor(from: RgbaColor, to: RgbaColor, amount: number): RgbaColor {
  return {
    r: from.r + (to.r - from.r) * amount,
    g: from.g + (to.g - from.g) * amount,
    b: from.b + (to.b - from.b) * amount,
    a: from.a + (to.a - from.a) * amount,
  };
}

function compositeColor(foreground: RgbaColor, background: RgbaColor) {
  const alpha = foreground.a + background.a * (1 - foreground.a);
  if (alpha === 0) return { r: 0, g: 0, b: 0, a: 0 };
  return {
    r:
      (foreground.r * foreground.a +
        background.r * background.a * (1 - foreground.a)) /
      alpha,
    g:
      (foreground.g * foreground.a +
        background.g * background.a * (1 - foreground.a)) /
      alpha,
    b:
      (foreground.b * foreground.a +
        background.b * background.a * (1 - foreground.a)) /
      alpha,
    a: alpha,
  };
}

function relativeLuminance(color: RgbaColor) {
  const linear = [color.r, color.g, color.b].map((channel) => {
    const value = channel / 255;
    return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * linear[0] + 0.7152 * linear[1] + 0.0722 * linear[2];
}

function contrastRatio(foreground: RgbaColor, background: RgbaColor) {
  const opaqueForeground = compositeColor(foreground, background);
  const foregroundLuminance = relativeLuminance(opaqueForeground);
  const backgroundLuminance = relativeLuminance(background);
  const lighter = Math.max(foregroundLuminance, backgroundLuminance);
  const darker = Math.min(foregroundLuminance, backgroundLuminance);
  return (lighter + 0.05) / (darker + 0.05);
}

function colorToCss(color: RgbaColor) {
  const channels = [color.r, color.g, color.b].map((channel) =>
    Math.round(channel).toString(16).padStart(2, "0"),
  );
  if (color.a >= 0.999) return `#${channels.join("")}`;
  return `rgba(${Math.round(color.r)}, ${Math.round(color.g)}, ${Math.round(
    color.b,
  )}, ${color.a.toFixed(3)})`;
}

function findReadableColor(
  sourceValue: string | undefined,
  surfaceValues: Array<string | undefined>,
  minimumContrast: number,
  fallback: string,
  includeTintedSurfaces = false,
  tintSurfaceValues = surfaceValues,
) {
  const source = parseCssColor(sourceValue);
  const surfaces = surfaceValues
    .map(parseCssColor)
    .filter((surface): surface is RgbaColor => surface !== null);
  const tintSurfaces = tintSurfaceValues
    .map(parseCssColor)
    .filter((surface): surface is RgbaColor => surface !== null);
  if (!source || surfaces.length === 0) return fallback;

  const testedSurfaces = includeTintedSurfaces
    ? [
        ...surfaces,
        ...tintSurfaces.flatMap((surface) =>
          [0.1, 0.2].map((opacity) =>
            compositeColor({ ...source, a: source.a * opacity }, surface),
          ),
        ),
      ]
    : surfaces;
  const targetContrast = minimumContrast + 0.05;
  const isReadable = (candidate: RgbaColor) =>
    testedSurfaces.every(
      (surface) => contrastRatio(candidate, surface) >= targetContrast,
    );
  if (isReadable(source)) return sourceValue ?? fallback;

  const targets: RgbaColor[] = [
    { r: 0, g: 0, b: 0, a: 1 },
    { r: 255, g: 255, b: 255, a: 1 },
  ];
  for (let step = 1; step <= 1000; step += 1) {
    const amount = step / 1000;
    for (const target of targets) {
      const candidate = mixColor(source, target, amount);
      if (isReadable(candidate)) return colorToCss(candidate);
    }
  }
  return fallback;
}

export function getThemeMode(colors: Record<string, string>): "light" | "dark" {
  const preset = getThemeByColors(colors);
  if (preset) return preset.mode;

  const background = parseCssColor(colors["--background"]);
  if (!background) return "dark";
  const opaqueBackground = compositeColor(background, {
    r: 255,
    g: 255,
    b: 255,
    a: 1,
  });
  const blackContrast = contrastRatio(
    { r: 0, g: 0, b: 0, a: 1 },
    opaqueBackground,
  );
  const whiteContrast = contrastRatio(
    { r: 255, g: 255, b: 255, a: 1 },
    opaqueBackground,
  );
  return blackContrast >= whiteContrast ? "light" : "dark";
}

export function getDerivedThemeColors(
  colors: Record<string, string>,
): Record<(typeof DERIVED_THEME_VARIABLES)[number], string> {
  const surfaces = [
    colors["--background"],
    colors["--card"],
    colors["--popover"],
    colors["--muted"],
  ];
  const tintSurfaces = surfaces.slice(0, 3);
  const foreground = colors["--foreground"] ?? "#ffffff";
  const primaryText = findReadableColor(
    colors["--primary"],
    surfaces,
    4.5,
    foreground,
    true,
    tintSurfaces,
  );
  const destructiveText = findReadableColor(
    colors["--destructive"],
    surfaces,
    4.5,
    foreground,
    true,
    tintSurfaces,
  );
  const successText = findReadableColor(
    colors["--success"],
    surfaces,
    4.5,
    foreground,
    true,
    tintSurfaces,
  );
  const warningText = findReadableColor(
    colors["--warning"],
    surfaces,
    4.5,
    foreground,
    true,
    tintSurfaces,
  );
  const input = findReadableColor(colors["--border"], surfaces, 3, foreground);

  return {
    "--primary-text": primaryText,
    "--destructive-text": destructiveText,
    "--success-text": successText,
    "--warning-text": warningText,
    "--input": input,
    "--ring": primaryText,
    "--sidebar": colors["--background"],
    "--sidebar-foreground": colors["--foreground"],
    "--sidebar-primary": colors["--primary"],
    "--sidebar-primary-foreground": colors["--primary-foreground"],
    "--sidebar-accent": colors["--accent"],
    "--sidebar-accent-foreground": colors["--accent-foreground"],
    "--sidebar-border": colors["--border"],
    "--sidebar-ring": primaryText,
  };
}

export function applyThemeColors(colors: Record<string, string>): void {
  const normalizedColors = normalizeThemeColors(colors);
  const root = document.documentElement;
  root.classList.remove("light", "dark");
  root.classList.add(getThemeMode(normalizedColors));
  Object.entries(normalizedColors).forEach(([key, value]) => {
    root.style.setProperty(key, value, "important");
  });
  Object.entries(getDerivedThemeColors(normalizedColors)).forEach(
    ([key, value]) => {
      root.style.setProperty(key, value, "important");
    },
  );
}

export function clearThemeColors(): void {
  const root = document.documentElement;
  THEME_VARIABLES.forEach(({ key }) => {
    root.style.removeProperty(key as string);
  });
  DERIVED_THEME_VARIABLES.forEach((key) => {
    root.style.removeProperty(key);
  });
}

/**
 * Run a theme mutation inside a View Transition so the whole UI cross-fades
 * (~200ms, tuned in globals.css) instead of hard-cutting between palettes.
 * Falls back to an instant switch when the API is unavailable or the user
 * prefers reduced motion.
 */
export function withThemeTransition(mutate: () => void): void {
  if (typeof document === "undefined") {
    mutate();
    return;
  }
  const reduced =
    typeof window !== "undefined" &&
    window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  const doc = document as Document & {
    startViewTransition?: (callback: () => void) => unknown;
  };
  if (reduced || typeof doc.startViewTransition !== "function") {
    mutate();
    return;
  }
  doc.startViewTransition(mutate);
}
