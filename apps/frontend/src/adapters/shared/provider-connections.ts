// Provider Connections — contract-safe local status for typed provider allowlist.
//
// No real provider network calls, credentials, or secrets are involved.
// See apps/server/src/api/provider_connections.rs for the backend module.

import { invoke } from "#platform";

// ────────────────────────────────────────────────────────────────────────────
// Types
// ────────────────────────────────────────────────────────────────────────────

/** Canonical provider identifiers. */
export type ProviderKind = "investec" | "mercury" | "plaid" | "snaptrade";

/** Truthful connection status for one provider. */
export interface ProviderConnectionStatus {
  provider: ProviderKind;
  /** One of `"not_configured"` or `"unavailable"`. Never `"connected"`. */
  status: "not_configured" | "unavailable" | "auth_required";
  /** Human-readable explanation suitable for UI display. */
  message: string;
}

// ────────────────────────────────────────────────────────────────────────────
// API Functions
// ────────────────────────────────────────────────────────────────────────────

/**
 * Fetch provider connection statuses.
 * Returns truthful `not_configured` states — no secrets, no synthetic URLs.
 */
export async function listProviderConnections(): Promise<ProviderConnectionStatus[]> {
  return invoke<ProviderConnectionStatus[]>("list_provider_connections");
}

/**
 * Initiate a provider connection attempt. Always fails closed — returns 501
 * Not Implemented for every provider because real provider linking is not
 * yet implemented. No URL, token, or credentials are generated.
 */
export async function startProviderConnection(provider: ProviderKind): Promise<void> {
  await invoke("start_provider_connection", { provider });
}
