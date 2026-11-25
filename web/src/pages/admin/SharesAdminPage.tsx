// Admins: every validated share (and cancelling one), the ranking with the legend badge,
// and the settings of the feature.

import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Crown, ListChecks, Medal, Plus, Settings2, Trash2 } from 'lucide-react';
import { useState, type FormEvent } from 'react';
import { MentionChip } from '../../components/shares';
import {
  Alert,
  Badge,
  Button,
  Card,
  Empty,
  ErrorAlert,
  Loading,
  Modal,
  PageHeader,
  Pagination,
  Table,
  Tabs,
  Td,
  TextAreaField,
  TextField,
  Th,
} from '../../components/ui';
import { api, query } from '../../lib/api';
import { cohortLabel, formatAmount, formatDateTime, formatEpc, parseEpc } from '../../lib/format';
import { WEEKDAYS } from '../../lib/shares';
import type { ManagedShare, Paged, Ranking, ShareSettings } from '../../lib/types';
import { ShareStatusBadge } from '../staff/ValidateSharePage';

type Tab = 'shares' | 'ranking' | 'settings';
const PAGE = 20;

function CancelDialog({ share, onClose }: { share: ManagedShare | null; onClose: () => void }) {
  const queryClient = useQueryClient();
  const [reason, setReason] = useState('');
  const cancel = useMutation({
    mutationFn: () =>
      api<void>(`/api/admin/shares/${share?.id}/cancel`, { method: 'POST', body: { reason } }),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ['shares'] });
      setReason('');
      onClose();
    },
  });
  const submit = (event: FormEvent) => {
    event.preventDefault();
    cancel.mutate();
  };
  return (
    <Modal open={share !== null} title="Annuler un partage" onClose={onClose}>
      {share && (
        <form onSubmit={submit} className="space-y-4">
          <p className="text-sm text-slate-700">
            {share.student_name ?? 'Compte supprimé'} · {share.topic || 'Sans sujet'} ·{' '}
            {formatEpc(share.amount + share.bonus)}
          </p>
          <Alert tone="info">
            Le partage sort de la série, des badges et du classement de l’étudiant. Les EPC versés
            restent sur son compte : la blockchain ne peut pas les reprendre.
          </Alert>
          <TextAreaField
            label="Justification"
            hint="De 5 à 1 000 caractères. L’étudiant la voit dans ses partages."
            rows={3}
            required
            minLength={5}
            maxLength={1000}
            value={reason}
            onChange={(event) => setReason(event.target.value)}
          />
          <ErrorAlert error={cancel.error} />
          <div className="flex justify-end gap-2">
            <Button variant="secondary" onClick={onClose}>
              Retour
            </Button>
            <Button type="submit" variant="danger" busy={cancel.isPending} disabled={reason.trim().length < 5}>
              Annuler le partage
            </Button>
          </div>
        </form>
      )}
    </Modal>
  );
}

