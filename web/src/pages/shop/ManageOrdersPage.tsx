// Shop manager: the order queue.

import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  Ban,
  List,
  PackageCheck,
  PackageOpen,
  Truck,
  type LucideIcon,
} from 'lucide-react';
import { useState } from 'react';
import {
  Badge,
  Button,
  Card,
  Empty,
  ErrorAlert,
  Loading,
  PageHeader,
  Pagination,
  Tabs,
} from '../../components/ui';
import { api, query } from '../../lib/api';
import { formatDateTime, formatEpc, ORDER_STATUS_LABELS } from '../../lib/format';
import type { Order, OrderStatus, Paged } from '../../lib/types';
import { OrderItems, ORDER_TONES } from '../student/OrdersPage';

const PAGE = 20;

const FILTERS: { value: OrderStatus | ''; label: string; icon: LucideIcon }[] = [
  { value: 'pending', label: 'À préparer', icon: PackageOpen },
  { value: 'ready', label: 'Prêtes à remettre', icon: PackageCheck },
  { value: 'delivered', label: 'Remises', icon: Truck },
  { value: 'cancelled', label: 'Annulées', icon: Ban },
  { value: '', label: 'Toutes', icon: List },
];

export function ManageOrdersPage() {
  const queryClient = useQueryClient();
  const [status, setStatus] = useState<OrderStatus | ''>('pending');
  const [offset, setOffset] = useState(0);

  const orders = useQuery({
    queryKey: ['manage-orders', status, offset],
    queryFn: () =>
      api<Paged<Order>>(`/api/shop/manage/orders${query({ status, limit: PAGE, offset })}`),
    placeholderData: keepPreviousData,
    // A work queue must stay current while it is on screen.
    refetchInterval: 10_000,
  });

  const refresh = () => queryClient.invalidateQueries({ queryKey: ['manage-orders'] });
  const advance = useMutation({
    mutationFn: ({ order, to }: { order: Order; to: 'ready' | 'delivered' }) =>
      api<Order>(`/api/shop/manage/orders/${order.id}/status`, {
        method: 'POST',
        body: { status: to },
      }),
    onSettled: refresh,
  });
  const cancel = useMutation({
    mutationFn: (order: Order) =>
      api<Order>(`/api/shop/manage/orders/${order.id}/cancel`, { method: 'POST' }),
    onSettled: refresh,
  });
  const busy = advance.isPending || cancel.isPending;

  return (
    <>
      <PageHeader title="Commandes" />
      <Tabs
        label="Filtrer par état"
        value={status}
        options={FILTERS}
        onChange={(value) => {
          setStatus(value);
          setOffset(0);
        }}
      />

      <ErrorAlert error={orders.error} />
      <ErrorAlert error={advance.error} />
      <ErrorAlert error={cancel.error} />
      {orders.isPending ? (
        <Loading />
      ) : orders.data && orders.data.items.length === 0 ? (
        <Empty>Aucune commande dans cette file.</Empty>
      ) : (
        orders.data && (
          <>
            <ul className="space-y-4">
              {orders.data.items.map((order) => (
                <li key={order.id}>
                  <Card>
                    <div className="mb-3 flex flex-wrap items-start justify-between gap-2">
                      <div>
                        <p className="font-medium text-slate-900">
                          {order.buyer?.name ?? <em className="text-slate-500">Compte supprimé</em>}
                        </p>
                        <p className="text-sm text-slate-600">
                          {order.buyer?.email && `${order.buyer.email} · `}
                          {formatDateTime(order.created_at)}
                        </p>
                      </div>
                      <Badge tone={ORDER_TONES[order.status]}>
                        {ORDER_STATUS_LABELS[order.status]}
                      </Badge>
                    </div>
                    <OrderItems order={order} />
                    <div className="mt-3 flex flex-wrap items-center justify-between gap-3 border-t border-line pt-3">
                      <p className="font-semibold tabular-nums">{formatEpc(order.total)}</p>
                      <div className="flex flex-wrap gap-2">
                        {order.status === 'pending' && (
                          <Button
                            size="sm"
                            disabled={busy}
                            onClick={() => advance.mutate({ order, to: 'ready' })}
                          >
                            Marquer comme prête
                          </Button>
                        )}
                        {order.status === 'ready' && (
                          <Button
                            size="sm"
                            disabled={busy}
                            onClick={() => advance.mutate({ order, to: 'delivered' })}
                          >
                            Valider la remise
                          </Button>
                        )}
                        {(order.status === 'pending' || order.status === 'ready') && (
                          <Button
                            size="sm"
                            variant="danger"
                            disabled={busy}
                            onClick={() => {
                              if (
                                window.confirm(
                                  `Annuler cette commande et rembourser ${formatEpc(order.total)} ?`,
                                )
                              ) {
                                cancel.mutate(order);
                              }
                            }}
                          >
                            Annuler et rembourser
                          </Button>
                        )}
                      </div>
                    </div>
                  </Card>
                </li>
              ))}
            </ul>
            <Pagination offset={offset} limit={PAGE} total={orders.data.total} onChange={setOffset} />
          </>
        )
      )}
    </>
  );
}
