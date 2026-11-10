import { describe, expect, it } from "vitest";
import { Mixer } from "./mixer";

describe("Mixer", () => {
  // Verifies: VOL-001
  it("rejects levels above 100", () => {
    expect(() => new Mixer().setVolume(101)).toThrow(RangeError);
  });

  it("keeps an accepted level", () => {
    const m = new Mixer();
    m.setVolume(40);
    expect(m.volume).toBe(40);
  });
});
