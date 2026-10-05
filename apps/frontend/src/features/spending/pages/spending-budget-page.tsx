import { useCallback } from "react";
import { Link, Navigate, useNavigate, useSearchParams } from "react-router-dom";
import { useTranslation } from "react-i18next";

import { Button, Icons, Page, PageContent, PageHeader } from "@wealthfolio/ui";

import { useSettingsContext } from "@/lib/settings-provider";

import { BudgetEditor } from "../components/budget-editor";
import { useSpendingSettings } from "../hooks/use-spending-settings";
import {
  budgetPeriodKeyForDate,
  budgetPeriodLabel,
  budgetPeriodParts,
  currentBudgetPeriodKey,
  shiftBudgetPeriodKey,
} from "../lib/month-period";
import type { BudgetCycleSettings } from "../types";

function isValidPeriodKey(s: string | null, cycle: BudgetCycleSettings): s is string {
  return !!s && budgetPeriodParts(s, cycle) !== null;
}

function legacyMonthToPeriodKey(
  monthKey: string | null,
  cycle: BudgetCycleSettings,
): string | null {
  const match = /^(\d{4})-(\d{2})$/.exec(monthKey ?? "");
  if (!match) return null;
  const year = Number(match[1]);
  const month = Number(match[2]);
  if (month < 1 || month > 12) return null;
  return budgetPeriodKeyForDate({ year, month, day: 1 }, cycle);
}

export default function SpendingBudgetPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [searchParams, setSearchParams] = useSearchParams();
  const { isEnabled, isLoading: settingsLoading, budgetCycle } = useSpendingSettings();
  const { settings } = useSettingsContext();
  const currentPeriodKey = currentBudgetPeriodKey(settings?.timezone, budgetCycle);
  const urlPeriod = searchParams.get("period");
  const legacyPeriod = legacyMonthToPeriodKey(searchParams.get("month"), budgetCycle);
  const periodKey = isValidPeriodKey(urlPeriod, budgetCycle)
    ? urlPeriod
    : (legacyPeriod ?? currentPeriodKey);
  const setPeriodKey = useCallback(
    (next: string) => {
      setSearchParams(
        (prev) => {
          const p = new URLSearchParams(prev);
          p.set("period", next);
          return p;
        },
        { replace: true },
      );
    },
    [setSearchParams],
  );

  if (!settingsLoading && !isEnabled) {
    return <Navigate to="/dashboard?tab=spending" replace />;
  }

  const headerActions = (
    <div className="flex w-full min-w-0 max-w-full flex-wrap items-center gap-2 sm:w-auto sm:justify-end">
      <PeriodSwitcher
        periodKey={periodKey}
        currentPeriodKey={currentPeriodKey}
        cycle={budgetCycle}
        onChange={setPeriodKey}
      />
      <Button asChild variant="ghost" size="sm" className="text-muted-foreground">
        <Link to="/settings/spending#budget-cycle">
          <Icons.Settings className="mr-1.5 h-3.5 w-3.5" />
          {t("spending:budgetPage.cycleSettings")}
        </Link>
      </Button>
    </div>
  );

  return (
    <Page className="overflow-x-hidden">
      <PageHeader
        heading={t("spending:budgetPage.heading")}
        text={t("spending:budgetPage.subheading")}
        onBack={() => {
          if (window.history.length > 1) navigate(-1);
          else navigate("/dashboard?tab=spending");
        }}
        actions={headerActions}
      />
      <PageContent className="min-w-0 max-w-full space-y-5 overflow-x-hidden">
        {settingsLoading ? null : <BudgetEditor mode="period" periodKey={periodKey} />}
      </PageContent>
    </Page>
  );
}

function PeriodSwitcher({
  periodKey,
  currentPeriodKey,
  cycle,
  onChange,
}: {
  periodKey: string;
  currentPeriodKey: string;
  cycle: BudgetCycleSettings;
  onChange: (next: string) => void;
}) {
  const { t } = useTranslation();
  const shift = (delta: number) => onChange(shiftBudgetPeriodKey(periodKey, delta, cycle));
  const selectDate = (value: string) => {
    const [year, month, day] = value.split("-").map(Number);
    if (!year || !month || !day) return;
    onChange(budgetPeriodKeyForDate({ year, month, day }, cycle));
  };

  return (
    <div className="flex min-w-0 max-w-full items-center gap-1">
      <Button
        type="button"
        variant="ghost"
        size="sm"
        className="h-8 w-8 p-0"
        aria-label={t("spending:calendar.previousPeriod")}
        onClick={() => shift(-1)}
      >
        <Icons.ChevronLeft className="h-4 w-4" />
      </Button>
      {/* The period itself is the control: tapping it opens the date picker. */}
      <label className="relative cursor-pointer text-sm">
        <span>{budgetPeriodLabel(periodKey, "short", undefined, cycle)}</span>
        <input
          type="date"
          value={periodKey}
          onChange={(event) => selectDate(event.target.value)}
          className="absolute inset-0 h-full w-full cursor-pointer opacity-0"
          aria-label={t("spending:calendar.choosePeriodDate")}
        />
      </label>
      <Button
        type="button"
        variant="ghost"
        size="sm"
        className="h-8 w-8 p-0"
        aria-label={t("spending:calendar.nextPeriod")}
        onClick={() => shift(1)}
      >
        <Icons.ChevronRight className="h-4 w-4" />
      </Button>
      {periodKey !== currentPeriodKey && (
        <button
          type="button"
          className="text-muted-foreground hover:text-foreground px-1 text-sm"
          onClick={() => onChange(currentPeriodKey)}
        >
          {t("spending:period.currentPeriod")}
        </button>
      )}
    </div>
  );
}
