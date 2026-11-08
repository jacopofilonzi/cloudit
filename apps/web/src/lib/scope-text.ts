// Small text helpers shared by the scope views.
import { locale } from '@cloudit/ui';
import type { Lang } from '@cloudit/ui/i18n';

/** "Anna, Marco and Luca" in the language's list style. */
export function joinNames(names: readonly string[], lang: Lang): string {
  return new Intl.ListFormat(locale(lang), { style: 'long', type: 'conjunction' }).format(names);
}
