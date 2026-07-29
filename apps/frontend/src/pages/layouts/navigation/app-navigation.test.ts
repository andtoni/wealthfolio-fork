import { describe, expect, it } from "vitest";
import { isPathActive } from "./app-navigation";

describe("isPathActive", () => {
  it("keeps Settings active for settings pages without dedicated navigation", () => {
    expect(isPathActive("/settings/general", "/settings")).toBe(true);
    expect(isPathActive("/settings/appearance", "/settings")).toBe(true);
  });

  it("defers account and spending pages to their dedicated navigation items", () => {
    expect(isPathActive("/settings/accounts", "/settings")).toBe(false);
    expect(isPathActive("/settings/accounts/groups", "/settings")).toBe(false);
    expect(isPathActive("/settings/spending", "/settings")).toBe(false);
    expect(isPathActive("/settings/spending/categories", "/settings")).toBe(false);
    expect(isPathActive("/settings/accounts", "/settings/accounts")).toBe(true);
    expect(isPathActive("/settings/spending", "/settings/spending")).toBe(true);
  });
});
