import type { DateRange } from "@/lib/types";

import type { BudgetCycleSettings } from "../types";
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
export const DEFAULT_BUDGET_CYCLE: BudgetCycleSettings = {
  cadence: "monthly",
  anchorDate: "2026-07-25",
};

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

export interface BudgetPeriodParts {
  start: ZonedCalendarDate;
  end: ZonedCalendarDate;
}

export function budgetPeriodKeyForDate(
  date: ZonedCalendarDate,
  cycle: BudgetCycleSettings = DEFAULT_BUDGET_CYCLE,
): string {
  return formatDateKey(budgetPeriodForDate(date, cycle).start);
}

export function currentBudgetPeriodKey(
  timezone?: string | null,
  cycle: BudgetCycleSettings = DEFAULT_BUDGET_CYCLE,
  now: Date = new Date(),
): string {
  return budgetPeriodKeyForDate(getZonedDateParts(now, timezone), cycle);
}

export function budgetPeriodParts(
  periodKey: string,
  cycle: BudgetCycleSettings = DEFAULT_BUDGET_CYCLE,
): BudgetPeriodParts | null {
  const start = parseDateKey(periodKey);
  if (!start) return null;
  const period = budgetPeriodForDate(start, cycle);
  return sameDate(period.start, start) ? period : null;
}

export function shiftBudgetPeriodKey(
  periodKey: string,
  periods: number,
  cycle: BudgetCycleSettings = DEFAULT_BUDGET_CYCLE,
): string {
  const period = budgetPeriodParts(periodKey, cycle);
  const anchor = parseDateKey(cycle.anchorDate);
  if (!period || !anchor) return currentBudgetPeriodKey(undefined, cycle);

  if (cycle.cadence !== "monthly") {
    const days = cycle.cadence === "weekly" ? 7 : 14;
    return formatDateKey(addDays(period.start, periods * days));
  }

  const index = monthIndex(period.start) - monthIndex(anchor);
  return formatDateKey(monthlyOccurrence(anchor, index + periods));
}

export function budgetPeriodRange(
  periodKey: string,
  cycle: BudgetCycleSettings = DEFAULT_BUDGET_CYCLE,
): { from: Date; to: Date } {
  const period = budgetPeriodParts(periodKey, cycle);
  if (!period) return budgetPeriodRange(currentBudgetPeriodKey(undefined, cycle), cycle);
  return {
    from: localDateFromParts(period.start),
    to: localDateFromParts(period.end),
  };
}

export function budgetPeriodDayIndex(
  dateKey: string,
  periodKey: string,
  cycle: BudgetCycleSettings = DEFAULT_BUDGET_CYCLE,
): number | null {
  const date = parseDateKey(dateKey);
  const period = budgetPeriodParts(periodKey, cycle);
  if (!date || !period) return null;
  const index = calendarDaysBetweenInclusive(period.start, date);
  const periodDays = calendarDaysBetweenInclusive(period.start, period.end);
  return index >= 1 && index <= periodDays ? index : null;
}

export function budgetPeriodLabel(
  periodKey: string,
  format: "long" | "short" = "short",
  locale?: string,
  cycle: BudgetCycleSettings = DEFAULT_BUDGET_CYCLE,
): string {
  const period = budgetPeriodParts(periodKey, cycle);
  if (!period) return "";
  const start = new Date(Date.UTC(period.start.year, period.start.month - 1, period.start.day));
  const end = new Date(Date.UTC(period.end.year, period.end.month - 1, period.end.day));
  const startLabel = start.toLocaleDateString(locale, {
    month: format,
    day: "numeric",
    year: period.start.year !== period.end.year ? "numeric" : undefined,
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

export function addMonthsToMonthKey(monthKey: string, months: number): string {
  const parts = parseMonthKey(monthKey);
  if (!parts) return monthKey;
  return monthKeyFromParts(addCalendarMonths({ ...parts, day: 1 }, months));
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

function budgetPeriodForDate(
  date: ZonedCalendarDate,
  cycle: BudgetCycleSettings,
): BudgetPeriodParts {
  const anchor = parseDateKey(cycle.anchorDate) ?? parseDateKey(DEFAULT_BUDGET_CYCLE.anchorDate)!;
  if (cycle.cadence !== "monthly") {
    const days = cycle.cadence === "weekly" ? 7 : 14;
    const index = Math.floor(daysBetween(anchor, date) / days);
    const start = addDays(anchor, index * days);
    return { start, end: addDays(start, days - 1) };
  }

  let index = monthIndex(date) - monthIndex(anchor);
  if (compareDates(monthlyOccurrence(anchor, index), date) > 0) index -= 1;
  const start = monthlyOccurrence(anchor, index);
  return { start, end: addDays(monthlyOccurrence(anchor, index + 1), -1) };
}

function monthlyOccurrence(anchor: ZonedCalendarDate, offset: number): ZonedCalendarDate {
  const absolute = monthIndex(anchor) + offset;
  const year = Math.floor(absolute / 12);
  const month = (((absolute % 12) + 12) % 12) + 1;
  const lastDay = daysInCalendarMonth(year, month);
  const anchorIsMonthEnd = anchor.day === daysInCalendarMonth(anchor.year, anchor.month);
  return {
    year,
    month,
    day: anchorIsMonthEnd ? lastDay : Math.min(anchor.day, lastDay),
  };
}

function monthIndex(date: Pick<ZonedCalendarDate, "year" | "month">): number {
  return date.year * 12 + date.month - 1;
}

function parseDateKey(value: string | null | undefined): ZonedCalendarDate | null {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value ?? "");
  if (!match) return null;
  const date = { year: Number(match[1]), month: Number(match[2]), day: Number(match[3]) };
  if (date.month < 1 || date.month > 12) return null;
  if (date.day < 1 || date.day > daysInCalendarMonth(date.year, date.month)) return null;
  return date;
}

function formatDateKey(date: ZonedCalendarDate): string {
  return `${date.year}-${String(date.month).padStart(2, "0")}-${String(date.day).padStart(2, "0")}`;
}

function addDays(date: ZonedCalendarDate, days: number): ZonedCalendarDate {
  const value = new Date(Date.UTC(date.year, date.month - 1, date.day + days));
  return { year: value.getUTCFullYear(), month: value.getUTCMonth() + 1, day: value.getUTCDate() };
}

function daysBetween(start: ZonedCalendarDate, end: ZonedCalendarDate): number {
  return Math.round(
    (Date.UTC(end.year, end.month - 1, end.day) -
      Date.UTC(start.year, start.month - 1, start.day)) /
      86_400_000,
  );
}

function compareDates(left: ZonedCalendarDate, right: ZonedCalendarDate): number {
  return daysBetween(right, left);
}

function sameDate(left: ZonedCalendarDate, right: ZonedCalendarDate): boolean {
  return left.year === right.year && left.month === right.month && left.day === right.day;
}
