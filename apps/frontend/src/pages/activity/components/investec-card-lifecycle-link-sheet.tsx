import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { searchActivities } from "@/adapters";
import { ActivityStatus } from "@/lib/constants";
import { useSettingsContext } from "@/lib/settings-provider";
import type { Account, ActivityDetails } from "@/lib/types";
import { formatDateTime, resolveDisplayTimezone } from "@/lib/utils";
import {
  Button,
  formatAmount,
  Icons,
  ScrollArea,
  Sheet,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
  Skeleton,
} from "@wealthfolio/ui";
import { Textarea } from "@wealthfolio/ui/components/ui/textarea";
import { useActivityMutations } from "../hooks/use-activity-mutations";

interface InvestecCardLifecycleLinkSheetProps {
  open: boolean;
  sourceActivity?: ActivityDetails | null;
  accounts: Account[];
  onOpenChange: (open: boolean) => void;
  onComplete?: () => void | Promise<unknown>;
}

const SETTLEMENT_WINDOW_DAYS = 3;
const DEFAULT_REASON = "Online-order final amount adjustment";

function dateKey(date: Date | string, timezone?: string): string {
  if (typeof date === "string" && /^\d{4}-\d{2}-\d{2}$/.test(date.trim())) return date.trim();
  const instant = date instanceof Date ? date : new Date(date);
  if (!Number.isFinite(instant.getTime())) return "";
  const parts = new Intl.DateTimeFormat("en-CA", {
    timeZone: resolveDisplayTimezone(timezone),
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
  }).formatToParts(instant);
  const value = (type: Intl.DateTimeFormatPartTypes) =>
    parts.find((part) => part.type === type)?.value ?? "";
  return `${value("year")}-${value("month")}-${value("day")}`;
}

function dateWindow(date: Date | string, timezone?: string): { from: string; to: string } {
  const from = dateKey(date, timezone);
  if (!from) return { from: "", to: "" };
  const to = new Date(`${from}T12:00:00Z`);
  to.setUTCDate(to.getUTCDate() + SETTLEMENT_WINDOW_DAYS);
  return { from, to: to.toISOString().slice(0, 10) };
}

function dateDiffDays(left: Date | string, right: Date | string): number | undefined {
  const leftMs = new Date(left).getTime();
  const rightMs = new Date(right).getTime();
  if (!Number.isFinite(leftMs) || !Number.isFinite(rightMs)) return undefined;
  return Math.round((rightMs - leftMs) / (24 * 60 * 60 * 1000));
}

function normalizedMerchant(activity: ActivityDetails): string {
  return (activity.comment ?? "").trim().replace(/\s+/g, " ").toUpperCase();
}

function merchantMatches(source: ActivityDetails, candidate: ActivityDetails): boolean {
  const sourceMerchant = normalizedMerchant(source);
  const candidateMerchant = normalizedMerchant(candidate);
  if (sourceMerchant.length < 3 || !candidateMerchant) return false;
  return candidateMerchant === sourceMerchant || candidateMerchant.startsWith(`${sourceMerchant} `);
}

function isProviderActivity(activity: ActivityDetails): boolean {
  const transactionId = activity.sourceRecordId?.trim();
  if (activity.sourceSystem !== "FINANCE" || !transactionId) return false;
  const suffix = `:${transactionId}`;
  if (!activity.id.endsWith(suffix)) return false;
  const resourcePrefix = activity.id.slice(0, -suffix.length);
  const source = resourcePrefix.slice(resourcePrefix.lastIndexOf(":") + 1);
  return /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(source);
}

function isAlreadyLinked(activity: ActivityDetails): boolean {
  const metadata = activity.metadata;
  if (!metadata || typeof metadata !== "object" || Array.isArray(metadata)) return false;
  const providerOwned = metadata.providerOwned;
  if (!providerOwned || typeof providerOwned !== "object" || Array.isArray(providerOwned))
    return false;
  const correlation = (providerOwned as Record<string, unknown>).investecCorrelation;
  return Boolean(correlation && typeof correlation === "object" && !Array.isArray(correlation));
}

function amount(activity: ActivityDetails): number | undefined {
  if (activity.amount === null || activity.amount === undefined) return undefined;
  const value = Number(activity.amount);
  return Number.isFinite(value) ? Math.abs(value) : undefined;
}

