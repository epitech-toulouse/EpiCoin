// Admin: the dashboard — what needs doing, then the state of the platform.

import { useQuery } from '@tanstack/react-query';
import { ClipboardList, Coins, MailWarning, Store, Tags, UserCheck, Users } from 'lucide-react';
import {
  ActionCard,
  Badge,
  Card,
  CardTitle,
  Donut,
  Empty,
  ErrorAlert,
  Loading,
  PageHeader,
  RowBar,
  SectionTitle,
  SegmentedBar,
  Stat,
  Table,
  Td,
  Th,
} from '../../components/ui';
import { api } from '../../lib/api';
import { formatEpc, reasonLabel, ROLE_LABELS } from '../../lib/format';
import type { Order, Paged, Stats } from '../../lib/types';

export function DashboardPage() {
  const stats = useQuery({
    queryKey: ['stats'],
    queryFn: () => api<Stats>('/api/admin/stats'),
    refetchInterval: 30_000,
  });
  const toPrepare = useQuery({
    queryKey: ['manage-orders', 'pending', 'count'],
    queryFn: () => api<Paged<Order>>('/api/shop/manage/orders?status=pending&limit=1'),
    refetchInterval: 30_000,
  });

  if (stats.isPending) return <Loading />;
  if (stats.isError) return <ErrorAlert error={stats.error} />;
  const { supply, chain, minted_by_staff, minted_by_reason, shop, users } = stats.data;

  const accounts = users.reduce((sum, row) => sum + row.total, 0);
  const activated = users.reduce((sum, row) => sum + row.activated, 0);
  const pendingInvitations = accounts - activated;
  const ordersToPrepare = toPrepare.data?.total ?? 0;
  const maxStaff = Math.max(...minted_by_staff.map((row) => row.total), 0);
  const maxReason = Math.max(...minted_by_reason.map((row) => row.total), 0);

  return (
    <>
      <PageHeader
        title="Tableau de bord"
        meta={
          <>
            <Badge>Chaîne {chain.chain_id}</Badge>
            <Badge tone="neutral">Bloc n° {chain.height}</Badge>
          </>
        }
      />

      <section className="mb-10">
        <SectionTitle tone="danger">Actions prioritaires</SectionTitle>
        <div className="grid gap-5 lg:grid-cols-2">
          <ActionCard
            count={ordersToPrepare}
            title="Commandes à préparer"
            icon={ClipboardList}
            to="/gestion/commandes"
          >
            {ordersToPrepare > 0
              ? `${ordersToPrepare} commande(s) payée(s) en attente de préparation.`
              : 'Aucune commande en attente.'}
          </ActionCard>
          <ActionCard
            count={pendingInvitations}
            title="Comptes non activés"
            icon={MailWarning}
            to="/admin/utilisateurs"
          >
            {pendingInvitations > 0
              ? `${pendingInvitations} invitation(s) envoyée(s) et pas encore acceptée(s).`
              : 'Tous les comptes sont activés.'}
          </ActionCard>
        </div>
      </section>

      <section>
        <SectionTitle>Aperçu</SectionTitle>
        <div className="grid gap-5 lg:grid-cols-2">
          <Card className="lg:col-span-2">
            <CardTitle icon={Coins} to="/admin/explorateur">
              Masse monétaire
            </CardTitle>
            <div className="mb-6 grid gap-6 sm:grid-cols-3">
              <Stat
                label="En circulation"
                value={formatEpc(supply.circulating)}
                size="lg"
                hint={`${formatEpc(supply.total_minted)} émis depuis la genèse`}
              />
              <Stat
                label="Détenus par les étudiants"
                value={formatEpc(supply.held_by_users)}
              />
              <Stat
                label="Trésorerie de la boutique"
                value={formatEpc(supply.treasury_balance)}
                tone="teal"
              />
            </div>
            <SegmentedBar
              label="Répartition des EPC en circulation"
              parts={[
                {
                  label: 'Étudiants et encadrants',
                  value: supply.held_by_users,
                  display: formatEpc(supply.held_by_users),
                  tone: 'brand',
                },
                {
                  label: 'Trésorerie',
                  value: supply.treasury_balance,
                  display: formatEpc(supply.treasury_balance),
                  tone: 'teal',
                },
              ]}
            />
          </Card>

          <Card>
            <CardTitle icon={UserCheck} tone="ok" to="/admin/utilisateurs">
              Comptes
            </CardTitle>
            <Donut
              label="Comptes actifs"
              total={String(accounts)}
              parts={[
                { label: 'Activés', value: activated, display: String(activated), tone: 'ok' },
                {
                  label: 'Invitation en attente',
                  value: pendingInvitations,
                  display: String(pendingInvitations),
                  tone: 'warn',
                },
              ]}
            />
            <ul className="mt-5 flex flex-wrap gap-2 border-t border-line pt-4">
              {users.map((row) => (
                <li key={row.role}>
                  <Badge tone="neutral">
                    {ROLE_LABELS[row.role]} · {row.total}
                  </Badge>
                </li>
              ))}
            </ul>
          </Card>

          <Card>
            <CardTitle icon={Store} tone="accent" to="/gestion/commandes">
              Boutique
            </CardTitle>
            <Stat
              label="Ventes"
              value={formatEpc(shop.totals.revenue)}
              tone="accent"
              size="lg"
              hint={`${shop.totals.orders} commande(s), ${shop.totals.refunded_orders} remboursée(s)`}
            />
            {shop.by_product.length > 0 && (
              <div className="mt-5 border-t border-line pt-2">
                <Table caption="Ventes par produit" bare>
                  <thead>
                    <tr>
                      <Th className="pl-0">Produit</Th>
                      <Th className="text-right">Quantité</Th>
                      <Th className="pr-0 text-right">Recette</Th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-line">
                    {shop.by_product.slice(0, 5).map((row) => (
                      <tr key={row.product_id}>
                        <Td className="pl-0">{row.name}</Td>
                        <Td className="text-right tabular-nums">{row.quantity}</Td>
                        <Td className="pr-0 text-right whitespace-nowrap tabular-nums">
                          {formatEpc(row.revenue)}
                        </Td>
                      </tr>
                    ))}
                  </tbody>
                </Table>
              </div>
            )}
          </Card>

          <Card>
            <CardTitle icon={Users}>Distribué par personne</CardTitle>
            {minted_by_staff.length === 0 ? (
              <Empty>Aucune distribution pour l'instant.</Empty>
            ) : (
              <Table caption="EPC distribués par personne" bare>
                <thead>
                  <tr>
                    <Th className="pl-0">Encadrant ou admin</Th>
                    <Th className="text-right">Distributions</Th>
                    <Th className="text-right">Total</Th>
                    <Th className="w-1/3 pr-0">
                      <span className="sr-only">Part</span>
                    </Th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-line">
                  {minted_by_staff.map((row) => (
                    <tr key={row.user_id ?? 'deleted'}>
                      <Td className="pl-0">
                        {row.name ?? <em className="text-slate-500">Compte supprimé</em>}
                      </Td>
                      <Td className="text-right tabular-nums">{row.count}</Td>
                      <Td className="text-right whitespace-nowrap tabular-nums">
                        {formatEpc(row.total)}
                      </Td>
                      <Td className="pr-0 align-middle">
                        <RowBar value={row.total} max={maxStaff} />
                      </Td>
                    </tr>
                  ))}
                </tbody>
              </Table>
            )}
          </Card>

          <Card>
            <CardTitle icon={Tags} tone="teal">
              Distribué par motif
            </CardTitle>
            {minted_by_reason.length === 0 ? (
              <Empty>Aucune distribution pour l'instant.</Empty>
            ) : (
              <Table caption="EPC distribués par motif" bare>
                <thead>
                  <tr>
                    <Th className="pl-0">Motif</Th>
                    <Th className="text-right">Distributions</Th>
                    <Th className="text-right">Total</Th>
                    <Th className="w-1/3 pr-0">
                      <span className="sr-only">Part</span>
                    </Th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-line">
                  {minted_by_reason.map((row) => (
                    <tr key={row.reason_code ?? 'none'}>
                      <Td className="pl-0">{reasonLabel(row.reason_code)}</Td>
                      <Td className="text-right tabular-nums">{row.count}</Td>
                      <Td className="text-right whitespace-nowrap tabular-nums">
                        {formatEpc(row.total)}
                      </Td>
                      <Td className="pr-0 align-middle">
                        <RowBar value={row.total} max={maxReason} />
                      </Td>
                    </tr>
                  ))}
                </tbody>
              </Table>
            )}
          </Card>
        </div>
      </section>
    </>
  );
}
