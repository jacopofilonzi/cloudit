// Scope roles are ordered viewer < editor < admin < owner (see `ScopeRole`).
import type { ScopeRole } from '@cloudit/api-types';

const ORDER: readonly ScopeRole[] = ['viewer', 'editor', 'admin', 'owner'];

/** Whether `role` is `min` or more powerful. */
export function atLeast(role: ScopeRole, min: ScopeRole): boolean {
  return ORDER.indexOf(role) >= ORDER.indexOf(min);
}
