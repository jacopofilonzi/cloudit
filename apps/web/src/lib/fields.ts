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
