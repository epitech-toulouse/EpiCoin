// Building blocks shared by every screen, in the campus intranet style: square white cards
// on a dotted canvas, monospace uppercase labels, condensed display figures.

import type { LucideIcon } from 'lucide-react';
import { ArrowRight } from 'lucide-react';
import {
  useEffect,
  useId,
  useRef,
  type ButtonHTMLAttributes,
  type InputHTMLAttributes,
  type ReactNode,
  type SelectHTMLAttributes,
  type TextareaHTMLAttributes,
} from 'react';
import { Link } from 'react-router-dom';
import { errorMessage } from '../lib/api';
import { formatAmount } from '../lib/format';

export function cx(...classes: (string | false | null | undefined)[]): string {
  return classes.filter(Boolean).join(' ');
}

/** The colours a title, a figure or an icon can take. */
export type Tone = 'brand' | 'ok' | 'warn' | 'danger' | 'teal' | 'accent' | 'neutral';

/** Text colours: darker steps of each hue, readable on white (contrast ≥ 4.5:1). */
export const TONE_TEXT: Record<Tone, string> = {
  brand: 'text-brand-600',
  ok: 'text-emerald-700',
  warn: 'text-amber-700',
  danger: 'text-red-600',
  teal: 'text-teal-700',
  accent: 'text-fuchsia-700',
  neutral: 'text-slate-600',
};

/** The CSS colour of each tone, for SVG marks. */
const TONE_VAR: Record<Tone, string> = {
  brand: 'var(--color-brand-600)',
  ok: 'var(--color-ok)',
  warn: 'var(--color-warn)',
  danger: 'var(--color-danger)',
  teal: 'var(--color-teal)',
  accent: 'var(--color-accent)',
  neutral: 'var(--color-slate-400)',
};

export const TONE_BAR: Record<Tone, string> = {
  brand: 'bg-brand-600',
  ok: 'bg-ok',
  warn: 'bg-warn',
  danger: 'bg-danger',
  teal: 'bg-teal',
  accent: 'bg-accent',
  neutral: 'bg-slate-400',
};

/** The trailing underscore of the intranet's titles. Decorative: hidden from screen readers. */
function Cursor() {
  return <span aria-hidden="true">_</span>;
}

// --- Buttons ---------------------------------------------------------------------------

type Variant = 'primary' | 'secondary' | 'danger' | 'ghost';

const VARIANTS: Record<Variant, string> = {
  primary: 'bg-brand-600 text-white hover:bg-brand-700',
  secondary: 'bg-white text-slate-800 ring-1 ring-inset ring-line hover:bg-slate-50',
  danger: 'bg-danger text-white hover:brightness-95',
  ghost: 'text-brand-700 hover:bg-brand-50',
};

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: Variant;
  size?: 'sm' | 'md';
  /** Shows that the action is running and prevents a second click. */
  busy?: boolean;
}

export function Button({
  variant = 'primary',
  size = 'md',
  busy = false,
  className,
  children,
  disabled,
  type = 'button',
  ...rest
}: ButtonProps) {
  return (
    <button
      type={type}
      disabled={disabled || busy}
      aria-busy={busy || undefined}
      className={cx(
        'inline-flex items-center justify-center gap-2 rounded-sm font-medium transition-colors',
        'disabled:cursor-not-allowed disabled:opacity-50',
        size === 'sm' ? 'min-h-9 px-3 text-sm' : 'min-h-11 px-4 text-sm',
        VARIANTS[variant],
        className,
      )}
      {...rest}
    >
      {busy && <Spinner small />}
      {children}
    </button>
  );
}

/** A link that looks like a primary button. */
export function ButtonLink({ to, children }: { to: string; children: ReactNode }) {
  return (
    <Link
      to={to}
      className="inline-flex min-h-11 items-center gap-2 rounded-sm bg-brand-600 px-4 text-sm font-medium text-white hover:bg-brand-700"
    >
      {children}
    </Link>
  );
}

