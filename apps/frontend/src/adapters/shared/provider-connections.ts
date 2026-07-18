import { invoke } from "#platform";

export type ProviderKind = "investec" | "mercury" | "plaid" | "snaptrade";

export type ProviderConnectionState =
  | "configured"
  | "not_configured"
  | "unavailable"
  | "auth_required"
  | "pending"
  | "link_ready"
  | "connected"
  | "completed";

export interface ProviderConnectionStatus {
  provider: ProviderKind;
  status: ProviderConnectionState;
  message: string;
}

export interface ProviderConnectionStartResponse extends ProviderConnectionStatus {
  /** Short-lived Plaid Link token. Never persist this value. */
  linkToken?: string;
  /** Short-lived SnapTrade connection redirect. Never persist this value. */
  redirectUrl?: string;
}

export interface ProviderConnectionCompleteResponse extends ProviderConnectionStatus {}

export async function listProviderConnections(): Promise<ProviderConnectionStatus[]> {
  return invoke<ProviderConnectionStatus[]>("list_provider_connections");
}

export async function startProviderConnection(
  provider: ProviderKind,
): Promise<ProviderConnectionStartResponse> {
  return invoke<ProviderConnectionStartResponse>("start_provider_connection", { provider });
}

export async function completePlaidConnection(
  publicToken: string,
): Promise<ProviderConnectionCompleteResponse> {
  return invoke<ProviderConnectionCompleteResponse>("complete_plaid_connection", { publicToken });
}
