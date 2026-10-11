/** One page per line, as the start-pages fields show and take them. */
export function startPagesFromText(text: string): string[] {
  return text
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean);
}
