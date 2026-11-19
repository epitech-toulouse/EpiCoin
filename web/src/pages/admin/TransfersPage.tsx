// Admin: every transfer between students, with its message.

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
import { formatDateTime, formatEpc } from '../../lib/format';
import type { AdminTransfer, Paged } from '../../lib/types';

const PAGE = 50;

export function TransfersPage() {
  const [offset, setOffset] = useState(0);
  const transfers = useQuery({
    queryKey: ['admin-transfers', offset],
    queryFn: () =>
      api<Paged<AdminTransfer>>(`/api/admin/transfers${query({ limit: PAGE, offset })}`),
    placeholderData: keepPreviousData,
  });

  return (
    <>
      <PageHeader
        title="Échanges"
        description="Les EPC envoyés entre étudiants, avec leurs messages. Les messages restent dans la base, jamais sur la blockchain."
      />
      <ErrorAlert error={transfers.error} />
      {transfers.isPending ? (
        <Loading />
      ) : transfers.data && transfers.data.items.length === 0 ? (
        <Empty>Aucun échange pour l'instant.</Empty>
      ) : (
        transfers.data && (
          <>
            <Table caption="Échanges entre étudiants">
              <thead>
                <tr>
                  <Th>Date</Th>
                  <Th>De</Th>
                  <Th>À</Th>
                  <Th className="text-right">Montant</Th>
                  <Th>Message</Th>
                  <Th>État</Th>
                </tr>
              </thead>
              <tbody className="divide-y divide-line">
                {transfers.data.items.map((transfer) => (
                  <tr key={transfer.tx_hash}>
                    <Td className="whitespace-nowrap">{formatDateTime(transfer.created_at)}</Td>
                    <Td>{transfer.sender_name ?? 'Compte supprimé'}</Td>
                    <Td>{transfer.recipient_name ?? 'Compte supprimé'}</Td>
                    <Td className="text-right whitespace-nowrap">{formatEpc(transfer.amount)}</Td>
                    <Td className="max-w-xs break-words text-slate-700">{transfer.message}</Td>
                    <Td>
                      {transfer.status === 'included' ? (
                        <Badge tone="success">Confirmé</Badge>
                      ) : transfer.status === 'pending' ? (
                        <Badge tone="warning">En cours</Badge>
                      ) : (
                        <Badge tone="error">Refusé</Badge>
                      )}
                    </Td>
                  </tr>
                ))}
              </tbody>
            </Table>
            <Pagination
              offset={offset}
              limit={PAGE}
              total={transfers.data.total}
              onChange={setOffset}
            />
          </>
        )
      )}
    </>
  );
}
