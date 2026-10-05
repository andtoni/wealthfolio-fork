import { BlockBar } from "@/components/block-bar";
import { useMemo } from "react";
import { CATEGORY_CSS_COLORS, type ParsedNetWorth } from "./utils";

/** Asset composition (% of assets) as blocks. The breakdown rows below act as its legend. */
export function CompositionBar({ data, className }: { data: ParsedNetWorth; className?: string }) {
  const segments = useMemo(
    () =>
      data.assets.breakdown
        .filter((item) => item.value > 0)
        .sort((a, b) => b.value - a.value)
        .map((item) => ({
          value: item.value,
          color: CATEGORY_CSS_COLORS[item.category] ?? "var(--muted-foreground)",
          label: `${item.name} ${((item.value / data.assets.total) * 100).toFixed(1)}%`,
        })),
    [data],
  );

  if (segments.length === 0) return null;

  return <BlockBar segments={segments} className={className} />;
}
