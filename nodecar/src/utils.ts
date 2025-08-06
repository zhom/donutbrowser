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
