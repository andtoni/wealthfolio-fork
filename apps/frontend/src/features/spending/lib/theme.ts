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

/** The warm gold the stock app used behind Net Worth; here it is the Spending tab's page wash. */
export const SPENDING_WASH = "hsl(38 75% 50%)";

export const themeBg = (p: Palette, alpha: number): string =>
  `color-mix(in srgb, ${p.hsl} ${alpha * 100}%, transparent)`;
