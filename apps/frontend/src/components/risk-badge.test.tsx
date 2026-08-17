import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { RiskBadge } from "./risk-badge";

describe("RiskBadge", () => {
  it("normalizes provider-style risk values", () => {
    render(<RiskBadge level="  medium  " />);

    expect(screen.getByText("Medium")).toBeInTheDocument();
  });

  it("renders unknown for missing or unsupported values", () => {
    const { rerender } = render(<RiskBadge level={null} />);
    expect(screen.getByText("Unknown")).toBeInTheDocument();

    rerender(<RiskBadge level="speculative" />);
    expect(screen.getByText("Unknown")).toBeInTheDocument();
  });
});
