import { afterEach, describe, expect, it, vi } from "vitest";

import {
  addMonthsToMonthKey,
  budgetPeriodDayIndex,
  budgetPeriodKeyForDate,
  budgetPeriodLabel,
  budgetPeriodParts,
  currentBudgetPeriodKey,
  shiftBudgetPeriodKey,
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

  it("preserves the existing monthly cycle at the 24th/25th boundary", () => {
    expect(budgetPeriodKeyForDate({ year: 2026, month: 7, day: 24 })).toBe("2026-06-25");
    expect(budgetPeriodKeyForDate({ year: 2026, month: 7, day: 25 })).toBe("2026-07-25");
    expect(budgetPeriodKeyForDate({ year: 2026, month: 12, day: 25 })).toBe("2026-12-25");
  });

  it("reads the current budget period in the configured timezone", () => {
    const instant = new Date("2026-07-24T22:30:00.000Z");

    expect(currentBudgetPeriodKey("Africa/Johannesburg", undefined, instant)).toBe("2026-07-25");
    expect(currentBudgetPeriodKey("America/Toronto", undefined, instant)).toBe("2026-06-25");
  });

  it("resolves monthly period bounds and sequential days across calendar months", () => {
    expect(budgetPeriodParts("2026-07-25")).toEqual({
      start: { year: 2026, month: 7, day: 25 },
      end: { year: 2026, month: 8, day: 24 },
    });
    expect(budgetPeriodDayIndex("2026-07-25", "2026-07-25")).toBe(1);
    expect(budgetPeriodDayIndex("2026-08-01", "2026-07-25")).toBe(8);
    expect(budgetPeriodDayIndex("2026-08-24", "2026-07-25")).toBe(31);
    expect(budgetPeriodDayIndex("2026-08-25", "2026-07-25")).toBeNull();
    expect(budgetPeriodLabel("2026-07-25", "short", "en-US")).toBe("Jul 25 – Aug 24, 2026");
  });

  it("supports anchored weekly and biweekly cycles", () => {
    const weekly = { cadence: "weekly" as const, anchorDate: "2026-01-07" };
    const biweekly = { cadence: "biweekly" as const, anchorDate: "2026-01-01" };

    expect(budgetPeriodKeyForDate({ year: 2026, month: 1, day: 6 }, weekly)).toBe("2025-12-31");
    expect(budgetPeriodParts("2026-01-29", biweekly)).toEqual({
      start: { year: 2026, month: 1, day: 29 },
      end: { year: 2026, month: 2, day: 11 },
    });
    expect(shiftBudgetPeriodKey("2026-01-29", 1, biweekly)).toBe("2026-02-12");
  });

  it("clamps short months and preserves an end-of-month anchor across leap years", () => {
    const day30 = { cadence: "monthly" as const, anchorDate: "2024-01-30" };
    expect(budgetPeriodParts("2024-02-29", day30)).toEqual({
      start: { year: 2024, month: 2, day: 29 },
      end: { year: 2024, month: 3, day: 29 },
    });

    const monthEnd = { cadence: "monthly" as const, anchorDate: "2024-02-29" };
    expect(budgetPeriodParts("2025-02-28", monthEnd)).toEqual({
      start: { year: 2025, month: 2, day: 28 },
      end: { year: 2025, month: 3, day: 30 },
    });
    expect(budgetPeriodParts("2025-02-27", monthEnd)).toBeNull();
  });

  it("converts Johannesburg budget bounds to the expected UTC report window", () => {
    const period = budgetPeriodParts("2026-07-25");
    expect(period).not.toBeNull();
    expect(
      zonedCalendarDateBoundaryToDate(period!.start, "start", "Africa/Johannesburg").toISOString(),
    ).toBe("2026-07-24T22:00:00.000Z");
    expect(
      zonedCalendarDateBoundaryToDate(period!.end, "end", "Africa/Johannesburg").toISOString(),
    ).toBe("2026-08-24T21:59:59.999Z");
  });
});
