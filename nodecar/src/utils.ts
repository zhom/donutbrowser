import type { LaunchOptions } from "playwright-core";

const OS_MAP: { [key: string]: "mac" | "win" | "lin" } = {
  darwin: "mac",
  linux: "lin",
  win32: "win",
};

const OS_NAME: "mac" | "win" | "lin" = OS_MAP[process.platform];

export function getEnvVars(configMap: Record<string, string>) {
  const envVars: {
    [key: string]: string | number | boolean;
  } = {};
  let updatedConfigData: Uint8Array;

  try {
    updatedConfigData = new TextEncoder().encode(JSON.stringify(configMap));
  } catch (e) {
    console.error(`Error updating config: ${e}`);
    process.exit(1);
  }

  const chunkSize = OS_NAME === "win" ? 2047 : 32767;
  const configStr = new TextDecoder().decode(updatedConfigData);

  for (let i = 0; i < configStr.length; i += chunkSize) {
    const chunk = configStr.slice(i, i + chunkSize);
    const envName = `CAMOU_CONFIG_${Math.floor(i / chunkSize) + 1}`;
    try {
      envVars[envName] = chunk;
    } catch (e) {
      console.error(`Error setting ${envName}: ${e}`);
      process.exit(1);
    }
  }

  return envVars;
}

export function parseProxyString(proxyString: LaunchOptions["proxy"] | string) {
  if (typeof proxyString === "object") {
    return proxyString;
  }

  if (!proxyString || typeof proxyString !== "string") {
    throw new Error("Invalid proxy string provided");
  }

  // Remove any leading/trailing whitespace
  const trimmed = proxyString.trim();

  // Handle different proxy string formats:
  // 1. http://username:password@host:port
  // 2. host:port
  // 3. protocol://host:port
  // 4. username:password@host:port

  let server = "";
  let username: string | undefined;
  let password: string | undefined;

  try {
    // Try parsing as URL first (handles protocol://username:password@host:port)
    if (trimmed.includes("://")) {
      const url = new URL(trimmed);
      server = `${url.hostname}:${url.port}`;

      if (url.username) {
        username = decodeURIComponent(url.username);
      }
      if (url.password) {
        password = decodeURIComponent(url.password);
      }
    } else {
      // Handle formats without protocol
      let workingString = trimmed;

      // Check for username:password@ prefix
      const authMatch = workingString.match(/^([^:@]+):([^@]+)@(.+)$/);
      if (authMatch) {
        username = authMatch[1];
        password = authMatch[2];
        workingString = authMatch[3];
      }

      // The remaining part should be host:port
      server = workingString;
    }

    // Validate that we have a server
    if (!server) {
      throw new Error("Could not extract server information");
    }

    // Basic validation for host:port format
    if (!server.includes(":") || server.split(":").length !== 2) {
      throw new Error("Server must be in host:port format");
    }

    const result: LaunchOptions["proxy"] = { server };

    if (username !== undefined) {
      result.username = username;
    }

    if (password !== undefined) {
      result.password = password;
    }

    return result;
  } catch (error) {
    throw new Error(
      `Failed to parse proxy string: ${error instanceof Error ? error.message : "Unknown error"}`,
    );
  }
}
