import {
  useMemo,
  useRef,
  type MouseEvent as ReactMouseEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";
import { Link } from "react-router-dom";
import { useTranslation } from "react-i18next";

import { DashboardCard } from "@/components/dashboard-card";
import { cn } from "@/lib/utils";
import { formatCompactAmount, Icons, PrivacyAmount, useBalancePrivacy } from "@wealthfolio/ui";

import { CategoryIcon, type CategoryMetaMap } from "./category-chips";
import { topCategoryId } from "../lib/category-rollup";
import {
  budgetPeriodDayIndex,
  budgetPeriodKeyForDate,
  budgetPeriodLabel,
  budgetPeriodParts,
  compactMonthLabel,
} from "../lib/month-period";
import { calendarDaysBetweenInclusive } from "../lib/timezone";
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

function useHorizontalDragScroll() {
  const drag = useRef<{
    pointerId: number;
    startX: number;
    startY: number;
    scrollLeft: number;
    dragging: boolean;
  } | null>(null);
  const blockNextClick = useRef(false);

  const clear = (event: ReactPointerEvent<HTMLDivElement>) => {
    const state = drag.current;
    if (!state || state.pointerId !== event.pointerId) return;
    if (state.dragging) {
      blockNextClick.current = true;
      window.setTimeout(() => {
        blockNextClick.current = false;
      }, 0);
    }
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    drag.current = null;
  };

  return {
    onPointerDown: (event: ReactPointerEvent<HTMLDivElement>) => {
      if (!event.isPrimary || event.pointerType === "mouse") return;
      drag.current = {
        pointerId: event.pointerId,
        startX: event.clientX,
        startY: event.clientY,
        scrollLeft: event.currentTarget.scrollLeft,
        dragging: false,
      };
      event.currentTarget.setPointerCapture(event.pointerId);
    },
    onPointerMove: (event: ReactPointerEvent<HTMLDivElement>) => {
      const state = drag.current;
      if (!state || state.pointerId !== event.pointerId) return;
      const deltaX = event.clientX - state.startX;
      const deltaY = event.clientY - state.startY;
      if (!state.dragging) {
        if (Math.abs(deltaX) < 6 || Math.abs(deltaX) <= Math.abs(deltaY)) return;
        state.dragging = true;
      }
      event.preventDefault();
      event.currentTarget.scrollLeft = state.scrollLeft - deltaX;
    },
    onPointerUp: clear,
    onPointerCancel: clear,
    onClickCapture: (event: ReactMouseEvent<HTMLDivElement>) => {
      if (!blockNextClick.current) return;
      blockNextClick.current = false;
      event.preventDefault();
      event.stopPropagation();
    },
  };
}

function parseMonthKey(value: string | null | undefined): { year: number; month: number } | null {
  if (!value || !/^\d{4}-(0[1-9]|1[0-2])$/.test(value)) return null;
  const [year, month] = value.split("-").map(Number);
  return { year, month };
}

const STATUS_ACCENTS: Record<
  Status,
  {
    lineColor: string;
    pillBg: string;
    accent: string;
    Icon: typeof Icons.AlertCircle;
    labelKey: string;
  }
> = {
  over: {
    lineColor: "#B85544",
    pillBg: "var(--destructive)",
    accent: "var(--destructive)",
    Icon: Icons.AlertTriangle,
    labelKey: "spending:budgetChart.overBudget",
  },
  warn: {
    lineColor: "#C28B47",
    pillBg: "#C28B47",
    accent: "#C28B47",
    Icon: Icons.AlertCircle,
    labelKey: "spending:budgetChart.trendingHigh",
  },
  ok: {
    lineColor: "hsl(73 84% 27%)",
    pillBg: "hsl(73 84% 27%)",
    accent: "var(--success)",
    Icon: Icons.CheckCircle ?? Icons.AlertCircle,
    labelKey: "spending:budgetChart.onTrack",
  },
};

export function BudgetLineChartCard({
  monthKey,
  today,
  isCurrentMonth,
  onPreviousMonth,
  onNextMonth,
  canGoNextMonth,
  activityRange,
  target,
  spent,
  currency,
  historicalDailyAvg,
  allocations,
  spendingBreakdown,
  categoriesMeta,
  monthByDay,
  historicalByDay,
}: {
  monthKey: string;
  today: BudgetToday;
  isCurrentMonth: boolean;
  onPreviousMonth: () => void;
  onNextMonth: () => void;
  canGoNextMonth: boolean;
  activityRange: { from: string; to: string };
  target: number;
  spent: number;
  currency: string;
  historicalDailyAvg: number;
  allocations: BudgetCategoryRow[];
  spendingBreakdown: { categoryId: string; amount: number; count: number }[];
  categoriesMeta: CategoryMetaMap;
  monthByDay: DayBucket[];
  historicalByDay: DayBucket[];
}) {
  const { t } = useTranslation();
  const ringScrollHandlers = useHorizontalDragScroll();
  // All hooks must run unconditionally — the `target <= 0` early return below
  // sits between hooks otherwise, which trips "Rendered more hooks than during
  // the previous render" when a target is added or cleared.
  const monthMeta = useMemo(() => {
    const period = budgetPeriodParts(monthKey);
    const fallback = parseMonthKey(monthKey) ?? today;
    const start = period?.start ?? { ...fallback, day: 1 };
    const end = period?.end ?? {
      ...fallback,
      day: new Date(fallback.year, fallback.month, 0).getDate(),
    };
    const daysInPeriod = calendarDaysBetweenInclusive(start, end);
    const todayIndex = calendarDaysBetweenInclusive(start, today);
    const dayOfPeriod = isCurrentMonth
      ? Math.min(daysInPeriod, Math.max(1, todayIndex))
      : daysInPeriod;
    return {
      dayOfPeriod,
      daysInPeriod,
      daysRemaining: isCurrentMonth ? Math.max(0, daysInPeriod - dayOfPeriod) : 0,
      monthLabel: budgetPeriodLabel(monthKey, "short").toUpperCase(),
      shortLabel: compactMonthLabel(monthKey).toUpperCase(),
      startLabel: formatCycleEndpoint(start),
      endLabel: formatCycleEndpoint(end),
    };
  }, [monthKey, isCurrentMonth, today]);
  const { dayOfPeriod, daysInPeriod, daysRemaining, monthLabel } = monthMeta;

  const cumulative = useMemo(() => {
    const byDay = new Map<number, number>();
    for (const bucket of monthByDay) {
      const day = budgetPeriodDayIndex(bucket.date, monthKey);
      if (day !== null) byDay.set(day, (byDay.get(day) ?? 0) + bucket.outflow);
    }
    let running = 0;
    const out: { day: number; value: number }[] = [];
    for (let day = 1; day <= dayOfPeriod; day++) {
      running += byDay.get(day) ?? 0;
      out.push({ day, value: running });
    }
    return out;
  }, [monthByDay, monthKey, dayOfPeriod]);

  const rings = useMemo(() => {
    const spentByTop = new Map<string, number>();
    for (const row of spendingBreakdown) {
      const topId = topCategoryId(row.categoryId, categoriesMeta);
      spentByTop.set(topId, (spentByTop.get(topId) ?? 0) + row.amount);
    }
    return allocations
      .map((al) => {
        const t = al.target || 0;
        if (t <= 0) return null;
        const meta = categoriesMeta.get(al.categoryId);
        const s = spentByTop.get(al.categoryId) ?? 0;
        return {
          id: al.categoryId,
          categoryId: al.categoryId,
          name: meta?.name ?? al.categoryId,
          color: meta?.color ?? null,
          icon: meta?.icon ?? null,
          target: t,
          spent: Math.max(0, s),
          pct: Math.max(0, s) / t,
        };
      })
      .filter((r): r is NonNullable<typeof r> => r !== null)
      .sort((x, y) => y.pct - x.pct);
  }, [allocations, spendingBreakdown, categoriesMeta]);

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

  const historicalPace = useMemo(
    () => buildHistoricalPaceCurve(historicalByDay, daysInPeriod),
    [historicalByDay, daysInPeriod],
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
        title={t("spending:budgetChart.monthlyBudget")}
        subtitle={monthMeta.monthLabel}
        action={headerAction}
        className="text-center"
      >
        <p className="text-muted-foreground text-sm">{t("spending:budgetChart.noTarget")}</p>
        <Link
          to={`/spending/budget?month=${monthKey}`}
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
  const { Icon } = a;
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
  const pillLeftPct = Math.min(78, Math.max(8, pillLeftPctRaw - 4));
  // Near the right edge, anchor the badge to the endpoint and grow leftward so
  // it never overflows the card.
  const pillFlip = pillLeftPctRaw > 55;
  const pillTopPx = Math.max(0, endY - 28);

  return (
    <DashboardCard
      title={t("spending:budgetChart.monthlyBudget")}
      subtitle={monthLabel}
      action={headerAction}
    >
      <div className="flex items-center gap-2">
        <Icon className="h-4 w-4 shrink-0" style={{ color: a.accent }} />
        <span className="text-foreground text-sm font-semibold">{statusLabel}</span>
        <span className="text-muted-foreground/70 ml-auto text-xs tabular-nums">
          {isCurrentMonth
            ? t("spending:budgetChart.dayOf", { day: dayOfPeriod, total: daysInPeriod })
            : t("spending:budgetChart.closed")}
        </span>
      </div>

      <div className="mt-3">
        {isCurrentMonth && willOverspend && forecastDelta > target * 0.05 ? (
          <>
            <div className="text-foreground text-2xl font-bold tabular-nums tracking-tight">
              <PrivacyAmount value={forecast} currency={currency} />{" "}
              <span className="text-muted-foreground/70 text-base font-medium">
                {t("spending:budgetChart.forecastLower")}
              </span>
            </div>
            <div className="text-destructive mt-0.5 inline-flex items-center gap-1 text-xs font-semibold tabular-nums">
              <Icons.ArrowUp className="h-3 w-3" />
              <PrivacyAmount value={forecastDelta} currency={currency} />{" "}
              {t("spending:budgetChart.overBudgetLower")}
            </div>
            <div className="text-muted-foreground/80 mt-0.5 text-xs tabular-nums">
              <PrivacyAmount value={remaining} currency={currency} />{" "}
              {t("spending:budgetChart.leftTodayOf")}{" "}
              <PrivacyAmount value={target} currency={currency} />{" "}
              {t("spending:budgetChart.budgetedThisMonth")}
            </div>
          </>
        ) : !isCurrentMonth ? (
          <>
            <div className="text-foreground text-2xl font-bold tabular-nums tracking-tight">
              <PrivacyAmount value={spent} currency={currency} />{" "}
              <span className="text-muted-foreground/70 text-base font-medium">
                {t("spending:budgetChart.spentLower")}
              </span>
            </div>
            <div
              className={cn(
                "mt-0.5 inline-flex items-center gap-1 text-xs font-semibold tabular-nums",
                isOver ? "text-destructive" : "text-success",
              )}
            >
              <PrivacyAmount value={isOver ? overBy : remaining} currency={currency} />{" "}
              {isOver
                ? t("spending:budgetChart.overBudgetLower")
                : t("spending:budgetChart.leftLower")}
            </div>
            <div className="text-muted-foreground/80 mt-0.5 text-xs tabular-nums">
              {t("spending:budgetChart.ofLower")}{" "}
              <PrivacyAmount value={target} currency={currency} />{" "}
              {t("spending:budgetChart.budgetedLower")}
            </div>
          </>
        ) : (
          <>
            <div className="text-foreground text-2xl font-bold tabular-nums tracking-tight">
              <PrivacyAmount value={isOver ? overBy : remaining} currency={currency} />{" "}
              <span className="text-muted-foreground/70 text-base font-medium">
                {isOver ? t("spending:budgetChart.overLower") : t("spending:budgetChart.leftLower")}
              </span>
            </div>
            <div className="text-muted-foreground/80 text-xs tabular-nums">
              {t("spending:budgetChart.ofLower")}{" "}
              <PrivacyAmount value={target} currency={currency} />{" "}
              {t("spending:budgetChart.budgetedThisMonth")}
            </div>
          </>
        )}
      </div>

      <div className="relative mt-4 w-full">
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
              strokeDasharray="3 4"
              strokeWidth={1.25}
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
          {actualPath && (
            <path
              d={actualPath}
              fill="none"
              stroke={a.lineColor}
              strokeWidth={2.5}
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
            className="absolute h-[9px] w-[9px] rounded-full bg-white"
            style={{
              left: `${pillLeftPctRaw}%`,
              top: `${endY}px`,
              transform: "translate(-50%, -50%)",
              border: `2.5px solid ${a.lineColor}`,
            }}
          />
        )}
        {cumulative.length > 0 && (
          <div
            className="absolute whitespace-nowrap rounded-full px-2 py-0.5 text-[11px] font-semibold tabular-nums shadow-sm"
            style={{
              left: `${pillFlip ? pillLeftPctRaw : pillLeftPct}%`,
              top: `${pillTopPx}px`,
              transform: pillFlip ? "translateX(calc(-100% - 6px))" : undefined,
              backgroundColor: a.pillBg,
              color: "white",
            }}
          >
            {gapLabel}
          </div>
        )}
      </div>
      <div className="text-muted-foreground/70 mt-1 flex justify-between text-[10px] tabular-nums">
        <span>{monthMeta.startLabel}</span>
        <span>{monthMeta.endLabel}</span>
      </div>

      <div className="border-border mt-4 grid grid-cols-2 gap-3 border-t pt-3 text-xs">
        <div>
          <div className="text-muted-foreground/70 text-[11px] uppercase tracking-wide">
            {isCurrentMonth
              ? t("spending:budgetChart.spentSoFar")
              : t("spending:budgetChart.spentUpper")}
          </div>
          <div className="text-foreground text-sm font-semibold tabular-nums">
            <PrivacyAmount value={spent} currency={currency} />
          </div>
        </div>
        <div className="text-right">
          <div className="text-muted-foreground/70 text-[11px] uppercase tracking-wide">
            {isCurrentMonth
              ? t("spending:budgetChart.forecastUpper")
              : t("spending:budgetChart.result")}
          </div>
          {isCurrentMonth ? (
            <div
              className={cn(
                "text-sm font-semibold tabular-nums",
                forecastReliable
                  ? willOverspend
                    ? "text-destructive"
                    : "text-foreground"
                  : "text-muted-foreground/60",
              )}
            >
              {forecastReliable ? <PrivacyAmount value={forecast} currency={currency} /> : "—"}
            </div>
          ) : (
            <div
              className={cn(
                "text-sm font-semibold tabular-nums",
                isOver ? "text-destructive" : "text-foreground",
              )}
            >
              <PrivacyAmount value={isOver ? overBy : remaining} currency={currency} />
            </div>
          )}
          <div className="text-muted-foreground/60 text-[10px]">
            {isCurrentMonth
              ? forecastReliable
                ? haveHistory
                  ? t("spending:budgetChart.vsLast3Months")
                  : t("spending:budgetChart.atCurrentPace")
                : t("spending:budgetChart.moreDataNeeded")
              : isOver
                ? t("spending:budgetChart.overBudgetLower")
                : t("spending:budgetChart.leftLower")}
          </div>
        </div>
      </div>

      <div className="border-border mt-5 border-t pt-4">
        <div className="mb-3 flex items-center justify-between">
          <span className="text-muted-foreground/80 text-[11px] font-semibold uppercase tracking-wide">
            {t("spending:budgetChart.byCategory")}
          </span>
          <BudgetManageLink monthKey={monthKey} />
        </div>
        {rings.length === 0 ? (
          <div className="text-muted-foreground py-2 text-center text-xs">
            {t("spending:budgetBars.noBudgets")}{" "}
            <Link
              to="/settings/spending/setup"
              className="hover:text-foreground underline-offset-4 hover:underline"
            >
              {t("spending:budgetBars.setOne")}
            </Link>
          </div>
        ) : (
          <div
            data-no-swipe-drag
            {...ringScrollHandlers}
            className="-mx-1 flex min-w-0 cursor-grab touch-pan-y gap-3 overflow-x-auto overscroll-x-contain px-1 pb-1 [-webkit-overflow-scrolling:touch] [scrollbar-width:none] active:cursor-grabbing [&::-webkit-scrollbar]:hidden"
            style={{
              maskImage: "linear-gradient(to right, black calc(100% - 32px), transparent 100%)",
              WebkitMaskImage:
                "linear-gradient(to right, black calc(100% - 32px), transparent 100%)",
            }}
          >
            {rings.map((r) => (
              <BudgetRing key={r.id} ring={r} currency={currency} activityRange={activityRange} />
            ))}
          </div>
        )}
      </div>
    </DashboardCard>
  );
}

function buildHistoricalPaceCurve(
  byDay: DayBucket[],
  currentDaysInPeriod: number,
): { points: PacePoint[]; pctByDay: number[] } | null {
  const periods = new Map<
    string,
    { daysInPeriod: number; outflowByDay: Map<number, number>; total: number }
  >();

  for (const bucket of byDay) {
    const parsed = parseDayBucketDate(bucket.date);
    if (!parsed) continue;
    const periodKey = budgetPeriodKeyForDate(parsed);
    const period = budgetPeriodParts(periodKey);
    const day = budgetPeriodDayIndex(bucket.date, periodKey);
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
  return new Date(Date.UTC(date.year, date.month - 1, date.day))
    .toLocaleDateString(undefined, { month: "short", day: "numeric", timeZone: "UTC" })
    .toUpperCase();
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
      <div className="bg-muted/60 inline-flex items-center rounded-full p-0.5">
        <button
          type="button"
          onClick={onPreviousMonth}
          className="hover:bg-background flex h-6 w-6 items-center justify-center rounded-full transition-colors"
          aria-label={t("spending:budgetChart.previousBudgetMonth")}
        >
          <Icons.ChevronLeft className="h-3.5 w-3.5" />
        </button>
        <span className="text-foreground min-w-[74px] whitespace-nowrap px-1 text-center text-[11px] font-medium tabular-nums">
          {monthLabel}
        </span>
        <button
          type="button"
          onClick={onNextMonth}
          disabled={!canGoNextMonth}
          className="hover:bg-background disabled:text-muted-foreground/40 flex h-6 w-6 items-center justify-center rounded-full transition-colors disabled:cursor-not-allowed"
          aria-label={t("spending:budgetChart.nextBudgetMonth")}
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
      to={`/spending/budget?month=${monthKey}`}
      className="text-muted-foreground hover:text-foreground text-xs underline-offset-4 hover:underline"
    >
      {t("spending:budgetChart.manage")}
    </Link>
  );
};

function BudgetRing({
  ring,
  currency,
  activityRange,
}: {
  ring: {
    categoryId: string;
    name: string;
    color: string | null;
    icon: string | null;
    target: number;
    spent: number;
    pct: number;
  };
  currency: string;
  activityRange: { from: string; to: string };
}) {
  const { t } = useTranslation();
  const { isBalanceHidden } = useBalancePrivacy();
  const isOver = ring.spent > ring.target;
  const remaining = ring.target - ring.spent;
  const ringColor = isOver ? "var(--destructive)" : ring.pct > 0.85 ? "#C28B47" : "var(--success)";
  const displayAmount = Math.abs(isOver ? ring.spent - ring.target : remaining);

  const size = 56;
  const stroke = 4;
  const r = (size - stroke) / 2;
  const c = 2 * Math.PI * r;
  const fillPct = Math.min(1, ring.pct);
  const dash = `${c * fillPct} ${c}`;

  return (
    <Link
      to={`/activities?tab=spending&category=${encodeURIComponent(ring.categoryId)}&from=${
        activityRange.from
      }&to=${activityRange.to}`}
      className="hover:bg-muted/40 flex w-16 shrink-0 flex-col items-center gap-1 rounded-md px-1 py-1 transition-colors"
      title={`${ring.name}: ${ring.spent.toFixed(2)} / ${ring.target.toFixed(2)}`}
    >
      <div className="relative" style={{ width: size, height: size }}>
        <svg width={size} height={size} className="-rotate-90">
          <circle
            cx={size / 2}
            cy={size / 2}
            r={r}
            fill="none"
            stroke={ringColor}
            strokeOpacity={0.22}
            strokeWidth={stroke}
          />
          <circle
            cx={size / 2}
            cy={size / 2}
            r={r}
            fill="none"
            stroke={ringColor}
            strokeWidth={stroke}
            strokeLinecap="round"
            strokeDasharray={dash}
          />
        </svg>
        <div
          className="absolute inset-0 flex items-center justify-center"
          style={{ color: ring.color ?? ringColor }}
        >
          <CategoryIcon icon={ring.icon} fallback={ring.name} className="h-5 w-5" />
        </div>
      </div>
      <div className="text-foreground text-xs font-semibold tabular-nums">
        {isBalanceHidden ? "••••" : formatCompactAmount(displayAmount, currency)}
      </div>
      <div
        className={cn(
          "text-[10px] uppercase tracking-wide",
          isOver ? "text-destructive" : "text-muted-foreground/70",
        )}
      >
        {isOver ? t("spending:budgetChart.overLower") : t("spending:budgetChart.leftLower")}
      </div>
    </Link>
  );
}
