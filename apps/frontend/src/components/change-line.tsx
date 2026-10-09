import { useBalancePrivacy } from "@/hooks/use-balance-privacy";
import type { Change } from "@/lib/performance";
import { cn } from "@/lib/utils";

interface ChangeLineProps {
  change: Change;
  currency: string;
  label: string;
  /** Grey throughout, for the secondary line. */
  muted?: boolean;
  className?: string;
}

const sign = (value: number) => (value > 0 ? "+" : value < 0 ? "−" : "");

/** "+R 10,000 +1.0% this month": a change under a headline figure. */
export function ChangeLine({ change, currency, label, muted, className }: ChangeLineProps) {
  const { isBalanceHidden } = useBalancePrivacy();
  const amount = Math.round(change.amount);
  const money = isBalanceHidden
    ? "••••"
    : new Intl.NumberFormat(undefined, {
        style: "currency",
        currency,
        currencyDisplay: "narrowSymbol",
        maximumFractionDigits: 0,
      }).format(Math.abs(amount));
  const tone = muted
    ? undefined
    : amount > 0
      ? "text-success"
      : amount < 0
        ? "text-destructive"
        : undefined;

  return (
    <p className={cn("text-muted-foreground whitespace-nowrap tabular-nums", className)}>
      <span className={tone}>
        {sign(amount)}
        {money}
        {change.percent != null &&
          ` ${sign(change.percent)}${(Math.abs(change.percent) * 100).toFixed(1)}%`}
      </span>{" "}
      {label}
    </p>
  );
}
