// The student's home: balance, and everything that moved it.

import { keepPreviousData, useQuery } from '@tanstack/react-query';
import { useState } from 'react';
import { Link } from 'react-router-dom';
import { useAuth } from '../../auth';
import {
  Badge,
  Card,
  Empty,
  ErrorAlert,
  Loading,
  PageHeader,
  Pagination,
  Table,
  Td,
  Th,
} from '../../components/ui';
import { api, query } from '../../lib/api';
import { formatAmount, formatDateTime, formatEpc, reasonLabel } from '../../lib/format';
import type { HistoryLine, Paged, Wallet } from '../../lib/types';

const PAGE = 20;

function describe(line: HistoryLine): { title: string; detail: string | null } {
  switch (line.kind) {
    case 'mint':
      return {
        title: reasonLabel(line.reason_code),
        detail: [line.from_name && `par ${line.from_name}`, line.comment].filter(Boolean).join(' — ') || null,
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
  const wallet = useQuery({ queryKey: ['wallet'], queryFn: () => api<Wallet>('/api/me/wallet') });
  const history = useQuery({
    queryKey: ['history', offset],
    queryFn: () =>
      api<Paged<HistoryLine>>(`/api/me/transactions${query({ limit: PAGE, offset })}`),
    placeholderData: keepPreviousData,
  });

  return (
    <>
      <PageHeader title={`Bonjour ${user?.first_name ?? ''}`} />

      <Card className="mb-8 flex flex-wrap items-center justify-between gap-4">
        <div>
          <p className="text-sm text-slate-600">Mon solde</p>
          {wallet.isPending ? (
            <Loading label="Lecture du solde…" />
          ) : wallet.isError ? (
            <ErrorAlert error={wallet.error} />
          ) : (
            <p className="text-4xl font-semibold tabular-nums text-slate-900" data-testid="balance">
              {formatAmount(wallet.data.balance)}
              <span className="ml-2 text-lg font-medium text-slate-500">EPC</span>
            </p>
          )}
        </div>
        <Link
          to="/boutique"
          className="inline-flex min-h-11 items-center rounded-md bg-brand-600 px-4 text-sm font-medium text-white hover:bg-brand-700"
        >
          Aller à la boutique
        </Link>
      </Card>

      <h2 className="mb-3 text-lg font-semibold text-slate-900">Historique</h2>
      <ErrorAlert error={history.error} />
      {history.isPending ? (
        <Loading />
      ) : history.data && history.data.items.length === 0 ? (
        <Empty>Aucun mouvement pour l'instant. Vos EPC apparaîtront ici dès qu'un encadrant vous en attribuera.</Empty>
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
              <tbody className="divide-y divide-slate-100">
                {history.data.items.map((line) => {
                  const { title, detail } = describe(line);
                  const credit = line.direction === 'credit';
                  return (
                    <tr key={line.tx_hash}>
                      <Td className="whitespace-nowrap text-slate-600">
                        {formatDateTime(line.created_at)}
                      </Td>
                      <Td>
                        <span className="font-medium">{title}</span>
                        {line.status === 'pending' && (
                          <span className="ml-2">
                            <Badge tone="warning">En cours</Badge>
                          </span>
                        )}
                        {detail && <span className="block text-slate-600">{detail}</span>}
                      </Td>
                      <Td
                        className={`text-right font-medium whitespace-nowrap tabular-nums ${
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
            <Pagination offset={offset} limit={PAGE} total={history.data.total} onChange={setOffset} />
          </>
        )
      )}
    </>
  );
}
