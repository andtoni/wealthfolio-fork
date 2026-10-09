/**
 * The colour wash of the Investments and Net Worth tabs: it rises softly behind the chart,
 * then fades into the page background and stays there, however long the page is.
 */
export function pageWash(color: string): string {
  return `linear-gradient(to bottom, transparent, color-mix(in srgb, ${color} 14%, transparent) 140px, color-mix(in srgb, ${color} 10%, transparent) 280px, transparent 760px)`;
}