export function Spinner({ small = false }: { small?: boolean }) {
  return (
    <span
      aria-hidden="true"
      className={cx(
        'inline-block animate-spin rounded-full border-2 border-current border-r-transparent',
        small ? 'size-4' : 'size-6',
      )}
    />
  );
}

export function Loading({ label = 'Chargement…' }: { label?: string }) {
  return (
    <p role="status" className="flex items-center gap-3 py-8 text-slate-600">
      <Spinner /> {label}
    </p>
  );
}

// --- Form fields -----------------------------------------------------------------------

const CONTROL =
  'block w-full rounded-sm border-0 bg-white px-3 py-2 text-base text-slate-900 ring-1 ' +
  'ring-inset ring-line placeholder:text-slate-400 focus:ring-2 focus:ring-brand-600 ' +
  'disabled:bg-slate-100 sm:text-sm';

interface FieldProps {
  label: string;
  hint?: string;
  error?: string | null;
}

function FieldShell({
  label,
  hint,
  error,
  id,
  children,
}: FieldProps & { id: string; children: ReactNode }) {
  return (
    <div>
      <label htmlFor={id} className="label-mono mb-1.5 block text-slate-600">
        {label}
      </label>
      {children}
      {hint && !error && (
        <p id={`${id}-hint`} className="mt-1 text-sm text-slate-600">
          {hint}
        </p>
      )}
      {error && (
        <p id={`${id}-error`} className="mt-1 text-sm text-red-700">
          {error}
        </p>
      )}
    </div>
  );
}

function describedBy(id: string, { hint, error }: FieldProps): string | undefined {
  if (error) return `${id}-error`;
  if (hint) return `${id}-hint`;
  return undefined;
}

export function TextField({
  label,
  hint,
  error,
  className,
  ...rest
}: FieldProps & InputHTMLAttributes<HTMLInputElement>) {
  const id = useId();
  return (
    <FieldShell id={id} label={label} hint={hint} error={error}>
      <input
        id={id}
        aria-invalid={error ? true : undefined}
        aria-describedby={describedBy(id, { label, hint, error })}
        className={cx(CONTROL, className)}
        {...rest}
      />
    </FieldShell>
  );
}

export function SelectField({
  label,
  hint,
  error,
  className,
  children,
  ...rest
}: FieldProps & SelectHTMLAttributes<HTMLSelectElement>) {
  const id = useId();
  return (
    <FieldShell id={id} label={label} hint={hint} error={error}>
      <select
        id={id}
        aria-invalid={error ? true : undefined}
        aria-describedby={describedBy(id, { label, hint, error })}
        className={cx(CONTROL, className)}
        {...rest}
      >
        {children}
      </select>
    </FieldShell>
  );
}

export function TextAreaField({
  label,
  hint,
  error,
  className,
  ...rest
}: FieldProps & TextareaHTMLAttributes<HTMLTextAreaElement>) {
  const id = useId();
  return (
    <FieldShell id={id} label={label} hint={hint} error={error}>
      <textarea
        id={id}
        aria-invalid={error ? true : undefined}
        aria-describedby={describedBy(id, { label, hint, error })}
        className={cx(CONTROL, className)}
        {...rest}
      />
    </FieldShell>
  );
}

// --- Feedback --------------------------------------------------------------------------

type AlertTone = 'info' | 'success' | 'warning' | 'error';

const ALERT_TONES: Record<AlertTone, string> = {
  info: 'border-brand-600 bg-brand-50 text-brand-900',
  success: 'border-ok bg-emerald-50 text-emerald-900',
  warning: 'border-warn bg-amber-50 text-amber-900',
  error: 'border-danger bg-red-50 text-red-900',
};

export function Alert({ tone = 'info', children }: { tone?: AlertTone; children: ReactNode }) {
  return (
    <div
      role={tone === 'error' ? 'alert' : 'status'}
      className={cx('border-l-4 px-4 py-3 text-sm', ALERT_TONES[tone])}
    >
      {children}
    </div>
  );
}

