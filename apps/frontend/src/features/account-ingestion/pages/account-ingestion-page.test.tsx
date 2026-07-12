import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it, vi } from "vitest";
import AccountIngestionPage from "./account-ingestion-page";

vi.mock("@/adapters", () => ({
  listProviderConnections: vi.fn().mockResolvedValue([]),
  startProviderConnection: vi.fn().mockRejectedValue(new Error("Not implemented")),
}));

vi.mock("@/context/auth-context", () => ({
  useAuth: () => ({ isAuthenticated: true, requiresAuth: true }),
}));

function renderPage() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter>
        <AccountIngestionPage />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

describe("AccountIngestionPage", () => {
  it("renders the page with connect heading from translations", () => {
    renderPage();

    // Heading appears in both PageHeader (h1) and content area (h2)
    const headings = screen.getAllByText("Connect");
    expect(headings.length).toBeGreaterThanOrEqual(1);
  });

  it("shows the Wealthfolio logo fallback", () => {
    renderPage();

    // Avatar renders WF fallback when image hasnt loaded in test env
    expect(screen.getByText("WF")).toBeInTheDocument();
  });

  it("shows zero connected accounts empty state", () => {
    renderPage();

    expect(screen.getByText("No Connected Accounts")).toBeInTheDocument();
  });

  it("renders a native Add account action linking to settings accounts", () => {
    renderPage();

    const link = screen.getByRole("link", { name: /add account/i });
    expect(link).toBeInTheDocument();
    expect(link).toHaveAttribute("href", "/settings/accounts");
  });

  it("renders a secondary CSV import action linking to the import route", () => {
    renderPage();

    const link = screen.getByRole("link", { name: /import csv/i });
    expect(link).toBeInTheDocument();
    expect(link).toHaveAttribute("href", "/import");
  });

  it("shows secure local data status information", () => {
    renderPage();

    expect(screen.getByText("Your Data Stays Local")).toBeInTheDocument();
  });
});
