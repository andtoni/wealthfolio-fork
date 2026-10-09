export type Palette = {
  key: string;
  label: string;
  hsl: string;
  deep: string;
  mid: string;
};

export const FOREST_THEME: Palette = {
  key: "forest",
  label: "Forest",
  hsl: "var(--success)",
  deep: "var(--success)",
  mid: "color-mix(in srgb, var(--success) 60%, transparent)",
};

export const themeBg = (p: Palette, alpha: number): string =>
  `color-mix(in srgb, ${p.hsl} ${alpha * 100}%, transparent)`;
