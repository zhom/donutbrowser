/**
 * Trims a name to a maximum length and adds ellipsis if needed
 * @param name The name to trim
 * @param maxLength Maximum length before truncation (default: 30)
 * @returns Trimmed name with ellipsis if truncated
 */
export function trimName(name: string, maxLength: number = 30): string {
  return name.length > maxLength ? `${name.substring(0, maxLength)}...` : name;
}
