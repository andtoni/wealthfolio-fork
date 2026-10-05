import { Fragment, useMemo } from "react";
import { useQueries } from "@tanstack/react-query";
import { Link } from "react-router-dom";
import { useTranslation } from "react-i18next";

import { LIST_ROW, SECTION_HEADING } from "@/components/dashboard-card";
import { QueryKeys } from "@/lib/query-keys";
import { isProvisionalCardSpending } from "@/lib/original-currency";
import type { Activity } from "@/lib/types";
import { formatDateISO } from "@/lib/utils";
import { Icons } from "@wealthfolio/ui";

import { getActivityAssignments } from "../adapters/cash-activities";
import {
  getActivitySpendingAmount,
  getEffectiveCashActivityType,
  isCashActivityIncome,
} from "../lib/constants";
import type { CategoryMetaMap } from "./category-chips";
import { MerchantAvatar } from "./merchant-avatar";
import { SplitAmount } from "./split-amount";

const SPENDING_TAXONOMY = "spending_categories";
const SHOWN = 8;
const ALL_TRANSACTIONS = "/activities?tab=spending";

/** The latest few transactions on the Spending tab, by day; the heading opens the full list. */
export function RecentTransactions({
  activities,
  accountTypeById,
  categoriesMeta,
}: {
  activities: Activity[];
  accountTypeById?: Map<string, string>;
  categoriesMeta: CategoryMetaMap;
}) {
  const { t } = useTranslation();
  const recent = useMemo(
    () =>
      activities
        .map((activity) => {
          const accountType = accountTypeById?.get(activity.accountId);
          const isIncome = isCashActivityIncome(
            getEffectiveCashActivityType(activity),
            accountType,
            activity.subtype,
          );
          return { activity, isIncome, spent: getActivitySpendingAmount(activity, accountType) };
        })
        .filter((row) => row.spent !== 0 || row.isIncome)
        .sort((a, b) => b.activity.activityDate.localeCompare(a.activity.activityDate))
        .slice(0, SHOWN),
    [activities, accountTypeById],
  );

  const assignmentQueries = useQueries({
    queries: recent.map(({ activity }) => ({
      queryKey: [QueryKeys.SPENDING_TRANSACTIONS, "assignments", activity.id],
      queryFn: () => getActivityAssignments(activity.id),
      staleTime: 30_000,
    })),
  });

  if (recent.length === 0) return null;

  const today = new Date();
  const yesterday = new Date(today);
  yesterday.setDate(today.getDate() - 1);
  const dayLabel = (day: string) =>
    day === formatDateISO(today)
      ? t("spending:dashboard.today")
      : day === formatDateISO(yesterday)
        ? t("spending:dashboard.yesterday")
        : new Date(`${day}T00:00:00`).toLocaleDateString(undefined, {
            weekday: "short",
            month: "short",
            day: "numeric",
          });
  let lastDay = "";

  return (
    <div>
      <Link to={ALL_TRANSACTIONS} className={`${SECTION_HEADING} inline-flex items-center gap-1`}>
        {t("dashboard:tabs.transactions")}
        <Icons.ChevronRight className="text-muted-foreground h-5 w-5" aria-hidden="true" />
      </Link>
      <div className="pt-1">
        {recent.map(({ activity, isIncome, spent }, index) => {
          const assigned = assignmentQueries[index]?.data?.find(
            (assignment) => assignment.taxonomyId === SPENDING_TAXONOMY,
          );
          const category = assigned ? categoriesMeta.get(assigned.categoryId) : undefined;
          const topCategory = category?.parentId
            ? (categoriesMeta.get(category.parentId) ?? category)
            : category;
          const day = activity.activityDate.slice(0, 10);
          const heading = day === lastDay ? null : day;
          lastDay = day;
          return (
            <Fragment key={activity.id}>
              {heading && (
                <h3 className="text-muted-foreground pb-0.5 pt-3 text-[13px] first:pt-1">
                  {dayLabel(heading)}
                </h3>
              )}
              <Link to={ALL_TRANSACTIONS} className={`${LIST_ROW} py-1.5`}>
                <MerchantAvatar
                  name={activity.notes ?? null}
                  currency={activity.currency}
                  tint={topCategory?.color}
                  logo={!isIncome}
                  className="h-8 w-8 text-[11px]"
                />
                <div className="min-w-0 flex-1">
                  <div className="truncate text-[15px] leading-snug">{activity.notes ?? "—"}</div>
                  <div className="text-muted-foreground flex min-w-0 items-center gap-1.5 text-[13px] leading-snug">
                    <span className="truncate">
                      {topCategory?.name ??
                        (assignmentQueries[index]?.isLoading
                          ? " "
                          : t("spending:dashboard.uncategorized"))}
                    </span>
                    {isProvisionalCardSpending(activity) && (
                      <span className="bg-foreground/[0.07] shrink-0 rounded px-1 text-[11px]">
                        {t("spending:transactions.provisional")}
                      </span>
                    )}
                  </div>
                </div>
                <div className={isIncome ? "text-success text-[16px]" : "text-[16px]"}>
                  <SplitAmount
                    value={isIncome ? Number(activity.amount) : spent}
                    currency={activity.currency}
                    sign={isIncome ? "+" : "-"}
                  />
                </div>
              </Link>
            </Fragment>
          );
        })}
      </div>
    </div>
  );
}
