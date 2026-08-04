import { afterEach, describe, expect, it, vi } from "vitest";

import { periodToReportsRange } from "./reports-period";

describe("anchored reports periods", () => {
  afterEach(() => vi.useRealTimers());

  it("uses the configured current and prior budget cycles for MTD and last month", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-08-04T12:00:00.000Z"));
    const cycle = { cadence: "monthly" as const, anchorDate: "2026-07-25" };

    const current = periodToReportsRange("MTD", "UTC", cycle);
    const prior = periodToReportsRange("LAST_MONTH", "UTC", cycle);

    expect(current.start.toISOString()).toBe("2026-07-25T00:00:00.000Z");
    expect(current.end.toISOString()).toBe("2026-08-24T23:59:59.999Z");
    expect(prior.start.toISOString()).toBe("2026-06-25T00:00:00.000Z");
    expect(prior.end.toISOString()).toBe("2026-07-24T23:59:59.999Z");
  });

  it("supports anchored weekly periods without changing longer calendar ranges", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-08-04T12:00:00.000Z"));
    const cycle = { cadence: "weekly" as const, anchorDate: "2026-07-29" };

    const current = periodToReportsRange("MTD", "UTC", cycle);
    const longer = periodToReportsRange("3M", "UTC", cycle);

    expect(current.start.toISOString()).toBe("2026-07-29T00:00:00.000Z");
    expect(current.end.toISOString()).toBe("2026-08-04T23:59:59.999Z");
    expect(longer.start.toISOString()).toBe("2026-06-01T00:00:00.000Z");
  });
});
