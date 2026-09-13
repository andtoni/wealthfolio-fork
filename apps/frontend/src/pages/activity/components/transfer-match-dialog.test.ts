import { describe, expect, it } from "vitest";

import type { Activity, ActivityDetails } from "@/lib/types";
import {
  getEffectiveTransferActivityType,
  isStrictTransferMatch,
  type NormalizedTransferActivity,
} from "./transfer-match-dialog";

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

function normalizedTransfer(
  overrides: Partial<NormalizedTransferActivity> = {},
): NormalizedTransferActivity {
  return {
    id: "transfer-in",
    accountId: "account-1",
    accountName: "Account",
    accountType: "checking",
    accountCurrency: "USD",
    activityType: "TRANSFER_IN",
    date: "2026-08-07T00:00:00.000Z",
    amount: "3150",
    quantity: null,
    unitPrice: null,
    currency: "USD",
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

  it("keeps cross-currency cash legs selectable in strict manual search", () => {
    expect(
      isStrictTransferMatch(
        normalizedTransfer({ accountId: "account-1", currency: "USD", amount: "100" }),
        normalizedTransfer({
          id: "transfer-out",
          accountId: "account-2",
          activityType: "TRANSFER_OUT",
          currency: "EUR",
          amount: "92",
        }),
      ),
    ).toBe(true);
  });

  it("rejects cross-currency cash legs with missing or zero amounts", () => {
    expect(
      isStrictTransferMatch(
        normalizedTransfer({ amount: null, quantity: "10", unitPrice: "100" }),
        normalizedTransfer({
          id: "transfer-out",
          accountId: "account-2",
          currency: "EUR",
          amount: null,
          quantity: "9",
          unitPrice: "100",
        }),
      ),
    ).toBe(false);
    expect(
      isStrictTransferMatch(
        normalizedTransfer({ amount: "0" }),
        normalizedTransfer({
          id: "transfer-out",
          accountId: "account-2",
          currency: "EUR",
          amount: "92",
        }),
      ),
    ).toBe(false);
  });

  it("still rejects same-currency cash amount mismatches in strict mode", () => {
    expect(
      isStrictTransferMatch(
        normalizedTransfer({ amount: "100" }),
        normalizedTransfer({
          id: "transfer-out",
          accountId: "account-2",
          activityType: "TRANSFER_OUT",
          amount: "92",
        }),
      ),
    ).toBe(false);
  });
});
