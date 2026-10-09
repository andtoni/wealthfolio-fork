import { createPortal } from "react-dom";
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
  /** Leaves select mode. */
  onDone: () => void;
}

export function TransactionsBulkBar({
  selectedCount,
  categoryScope,
  budgetPeriods,
  onCategorize,
  onTagEvent,
  onSetBudgetPeriod,
  onDelete,
  onDone,
}: TransactionsBulkBarProps) {
  const { t } = useTranslation();
  const none = selectedCount === 0;
  // Fixed to the screen, so the actions stay in reach however far the list is scrolled.
  return createPortal(
    <div
      role="region"
      aria-label={t("spending:transactions.bulkActions")}
      className="bg-card fixed inset-x-3 bottom-[calc(var(--mobile-nav-total-offset)+0.5rem)] z-40 mx-auto flex max-w-2xl items-center justify-between gap-2 rounded-2xl py-2 pl-4 pr-2 shadow-lg md:bottom-6"
    >
      <span className="truncate text-[15px] tabular-nums">
        {t("spending:transactions.selectedCount", { count: selectedCount })}
      </span>
      <div className="flex shrink-0 items-center gap-1">
        {categoryScope ? (
          <QuickCategorizePopover
            align="end"
            scope={categoryScope}
            onSelect={onCategorize}
            trigger={
              <Button size="sm" variant="ghost" aria-label={t("spending:transactions.categorize")}>
                <Icons.Tag className="h-[18px] w-[18px] sm:mr-1.5" aria-hidden="true" />
                <span className="hidden sm:inline">{t("spending:transactions.categorize")}</span>
              </Button>
            }
          />
        ) : (
          <Button
            size="sm"
            variant="ghost"
            disabled
            title={t("spending:transactions.categorizeHint")}
            aria-label={t("spending:transactions.categorize")}
          >
            <Icons.Tag className="h-[18px] w-[18px] sm:mr-1.5" aria-hidden="true" />
            <span className="hidden sm:inline">{t("spending:transactions.categorize")}</span>
          </Button>
        )}
        <QuickEventPopover
          align="end"
          onSelect={(eventId) => onTagEvent(eventId)}
          onClear={() => onTagEvent(null)}
          trigger={
            <Button
              size="sm"
              variant="ghost"
              disabled={none}
              aria-label={t("spending:transactions.tagEvent")}
            >
              <Icons.Calendar className="h-[18px] w-[18px] sm:mr-1.5" aria-hidden="true" />
              <span className="hidden sm:inline">{t("spending:transactions.tagEvent")}</span>
            </Button>
          }
        />
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              size="sm"
              variant="ghost"
              disabled={none}
              aria-label={t("spending:transactions.budgetPeriod")}
            >
              <Icons.PieChart className="h-[18px] w-[18px] sm:mr-1.5" aria-hidden="true" />
              <span className="hidden sm:inline">{t("spending:transactions.budgetPeriod")}</span>
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
          variant="ghost"
          className="text-destructive hover:bg-destructive/10"
          disabled={none}
          onClick={onDelete}
          aria-label={t("common:delete")}
        >
          <Icons.Trash className="h-[18px] w-[18px] sm:mr-1.5" aria-hidden="true" />
          <span className="hidden sm:inline">{t("common:delete")}</span>
        </Button>
        <Button size="sm" variant="secondary" onClick={onDone}>
          {t("spending:filters.done")}
        </Button>
      </div>
    </div>,
    document.body,
  );
}
