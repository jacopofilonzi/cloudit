import { describe, expect, it as test } from 'vitest';
import { FOLDER_EMOJI, isValidEmoji } from './emoji';

describe('isValidEmoji', () => {
  test('matches the server rules', () => {
    for (const ok of ['', '  ', '🌍', ' 🏝️ ', '1️⃣', '👨‍👩‍👧‍👦', '🇮🇹']) {
      expect(isValidEmoji(ok), ok).toBe(true);
    }
    for (const bad of ['a', 'ab', '12', '🌍 🌍', '🌍\u0007🌍', 'é', '🌍'.repeat(9)]) {
      expect(isValidEmoji(bad), bad).toBe(false);
    }
  });

  test('every curated emoji is valid', () => {
    expect(FOLDER_EMOJI.every((emoji) => isValidEmoji(emoji))).toBe(true);
  });
});
