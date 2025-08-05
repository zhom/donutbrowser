/**
 * Extracts the root error message from nested error strings
 * Removes redundant "Failed to..." prefixes to show only the most specific error
 */
export function extractRootError(error: unknown): string {
  if (!error) return "Unknown error";

  const errorStr = error instanceof Error ? error.message : String(error);

  // Split by common error prefixes and take the last meaningful part
  const errorParts = errorStr.split(/Failed to [^:]+: /);
  const rootError = errorParts[errorParts.length - 1];

  // Clean up any remaining nested structure
  const cleanError = rootError.replace(/^"([^"]+)"$/, "$1");

  return cleanError || errorStr;
}

/**
 * Shows error toast with cleaned error message
 */
export function showCleanErrorToast(error: unknown, prefix?: string) {
  const rootError = extractRootError(error);
  const message = prefix ? `${prefix}: ${rootError}` : rootError;

  // Import dynamically to avoid circular dependencies
  import("./toast-utils").then(({ showErrorToast }) => {
    showErrorToast(message);
  });
}
