import { useBalancePrivacy } from "@/hooks/use-balance-privacy";
import { useTaxonomy } from "@/hooks/use-taxonomies";
import { SECTION_HEADING } from "@/components/dashboard-card";
import { cn } from "@/lib/utils";
import type { DriftReport, DriftRow, AllocationTarget } from "@/lib/types";
import { Button, Card, Icons, Skeleton } from "@wealthfolio/ui";
import { useTranslation } from "react-i18next";
import {
  allocationTargetColorForRow,
  buildAllocationTargetColorMap,
} from "@/pages/allocation-targets/components/allocation-target-colors";
import { formatTolerance } from "@/pages/allocation-targets/components/drift-copy";
import { resolveDriftReportCategories } from "@/pages/allocation-targets/components/drift-report-resolver";
import {
  formatDriftBps,
  hasVisibleAllocation,
  isOutOfBand,
  rebalanceMove,
} from "@/pages/allocation-targets/components/drift-row-utils";
import { formatCompact } from "./allocation-derivations";

interface TargetRailsCardProps {
  targets: AllocationTarget[];
  selectedTargetId: string | null;
  onTargetChange: (id: string) => void;
  driftReport: DriftReport | null;
  isLoading?: boolean;
  onCreateTarget?: () => void;
  /** Opens the full current-vs-target analysis. */
  onViewDetails?: () => void;
}

function driftClass(row: DriftRow): string {
  if (row.status === "overweight" || row.status === "not_targeted") return "text-destructive";
  if (row.status === "underweight") return "text-blue-600 dark:text-blue-400";
  return "text-muted-foreground";
}

