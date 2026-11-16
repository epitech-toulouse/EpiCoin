// Staff: what I distributed.

import { keepPreviousData, useQuery } from '@tanstack/react-query';
import { useState } from 'react';
import {
  Badge,
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
import { formatDateTime, formatEpc, reasonLabel } from '../../lib/format';
import type { DistributionLine, Paged } from '../../lib/types';

const PAGE = 30;

const STATUS: Record<DistributionLine['status'], { label: string; tone: 'success' | 'warning' | 'error' }> = {
  included: { label: 'Confirmée', tone: 'success' },
  pending: { label: 'En cours', tone: 'warning' },
  failed: { label: 'Refusée', tone: 'error' },
};

export function DistributionsPage() {
  const [offset, setOffset] = useState(0);
  const lines = useQuery({
    queryKey: ['distributions', offset],
    queryFn: () =>
      api<Paged<DistributionLine>>(`/api/staff/distributions${query({ limit: PAGE, offset })}`),
    placeholderData: keepPreviousData,
  });

  return (
    <>
      <PageHeader title="Mes distributions" />
      <ErrorAlert error={lines.error} />
      {lines.isPending ? (
        <Loading />
      ) : lines.data && lines.data.items.length === 0 ? (
        <Empty>Vous n'avez encore rien distribué.</Empty>
      ) : (
        lines.data && (
          <>
            <Table caption="Historique de mes distributions">
              <thead>
                <tr>
                  <Th>Date</Th>
                  <Th>Étudiant</Th>
                  <Th>Motif</Th>
                  <Th className="text-right">Montant</Th>
                  <Th>État</Th>
                </tr>
              </thead>
              <tbody className="divide-y divide-line">
                {lines.data.items.map((line) => (
                  <tr key={line.tx_hash}>
                    <Td className="whitespace-nowrap text-slate-600">
                      {formatDateTime(line.created_at)}
                    </Td>
                    <Td>{line.recipient_name ?? <em className="text-slate-500">Compte supprimé</em>}</Td>
                    <Td>
                      {reasonLabel(line.reason_code)}
                      {line.comment && <span className="block text-slate-600">{line.comment}</span>}
                    </Td>
                    <Td className="text-right font-medium whitespace-nowrap tabular-nums">
                      {formatEpc(line.amount ?? 0)}
                    </Td>
                    <Td>
                      <Badge tone={STATUS[line.status].tone}>{STATUS[line.status].label}</Badge>
                    </Td>
                  </tr>
                ))}
              </tbody>
            </Table>
            <Pagination offset={offset} limit={PAGE} total={lines.data.total} onChange={setOffset} />
          </>
        )
      )}
    </>
  );
}
