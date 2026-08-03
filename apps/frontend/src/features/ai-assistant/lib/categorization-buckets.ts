import type { QuickCategorizeScope } from "@/features/spending/components/quick-categorize-popover";
import type { CashFlowBucket } from "@/features/spending/types/cash-activity";

const TAXONOMY_BY_BUCKET: Record<Exclude<CashFlowBucket, "neutral">, string> = {
  spending: "spending_categories",
  income: "income_sources",
  saving: "savings_categories",
};

export function taxonomyForCashFlowBucket(bucket?: CashFlowBucket): string | null {
  return bucket && bucket !== "neutral" ? TAXONOMY_BY_BUCKET[bucket] : null;
}

export function categoryMatchesCashFlowBucket(
  bucket: CashFlowBucket | undefined,
  taxonomyId: string,
): boolean {
  return taxonomyForCashFlowBucket(bucket) === taxonomyId;
}

export function categoryScopeForCashFlowBucket(
  bucket?: CashFlowBucket,
): QuickCategorizeScope | null {
  if (bucket === "spending") return "expense";
  if (bucket === "income") return "income";
  if (bucket === "saving") return "saving";
  return null;
}