export function TargetRailsCard({
  targets,
  selectedTargetId,
  onTargetChange,
  driftReport,
  isLoading,
  onCreateTarget,
  onViewDetails,
}: TargetRailsCardProps) {
  const { t } = useTranslation();
  const { isBalanceHidden } = useBalancePrivacy();
  const selectedTarget = targets.find((target) => target.id === selectedTargetId) ?? null;
  const { data: taxonomy } = useTaxonomy(selectedTarget?.taxonomyId ?? null);
  const resolvedDriftReport = driftReport
    ? resolveDriftReportCategories(driftReport, taxonomy?.categories)
    : null;
  const colorByCategory = resolvedDriftReport
    ? buildAllocationTargetColorMap(resolvedDriftReport.rows)
    : undefined;

  if (isLoading) {
    return (
      <Card className="space-y-4 p-5 xl:h-full">
        <Skeleton className="h-5 w-40" />
        <Skeleton className="h-8 w-full" />
        <Skeleton className="h-32 w-full" />
      </Card>
    );
  }

  const hasTarget = !!resolvedDriftReport;
  const currency = resolvedDriftReport?.baseCurrency ?? "USD";
  const rows = resolvedDriftReport?.rows.filter(hasVisibleAllocation) ?? [];

  const toleranceLabel = (() => {
    const requiredRows = rows.filter((r) => r.isRequired && r.targetBps > 0);
    if (!requiredRows.length) return formatTolerance(selectedTarget?.driftBandBps ?? 0);
    const bands = requiredRows.map((r) => r.effectiveBandBps);
    const minBand = Math.min(...bands);
    const maxBand = Math.max(...bands);
    if (minBand === maxBand) return formatTolerance(minBand);
    const fmt = (bps: number) => {
      const pp = bps / 100;
      return Number.isInteger(pp) ? pp.toFixed(0) : pp.toFixed(1);
    };
    return `±${fmt(minBand)}–${fmt(maxBand)}%`;
  })();
  const maxScale =
    Math.max(1, ...rows.flatMap((r) => [r.currentBps / 100, r.targetBps / 100])) * 1.08;
  const withinTolerance = resolvedDriftReport ? resolvedDriftReport.outOfBandCount === 0 : false;
  const largestGapRow = rows
    .filter(isOutOfBand)
    .sort((a, b) => Math.abs(b.driftBps) - Math.abs(a.driftBps))[0];
  const largestGapLabel = formatDriftBps(
    largestGapRow?.driftBps ?? resolvedDriftReport?.maxDriftBps ?? 0,
  );

  // Suggested rebalance moves for the out-of-range categories, largest first.
  const moves = rows
    .map((row, index) => ({
      row,
      color: allocationTargetColorForRow(row, colorByCategory, index),
    }))
    .filter(({ row }) => isOutOfBand(row))
    .sort((a, b) => Math.abs(b.row.valueDelta) - Math.abs(a.row.valueDelta));
  const money = (value: number) =>
    isBalanceHidden ? "••••" : formatCompact(Math.abs(value), currency);

  return (
    <Card className="flex flex-col gap-3 xl:h-full">
      {/* Header: title + Details (top-right) */}
      <div className="flex items-center justify-between gap-3">
        <h2 className={SECTION_HEADING}>{t("insights:insights.rails.target_allocation")}</h2>
        {hasTarget && onViewDetails && (
          <Button
            variant="ghost"
            size="sm"
            className="text-muted-foreground hover:bg-foreground/5 h-8 px-3 text-sm"
            onClick={onViewDetails}
          >
            {t("insights:insights.rails.details")}
          </Button>
        )}
      </div>

      {/* Target selector — pill carousel */}
      {targets.length > 0 && (
        <div className="-mx-1 flex gap-1.5 overflow-x-auto px-1 pb-1 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
          {targets.map((p) => (
            <button
              key={p.id}
              type="button"
              onClick={() => onTargetChange(p.id)}
              className={cn(
                "flex h-9 shrink-0 items-center gap-1.5 whitespace-nowrap rounded-full px-3.5 text-sm transition-colors",
                p.id === selectedTargetId
                  ? "bg-foreground text-background"
                  : "bg-foreground/[0.07]",
              )}
            >
              {p.name}
            </button>
          ))}
        </div>
      )}

      {hasTarget ? (
        <>
          {/* Rails */}
          <div className="flex flex-col">
            {rows.map((row, index) => {
              const cur = row.currentBps / 100;
              const tgt = row.targetBps / 100;
              const color = allocationTargetColorForRow(row, colorByCategory, index);
              return (
                <div
                  key={row.categoryId}
                  className="grid grid-cols-[minmax(0,9rem)_1fr_4rem] items-center gap-3 py-2.5"
                >
                  <span className="flex items-center gap-2 overflow-hidden text-[15px]">
                    <span
                      className="h-2.5 w-2.5 shrink-0 rounded-full"
                      style={{ background: color }}
                    />
                    <span className="truncate">{row.categoryName}</span>
                  </span>
                  <span className="bg-foreground/10 relative h-2.5 rounded-[4px]">
                    <span
                      className="absolute top-0 h-full rounded-[4px]"
                      style={{ width: `${(cur / maxScale) * 100}%`, background: color }}
                    />
                    <span
                      className="bg-foreground absolute -top-1 h-4 w-[2.5px] rounded-sm"
                      style={{ left: `calc(${(tgt / maxScale) * 100}% - 1px)` }}
                    />
                  </span>
                  <span className={cn("text-right text-[15px] tabular-nums", driftClass(row))}>
                    {formatDriftBps(row.driftBps)}
                  </span>
                </div>
              );
            })}
          </div>

          {/* Suggested moves — fills the remaining height */}
          <div className="flex flex-col pt-5 xl:flex-1">
            <div className="text-muted-foreground mb-2 text-[13px]">
              {t("insights:insights.rails.suggested_moves")}
            </div>
            {moves.length === 0 ? (
              <div className="text-muted-foreground flex flex-col items-center justify-center gap-1.5 py-4 text-center xl:flex-1">
                <Icons.CheckCircle className="text-success h-6 w-6" />
                <span className="text-[15px]">{t("insights:insights.rails.all_in_range")}</span>
              </div>
            ) : (
              <div className="flex flex-col gap-2.5">
                {moves.slice(0, 6).map(({ row, color }) => {
                  const move = rebalanceMove(row);
                  const add = move.action === "Add";
                  return (
                    <div
                      key={row.categoryId}
                      className="flex items-center justify-between gap-2 text-[15px]"
                    >
                      <span className="flex min-w-0 items-center gap-2">
                        <span
                          className="h-2 w-2 shrink-0 rounded-sm"
                          style={{ background: color }}
                        />
                        <span className="text-muted-foreground">
                          {add
                            ? t("insights:insights.rails.add")
                            : t("insights:insights.rails.trim")}
                        </span>
                        <span className="text-foreground truncate font-medium">
                          {row.categoryName}
                        </span>
                      </span>
                      <span
                        className={cn(
                          "shrink-0 font-medium tabular-nums",
                          add ? "text-success" : "text-destructive",
                        )}
                      >
                        {add ? "+" : "−"}
                        {money(move.amount)}
                      </span>
                    </div>
                  );
                })}
              </div>
            )}
          </div>

          {/* Bottom status */}
          <div
            className={cn(
              "mt-3 flex items-center gap-2 text-[13px] xl:mt-auto",
              withinTolerance ? "text-success" : "text-warning",
            )}
          >
            <span
              className={cn(
                "h-1.5 w-1.5 rounded-full",
                withinTolerance ? "bg-success" : "bg-warning",
              )}
            />
            <span className="min-w-0 flex-1 truncate">
              {withinTolerance
                ? t("insights:insights.rails.inside_target_range")
                : t("insights:insights.rails.outside_range", {
                    count: resolvedDriftReport?.outOfBandCount ?? 0,
                    gap: largestGapLabel,
                  })}
            </span>
            <span className="shrink-0 tabular-nums">
              {t("insights:insights.rails.tolerance", { tolerance: toleranceLabel })}
            </span>
          </div>
        </>
      ) : (
        <p className="text-muted-foreground text-[15px]">
          {t("insights:insights.rails.no_target_yet")}.{" "}
          {t("insights:insights.rails.compare_weights")}{" "}
          {onCreateTarget && (
            <button
              type="button"
              className="text-foreground hover:underline"
              onClick={onCreateTarget}
            >
              {t("insights:insights.rails.set_target_allocation")}
            </button>
          )}
        </p>
      )}
    </Card>
  );
}
