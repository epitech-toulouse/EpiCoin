// Knowledge shares ("Partages"): the names of mentions and badges, and the token of a
// scanned QR code. The API only knows codes; what they are called lives here.

export interface Mention {
  code: string;
  label: string;
  description: string;
  /** Only admins may give it. */
  adminOnly?: boolean;
}

/** In the API's order (`shares::badges::MENTIONS`). */
export const MENTIONS: Mention[] = [
  { code: 'genius', label: 'Genius', description: 'Une connaissance impressionnante' },
  { code: 'wtf', label: 'WTF?!', description: 'Une information improbable' },
  { code: 'mind_blown', label: 'Mind Blown', description: 'Une nouvelle façon de voir' },
  { code: 'life_hack', label: 'Life Hack', description: 'Une astuce immédiatement utile' },
  { code: 'plot_twist', label: 'Plot Twist', description: 'Une révélation inattendue' },
  { code: 'showtime', label: 'Showtime', description: 'Une présentation captivante' },
  { code: 'rabbit_hole', label: 'Rabbit Hole', description: 'Un sujet à explorer' },
  { code: 'mad_scientist', label: 'Mad Scientist', description: 'Une démonstration étonnante' },
  { code: 'culture_shock', label: 'Culture Shock', description: 'Une découverte culturelle' },
  {
    code: 'legendary',
    label: 'Legendary',
    description: 'Un partage exceptionnel',
    adminOnly: true,
  },
];

export function mentionLabel(code: string | null | undefined): string | null {
  if (!code) return null;
  return MENTIONS.find((mention) => mention.code === code)?.label ?? code;
}

export interface BadgeInfo {
  code: string;
  label: string;
  description: string;
  kind: 'badge' | 'mention';
}

export const BADGES: BadgeInfo[] = [
  { code: 'first', label: 'Premiers pas', description: 'Premier partage validé.', kind: 'badge' },
  {
    code: 'tireless',
    label: 'Infatigable',
    description: '20 jours ouvrés de partage d’affilée.',
    kind: 'badge',
  },
  { code: 'encyclopedia', label: 'Encyclopédie', description: '50 partages validés.', kind: 'badge' },
  {
    code: 'tour',
    label: 'Tour du campus',
    description: 'Des partages validés par 5 encadrants différents.',
    kind: 'badge',
  },
  {
    code: 'tip',
    label: 'Le Pourboire',
    description: 'Recevoir plus que le montant conseillé.',
    kind: 'badge',
  },
  {
    code: 'legend',
    label: 'Légende du campus',
    description: 'Distinction remise par un administrateur.',
    kind: 'badge',
  },
  ...MENTIONS.map<BadgeInfo>((mention) => ({
    code: mention.code,
    label: mention.label,
    description: `Recevoir la mention ${mention.label} : ${mention.description.toLowerCase()}.`,
    kind: 'mention',
  })),
];

/** ISO weekday (1 = Monday) → French name. */
export const WEEKDAYS: { value: number; label: string }[] = [
  { value: 1, label: 'Lundi' },
  { value: 2, label: 'Mardi' },
  { value: 3, label: 'Mercredi' },
  { value: 4, label: 'Jeudi' },
  { value: 5, label: 'Vendredi' },
  { value: 6, label: 'Samedi' },
  { value: 7, label: 'Dimanche' },
];

// --- The token of a scanned QR code ----------------------------------------------------
//
// The QR code opens `/partage#token=…`. The token is moved to sessionStorage at once (the
// student may have to log in first), then read by the claim page. It never reaches a server
// in a URL: the fragment is not sent, and the API receives it in a request body.

const STORAGE_KEY = 'epicoin.share-token';

export function tokenFromFragment(hash: string): string {
  return new URLSearchParams(hash.replace(/^#/, '')).get('token') ?? '';
}

export function rememberShareToken(token: string): void {
  try {
    sessionStorage.setItem(STORAGE_KEY, token);
  } catch {
    // Storage blocked: the claim page will say the link must be opened again.
  }
}

export function pendingShareToken(): string {
  try {
    return sessionStorage.getItem(STORAGE_KEY) ?? '';
  } catch {
    return '';
  }
}

export function forgetShareToken(): void {
  try {
    sessionStorage.removeItem(STORAGE_KEY);
  } catch {
    // Nothing to forget.
  }
}
