import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import {
  Button,
  Icons,
  Input,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@wealthfolio/ui";

import {
  useSpendingSettings,
  useSpendingSettingsMutation,
} from "@/features/spending/hooks/use-spending-settings";
import { budgetPeriodLabel, currentBudgetPeriodKey } from "@/features/spending/lib/month-period";
import type { BudgetCadence, BudgetCycleSettings } from "@/features/spending/types";
import { useSettingsContext } from "@/lib/settings-provider";

function isValidAnchorDate(value: string): boolean {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (!match) return false;
  const year = Number(match[1]);
  const month = Number(match[2]);
  const day = Number(match[3]);
  const date = new Date(Date.UTC(year, month - 1, day));
  return (
    date.getUTCFullYear() === year && date.getUTCMonth() + 1 === month && date.getUTCDate() === day
  );
}

export function BudgetCycleCard() {
  const { t } = useTranslation();
  const { budgetCycle, isLoading } = useSpendingSettings();
  const updateSettings = useSpendingSettingsMutation();
  const { settings } = useSettingsContext();
  const [draft, setDraft] = useState<BudgetCycleSettings>(budgetCycle);

  useEffect(() => setDraft(budgetCycle), [budgetCycle]);

  const currentPeriod = useMemo(
    () => currentBudgetPeriodKey(settings?.timezone, draft),
    [draft, settings?.timezone],
  );
  const dirty =
    draft.cadence !== budgetCycle.cadence || draft.anchorDate !== budgetCycle.anchorDate;
  const validDate = isValidAnchorDate(draft.anchorDate);

  if (isLoading) return <div className="bg-muted/40 h-44 animate-pulse rounded-lg" />;

  return (
    <section id="budget-cycle" className="scroll-mt-6">
      <div className="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <Icons.Calendar className="text-muted-foreground h-4 w-4" aria-hidden />
            <h3 className="text-base font-medium">{t("settings:spending.budget_cycle.title")}</h3>
          </div>
          <p className="text-muted-foreground mt-1 max-w-[64ch] text-xs leading-relaxed">
            {t("settings:spending.budget_cycle.description")}
          </p>
        </div>
        <Button
          type="button"
          size="sm"
          className="shrink-0"
          disabled={!dirty || !validDate || updateSettings.isPending}
          onClick={() => updateSettings.mutate({ budgetCycle: draft })}
        >
          {updateSettings.isPending
            ? t("settings:spending.budget_cycle.saving")
            : t("settings:spending.budget_cycle.save")}
        </Button>
      </div>

      <div className="mt-4 grid gap-4 sm:grid-cols-2">
        <label className="space-y-1.5">
          <span className="text-foreground text-xs font-medium">
            {t("settings:spending.budget_cycle.frequency")}
          </span>
          <Select
            value={draft.cadence}
            onValueChange={(cadence) =>
              setDraft((current) => ({ ...current, cadence: cadence as BudgetCadence }))
            }
          >
            <SelectTrigger
              className="w-full"
              aria-label={t("settings:spending.budget_cycle.frequency")}
            >
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="weekly">{t("settings:spending.budget_cycle.weekly")}</SelectItem>
              <SelectItem value="biweekly">
                {t("settings:spending.budget_cycle.biweekly")}
              </SelectItem>
              <SelectItem value="monthly">{t("settings:spending.budget_cycle.monthly")}</SelectItem>
            </SelectContent>
          </Select>
        </label>

        <label className="space-y-1.5">
          <span className="text-foreground text-xs font-medium">
            {t("settings:spending.budget_cycle.anchor_date")}
          </span>
          <Input
            type="date"
            value={draft.anchorDate}
            onChange={(event) =>
              setDraft((current) => ({ ...current, anchorDate: event.target.value }))
            }
          />
        </label>
      </div>

      <div className="mt-4 text-[15px]">
        <div className="text-foreground font-medium">
          {t("settings:spending.budget_cycle.current_period")}
        </div>
        <div className="text-muted-foreground mt-0.5">
          {budgetPeriodLabel(currentPeriod, "long", undefined, draft)}
        </div>
      </div>

      <p className="text-muted-foreground mt-3 text-[11px] leading-relaxed">
        {t("settings:spending.budget_cycle.history_note")}
      </p>
    </section>
  );
}
