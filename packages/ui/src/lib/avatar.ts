// Avatar helpers: initials and a stable colour per user (no images are stored).

/** Number of `--cl-avatar-N` colour tokens in `styles.css`. */
export const AVATAR_COLORS = 8;

/** Up to two initials from a display name (or an email's local part). */
export function initials(name: string): string {
  const base = name.includes('@') ? (name.split('@')[0] ?? '') : name;
  const words = base
    .split(/[\s._-]+/u)
    .map((word) => word.trim())
    .filter(Boolean);
  const letters = words.length > 1 ? [words[0], words.at(-1)] : [words[0]];
  return letters
    .map((word) => (word ? Array.from(word)[0] : ''))
    .join('')
    .toLocaleUpperCase();
}

/** A colour index in `0..AVATAR_COLORS` derived from a stable key (user id or email). */
export function avatarColorIndex(key: string): number {
  let hash = 0;
  for (const char of key) hash = (hash * 31 + (char.codePointAt(0) ?? 0)) >>> 0;
  return hash % AVATAR_COLORS;
}
