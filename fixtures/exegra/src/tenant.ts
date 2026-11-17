/** Resolves the tenant of a request from its `Tenant` header. */
export function resolveTenant(headers: Record<string, string>): string {
  const t = headers["tenant"];
  if (!t) {
    throw new Error("no tenant");
  }
  return t;
}
