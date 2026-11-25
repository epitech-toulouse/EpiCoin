// Staff and admins: validating a knowledge share. Generate a QR code, the student scans it
// and is paid at once, from the staff member's quota.

import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { CircleCheck, Copy, QrCode as QrIcon, RefreshCw, ScanLine } from 'lucide-react';
import { useState, type FormEvent } from 'react';
import { useAuth } from '../../auth';
import { MentionChip, QrCode, useSecondsLeft } from '../../components/shares';
import {
  Alert,
  Badge,
  Button,
  Card,
  CardTitle,
  Empty,
  ErrorAlert,
  Loading,
  PageHeader,
  Pagination,
  SectionTitle,
  Table,
  Td,
  TextField,
  Th,
  cx,
} from '../../components/ui';
import { api, query } from '../../lib/api';
import { formatAmount, formatDateTime, formatEpc, formatSeconds, parseEpc } from '../../lib/format';
import { MENTIONS } from '../../lib/shares';
import type { ManagedShare, NewShare, Paged, Quota, ShareSettings } from '../../lib/types';

const PAGE = 20;
const MAX_TOPIC = 300;

/** The status chip of a scanned share. */
export function ShareStatusBadge({ share }: { share: ManagedShare }) {
  if (share.status === 'cancelled') return <Badge tone="neutral">Annulé</Badge>;
  if (share.reward_status === 'pending') return <Badge tone="warning">Versement en cours</Badge>;
  if (share.status === 'claimed') return <Badge tone="success">Versé</Badge>;
  return <Badge tone="neutral">{share.status}</Badge>;
}

function MentionPicker({
  value,
  onChange,
  admin,
}: {
  value: string;
  onChange: (code: string) => void;
  admin: boolean;
}) {
  const options = [
    { code: '', label: 'Aucune', description: 'Partage classique' },
    ...MENTIONS.filter((mention) => admin || !mention.adminOnly),
  ];
  return (
    <fieldset>
      <legend className="label-mono mb-1.5 text-slate-600">
        Mention spéciale · facultative, sans EPC en plus
      </legend>
      <div className="grid grid-cols-2 gap-2 sm:grid-cols-3">
        {options.map((option) => (
          <label
            key={option.code || 'none'}
            className={cx(
              'flex cursor-pointer flex-col gap-0.5 p-2.5 text-sm ring-1 ring-inset',
              // The radio itself is hidden: show keyboard focus on the whole card.
              'has-[:focus-visible]:outline-2 has-[:focus-visible]:outline-offset-2 has-[:focus-visible]:outline-brand-600',
              value === option.code
                ? 'bg-brand-50 ring-2 ring-brand-600'
                : 'bg-white ring-line hover:bg-slate-50',
            )}
          >
            <input
              type="radio"
              name="mention"
              className="sr-only"
              checked={value === option.code}
              onChange={() => onChange(option.code)}
            />
            <span className="font-medium text-slate-900">{option.label}</span>
            <span className="text-xs text-slate-600">{option.description}</span>
          </label>
        ))}
      </div>
    </fieldset>
  );
}

