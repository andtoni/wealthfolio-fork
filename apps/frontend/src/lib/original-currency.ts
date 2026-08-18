export interface OriginalCurrencyDisplay {
  amount: string;
  currency: string;
  label: string;
}

interface OriginalCurrencyActivity {
  metadata?: unknown;
  currency?: string | null;
  status?: string | null;
  sourceType?: string | null;
  subtype?: string | null;
  activityType?: string | null;
  activityTypeOverride?: string | null;
}

const DECIMAL_AMOUNT = /^[+-]?\d+(?:\.\d+)?$/;
const CURRENCY_CODE = /^[A-Z]{3}$/;

function record(value: unknown): Record<string, unknown> | undefined {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : undefined;
}

function parseMetadata(value: unknown): Record<string, unknown> | undefined {
  if (typeof value === "string") {
    try {
      return record(JSON.parse(value));
    } catch {
      return undefined;
    }
  }
  return record(value);
}

function formatAmount(amount: string, currency: string): string | undefined {
  const numeric = Number(amount);
  if (!Number.isFinite(numeric) || numeric === 0) return undefined;
  try {
    return new Intl.NumberFormat(undefined, {
      style: "currency",
      currency,
      currencyDisplay: "narrowSymbol",
      maximumFractionDigits: 8,
    }).format(Math.abs(numeric));
  } catch {
    return undefined;
  }
}

/**
 * Returns explicit original card currency evidence for a posted foreign-card
 * settlement. This is presentation-only: the activity's amount/currency stay
 * the posted settlement economics.
 */
export function isProvisionalCardSpending(activity: OriginalCurrencyActivity): boolean {
  const status = activity.status?.trim().toUpperCase();
  const sourceType = activity.sourceType?.trim().toUpperCase();
  const subtype = activity.subtype?.trim().toUpperCase();
  const activityType = activity.activityType?.trim().toUpperCase();
  const activityTypeOverride = activity.activityTypeOverride;
  const isCard =
    sourceType === "CARD" ||
    sourceType === "CARDPURCHASES" ||
    sourceType === "CARD_PURCHASE" ||
    subtype === "CARD_PURCHASE";
  if (
    status !== "PENDING" ||
    activityTypeOverride != null ||
    activityType !== "WITHDRAWAL" ||
    !isCard
  ) {
    return false;
  }

  const metadata = parseMetadata(activity.metadata);
  const investec = record(record(metadata?.providerOwned)?.investec);
  return investec?.provisionalSpending === true;
}

export function getOriginalCurrencyDisplay(
  activity: OriginalCurrencyActivity,
): OriginalCurrencyDisplay | null {
  const status = activity.status?.trim().toUpperCase();
  const sourceType = activity.sourceType?.trim().toUpperCase();
  const subtype = activity.subtype?.trim().toUpperCase();
  const primaryCurrency = activity.currency?.trim().toUpperCase();
  const isCardSettlement =
    sourceType === "CARD" ||
    sourceType === "CARDPURCHASES" ||
    sourceType === "CARD_PURCHASE" ||
    subtype === "CARD_PURCHASE";

  if (status !== "POSTED" || !isCardSettlement || !primaryCurrency) return null;

  const metadata = parseMetadata(activity.metadata);
  const providerOwned = record(metadata?.providerOwned);
  const original = record(providerOwned?.original);
  const rawAmount = original?.amount;
  const rawCurrency = original?.currency;
  const amount =
    typeof rawAmount === "string"
      ? rawAmount.trim()
      : typeof rawAmount === "number"
        ? String(rawAmount)
        : undefined;
  const currency = typeof rawCurrency === "string" ? rawCurrency.trim().toUpperCase() : undefined;

  if (
    !amount ||
    !DECIMAL_AMOUNT.test(amount) ||
    !currency ||
    !CURRENCY_CODE.test(currency) ||
    currency === primaryCurrency
  ) {
    return null;
  }

  const formattedAmount = formatAmount(amount, currency);
  return formattedAmount ? { amount, currency, label: `${formattedAmount} ${currency}` } : null;
}
