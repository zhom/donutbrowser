/** Where an agent points to drive this browser through Donut cloud. */
export const REMOTE_MCP_URL = "https://api.donutbrowser.com/api/mcp";

/**
 * fx refuses a literal Authorization header in its config and reads the
 * bearer token from this variable instead, so its install cannot carry the
 * credential and the user has to export it themselves.
 */
export const FX_TOKEN_ENV = "DONUT_MCP_TOKEN";
export const FX_AGENT_ID = "fx";

/** The line that hands fx the credential, or null while there is none. */
export function fxExportLine(key: string | null | undefined): string | null {
  return key ? `export ${FX_TOKEN_ENV}=${key}` : null;
}

export type AgentCategory = "desktop-app" | "cli" | "editor" | "editor-ext";

/** The two places a client can be pointed at; the `target` of `add_mcp_to_agent`. */
export type McpEndpoint = "local" | "remote";

export interface McpAgentInfo {
  id: string;
  display_name: string;
  category: AgentCategory;
  connected: boolean;
  detected: boolean;
  /** Which Donut endpoint the agent's existing entry points at, when connected. */
  endpoint?: McpEndpoint | null;
}

export interface McpRemoteStatus {
  enabled: boolean;
  connected: boolean;
  instanceId: string;
  lastError: string | null;
}

/**
 * Only the prefix leaves the backend; the plaintext is installed into agent
 * configs by the app itself. Both spellings are read because the spec writes
 * the Rust field as `token_prefix` while the sibling status struct serialises
 * camelCase, and a mismatch here would silently render a present credential
 * as missing.
 */
export interface McpRemoteCredential {
  present: boolean;
  tokenPrefix?: string | null;
  token_prefix?: string | null;
}

/**
 * What a rotation answers. The key is stored and installed by the time this
 * arrives; `failed_clients` are the ids of the clients whose config could not
 * be rewritten, for the user to retry, never a reason to mint again.
 */
export interface McpRemoteCredentialRotation
  extends Pick<McpRemoteCredential, "tokenPrefix" | "token_prefix"> {
  failed_clients?: string[];
}

export function credentialPrefixOf(
  credential: Pick<McpRemoteCredential, "tokenPrefix" | "token_prefix"> | null,
): string | null {
  return credential?.tokenPrefix ?? credential?.token_prefix ?? null;
}

/** Mirror of `mcp_migration::McpMigrationOffer`. */
export interface McpMigrationOffer {
  eligible: boolean;
  due: boolean;
  local_server_enabled: boolean;
  local_clients: string[];
}

/** Clients whose Donut entry still points at the removed local server. */
export function localMcpClients(agents: McpAgentInfo[]): McpAgentInfo[] {
  return agents.filter(
    (agent) => agent.connected && agent.endpoint === "local",
  );
}
