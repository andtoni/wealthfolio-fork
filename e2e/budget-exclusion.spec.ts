import { expect, test } from "@playwright/test";

import { completeOnboardingIfNeeded, gotoAppPath, BASE_URL } from "./helpers";

test.describe("Budget exclusions", () => {
  test("excludes a group from budget calculations without changing spending reports", async ({
    page,
  }) => {
    test.setTimeout(60000);
    await completeOnboardingIfNeeded(page);

    const api = `${BASE_URL}/api/v1`;
    const getBudget = async () => {
      const response = await page.request.get(`${api}/spending/budget`);
      expect(response.ok()).toBeTruthy();
      return response.json();
    };
    const updateGroup = async (groupId: string, excludeFromBudget: boolean) => {
      const response = await page.request.put(`${api}/spending/budget/groups/${groupId}`, {
        data: { excludeFromBudget },
      });
      expect(response.ok()).toBeTruthy();
      return response.json();
    };

    const initialBudget = await getBudget();
    const other = initialBudget.state.groups.find(
      (group: { key: string }) => group.key === "other",
    );
    expect(other).toBeTruthy();

    const reportRequest = {
      startDate: `${initialBudget.computed.periodStart}T00:00:00Z`,
      endDate: `${initialBudget.computed.periodEnd}T23:59:59Z`,
    };
    const getReport = async () => {
      const response = await page.request.post(`${api}/spending/report`, { data: reportRequest });
      expect(response.ok()).toBeTruthy();
      return response.json();
    };

    const originalExcluded = other.excludeFromBudget;
    try {
      // Establish an included baseline, then make the user-facing toggle perform
      // the exclusion. This also proves the setting is reversible.
      await updateGroup(other.id, false);
      const includedBudget = await getBudget();
      const includedReport = await getReport();
      const includedOther = includedBudget.computed.groupRows.find(
        (row: { group: { id: string } }) => row.group.id === other.id,
      );
      expect(includedOther).toBeTruthy();

      await gotoAppPath(page, `/spending/budget?period=${includedBudget.computed.periodKey}`);
      const editButton = page.getByRole("button", { name: "Edit Other", exact: true });
      await expect(editButton).toBeVisible();
      await editButton.click();

      const dialog = page.getByRole("dialog");
      const excludeSwitch = dialog.getByRole("switch", {
        name: "Exclude group from budget calculations",
      });
      await expect(excludeSwitch).toHaveAttribute("aria-checked", "false");
      await excludeSwitch.click();
      await expect(excludeSwitch).toHaveAttribute("aria-checked", "true");
      await dialog.getByRole("button", { name: "Save", exact: true }).click();
      await expect(dialog).not.toBeVisible();

      await expect
        .poll(async () => {
          const budget = await getBudget();
          const group = budget.state.groups.find((row: { id: string }) => row.id === other.id);
          return group?.excludeFromBudget;
        })
        .toBe(true);

      const excludedBudget = await getBudget();
      const excludedReport = await getReport();
      const excludedOther = excludedBudget.computed.groupRows.find(
        (row: { group: { id: string } }) => row.group.id === other.id,
      );
      expect(excludedOther).toBeTruthy();
      expect(excludedOther.actual).toBe(0);
      expect(
        excludedOther.categories.every((category: { actual: number }) => category.actual === 0),
      ).toBe(true);
      expect(excludedBudget.computed.totals.spendingActual).toBe(
        includedBudget.computed.totals.spendingActual - includedOther.actual,
      );

      // The report powers "Where it went" and must remain byte-for-byte the
      // same; only the budget snapshot is filtered.
      expect(excludedReport).toEqual(includedReport);
    } finally {
      await updateGroup(other.id, originalExcluded);
    }
  });

  test("does not apply touch-pan-y to Net Worth after returning to page scroll", async ({
    page,
  }) => {
    test.setTimeout(30000);
    await completeOnboardingIfNeeded(page);
    await page.setViewportSize({ width: 390, height: 844 });
    await gotoAppPath(page, "/dashboard?tab=net-worth");

    await expect
      .poll(async () =>
        page.locator(".native-scroll").evaluateAll((elements) => {
          const visible = elements.filter((element) => {
            const rect = element.getBoundingClientRect();
            return rect.width > 0 && rect.height > 0;
          });
          return (
            visible.length > 0 &&
            visible.every((element) => !element.className.includes("touch-pan-y"))
          );
        }),
      )
      .toBe(true);
  });
});