function AllShares() {
  const [offset, setOffset] = useState(0);
  const [cancelling, setCancelling] = useState<ManagedShare | null>(null);
  const shares = useQuery({
    queryKey: ['shares', 'all', offset],
    queryFn: () =>
      api<Paged<ManagedShare>>(`/api/staff/shares${query({ scope: 'all', limit: PAGE, offset })}`),
    placeholderData: keepPreviousData,
  });
  return (
    <>
      <ErrorAlert error={shares.error} />
      {shares.isPending ? (
        <Loading />
      ) : shares.data && shares.data.items.length === 0 ? (
        <Empty>Aucun partage validé pour l’instant.</Empty>
      ) : (
        shares.data && (
          <>
            <Table caption="Tous les partages validés">
              <thead>
                <tr>
                  <Th>Date</Th>
                  <Th>Étudiant</Th>
                  <Th>Encadrant</Th>
                  <Th>Sujet</Th>
                  <Th className="text-right">EPC</Th>
                  <Th>Statut</Th>
                  <Th>
                    <span className="sr-only">Actions</span>
                  </Th>
                </tr>
              </thead>
              <tbody className="divide-y divide-line">
                {shares.data.items.map((share) => (
                  <tr key={share.id}>
                    <Td className="font-mono text-xs whitespace-nowrap text-slate-600">
                      {share.claimed_at && formatDateTime(share.claimed_at)}
                    </Td>
                    <Td className="whitespace-nowrap">{share.student_name ?? 'Compte supprimé'}</Td>
                    <Td className="whitespace-nowrap">{share.staff_name ?? 'Compte supprimé'}</Td>
                    <Td>
                      <span className="flex flex-wrap items-center gap-2">
                        {share.topic || <span className="text-slate-500">Sans sujet</span>}
                        <MentionChip code={share.mention} />
                      </span>
                      {share.cancelled_at && (
                        <span className="mt-0.5 block text-xs text-slate-600">
                          Annulé le {formatDateTime(share.cancelled_at)} : {share.cancel_reason}
                        </span>
                      )}
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
                    <Td className="text-right">
                      {share.status === 'claimed' && (
                        <Button variant="ghost" size="sm" onClick={() => setCancelling(share)}>
                          Annuler
                        </Button>
                      )}
                    </Td>
                  </tr>
                ))}
              </tbody>
            </Table>
            <Pagination offset={offset} limit={PAGE} total={shares.data.total} onChange={setOffset} />
          </>
        )
      )}
      <CancelDialog share={cancelling} onClose={() => setCancelling(null)} />
    </>
  );
}

function RankingWithLegends() {
  const queryClient = useQueryClient();
  const ranking = useQuery({
    queryKey: ['shares', 'ranking'],
    queryFn: () => api<Ranking>('/api/shares/ranking'),
  });
  const legend = useMutation({
    mutationFn: ({ userId, give }: { userId: string; give: boolean }) =>
      api<void>(`/api/admin/shares/legends/${userId}`, { method: give ? 'PUT' : 'DELETE' }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['shares', 'ranking'] }),
  });
  return (
    <div className="space-y-4">
      <p className="text-sm text-slate-600">
        Les étudiants qui se sont retirés du classement n’y apparaissent pas. Le badge « Légende du
        campus » se remet ou se retire depuis cette liste.
      </p>
      <ErrorAlert error={ranking.error ?? legend.error} />
      {ranking.isPending ? (
        <Loading />
      ) : ranking.data && ranking.data.items.length === 0 ? (
        <Empty>Personne n’est encore classé.</Empty>
      ) : (
        ranking.data && (
          <Table caption="Classement des partages">
            <thead>
              <tr>
                <Th>Rang</Th>
                <Th>Étudiant</Th>
                <Th className="text-right">Partages</Th>
                <Th className="text-right">EPC gagnés</Th>
                <Th>Légende</Th>
              </tr>
            </thead>
            <tbody className="divide-y divide-line">
              {ranking.data.items.map((line) => (
                <tr key={line.user_id ?? line.name}>
                  <Td className="display text-xl text-brand-600">#{line.rank}</Td>
                  <Td>
                    <span className="font-medium">{line.name}</span>
                    {line.promo && (
                      <span className="block text-xs text-slate-600">{cohortLabel(line.promo)}</span>
                    )}
                  </Td>
                  <Td className="text-right font-mono tabular-nums">{line.shares}</Td>
                  <Td className="text-right font-mono font-semibold whitespace-nowrap tabular-nums">
                    {formatEpc(line.total)}
                  </Td>
                  <Td>
                    {line.user_id && (
                      <Button
                        variant={line.legend ? 'secondary' : 'ghost'}
                        size="sm"
                        disabled={legend.isPending}
                        aria-pressed={line.legend}
                        onClick={() => legend.mutate({ userId: line.user_id!, give: !line.legend })}
                      >
                        <Crown aria-hidden="true" className="size-4" />
                        {line.legend ? 'Retirer' : 'Remettre'}
                      </Button>
                    )}
                  </Td>
                </tr>
              ))}
            </tbody>
          </Table>
        )
      )}
    </div>
  );
}

