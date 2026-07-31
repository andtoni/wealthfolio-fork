import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  useSpendingSettings,
  useSpendingSettingsMutation,
} from "@/features/spending/hooks/use-spending-settings";

import { BudgetCycleCard } from "./budget-cycle-card";

vi.mock("@wealthfolio/ui", () => ({
  Button: ({ children, ...props }: React.ButtonHTMLAttributes<HTMLButtonElement>) => (
    <button {...props}>{children}</button>
  ),
  Icons: { Calendar: () => <span /> },
  Input: (props: React.InputHTMLAttributes<HTMLInputElement>) => <input {...props} />,
  Select: ({
    value,
    onValueChange,
    children,
  }: {
    value: string;
    onValueChange: (value: string) => void;
    children: React.ReactNode;
  }) => (
    <select value={value} onChange={(event) => onValueChange(event.target.value)}>
      {children}
    </select>
  ),
  SelectTrigger: () => null,
  SelectValue: () => null,
  SelectContent: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  SelectItem: ({ value, children }: { value: string; children: React.ReactNode }) => (
    <option value={value}>{children}</option>
  ),
}));

vi.mock("@/features/spending/hooks/use-spending-settings", () => ({
  useSpendingSettings: vi.fn(),
  useSpendingSettingsMutation: vi.fn(),
}));

vi.mock("@/lib/settings-provider", () => ({
  useSettingsContext: () => ({ settings: { timezone: "UTC" } }),
}));

const mockUseSpendingSettings = vi.mocked(useSpendingSettings);
const mockUseSpendingSettingsMutation = vi.mocked(useSpendingSettingsMutation);
const mutate = vi.fn();

beforeEach(() => {
  vi.clearAllMocks();
  mockUseSpendingSettings.mockReturnValue({
    settings: {
      enabled: true,
      accountIds: [],
      budgetCycle: { cadence: "monthly", anchorDate: "2026-07-25" },
    },
    isEnabled: true,
    accountIds: [],
    budgetCycle: { cadence: "monthly", anchorDate: "2026-07-25" },
    isLoading: false,
    error: null,
  });
  mockUseSpendingSettingsMutation.mockReturnValue({
    mutate,
    isPending: false,
  } as unknown as ReturnType<typeof useSpendingSettingsMutation>);
});

describe("BudgetCycleCard", () => {
  it("saves a chosen cadence and anchor date", () => {
    render(<BudgetCycleCard />);

    fireEvent.change(screen.getByRole("combobox"), { target: { value: "biweekly" } });
    fireEvent.change(screen.getByLabelText("Starting on"), {
      target: { value: "2026-08-07" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save cycle" }));

    expect(mutate).toHaveBeenCalledWith({
      budgetCycle: { cadence: "biweekly", anchorDate: "2026-08-07" },
    });
  });
});
