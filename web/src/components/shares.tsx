// Pieces of the knowledge-share screens: the QR code, badge tiles, mention chips.

import {
  BookOpen,
  Brain,
  Crown,
  FlaskConical,
  Flame,
  Footprints,
  Globe,
  HandCoins,
  MapIcon,
  Mic,
  Rabbit,
  Shuffle,
  Sparkles,
  Star,
  Wrench,
  Zap,
  type LucideIcon,
} from 'lucide-react';
import { useEffect, useState } from 'react';
import { formatDate } from '../lib/format';
import { mentionLabel, type BadgeInfo } from '../lib/shares';
import type { EarnedBadge } from '../lib/types';
import { cx } from './ui';

export const BADGE_ICONS: Record<string, LucideIcon> = {
  first: Footprints,
  tireless: Flame,
  encyclopedia: BookOpen,
  tour: MapIcon,
  tip: HandCoins,
  legend: Crown,
  genius: Brain,
  wtf: Zap,
  mind_blown: Sparkles,
  life_hack: Wrench,
  plot_twist: Shuffle,
  showtime: Mic,
  rabbit_hole: Rabbit,
  mad_scientist: FlaskConical,
  culture_shock: Globe,
  legendary: Star,
};

/**
 * A QR code drawn from its modules, black on white with a four-module quiet zone, so that
 * any phone camera reads it. One SVG path: no image, nothing loaded from elsewhere.
 */
export function QrCode({ size, modules, label }: { size: number; modules: string; label: string }) {
  const quiet = 4;
  let path = '';
  for (let row = 0; row < size; row += 1) {
    for (let column = 0; column < size; column += 1) {
      if (modules[row * size + column] === '1') path += `M${column + quiet} ${row + quiet}h1v1h-1z`;
    }
  }
  const full = size + 2 * quiet;
  return (
    <svg
      role="img"
      aria-label={label}
      viewBox={`0 0 ${full} ${full}`}
      shapeRendering="crispEdges"
      className="aspect-square w-full max-w-72 bg-white"
    >
      <rect width={full} height={full} fill="#ffffff" />
      <path d={path} fill="#000000" />
    </svg>
  );
}

/** A mention, as a small chip. */
export function MentionChip({ code }: { code: string | null }) {
  const label = mentionLabel(code);
  if (!code || !label) return null;
  const Icon = BADGE_ICONS[code] ?? Star;
  return (
    <span className="inline-flex items-center gap-1 bg-fuchsia-50 px-2 py-0.5 font-mono text-[0.6875rem] font-semibold text-fuchsia-800 ring-1 ring-fuchsia-200 ring-inset">
      <Icon aria-hidden="true" className="size-3" strokeWidth={2} />
      {label}
    </span>
  );
}

/** One badge of the collection: in colour once earned, greyed out before. */
export function BadgeTile({ badge, earned }: { badge: BadgeInfo; earned: EarnedBadge | undefined }) {
  const Icon = BADGE_ICONS[badge.code] ?? Star;
  const mention = badge.kind === 'mention';
  return (
    <li
      className={cx(
        'flex items-start gap-3 p-3 ring-1 ring-inset',
        earned ? 'bg-white ring-line' : 'bg-slate-50 ring-slate-200',
      )}
    >
      <span
        className={cx(
          'flex size-10 shrink-0 items-center justify-center',
          earned ? (mention ? 'bg-fuchsia-700 text-white' : 'bg-brand-600 text-white') : 'bg-slate-200 text-slate-400',
        )}
      >
        <Icon aria-hidden="true" className="size-5" strokeWidth={1.75} />
      </span>
      <span className="min-w-0">
        <span className={cx('block text-sm font-semibold', earned ? 'text-slate-900' : 'text-slate-500')}>
          {badge.label}
          {earned && earned.count > 1 && (
            <span className="ml-1.5 font-mono text-xs text-fuchsia-700">×{earned.count}</span>
          )}
        </span>
        <span className="block text-xs text-slate-600">{badge.description}</span>
        <span className="mt-1 block font-mono text-[0.6875rem] text-slate-500">
          {earned ? `Obtenu le ${formatDate(earned.obtained_at)}` : 'Pas encore obtenu'}
        </span>
      </span>
    </li>
  );
}

/** Seconds left until `deadline` (milliseconds since the epoch), updated several times a second. */
export function useSecondsLeft(deadline: number | null): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (deadline === null) return;
    const timer = setInterval(() => setNow(Date.now()), 250);
    return () => clearInterval(timer);
  }, [deadline]);
  return deadline === null ? 0 : Math.max(0, Math.ceil((deadline - now) / 1000));
}
