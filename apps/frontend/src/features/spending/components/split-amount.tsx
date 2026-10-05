import { useBalancePrivacy } from "@wealthfolio/ui";

interface SplitAmountProps {
  value: number;
  currency: string;
  /** "-" for money out, "+" for money in; left out, a negative value shows its minus. */
  sign?: string;
}

/** An amount with the cents small and raised, so the whole number reads first. */
export function SplitAmount({ value, currency, sign = value < 0 ? "-" : "" }: SplitAmountProps) {
  const { isBalanceHidden } = useBalancePrivacy();
  if (isBalanceHidden) return <span>••••</span>;
  const parts = new Intl.NumberFormat("en-US", {
    style: "currency",
    currency,
    currencyDisplay: "narrowSymbol",
  }).formatToParts(Math.abs(value));
  const whole = parts
    .filter((part) => part.type !== "decimal" && part.type !== "fraction")
    .map((part) => part.value)
    .join("");
  const cents = parts.find((part) => part.type === "fraction")?.value;
  return (
    <span className="whitespace-nowrap tabular-nums">
      {sign === "-" ? "−" : sign}
      {whole}
      {cents && <span className="align-super text-[0.62em]">.{cents}</span>}
    </span>
  );
}
