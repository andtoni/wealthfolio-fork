import { useTranslation } from "react-i18next";

import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  Icons,
} from "@wealthfolio/ui";

import { QuickCategorizePopover, type QuickCategorizeScope } from "./quick-categorize-popover";
import { QuickEventPopover } from "./quick-event-popover";

export interface BudgetPeriodOption {
  /** Period start date (`YYYY-MM-DD`). */
  key: string;
  label: string;
  isCurrent: boolean;
}

interface TransactionsBulkBarProps {
  selectedCount: number;
  categoryScope: QuickCategorizeScope | null;
  budgetPeriods: BudgetPeriodOption[];
  onCategorize: (taxonomyId: string, categoryId: string) => void;
  onTagEvent: (eventId: string | null) => void;
  /** `null` counts the transactions in the period of their own date again. */
  onSetBudgetPeriod: (periodStart: string | null) => void;
  onDelete: () => void;
  onClearSelection: () => void;
}

export function TransactionsBulkBar({
  selectedCount,
  categoryScope,
  budgetPeriods,
  onCategorize,
  onTagEvent,
  onSetBudgetPeriod,
  onDelete,
  onClearSelection,
}: TransactionsBulkBarProps) {
  const { t } = useTranslation();
  return (
    <div
      role="region"
      aria-label={t("spending:transactions.bulkActions")}
      className="bg-muted/40 ring-border flex flex-wrap items-center justify-between gap-2 rounded-md px-3 py-2 ring-1"
    >
      <div className="text-foreground flex items-center gap-2 text-sm">
        <Icons.Check className="h-4 w-4" aria-hidden="true" />
        <span className="font-medium">
          {t("spending:transactions.selectedCount", { count: selectedCount })}
        </span>
      </div>
      <div className="flex flex-wrap items-center gap-2">
        {categoryScope ? (
          <QuickCategorizePopover
            align="end"
            scope={categoryScope}
            onSelect={onCategorize}
            trigger={
              <Button size="sm" variant="default">
                <Icons.Tag className="mr-1.5 h-3.5 w-3.5" aria-hidden="true" />
                {t("spending:transactions.categorize")}
              </Button>
            }
          />
        ) : (
          <Button
            size="sm"
            variant="default"
            disabled
            title={t("spending:transactions.categorizeHint")}
          >
            <Icons.Tag className="mr-1.5 h-3.5 w-3.5" aria-hidden="true" />
            {t("spending:transactions.categorize")}
          </Button>
        )}
        <QuickEventPopover
          align="end"
          onSelect={(eventId) => onTagEvent(eventId)}
          onClear={() => onTagEvent(null)}
          trigger={
            <Button size="sm" variant="outline">
              <Icons.Calendar className="mr-1.5 h-3.5 w-3.5" aria-hidden="true" />
              {t("spending:transactions.tagEvent")}
            </Button>
          }
        />
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button size="sm" variant="outline">
              <Icons.PieChart className="mr-1.5 h-3.5 w-3.5" aria-hidden="true" />
              {t("spending:transactions.budgetPeriod")}
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuLabel>{t("spending:transactions.countInBudget")}</DropdownMenuLabel>
            {budgetPeriods.map((period) => (
              <DropdownMenuItem key={period.key} onSelect={() => onSetBudgetPeriod(period.key)}>
                {period.isCurrent
                  ? t("spending:transactions.currentBudgetPeriod", { period: period.label })
                  : period.label}
              </DropdownMenuItem>
            ))}
            <DropdownMenuSeparator />
            <DropdownMenuItem onSelect={() => onSetBudgetPeriod(null)}>
              {t("spending:transactions.useTransactionDate")}
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
        <Button
          size="sm"
          variant="outline"
          className="text-destructive hover:bg-destructive/10"
          onClick={onDelete}
        >
          <Icons.Trash className="mr-1.5 h-3.5 w-3.5" aria-hidden="true" />
          {t("common:delete")}
        </Button>
        <Button size="sm" variant="ghost" onClick={onClearSelection}>
          {t("common:clear")}
        </Button>
      </div>
    </div>
  );
}
