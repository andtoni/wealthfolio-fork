import type { DateRange } from "@/lib/types";

import type { ReportsRange } from "./reports-period";
import {
  addCalendarMonths,
  calendarDaysBetweenInclusive,
  calendarMonthsBetweenInclusive,
  daysInCalendarMonth,
  getZonedDateParts,
  zonedCalendarDateBoundaryToDate,
  type ZonedCalendarDate,
} from "./timezone";

export const SPENDING_MONTH_PARAM = "spendingMonth";
export const SPENDING_MONTH_STORAGE_KEY = "spending-month";
export const BUDGET_CYCLE_START_DAY = 25;
export const BUDGET_CYCLE_END_DAY = 24;

export function localDateFromParts(date: ZonedCalendarDate): Date {
  return new Date(date.year, date.month - 1, date.day);
}

export function monthKeyFromParts(date: Pick<ZonedCalendarDate, "year" | "month">): string {
  return `${date.year}-${String(date.month).padStart(2, "0")}`;
}

export function parseMonthKey(
  value: string | null | undefined,
): { year: number; month: number } | null {
  if (!value || !/^\d{4}-(0[1-9]|1[0-2])$/.test(value)) return null;
  const [year, month] = value.split("-").map(Number);
  return { year, month };
}

export function currentMonthKey(timezone?: string | null): string {
  return monthKeyFromParts(getZonedDateParts(new Date(), timezone));
}

/** Budget period keys name the ending month: 2026-08 spans Jul 25–Aug 24. */
export function budgetPeriodKeyForDate(date: ZonedCalendarDate): string {
  const endingMonth =
    date.day >= BUDGET_CYCLE_START_DAY
      ? addCalendarMonths({ ...date, day: 1 }, 1)
      : { ...date, day: 1 };
  return monthKeyFromParts(endingMonth);
}

export function currentBudgetPeriodKey(timezone?: string | null, now: Date = new Date()): string {
  return budgetPeriodKeyForDate(getZonedDateParts(now, timezone));
}

export function addMonthsToMonthKey(monthKey: string, months: number): string {
  const parts = parseMonthKey(monthKey);
  if (!parts) return monthKey;
  return monthKeyFromParts(addCalendarMonths({ ...parts, day: 1 }, months));
}

export interface BudgetPeriodParts {
  start: ZonedCalendarDate;
  end: ZonedCalendarDate;
}

export function budgetPeriodParts(periodKey: string): BudgetPeriodParts | null {
  const endingMonth = parseMonthKey(periodKey);
  if (!endingMonth) return null;
  return {
    start: {
      ...addCalendarMonths({ ...endingMonth, day: 1 }, -1),
      day: BUDGET_CYCLE_START_DAY,
    },
    end: { ...endingMonth, day: BUDGET_CYCLE_END_DAY },
  };
}

export function budgetPeriodRange(periodKey: string): { from: Date; to: Date } {
  const period = budgetPeriodParts(periodKey);
  if (!period) return budgetPeriodRange(currentBudgetPeriodKey());
  return {
    from: localDateFromParts(period.start),
    to: localDateFromParts(period.end),
  };
}

export function budgetPeriodDayIndex(dateKey: string, periodKey: string): number | null {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(dateKey);
  const period = budgetPeriodParts(periodKey);
  if (!match || !period) return null;
  const date = { year: Number(match[1]), month: Number(match[2]), day: Number(match[3]) };
  const index = calendarDaysBetweenInclusive(period.start, date);
  const periodDays = calendarDaysBetweenInclusive(period.start, period.end);
  return index >= 1 && index <= periodDays ? index : null;
}

export function budgetPeriodLabel(
  periodKey: string,
  format: "long" | "short" = "short",
  locale?: string,
): string {
  const period = budgetPeriodParts(periodKey);
  if (!period) return "";
  const start = new Date(Date.UTC(period.start.year, period.start.month - 1, period.start.day));
  const end = new Date(Date.UTC(period.end.year, period.end.month - 1, period.end.day));
  const startLabel = start.toLocaleDateString(locale, {
    month: format,
    day: "numeric",
    timeZone: "UTC",
  });
  const endLabel = end.toLocaleDateString(locale, {
    month: format,
    day: "numeric",
    year: "numeric",
    timeZone: "UTC",
  });
  return `${startLabel} – ${endLabel}`;
}

export function monthRange(monthKey: string): DateRange {
  const parts = parseMonthKey(monthKey) ?? parseMonthKey(currentMonthKey());
  const month = parts ?? getZonedDateParts(new Date());
  const start = { ...month, day: 1 };
  const end = { ...month, day: daysInCalendarMonth(month.year, month.month) };
  return { from: localDateFromParts(start), to: localDateFromParts(end) };
}

export function monthReportsRange(monthKey: string, timezone?: string | null): ReportsRange | null {
  const month = parseMonthKey(monthKey);
  if (!month) return null;
  const start = { ...month, day: 1 };
  const end = { ...month, day: daysInCalendarMonth(month.year, month.month) };
  return {
    start: zonedCalendarDateBoundaryToDate(start, "start", timezone),
    end: zonedCalendarDateBoundaryToDate(end, "end", timezone),
    days: calendarDaysBetweenInclusive(start, end),
    months: calendarMonthsBetweenInclusive(start, end),
  };
}

export function monthLabel(monthKey: string, format: "long" | "short" = "long"): string {
  const parts = parseMonthKey(monthKey);
  if (!parts) return "";
  return new Date(parts.year, parts.month - 1, 1).toLocaleString(undefined, {
    month: format,
    year: "numeric",
  });
}

export function compactMonthLabel(monthKey: string): string {
  const parts = parseMonthKey(monthKey);
  if (!parts) return "";
  const month = new Date(parts.year, parts.month - 1, 1).toLocaleString(undefined, {
    month: "short",
  });
  return `${month} '${String(parts.year).slice(2)}`;
}
