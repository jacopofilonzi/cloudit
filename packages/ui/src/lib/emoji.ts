// Folder emoji: the curated picker set and the validation rules shared with the
// server (`validate::emoji`).

/** A small curated set for folders (documents, games, places, media, objects). */
// prettier-ignore
export const FOLDER_EMOJI = [
  '📁', '🗂️', '📄', '📚', '📝', '💼', '🧾', '📊',
  '🎮', '🕹️', '👾', '🏰', '⚔️', '🛡️', '🧙', '🐉',
  '🌍', '🏝️', '🏔️', '🌲', '🚀', '🪐', '⛏️', '🧱',
  '🎵', '🎬', '📷', '🎨', '🖼️', '🎧', '📺', '🎲',
  '💻', '🖥️', '⚙️', '🔧', '🧪', '🔒', '💾', '☁️',
  '🏠', '🚗', '✈️', '🍕', '❤️', '⭐', '🔥', '🌈'
] as const;

/** Longest emoji in UTF-8 bytes (as the server). */
export const MAX_EMOJI_BYTES = 32;

/**
 * Whether the server accepts `value` as a folder emoji: trimmed, `""` (none) or at
 * most 32 bytes with no whitespace, control characters or letters and at least one
 * non-ASCII character.
 */
export function isValidEmoji(value: string): boolean {
  const text = value.trim();
  if (!text) return true;
  if (new TextEncoder().encode(text).length > MAX_EMOJI_BYTES) return false;
  if (/[\s\p{Cc}\p{Alphabetic}]/u.test(text)) return false;
  return [...text].some((char) => (char.codePointAt(0) ?? 0) > 0x7f);
}
