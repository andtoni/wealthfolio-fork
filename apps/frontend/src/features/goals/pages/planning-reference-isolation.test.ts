import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const pagePath = resolve(import.meta.dirname, "planning-reference-page.tsx");
const dashboardPath = resolve(import.meta.dirname, "goals-dashboard-page.tsx");
const routesPath = resolve(import.meta.dirname, "../../../routes.tsx");
const forbiddenPatterns = [
  /@\/adapters/,
  /@\/commands/,
  /finance-sync/i,
  /localStorage/,
  /sessionStorage/,
  /useQuery/,
  /use[A-Z]\w*(Accounts|Goals|Holdings|Settings|Portfolio|Provider)/,
  /tauri/i,
  /tarn/i,
];

describe("Planning Reference isolation boundary", () => {
  it("contains no data, persistence, provider, or retired-product integration surface", () => {
    const source = readFileSync(pagePath, "utf8");

    for (const pattern of forbiddenPatterns) {
      expect(source, `${pagePath} should not match ${pattern}`).not.toMatch(pattern);
    }
  });

  it("is discoverable from Goals and routes both reference surfaces", () => {
    expect(readFileSync(dashboardPath, "utf8")).toContain('to="/goals/planning-reference"');

    const routes = readFileSync(routesPath, "utf8");
    expect(routes).toContain('path="goals/planning-reference"');
    expect(routes).toContain('path="goals/retirement-guide"');
  });
});
