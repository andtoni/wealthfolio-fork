import type { BudgetGroupAssignment, BudgetTarget } from "../types/budget";

export function categoryPairKey(taxonomyId: string, categoryId: string): string {
  return `${taxonomyId}:${categoryId}`;
}

interface BudgetCategoryConfigurationState {
  groupAssignments?: readonly Pick<BudgetGroupAssignment, "taxonomyId" | "categoryId">[];
  targets?: readonly Pick<BudgetTarget, "periodKey" | "targetType" | "taxonomyId" | "categoryId">[];
}

/** Categories selected by budget groups or explicit category targets. */
export function configuredCategoryPairsFromBudget(
  state: BudgetCategoryConfigurationState | null | undefined,
): Set<string> {
  const pairs = new Set(
    (state?.groupAssignments ?? []).map((assignment) =>
      categoryPairKey(assignment.taxonomyId, assignment.categoryId),
    ),
  );
  for (const target of state?.targets ?? []) {
    if (
      target.periodKey === "default" &&
      target.targetType === "category" &&
      target.taxonomyId &&
      target.categoryId
    ) {
      pairs.add(categoryPairKey(target.taxonomyId, target.categoryId));
    }
  }
  return pairs;
}
