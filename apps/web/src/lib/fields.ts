// Small form helpers shared by the auth pages.

/** Accepts an invite token or a full invite link (`…/invite/<token>`). */
export function inviteToken(input: string): string {
  const value = input.trim();
  const match = /\/invite\/([^/?#\s]+)/.exec(value);
  return match?.[1] ? decodeURIComponent(match[1]) : value;
}

/** Display name of a client OS sent by the desktop app (`windows`, `linux`…). */
export function osName(os: string): string {
  const names: Record<string, string> = {
    windows: 'Windows',
    linux: 'Linux',
    macos: 'macOS',
    darwin: 'macOS'
  };
  return names[os.trim().toLowerCase()] ?? os.trim();
}

/** A loose shape check for an email address; the server decides what is valid. */
export function isEmail(value: string): boolean {
  return /^[^\s@]+@[^\s@]+\.[^\s@]+$/u.test(value.trim());
}

/** "Chrome · Windows" from a user agent, or `null` when nothing is recognised. */
export function browserLabel(userAgent: string | null): string | null {
  if (!userAgent) return null;
  const browsers: [RegExp, string][] = [
    [/Edg\//u, 'Edge'],
    [/OPR\//u, 'Opera'],
    [/Firefox\//u, 'Firefox'],
    [/Chrome\//u, 'Chrome'],
    [/Safari\//u, 'Safari']
  ];
  const systems: [RegExp, string][] = [
    [/Windows/u, 'Windows'],
    [/Android/u, 'Android'],
    [/iPhone|iPad/u, 'iOS'],
    [/Mac OS X/u, 'macOS'],
    [/Linux/u, 'Linux']
  ];
  const browser = browsers.find(([pattern]) => pattern.test(userAgent))?.[1];
  const system = systems.find(([pattern]) => pattern.test(userAgent))?.[1];
  const parts = [browser, system].filter(Boolean);
  return parts.length > 0 ? parts.join(' · ') : null;
}
