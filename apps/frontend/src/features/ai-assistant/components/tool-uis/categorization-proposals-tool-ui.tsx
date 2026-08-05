import { updateToolResult } from "@/adapters";
import { useBulkAssignCategories } from "@/features/spending/hooks/use-cash-activities";
import { QuickCategorizePopover } from "@/features/spending/components/quick-categorize-popover";
import { useBalancePrivacy } from "@/hooks/use-balance-privacy";
import { useSettingsContext } from "@/lib/settings-provider";
import { cn } from "@/lib/utils";
import type { ToolCallMessagePartProps } from "@assistant-ui/react";
import { makeAssistantToolUI } from "@assistant-ui/react";
import {
  Alert,
  AlertDescription,
  AlertTitle,
  Badge,
  Button,
  Card,
  CardContent,
  CardHeader,
  CardTitle,
  Checkbox,
} from "@wealthfolio/ui";
import { Icons } from "@wealthfolio/ui/components/ui/icons";
import { memo, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useRuntimeContext } from "../../hooks/use-runtime-context";
import {
  categoryMatchesCashFlowBucket,
  categoryScopeForCashFlowBucket,
} from "../../lib/categorization-buckets";
import type { CashFlowBucket } from "@/features/spending/types/cash-activity";
import type {
  ProposeCategoriesArgs,
  ProposeCategoriesOutput,
  ProposeCategoryOption,
  ProposeCategoryProposal,
} from "../../types";
import { createActivityAmountFormatter, formatActivityAmount, formatActivityDate } from "./shared";

type CategorizationProposalsToolUIContentProps = ToolCallMessagePartProps<
  ProposeCategoriesArgs,
  ProposeCategoriesOutput
>;

type UnknownRecord = Record<string, unknown>;

const TOOL_RESULT_PATCH_RETRY_DELAYS_MS = [500, 1500, 3000, 6000];

