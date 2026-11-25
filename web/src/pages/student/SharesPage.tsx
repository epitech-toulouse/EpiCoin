// Students: knowledge shares. Their streak, badges, shares, and the ranking.

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Award, Flame, Lightbulb, ListChecks, Medal, Trophy } from 'lucide-react';
import { useState } from 'react';
import { BadgeTile, MentionChip } from '../../components/shares';
import {
  Alert,
  Amount,
  Badge,
  Card,
  CardTitle,
  Empty,
  ErrorAlert,
  Loading,
  Meter,
  PageHeader,
  Stat,
  Table,
  Tabs,
  Td,
  Th,
  cx,
} from '../../components/ui';
import { api } from '../../lib/api';
import { cohortLabel, formatDateTime, formatEpc } from '../../lib/format';
import { BADGES } from '../../lib/shares';
import type { MyShares, Ranking, ShareSettings } from '../../lib/types';

type Tab = 'shares' | 'badges' | 'ranking';

function StreakCard({ data, settings }: { data: MyShares; settings: ShareSettings | undefined }) {
  const { current, best, next, today_counted } = data.progress;
  return (
    <Card>
      <CardTitle icon={Flame} tone="accent">
        Garde le rythme
      </CardTitle>
      <Stat
        label="Série en cours"
        value={`${current} jour${current > 1 ? 's' : ''}`}
        tone="accent"
        size="lg"
        hint={`Meilleure série : ${best} jour${best > 1 ? 's' : ''} ouvré${best > 1 ? 's' : ''}`}
      />
      {next ? (
        <div className="mt-4 space-y-1.5">
          <Meter
            value={current}
            max={next.days}
            tone="accent"
            label={`Série : ${current} jours sur ${next.days}`}
          />
          <p className="flex justify-between gap-3 font-mono text-xs text-slate-600">
            <span>Prochain palier : {next.days} jours</span>
            <span className="font-semibold text-emerald-700">+{formatEpc(next.bonus)}</span>
          </p>
        </div>
      ) : (
        settings &&
        settings.tiers.length > 0 && (
          <p className="mt-4 text-sm text-slate-600">Tous les paliers sont atteints. Bravo !</p>
        )
      )}
      <p className="mt-4 text-sm text-slate-600">
        {today_counted
          ? 'Aujourd’hui compte déjà pour votre série.'
          : current > 0
            ? 'Un partage aujourd’hui prolonge votre série.'
            : 'Un partage aujourd’hui commence une nouvelle série.'}{' '}
        Les week-ends et les fermetures du campus ne la cassent pas.
      </p>
    </Card>
  );
}

function MySharesTable({ data }: { data: MyShares }) {
  if (data.recent.length === 0) {
    return (
      <Empty>
        Pas encore de partage. Une astuce, une anecdote, une découverte ? Allez la raconter à un
        encadrant : il vous montrera un QR code à scanner.
      </Empty>
    );
  }
  return (
    <Table caption="Mes partages">
      <thead>
        <tr>
          <Th className="hidden sm:table-cell">Date</Th>
          <Th>Partage</Th>
          <Th className="text-right">EPC</Th>
        </tr>
      </thead>
      <tbody className="divide-y divide-line">
        {data.recent.map((share) => (
          <tr key={share.id} className={share.cancelled_at ? 'bg-slate-50' : undefined}>
            <Td className="hidden font-mono text-xs whitespace-nowrap text-slate-600 sm:table-cell">
              {formatDateTime(share.claimed_at)}
            </Td>
            <Td>
              {/* On a phone, the date moves here so that the amount stays on screen. */}
              <span className="mb-1 block font-mono text-xs text-slate-600 sm:hidden">
                {formatDateTime(share.claimed_at)}
              </span>
              <span className="flex flex-wrap items-center gap-2">
                <span className={cx('font-medium', share.cancelled_at && 'line-through')}>
                  {share.topic || 'Partage de connaissance'}
                </span>
                <MentionChip code={share.mention} />
                {share.reward_status === 'pending' && <Badge tone="warning">Versement en cours</Badge>}
                {share.cancelled_at && <Badge tone="neutral">Annulé</Badge>}
              </span>
              {share.staff_name && (
                <span className="mt-0.5 block text-slate-600">avec {share.staff_name}</span>
              )}
              {share.cancelled_at && (
                <span className="mt-0.5 block text-sm text-slate-600">
                  Annulé par l’administration{share.cancel_reason && ` : ${share.cancel_reason}`}.
                  Il ne compte plus pour la série, les badges ni le classement ; les EPC restent
                  acquis.
                </span>
              )}
            </Td>
            <Td className="text-right font-mono font-semibold whitespace-nowrap text-emerald-700 tabular-nums">
              +{formatEpc(share.amount)}
              {share.bonus > 0 && (
                <span className="block text-xs font-normal text-slate-600">
                  + {formatEpc(share.bonus)} de bonus ({share.streak_days} jours)
                </span>
              )}
            </Td>
          </tr>
        ))}
      </tbody>
    </Table>
  );
}