/** Shows the error of a query or mutation, if there is one. */
export function ErrorAlert({ error }: { error: unknown }) {
  if (!error) return null;
  return <Alert tone="error">{errorMessage(error)}</Alert>;
}

const CHIP_TONES: Record<AlertTone | 'neutral', string> = {
  info: 'bg-brand-50 text-brand-700 ring-brand-200',
  success: 'bg-emerald-50 text-emerald-800 ring-emerald-200',
  warning: 'bg-amber-50 text-amber-800 ring-amber-200',
  error: 'bg-red-50 text-red-800 ring-red-200',
  neutral: 'bg-slate-50 text-slate-700 ring-slate-200',
};

/** A status chip, in the intranet's monospace style. */
export function Badge({
  tone = 'info',
  children,
}: {
  tone?: AlertTone | 'neutral';
  children: ReactNode;
}) {
  return (
    <span
      className={cx(
        'inline-flex items-center px-2 py-0.5 font-mono text-[0.6875rem] font-semibold tracking-wide whitespace-nowrap ring-1 ring-inset',
        CHIP_TONES[tone],
      )}
    >
      {children}
    </span>
  );
}

// --- Page structure --------------------------------------------------------------------

/** The white title bar at the top of every page: condensed blue title, actions on the right. */
export function PageHeader({
  title,
  description,
  meta,
  actions,
}: {
  title: string;
  description?: string;
  /** Chips next to the title (filters, context). */
  meta?: ReactNode;
  actions?: ReactNode;
}) {
  return (
    <div className="mb-8 border-t-2 border-brand-600 bg-white ring-1 ring-line">
      <div className="flex flex-wrap items-center justify-between gap-4 px-5 py-4">
        <div className="flex flex-wrap items-center gap-x-5 gap-y-2">
          <h1 className="display text-3xl text-brand-600 uppercase">
            {title}
            <Cursor />
          </h1>
          {meta && <div className="flex flex-wrap items-center gap-2">{meta}</div>}
        </div>
        {actions && <div className="flex flex-wrap gap-2">{actions}</div>}
      </div>
      {description && (
        <p className="border-t border-line px-5 py-2.5 text-sm text-slate-600">{description}</p>
      )}
    </div>
  );
}

/** A section of a page: condensed black title with a coloured bar on its left. */
export function SectionTitle({
  children,
  tone = 'brand',
  as: Heading = 'h2',
}: {
  children: ReactNode;
  tone?: Tone;
  as?: 'h2' | 'h3';
}) {
  return (
    <div className="mb-4 flex items-center gap-2.5">
      <span aria-hidden="true" className={cx('h-6 w-1', TONE_BAR[tone])} />
      <Heading className="display text-xl text-slate-900 uppercase">{children}</Heading>
    </div>
  );
}

export function Card({ children, className }: { children: ReactNode; className?: string }) {
  // `min-w-0`: inside a grid, a card may shrink below its content (its tables scroll).
  return <div className={cx('min-w-0 bg-white p-5 ring-1 ring-line', className)}>{children}</div>;
}

/** The monospace title of a card, with its icon and, when it leads somewhere, an arrow. */
export function CardTitle({
  icon: Icon,
  tone = 'brand',
  to,
  children,
}: {
  icon?: LucideIcon;
  tone?: Tone;
  /** Makes the whole title a link, with an arrow on the right. */
  to?: string;
  children: ReactNode;
}) {
  const content = (
    <span className={cx('label-mono flex items-center gap-2', TONE_TEXT[tone])}>
      {Icon && <Icon aria-hidden="true" className="size-4 shrink-0" strokeWidth={1.75} />}
      <span>
        {children}
        <Cursor />
      </span>
    </span>
  );
  return (
    <div className="mb-4 flex items-start justify-between gap-3">
      <h2>{to ? <Link to={to}>{content}</Link> : content}</h2>
      {to && (
        <Link to={to} className="text-slate-500 hover:text-slate-900" tabIndex={-1} aria-hidden="true">
          <ArrowRight className="size-4" strokeWidth={1.75} />
        </Link>
      )}
    </div>
  );
}

