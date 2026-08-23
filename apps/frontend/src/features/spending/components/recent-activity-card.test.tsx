import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { Activity } from "@/lib/types";

import { RecentActivityCard } from "./recent-activity-card";

vi.mock("@tanstack/react-query", () => ({
  useQueries: vi.fn(() => []),
}));

function renderRecentActivityCard() {
  return render(
    <MemoryRouter>
      <RecentActivityCard activities={[]} categoriesMeta={new Map()} />
    </MemoryRouter>,
  );
}

describe("RecentActivityCard", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("shows the standard empty state without a setup link", () => {
    renderRecentActivityCard();

    expect(screen.getByText("No recent activity.")).toBeInTheDocument();
    expect(screen.queryByText("No spending accounts selected.")).not.toBeInTheDocument();
    expect(
      screen.queryByRole("link", { name: "Open spending settings →" }),
    ).not.toBeInTheDocument();
  });

  it("uses each activity's currency for the summary amount", () => {
    const activity: Activity = {
      id: "usd-activity",
      accountId: "account-1",
      activityType: "WITHDRAWAL",
      status: "POSTED",
      activityDate: "2026-08-23T00:00:00Z",
      amount: "57.04",
      currency: "USD",
      isUserModified: false,
      needsReview: false,
      createdAt: "2026-08-23T00:00:00Z",
      updatedAt: "2026-08-23T00:00:00Z",
    };

    render(
      <MemoryRouter>
        <RecentActivityCard activities={[activity]} categoriesMeta={new Map()} />
      </MemoryRouter>,
    );

    expect(screen.getByText("$57.04")).toBeInTheDocument();
    expect(screen.queryByText(/ZAR/)).not.toBeInTheDocument();
  });
});
