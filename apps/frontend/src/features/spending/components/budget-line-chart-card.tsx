import { useMemo } from "react";
import { Link } from "react-router-dom";
import { useTranslation } from "react-i18next";

import { BlockBar } from "@/components/block-bar";
import { DashboardCard, SECTION_HEADING } from "@/components/dashboard-card";
import { cn } from "@/lib/utils";
import { formatCompactAmount, Icons, PrivacyAmount, useBalancePrivacy } from "@wealthfolio/ui";

import { CategoryIcon, type CategoryMetaMap } from "./category-chips";
import { topCategoryId } from "../lib/category-rollup";
import {
  budgetPeriodDayIndex,
  budgetPeriodKeyForDate,
  budgetPeriodLabel,
  budgetPeriodParts,
} from "../lib/month-period";
import { calendarDaysBetweenInclusive } from "../lib/timezone";
import type { BudgetCycleSettings } from "../types";
import type { BudgetCategoryRow } from "../types/budget";
import type { DayBucket } from "../types/report";

type Status = "ok" | "warn" | "over";
interface PacePoint {
  day: number;
  value: number;
}
interface BudgetToday {
  year: number;
  month: number;
  day: number;
}

const MIN_HISTORICAL_PACE_MONTHS = 2;

const STATUS_ACCENTS: Record<
  Status,
  {
    lineColor: string;
    labelKey: string;
  }
> = {
  over: {
    lineColor: "var(--destructive)",
    labelKey: "spending:budgetChart.overBudget",
  },
  warn: {
    lineColor: "#C28B47",
    labelKey: "spending:budgetChart.trendingHigh",
  },
  ok: {
    lineColor: "var(--success)",
    labelKey: "spending:budgetChart.onTrack",
  },
};

