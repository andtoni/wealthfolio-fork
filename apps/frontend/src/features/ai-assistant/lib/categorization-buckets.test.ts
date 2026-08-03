import { describe, expect, it } from "vitest";

import {
  categoryMatchesCashFlowBucket,
  categoryScopeForCashFlowBucket,
  taxonomyForCashFlowBucket,
} from "./categorization-buckets";

describe("categorization bucket invariants", () => {
  it.each([
    ["spending", "spending_categories", "expense"],
    ["income", "income_sources", "income"],
    ["saving", "savings_categories", "saving"],
  ] as const)("maps %s to its only valid taxonomy and picker scope", (bucket, taxonomy, scope) => {
    expect(taxonomyForCashFlowBucket(bucket)).toBe(taxonomy);
    expect(categoryScopeForCashFlowBucket(bucket)).toBe(scope);
    expect(categoryMatchesCashFlowBucket(bucket, taxonomy)).toBe(true);
  });

  it("rejects cross-bucket, neutral, and legacy bucketless proposals", () => {
    expect(categoryMatchesCashFlowBucket("income", "spending_categories")).toBe(false);
    expect(categoryMatchesCashFlowBucket("neutral", "spending_categories")).toBe(false);
    expect(categoryMatchesCashFlowBucket(undefined, "spending_categories")).toBe(false);
    expect(categoryScopeForCashFlowBucket(undefined)).toBeNull();
  });
});
