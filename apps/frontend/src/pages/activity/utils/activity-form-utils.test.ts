import { describe, expect, it } from "vitest";

import { ActivityType } from "@/lib/constants";
import { canChangeActivityType, mapActivityTypeToPicker } from "./activity-form-utils";

describe("activity form type editing", () => {
  it("allows standalone transfer overrides to change type", () => {
    expect(
      canChangeActivityType({
        id: "transfer-in",
        activityType: ActivityType.TRANSFER_IN,
      }),
    ).toBe(true);
    expect(
      canChangeActivityType({
        id: "provider-deposit",
        activityType: ActivityType.DEPOSIT,
        activityTypeOverride: ActivityType.TRANSFER_IN,
      }),
    ).toBe(true);
  });

  it("keeps a paired transfer in the paired editor", () => {
    expect(
      canChangeActivityType({
        id: "transfer-in",
        activityType: ActivityType.TRANSFER_IN,
        transferOutId: "transfer-out",
        transferInId: "transfer-in",
      }),
    ).toBe(false);
  });

  it("does not unlock ordinary activity edits", () => {
    expect(canChangeActivityType({ id: "deposit", activityType: ActivityType.DEPOSIT })).toBe(
      false,
    );
  });

  it("does not unlock an explicitly locked form", () => {
    expect(
      canChangeActivityType({ id: "transfer-in", activityType: ActivityType.TRANSFER_IN }, true),
    ).toBe(false);
  });

  it("maps transfer directions to the shared transfer picker", () => {
    expect(mapActivityTypeToPicker(ActivityType.TRANSFER_IN)).toBe("TRANSFER");
    expect(mapActivityTypeToPicker(ActivityType.TRANSFER_OUT)).toBe("TRANSFER");
  });
});
