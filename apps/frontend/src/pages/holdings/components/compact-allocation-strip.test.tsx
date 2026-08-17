import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { TaxonomyAllocation } from "@/lib/types";

import { CompactAllocationStrip } from "./compact-allocation-strip";

const riskAllocation: TaxonomyAllocation = {
  taxonomyId: "risk_category",
  taxonomyName: "Risk Category",
  color: "#000000",
  categories: [
    {
      categoryId: "HIGH",
      categoryName: "Provider high label",
      color: "#d14d41",
      value: 75,
      percentage: 75,
    },
    {
      categoryId: "LOW",
      categoryName: "Provider low label",
      color: "#879a39",
      value: 25,
      percentage: 25,
    },
  ],
};

describe("CompactAllocationStrip", () => {
  it("uses canonical risk ids and renders the fixed risk order", () => {
    render(
      <CompactAllocationStrip
        title="Risk"
        allocation={riskAllocation}
        variant="risk-composition"
      />,
    );

    const bodyText = document.body.textContent ?? "";
    expect(bodyText.indexOf("Low")).toBeLessThan(bodyText.indexOf("Med"));
    expect(bodyText.indexOf("Med")).toBeLessThan(bodyText.indexOf("High"));
    expect(screen.getByText("25%")).toBeInTheDocument();
    expect(screen.getByText("75%")).toBeInTheDocument();
  });

  it("includes unsupported positive risk values in a non-clickable unknown slot", () => {
    const onSegmentClick = vi.fn();
    render(
      <CompactAllocationStrip
        title="Risk"
        allocation={{
          ...riskAllocation,
          categories: [
            ...riskAllocation.categories,
            {
              categoryId: "VERY_HIGH",
              categoryName: "Very high",
              color: "#000000",
              value: 25,
              percentage: 20,
            },
          ],
        }}
        variant="risk-composition"
        onSegmentClick={onSegmentClick}
      />,
    );

    const unknownSlot = screen.getByText("-").parentElement;
    expect(unknownSlot).toHaveTextContent("20%");
    expect(unknownSlot).not.toHaveAttribute("role", "button");
    fireEvent.click(unknownSlot!);
    expect(onSegmentClick).not.toHaveBeenCalled();
  });
});
