/** Loads a widget; malformed input is an error. */
export function loadWidget(text: string): string {
  if (text.length === 0) {
    throw new Error("empty widget");
  }
  return text;
}

/** Renders a placeholder for a widget with no data. */
export function placeholder(): string {
  return "-";
}
