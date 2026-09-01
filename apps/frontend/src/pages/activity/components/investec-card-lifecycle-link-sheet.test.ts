import { describe, expect, it } from "vitest";

import type { ActivityDetails } from "@/lib/types";
import { orderLifecycleCandidates } from "./investec-card-lifecycle-link-sheet";

function activity(id: string, comment: string, date: string, amount: string): ActivityDetails {
  return { id, comment, date, amount, currency: "ZAR" } as ActivityDetails;
}

describe("orderLifecycleCandidates", () => {
  it("places the merchant match first when it was returned sixth", () => {
    const source = activity("pending", "SAMPLEMART ONLINE", "2026-08-26T15:44:19Z", "1433.86");
    const matching = activity(
      "posted-match",
      "SAMPLEMART ONLINE PURCHASE",
      "2026-08-26T00:00:00Z",
      "1430.40",
    );
    const candidates = [
      activity("posted-1", "MERCHANT ONE", "2026-08-26T00:00:00Z", "1433.86"),
      activity("posted-2", "MERCHANT TWO", "2026-08-26T01:00:00Z", "1433.86"),
      activity("posted-3", "MERCHANT THREE", "2026-08-26T02:00:00Z", "1433.86"),
      activity("posted-4", "MERCHANT FOUR", "2026-08-26T03:00:00Z", "1433.86"),
      activity("posted-5", "MERCHANT FIVE", "2026-08-26T04:00:00Z", "1433.86"),
      matching,
    ];

    const ordered = orderLifecycleCandidates(source, candidates);

    expect(ordered[0]).toBe(matching);
    expect(candidates[0].id).toBe("posted-1");
  });

  it("uses date distance, amount distance, and id as deterministic tie breakers", () => {
    const source = activity("pending", "WOOLWORTHS", "2026-08-26T12:00:00Z", "100");
    const candidates = [
      activity("c", "OTHER", "2026-08-27T12:00:00Z", "101"),
      activity("b", "OTHER", "2026-08-26T13:00:00Z", "102"),
      activity("a", "OTHER", "2026-08-26T13:00:00Z", "102"),
      activity("d", "OTHER", "2026-08-26T13:00:00Z", "103"),
    ];

    expect(orderLifecycleCandidates(source, candidates).map(({ id }) => id)).toEqual([
      "a",
      "b",
      "d",
      "c",
    ]);
  });
});
