import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { ComponentProps, ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { ProposeCategoriesOutput } from "../../types";
import { CategorizationProposalsContentImpl } from "./categorization-proposals-tool-ui";

const mocks = vi.hoisted(() => ({
  bulkAssign: vi.fn(),
  updateToolResult: vi.fn(),
}));

vi.mock("@/adapters", () => ({
  updateToolResult: mocks.updateToolResult,
}));
vi.mock("@/features/spending/hooks/use-cash-activities", () => ({
  useBulkAssignCategories: () => ({
    isPending: false,
    mutateAsync: mocks.bulkAssign,
  }),
}));
vi.mock("@/features/spending/components/quick-categorize-popover", () => ({
  QuickCategorizePopover: ({ trigger, scope }: { trigger: ReactNode; scope?: string }) => (
    <div data-scope={scope}>{trigger}</div>
  ),
}));
vi.mock("@/hooks/use-balance-privacy", () => ({
  useBalancePrivacy: () => ({ isBalanceHidden: false }),
}));
vi.mock("@/lib/settings-provider", () => ({
  useSettingsContext: () => ({ settings: { baseCurrency: "USD" } }),
}));
vi.mock("../../hooks/use-runtime-context", () => ({
  useRuntimeContext: () => ({ currentThreadId: "thread-1" }),
}));

type WidgetProps = ComponentProps<typeof CategorizationProposalsContentImpl>;

function staleDraft(bucket?: "spending" | "income"): ProposeCategoriesOutput {
  return {
    proposals: [
      {
        activityId: "activity-1",
        activityDate: "2026-08-01",
        amount: -42,
        currency: "USD",
        notes: "Legacy merchant",
        cashFlowBucket: bucket,
        taxonomyId: "spending_categories",
        categoryId: "category-food",
        categoryPath: "Food",
        confidence: 0.95,
        source: "rule",
        explanation: "Legacy draft",
      },
    ],
    unproposed: [],
    summary: {
      total: 1,
      proposed: 1,
      unproposed: 0,
      avgConfidence: 0.95,
    },
    taxonomies: [
      {
        taxonomyId: "spending_categories",
        taxonomyName: "Spending",
        categories: [
          {
            categoryId: "category-food",
            key: "food",
            name: "Food",
            path: "Food",
            color: "#22c55e",
          },
        ],
      },
    ],
    examples: [],
    draftStatus: "draft",
  };
}

function renderWidget(output: ProposeCategoriesOutput) {
  return render(
    <CategorizationProposalsContentImpl
      args={{}}
      argsText=""
      result={output}
      status={{ type: "complete" } as WidgetProps["status"]}
      toolName="propose_transaction_categories"
      toolCallId="tool-call-1"
      type="tool-call"
      addResult={vi.fn()}
      resume={vi.fn()}
    />,
  );
}

describe("CategorizationProposalsContentImpl", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.bulkAssign.mockResolvedValue([]);
    mocks.updateToolResult.mockResolvedValue(undefined);
  });

  it.each([
    ["bucketless", undefined],
    ["cross-bucket", "income"],
  ] as const)("expires %s saved drafts before they can be applied", (_label, bucket) => {
    renderWidget(staleDraft(bucket));

    expect(screen.getByText("Refresh these suggestions")).toBeInTheDocument();
    expect(screen.getByRole("checkbox", { name: "Select row" })).toBeDisabled();

    const apply = screen.getByRole("button", { name: "Apply" });
    expect(apply).toBeDisabled();
    fireEvent.click(apply);

    expect(mocks.bulkAssign).not.toHaveBeenCalled();
    expect(mocks.updateToolResult).not.toHaveBeenCalled();
  });

  it("applies valid proposals without expiring a neutral unproposed row", async () => {
    const output = staleDraft("spending");
    output.unproposed = [
      {
        activityId: "neutral-1",
        activityDate: "2026-08-01",
        amount: 100,
        currency: "USD",
        notes: "Internal transfer",
        cashFlowBucket: "neutral",
        reason: "Internal transfers cannot be categorized.",
      },
    ];
    output.summary = {
      total: 2,
      proposed: 1,
      unproposed: 1,
      avgConfidence: 0.95,
    };

    renderWidget(output);

    expect(screen.queryByText("Refresh these suggestions")).not.toBeInTheDocument();
    const apply = screen.getByRole("button", { name: "Apply 1 selected" });
    expect(apply).toBeEnabled();

    fireEvent.click(apply);

    await waitFor(() => {
      expect(mocks.bulkAssign).toHaveBeenCalledWith([
        {
          activityId: "activity-1",
          taxonomyId: "spending_categories",
          categoryId: "category-food",
        },
      ]);
    });
  });
});