interface TierRow {
  days: string;
  bonus: string;
}

function SettingsForm({ settings }: { settings: ShareSettings }) {
  const queryClient = useQueryClient();
  const [standard, setStandard] = useState(formatAmount(settings.standard));
  const [max, setMax] = useState(formatAmount(settings.max_amount));
  const [seconds, setSeconds] = useState(String(settings.qr_seconds));
  const [timeZone, setTimeZone] = useState(settings.time_zone);
  const [weekdays, setWeekdays] = useState<number[]>(settings.weekdays);
  const [closures, setClosures] = useState(settings.closures.join('\n'));
  const [tiers, setTiers] = useState<TierRow[]>(
    settings.tiers.map((tier) => ({ days: String(tier.days), bonus: formatAmount(tier.bonus) })),
  );
  const [dailyLimit, setDailyLimit] = useState(settings.daily_limit ? String(settings.daily_limit) : '');
  const [saved, setSaved] = useState(false);

  const save = useMutation({
    mutationFn: (body: ShareSettings) =>
      api<ShareSettings>('/api/admin/shares/settings', { method: 'PUT', body }),
    onSuccess: (fresh) => {
      queryClient.setQueryData(['shares', 'settings'], fresh);
      setSaved(true);
    },
  });
  const toggleDay = (day: number) =>
    setWeekdays((current) =>
      current.includes(day) ? current.filter((d) => d !== day) : [...current, day].sort(),
    );
  const setTier = (index: number, change: Partial<TierRow>) =>
    setTiers((current) => current.map((tier, i) => (i === index ? { ...tier, ...change } : tier)));

  const parsedTiers = tiers.map((tier) => ({ days: Number(tier.days), bonus: parseEpc(tier.bonus) }));
  const invalid =
    parseEpc(standard) === null ||
    parseEpc(max) === null ||
    !/^\d+$/.test(seconds) ||
    (dailyLimit !== '' && !/^\d+$/.test(dailyLimit)) ||
    parsedTiers.some((tier) => !Number.isInteger(tier.days) || tier.bonus === null);

  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (invalid) return;
    setSaved(false);
    save.mutate({
      standard: parseEpc(standard) ?? 0,
      max_amount: parseEpc(max) ?? 0,
      qr_seconds: Number(seconds),
      time_zone: timeZone,
      weekdays,
      closures: closures
        .split(/[\s,;]+/)
        .map((day) => day.trim())
        .filter(Boolean),
      tiers: parsedTiers.map((tier) => ({ days: tier.days, bonus: tier.bonus ?? 0 })),
      daily_limit: dailyLimit === '' ? null : Number(dailyLimit),
    });
  };

  return (
    <form onSubmit={submit} className="space-y-6">
      <Card>
        <h2 className="label-mono mb-4 text-slate-600">Montants et QR codes</h2>
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
          <TextField
            label="Montant conseillé (EPC)"
            inputMode="decimal"
            value={standard}
            onChange={(event) => setStandard(event.target.value)}
          />
          <TextField
            label="Montant maximal (EPC)"
            inputMode="decimal"
            value={max}
            onChange={(event) => setMax(event.target.value)}
          />
          <TextField
            label="Validité d’un QR code (s)"
            inputMode="numeric"
            hint="De 15 à 600 secondes."
            value={seconds}
            onChange={(event) => setSeconds(event.target.value)}
          />
          <TextField
            label="Partages par jour et par étudiant"
            inputMode="numeric"
            hint="Vide : pas de limite."
            value={dailyLimit}
            onChange={(event) => setDailyLimit(event.target.value)}
          />
        </div>
      </Card>

      <Card>
        <h2 className="label-mono mb-4 text-slate-600">Calendrier du campus</h2>
        <div className="grid gap-5 lg:grid-cols-2">
          <div className="space-y-4">
            <fieldset>
              <legend className="label-mono mb-2 text-slate-600">Jours ouvrés</legend>
              <div className="flex flex-wrap gap-x-4 gap-y-2">
                {WEEKDAYS.map((day) => (
                  <label key={day.value} className="flex items-center gap-2 text-sm">
                    <input
                      type="checkbox"
                      className="size-4"
                      checked={weekdays.includes(day.value)}
                      onChange={() => toggleDay(day.value)}
                    />
                    {day.label}
                  </label>
                ))}
              </div>
            </fieldset>
            <TextField
              label="Fuseau horaire"
              hint="Décide du jour d’un partage. Exemple : Europe/Paris."
              value={timeZone}
              onChange={(event) => setTimeZone(event.target.value)}
            />
          </div>
          <TextAreaField
            label="Jours de fermeture"
            hint="Une date par ligne, au format AAAA-MM-JJ. Ils ne cassent pas les séries."
            rows={6}
            value={closures}
            onChange={(event) => setClosures(event.target.value)}
          />
        </div>
      </Card>

      <Card>
        <h2 className="label-mono mb-1 text-slate-600">Paliers de régularité</h2>
        <p className="mb-4 text-sm text-slate-600">
          Atteindre ce nombre de jours ouvrés de partage d’affilée verse le bonus, une fois par
          série, en plus du partage du jour et sur le quota de l’encadrant qui le valide. Un
          changement s’applique aux prochains scans, y compris dans une série déjà commencée.
        </p>
        <ul className="space-y-3">
          {tiers.map((tier, index) => (
            <li key={index} className="flex flex-wrap items-end gap-3">
              <div className="w-32">
                <TextField
                  label="Jours"
                  inputMode="numeric"
                  value={tier.days}
                  onChange={(event) => setTier(index, { days: event.target.value })}
                />
              </div>
              <div className="w-40">
                <TextField
                  label="Bonus (EPC)"
                  inputMode="decimal"
                  value={tier.bonus}
                  onChange={(event) => setTier(index, { bonus: event.target.value })}
                />
              </div>
              <Button
                variant="ghost"
                size="sm"
                aria-label={`Supprimer le palier ${index + 1}`}
                onClick={() => setTiers((current) => current.filter((_, i) => i !== index))}
              >
                <Trash2 aria-hidden="true" className="size-4" />
              </Button>
            </li>
          ))}
        </ul>
        {tiers.length < 10 && (
          <Button
            variant="secondary"
            size="sm"
            className="mt-4"
            onClick={() => setTiers((current) => [...current, { days: '', bonus: '' }])}
          >
            <Plus aria-hidden="true" className="size-4" />
            Ajouter un palier
          </Button>
        )}
      </Card>

      <ErrorAlert error={save.error} />
      {saved && <Alert tone="success">Réglages enregistrés.</Alert>}
      <div className="flex justify-end">
        <Button type="submit" busy={save.isPending} disabled={invalid}>
          Enregistrer les réglages
        </Button>
      </div>
    </form>
  );
}

function SettingsTab() {
  const settings = useQuery({
    queryKey: ['shares', 'settings'],
    queryFn: () => api<ShareSettings>('/api/shares/settings'),
  });
  if (settings.isPending) return <Loading />;
  if (settings.isError) return <ErrorAlert error={settings.error} />;
  return <SettingsForm settings={settings.data} />;
}

export function SharesAdminPage() {
  const [tab, setTab] = useState<Tab>('shares');
  return (
    <>
      <PageHeader
        title="Partages"
        meta={<Badge tone="neutral">Apprends-nous quelque chose !</Badge>}
        description="Les partages de connaissances validés par QR code : annulation, badge « Légende du campus » et réglages."
      />
      <Tabs
        label="Partages"
        value={tab}
        onChange={setTab}
        options={[
          { value: 'shares', label: 'Validations', icon: ListChecks },
          { value: 'ranking', label: 'Classement', icon: Medal },
          { value: 'settings', label: 'Réglages', icon: Settings2 },
        ]}
      />
      {tab === 'shares' && <AllShares />}
      {tab === 'ranking' && <RankingWithLegends />}
      {tab === 'settings' && <SettingsTab />}
    </>
  );
}
