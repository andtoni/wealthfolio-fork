import { describe, expect, it } from "vitest";

import { isRiskWarning } from "./performance-page";

describe("isRiskWarning", () => {
  it("includes degraded external-flow provenance warnings", () => {
    expect(
      isRiskWarning(
        "External cash flows were inferred from net contribution deltas because gross daily flow data was unavailable.",
      ),
    ).toBe(true);
    expect(isRiskWarning("External cash flow provenance is incomplete for this period.")).toBe(
      true,
    );
  });

  it("does not attach unrelated return basis reasons to risk metrics", () => {
    expect(isRiskWarning("Book basis is unavailable for value return.")).toBe(false);
  });
});
