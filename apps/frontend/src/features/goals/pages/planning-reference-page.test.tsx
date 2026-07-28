import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it } from "vitest";
import PlanningReferencePage from "./planning-reference-page";

describe("PlanningReferencePage", () => {
  it("renders static educational sections and a safe retirement-goal link", () => {
    render(
      <MemoryRouter>
        <PlanningReferencePage />
      </MemoryRouter>,
    );

    expect(screen.getByRole("heading", { name: "Planning Reference" })).toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent("Educational reference only");
    expect(screen.getByRole("region", { name: "Borrowing reference" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Retirement projections" })).toBeInTheDocument();
    expect(
      screen.getByRole("region", { name: "United States investment order" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Open the retirement guide" })).toHaveAttribute(
      "href",
      "/goals/retirement-guide",
    );
  });

  it("uses accessible accordion controls for the educational details", () => {
    render(
      <MemoryRouter>
        <PlanningReferencePage />
      </MemoryRouter>,
    );

    expect(screen.getByRole("button", { name: "Amortization and total cost" })).toHaveAttribute(
      "aria-expanded",
      "false",
    );
    expect(screen.getByRole("button", { name: "Rate stress" })).toBeInTheDocument();
  });
});