function isRecord(value: unknown): value is UnknownRecord {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function normalizeCategorizationResult(value: unknown): ProposeCategoriesOutput | undefined {
  if (!isRecord(value)) return undefined;

  if ("data" in value) {
    const nested = normalizeCategorizationResult(value.data);
    if (!nested) return undefined;
    const meta = isRecord(value.meta) ? value.meta : {};
    return {
      ...nested,
      ...(typeof meta.submitted === "boolean" ? { submitted: meta.submitted } : {}),
      ...(typeof meta.appliedCount === "number" ? { appliedCount: meta.appliedCount } : {}),
      ...(meta.draftStatus === "draft" || meta.draftStatus === "applied"
        ? { draftStatus: meta.draftStatus }
        : {}),
      ...(typeof meta.submittedAt === "string" ? { submittedAt: meta.submittedAt } : {}),
    };
  }

  if (Array.isArray(value.proposals) && Array.isArray(value.taxonomies)) {
    return value as unknown as ProposeCategoriesOutput;
  }

  return undefined;
}

function delay(ms: number) {
  return new Promise<void>((resolve) => window.setTimeout(resolve, ms));
}

async function persistCategorizationToolResult(
  request: Parameters<typeof updateToolResult>[0],
): Promise<void> {
  let lastError: unknown;
  for (let attempt = 0; attempt <= TOOL_RESULT_PATCH_RETRY_DELAYS_MS.length; attempt += 1) {
    try {
      await updateToolResult(request);
      return;
    } catch (error) {
      lastError = error;
      const retryDelay = TOOL_RESULT_PATCH_RETRY_DELAYS_MS[attempt];
      if (retryDelay === undefined) break;
      await delay(retryDelay);
    }
  }
  throw lastError;
}

interface RowDraft {
  activityId: string;
  activityDate: string;
  amount: number;
  currency: string;
  notes: string | null;
  cashFlowBucket?: CashFlowBucket;
  taxonomyId: string;
  categoryId: string;
  categoryPath: string;
  categoryColor: string;
  confidence: number;
  source: string;
  explanation: string;
  selected: boolean;
}

function confidencePillClass(confidence: number): string {
  if (confidence >= 0.85) return "bg-success/15 text-success border-success/30";
  if (confidence >= 0.6) return "bg-warning/15 text-warning border-warning/30";
  return "bg-muted text-muted-foreground border-border";
}

function confidenceLabel(confidence: number): string {
  return `${Math.round(confidence * 100)}%`;
}

function ProposalsLoadingState() {
  const { t } = useTranslation();
  return (
    <Card className="bg-muted/40 border-primary/10 w-full overflow-hidden">
      <CardContent className="flex items-center gap-3 py-5">
        <div className="bg-primary/10 flex h-9 w-9 shrink-0 items-center justify-center rounded-full">
          <Icons.Sparkles className="text-primary h-4 w-4 animate-pulse" />
        </div>
        <div className="min-w-0 flex-1">
          <p className="text-sm font-medium">{t("ai:categorization.categorizing")}</p>
          <p className="text-muted-foreground mt-0.5 text-xs">
            {t("ai:categorization.categorizingHint")}
          </p>
        </div>
        <Icons.Spinner className="text-muted-foreground h-4 w-4 shrink-0 animate-spin" />
      </CardContent>
    </Card>
  );
}

export function CategorizationProposalsContentImpl({
  result: rawResult,
  status,
  toolCallId,
}: CategorizationProposalsToolUIContentProps) {
  const { t } = useTranslation();
  const result = useMemo(() => normalizeCategorizationResult(rawResult) ?? rawResult, [rawResult]);
  const { settings } = useSettingsContext();
  const baseCurrency = settings?.baseCurrency ?? "USD";
  const { isBalanceHidden } = useBalancePrivacy();
  const runtime = useRuntimeContext();
  const threadId = runtime.currentThreadId;
  const amountFormatter = useMemo(() => createActivityAmountFormatter(), []);
  const bulkAssign = useBulkAssignCategories();
  const cashFlowLabels: Record<CashFlowBucket, string> = {
    spending: t("spending:cashFlow.spending"),
    income: t("spending:cashFlow.income"),
    saving: t("spending:cashFlow.saving"),
    neutral: t("ai:categorization.unknownBucket"),
  };

  const isLoading = status?.type === "running";
  const isIncomplete = status?.type === "incomplete";

  const categoryColorMap = useMemo(() => {
    const map = new Map<string, string>();
    for (const tax of result?.taxonomies ?? []) {
      for (const cat of tax.categories) {
        map.set(cat.categoryId, cat.color);
      }
    }
    return map;
  }, [result?.taxonomies]);

  const categoryPathMap = useMemo(() => {
    const map = new Map<string, ProposeCategoryOption & { taxonomyId: string }>();
    for (const tax of result?.taxonomies ?? []) {
      for (const cat of tax.categories) {
        map.set(cat.categoryId, { ...cat, taxonomyId: tax.taxonomyId });
      }
    }
    return map;
  }, [result?.taxonomies]);

  const initialRows = useMemo<RowDraft[]>(() => {
    if (!result) return [];
    return (result.proposals ?? []).map((p: ProposeCategoryProposal) => ({
      activityId: p.activityId,
      activityDate: p.activityDate,
      amount: p.amount,
      currency: p.currency,
      notes: p.notes,
      cashFlowBucket: p.cashFlowBucket,
      taxonomyId: p.taxonomyId,
      categoryId: p.categoryId,
      categoryPath: p.categoryPath,
      categoryColor: categoryColorMap.get(p.categoryId) ?? "#94a3b8",
      confidence: p.confidence,
      source: p.source,
      explanation: p.explanation,
      selected:
        p.confidence >= 0.6 && categoryMatchesCashFlowBucket(p.cashFlowBucket, p.taxonomyId),
    }));
  }, [result, categoryColorMap]);

  const [rows, setRows] = useState<RowDraft[]>(initialRows);
  const [pickedUnproposedIds, setPickedUnproposedIds] = useState<Set<string>>(new Set());
  const [submitError, setSubmitError] = useState<string | null>(null);
  const [persistError, setPersistError] = useState(false);
  const [localSubmitted, setLocalSubmitted] = useState(false);
  const [localAppliedCount, setLocalAppliedCount] = useState<number | null>(null);

  useEffect(() => {
    setRows(initialRows);
    setPickedUnproposedIds(new Set());
  }, [initialRows]);

  const visibleUnproposed = useMemo(
    () => (result?.unproposed ?? []).filter((row) => !pickedUnproposedIds.has(row.activityId)),
    [result?.unproposed, pickedUnproposedIds],
  );

  const isSubmitted =
    localSubmitted || result?.submitted === true || result?.draftStatus === "applied";
  const selectedCount = rows.filter((r) => r.selected).length;
  // Unproposed rows are intentionally non-selectable and may be neutral (or
  // legacy bucketless rows). They must not block valid proposals from being
  // applied; only editable proposal rows can invalidate this draft.
  const hasInvalidDraftRows = rows.some(
    (row) => !categoryMatchesCashFlowBucket(row.cashFlowBucket, row.taxonomyId),
  );
  const isSubmitting = bulkAssign.isPending;
  const canSubmit = !isSubmitted && !isSubmitting && !hasInvalidDraftRows && selectedCount > 0;
  const appliedCount = localAppliedCount ?? result?.appliedCount ?? 0;

  if (isLoading) return <ProposalsLoadingState />;

  if (isIncomplete) {
    return (
      <Card className="border-destructive/30 bg-destructive/5">
        <CardContent className="py-4">
          <p className="text-destructive text-sm font-medium">
            {t("ai:categorization.failedLoad")}
          </p>
        </CardContent>
      </Card>
    );
  }

  if (!result) {
    return (
      <Card className="border-destructive/30 bg-destructive/5">
        <CardContent className="py-4">
          <p className="text-destructive text-sm font-medium">
            {t("ai:categorization.noProposals")}
          </p>
        </CardContent>
      </Card>
    );
  }

  const summary = result.summary ?? {
    total: (result.proposals?.length ?? 0) + (result.unproposed?.length ?? 0),
    proposed: result.proposals?.length ?? 0,
    unproposed: result.unproposed?.length ?? 0,
    avgConfidence:
      (result.proposals?.length ?? 0) > 0
        ? result.proposals.reduce((acc, p) => acc + (p.confidence ?? 0), 0) /
          result.proposals.length
        : 0,
  };

  const toggleRow = (activityId: string) => {
    setRows((prev) =>
      prev.map((r) =>
        r.activityId === activityId && categoryMatchesCashFlowBucket(r.cashFlowBucket, r.taxonomyId)
          ? { ...r, selected: !r.selected }
          : r,
      ),
    );
  };

  const updateRowCategory = (activityId: string, taxonomyId: string, categoryId: string) => {
    const row = rows.find((candidate) => candidate.activityId === activityId);
    if (!row || !categoryMatchesCashFlowBucket(row.cashFlowBucket, taxonomyId)) return;
    const cat = categoryPathMap.get(categoryId);
    setRows((prev) =>
      prev.map((r) =>
        r.activityId === activityId
          ? {
              ...r,
              taxonomyId,
              categoryId,
              categoryPath: cat?.path ?? r.categoryPath,
              categoryColor: cat?.color ?? r.categoryColor,
              source: "manual",
              explanation: t("ai:categorization.editedByUser"),
              confidence: 1,
              selected: true,
            }
          : r,
      ),
    );
  };

  const handleAcceptAll = () =>
    setRows((prev) =>
      prev.map((r) => ({
        ...r,
        selected: categoryMatchesCashFlowBucket(r.cashFlowBucket, r.taxonomyId),
      })),
    );
  const handleAcceptHighConfidence = () =>
    setRows((prev) =>
      prev.map((r) => ({
        ...r,
        selected:
          r.confidence >= 0.85 && categoryMatchesCashFlowBucket(r.cashFlowBucket, r.taxonomyId),
      })),
    );
  const handleRejectAll = () => setRows((prev) => prev.map((r) => ({ ...r, selected: false })));

  const handleSubmit = async () => {
    if (!canSubmit) return;
    setSubmitError(null);
    setPersistError(false);

    const accepted = rows.filter((r) => r.selected);
    try {
      const applied = await bulkAssign.mutateAsync(
        accepted.map((r) => ({
          activityId: r.activityId,
          taxonomyId: r.taxonomyId,
          categoryId: r.categoryId,
        })),
      );

      setLocalSubmitted(true);
      setLocalAppliedCount(applied.length);

      if (!threadId || !toolCallId) {
        setPersistError(true);
        return;
      }

      try {
        await persistCategorizationToolResult({
          threadId,
          toolCallId,
          resultPatch: {
            submitted: true,
            draftStatus: "applied",
            appliedCount: applied.length,
            submittedAt: new Date().toISOString(),
          },
        });
      } catch (error) {
        setPersistError(true);
        console.error("Failed to persist categorization tool state:", error);
      }
    } catch (error) {
      setSubmitError(error instanceof Error ? error.message : t("ai:categorization.failedApply"));
    }
  };

  return (
    <Card className="bg-muted/40 border-primary/10 w-full overflow-hidden">
      <CardHeader className="pb-2">
        <div className="flex flex-wrap items-start justify-between gap-2">
          <div>
            <CardTitle className="text-sm font-medium">{t("ai:categorization.title")}</CardTitle>
            <p className="text-muted-foreground mt-1 text-xs">
              {t("ai:categorization.summary", {
                proposed: summary.proposed,
                unproposed: summary.unproposed,
                confidence: Math.round((summary.avgConfidence ?? 0) * 100),
              })}
            </p>
          </div>
          <Badge variant="outline" className="text-xs">
            {t("ai:categorization.selected", { count: selectedCount })}
          </Badge>
        </div>
      </CardHeader>
      <CardContent className="space-y-3 px-0 pb-3">
        {hasInvalidDraftRows && !isSubmitted && (
          <div className="mx-3 sm:mx-6">
            <Alert variant="error">
              <Icons.AlertCircle className="h-4 w-4" />
              <AlertTitle>{t("ai:categorization.expiredTitle")}</AlertTitle>
              <AlertDescription className="text-xs">
                {t("ai:categorization.expiredDescription")}
              </AlertDescription>
            </Alert>
          </div>
        )}

        {!isSubmitted && rows.length > 0 && (
          <div className="flex flex-wrap items-center justify-between gap-2 px-3 sm:px-6">
            <label className="flex cursor-pointer items-center gap-2 text-xs">
              <Checkbox
                checked={
                  selectedCount === 0
                    ? false
                    : selectedCount === rows.length
                      ? true
                      : "indeterminate"
                }
                onCheckedChange={(checked) =>
                  checked === true ? handleAcceptAll() : handleRejectAll()
                }
                aria-label={t("ai:categorization.selectAllRows")}
              />
              <span className="text-muted-foreground">
                {t("ai:categorization.selectAll", {
                  selected: selectedCount,
                  total: rows.length,
                })}
              </span>
            </label>
            <Button
              size="sm"
              variant="outline"
              onClick={handleAcceptHighConfidence}
              disabled={isSubmitting}
            >
              {t("ai:categorization.selectHighConfidence")}
            </Button>
          </div>
        )}

        <div className="max-h-[420px] overflow-y-auto px-2">
          {rows.length === 0 && visibleUnproposed.length === 0 && (
            <p className="text-muted-foreground px-4 py-6 text-center text-xs">
              {(result.taxonomies?.length ?? 0) === 0
                ? t("ai:categorization.noTaxonomies")
                : t("ai:categorization.noTransactions")}
            </p>
          )}

          {rows.map((row) => (
            <div
              key={row.activityId}
              className={cn(
                "hover:bg-background/50 grid min-w-0 grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-x-2 gap-y-2 rounded-md px-3 py-2 text-xs sm:grid-cols-[auto_6rem_6rem_minmax(0,1fr)_auto_auto] sm:gap-x-3 sm:px-4",
                row.selected && "bg-background/40",
              )}
            >
              <Checkbox
                checked={row.selected}
                onCheckedChange={() => toggleRow(row.activityId)}
                disabled={
                  isSubmitted || !categoryMatchesCashFlowBucket(row.cashFlowBucket, row.taxonomyId)
                }
                aria-label={t("ai:categorization.selectRow")}
              />
              <span className="text-muted-foreground min-w-0 whitespace-nowrap tabular-nums">
                {formatActivityDate(row.activityDate)}
              </span>
              <span className="whitespace-nowrap text-right tabular-nums">
                {formatActivityAmount(
                  row.amount,
                  amountFormatter,
                  isBalanceHidden,
                  row.currency || baseCurrency,
                )}
              </span>
              <span className="col-span-3 min-w-0 truncate sm:col-span-1" title={row.notes ?? ""}>
                {row.notes ?? (
                  <span className="text-muted-foreground italic">
                    {t("ai:categorization.noNotes")}
                  </span>
                )}
              </span>
              <QuickCategorizePopover
                trigger={
                  <button
                    type="button"
                    className="hover:bg-muted/70 col-span-2 inline-flex min-w-0 max-w-full items-center gap-1.5 rounded-full border px-2 py-0.5 text-[11px] sm:col-span-1"
                    disabled={
                      isSubmitted || categoryScopeForCashFlowBucket(row.cashFlowBucket) === null
                    }
                  >
                    <span
                      className="h-2 w-2 rounded-full"
                      style={{ backgroundColor: row.categoryColor }}
                    />
                    <span className="text-muted-foreground shrink-0">
                      {row.cashFlowBucket
                        ? cashFlowLabels[row.cashFlowBucket]
                        : t("ai:categorization.unknownBucket")}
                    </span>
                    <span className="min-w-0 max-w-[180px] truncate">{row.categoryPath}</span>
                  </button>
                }
                selectedCategoryId={row.categoryId}
                scope={categoryScopeForCashFlowBucket(row.cashFlowBucket) ?? "both"}
                onSelect={(taxonomyId, categoryId) =>
                  updateRowCategory(row.activityId, taxonomyId, categoryId)
                }
              />
              <Badge
                variant="outline"
                className={cn("text-[10px] uppercase", confidencePillClass(row.confidence))}
                title={row.explanation}
              >
                {confidenceLabel(row.confidence)}
              </Badge>
            </div>
          ))}

          {visibleUnproposed.length > 0 && (
            <div className="mt-3 border-t pt-2">
              <p className="text-muted-foreground px-4 pb-1 text-[11px] uppercase tracking-wide">
                {t("ai:categorization.needsManualChoice", { count: visibleUnproposed.length })}
              </p>
              {visibleUnproposed.map((row) => (
                <div
                  key={row.activityId}
                  className="hover:bg-background/40 grid min-w-0 grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-x-2 gap-y-2 rounded-md px-3 py-2 text-xs sm:grid-cols-[auto_6rem_6rem_minmax(0,1fr)_auto] sm:gap-x-3 sm:px-4"
                >
                  <span className="w-5" />
                  <span className="text-muted-foreground min-w-0 whitespace-nowrap tabular-nums">
                    {formatActivityDate(row.activityDate)}
                  </span>
                  <span className="whitespace-nowrap text-right tabular-nums">
                    {formatActivityAmount(
                      row.amount,
                      amountFormatter,
                      isBalanceHidden,
                      row.currency || baseCurrency,
                    )}
                  </span>
                  <span
                    className="col-span-3 min-w-0 truncate sm:col-span-1"
                    title={row.notes ?? ""}
                  >
                    {row.notes ?? (
                      <span className="text-muted-foreground italic">
                        {t("ai:categorization.noNotes")}
                      </span>
                    )}
                  </span>
                  <QuickCategorizePopover
                    trigger={
                      <button
                        type="button"
                        className="text-muted-foreground hover:bg-muted/70 col-span-3 inline-flex min-w-0 max-w-full items-center justify-center gap-1 rounded-full border border-dashed px-2 py-0.5 text-[11px] sm:col-span-1"
                        disabled={categoryScopeForCashFlowBucket(row.cashFlowBucket) === null}
                      >
                        <Icons.Plus className="h-3 w-3" />
                        {t("ai:categorization.pickCategory")}
                      </button>
                    }
                    scope={categoryScopeForCashFlowBucket(row.cashFlowBucket) ?? "both"}
                    onSelect={(taxonomyId, categoryId) => {
                      if (!categoryMatchesCashFlowBucket(row.cashFlowBucket, taxonomyId)) return;
                      const cat = categoryPathMap.get(categoryId);
                      setPickedUnproposedIds((prev) => {
                        const next = new Set(prev);
                        next.add(row.activityId);
                        return next;
                      });
                      setRows((prev) => [
                        ...prev,
                        {
                          activityId: row.activityId,
                          activityDate: row.activityDate,
                          amount: row.amount,
                          currency: row.currency,
                          notes: row.notes,
                          cashFlowBucket: row.cashFlowBucket,
                          taxonomyId,
                          categoryId,
                          categoryPath: cat?.path ?? "",
                          categoryColor: cat?.color ?? "#94a3b8",
                          confidence: 1,
                          source: "manual",
                          explanation: t("ai:categorization.pickedManually"),
                          selected: true,
                        },
                      ]);
                    }}
                  />
                </div>
              ))}
            </div>
          )}
        </div>

        {submitError && (
          <div className="mx-3 sm:mx-6">
            <Alert variant="error">
              <Icons.AlertCircle className="h-4 w-4" />
              <AlertTitle>{t("ai:categorization.failedApplyTitle")}</AlertTitle>
              <AlertDescription className="break-words text-xs">{submitError}</AlertDescription>
            </Alert>
          </div>
        )}

        {persistError && (
          <p className="text-destructive px-3 text-xs sm:px-6">
            {t("ai:assetClassification.chatNotUpdated")}
          </p>
        )}

        {isSubmitted && (
          <div className="px-3 sm:px-6">
            <div className="border-success/30 bg-success/10 flex items-center gap-3 rounded-md border px-3 py-2">
              <div className="bg-success/20 text-success flex h-7 w-7 shrink-0 items-center justify-center rounded-full">
                <Icons.Check className="h-4 w-4" />
              </div>
              <div className="min-w-0 flex-1">
                <p className="text-success text-sm font-medium">
                  {t("ai:categorization.transactionsCategorized", { count: appliedCount })}
                </p>
                <p className="text-muted-foreground text-xs">
                  {t("ai:categorization.categoriesSaved")}
                </p>
              </div>
            </div>
          </div>
        )}

        {!isSubmitted && rows.length > 0 && (
          <div className="flex items-center justify-end px-3 pt-1 sm:px-6">
            <Button onClick={handleSubmit} disabled={!canSubmit}>
              {isSubmitting ? (
                <Icons.Spinner className="mr-2 h-4 w-4 animate-spin" />
              ) : (
                <Icons.Check className="mr-2 h-4 w-4" />
              )}
              {selectedCount > 0
                ? t("ai:categorization.applySelected", { count: selectedCount })
                : t("ai:categorization.apply")}
            </Button>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

const CategorizationProposalsContent = memo(CategorizationProposalsContentImpl);

export const CategorizationProposalsToolUI = makeAssistantToolUI<
  ProposeCategoriesArgs,
  ProposeCategoriesOutput
>({
  toolName: "propose_transaction_categories",
  render: (props) => {
    return <CategorizationProposalsContent {...props} />;
  },
});
