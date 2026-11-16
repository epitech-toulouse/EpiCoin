// Admin: the audit trail.

import { keepPreviousData, useQuery } from '@tanstack/react-query';
import { useState } from 'react';
import {
  Empty,
  ErrorAlert,
  Loading,
  Mono,
  PageHeader,
  Pagination,
  Table,
  Td,
  Th,
} from '../../components/ui';
import { api, query } from '../../lib/api';
import { formatDateTime } from '../../lib/format';
import type { AuditEntry, Paged } from '../../lib/types';

const PAGE = 50;

const ACTIONS: Record<string, string> = {
  'auth.login': 'Connexion',
  'auth.account_activated': 'Compte activé',
  'auth.password_reset': 'Mot de passe réinitialisé',
  'users.imported': 'Import d’utilisateurs',
  'users.created': 'Utilisateur créé',
  'users.updated': 'Utilisateur modifié',
  'users.deleted': 'Utilisateur supprimé',
  'users.invitation_sent': 'Invitation renvoyée',
  'authority.granted': 'Autorité de distribution accordée',
  'authority.revoked': 'Autorité de distribution révoquée',
  'mint.distributed': 'Distribution d’EPC',
  'shop.product_created': 'Produit créé',
  'shop.product_updated': 'Produit modifié',
  'shop.product_deleted': 'Produit supprimé',
  'shop.order_status_changed': 'Statut de commande modifié',
  'shop.order_refunded': 'Commande remboursée',
};

export function AuditPage() {
  const [offset, setOffset] = useState(0);
  const entries = useQuery({
    queryKey: ['audit', offset],
    queryFn: () => api<Paged<AuditEntry>>(`/api/admin/audit${query({ limit: PAGE, offset })}`),
    placeholderData: keepPreviousData,
  });

  return (
    <>
      <PageHeader
        title="Journal d'audit"
        description="Toutes les actions sensibles, avec leur auteur et leur date."
      />
      <ErrorAlert error={entries.error} />
      {entries.isPending ? (
        <Loading />
      ) : entries.data && entries.data.items.length === 0 ? (
        <Empty>Le journal est vide.</Empty>
      ) : (
        entries.data && (
          <>
            <Table caption="Journal d'audit">
              <thead>
                <tr>
                  <Th>Date</Th>
                  <Th>Auteur</Th>
                  <Th>Action</Th>
                  <Th>Détails</Th>
                </tr>
              </thead>
              <tbody className="divide-y divide-slate-100">
                {entries.data.items.map((entry) => (
                  <tr key={entry.id}>
                    <Td className="whitespace-nowrap text-slate-600">{formatDateTime(entry.at)}</Td>
                    <Td>
                      {entry.actor_name ??
                        (entry.actor_user_id ? (
                          <em className="text-slate-500">Compte supprimé</em>
                        ) : (
                          <em className="text-slate-500">Système</em>
                        ))}
                    </Td>
                    <Td>{ACTIONS[entry.action] ?? entry.action}</Td>
                    <Td>
                      <Mono>{JSON.stringify(entry.details)}</Mono>
                    </Td>
                  </tr>
                ))}
              </tbody>
            </Table>
            <Pagination offset={offset} limit={PAGE} total={entries.data.total} onChange={setOffset} />
          </>
        )
      )}
    </>
  );
}