/** An amount in EPC: big figure, small unit. Its text reads "12,50 EPC". */
export function Amount({ units, className }: { units: number; className?: string }) {
  return (
    <span className={className}>
      {formatAmount(units)}
      <span className="text-[0.5em] tracking-wide">{'\u00a0'}EPC</span>
    </span>
  );
}

/** A headline figure with its label, in the condensed face. */
export function Stat({
  label,
  value,
  hint,
  tone = 'brand',
  size = 'md',
}: {
  label: string;
  value: ReactNode;
  hint?: ReactNode;
  tone?: Tone;
  size?: 'md' | 'lg';
}) {
  return (
    <div>
      <p className="label-mono text-slate-500">{label}</p>
      <p className={cx('display mt-1.5', size === 'lg' ? 'text-5xl' : 'text-3xl', TONE_TEXT[tone])}>
        {value}
      </p>
      {hint && <p className="mt-1.5 font-mono text-xs text-slate-500">{hint}</p>}
    </div>
  );
}

/** A card that calls for an action: big number, title, explanation, arrow. */
export function ActionCard({
  count,
  title,
  icon: Icon,
  to,
  children,
}: {
  count: number;
  title: string;
  icon: LucideIcon;
  to: string;
  children: ReactNode;
}) {
  const tone: Tone = count > 0 ? 'danger' : 'ok';
  return (
    <Link
      to={to}
      className="group flex items-center gap-5 bg-white px-5 py-4 ring-1 ring-line hover:ring-slate-300"
    >
      <span className={cx('display w-10 shrink-0 text-4xl', TONE_TEXT[tone])}>{count}</span>
      <span className="min-w-0 flex-1">
        <span className={cx('label-mono flex items-center gap-2', TONE_TEXT[tone])}>
          <Icon aria-hidden="true" className="size-4" strokeWidth={1.75} />
          <span>
            {title}
            <Cursor />
          </span>
        </span>
        <span className="mt-1 block text-sm text-slate-600">{children}</span>
      </span>
      <ArrowRight
        aria-hidden="true"
        className="size-4 text-slate-500 group-hover:text-slate-900"
        strokeWidth={1.75}
      />
    </Link>
  );
}

export function Empty({ children }: { children: ReactNode }) {
  return (
    <p className="border border-dashed border-slate-300 bg-white/60 px-4 py-10 text-center text-sm text-slate-600">
      {children}
    </p>
  );
}

/** A table that scrolls sideways on small screens instead of breaking the page. `bare`
 *  drops the frame, for a table inside a card. */
export function Table({
  caption,
  bare = false,
  children,
}: {
  caption: string;
  bare?: boolean;
  children: ReactNode;
}) {
  return (
    // `relative`: screen-reader-only labels (absolutely positioned) stay inside the scrolling
    // box instead of widening the page.
    <div className={cx('relative overflow-x-auto bg-white', !bare && 'ring-1 ring-line')}>
      <table className="min-w-full divide-y divide-line text-left text-sm">
        <caption className="sr-only">{caption}</caption>
        {children}
      </table>
    </div>
  );
}

export function Th({ children, className }: { children?: ReactNode; className?: string }) {
  return (
    <th scope="col" className={cx('label-mono px-4 py-3 whitespace-nowrap text-slate-500', className)}>
      {children}
    </th>
  );
}

export function Td({ children, className }: { children?: ReactNode; className?: string }) {
  return <td className={cx('px-4 py-3 align-top text-slate-800', className)}>{children}</td>;
}

export function Pagination({
  offset,
  limit,
  total,
  onChange,
}: {
  offset: number;
  limit: number;
  total: number;
  onChange: (offset: number) => void;
}) {
  if (total <= limit) return null;
  const first = offset + 1;
  const last = Math.min(offset + limit, total);
  return (
    <nav aria-label="Pagination" className="mt-4 flex items-center justify-between gap-4 text-sm">
      <p className="font-mono text-xs text-slate-600">
        {first}–{last} sur {total}
      </p>
      <div className="flex gap-2">
        <Button
          variant="secondary"
          size="sm"
          disabled={offset === 0}
          onClick={() => onChange(Math.max(offset - limit, 0))}
        >
          Précédent
        </Button>
        <Button
          variant="secondary"
          size="sm"
          disabled={last >= total}
          onClick={() => onChange(offset + limit)}
        >
          Suivant
        </Button>
      </div>
    </nav>
  );
}

