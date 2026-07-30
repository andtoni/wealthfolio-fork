import { afterEach, describe, expect, it, vi } from "vitest";

import {
  addMonthsToMonthKey,
  budgetPeriodDayIndex,
  budgetPeriodKeyForDate,
  budgetPeriodLabel,
  budgetPeriodParts,
  currentBudgetPeriodKey,
  currentMonthKey,
  monthReportsRange,
  parseMonthKey,
} from "./month-period";
import { zonedCalendarDateBoundaryToDate } from "./timezone";

describe("spending month periods", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("parses and shifts valid month keys", () => {
    expect(parseMonthKey("2026-05")).toEqual({ year: 2026, month: 5 });
    expect(parseMonthKey("2026-13")).toBeNull();
    expect(addMonthsToMonthKey("2026-01", -1)).toBe("2025-12");
  });

  it("resolves a month key to a full calendar month report range", () => {
    const range = monthReportsRange("2026-05", "UTC");

    expect(range?.start.toISOString()).toBe("2026-05-01T00:00:00.000Z");
    expect(range?.end.toISOString()).toBe("2026-05-31T23:59:59.999Z");
    expect(range?.days).toBe(31);
    expect(range?.months).toBe(1);
  });

  it("reads the current month in the requested timezone", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-06-01T03:00:00.000Z"));

    expect(currentMonthKey("America/Toronto")).toBe("2026-05");
  });

  it("uses the ending month as the budget period key at the 24th/25th boundary", () => {
    expect(budgetPeriodKeyForDate({ year: 2026, month: 7, day: 24 })).toBe("2026-07");
    expect(budgetPeriodKeyForDate({ year: 2026, month: 7, day: 25 })).toBe("2026-08");
    expect(budgetPeriodKeyForDate({ year: 2026, month: 12, day: 25 })).toBe("2027-01");
  });

  it("reads the current budget period in the configured timezone", () => {
    const instant = new Date("2026-07-24T22:30:00.000Z");

    expect(currentBudgetPeriodKey("Africa/Johannesburg", instant)).toBe("2026-08");
    expect(currentBudgetPeriodKey("America/Toronto", instant)).toBe("2026-07");
  });

  it("resolves budget period bounds and sequential days across calendar months", () => {
    expect(budgetPeriodParts("2026-08")).toEqual({
      start: { year: 2026, month: 7, day: 25 },
      end: { year: 2026, month: 8, day: 24 },
    });
    expect(budgetPeriodDayIndex("2026-07-25", "2026-08")).toBe(1);
    expect(budgetPeriodDayIndex("2026-08-01", "2026-08")).toBe(8);
    expect(budgetPeriodDayIndex("2026-08-24", "2026-08")).toBe(31);
    expect(budgetPeriodDayIndex("2026-08-25", "2026-08")).toBeNull();
    expect(budgetPeriodLabel("2026-08", "short", "en-US")).toBe("Jul 25 – Aug 24, 2026");
  });

  it("converts Johannesburg budget bounds to the expected UTC report window", () => {
    const period = budgetPeriodParts("2026-08");
    expect(period).not.toBeNull();
    expect(
      zonedCalendarDateBoundaryToDate(period!.start, "start", "Africa/Johannesburg").toISOString(),
    ).toBe("2026-07-24T22:00:00.000Z");
    expect(
      zonedCalendarDateBoundaryToDate(period!.end, "end", "Africa/Johannesburg").toISOString(),
    ).toBe("2026-08-24T21:59:59.999Z");
  });
});
