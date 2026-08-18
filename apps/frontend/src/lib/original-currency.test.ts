import { describe, expect, it } from "vitest";
import { getOriginalCurrencyDisplay, isProvisionalCardSpending } from "./original-currency";

const original = (amount: string, currency: string) => ({
  providerOwned: { original: { amount, currency } },
});

describe("getOriginalCurrencyDisplay", () => {
  it("returns explicit USD original evidence for a posted ZAR card settlement", () => {
    const result = getOriginalCurrencyDisplay({
      status: "POSTED",
      sourceType: "CARDPURCHASES",
      subtype: "CARD_PURCHASE",
      currency: "ZAR",
      metadata: original("5.74", "USD"),
    });

    expect(result).toMatchObject({ amount: "5.74", currency: "USD" });
    expect(result?.label).toContain("5.74");
    expect(result?.label).toContain("USD");
  });

  it("supports EUR original evidence and a posted card subtype", () => {
    expect(
      getOriginalCurrencyDisplay({
        status: "POSTED",
        sourceType: "CARD",
        subtype: "CARD_PURCHASE",
        currency: "ZAR",
        metadata: original("12.345", "eur"),
      }),
    ).toMatchObject({ amount: "12.345", currency: "EUR" });
  });

  it("accepts numeric original amount", () => {
    expect(
      getOriginalCurrencyDisplay({
        status: "POSTED",
        sourceType: "CARD",
        currency: "ZAR",
        metadata: { providerOwned: { original: { amount: 5.74, currency: "USD" } } },
      }),
    ).toMatchObject({ amount: "5.74", currency: "USD" });
  });

  it("supports CARD_PURCHASE sourceType detection", () => {
    expect(
      getOriginalCurrencyDisplay({
        status: "POSTED",
        sourceType: "CARD_PURCHASE",
        currency: "ZAR",
        metadata: original("10", "EUR"),
      }),
    ).toMatchObject({ amount: "10", currency: "EUR" });
  });

  it("hides same-currency evidence", () => {
    expect(
      getOriginalCurrencyDisplay({
        status: "POSTED",
        sourceType: "CARDPURCHASES",
        currency: "ZAR",
        metadata: original("95.41", "ZAR"),
      }),
    ).toBeNull();
  });

  it("hides pending authorizations even when original evidence exists", () => {
    expect(
      getOriginalCurrencyDisplay({
        status: "PENDING",
        sourceType: "CARD",
        subtype: "CARD_PURCHASE",
        currency: "USD",
        metadata: original("1349.99", "USD"),
      }),
    ).toBeNull();
  });

  it("hides pocket conversions", () => {
    expect(
      getOriginalCurrencyDisplay({
        status: "POSTED",
        sourceType: "MCPBAPOCKETTRANSFERS",
        subtype: "POCKET_TRANSFER",
        currency: "ZAR",
        metadata: original("1400", "USD"),
      }),
    ).toBeNull();
  });

  it("recognizes only explicitly marked pending card authorizations as provisional", () => {
    const metadata = {
      providerOwned: { investec: { provisionalSpending: true } },
    };
    expect(
      isProvisionalCardSpending({
        status: "PENDING",
        sourceType: "CARD",
        subtype: "CARD_PURCHASE",
        activityType: "WITHDRAWAL",
        metadata,
      }),
    ).toBe(true);
    expect(
      isProvisionalCardSpending({
        status: "POSTED",
        sourceType: "CARD",
        subtype: "CARD_PURCHASE",
        activityType: "WITHDRAWAL",
        metadata,
      }),
    ).toBe(false);
    expect(
      isProvisionalCardSpending({
        status: "PENDING",
        sourceType: "MCPBAPOCKETTRANSFERS",
        subtype: "POCKET_TRANSFER",
        activityType: "WITHDRAWAL",
        metadata,
      }),
    ).toBe(false);
    // No override
    expect(
      isProvisionalCardSpending({
        status: "PENDING",
        sourceType: "CARD",
        activityType: "WITHDRAWAL",
        activityTypeOverride: "WITHDRAWAL",
        metadata,
      }),
    ).toBe(false);
    // Native type must be WITHDRAWAL, not credit
    expect(
      isProvisionalCardSpending({
        status: "PENDING",
        sourceType: "CARD",
        activityType: "CREDIT",
        metadata,
      }),
    ).toBe(false);
    // Card detection variations
    expect(
      isProvisionalCardSpending({
        status: "PENDING",
        sourceType: "CARDPURCHASES",
        activityType: "WITHDRAWAL",
        metadata,
      }),
    ).toBe(true);
    expect(
      isProvisionalCardSpending({
        status: "PENDING",
        sourceType: "CARD_PURCHASE",
        activityType: "WITHDRAWAL",
        metadata,
      }),
    ).toBe(true);
  });

  it("hides malformed metadata and invalid evidence", () => {
    const base = { status: "POSTED", sourceType: "CARDPURCHASES", currency: "ZAR" };
    expect(getOriginalCurrencyDisplay({ ...base, metadata: "not-json" })).toBeNull();
    expect(getOriginalCurrencyDisplay({ ...base, metadata: original("NaN", "USD") })).toBeNull();
    expect(getOriginalCurrencyDisplay({ ...base, metadata: original("5.74", "USDX") })).toBeNull();
  });
});
