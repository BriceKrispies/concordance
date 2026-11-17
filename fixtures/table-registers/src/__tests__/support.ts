/** Test helper: a widget source that fails on demand. */
export function failingSource(): string {
  throw new Error("fixture failure");
}
