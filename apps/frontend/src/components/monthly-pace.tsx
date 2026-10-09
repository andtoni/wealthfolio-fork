import { useBalancePrivacy } from "@/hooks/use-balance-privacy";
import { cn } from "@/lib/utils";
import { formatCompactAmount, formatPercent } from "@wealthfolio/ui";
import { useTranslation } from "react-i18next";

interface MonthlyPaceProps {
  /** Average change per month, as money or as a rate (0.02 is 2%). */
  value: number;
  /** Set for money; left out, `value` is shown as a percentage. */
  currency?: string;
  className?: string;
}

/** "+R 141K a month": the average monthly pace behind a headline figure. */
export function MonthlyPace({ value, currency, className }: MonthlyPaceProps) {
  const { t } = useTranslation();
  const { isBalanceHidden } = useBalancePrivacy();
  const size = Math.abs(value);
  const text = !currency
    ? formatPercent(size)
    : isBalanceHidden
      ? "••••"
      : formatCompactAmount(size, currency);
  return (
    <span className={cn("whitespace-nowrap tabular-nums", className)}>
      <span
        className={
          value > 0 ? "text-success" : value < 0 ? "text-destructive" : "text-muted-foreground"
        }
      >
        {value > 0 ? "+" : value < 0 ? "−" : ""}
        {text}
      </span>{" "}
      <span className="text-muted-foreground">{t("ui:pace.a_month")}</span>
    </span>
  );
}