function CandidateRow({
  activity,
  source,
  selected,
  onSelect,
}: {
  activity: ActivityDetails;
  source: ActivityDetails;
  selected: boolean;
  onSelect: () => void;
}) {
  const { t } = useTranslation();
  const candidateAmount = amount(activity);
  const sourceAmount = amount(source);
  const difference =
    candidateAmount !== undefined && sourceAmount !== undefined
      ? sourceAmount - candidateAmount
      : undefined;
  const date = formatDateTime(activity.date).date;
  const days = dateDiffDays(source.date, activity.date);

  return (
    <button
      type="button"
      className={`hover:bg-muted/50 flex w-full items-start gap-3 rounded-md border p-3 text-left transition-colors ${selected ? "border-primary bg-primary/5" : ""}`}
      onClick={onSelect}
      aria-pressed={selected}
    >
      {selected ? (
        <Icons.CheckCircle className="text-primary mt-0.5 h-4 w-4 shrink-0" />
      ) : (
        <Icons.Circle className="text-muted-foreground/30 mt-0.5 h-4 w-4 shrink-0" />
      )}
      <span className="min-w-0 flex-1">
        <span className="flex items-center justify-between gap-2 text-sm">
          <span className="truncate font-medium">
            {activity.comment || t("activity:date_list.cash")}
          </span>
          <span className="shrink-0 font-medium tabular-nums">
            {candidateAmount !== undefined
              ? formatAmount(candidateAmount, activity.currency)
              : activity.currency}
          </span>
        </span>
        <span className="text-muted-foreground mt-1 flex items-center justify-between gap-2 text-xs">
          <span>{date}</span>
          <span>
            {days !== undefined ? t("activity:lifecycle_link.days_after", { count: days }) : null}
          </span>
        </span>
        {difference !== undefined ? (
          <span className="text-muted-foreground mt-1 block text-xs">
            {t("activity:lifecycle_link.difference", {
              value: formatAmount(Math.abs(difference), source.currency),
            })}
          </span>
        ) : null}
        {!merchantMatches(source, activity) ? (
          <span className="mt-1 block text-xs text-amber-600">
            {t("activity:lifecycle_link.merchant_warning")}
          </span>
        ) : null}
      </span>
    </button>
  );
}

