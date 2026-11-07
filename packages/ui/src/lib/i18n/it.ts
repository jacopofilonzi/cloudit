import type { Dictionary } from './en';

export const it: Dictionary = {
  'app.name': 'cloudit',
  'app.tagline': 'Sincronizzazione di cartelle self-hosted',
  'language.label': 'Lingua',
  'language.en': 'English',
  'language.it': 'Italiano',
  'health.title': 'Stato del server',
  'health.loading': 'Verifica del server…',
  'health.ok': 'Online',
  'health.version': 'Versione {version}',
  'health.retry': 'Verifica di nuovo',
  'error.bad_request': 'La richiesta non è valida.',
  'error.not_found': 'Non trovato.',
  'error.unavailable': 'Il server non è al momento disponibile. Riprova più tardi.',
  'error.internal': 'Si è verificato un errore sul server.',
  'error.unauthorized': 'Accedi per continuare.',
  'error.forbidden': 'Non hai i permessi per questa operazione.',
  'error.invalid_credentials': 'Email o password errata.',
  'error.account_disabled': 'Questo account è stato disattivato.',
  'error.registration_closed': 'Le registrazioni sono chiuse su questo server.',
  'error.invite_invalid':
    "Questo invito non è valido: potrebbe essere già usato, scaduto o destinato a un'altra email.",
  'error.token_invalid': 'Questo link o codice non è valido o è scaduto.',
  'error.email_taken': 'Esiste già un account con questa email.',
  'error.validation_failed': 'Alcuni campi non sono validi.',
  'error.rate_limited': 'Troppi tentativi. Attendi un momento e riprova.',
  'error.csrf_failed':
    'La richiesta è stata bloccata per motivi di sicurezza. Ricarica la pagina e riprova.',
  'error.setup_required': 'Il server deve prima essere configurato.',
  'error.setup_already_done': 'Il server è già stato configurato.',
  'error.setup_token_invalid': 'Token di configurazione errato.',
  'error.last_admin': 'Deve rimanere almeno un amministratore attivo.',
  'error.provider_not_found': 'Questo provider di accesso non esiste.',
  'error.provider_id_taken': 'Questo id di provider è già in uso (anche da un provider eliminato).',
  'error.provider_discovery_failed':
    "Impossibile leggere la configurazione del provider di accesso. Controlla l'URL dell'issuer.",
  'error.provider_error':
    'Il provider di accesso non ha risposto come previsto. Riprova più tardi.',
  'error.network': 'Impossibile raggiungere il server.',
  'error.unknown': 'Errore imprevisto.'
};
