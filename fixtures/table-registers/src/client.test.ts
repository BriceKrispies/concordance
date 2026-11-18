import { it, expect } from "vitest";
import { placeholder } from "./client";

export function makeWidget(text: string): string {
  if (!text) {
    throw new Error("no text");
  }
  return text;
}

it("renders a placeholder", () => {
  expect(placeholder()).toBe("-");
});