export function InvestecCardLifecycleLinkSheet({
  open,
  sourceActivity,
  accounts,
  onOpenChange,
  onComplete,
}: InvestecCardLifecycleLinkSheetProps) {
  const { t } = useTranslation();
  const { settings } = useSettingsContext();
  const { linkInvestecCardLifecycleMutation } = useActivityMutations();
  const [candidates, setCandidates] = useState<ActivityDetails[]>([]);
  const [selected, setSelected] = useState<ActivityDetails | null>(null);
  const [reason, setReason] = useState(DEFAULT_REASON);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const candidateRequestId = useRef(0);

  const accountName = useMemo(
    () => accounts.find((account) => account.id === sourceActivity?.accountId)?.name,
    [accounts, sourceActivity?.accountId],
  );

  const loadCandidates = useCallback(async () => {
    if (!sourceActivity) return;
    const requestId = ++candidateRequestId.current;
    const window = dateWindow(sourceActivity.date, settings?.timezone);
    if (!window.from || !window.to) {
      setError(t("activity:lifecycle_link.invalid_date"));
      return;
    }
    setLoading(true);
    setError(null);
    try {
      const pageSize = 100;
      const activities: ActivityDetails[] = [];
      for (let page = 0; ; page += 1) {
        const response = await searchActivities(
          page,
          pageSize,
          {
            accountIds: [sourceActivity.accountId],
            dateFrom: window.from,
            dateTo: window.to,
          },
          "",
          { id: "date", desc: false },
        );
        activities.push(...response.data);
        if (
          response.data.length === 0 ||
          activities.length >= response.meta.totalRowCount ||
          requestId !== candidateRequestId.current
        ) {
          break;
        }
      }
      if (requestId !== candidateRequestId.current) return;
      setCandidates(
        activities.filter(
          (activity) =>
            activity.id !== sourceActivity.id &&
            activity.status === ActivityStatus.POSTED &&
            activity.currency === sourceActivity.currency &&
            isProviderActivity(activity) &&
            !isAlreadyLinked(activity),
        ),
      );
    } catch (cause) {
      if (requestId !== candidateRequestId.current) return;
      setError(cause instanceof Error ? cause.message : t("activity:lifecycle_link.load_error"));
    } finally {
      if (requestId === candidateRequestId.current) setLoading(false);
    }
  }, [settings?.timezone, sourceActivity, t]);

  useEffect(() => {
    if (!open) {
      candidateRequestId.current += 1;
      return;
    }
    setCandidates([]);
    setSelected(null);
    setReason(DEFAULT_REASON);
    setError(null);
    void loadCandidates();
    return () => {
      candidateRequestId.current += 1;
    };
  }, [loadCandidates, open]);

  const confirm = useCallback(async () => {
    if (!sourceActivity || !selected || !reason.trim()) return;
    await linkInvestecCardLifecycleMutation.mutateAsync({
      pendingActivityId: sourceActivity.id,
      postedActivityId: selected.id,
      reason: reason.trim(),
    });
    await onComplete?.();
    onOpenChange(false);
  }, [
    linkInvestecCardLifecycleMutation,
    onComplete,
    onOpenChange,
    reason,
    selected,
    sourceActivity,
  ]);

  const sourceAmount = sourceActivity ? amount(sourceActivity) : undefined;
  const selectedAmount = selected ? amount(selected) : undefined;
  const difference =
    sourceAmount !== undefined && selectedAmount !== undefined
      ? sourceAmount - selectedAmount
      : undefined;

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent className="flex w-full flex-col overflow-hidden sm:max-w-[620px]">
        <SheetHeader>
          <SheetTitle>{t("activity:lifecycle_link.title")}</SheetTitle>
          <SheetDescription>
            {t("activity:lifecycle_link.description", {
              account: accountName ?? sourceActivity?.accountId ?? "",
            })}
          </SheetDescription>
        </SheetHeader>

        <div className="min-h-0 flex-1 space-y-4 overflow-y-auto py-4">
          {sourceActivity ? (
            <div className="bg-muted/30 rounded-md border p-3 text-sm">
              <div className="text-muted-foreground text-xs font-medium uppercase">
                {t("activity:lifecycle_link.authorization")}
              </div>
              <div className="mt-1 flex items-center justify-between gap-3">
                <span className="truncate font-medium">
                  {sourceActivity.comment || t("activity:date_list.cash")}
                </span>
                <span className="shrink-0 font-medium tabular-nums">
                  {sourceAmount !== undefined
                    ? formatAmount(sourceAmount, sourceActivity.currency)
                    : sourceActivity.currency}
                </span>
              </div>
              <div className="text-muted-foreground mt-1 text-xs">
                {formatDateTime(sourceActivity.date).date}
              </div>
            </div>
          ) : null}

          <div className="space-y-2">
            <div className="text-muted-foreground text-xs font-medium uppercase">
              {t("activity:lifecycle_link.posted_candidates")}
            </div>
            {loading ? (
              <div className="space-y-2">
                <Skeleton className="h-20 w-full rounded-md" />
                <Skeleton className="h-20 w-full rounded-md" />
              </div>
            ) : error ? (
              <div className="text-destructive rounded-md border p-3 text-sm">{error}</div>
            ) : candidates.length > 0 && sourceActivity ? (
              <ScrollArea className="max-h-72">
                <div className="space-y-2 pr-3">
                  {candidates.map((candidate) => (
                    <CandidateRow
                      key={candidate.id}
                      activity={candidate}
                      source={sourceActivity}
                      selected={selected?.id === candidate.id}
                      onSelect={() => setSelected(candidate)}
                    />
                  ))}
                </div>
              </ScrollArea>
            ) : (
              <div className="text-muted-foreground rounded-md border border-dashed p-4 text-center text-sm">
                {t("activity:lifecycle_link.no_candidates")}
              </div>
            )}
          </div>

          {selected ? (
            <div className="space-y-3 rounded-md border border-amber-500/50 bg-amber-500/5 p-3">
              <div className="flex items-start gap-2 text-sm">
                <Icons.AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-amber-600" />
                <span>{t("activity:lifecycle_link.confirm_warning")}</span>
              </div>
              {difference !== undefined ? (
                <div className="text-sm font-medium">
                  {t("activity:lifecycle_link.amount_summary", {
                    authorization: formatAmount(
                      sourceAmount ?? 0,
                      sourceActivity?.currency ?? selected.currency,
                    ),
                    posted: formatAmount(selectedAmount ?? 0, selected.currency),
                    difference: formatAmount(Math.abs(difference), selected.currency),
                  })}
                </div>
              ) : null}
              <Textarea
                value={reason}
                onChange={(event) => setReason(event.target.value)}
                maxLength={2048}
                placeholder={t("activity:lifecycle_link.reason_placeholder")}
                aria-label={t("activity:lifecycle_link.reason")}
                rows={3}
              />
            </div>
          ) : null}
        </div>

        <SheetFooter>
          <Button
            variant="outline"
            onClick={() => onOpenChange(false)}
            disabled={linkInvestecCardLifecycleMutation.isPending}
          >
            {t("common:cancel")}
          </Button>
          <Button
            onClick={() => void confirm()}
            disabled={!selected || !reason.trim() || linkInvestecCardLifecycleMutation.isPending}
          >
            {linkInvestecCardLifecycleMutation.isPending ? (
              <Icons.Spinner className="mr-2 h-4 w-4 animate-spin" />
            ) : (
              <Icons.Link className="mr-2 h-4 w-4" />
            )}
            {t("activity:lifecycle_link.confirm")}
          </Button>
        </SheetFooter>
      </SheetContent>
    </Sheet>
  );
}