/** A row of tabs, underlined in blue when active. */
export function Tabs<T extends string>({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: T;
  options: { value: T; label: string; icon?: LucideIcon }[];
  onChange: (value: T) => void;
}) {
  return (
    <div
      role="tablist"
      aria-label={label}
      className="mb-6 grid grid-cols-2 bg-white ring-1 ring-line sm:flex"
    >
      {options.map(({ value: option, label: text, icon: Icon }) => {
        const active = option === value;
        return (
          <button
            key={option}
            type="button"
            role="tab"
            aria-selected={active}
            onClick={() => onChange(option)}
            className={cx(
              'flex min-h-11 flex-1 items-center justify-center gap-2 border-b-2 px-4 text-sm',
              active
                ? 'border-brand-600 font-medium text-slate-900'
                : 'border-transparent text-slate-600 hover:bg-slate-50',
            )}
          >
            {Icon && (
              <Icon
                aria-hidden="true"
                className={cx('size-4', active ? 'text-brand-600' : 'text-slate-400')}
                strokeWidth={1.75}
              />
            )}
            {text}
          </button>
        );
      })}
    </div>
  );
}

// --- Figures and charts ----------------------------------------------------------------

/** How full something is (a quota): one bar, a lighter track of the same hue. */
export function Meter({ value, max, label }: { value: number; max: number; label: string }) {
  const share = max > 0 ? Math.min(value / max, 1) : 0;
  const tone = share >= 1 ? 'bg-danger' : share >= 0.8 ? 'bg-warn' : 'bg-brand-600';
  return (
    <div
      role="meter"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={max}
      aria-valuenow={value}
      title={`${Math.round(share * 100)} %`}
      className="h-2 w-full rounded-full bg-brand-100"
    >
      <div className={cx('h-2 rounded-full', tone)} style={{ width: `${share * 100}%` }} />
    </div>
  );
}

export interface Part {
  label: string;
  value: number;
  /** A text version of the value, for the legend. */
  display: string;
  tone: Tone;
}

/** A whole split into parts: a thin bar, then a legend that names and quantifies each part. */
export function SegmentedBar({ label, parts }: { label: string; parts: Part[] }) {
  const total = parts.reduce((sum, part) => sum + part.value, 0);
  const summary = parts.map((part) => `${part.label} : ${part.display}`).join(', ');
  return (
    <div>
      <div role="img" aria-label={`${label}. ${summary}`} className="flex h-2 w-full gap-0.5">
        {total === 0 ? (
          <div className="h-2 w-full rounded-full bg-slate-100" />
        ) : (
          parts
            .filter((part) => part.value > 0)
            .map((part) => (
              <div
                key={part.label}
                title={`${part.label} : ${part.display}`}
                className={cx('h-2 rounded-full', TONE_BAR[part.tone])}
                style={{ width: `${(part.value / total) * 100}%` }}
              />
            ))
        )}
      </div>
      <Legend parts={parts} className="mt-3" />
    </div>
  );
}

function Legend({ parts, className }: { parts: Part[]; className?: string }) {
  return (
    <ul className={cx('flex flex-wrap gap-x-5 gap-y-1', className)}>
      {parts.map((part) => (
        <li key={part.label} className="label-mono flex items-center gap-1.5 text-slate-500">
          <span aria-hidden="true" className={cx('size-2', TONE_BAR[part.tone])} />
          {part.label}
        </li>
      ))}
    </ul>
  );
}

/**
 * A donut with its total in the middle and a legend listing each part with its value: the
 * values are always written out, so the colours are never the only way to read them.
 */
