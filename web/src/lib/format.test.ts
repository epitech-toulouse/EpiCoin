import { describe, expect, it } from 'vitest';
import { extractEmails } from './csv';
import { formatAmount, formatEpc, parseEpc, reasonLabel, shortHash } from './format';

describe('formatAmount', () => {
  it('shows whole amounts without decimals', () => {
    expect(formatAmount(0)).toBe('0');
    expect(formatAmount(1200)).toBe('12');
  });

  it('shows two decimals when there are cents', () => {
    expect(formatAmount(1250)).toBe('12,50');
    expect(formatAmount(5)).toBe('0,05');
  });

  it('groups thousands', () => {
    expect(formatAmount(123_456_700)).toBe('1\u202f234\u202f567');
    expect(formatEpc(100_000)).toBe('1\u202f000\u00a0EPC');
  });
});

describe('parseEpc', () => {
  it('accepts commas, dots and spaces', () => {
    expect(parseEpc('12')).toBe(1200);
    expect(parseEpc('12,5')).toBe(1250);
    expect(parseEpc('12.50')).toBe(1250);
    expect(parseEpc(' 1 200 ')).toBe(120_000);
    expect(parseEpc('0,07')).toBe(7);
  });

  it('is exact where floating point is not', () => {
    // 0.1 + 0.2 !== 0.3 in floating point; amounts never go through it.
    expect(parseEpc('0,1')! + parseEpc('0,2')!).toBe(parseEpc('0,3'));
    expect(parseEpc('1,15')).toBe(115);
    expect(parseEpc('4,35')).toBe(435);
  });

  it('rejects what is not an amount', () => {
    for (const bad of ['', '-5', '1,234', 'abc', '1e3', '1,2,3', ',5', '5,', '12 EPC']) {
      expect(parseEpc(bad), bad).toBeNull();
    }
  });

  it('round-trips with formatAmount', () => {
    for (const units of [0, 1, 99, 100, 101, 1250, 123_456]) {
      expect(parseEpc(formatAmount(units))).toBe(units);
    }
  });
});

describe('labels', () => {
  it('translates reason codes', () => {
    expect(reasonLabel('peer_help')).toBe('Entraide');
    expect(reasonLabel(null)).toBe('Sans motif');
    expect(reasonLabel('unknown_code')).toBe('Sans motif');
  });

  it('shortens hashes', () => {
    expect(shortHash('ab'.repeat(32))).toBe('abababab…ababab');
    expect(shortHash('short')).toBe('short');
  });
});

describe('extractEmails', () => {
  it('finds emails whatever the layout', () => {
    const csv = 'email;prenom;nom\nAda@Epitech.eu;Ada;Lovelace\n"grace@epitech.eu";Grace;Hopper\n';
    expect(extractEmails(csv)).toEqual(['ada@epitech.eu', 'grace@epitech.eu']);
    expect(extractEmails('a@epitech.eu, b@epitech.eu\tc@epitech.eu')).toHaveLength(3);
  });

  it('deduplicates and ignores the rest', () => {
    expect(extractEmails('ada@epitech.eu\nADA@epitech.eu\nnot an email\n@nope\n')).toEqual([
      'ada@epitech.eu',
    ]);
    expect(extractEmails('')).toEqual([]);
  });
});
