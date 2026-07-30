import { describe, expect, it } from "vitest";
import { buildStaticNavigation, isPathActive } from "./app-navigation";

describe("isPathActive", () => {
  it("keeps Settings active for settings pages without dedicated navigation", () => {
    expect(isPathActive("/settings/general", "/settings")).toBe(true);
    expect(isPathActive("/settings/appearance", "/settings")).toBe(true);
  });

  it("keeps account management dedicated while spending setup remains under Settings", () => {
    expect(isPathActive("/settings/accounts", "/settings")).toBe(false);
    expect(isPathActive("/settings/accounts/groups", "/settings")).toBe(false);
    expect(isPathActive("/settings/spending", "/settings")).toBe(true);
    expect(isPathActive("/settings/spending/categories", "/settings")).toBe(true);
    expect(isPathActive("/settings/accounts", "/settings/accounts")).toBe(true);
    expect(isPathActive("/spending/budget", "/spending/budget")).toBe(true);
  });

  it("keeps native portfolio Insights second on mobile and sends Budget to the budget screen", () => {
    const navigation = buildStaticNavigation(((key: string) => key) as never);
    expect(navigation.primary.slice(0, 3).map((item) => item.href)).toEqual([
      "/dashboard",
      "/insights",
      "/spending/budget",
    ]);
  });
});
