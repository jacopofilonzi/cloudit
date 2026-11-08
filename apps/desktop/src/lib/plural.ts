// "1 folder" / "3 folders": dictionary keys come in `<base>_one` / `<base>_other` pairs
// (English and Italian only need these two forms).
import { t, type MessageKey } from '@cloudit/ui/i18n';

type BaseOf<K> = K extends `${infer B}_one` ? B : never;

/** A key base that has both a `_one` and an `_other` form. */
export type PluralBase = BaseOf<MessageKey>;

export function tn(base: PluralBase, count: number, params: Record<string, string | number> = {}) {
  const key = `${base}_${count === 1 ? 'one' : 'other'}` as MessageKey;
  return t(key, { count, ...params });
}
