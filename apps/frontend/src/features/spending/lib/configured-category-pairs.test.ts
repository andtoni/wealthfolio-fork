import { describe, expect, it } from "vitest";

import { categoryPairKey, configuredCategoryPairsFromBudget } from "./configured-category-pairs";

describe("configuredCategoryPairsFromBudget", () => {
  it("keeps taxonomy and category together across groups and targets", () => {
    const pairs = configuredCategoryPairsFromBudget({
      groupAssignments: [{ taxonomyId: "spending_categories", categoryId: "shared-id" }],
      targets: [
        {
          periodKey: "default",
          targetType: "category",
          taxonomyId: "income_sources",
          categoryId: "shared-id",
        },
        {
          periodKey: "default",
          targetType: "group_buffer",
          taxonomyId: null,
          categoryId: null,
        },
      ],
    });

    expect(pairs).toEqual(
      new Set([
        categoryPairKey("spending_categories", "shared-id"),
        categoryPairKey("income_sources", "shared-id"),
      ]),
    );
  });

  it("ignores period-only category targets", () => {
    const pairs = configuredCategoryPairsFromBudget({
      groupAssignments: [],
      targets: [
        {
          periodKey: "2026-08",
          targetType: "category",
          taxonomyId: "spending_categories",
          categoryId: "monthly-only",
        },
      ],
    });

    expect(pairs).toEqual(new Set());
  });

  it("returns no choices when budget state has no configured categories", () => {
    expect(configuredCategoryPairsFromBudget({ groupAssignments: [], targets: [] })).toEqual(
      new Set(),
    );
  });
});