export function BudgetLineChartCard({
  monthKey,
  cycle,
  today,
  isCurrentMonth,
  onPreviousMonth,
  onNextMonth,
  canGoNextMonth,
  target,
  spent,
  currency,
  historicalDailyAvg,
  monthByDay,
  historicalByDay,
}: {
  monthKey: string;
  cycle: BudgetCycleSettings;
  today: BudgetToday;
  isCurrentMonth: boolean;
  onPreviousMonth: () => void;
  onNextMonth: () => void;
  canGoNextMonth: boolean;
  target: number;
  spent: number;
  currency: string;
  historicalDailyAvg: number;
  monthByDay: DayBucket[];
  historicalByDay: DayBucket[];
}) {
  const { t } = useTranslation();
  // All hooks must run unconditionally — the `target <= 0` early return below
  // sits between hooks otherwise, which trips "Rendered more hooks than during
  // the previous render" when a target is added or cleared.
  const monthMeta = useMemo(() => {
    const period = budgetPeriodParts(monthKey, cycle);
    const start = period?.start ?? today;
    const end = period?.end ?? today;
    const daysInPeriod = calendarDaysBetweenInclusive(start, end);
    const todayIndex = calendarDaysBetweenInclusive(start, today);
    const dayOfPeriod = isCurrentMonth
      ? Math.min(daysInPeriod, Math.max(1, todayIndex))
      : daysInPeriod;
    return {
      dayOfPeriod,
      daysInPeriod,
      daysRemaining: isCurrentMonth ? Math.max(0, daysInPeriod - dayOfPeriod) : 0,
      monthLabel: budgetPeriodLabel(monthKey, "short", undefined, cycle),
      shortLabel: formatCycleEndpoint(start),
      startLabel: formatCycleEndpoint(start),
      endLabel: formatCycleEndpoint(end),
    };
  }, [monthKey, cycle, isCurrentMonth, today]);
  const { dayOfPeriod, daysInPeriod, daysRemaining } = monthMeta;

  const cumulative = useMemo(() => {
    const byDay = new Map<number, number>();
    for (const bucket of monthByDay) {
      const day = budgetPeriodDayIndex(bucket.date, monthKey, cycle);
      if (day !== null) byDay.set(day, (byDay.get(day) ?? 0) + bucket.outflow);
    }
    let running = 0;
    const out: { day: number; value: number }[] = [];
    for (let day = 1; day <= dayOfPeriod; day++) {
      running += byDay.get(day) ?? 0;
      out.push({ day, value: running });
    }
    return out;
  }, [monthByDay, monthKey, cycle, dayOfPeriod]);

  // Chart geometry derived from target — captured here so actualPath useMemo
  // can depend on stable primitives instead of recomputing each render.
  const chartW = 320;
  const chartH = 110;
  const padL = 0;
  const padR = 0;
  const padT = 24;
  const padB = 14;
  const innerW = chartW - padL - padR;
  const innerH = chartH - padT - padB;
  const yMax = Math.max(target, spent) * 1.05;

  const actualPath = useMemo(() => {
    if (!cumulative.length) return "";
    const xForDay = (day: number) => padL + ((day - 1) / Math.max(1, daysInPeriod - 1)) * innerW;
    const yForVal = (v: number) => padT + (1 - v / yMax) * innerH;
    return (
      "M " +
      cumulative
        .map((p) => `${xForDay(p.day).toFixed(2)} ${yForVal(p.value).toFixed(2)}`)
        .join(" L ")
    );
  }, [cumulative, daysInPeriod, innerW, innerH, padL, padT, yMax]);
  // The same line closed down to the baseline, for the soft fill under it.
  const actualAreaPath = useMemo(() => {
    if (!actualPath || !cumulative.length) return "";
    const lastX =
      padL + ((cumulative[cumulative.length - 1].day - 1) / Math.max(1, daysInPeriod - 1)) * innerW;
    const baseY = padT + innerH;
    return `${actualPath} L ${lastX.toFixed(2)} ${baseY} L ${padL} ${baseY} Z`;
  }, [actualPath, cumulative, daysInPeriod, innerW, innerH, padL, padT]);

  const historicalPace = useMemo(
    () => buildHistoricalPaceCurve(historicalByDay, daysInPeriod, cycle),
    [historicalByDay, daysInPeriod, cycle],
  );

  const targetPacePath = useMemo(() => {
    if (!historicalPace || target <= 0) return "";
    const xForDay = (day: number) => padL + ((day - 1) / Math.max(1, daysInPeriod - 1)) * innerW;
    const yForVal = (v: number) => padT + (1 - v / yMax) * innerH;
    return toSvgPath(
      historicalPace.points.map((p) => ({
        day: p.day,
        value: p.value * target,
      })),
      xForDay,
      yForVal,
    );
  }, [historicalPace, target, daysInPeriod, innerW, innerH, padL, padT, yMax]);

  const haveHistory = historicalDailyAvg > 0;
  const forecast =
    target > 0 && isCurrentMonth
      ? haveHistory
        ? spent + historicalDailyAvg * daysRemaining
        : dayOfPeriod > 0
          ? (spent / dayOfPeriod) * daysInPeriod
          : 0
      : 0;
  const headerAction = (
    <BudgetCardHeaderActions
      monthLabel={monthMeta.shortLabel}
      monthKey={monthKey}
      onPreviousMonth={onPreviousMonth}
      onNextMonth={onNextMonth}
      canGoNextMonth={canGoNextMonth}
    />
  );

  if (target <= 0) {
    return (
      <DashboardCard
        title={t("spending:budgetChart.cycleBudget")}
        subtitle={monthMeta.monthLabel}
        action={headerAction}
        className="text-center"
      >
        <p className="text-muted-foreground text-sm">{t("spending:budgetChart.noTarget")}</p>
        <Link
          to={`/spending/budget?period=${monthKey}`}
          className="text-foreground mt-2 inline-flex text-xs underline-offset-4 hover:underline"
        >
          {t("spending:budgetChart.setBudget")}
        </Link>
      </DashboardCard>
    );
  }

  const remaining = Math.max(0, target - spent);
  const overBy = spent - target;
  const isOver = overBy > 0;
  const forecastReliable = isCurrentMonth && (haveHistory || dayOfPeriod >= 7);
  const forecastDelta = forecast - target;
  const willOverspend = forecastReliable && forecastDelta > 0;

  const historicalPaceAtToday = historicalPace?.pctByDay[dayOfPeriod];
  const paceAtToday =
    target *
    (historicalPaceAtToday !== undefined ? historicalPaceAtToday : dayOfPeriod / daysInPeriod);
  const gapVsPace = spent - paceAtToday;
  const aheadOfPace = gapVsPace < 0;

  const status: Status = isOver ? "over" : isCurrentMonth && !aheadOfPace ? "warn" : "ok";
  const a = STATUS_ACCENTS[status];
  const statusLabel =
    !isCurrentMonth && !isOver ? t("spending:budgetChart.underBudget") : t(a.labelKey);

  const xForDay = (day: number) => padL + ((day - 1) / Math.max(1, daysInPeriod - 1)) * innerW;
  const yForVal = (v: number) => padT + (1 - v / yMax) * innerH;

  const paceX1 = xForDay(1);
  const paceY1 = yForVal(0);
  const paceX2 = xForDay(daysInPeriod);
  const paceY2 = yForVal(target);

  const endX = cumulative.length ? xForDay(cumulative[cumulative.length - 1].day) : padL;
  const endY = cumulative.length ? yForVal(cumulative[cumulative.length - 1].value) : padT + innerH;

  const gapAbs = Math.abs(gapVsPace);
  const gapLabel = isCurrentMonth
    ? isOver
      ? t("spending:budgetChart.overBudgetAmount", {
          amount: formatCompactAmount(overBy, currency),
        })
      : aheadOfPace
        ? t("spending:budgetChart.underBudgetAmount", {
            amount: formatCompactAmount(gapAbs, currency),
          })
        : t("spending:budgetChart.overPaceAmount", {
            amount: formatCompactAmount(gapAbs, currency),
          })
    : isOver
      ? t("spending:budgetChart.overBudgetAmount", {
          amount: formatCompactAmount(overBy, currency),
        })
      : t("spending:budgetChart.leftAmount", { amount: formatCompactAmount(remaining, currency) });

  const pillLeftPctRaw = (endX / chartW) * 100;

  // One sentence, so the card never says "on track" and "over budget" together.
  const headingOver = isCurrentMonth && !isOver && willOverspend && forecastDelta > target * 0.05;

  return (
    <div className="w-full min-w-0">
      <div className="flex flex-wrap items-center justify-between gap-x-3 gap-y-1 pb-1">
        <h2 className={SECTION_HEADING}>{t("spending:tabContent.sectionPace")}</h2>
        {headerAction}
      </div>
      <div>
        <div className="text-sm">
          {headingOver ? (
            <span className="text-destructive">
              {t("spending:budgetChart.forecastUpper")}{" "}
              <PrivacyAmount value={forecast} currency={currency} />
              {", "}
              <PrivacyAmount value={forecastDelta} currency={currency} />{" "}
              {t("spending:budgetChart.overBudgetLower")}
            </span>
          ) : (
            <span style={{ color: a.lineColor }}>
              {statusLabel}, {gapLabel}
            </span>
          )}
        </div>
      </div>
      <div className="relative mt-2 w-full">
        <svg
          viewBox={`0 0 ${chartW} ${chartH}`}
          preserveAspectRatio="none"
          className="block h-[110px] w-full"
        >
          {targetPacePath ? (
            <path
              d={targetPacePath}
              fill="none"
              stroke="var(--muted-foreground)"
              strokeOpacity={0.35}
              strokeDasharray="2 4"
              strokeWidth={1}
              strokeLinecap="round"
              strokeLinejoin="round"
              vectorEffect="non-scaling-stroke"
            />
          ) : (
            <line
              x1={paceX1}
              y1={paceY1}
              x2={paceX2}
              y2={paceY2}
              stroke="var(--muted-foreground)"
              strokeOpacity={0.35}
              strokeDasharray="3 4"
              strokeWidth={1.25}
              vectorEffect="non-scaling-stroke"
            />
          )}
          <defs>
            <linearGradient id="budget-pace-fill" x1="0" y1="0" x2="0" y2="1">
              <stop offset="5%" stopColor={a.lineColor} stopOpacity={0.2} />
              <stop offset="70%" stopColor={a.lineColor} stopOpacity={0.12} />
              <stop offset="100%" stopColor={a.lineColor} stopOpacity={0} />
            </linearGradient>
          </defs>
          {actualAreaPath && (
            <path d={actualAreaPath} fill="url(#budget-pace-fill)" stroke="none" />
          )}
          {actualPath && (
            <path
              d={actualPath}
              fill="none"
              stroke={a.lineColor}
              strokeWidth={1.5}
              strokeLinecap="round"
              strokeLinejoin="round"
              vectorEffect="non-scaling-stroke"
            />
          )}
        </svg>
        {cumulative.length > 0 && (
          // Rendered as HTML rather than an SVG <circle> so it stays round: the
          // SVG uses preserveAspectRatio="none", which would stretch a circle
          // into an ellipse.
          <div
            className="absolute h-[6px] w-[6px] rounded-full"
            style={{
              left: `${pillLeftPctRaw}%`,
              top: `${endY}px`,
              transform: "translate(-50%, -50%)",
              backgroundColor: a.lineColor,
            }}
          />
        )}
      </div>
      <div className="text-muted-foreground/70 mt-1 flex justify-between text-[10px] tabular-nums">
        <span>{monthMeta.startLabel}</span>
        <span>{monthMeta.endLabel}</span>
      </div>
    </div>
  );
}