export function Donut({ label, total, parts }: { label: string; total: string; parts: Part[] }) {
  const sum = parts.reduce((acc, part) => acc + part.value, 0);
  const radius = 40;
  const circumference = 2 * Math.PI * radius;
  // A 2-unit gap between arcs, in the surface colour.
  const gap = parts.filter((part) => part.value > 0).length > 1 ? 2 : 0;
  let offset = 0;
  return (
    <div className="flex flex-wrap items-center gap-6">
      <svg
        viewBox="0 0 100 100"
        className="size-28 shrink-0 -rotate-90"
        role="img"
        aria-label={`${label} : ${parts.map((part) => `${part.label} ${part.display}`).join(', ')}`}
      >
        <circle cx="50" cy="50" r={radius} fill="none" stroke="#f1f1ee" strokeWidth="10" />
        {sum > 0 &&
          parts
            .filter((part) => part.value > 0)
            .map((part) => {
              const length = (part.value / sum) * circumference;
              const arc = (
                <circle
                  key={part.label}
                  cx="50"
                  cy="50"
                  r={radius}
                  fill="none"
                  strokeWidth="10"
                  style={{ stroke: TONE_VAR[part.tone] }}
                  strokeDasharray={`${Math.max(length - gap, 0)} ${circumference}`}
                  strokeDashoffset={-offset}
                >
                  <title>{`${part.label} : ${part.display}`}</title>
                </circle>
              );
              offset += length;
              return arc;
            })}
        <text
          x="50"
          y="50"
          transform="rotate(90 50 50)"
          textAnchor="middle"
          dominantBaseline="central"
          className="display fill-slate-900 text-[22px]"
        >
          {total}
        </text>
      </svg>
      <dl className="min-w-40 flex-1 space-y-2">
        {parts.map((part) => (
          <div key={part.label} className="flex items-center justify-between gap-4">
            <dt className="label-mono flex items-center gap-2 text-slate-500">
              <span aria-hidden="true" className={cx('size-2.5', TONE_BAR[part.tone])} />
              {part.label}
            </dt>
            <dd className="font-mono text-sm font-semibold text-slate-900">{part.display}</dd>
          </div>
        ))}
      </dl>
    </div>
  );
}

/** A horizontal bar for a table row: length proportional to `value / max`. */
export function RowBar({ value, max }: { value: number; max: number }) {
  const percent = max > 0 ? (value / max) * 100 : 0;
  return (
    <div aria-hidden="true" className="h-2 w-full min-w-24 bg-slate-100">
      <div className="h-2 rounded-r-full bg-brand-600" style={{ width: `${percent}%` }} />
    </div>
  );
}

// --- Dialog ----------------------------------------------------------------------------

/** A modal dialog built on the native `<dialog>`: focus trap and Escape come for free. */
export function Modal({
  open,
  title,
  onClose,
  children,
}: {
  open: boolean;
  title: string;
  onClose: () => void;
  children: ReactNode;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const titleId = useId();

  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    if (open && !dialog.open) dialog.showModal();
    if (!open && dialog.open) dialog.close();
  }, [open]);

  return (
    <dialog
      ref={ref}
      aria-labelledby={titleId}
      onClose={onClose}
      onClick={(event) => {
        // A click on the backdrop (the dialog element itself) closes it.
        if (event.target === ref.current) onClose();
      }}
      className="m-auto w-[min(92vw,34rem)] border-t-2 border-brand-600 p-0 shadow-xl"
    >
      {open && (
        <div className="p-6">
          <div className="mb-5 flex items-start justify-between gap-4">
            <h2 id={titleId} className="display text-xl text-brand-600 uppercase">
              {title}
              <Cursor />
            </h2>
            <button
              type="button"
              onClick={onClose}
              aria-label="Fermer"
              className="px-2 py-1 text-slate-500 hover:bg-slate-100"
            >
              ✕
            </button>
          </div>
          {children}
        </div>
      )}
    </dialog>
  );
}

export function Mono({ children }: { children: ReactNode }) {
  return <span className="font-mono text-xs break-all text-slate-700">{children}</span>;
}
