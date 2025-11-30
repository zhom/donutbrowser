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
  colors: ThemeColors;
}

export const THEMES: Theme[] = [
  {
    id: "tokyo-night",
    name: "Tokyo Night",
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
    colors: {
      "--background": "#282a36",
      "--foreground": "#f8f8f2",
      "--card": "#44475a",
      "--card-foreground": "#f8f8f2",
      "--popover": "#44475a",
      "--popover-foreground": "#f8f8f2",
      "--primary": "#bd93f9",
      "--primary-foreground": "#282a36",
      "--secondary": "#8be9fd",
      "--secondary-foreground": "#282a36",
      "--muted": "#6272a4",
      "--muted-foreground": "#f8f8f2",
      "--accent": "#ff79c6",
      "--accent-foreground": "#282a36",
      "--destructive": "#ff5555",
      "--destructive-foreground": "#f8f8f2",
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
      "--muted-foreground": "#7ea4b0",
      "--accent": "#d2b48c",
      "--accent-foreground": "#273136",
      "--destructive": "#ff819f",
      "--destructive-foreground": "#273136",
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
      "--secondary-foreground": "#f7f7f8",
      "--muted": "#2a2e39",
      "--muted-foreground": "#9ca3af",
      "--accent": "#0ea5e9",
      "--accent-foreground": "#f7f7f8",
      "--destructive": "#ef4444",
      "--destructive-foreground": "#f7f7f8",
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
      "--muted-foreground": "#5c6773",
      "--accent": "#d2a6ff",
      "--accent-foreground": "#0a0e14",
      "--destructive": "#f07178",
      "--destructive-foreground": "#b3b1ad",
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
    colors: {
      "--background": "#fafafa",
      "--foreground": "#5c6773",
      "--card": "#ffffff",
      "--card-foreground": "#5c6773",
      "--popover": "#ffffff",
      "--popover-foreground": "#5c6773",
      "--primary": "#399ee6",
      "--primary-foreground": "#fafafa",
      "--secondary": "#fa8d3e",
      "--secondary-foreground": "#fafafa",
      "--muted": "#f0f0f0",
      "--muted-foreground": "#828c99",
      "--accent": "#a37acc",
      "--accent-foreground": "#fafafa",
      "--destructive": "#f07178",
      "--destructive-foreground": "#fafafa",
      "--border": "#e7eaed",
      "--chart-1": "#399ee6",
      "--chart-2": "#86b300",
      "--chart-3": "#a37acc",
      "--chart-4": "#fa8d3e",
      "--chart-5": "#f07178",
    },
  },
  {
    id: "catppuccin-latte",
    name: "Catppuccin Latte",
    colors: {
      "--background": "#eff1f5",
      "--foreground": "#4c4f69",
      "--card": "#ccd0da",
      "--card-foreground": "#4c4f69",
      "--popover": "#ccd0da",
      "--popover-foreground": "#4c4f69",
      "--primary": "#1e66f5",
      "--primary-foreground": "#eff1f5",
      "--secondary": "#04a5e5",
      "--secondary-foreground": "#eff1f5",
      "--muted": "#bcc0cc",
      "--muted-foreground": "#5c5f77",
      "--accent": "#8839ef",
      "--accent-foreground": "#eff1f5",
      "--destructive": "#d20f39",
      "--destructive-foreground": "#eff1f5",
      "--border": "#9ca0b0",
      "--chart-1": "#1e66f5",
      "--chart-2": "#40a02b",
      "--chart-3": "#8839ef",
      "--chart-4": "#04a5e5",
      "--chart-5": "#df8e1d",
    },
  },
  {
    id: "catppuccin-frappe",
    name: "Catppuccin Frappe",
    colors: {
      "--background": "#303446",
      "--foreground": "#c6d0f5",
      "--card": "#414559",
      "--card-foreground": "#c6d0f5",
      "--popover": "#414559",
      "--popover-foreground": "#c6d0f5",
      "--primary": "#8caaee",
      "--primary-foreground": "#303446",
      "--secondary": "#99d1db",
      "--secondary-foreground": "#303446",
      "--muted": "#51576d",
      "--muted-foreground": "#b5bfe2",
      "--accent": "#ca9ee6",
      "--accent-foreground": "#303446",
      "--destructive": "#e78284",
      "--destructive-foreground": "#303446",
      "--border": "#737994",
      "--chart-1": "#8caaee",
      "--chart-2": "#a6d189",
      "--chart-3": "#ca9ee6",
      "--chart-4": "#99d1db",
      "--chart-5": "#e5c890",
    },
  },
  {
    id: "catppuccin-macchiato",
    name: "Catppuccin Macchiato",
    colors: {
      "--background": "#24273a",
      "--foreground": "#cad3f5",
      "--card": "#363a4f",
      "--card-foreground": "#cad3f5",
      "--popover": "#363a4f",
      "--popover-foreground": "#cad3f5",
      "--primary": "#8aadf4",
      "--primary-foreground": "#24273a",
      "--secondary": "#91d7e3",
      "--secondary-foreground": "#24273a",
      "--muted": "#494d64",
      "--muted-foreground": "#b8c0e0",
      "--accent": "#c6a0f6",
      "--accent-foreground": "#24273a",
      "--destructive": "#ed8796",
      "--destructive-foreground": "#24273a",
      "--border": "#6e738d",
      "--chart-1": "#8aadf4",
      "--chart-2": "#a6da95",
      "--chart-3": "#c6a0f6",
      "--chart-4": "#91d7e3",
      "--chart-5": "#eed49f",
    },
  },
  {
    id: "catppuccin-mocha",
    name: "Catppuccin Mocha",
    colors: {
      "--background": "#1e1e2e",
      "--foreground": "#cdd6f4",
      "--card": "#313244",
      "--card-foreground": "#cdd6f4",
      "--popover": "#313244",
      "--popover-foreground": "#cdd6f4",
      "--primary": "#89b4fa",
      "--primary-foreground": "#1e1e2e",
      "--secondary": "#89dceb",
      "--secondary-foreground": "#1e1e2e",
      "--muted": "#45475a",
      "--muted-foreground": "#bac2de",
      "--accent": "#cba6f7",
      "--accent-foreground": "#1e1e2e",
      "--destructive": "#f38ba8",
      "--destructive-foreground": "#1e1e2e",
      "--border": "#585b70",
      "--chart-1": "#89b4fa",
      "--chart-2": "#a6e3a1",
      "--chart-3": "#cba6f7",
      "--chart-4": "#89dceb",
      "--chart-5": "#f9e2af",
    },
  },
];

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
  return THEMES.find((theme) => {
    return THEME_VARIABLES.every(({ key }) => {
      return theme.colors[key] === colors[key];
    });
  });
}

export function applyThemeColors(colors: Record<string, string>): void {
  const root = document.documentElement;
  Object.entries(colors).forEach(([key, value]) => {
    root.style.setProperty(key, value, "important");
  });
}

export function clearThemeColors(): void {
  const root = document.documentElement;
  THEME_VARIABLES.forEach(({ key }) => {
    root.style.removeProperty(key as string);
  });
}