function buildHistoricalPaceCurve(
  byDay: DayBucket[],
  currentDaysInPeriod: number,
  cycle: BudgetCycleSettings,
): { points: PacePoint[]; pctByDay: number[] } | null {
  const periods = new Map<
    string,
    { daysInPeriod: number; outflowByDay: Map<number, number>; total: number }
  >();

  for (const bucket of byDay) {
    const parsed = parseDayBucketDate(bucket.date);
    if (!parsed) continue;
    const periodKey = budgetPeriodKeyForDate(parsed, cycle);
    const period = budgetPeriodParts(periodKey, cycle);
    const day = budgetPeriodDayIndex(bucket.date, periodKey, cycle);
    if (!period || day === null) continue;
    const outflow = Number.isFinite(bucket.outflow) ? bucket.outflow : 0;
    const row = periods.get(periodKey) ?? {
      daysInPeriod: calendarDaysBetweenInclusive(period.start, period.end),
      outflowByDay: new Map<number, number>(),
      total: 0,
    };
    row.outflowByDay.set(day, (row.outflowByDay.get(day) ?? 0) + outflow);
    row.total += outflow;
    periods.set(periodKey, row);
  }

  const eligiblePeriods = Array.from(periods.values())
    .filter((period) => period.total > 0)
    .map((period) => {
      const cumulativeByDay = Array.from({ length: period.daysInPeriod + 1 }, () => 0);
      let running = 0;
      for (let day = 1; day <= period.daysInPeriod; day++) {
        running += period.outflowByDay.get(day) ?? 0;
        cumulativeByDay[day] = Math.max(cumulativeByDay[day - 1], clamp(running, 0, period.total));
      }
      return { ...period, cumulativeByDay };
    });

  if (eligiblePeriods.length < MIN_HISTORICAL_PACE_MONTHS) return null;

  const pctByDay = Array.from({ length: currentDaysInPeriod + 1 }, () => 0);
  const points: PacePoint[] = [];
  for (let day = 1; day <= currentDaysInPeriod; day++) {
    const values = eligiblePeriods.map((period) => {
      const historyDay = Math.min(
        period.daysInPeriod,
        Math.max(1, Math.ceil((day / currentDaysInPeriod) * period.daysInPeriod)),
      );
      return clamp(period.cumulativeByDay[historyDay] / period.total, 0, 1);
    });
    const value = median(values);
    pctByDay[day] = value;
    points.push({ day, value });
  }

  return { points, pctByDay };
}

