import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router-dom";

import { Icons, Switch } from "@wealthfolio/ui";

import {
  useSpendingSettings,
  useSpendingSettingsMutation,
} from "@/features/spending/hooks/use-spending-settings";
import { useBudget } from "@/features/spending/hooks/use-budget";
import {
  useCategorizationRules,
  useRulePresets,
} from "@/features/spending/hooks/use-categorization-rules";
import { isSpendingAccountType } from "@/features/spending/lib/constants";
import { useAccounts } from "@/hooks/use-accounts";
import { useBalancePrivacy } from "@/hooks/use-balance-privacy";
import type { Account } from "@/lib/types";
import { LIST_ROW } from "@/components/dashboard-card";
import { cn } from "@/lib/utils";

import { formatAmountWhole } from "./format";

export function ModuleCard() {
  const { t } = useTranslation();
  const { settings, isLoading } = useSpendingSettings();
  const { isBalanceHidden } = useBalancePrivacy();
  const fmt = (amount: number, currency: string) =>
    isBalanceHidden ? "••••" : formatAmountWhole(amount, currency);
  const mutation = useSpendingSettingsMutation();
  const { accounts } = useAccounts({ filterActive: true });
  const { data: budget } = useBudget();
  const { data: rules, isError: rulesErrored } = useCategorizationRules();
  const { data: presets, isError: presetsErrored } = useRulePresets();

  const enabled = settings?.enabled ?? false;
  const accountIds = useMemo(() => settings?.accountIds ?? [], [settings?.accountIds]);

  const spendingAccounts = useMemo<Account[]>(
    () => (accounts ?? []).filter((a) => isSpendingAccountType(a.accountType)),
    [accounts],
  );
  const tracked = useMemo(
    () => spendingAccounts.filter((a) => accountIds.includes(a.id)),
    [spendingAccounts, accountIds],
  );

  const handleToggle = (next: boolean) => {
    let nextIds = accountIds;
    if (next && accountIds.length === 0 && spendingAccounts.length > 0) {
      nextIds = spendingAccounts.map((a) => a.id);
    }
    mutation.mutate({ enabled: next, accountIds: nextIds });
  };

  // Spending-accounts subline: dominant type · currency
  const trackedSummary = useMemo(() => {
    if (tracked.length === 0) return t("settings:spending.module.no_accounts_tracked");
    const currencies = Array.from(new Set(tracked.map((a) => a.currency)));
    const types = Array.from(new Set(tracked.map((a) => a.accountType)));
    const typeLabel =
      types.length === 1
        ? types[0] === "CREDIT_CARD"
          ? t("settings:spending.module.type_credit_card")
          : types[0] === "CASH"
            ? t("settings:spending.module.type_cash")
            : t("settings:spending.module.type_mixed")
        : t("settings:spending.module.type_mixed");
    return `${typeLabel}, ${currencies.join(", ")}`;
  }, [tracked, t]);

  // Rules + region count (regions = installed country presets)
  const rulesUnavailable = rulesErrored || presetsErrored;
  const ruleCount = rules?.length ?? 0;
  const regionCount = useMemo(() => (presets ?? []).filter((p) => p.installed).length, [presets]);

  // Budget totals
  const spendingPlanned = budget?.computed.totals.spendingPlanned ?? 0;
  const incomePlanned = budget?.computed.totals.incomePlanned ?? 0;
  const currency = budget?.computed.currency ?? "USD";
  const plannedPct = incomePlanned > 0 ? Math.round((spendingPlanned / incomePlanned) * 100) : null;
  const overPlan = plannedPct !== null && plannedPct > 100;

  const activeGroups = useMemo(
    () =>
      (budget?.computed.groupRows ?? []).filter((row) => row.plannedTotal > 0 || row.actual > 0),
    [budget?.computed.groupRows],
  );

  const groupNamesSubline = useMemo(() => {
    if (activeGroups.length === 0) return undefined;
    return (
      activeGroups
        .slice(0, 3)
        .map((g) => g.group.name)
        .join(", ") + (activeGroups.length > 3 ? "…" : "")
    );
  }, [activeGroups]);

  return (
    <section aria-label={t("settings:spending.module.tracker_status_aria")}>
      <label className="flex cursor-pointer items-center justify-between gap-4">
        <span className="min-w-0">
          <span className="block text-[17px] leading-snug">
            {enabled
              ? t("settings:spending.module.tracking_active")
              : t("settings:spending.module.tracking_disabled")}
          </span>
          <span className="text-muted-foreground block text-[15px] leading-snug">
            {enabled
              ? t("settings:spending.module.headline_enabled", {
                  tracked: tracked.length,
                  total: spendingAccounts.length,
                  count: spendingAccounts.length,
                })
              : t("settings:spending.module.headline_disabled")}
          </span>
        </span>
        <Switch
          checked={enabled}
          onCheckedChange={handleToggle}
          disabled={isLoading || mutation.isPending}
        />
      </label>

      {/* Only meaningful when tracking is enabled */}
      {enabled && (
        <div className="pt-4">
          <StatRow
            label={t("settings:spending.module.stat_accounts")}
            value={tracked.length}
            unit={t("settings:spending.module.stat_accounts_unit", {
              count: spendingAccounts.length,
            })}
            sub={trackedSummary}
          />
          <StatRow
            label={t("settings:spending.module.stat_categorization")}
            value={rulesUnavailable ? "—" : ruleCount}
            unit={
              rulesUnavailable
                ? undefined
                : t("settings:spending.module.stat_rules_unit", { count: ruleCount })
            }
            sub={
              rulesUnavailable
                ? t("settings:spending.module.rules_unavailable")
                : regionCount > 0
                  ? t("settings:spending.module.region_count", { count: regionCount })
                  : undefined
            }
          />
          <StatRow
            label={t("settings:spending.module.stat_planned_vs_income")}
            value={plannedPct ?? "—"}
            unit={plannedPct !== null ? "%" : undefined}
            valueClassName={overPlan ? "text-warning" : undefined}
            sub={
              spendingPlanned > 0 || incomePlanned > 0
                ? `${fmt(spendingPlanned, currency)} / ${fmt(incomePlanned, currency)}`
                : t("settings:spending.module.setup_budget")
            }
          />
          <StatRow
            label={t("settings:spending.module.stat_budget_groups")}
            value={activeGroups.length}
            unit={t("settings:spending.module.stat_active")}
            sub={groupNamesSubline}
          />
        </div>
      )}

      {/* Categorization rules live at the bottom of the page — easy to scroll
          past. When tracking is on but no rules exist yet, link to them here. */}
      {enabled && !rulesUnavailable && ruleCount === 0 && (
        <Link to="/settings/spending/rules" className={LIST_ROW}>
          <span className="bg-foreground/[0.07] flex h-10 w-10 shrink-0 items-center justify-center rounded-full">
            <Icons.Sparkles className="h-[18px] w-[18px]" aria-hidden />
          </span>
          <span className="min-w-0 flex-1">
            <span className="block text-[17px] leading-snug">
              {t("settings:spending.module.setup_rules")}
            </span>
            <span className="text-muted-foreground block truncate text-[15px] leading-snug">
              {t("settings:spending.module.no_rules_prompt")}
            </span>
          </span>
        </Link>
      )}
    </section>
  );
}

interface StatRowProps {
  label: string;
  value: number | string;
  unit?: string;
  sub?: string;
  valueClassName?: string;
}

function StatRow({ label, value, unit, sub, valueClassName }: StatRowProps) {
  return (
    <div className="flex items-center justify-between gap-4 py-2.5">
      <div className="min-w-0">
        <div className="text-[17px] leading-snug">{label}</div>
        {sub && (
          <div className="text-muted-foreground truncate text-[15px] leading-snug">{sub}</div>
        )}
      </div>
      <div className="shrink-0 tabular-nums">
        <span className={cn("text-[19px]", valueClassName)}>{value}</span>
        {unit && <span className="text-muted-foreground ml-1 text-[15px]">{unit}</span>}
      </div>
    </div>
  );
}