function Badges({ data }: { data: MyShares }) {
  const earned = new Map(data.badges.map((badge) => [badge.code, badge]));
  const regular = BADGES.filter((badge) => badge.kind === 'badge');
  const mentions = BADGES.filter((badge) => badge.kind === 'mention');
  return (
    <div className="space-y-6">
      <p className="font-mono text-xs text-slate-600">
        {data.badges.length} sur {BADGES.length} obtenus · les badges et mentions ne rapportent pas
        d’EPC
      </p>
      <ul className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3" aria-label="Badges">
        {regular.map((badge) => (
          <BadgeTile key={badge.code} badge={badge} earned={earned.get(badge.code)} />
        ))}
      </ul>
      <div>
        <h3 className="label-mono mb-3 text-slate-600">Mentions des encadrants</h3>
        <ul className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3" aria-label="Mentions">
          {mentions.map((badge) => (
            <BadgeTile key={badge.code} badge={badge} earned={earned.get(badge.code)} />
          ))}
        </ul>
      </div>
    </div>
  );
}

function RankingTab({ data }: { data: MyShares }) {
  const queryClient = useQueryClient();
  const ranking = useQuery({
    queryKey: ['shares', 'ranking'],
    queryFn: () => api<Ranking>('/api/shares/ranking'),
  });
  const visibility = useMutation({
    mutationFn: (hidden: boolean) =>
      api<{ hidden_from_ranking: boolean }>('/api/shares/me', {
        method: 'PATCH',
        body: { hidden_from_ranking: hidden },
      }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['shares'] }),
  });
  return (
    <div className="space-y-4">
      <Card>
        <label className="flex items-start gap-3 text-sm">
          <input
            type="checkbox"
            className="mt-0.5 size-4"
            checked={!data.hidden_from_ranking}
            disabled={visibility.isPending}
            onChange={(event) => visibility.mutate(!event.target.checked)}
          />
          <span>
            <span className="font-medium text-slate-900">Apparaître dans le classement</span>
            <span className="block text-slate-600">
              Les autres voient votre prénom, l’initiale de votre nom, votre cohorte et les EPC
              gagnés en partageant. Décochez pour vous retirer à tout moment.
            </span>
          </span>
        </label>
        <ErrorAlert error={visibility.error} />
      </Card>
      <ErrorAlert error={ranking.error} />
      {ranking.isPending ? (
        <Loading />
      ) : ranking.data && ranking.data.items.length === 0 ? (
        <Empty>Personne n’est encore classé. Le premier partage ouvre le bal !</Empty>
      ) : (
        ranking.data && (
          <Table caption="Classement des partages">
            <thead>
              <tr>
                <Th>Rang</Th>
                <Th>Étudiant</Th>
                <Th className="text-right">Partages</Th>
                <Th className="text-right">EPC gagnés</Th>
              </tr>
            </thead>
            <tbody className="divide-y divide-line">
              {ranking.data.items.map((line) => (
                <tr
                  key={`${line.rank}-${line.name}-${line.promo}`}
                  className={line.me ? 'bg-brand-50' : undefined}
                  aria-current={line.me ? 'true' : undefined}
                >
                  <Td className="display text-xl text-brand-600">#{line.rank}</Td>
                  <Td>
                    <span className="flex flex-wrap items-center gap-2">
                      <span className="font-medium">{line.name}</span>
                      {line.me && <Badge>Vous</Badge>}
                      {line.legend && <Badge tone="warning">Légende</Badge>}
                    </span>
                    {line.promo && (
                      <span className="block text-xs text-slate-600">{cohortLabel(line.promo)}</span>
                    )}
                  </Td>
                  <Td className="text-right font-mono tabular-nums">{line.shares}</Td>
                  <Td className="text-right font-mono font-semibold whitespace-nowrap tabular-nums">
                    {formatEpc(line.total)}
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

export function SharesPage() {
  const [tab, setTab] = useState<Tab>('shares');
  const mine = useQuery({ queryKey: ['shares', 'me'], queryFn: () => api<MyShares>('/api/shares/me') });
  const settings = useQuery({
    queryKey: ['shares', 'settings'],
    queryFn: () => api<ShareSettings>('/api/shares/settings'),
  });

  return (
    <>
      <PageHeader
        title="Partages"
        description="Apprends-nous quelque chose ! Une astuce, une anecdote, une découverte : racontez-la à un encadrant. Il vous affiche un QR code ; scannez-le avec votre téléphone et les EPC arrivent aussitôt."
      />
      <ErrorAlert error={mine.error} />
      {mine.isPending ? (
        <Loading />
      ) : (
        mine.data && (
          <>
            <section className="mb-8 grid gap-5 lg:grid-cols-3">
              <StreakCard data={mine.data} settings={settings.data} />
              <Card>
                <CardTitle icon={Lightbulb} tone="ok">
                  Gagnés en partageant
                </CardTitle>
                <Stat
                  label="Depuis le début"
                  value={<Amount units={mine.data.total} />}
                  tone="ok"
                  size="lg"
                  hint={`${mine.data.shares} partage${mine.data.shares > 1 ? 's' : ''} validé${mine.data.shares > 1 ? 's' : ''}, bonus compris`}
                />
                {settings.data && settings.data.tiers.length > 0 && (
                  <p className="mt-4 text-sm text-slate-600">
                    Bonus de régularité :{' '}
                    {settings.data.tiers
                      .map((tier) => `+${formatEpc(tier.bonus)} à ${tier.days} jours`)
                      .join(', ')}
                    .
                  </p>
                )}
              </Card>
              <Card>
                <CardTitle icon={Trophy} tone="brand">
                  Classement
                </CardTitle>
                <Stat
                  label="Votre rang"
                  value={mine.data.rank ? `#${mine.data.rank}` : '—'}
                  size="lg"
                  hint={
                    mine.data.hidden_from_ranking
                      ? 'Vous êtes retiré du classement'
                      : mine.data.rank
                        ? 'parmi les étudiants qui partagent'
                        : 'classé dès votre premier partage'
                  }
                />
              </Card>
            </section>

            {mine.data.hidden_from_ranking && tab === 'ranking' && (
              <div className="mb-4">
                <Alert tone="info">Vous n’apparaissez pas dans le classement.</Alert>
              </div>
            )}
            <Tabs
              label="Partages"
              value={tab}
              onChange={setTab}
              options={[
                { value: 'shares', label: 'Mes partages', icon: ListChecks },
                { value: 'badges', label: `Badges (${mine.data.badges.length})`, icon: Award },
                { value: 'ranking', label: 'Classement', icon: Medal },
              ]}
            />
            {tab === 'shares' && <MySharesTable data={mine.data} />}
            {tab === 'badges' && <Badges data={mine.data} />}
            {tab === 'ranking' && <RankingTab data={mine.data} />}
          </>
        )
      )}
    </>
  );
}