function CurrentQr({
  share,
  deadline,
  onAgain,
  again,
}: {
  share: NewShare;
  deadline: number;
  onAgain: () => void;
  again: boolean;
}) {
  const queryClient = useQueryClient();
  const [copied, setCopied] = useState(false);
  const secondsLeft = useSecondsLeft(deadline);
  const status = useQuery({
    queryKey: ['shares', 'status', share.id],
    queryFn: () => api<ManagedShare>(`/api/staff/shares/${share.id}`),
    refetchInterval: (current) =>
      current.state.data === undefined || current.state.data.status === 'open' ? 1_500 : false,
  });
  const withdraw = useMutation({
    mutationFn: () => api<void>(`/api/staff/shares/${share.id}/invalidate`, { method: 'POST' }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['shares', 'status', share.id] }),
  });
  const current = status.data;
  const claimed = current?.status === 'claimed';
  const open = !current || current.status === 'open';
  const expired = open && secondsLeft === 0;

  if (claimed) {
    return (
      <div className="flex flex-col items-center gap-3 py-6 text-center" role="status">
        <CircleCheck aria-hidden="true" className="size-12 text-emerald-700" strokeWidth={1.5} />
        <p className="display text-2xl text-slate-900">
          {current.student_name ?? 'L’étudiant'} a reçu {formatEpc(current.amount + current.bonus)}
        </p>
        {current.bonus > 0 && (
          <p className="text-sm text-slate-600">
            dont {formatEpc(current.bonus)} de bonus pour {current.streak_days} jours de partage
            d’affilée, compté sur votre quota.
          </p>
        )}
        <Button onClick={onAgain} busy={again}>
          <RefreshCw aria-hidden="true" className="size-4" />
          Même partage, nouveau QR code
        </Button>
      </div>
    );
  }
  if (!open || expired) {
    return (
      <div className="flex flex-col items-center gap-3 py-6 text-center">
        <p className="text-slate-700">
          {current?.status === 'invalidated'
            ? 'Ce QR code a été retiré.'
            : 'Ce QR code a expiré sans être scanné.'}
        </p>
        <Button onClick={onAgain} busy={again}>
          <RefreshCw aria-hidden="true" className="size-4" />
          Générer à nouveau
        </Button>
      </div>
    );
  }
  return (
    <div className="flex flex-col items-center gap-4">
      <QrCode
        size={share.qr.size}
        modules={share.qr.modules}
        label={`QR code du partage de ${formatEpc(share.amount)}`}
      />
      <p className="display text-3xl text-brand-600">{formatEpc(share.amount)}</p>
      <MentionChip code={share.mention} />
      <p className="font-mono text-sm text-slate-700" aria-live="polite">
        En attente du scan · encore {formatSeconds(secondsLeft)}
      </p>
      <ErrorAlert error={withdraw.error} />
      <div className="flex flex-wrap justify-center gap-2">
        <Button
          variant="secondary"
          size="sm"
          onClick={() => {
            void navigator.clipboard?.writeText(share.link).then(() => setCopied(true));
          }}
        >
          <Copy aria-hidden="true" className="size-4" />
          {copied ? 'Lien copié' : 'Copier le lien'}
        </Button>
        <Button variant="ghost" size="sm" busy={withdraw.isPending} onClick={() => withdraw.mutate()}>
          Retirer ce QR code
        </Button>
      </div>
      <p className="text-center text-xs text-slate-600">
        Pas d’appareil photo ? Envoyez le lien à l’étudiant ; il l’ouvre une fois connecté.
      </p>
    </div>
  );
}

function History() {
  const [offset, setOffset] = useState(0);
  const history = useQuery({
    queryKey: ['shares', 'history', offset],
    queryFn: () => api<Paged<ManagedShare>>(`/api/staff/shares${query({ limit: PAGE, offset })}`),
    placeholderData: keepPreviousData,
  });
  return (
    <section className="mt-10">
      <SectionTitle>Mes validations</SectionTitle>
      <ErrorAlert error={history.error} />
      {history.isPending ? (
        <Loading />
      ) : history.data && history.data.items.length === 0 ? (
        <Empty>Aucun partage validé pour l’instant.</Empty>
      ) : (
        history.data && (
          <>
            <Table caption="Mes validations">
              <thead>
                <tr>
                  <Th>Date</Th>
                  <Th>Étudiant</Th>
                  <Th>Sujet</Th>
                  <Th className="text-right">EPC</Th>
                  <Th>Statut</Th>
                </tr>
              </thead>
              <tbody className="divide-y divide-line">
                {history.data.items.map((share) => (
                  <tr key={share.id}>
                    <Td className="font-mono text-xs whitespace-nowrap text-slate-600">
                      {share.claimed_at && formatDateTime(share.claimed_at)}
                    </Td>
                    <Td className="whitespace-nowrap">{share.student_name ?? 'Compte supprimé'}</Td>
                    <Td>
                      <span className="flex flex-wrap items-center gap-2">
                        {share.topic || <span className="text-slate-500">Sans sujet</span>}
                        <MentionChip code={share.mention} />
                      </span>
                    </Td>
                    <Td className="text-right font-mono whitespace-nowrap tabular-nums">
                      {formatEpc(share.amount)}
                      {share.bonus > 0 && (
                        <span className="block text-xs text-slate-600">+ {formatEpc(share.bonus)} bonus</span>
                      )}
                    </Td>
                    <Td>
                      <ShareStatusBadge share={share} />
                    </Td>
                  </tr>
                ))}
              </tbody>
            </Table>
            <Pagination offset={offset} limit={PAGE} total={history.data.total} onChange={setOffset} />
          </>
        )
      )}
    </section>
  );
}

