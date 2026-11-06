import { afterEach, describe, expect, it as test } from 'vitest';
import { en } from './en';
import { it } from './it';
import { DEFAULT_LANG, detectLang, getLang, isLang, setLang, t } from './i18n.svelte';

afterEach(() => setLang(DEFAULT_LANG));

describe('t', () => {
  test('translates in the default language', () => {
    expect(getLang()).toBe('en');
    expect(t('health.title')).toBe(en['health.title']);
  });

  test('follows the current language', () => {
    setLang('it');
    expect(t('health.title')).toBe(it['health.title']);
  });

  test('replaces placeholders and keeps unknown ones', () => {
    expect(t('health.version', { version: '1.2.3' })).toBe('Version 1.2.3');
    expect(t('health.version', {})).toBe('Version {version}');
  });
});

describe('detectLang', () => {
  test('picks the first supported base language', () => {
    expect(detectLang(['de-DE', 'it-IT', 'en'])).toBe('it');
    expect(detectLang(['EN-gb'])).toBe('en');
  });

  test('falls back to the default', () => {
    expect(detectLang([])).toBe(DEFAULT_LANG);
    expect(detectLang(['fr'])).toBe(DEFAULT_LANG);
  });
});

describe('dictionaries', () => {
  test('have the same keys', () => {
    expect(Object.keys(it).sort()).toEqual(Object.keys(en).sort());
    expect(isLang('it')).toBe(true);
    expect(isLang('fr')).toBe(false);
  });
});
