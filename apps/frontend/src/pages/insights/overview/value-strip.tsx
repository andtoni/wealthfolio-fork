import { SplitAmount } from "@/features/spending/components/split-amount";
import { formatAmount, formatPercent, Skeleton } from "@wealthfolio/ui";
import { useBalancePrivacy } from "@/hooks/use-balance-privacy";
import { useTranslation } from "react-i18next";
import type { ValueStripData } from "./allocation-derivations";

interface ValueStripProps {
  data: ValueStripData;
  currency: string;
  isLoading?: boolean;
}

/** Portfolio value with its cash, invested and book cost parts, as plain rows. */
export function ValueStrip({ data, currency, isLoading }: ValueStripProps) {
  const { t } = useTranslation();
  const { isBalanceHidden } = useBalancePrivacy();

  if (isLoading) {
    return (
      <div className="space-y-3">
        <Skeleton className="h-9 w-48" />
        <Skeleton className="h-4 w-40" />
        <Skeleton className="h-12 w-full" />
      </div>
    );
  }

  const share = (value: number) =>
    t("insights:insights.value_strip.of_portfolio", {
      percent: formatPercent(data.total > 0 ? value / data.total : 0),
    });
  const byCurrency = (split: { currency: string; value: number }[]) =>
    isBalanceHidden
      ? split.map((part) => part.currency).join(", ")
      : split
          .slice(0, 4)
          .map((part) => formatAmount(part.value, part.currency))
          .join(", ");

  const parts = [
    {
      label: t("insights:insights.value_strip.cash_balance"),
      value: data.cash,
      detail:
        data.cashCurrencySplit.length === 0
          ? t("insights:insights.value_strip.no_cash_balance")
          : data.cashCurrencySplit.length > 1
            ? byCurrency(data.cashCurrencySplit)
            : share(data.cash),
    },
    {
      label: t("insights:insights.value_strip.invested"),
      value: data.invested,
      detail: share(data.invested),
    },
    {
      label: t("insights:insights.value_strip.book_cost"),
      value: data.bookCost,
      detail:
        data.bookCostCurrencySplit.length > 1
          ? byCurrency(data.bookCostCurrencySplit)
          : share(data.bookCost),
    },
  ];

  return (
    <div>
      <h1 className="text-3xl font-medium tracking-tight">
        <SplitAmount value={data.total} currency={currency} />
      </h1>
      <div className="text-muted-foreground mt-1 text-[15px] tabular-nums">
        {[
          t("insights:insights.value_strip.holdings_count", { count: data.holdingsCount }),
          t("insights:insights.value_strip.accounts_count", { count: data.accountsCount }),
          ...(data.currencySplit.length > 1
            ? data.currencySplit
                .slice(0, 4)
                .map((part) => `${part.currency} ${formatPercent(part.percentage / 100)}`)
            : []),
        ].join(", ")}
      </div>
      <div className="mt-4 grid gap-x-12 sm:grid-cols-3">
        {parts.map((part) => (
          <div key={part.label} className="flex items-center justify-between gap-3 py-2">
            <div className="min-w-0">
              <div className="truncate text-[17px]">{part.label}</div>
              <div className="text-muted-foreground truncate text-[15px] tabular-nums">
                {part.detail}
              </div>
            </div>
            <div className="text-[17px]">
              <SplitAmount value={part.value} currency={currency} />
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
