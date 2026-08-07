import { describe, expect, it } from "vitest";

import type { Activity, ActivityDetails } from "@/lib/types";
import { getEffectiveTransferActivityType } from "./transfer-match-dialog";

function providerActivity(overrides: Partial<Activity> = {}): Activity {
  return {
    id: "provider-in",
    accountId: "account-1",
    activityType: "DEPOSIT",
    activityTypeOverride: "TRANSFER_IN",
    status: "POSTED",
    activityDate: "2026-08-07T00:00:00.000Z",
    currency: "ZAR",
    amount: "3150",
    isUserModified: true,
    needsReview: false,
    createdAt: "2026-08-07T00:00:00.000Z",
    updatedAt: "2026-08-07T00:00:00.000Z",
    ...overrides,
  };
}

describe("transfer match activity type handling", () => {
  it("uses a raw provider activity override for link validation", () => {
    expect(getEffectiveTransferActivityType(providerActivity())).toBe("TRANSFER_IN");
  });

  it("keeps canonical activity types when no override exists", () => {
    expect(getEffectiveTransferActivityType(providerActivity({ activityTypeOverride: null }))).toBe(
      "DEPOSIT",
    );
  });

  it("accepts the effective type already returned by ActivityDetails", () => {
    const details = {
      id: "provider-in",
      activityType: "TRANSFER_IN",
    } as ActivityDetails;

    expect(getEffectiveTransferActivityType(details)).toBe("TRANSFER_IN");
  });
});
