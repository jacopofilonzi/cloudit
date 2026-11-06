// English dictionary: the reference set of keys. Every other language is typed
// against these keys (see `Dictionary`), so a missing or extra key fails the type check.
export const en = {
  'app.name': 'cloudit',
  'app.tagline': 'Self-hosted folder sync',
  'language.label': 'Language',
  'language.en': 'English',
  'language.it': 'Italiano',
  'health.title': 'Server status',
  'health.loading': 'Checking the server…',
  'health.ok': 'Online',
  'health.version': 'Version {version}',
  'health.retry': 'Check again',
  'error.bad_request': 'The request was not valid.',
  'error.not_found': 'Not found.',
  'error.unavailable': 'The server is temporarily unavailable. Try again later.',
  'error.internal': 'Something went wrong on the server.',
  'error.network': 'Cannot reach the server.',
  'error.unknown': 'Unexpected error.'
} as const;

export type MessageKey = keyof typeof en;

/** A full translation: exactly the keys of `en`, each mapped to a string. */
export type Dictionary = { readonly [K in MessageKey]: string };
