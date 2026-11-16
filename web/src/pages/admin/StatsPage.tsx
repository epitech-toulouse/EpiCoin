// Admin: the dashboard.

import { useQuery } from '@tanstack/react-query';
import {
  Empty,
  ErrorAlert,
  Loading,
  PageHeader,
  Stat,
  Table,
  Td,
  Th,
} from '../../components/ui';
import { api } from '../../lib/api';
import { formatEpc, reasonLabel, ROLE_LABELS } from '../../lib/format';
import type { Stats } from '../../lib/types';

/** A horizontal bar proportional to `value / max`, with the value still readable as text. */
function Bar({ value, max }: { value: number; max: number }) {
  const percent = max > 0 ? Math.round((value / max) * 100) : 0;
  return (
    <div aria-hidden="true" className="h-2 w-full min-w-24 rounded-full bg-slate-100">
      <div className="h-2 rounded-full bg-brand-600" style={{ width: `${percent}%` }} />
    </div>
  );
}

export function StatsPage() {
  const stats = useQuery({
    queryKey: ['stats'],
    queryFn: () => api<Stats>('/api/admin/stats'),
    refetchInterval: 30_000,
  });

  if (stats.isPending) return <Loading />;
  if (stats.isError) return <ErrorAlert error={stats.error} />;
  const { supply, chain, minted_by_staff, minted_by_reason, shop, users } = stats.data;
  const maxStaff = Math.max(...minted_by_staff.map((row) => row.total), 0);
  const maxReason = Math.max(...minted_by_reason.map((row) => row.total), 0);

  return (
    <>
      <PageHeader
        title="Statistiques"
        description={`Chaîne ${chain.chain_id}, bloc n° ${chain.height}.`}
      />

      <div className="mb-8 grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
        <Stat label="Masse monétaire" value={formatEpc(supply.circulating)} hint="EPC en circulation" />
        <Stat label="Détenus par les étudiants" value={formatEpc(supply.held_by_users)} />
        <Stat label="Trésorerie de la boutique" value={formatEpc(supply.treasury_balance)} />
        <Stat
          label="Ventes de la boutique"
          value={formatEpc(shop.totals.revenue)}
          hint={`${shop.totals.orders} commande(s), ${shop.totals.refunded_orders} remboursée(s)`}
        />
      </div>

      <div className="grid gap-8 lg:grid-cols-2">
        <section>
          <h2 className="mb-3 text-lg font-semibold">EPC distribués par encadrant</h2>
          {minted_by_staff.length === 0 ? (
            <Empty>Aucune distribution pour l'instant.</Empty>
          ) : (
            <Table caption="EPC distribués par encadrant">
              <thead>
                <tr>
                  <Th>Encadrant</Th>
                  <Th className="text-right">Distributions</Th>
                  <Th className="text-right">Total</Th>
                  <Th className="w-1/3" />
                </tr>
              </thead>
              <tbody className="divide-y divide-slate-100">
                {minted_by_staff.map((row) => (
                  <tr key={row.user_id ?? 'deleted'}>
                    <Td>{row.name ?? <em className="text-slate-500">Compte supprimé</em>}</Td>
                    <Td className="text-right tabular-nums">{row.count}</Td>
                    <Td className="text-right whitespace-nowrap tabular-nums">{formatEpc(row.total)}</Td>
                    <Td className="align-middle">
                      <Bar value={row.total} max={maxStaff} />
                    </Td>
                  </tr>
                ))}
              </tbody>
            </Table>
          )}
        </section>

        <section>
          <h2 className="mb-3 text-lg font-semibold">EPC distribués par motif</h2>
          {minted_by_reason.length === 0 ? (
            <Empty>Aucune distribution pour l'instant.</Empty>
          ) : (
            <Table caption="EPC distribués par motif">
              <thead>
                <tr>
                  <Th>Motif</Th>
                  <Th className="text-right">Distributions</Th>
                  <Th className="text-right">Total</Th>
                  <Th className="w-1/3" />
                </tr>
              </thead>
              <tbody className="divide-y divide-slate-100">
                {minted_by_reason.map((row) => (
                  <tr key={row.reason_code ?? 'none'}>
                    <Td>{reasonLabel(row.reason_code)}</Td>
                    <Td className="text-right tabular-nums">{row.count}</Td>
                    <Td className="text-right whitespace-nowrap tabular-nums">{formatEpc(row.total)}</Td>
                    <Td className="align-middle">
                      <Bar value={row.total} max={maxReason} />
                    </Td>
                  </tr>
                ))}
              </tbody>
            </Table>
          )}
        </section>

        <section>
          <h2 className="mb-3 text-lg font-semibold">Ventes par produit</h2>
          {shop.by_product.length === 0 ? (
            <Empty>Aucune vente pour l'instant.</Empty>
          ) : (
            <Table caption="Ventes par produit">
              <thead>
                <tr>
                  <Th>Produit</Th>
                  <Th className="text-right">Quantité</Th>
                  <Th className="text-right">Recette</Th>
                </tr>
              </thead>
              <tbody className="divide-y divide-slate-100">
                {shop.by_product.map((row) => (
                  <tr key={row.product_id}>
                    <Td>{row.name}</Td>
                    <Td className="text-right tabular-nums">{row.quantity}</Td>
                    <Td className="text-right whitespace-nowrap tabular-nums">{formatEpc(row.revenue)}</Td>
                  </tr>
                ))}
              </tbody>
            </Table>
          )}
        </section>

        <section>
          <h2 className="mb-3 text-lg font-semibold">Comptes</h2>
          <Table caption="Comptes par rôle">
            <thead>
              <tr>
                <Th>Rôle</Th>
                <Th className="text-right">Comptes</Th>
                <Th className="text-right">Activés</Th>
              </tr>
            </thead>
            <tbody className="divide-y divide-slate-100">
              {users.map((row) => (
                <tr key={row.role}>
                  <Td>{ROLE_LABELS[row.role]}</Td>
                  <Td className="text-right tabular-nums">{row.total}</Td>
                  <Td className="text-right tabular-nums">{row.activated}</Td>
                </tr>
              ))}
            </tbody>
          </Table>
        </section>
      </div>
    </>
  );
}
