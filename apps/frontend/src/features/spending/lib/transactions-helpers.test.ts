import { describe, expect, it } from "vitest";

import type { CashActivity } from "../types/cash-activity";
import {
  getTransferLinkStatus,
  isLinkedTransferCashActivity,
  isTransferCashActivity,
  toEditableCashActivity,
  toRowVM,
} from "./transactions-helpers";

function cashActivity(overrides: Partial<CashActivity>): CashActivity {
  return {
    id: "activity-1",
    activityType: "WITHDRAWAL",
    activityDate: "2026-01-01T00:00:00.000Z",
    accountId: "account-1",
    amount: "100",
    currency: "USD",
    cashFlowBucket: "neutral",
    assignments: [],
    splits: [],
    isUserModified: false,
    needsReview: false,
    status: "POSTED",
    createdAt: "2026-01-01T00:00:00.000Z",
    updatedAt: "2026-01-01T00:00:00.000Z",
    ...overrides,
  } as CashActivity;
}

describe("spending transaction helpers", () => {
  it("treats activity type overrides as transfer rows", () => {
    const activity = cashActivity({
      activityTypeOverride: "TRANSFER_OUT",
      transferLinkStatus: "unlinked",
    });

    expect(isTransferCashActivity(activity)).toBe(true);
    expect(getTransferLinkStatus(activity)).toBe("unlinked");
  });

  it("uses the effective type in the standalone cash editor without mutating the row", () => {
    const activity = cashActivity({
      activityType: "DEPOSIT",
      activityTypeOverride: "TRANSFER_IN",
      transferLinkStatus: "unlinked",
    });

    const editable = toEditableCashActivity(activity);

    expect(editable.activityType).toBe("TRANSFER_IN");
    expect(editable.activityTypeOverride).toBeUndefined();
    expect(activity.activityType).toBe("DEPOSIT");
    expect(activity.activityTypeOverride).toBe("TRANSFER_IN");
  });

  it("distinguishes linked transfers from standalone or invalid groups", () => {
    expect(
      isLinkedTransferCashActivity(
        cashActivity({ activityTypeOverride: "TRANSFER_IN", transferLinkStatus: "linked" }),
      ),
    ).toBe(true);
    expect(
      isLinkedTransferCashActivity(
        cashActivity({ activityTypeOverride: "TRANSFER_IN", transferLinkStatus: "unlinked" }),
      ),
    ).toBe(false);
    expect(
      isLinkedTransferCashActivity(
        cashActivity({
          activityTypeOverride: "TRANSFER_IN",
          sourceGroupId: "orphan-group",
          transferLinkStatus: "invalid",
        }),
      ),
    ).toBe(false);
  });

  it("does not expose transfer link status for non-transfer effective types", () => {
    expect(getTransferLinkStatus(cashActivity({ sourceGroupId: "group-1" }))).toBeNull();
  });

  it("prefers split display state over a single category assignment", () => {
    const activity = cashActivity({
      cashFlowBucket: "spending",
      assignments: [
        {
          id: "assignment-1",
          activityId: "activity-1",
          taxonomyId: "spending_categories",
          categoryId: "groceries",
          weight: 10_000,
          source: "manual",
          createdAt: "2026-01-01T00:00:00.000Z",
          updatedAt: "2026-01-01T00:00:00.000Z",
        },
      ],
      splits: [
        {
          id: "split-1",
          activityId: "activity-1",
          taxonomyId: "spending_categories",
          categoryId: "groceries",
          amount: "80.00",
          note: null,
          sortOrder: 0,
          createdAt: "2026-01-01T00:00:00.000Z",
          updatedAt: "2026-01-01T00:00:00.000Z",
        },
        {
          id: "split-2",
          activityId: "activity-1",
          taxonomyId: "spending_categories",
          categoryId: "household",
          amount: "40.00",
          note: null,
          sortOrder: 1,
          createdAt: "2026-01-01T00:00:00.000Z",
          updatedAt: "2026-01-01T00:00:00.000Z",
        },
      ],
    });

    const row = toRowVM(
      activity,
      new Map([
        [
          "groceries",
          {
            id: "groceries",
            taxonomyId: "spending_categories",
            name: "Groceries",
            key: "groceries",
            color: "#4385be",
            sortOrder: 1,
            createdAt: "2026-01-01T00:00:00.000Z",
            updatedAt: "2026-01-01T00:00:00.000Z",
          },
        ],
      ]),
    );

    expect(row.category).toBeNull();
    expect(row.splitCount).toBe(2);
  });
});
