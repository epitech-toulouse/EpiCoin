// The student's home: balance, current orders, and everything that moved the balance.

import { keepPreviousData, useQuery } from '@tanstack/react-query';
import { ArrowRight, History, ShoppingBag, Wallet } from 'lucide-react';
import { useState } from 'react';
import { useAuth } from '../../auth';
import {
  Amount,
  Badge,
  ButtonLink,
  Card,
  CardTitle,
  Empty,
  ErrorAlert,
  Loading,
  PageHeader,
  Pagination,
  SectionTitle,
  Stat,
  Table,
  Td,
  Th,
} from '../../components/ui';
import { api, query } from '../../lib/api';
import { cohortLabel, formatDateTime, formatEpc, reasonLabel } from '../../lib/format';
import type { HistoryLine, Order, Paged, Wallet as WalletView } from '../../lib/types';

const PAGE = 20;

function describe(line: HistoryLine): { title: string; detail: string | null } {
  switch (line.kind) {
    case 'mint':
      return {
        title: reasonLabel(line.reason_code),
        detail:
          [line.from_name && `par ${line.from_name}`, line.comment].filter(Boolean).join(' — ') ||
          null,
      };
    case 'purchase':
      return { title: 'Achat en boutique', detail: null };
    case 'refund':
      return { title: 'Remboursement', detail: 'Commande annulée' };
  }
}

export function HomePage() {
  const { user } = useAuth();
  const [offset, setOffset] = useState(0);
  const wallet = useQuery({
    queryKey: ['wallet'],
    queryFn: () => api<WalletView>('/api/me/wallet'),
  });
  const orders = useQuery({
    queryKey: ['orders'],
    queryFn: () => api<Paged<Order>>('/api/shop/orders?limit=100'),
  });
  const history = useQuery({
    queryKey: ['history', offset],
    queryFn: () =>
      api<Paged<HistoryLine>>(`/api/me/transactions${query({ limit: PAGE, offset })}`),
    placeholderData: keepPreviousData,
  });

  const open = (orders.data?.items ?? []).filter((order) =>
    ['awaiting_payment', 'pending', 'ready', 'refunding'].includes(order.status),
  );
  const ready = open.filter((order) => order.status === 'ready').length;

  return (
    <>
      <PageHeader
        title={`Bonjour ${user?.first_name ?? ''}`}
        meta={user?.promo ? <Badge>{cohortLabel(user.promo)}</Badge> : undefined}
      />

      <section className="mb-10">
        <SectionTitle>Aperçu</SectionTitle>
        <div className="grid gap-5 lg:grid-cols-3">
          <Card className="lg:col-span-2">
            <CardTitle icon={Wallet}>Mon solde</CardTitle>
            {wallet.isPending ? (
              <Loading label="Lecture du solde…" />
            ) : wallet.isError ? (
              <ErrorAlert error={wallet.error} />
            ) : (
              <div className="flex flex-wrap items-end justify-between gap-5">
                <div>
                  <p className="display text-6xl text-brand-600" data-testid="balance">
                    <Amount units={wallet.data.balance} />
                  </p>
                  <p className="mt-2 font-mono text-xs text-slate-500">
                    Solde confirmé sur la blockchain EpiCoin
                  </p>
                </div>
                <ButtonLink to="/boutique">
                  Aller à la boutique
                  <ArrowRight aria-hidden="true" className="size-4" />
                </ButtonLink>
              </div>
            )}
          </Card>

          <Card>
            <CardTitle icon={ShoppingBag} tone="accent" to="/commandes">
              Commandes en cours
            </CardTitle>
            <Stat
              label="En cours"
              value={orders.isPending ? '…' : open.length}
              tone="accent"
              size="lg"
              hint={
                ready > 0
                  ? `dont ${ready} prête(s) à retirer à la boutique`
                  : open.length > 0
                    ? 'en préparation'
                    : 'aucune commande en cours'
              }
            />
          </Card>
        </div>
      </section>

      <section>
        <SectionTitle>Historique</SectionTitle>
        <ErrorAlert error={history.error} />
        {history.isPending ? (
          <Loading />
        ) : history.data && history.data.items.length === 0 ? (
          <Empty>
            Aucun mouvement pour l'instant. Vos EPC apparaîtront ici dès qu'un encadrant vous en
            attribuera.
          </Empty>
        ) : (
          history.data && (
            <>
              <Table caption="Historique de mes EPC">
                <thead>
                  <tr>
                    <Th>Date</Th>
                    <Th>Motif</Th>
                    <Th className="text-right">Montant</Th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-line">
                  {history.data.items.map((line) => {
                    const { title, detail } = describe(line);
                    const credit = line.direction === 'credit';
                    return (
                      <tr key={line.tx_hash}>
                        <Td className="font-mono text-xs whitespace-nowrap text-slate-600">
                          {formatDateTime(line.created_at)}
                        </Td>
                        <Td>
                          <span className="flex flex-wrap items-center gap-2">
                            <History
                              aria-hidden="true"
                              className={`size-4 ${credit ? 'text-emerald-700' : 'text-slate-400'}`}
                              strokeWidth={1.75}
                            />
                            <span className="font-medium">{title}</span>
                            {line.status === 'pending' && <Badge tone="warning">En cours</Badge>}
                          </span>
                          {detail && <span className="mt-0.5 block text-slate-600">{detail}</span>}
                        </Td>
                        <Td
                          className={`text-right font-mono font-semibold whitespace-nowrap tabular-nums ${
                            credit ? 'text-emerald-700' : 'text-slate-900'
                          }`}
                        >
                          {credit ? '+' : '−'}
                          {formatEpc(line.amount ?? 0)}
                        </Td>
                      </tr>
                    );
                  })}
                </tbody>
              </Table>
              <Pagination
                offset={offset}
                limit={PAGE}
                total={history.data.total}
                onChange={setOffset}
              />
            </>
          )
        )}
      </section>
    </>
  );
}