export function ValidateSharePage() {
  const { user } = useAuth();
  const queryClient = useQueryClient();
  const settings = useQuery({
    queryKey: ['shares', 'settings'],
    queryFn: () => api<ShareSettings>('/api/shares/settings'),
  });
  const quota = useQuery({ queryKey: ['quota'], queryFn: () => api<Quota>('/api/staff/quota') });
  // `null` until the settings arrive, then the suggested amount until the user types.
  const [amountText, setAmountText] = useState<string | null>(null);
  const [mention, setMention] = useState('');
  const [topic, setTopic] = useState('');
  const [current, setCurrent] = useState<{ share: NewShare; deadline: number } | null>(null);

  const shownAmount = amountText ?? (settings.data ? formatAmount(settings.data.standard) : '');
  const amount = parseEpc(shownAmount);
  const max = settings.data?.max_amount ?? 0;
  const amountError =
    shownAmount !== '' && (amount === null || amount === 0 || amount > max)
      ? `Un montant de 0,01 à ${formatEpc(max)}.`
      : null;

  const generate = useMutation({
    mutationFn: (body: { amount: number; mention: string | null; topic: string }) =>
      api<NewShare>('/api/staff/shares', { method: 'POST', body }),
    onSuccess: (share) => {
      setCurrent({ share, deadline: Date.now() + share.expires_in * 1000 });
      void queryClient.invalidateQueries({ queryKey: ['shares', 'history'] });
      void queryClient.invalidateQueries({ queryKey: ['quota'] });
    },
  });
  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (amount === null || amountError) return;
    generate.mutate({ amount, mention: mention || null, topic });
  };
  const again = () => {
    if (!current) return;
    generate.mutate({
      amount: current.share.amount,
      mention: current.share.mention,
      topic: current.share.topic,
    });
  };

  return (
    <>
      <PageHeader
        title="Valider un partage"
        description="Un étudiant vous a appris quelque chose ? Générez un QR code : il le scanne avec son téléphone et reçoit ses EPC aussitôt, prélevés sur votre quota."
      />
      {quota.data && !quota.data.active && (
        <div className="mb-6">
          <Alert tone="warning">
            Votre autorisation de distribution n’est pas encore active sur la blockchain : vous ne
            pouvez pas encore valider de partage.
          </Alert>
        </div>
      )}
      <div className="grid gap-5 lg:grid-cols-2">
        <Card>
          <CardTitle icon={ScanLine}>Le partage</CardTitle>
          {settings.isPending ? (
            <Loading />
          ) : settings.isError ? (
            <ErrorAlert error={settings.error} />
          ) : (
            <form onSubmit={submit} className="space-y-5">
              <TextField
                label={`Montant en EPC · conseillé ${formatEpc(settings.data.standard)}`}
                inputMode="decimal"
                required
                value={shownAmount}
                error={amountError}
                hint={
                  quota.data?.active
                    ? `Quota restant sur la période : ${formatEpc(quota.data.remaining)}.`
                    : undefined
                }
                onChange={(event) => setAmountText(event.target.value)}
              />
              <MentionPicker value={mention} onChange={setMention} admin={user?.role === 'admin'} />
              <TextField
                label="Sujet partagé · facultatif"
                placeholder="De quoi avez-vous parlé ?"
                maxLength={MAX_TOPIC}
                value={topic}
                hint="L’étudiant le retrouve dans son historique. Il ne va jamais sur la blockchain."
                onChange={(event) => setTopic(event.target.value)}
              />
              <ErrorAlert error={generate.error} />
              <Button type="submit" className="w-full" busy={generate.isPending} disabled={amount === null || amountError !== null}>
                <QrIcon aria-hidden="true" className="size-4" />
                Générer le QR code
              </Button>
              <p className="text-sm text-slate-600">
                Le QR code reste valable {formatSeconds(settings.data.qr_seconds)} et remplace le
                précédent. Si ce partage complète une série de l’étudiant, le bonus de régularité
                s’ajoute au montant et compte aussi sur votre quota.
              </p>
            </form>
          )}
        </Card>
        <Card>
          <CardTitle icon={QrIcon} tone="teal">
            À faire scanner
          </CardTitle>
          {current ? (
            <CurrentQr
              key={current.share.id}
              share={current.share}
              deadline={current.deadline}
              onAgain={again}
              again={generate.isPending}
            />
          ) : (
            <div className="flex flex-col items-center gap-2 py-12 text-center text-slate-600">
              <ScanLine aria-hidden="true" className="size-8 text-slate-400" strokeWidth={1.5} />
              <p className="font-medium text-slate-800">Le prochain partage commence ici</p>
              <p className="text-sm">Choisissez un montant puis générez le QR code à présenter à l’étudiant.</p>
            </div>
          )}
        </Card>
      </div>
      <History />
    </>
  );
}
