import { invoke } from "#platform";

export type ProviderKind = "investec" | "mercury" | "plaid" | "snaptrade";

export type ProviderConnectionState =
  | "available"
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
  /** One-use Plaid continuation bound to the authenticated subject. Never persist. */
  continuationToken?: string;
  /** Short-lived SnapTrade connection redirect. Never persist this value. */
  redirectUrl?: string;
}

export type ProviderConnectionCompleteResponse = ProviderConnectionStatus;

export interface DiscoveredProviderAsset {
  providerAssetId: string;
  symbol: string;
  name: string;
  currency: string;
  instrumentType: string;
}

export interface DiscoveredProviderAccount {
  connectionId: string;
  providerAccountId: string;
  name: string;
  type: string;
  currency: string;
  mask?: string;
  providerGeneration: number;
  assets: DiscoveredProviderAsset[];
}

export async function listProviderConnections(): Promise<ProviderConnectionStatus[]> {
  return invoke<ProviderConnectionStatus[]>("list_provider_connections");
}

export async function startProviderConnection(
  provider: ProviderKind,
): Promise<ProviderConnectionStartResponse> {
  return invoke<ProviderConnectionStartResponse>("start_provider_connection", { provider });
}

export async function listDiscoveredProviderAccounts(
  provider: ProviderKind,
): Promise<DiscoveredProviderAccount[]> {
  return invoke<DiscoveredProviderAccount[]>("list_discovered_provider_accounts", { provider });
}

export async function admitProviderAccount(
  provider: ProviderKind,
  input: Pick<DiscoveredProviderAccount, "connectionId" | "providerAccountId" | "name">,
): Promise<{ accountId: string }> {
  return invoke<{ accountId: string }>("admit_provider_account", { provider, ...input });
}

export async function completePlaidConnection(
  publicToken: string,
  continuationToken: string,
): Promise<ProviderConnectionCompleteResponse> {
  return invoke<ProviderConnectionCompleteResponse>("complete_plaid_connection", {
    publicToken,
    continuationToken,
  });
}
