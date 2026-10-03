import { useTranslation } from "react-i18next";

import { Badge } from "@wealthfolio/ui";

import { useSpendingSettings } from "../hooks/use-spending-settings";
import { budgetPeriodLabel } from "../lib/month-period";

/** Marks a transaction that counts in a budget period other than its own date's. */
export function BudgetPeriodBadge({ periodStart }: { periodStart?: string | null }) {
  const { t, i18n } = useTranslation();
  const { budgetCycle } = useSpendingSettings();
  if (!periodStart) return null;
  const period = budgetPeriodLabel(periodStart, "short", i18n.language, budgetCycle);
  if (!period) return null;
  return (
    <Badge
      variant="outline"
      className="shrink-0 border-violet-500/50 text-[10px] text-violet-600"
      title={t("spending:transactions.budgetPeriodHint", { period })}
    >
      {t("spending:transactions.budgetPeriodBadge", { period })}
    </Badge>
  );
}
