// Formatting and parsing of what users read and type.
//
// EPC amounts are integers in units (1 EPC = 100 units) everywhere in the code. Converting
// from and to text is done on digits, never through floating-point arithmetic.

import type { OrderStatus, ReasonCode, Role } from './types';

export const UNITS_PER_EPC = 100;

/** `1250` → `"12,50"`; whole amounts drop the decimals: `1200` → `"12"`. */
export function formatAmount(units: number): string {
  const sign = units < 0 ? '−' : '';
  const absolute = Math.abs(Math.trunc(units));
  const whole = Math.floor(absolute / UNITS_PER_EPC);
  const cents = absolute % UNITS_PER_EPC;
  // Narrow no-break spaces between thousands, as French typography wants.
  const grouped = String(whole).replace(/\B(?=(\d{3})+(?!\d))/g, '\u202f');
  return cents === 0 ? `${sign}${grouped}` : `${sign}${grouped},${String(cents).padStart(2, '0')}`;
}

/** `1250` → `"12,50 EPC"`. */
export function formatEpc(units: number): string {
  return `${formatAmount(units)}\u00a0EPC`;
}

/**
 * Parses what a user typed as an EPC amount (`"12"`, `"12,5"`, `"12.50"`, `"1 200"`) into
 * units. Returns `null` for anything else: negative, more than two decimals, not a number.
 */
export function parseEpc(text: string): number | null {
  const cleaned = text.replace(/[\s\u00a0\u202f]/g, '').replace(',', '.');
  const match = /^(\d{1,9})(?:\.(\d{1,2}))?$/.exec(cleaned);
  if (!match) return null;
  const whole = Number(match[1]);
  const cents = Number((match[2] ?? '').padEnd(2, '0'));
  return whole * UNITS_PER_EPC + cents;
}

const dateTime = new Intl.DateTimeFormat('fr-FR', { dateStyle: 'medium', timeStyle: 'short' });
const dateOnly = new Intl.DateTimeFormat('fr-FR', { dateStyle: 'long' });

export function formatDateTime(value: string | number): string {
  return dateTime.format(new Date(value));
}

export function formatDate(value: string | number): string {
  return dateOnly.format(new Date(value));
}

export const REASON_LABELS: Record<ReasonCode, string> = {
  participation: 'Participation',
  peer_help: 'Entraide',
  outstanding_project: 'Projet remarquable',
  event: 'Événement',
  campus_life: 'Vie du campus',
  challenge: 'Défi',
  other: 'Autre',
};

export const REASON_CODES = Object.keys(REASON_LABELS) as ReasonCode[];

export function reasonLabel(code: string | null | undefined): string {
  return (code && REASON_LABELS[code as ReasonCode]) || 'Sans motif';
}

const COHORT_PROGRAMS: Record<string, string> = { msc: 'MSc', pge: 'PGE', bach: 'Bachelor' };

/** `pge1` → "PGE 1", `msc2027` → "MSc 2027"; any other value is shown as is. */
export function cohortLabel(code: string): string {
  const [, program, number] = /^([a-z]+)(\d+)$/.exec(code) ?? [];
  if (!program || !number) return code;
  return `${COHORT_PROGRAMS[program] ?? program.toUpperCase()} ${number}`;
}

export const ROLE_LABELS: Record<Role, string> = {
  student: 'Étudiant',
  staff: 'Encadrant',
  shop_manager: 'Boutique',
  admin: 'Administrateur',
};

export const ORDER_STATUS_LABELS: Record<OrderStatus, string> = {
  awaiting_payment: 'Paiement en cours',
  failed: 'Paiement refusé',
  pending: 'En préparation',
  ready: 'Prête à retirer',
  delivered: 'Remise',
  refunding: 'Remboursement en cours',
  cancelled: 'Annulée et remboursée',
};

/** A short, readable form of a 64-character hash or address. */
export function shortHash(hash: string): string {
  return hash.length > 16 ? `${hash.slice(0, 8)}…${hash.slice(-6)}` : hash;
}

export function fullName(person: { first_name: string; last_name: string }): string {
  return `${person.first_name} ${person.last_name}`;
}
