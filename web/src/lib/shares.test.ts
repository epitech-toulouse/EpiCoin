import { describe, expect, it } from 'vitest';
import { formatSeconds, reasonLabel } from './format';
import { BADGES, MENTIONS, mentionLabel, tokenFromFragment } from './shares';

describe('knowledge shares', () => {
  it('reads the token of a QR code from the fragment only', () => {
    expect(tokenFromFragment('#token=ab12')).toBe('ab12');
    expect(tokenFromFragment('token=ab12')).toBe('ab12');
    expect(tokenFromFragment('')).toBe('');
    expect(tokenFromFragment('#other=1')).toBe('');
  });

  it('names mentions and keeps unknown codes readable', () => {
    expect(mentionLabel('wtf')).toBe('WTF?!');
    expect(mentionLabel('new_one')).toBe('new_one');
    expect(mentionLabel(null)).toBeNull();
  });

  it('has one badge per code of the API, the admin mention last', () => {
    // shares::badges: 6 badges and 10 mentions.
    expect(BADGES).toHaveLength(16);
    expect(new Set(BADGES.map((badge) => badge.code)).size).toBe(16);
    expect(MENTIONS.at(-1)).toMatchObject({ code: 'legendary', adminOnly: true });
  });

  it('labels the reason of a share payment', () => {
    expect(reasonLabel('knowledge_share')).toBe('Partage de connaissance');
  });

  it('formats a countdown', () => {
    expect(formatSeconds(125)).toBe('2 min 05 s');
    expect(formatSeconds(40)).toBe('40 s');
    expect(formatSeconds(0)).toBe('0 s');
  });
});
