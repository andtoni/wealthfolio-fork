import { memo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import {
  Checkbox,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Icons,
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@wealthfolio/ui";
import type { Account } from "@/lib/types";
import { getOriginalCurrencyDisplay, isProvisionalCardSpending } from "@/lib/original-currency";
import { useInvestecCardLifecycleLinkEnabled } from "@/lib/provider-features";
import { cn, formatDateTime } from "@/lib/utils";

import { BudgetPeriodBadge } from "./budget-period-badge";
import { LIST_ROW } from "@/components/dashboard-card";
import { MerchantAvatar } from "./merchant-avatar";
import { SplitAmount } from "./split-amount";
import { QuickCategorizePopover } from "./quick-categorize-popover";
import { QuickEventPopover } from "./quick-event-popover";
import { SuggestedCategoryButton, type SuggestedCategory } from "./suggested-category-button";
import {
  getActivityTypesForAccount,
  getCashActivityLabel,
  getEffectiveCashActivityType,
  isCreditCardAccountType,
  type CashActivityType,
} from "../lib/constants";
import {
  getTransactionDisplay,
  getTransferLinkStatus,
  isTransferCashActivity,
  type TransactionRowVM,
} from "../lib/transactions-helpers";

interface TransactionCardProps {
  row: TransactionRowVM;
  account: Account | undefined;
  event: { id: string; name: string; eventTypeId: string } | null;
  eventTypeColor: string | null;
  appTimezone?: string;
  isSelected: boolean;
  /** Checkboxes show, and a tap selects, only while the list is in select mode. */
  selectMode: boolean;
  /** Details open from the bottom on a phone and from the side on a computer. */
  isMobile: boolean;
  suggestedCategory?: SuggestedCategory | null;
  onToggleSelect: (id: string) => void;
  /** Press and hold: puts the list in select mode with this row selected. */
  onHoldSelect: (id: string) => void;
  onAssignCategory: (activityId: string, taxonomyId: string, categoryId: string) => void;
  onClearCategory: (activityId: string, taxonomyId: string) => void;
  onSetEvent: (activityId: string, eventId: string | null) => void;
  onMarkReimbursement: (row: TransactionRowVM) => void;
  onEditSplits: (row: TransactionRowVM) => void;
  onEdit: (row: TransactionRowVM) => void;
  onChangeType: (row: TransactionRowVM, type: CashActivityType) => void;
  onDuplicate: (row: TransactionRowVM) => void;
  onDelete: (row: TransactionRowVM) => void;
  onLinkTransfer?: (row: TransactionRowVM) => void;
  onUnlinkTransfer?: (row: TransactionRowVM) => void;
  onLinkInvestecCardLifecycle?: (row: TransactionRowVM) => void;
}

const SHEET_ROW = "flex min-h-11 items-center justify-between gap-3 py-1.5 text-[15px]";
const SHEET_ACTION =
  "hover:bg-foreground/5 -mx-2 flex min-h-11 w-[calc(100%+1rem)] items-center gap-3 rounded-xl px-2 text-left text-[15px]";

function TransactionCardImpl({
  row,
  account,
  event,
  eventTypeColor,
  appTimezone,
  isSelected,
  selectMode,
  isMobile,
  suggestedCategory,
  onToggleSelect,
  onHoldSelect,
  onAssignCategory,
  onClearCategory,
  onSetEvent,
  onMarkReimbursement,
  onEditSplits,
  onEdit,
  onChangeType,
  onDuplicate,
  onDelete,
  onLinkTransfer,
  onUnlinkTransfer,
  onLinkInvestecCardLifecycle,
}: TransactionCardProps) {
  const { t } = useTranslation();
  const [detailOpen, setDetailOpen] = useState(false);
  const holdTimer = useRef<number | undefined>(undefined);
  // True from the moment a hold selects the row until the click that follows it.
  const held = useRef(false);
  const cancelHold = () => window.clearTimeout(holdTimer.current);
  const investecCardLifecycleLinkEnabled = useInvestecCardLifecycleLinkEnabled();
  const a = row.activity;
  const originalCurrency = getOriginalCurrencyDisplay(a);
  const isProvisional = isProvisionalCardSpending(a);
  const { isOutflow, isIncome, isSaving, isNeutral, sign, safeAmount } = getTransactionDisplay(
    a,
    account?.accountType,
  );
  const accountName = account?.name ?? a.accountId;
  const activityType = getEffectiveCashActivityType(a);
  // The types this account allows, plus the current one if it is not among them.
  const allowedTypes = getActivityTypesForAccount(account?.accountType);
  const typeOptions = allowedTypes.includes(activityType as CashActivityType)
    ? allowedTypes
    : [...allowedTypes, activityType as CashActivityType];
  const typeLabel = getCashActivityLabel(
    activityType,
    account?.accountType,
    a.subtype,
    a.sourceType,
    a.activityTypeOverride,
  );
  const isTransfer = isTransferCashActivity(a);
  const transferLinkStatus = getTransferLinkStatus(a);
  const canMarkReimbursement =
    isIncome && !isCreditCardAccountType(account?.accountType) && activityType !== "CREDIT";
  const canLinkInvestecCardLifecycle =
    investecCardLifecycleLinkEnabled &&
    a.status === "PENDING" &&
    a.sourceSystem === "FINANCE" &&
    a.sourceRecordId?.startsWith("investec-card:") === true &&
    onLinkInvestecCardLifecycle !== undefined;
  const formattedDate = formatDateTime(a.activityDate, appTimezone, a.activityDatePrecision);
  const suggestion = !row.category && !isNeutral && row.splitCount === 0 ? suggestedCategory : null;
  const acceptSuggestion = () =>
    suggestion && onAssignCategory(a.id, suggestion.taxonomyId, suggestion.id);
  const amountTone = isNeutral
    ? "text-muted-foreground"
    : isIncome || isSaving
      ? "text-success"
      : "text-foreground";
  const act = (run: () => void) => () => {
    setDetailOpen(false);
    run();
  };

  return (
    <>
      <div
        role="button"
        tabIndex={0}
        data-state={isSelected ? "selected" : undefined}
        className={cn(LIST_ROW, "select-none [-webkit-touch-callout:none]")}
        onPointerDown={(e) => {
          held.current = false;
          if (selectMode || e.button !== 0) return;
          holdTimer.current = window.setTimeout(() => {
            held.current = true;
            onHoldSelect(a.id);
          }, 450);
        }}
        onPointerUp={cancelHold}
        onPointerLeave={cancelHold}
        onPointerCancel={cancelHold}
        onContextMenu={(e) => held.current && e.preventDefault()}
        onClick={() => {
          if (held.current) held.current = false;
          else if (selectMode) onToggleSelect(a.id);
          else setDetailOpen(true);
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            if (selectMode) onToggleSelect(a.id);
            else setDetailOpen(true);
          }
        }}
      >
        {selectMode && (
          <Checkbox
            checked={isSelected}
            aria-label={
              isSelected ? t("spending:transactions.deselect") : t("spending:transactions.select")
            }
            className="pointer-events-none shrink-0"
          />
        )}
        <MerchantAvatar
          name={a.notes ?? null}
          currency={a.currency}
          logo={isOutflow && !isNeutral}
          tint={row.category?.color}
        />
        <div className="min-w-0 flex-1">
          <div className="text-foreground truncate text-[17px] leading-snug">
            {a.notes ?? <span className="text-muted-foreground">—</span>}
          </div>
          <div className="text-muted-foreground flex min-w-0 items-center gap-2 text-[15px] leading-snug">
            {isNeutral ? (
              <span className="truncate">{typeLabel}</span>
            ) : row.splitCount > 0 ? (
              <span className="truncate">
                {t("spending:transactions.splitLines", { count: row.splitCount })}
              </span>
            ) : row.category ? (
              <span className="truncate">{row.category.name}</span>
            ) : suggestion ? (
              <span
                className="min-w-0 shrink-0"
                onClick={(e) => e.stopPropagation()}
                onKeyDown={(e) => e.stopPropagation()}
                role="presentation"
              >
                <SuggestedCategoryButton
                  category={suggestion}
                  onAccept={acceptSuggestion}
                  className="text-foreground inline-flex max-w-full items-center gap-1.5"
                />
              </span>
            ) : (
              <span className="truncate text-amber-600 dark:text-amber-500">
                {t("spending:dashboard.uncategorized")}
              </span>
            )}
          </div>
          {(isProvisional || row.needsReview) && (
            <div className="mt-1 flex gap-1.5">
              {isProvisional && (
                <span className="bg-foreground/[0.07] text-muted-foreground rounded-md px-1.5 py-0.5 text-xs">
                  {t("spending:transactions.provisional")}
                </span>
              )}
              {row.needsReview && (
                <span className="rounded-md bg-amber-500/15 px-1.5 py-0.5 text-xs text-amber-700 dark:text-amber-400">
                  {t("spending:transactions.review")}
                </span>
              )}
            </div>
          )}
        </div>
        <div className={cn("shrink-0 text-right text-[19px]", amountTone)}>
          <SplitAmount value={safeAmount} currency={a.currency} sign={sign} />
          {originalCurrency && (
            <div className="text-muted-foreground text-xs leading-tight">
              {originalCurrency.label}
            </div>
          )}
        </div>
      </div>

      <Sheet open={detailOpen} onOpenChange={setDetailOpen}>
        <SheetContent
          side={isMobile ? "bottom" : "right"}
          className={cn("overflow-y-auto", isMobile && "pb-safe max-h-[88vh] rounded-t-xl")}
        >
          <SheetHeader className="pr-10 text-left">
            <SheetTitle className="truncate text-base font-medium">{a.notes ?? "—"}</SheetTitle>
            <SheetDescription className="sr-only">{accountName}</SheetDescription>
          </SheetHeader>
          <div className={cn("mt-3 text-[32px] font-medium", amountTone)}>
            <SplitAmount value={safeAmount} currency={a.currency} sign={sign} />
          </div>
          {originalCurrency && (
            <div className="text-muted-foreground text-xs">
              {t("spending:transactions.original_amount", { value: originalCurrency.label })}
            </div>
          )}

          <div className="mt-4">
            <div className={SHEET_ROW}>
              <span className="text-muted-foreground text-[15px]">{t("activity:table_date")}</span>
              <span className="text-[15px]">
                {formattedDate.date} {formattedDate.time}
              </span>
            </div>
            <div className={SHEET_ROW}>
              <span className="text-muted-foreground text-[15px]">
                {t("activity:table_account")}
              </span>
              <span className="truncate text-[15px]">{accountName}</span>
            </div>
            <div className={SHEET_ROW}>
              <span className="text-muted-foreground text-[15px]">{t("activity:table_type")}</span>
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <button type="button" className="inline-flex items-center gap-1.5 text-[15px]">
                    {typeLabel}
                    <Icons.ChevronRight
                      className="text-muted-foreground h-4 w-4 shrink-0"
                      aria-hidden="true"
                    />
                  </button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end">
                  {typeOptions.map((type) => (
                    <DropdownMenuItem
                      key={type}
                      onSelect={() => type !== activityType && onChangeType(row, type)}
                    >
                      <span className="flex-1">
                        {getCashActivityLabel(
                          type,
                          account?.accountType,
                          type === "CREDIT" && !isCreditCardAccountType(account?.accountType)
                            ? "REIMBURSEMENT"
                            : undefined,
                        )}
                      </span>
                      {type === activityType && (
                        <Icons.Check className="ml-3 h-4 w-4" aria-hidden="true" />
                      )}
                    </DropdownMenuItem>
                  ))}
                </DropdownMenuContent>
              </DropdownMenu>
            </div>
            {!isNeutral && (
              <div className={SHEET_ROW}>
                <span className="text-muted-foreground text-[15px]">
                  {t("spending:filters.category")}
                </span>
                {row.splitCount > 0 ? (
                  <button
                    type="button"
                    className="text-[15px]"
                    onClick={act(() => onEditSplits(row))}
                  >
                    {t("spending:transactions.splitLines", { count: row.splitCount })}
                  </button>
                ) : (
                  <span className="flex min-w-0 items-center gap-3">
                    {suggestion && (
                      <SuggestedCategoryButton
                        category={suggestion}
                        onAccept={acceptSuggestion}
                        className="inline-flex min-w-0 items-center gap-1.5 text-[15px]"
                      />
                    )}
                    <QuickCategorizePopover
                      scope={isIncome ? "income" : isSaving ? "saving" : "expense"}
                      selectedCategoryId={row.category?.id ?? null}
                      onSelect={(taxonomyId, categoryId) =>
                        onAssignCategory(a.id, taxonomyId, categoryId)
                      }
                      onClear={() => row.category && onClearCategory(a.id, row.category.taxonomyId)}
                      align="end"
                      trigger={
                        <button
                          type="button"
                          aria-label={
                            row.category
                              ? t("spending:transactions.changeCategory", {
                                  name: row.category.name,
                                })
                              : t("spending:transactions.assignCategory")
                          }
                          className="inline-flex min-w-0 items-center gap-1.5 text-[15px]"
                        >
                          {row.category ? (
                            <>
                              {row.category.color && (
                                <span
                                  className="h-2.5 w-2.5 shrink-0 rounded-full"
                                  style={{ backgroundColor: row.category.color }}
                                  aria-hidden="true"
                                />
                              )}
                              <span className="truncate">{row.category.name}</span>
                            </>
                          ) : (
                            <span className="text-muted-foreground">
                              {t("spending:transactions.categorize")}
                            </span>
                          )}
                          <Icons.ChevronRight
                            className="text-muted-foreground h-4 w-4 shrink-0"
                            aria-hidden="true"
                          />
                        </button>
                      }
                    />
                  </span>
                )}
              </div>
            )}
            <div className={SHEET_ROW}>
              <span className="text-muted-foreground text-[15px]">
                {t("spending:filters.event")}
              </span>
              <QuickEventPopover
                selectedEventId={event?.id ?? null}
                onSelect={(eventId) => onSetEvent(a.id, eventId)}
                onClear={() => onSetEvent(a.id, null)}
                activityId={a.id}
                defaultDate={a.activityDate ? new Date(a.activityDate) : undefined}
                trigger={
                  <button
                    type="button"
                    aria-label={
                      event
                        ? t("spending:transactions.changeEvent", { name: event.name })
                        : t("spending:transactions.tagEvent")
                    }
                    className="inline-flex min-w-0 items-center gap-1.5 text-[15px]"
                  >
                    {event ? (
                      <>
                        <span
                          className="h-2 w-2 shrink-0 rounded-full"
                          style={{ backgroundColor: eventTypeColor ?? "var(--muted-foreground)" }}
                          aria-hidden="true"
                        />
                        <span className="truncate">{event.name}</span>
                      </>
                    ) : (
                      <span className="text-muted-foreground">
                        {t("spending:transactions.tagEvent")}
                      </span>
                    )}
                    <Icons.ChevronRight
                      className="text-muted-foreground h-4 w-4 shrink-0"
                      aria-hidden="true"
                    />
                  </button>
                }
              />
            </div>
            {a.budgetPeriodStart && (
              <div className={SHEET_ROW}>
                <span className="text-muted-foreground text-[15px]">
                  {t("spending:transactions.budgetPeriod")}
                </span>
                <BudgetPeriodBadge periodStart={a.budgetPeriodStart} />
              </div>
            )}
          </div>

          <div className="mt-3">
            <button type="button" className={SHEET_ACTION} onClick={act(() => onEdit(row))}>
              {t("common:edit")}
            </button>
            {!isNeutral && (
              <button type="button" className={SHEET_ACTION} onClick={act(() => onEditSplits(row))}>
                {t("spending:transactions.splitTransaction")}
              </button>
            )}
            {canMarkReimbursement && (
              <button
                type="button"
                className={SHEET_ACTION}
                onClick={act(() => onMarkReimbursement(row))}
              >
                {t("spending:transactions.markReimbursement")}
              </button>
            )}
            <button type="button" className={SHEET_ACTION} onClick={act(() => onDuplicate(row))}>
              {t("spending:transactions.duplicate")}
            </button>
            {isTransfer && transferLinkStatus === "linked" && onUnlinkTransfer && (
              <button
                type="button"
                className={SHEET_ACTION}
                onClick={act(() => onUnlinkTransfer(row))}
              >
                {t("spending:transactions.unlinkTransfer")}
              </button>
            )}
            {isTransfer && transferLinkStatus !== "linked" && onLinkTransfer && (
              <button
                type="button"
                className={SHEET_ACTION}
                onClick={act(() => onLinkTransfer(row))}
              >
                {t("spending:transactions.linkTransfer")}
              </button>
            )}
            {canLinkInvestecCardLifecycle && (
              <button
                type="button"
                className={SHEET_ACTION}
                onClick={act(() => onLinkInvestecCardLifecycle?.(row))}
              >
                {t("activity:operations.link_card_lifecycle")}
              </button>
            )}
            <button
              type="button"
              className={cn(SHEET_ACTION, "text-destructive")}
              onClick={act(() => onDelete(row))}
            >
              {t("common:delete")}
            </button>
          </div>
        </SheetContent>
      </Sheet>
    </>
  );
}

export const TransactionCard = memo(TransactionCardImpl);
