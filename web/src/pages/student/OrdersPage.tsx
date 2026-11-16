// The student's orders.

import { useQuery } from '@tanstack/react-query';
import { Badge, Card, Empty, ErrorAlert, Loading, PageHeader } from '../../components/ui';
import { api } from '../../lib/api';
import { formatDateTime, formatEpc, ORDER_STATUS_LABELS } from '../../lib/format';
import type { Order, OrderStatus, Paged } from '../../lib/types';

export const ORDER_TONES: Record<OrderStatus, 'info' | 'success' | 'warning' | 'error' | 'neutral'> = {
  awaiting_payment: 'warning',
  failed: 'error',
  pending: 'info',
  ready: 'success',
  delivered: 'neutral',
  refunding: 'warning',
  cancelled: 'neutral',
};

const HELP: Partial<Record<OrderStatus, string>> = {
  awaiting_payment: 'La blockchain confirme votre paiement, cela prend quelques secondes.',
  failed: "Le paiement n'a pas abouti : rien n'a été débité.",
  ready: 'Vous pouvez venir la retirer auprès de la boutique.',
  cancelled: 'Vos EPC vous ont été rendus.',
};

export function OrderItems({ order }: { order: Order }) {
  return (
    <ul className="text-sm text-slate-700">
      {order.items.map((item) => (
        <li key={item.product_id} className="flex justify-between gap-4 py-0.5">
          <span>
            {item.quantity} × {item.name}
          </span>
          <span className="tabular-nums text-slate-600">
            {formatEpc(item.unit_price * item.quantity)}
          </span>
        </li>
      ))}
    </ul>
  );
}

export function OrdersPage() {
  const orders = useQuery({
    queryKey: ['orders'],
    queryFn: () => api<Paged<Order>>('/api/shop/orders?limit=100'),
    // Keep an eye on orders whose status is about to change by itself.
    refetchInterval: (query) =>
      query.state.data?.items.some((order) =>
        ['awaiting_payment', 'refunding'].includes(order.status),
      )
        ? 2_000
        : false,
  });

  return (
    <>
      <PageHeader title="Mes commandes" />
      <ErrorAlert error={orders.error} />
      {orders.isPending ? (
        <Loading />
      ) : orders.data && orders.data.items.length === 0 ? (
        <Empty>Vous n'avez pas encore passé de commande.</Empty>
      ) : (
        <ul className="space-y-4">
          {orders.data?.items.map((order) => (
            <li key={order.id}>
              <Card>
                <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
                  <p className="text-sm text-slate-600">{formatDateTime(order.created_at)}</p>
                  <Badge tone={ORDER_TONES[order.status]}>{ORDER_STATUS_LABELS[order.status]}</Badge>
                </div>
                <OrderItems order={order} />
                <p className="mt-3 flex justify-between border-t border-slate-100 pt-3 font-semibold">
                  <span>Total</span>
                  <span className="tabular-nums">{formatEpc(order.total)}</span>
                </p>
                {HELP[order.status] && (
                  <p className="mt-2 text-sm text-slate-600">{HELP[order.status]}</p>
                )}
              </Card>
            </li>
          ))}
        </ul>
      )}
    </>
  );
}
