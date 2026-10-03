import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { TransactionsBulkBar } from "./transactions-bulk-bar";

vi.mock("./quick-event-popover", () => ({
  QuickEventPopover: ({ trigger }: { trigger: React.ReactNode }) => <>{trigger}</>,
}));

function renderBulkBar(onSetBudgetPeriod: (periodStart: string | null) => void) {
  return render(
    <TransactionsBulkBar
      selectedCount={2}
      categoryScope={null}
      budgetPeriods={[
        { key: "2026-08-25", label: "Aug 25 – Sep 24, 2026", isCurrent: false },
        { key: "2026-09-25", label: "Sep 25 – Oct 24, 2026", isCurrent: true },
        { key: "2026-10-25", label: "Oct 25 – Nov 24, 2026", isCurrent: false },
      ]}
      onCategorize={vi.fn()}
      onTagEvent={vi.fn()}
      onSetBudgetPeriod={onSetBudgetPeriod}
      onDelete={vi.fn()}
      onClearSelection={vi.fn()}
    />,
  );
}

describe("TransactionsBulkBar budget period", () => {
  it("assigns the selection to the chosen budget period", async () => {
    const onSetBudgetPeriod = vi.fn();
    renderBulkBar(onSetBudgetPeriod);

    await userEvent.click(screen.getByRole("button", { name: /budget period/i }));
    await userEvent.click(
      screen.getByRole("menuitem", { name: "Sep 25 – Oct 24, 2026 (current)" }),
    );

    expect(onSetBudgetPeriod).toHaveBeenCalledWith("2026-09-25");
  });

  it("reverts the selection to its own transaction dates", async () => {
    const onSetBudgetPeriod = vi.fn();
    renderBulkBar(onSetBudgetPeriod);

    await userEvent.click(screen.getByRole("button", { name: /budget period/i }));
    await userEvent.click(screen.getByRole("menuitem", { name: "Use transaction date" }));

    expect(onSetBudgetPeriod).toHaveBeenCalledWith(null);
  });
});
