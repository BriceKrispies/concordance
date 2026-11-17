import { describe, expect, it } from "vitest";
import { resolveTenant } from "./tenant";

describe("resolveTenant", () => {
  // Verifies: TEN-1
  it("reads the tenant header", () => {
    expect(resolveTenant({ tenant: "acme" })).toBe("acme");
  });

  // Verifies: FR-ISO-1
  it("rejects a request without a tenant", () => {
    expect(() => resolveTenant({})).toThrow();
  });
});
