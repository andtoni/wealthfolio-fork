import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it, vi } from "vitest";
import { getAccounts, listProviderConnections } from "@/adapters";
import AccountIngestionPage from "./account-ingestion-page";

vi.mock("@/adapters", () => ({
  admitProviderAccount: vi.fn(),
  completePlaidConnection: vi.fn(),
  completePlaidUpdate: vi.fn(),
  getAccounts: vi.fn().mockResolvedValue([]),
  listDiscoveredProviderAccounts: vi.fn().mockResolvedValue([]),
  listProviderConnections: vi.fn().mockResolvedValue([]),
  startPlaidUpdate: vi.fn(),
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

  it("shows direct providers before manual account fallbacks", () => {
    renderPage();

    expect(screen.getByText("Direct Provider Connections")).toBeInTheDocument();
    expect(screen.getByText("Other ways to add accounts")).toBeInTheDocument();
  });

  it("renders a native Add account action linking to settings accounts", () => {
    renderPage();

    const link = screen.getByRole("link", { name: /add manual account/i });
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

  it("shows already connected Plaid accounts and an add-accounts action", async () => {
    vi.mocked(listProviderConnections).mockResolvedValueOnce([
      { provider: "plaid", status: "configured", message: "Provider is configured." },
    ]);
    vi.mocked(getAccounts).mockResolvedValueOnce([
      {
        id: "plaid-account",
        name: "Connected savings",
        accountType: "CASH",
        balance: 0,
        currency: "USD",
        isDefault: false,
        isActive: true,
        isArchived: false,
        trackingMode: "TRANSACTIONS",
        createdAt: new Date("2026-01-01T00:00:00Z"),
        updatedAt: new Date("2026-01-01T00:00:00Z"),
        provider: "plaid",
        providerAccountId: "provider-account",
      },
    ]);

    renderPage();

    expect(await screen.findByText("Connected accounts")).toBeInTheDocument();
    expect(screen.getByText("Connected savings")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Add accounts" })).toBeEnabled();
  });

  it("keeps Add accounts enabled when Plaid reports a completed connection", async () => {
    vi.mocked(listProviderConnections).mockResolvedValueOnce([
      { provider: "plaid", status: "completed", message: "Connection completed." },
    ]);
    vi.mocked(getAccounts).mockResolvedValueOnce([
      {
        id: "plaid-account",
        name: "Connected checking",
        accountType: "CASH",
        balance: 0,
        currency: "USD",
        isDefault: false,
        isActive: true,
        isArchived: false,
        trackingMode: "TRANSACTIONS",
        createdAt: new Date("2026-01-01T00:00:00Z"),
        updatedAt: new Date("2026-01-01T00:00:00Z"),
        provider: "plaid",
        providerAccountId: "provider-account",
      },
    ]);

    renderPage();

    expect(await screen.findByRole("button", { name: "Add accounts" })).toBeEnabled();
  });

  it("shows provider failures instead of masking them as connected", async () => {
    vi.mocked(listProviderConnections).mockResolvedValueOnce([
      { provider: "plaid", status: "auth_required", message: "Authentication required." },
    ]);
    vi.mocked(getAccounts).mockResolvedValueOnce([
      {
        id: "plaid-account",
        name: "Expired checking",
        accountType: "CASH",
        balance: 0,
        currency: "USD",
        isDefault: false,
        isActive: true,
        isArchived: false,
        trackingMode: "TRANSACTIONS",
        createdAt: new Date("2026-01-01T00:00:00Z"),
        updatedAt: new Date("2026-01-01T00:00:00Z"),
        provider: "plaid",
        providerAccountId: "provider-account",
      },
    ]);

    renderPage();

    expect(await screen.findByText("auth required")).toBeInTheDocument();
  });

  it("does not treat legacy Wealthfolio Connect SnapTrade accounts as direct connections", async () => {
    vi.mocked(listProviderConnections).mockResolvedValueOnce([
      { provider: "snaptrade", status: "configured", message: "Direct connection is available." },
    ]);
    vi.mocked(getAccounts).mockResolvedValueOnce([
      {
        id: "legacy-snaptrade-account",
        name: "Legacy brokerage",
        accountType: "SECURITIES",
        balance: 0,
        currency: "USD",
        isDefault: false,
        isActive: true,
        isArchived: false,
        trackingMode: "HOLDINGS",
        createdAt: new Date("2026-01-01T00:00:00Z"),
        updatedAt: new Date("2026-01-01T00:00:00Z"),
        provider: "SNAPTRADE",
        providerAccountId: "legacy-provider-account",
      },
    ]);

    renderPage();

    const connectButtons = await screen.findAllByRole("button", { name: "Connect" });
    expect(connectButtons.some((button) => !button.hasAttribute("disabled"))).toBe(true);
    expect(screen.queryByText("Legacy brokerage")).not.toBeInTheDocument();
  });
});
