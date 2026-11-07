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
  'error.unauthorized': 'Please sign in.',
  'error.forbidden': 'You are not allowed to do this.',
  'error.invalid_credentials': 'Wrong email or password.',
  'error.account_disabled': 'This account has been disabled.',
  'error.registration_closed': 'Registration is closed on this server.',
  'error.invite_invalid':
    'This invite is not valid: it may be used, expired or meant for another email.',
  'error.token_invalid': 'This link or code is not valid or has expired.',
  'error.email_taken': 'An account with this email already exists.',
  'error.validation_failed': 'Some fields are not valid.',
  'error.rate_limited': 'Too many attempts. Wait a moment and try again.',
  'error.csrf_failed':
    'The request was blocked for security reasons. Reload the page and try again.',
  'error.setup_required': 'The server needs to be set up first.',
  'error.setup_already_done': 'The server has already been set up.',
  'error.setup_token_invalid': 'Wrong setup token.',
  'error.last_admin': 'At least one active administrator must remain.',
  'error.provider_not_found': 'This sign-in provider does not exist.',
  'error.provider_id_taken': 'This provider id is already in use (also by a deleted provider).',
  'error.provider_discovery_failed':
    'Cannot read the configuration of the sign-in provider. Check the issuer URL.',
  'error.provider_error': 'The sign-in provider did not answer as expected. Try again later.',
  'error.public_url_required':
    "The server's public address is not configured yet: an administrator must set it before signing in with a provider.",
  'error.provider_disabled': 'This sign-in method is disabled.',
  'error.oauth_state_invalid':
    'This sign-in attempt is no longer valid. Start again from the same browser.',
  'error.oauth_cancelled': 'Sign-in was cancelled.',
  'error.group_not_allowed': 'Your account at this provider is not allowed to sign in here.',
  'error.identity_in_use': 'This account is already linked to another cloudit user.',
  'error.identity_not_linked': 'This account is not linked to any cloudit user.',
  'error.onboarding_expired': 'This sign-up has expired. Sign in again.',
  'error.last_login_method':
    'This is your last way to sign in: set a password or link another account first.',
  'error.account_pending':
    'Your account is waiting for approval by an administrator. Try again later.',
  'error.account_rejected': 'This account has been rejected by an administrator.',
  'error.name_taken': 'This name is already in use here.',
  'error.user_not_found': 'No account with this email was found.',
  'error.network': 'Cannot reach the server.',
  'error.unknown': 'Unexpected error.'
} as const;

export type MessageKey = keyof typeof en;

/** A full translation: exactly the keys of `en`, each mapped to a string. */
export type Dictionary = { readonly [K in MessageKey]: string };