function parseDayBucketDate(date: string): { year: number; month: number; day: number } | null {
  const [yearRaw, monthRaw, dayRaw] = date.split("-");
  const year = Number(yearRaw);
  const month = Number(monthRaw);
  const day = Number(dayRaw);
  if (!Number.isInteger(year) || !Number.isInteger(month) || !Number.isInteger(day)) return null;
  if (month < 1 || month > 12 || day < 1 || day > 31) return null;
  return { year, month, day };
}

function formatCycleEndpoint(date: { year: number; month: number; day: number }): string {
  return new Date(Date.UTC(date.year, date.month - 1, date.day)).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    timeZone: "UTC",
  });
}

function toSvgPath(
  points: PacePoint[],
  xForDay: (day: number) => number,
  yForVal: (value: number) => number,
): string {
  if (!points.length) return "";
  return (
    "M " +
    points.map((p) => `${xForDay(p.day).toFixed(2)} ${yForVal(p.value).toFixed(2)}`).join(" L ")
  );
}

function median(values: number[]): number {
  if (!values.length) return 0;
  const sorted = [...values].sort((a, b) => a - b);
  const mid = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 1 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2;
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

function BudgetCardHeaderActions({
  monthLabel,
  monthKey,
  onPreviousMonth,
  onNextMonth,
  canGoNextMonth,
}: {
  monthLabel: string;
  monthKey: string;
  onPreviousMonth: () => void;
  onNextMonth: () => void;
  canGoNextMonth: boolean;
}) {
  const { t } = useTranslation();
  return (
    <div className="flex items-center gap-1.5">
      <div className="bg-card inline-flex items-center rounded-full p-1 shadow-sm">
        <button
          type="button"
          onClick={onPreviousMonth}
          className="hover:bg-foreground/5 flex h-7 w-7 items-center justify-center rounded-full transition-colors"
          aria-label={t("spending:budgetChart.previousBudgetPeriod")}
        >
          <Icons.ChevronLeft className="h-3.5 w-3.5" />
        </button>
        <span className="text-foreground min-w-[64px] whitespace-nowrap px-1 text-center text-[13px] tabular-nums">
          {monthLabel}
        </span>
        <button
          type="button"
          onClick={onNextMonth}
          disabled={!canGoNextMonth}
          className="hover:bg-foreground/5 disabled:text-muted-foreground/40 flex h-7 w-7 items-center justify-center rounded-full transition-colors disabled:cursor-not-allowed"
          aria-label={t("spending:budgetChart.nextBudgetPeriod")}
        >
          <Icons.ChevronRight className="h-3.5 w-3.5" />
        </button>
      </div>
      <BudgetManageLink monthKey={monthKey} />
    </div>
  );
}

const BudgetManageLink = ({ monthKey }: { monthKey: string }) => {
  const { t } = useTranslation();
  return (
    <Link
      to={`/spending/budget?period=${monthKey}`}
      className="text-muted-foreground hover:text-foreground text-sm underline-offset-4 hover:underline"
    >
      {t("spending:budgetChart.manage")}
    </Link>
  );
};

const BUDGET_NEAR_LIMIT = "#C28B47";

/** Green while there is room, amber in the last 15%, red once over. */
function budgetStatusColor(pct: number): string {
  return pct > 1 ? "var(--destructive)" : pct > 0.85 ? BUDGET_NEAR_LIMIT : "var(--success)";
}

const RING_SIZE = 64;
const RING_STROKE = 3;
/** The tinted disc behind the icon, the same circle a transaction row uses. */
const RING_DISC = 46;
const RING_RADIUS = (RING_SIZE - RING_STROKE) / 2;
const RING_LENGTH = 2 * Math.PI * RING_RADIUS;
/** How many irregular categories are named under the bar before the rest are counted. */
const IRREGULAR_LEGEND = 4;

/** Budget use as one ring per category, fullest first, in a grid that never scrolls sideways. */
export function BudgetRings({
  allocations,
  unbudgeted,
  spendingBreakdown,
  categoriesMeta,
  currency,
  activityRange,
  monthKey,
}: {
  allocations: BudgetCategoryRow[];
  /** Every spending category of the cycle; the ones without a budget feed the irregular bar. */
  unbudgeted: BudgetCategoryRow[];
  spendingBreakdown: { categoryId: string; amount: number; count: number }[];
  categoriesMeta: CategoryMetaMap;
  currency: string;
  activityRange: { from: string; to: string };
  monthKey: string;
}) {
  const { t } = useTranslation();
  const { isBalanceHidden } = useBalancePrivacy();
  // Every category with a budget; the bar totals them all, the rings show the eight fullest.
  const budgetedRows = useMemo(() => {
    const spentByTop = new Map<string, number>();
    for (const row of spendingBreakdown) {
      const topId = topCategoryId(row.categoryId, categoriesMeta);
      spentByTop.set(topId, (spentByTop.get(topId) ?? 0) + row.amount);
    }
    return allocations
      .filter((al) => (al.target || 0) > 0)
      .map((al) => {
        const meta = categoriesMeta.get(al.categoryId);
        const spent = Math.max(0, spentByTop.get(al.categoryId) ?? 0);
        return {
          categoryId: al.categoryId,
          name: meta?.name ?? al.categoryId,
          color: meta?.color ?? "var(--muted-foreground)",
          icon: meta?.icon ?? null,
          target: al.target,
          spent,
          pct: spent / al.target,
        };
      })
      .sort((x, y) => y.pct - x.pct);
  }, [allocations, spendingBreakdown, categoriesMeta]);
  const budgeted = useMemo(
    () => ({
      spent: budgetedRows.reduce((sum, row) => sum + row.spent, 0),
      target: budgetedRows.reduce((sum, row) => sum + row.target, 0),
    }),
    [budgetedRows],
  );
  const rings = budgetedRows.slice(0, 8);

  // Irregular spending: categories the plan gives nothing to but that were spent in,
  // whether they sit in a group, in an excluded group, or in no group at all.
  const irregular = useMemo(() => {
    const rows = unbudgeted
      .filter((row) => (row.target || 0) <= 0 && row.actual > 0)
      .map((row) => ({
        categoryId: row.categoryId,
        name: row.name,
        color: row.color ?? "var(--muted-foreground)",
        spent: row.actual,
      }))
      .sort((x, y) => y.spent - x.spent);
    return { rows, total: rows.reduce((sum, row) => sum + row.spent, 0) };
  }, [unbudgeted]);

  if (rings.length === 0) {
    return (
      <div className="text-muted-foreground text-sm">
        {t("spending:budgetBars.noBudgets")}{" "}
        <Link
          to="/settings/spending/setup"
          className="hover:text-foreground underline-offset-4 hover:underline"
        >
          {t("spending:budgetBars.setOne")}
        </Link>
      </div>
    );
  }

  return (
    <div>
      <div className="mb-6">
        <div className="flex items-baseline justify-between gap-3 pb-2">
          <span className="text-[15px]">{t("spending:budgetChart.budgetedTitle")}</span>
          <span className="text-[15px] tabular-nums">
            {isBalanceHidden
              ? "••••"
              : t("spending:budgetChart.budgetedOf", {
                  spent: formatCompactAmount(budgeted.spent, currency),
                  budget: formatCompactAmount(budgeted.target, currency),
                })}
          </span>
        </div>
        <BlockBar
          segments={[
            {
              value: budgeted.spent,
              color: budgetStatusColor(budgeted.spent / budgeted.target),
            },
          ]}
          total={budgeted.target}
        />
      </div>
      <div className="grid grid-cols-4 gap-x-2 gap-y-4">
        {rings.map((ring) => {
          const color = budgetStatusColor(ring.pct);
          const over = ring.spent > ring.target;
          return (
            <Link
              key={ring.categoryId}
              to={`/activities?tab=spending&category=${encodeURIComponent(ring.categoryId)}&from=${
                activityRange.from
              }&to=${activityRange.to}&budget=1`}
              className="flex min-w-0 flex-col items-center gap-1 text-center"
            >
              <div className="relative" style={{ width: RING_SIZE, height: RING_SIZE }}>
                <svg width={RING_SIZE} height={RING_SIZE} className="-rotate-90">
                  <circle
                    cx={RING_SIZE / 2}
                    cy={RING_SIZE / 2}
                    r={RING_RADIUS}
                    fill="none"
                    stroke="var(--foreground)"
                    strokeOpacity={0.09}
                    strokeWidth={RING_STROKE}
                  />
                  <circle
                    cx={RING_SIZE / 2}
                    cy={RING_SIZE / 2}
                    r={RING_RADIUS}
                    fill="none"
                    stroke={color}
                    strokeWidth={RING_STROKE}
                    strokeLinecap="round"
                    strokeDasharray={`${RING_LENGTH * Math.min(1, ring.pct)} ${RING_LENGTH}`}
                  />
                </svg>
                <div className="absolute inset-0 flex items-center justify-center">
                  <span
                    className="flex items-center justify-center rounded-full"
                    style={{
                      width: RING_DISC,
                      height: RING_DISC,
                      color: ring.color,
                      backgroundColor: `color-mix(in srgb, ${ring.color} 16%, transparent)`,
                    }}
                  >
                    <CategoryIcon icon={ring.icon} fallback={ring.name} className="h-6 w-6" />
                  </span>
                </div>
              </div>
              <div className="text-muted-foreground w-full truncate text-[11px]">{ring.name}</div>
              <div
                className={cn("w-full truncate text-xs tabular-nums", over && "text-destructive")}
              >
                {isBalanceHidden
                  ? "••••"
                  : formatCompactAmount(Math.abs(ring.target - ring.spent), currency)}{" "}
                <span className={over ? undefined : "text-muted-foreground"}>
                  {over ? t("spending:budgetChart.overLower") : t("spending:budgetChart.leftLower")}
                </span>
              </div>
            </Link>
          );
        })}
      </div>
      {irregular.total > 0 && (
        <div className="mt-6">
          <div className="flex items-baseline justify-between gap-3 pb-2">
            <span className="text-[15px]">{t("spending:budgetChart.irregularTitle")}</span>
            <span className="text-[15px] tabular-nums">
              {isBalanceHidden ? "••••" : formatCompactAmount(irregular.total, currency)}
            </span>
          </div>
          <BlockBar
            segments={irregular.rows.map((row) => ({
              value: row.spent,
              color: row.color,
              label: row.name,
            }))}
          />
          <div className="mt-2 flex flex-wrap gap-x-4 gap-y-1 text-[13px]">
            {irregular.rows.slice(0, IRREGULAR_LEGEND).map((row) => (
              <Link
                key={row.categoryId}
                to={`/activities?tab=spending&category=${encodeURIComponent(row.categoryId)}&from=${
                  activityRange.from
                }&to=${activityRange.to}&budget=1`}
                className="inline-flex min-w-0 items-center gap-1.5"
              >
                <span
                  className="h-2 w-2 shrink-0 rounded-full"
                  style={{ backgroundColor: row.color }}
                />
                <span className="truncate">{row.name}</span>
                <span className="text-muted-foreground tabular-nums">
                  {isBalanceHidden ? "••••" : formatCompactAmount(row.spent, currency)}
                </span>
              </Link>
            ))}
            {irregular.rows.length > IRREGULAR_LEGEND && (
              <span className="text-muted-foreground">
                {t("spending:budgetChart.irregularMore", {
                  count: irregular.rows.length - IRREGULAR_LEGEND,
                })}
              </span>
            )}
          </div>
        </div>
      )}
      <div className="mt-4">
        <BudgetManageLink monthKey={monthKey} />
      </div>
    </div>
  );
}
