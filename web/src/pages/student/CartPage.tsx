// The cart and its payment.

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useState } from 'react';
import { Link } from 'react-router-dom';
import { useCart } from '../../cart';
import { Alert, Button, Card, Empty, ErrorAlert, Loading, PageHeader } from '../../components/ui';
import { api, ApiError } from '../../lib/api';
import { formatEpc } from '../../lib/format';
import type { Order, Product, Wallet } from '../../lib/types';
import { ProductImage } from './ShopPage';

export function CartPage() {
  const cart = useCart();
  const queryClient = useQueryClient();
  const [paid, setPaid] = useState<Order | null>(null);
  const products = useQuery({
    queryKey: ['products'],
    queryFn: () => api<Product[]>('/api/shop/products'),
  });
  const wallet = useQuery({ queryKey: ['wallet'], queryFn: () => api<Wallet>('/api/me/wallet') });

  const lines = (products.data ?? [])
    .filter((product) => cart.lines[product.id])
    .map((product) => ({ product, quantity: cart.lines[product.id] ?? 0 }));
  // Products that left the catalogue since they were added.
  const missing = Object.keys(cart.lines).filter(
    (id) => products.data && !products.data.some((product) => product.id === id),
  );
  const total = lines.reduce((sum, line) => sum + line.product.price * line.quantity, 0);
  const balance = wallet.data?.balance;
  const tooExpensive = balance !== undefined && total > balance;

  const checkout = useMutation({
    mutationFn: () =>
      api<Order>('/api/shop/orders', {
        method: 'POST',
        body: {
          items: lines.map((line) => ({ product_id: line.product.id, quantity: line.quantity })),
        },
        // The same key is resent if the request is retried: paying twice is impossible.
        idempotencyKey: cart.checkoutKey,
      }),
    onSuccess: (order) => {
      setPaid(order);
      cart.clear();
    },
    onError: (error) => {
      // The server gave a definite "no": the next attempt is a new purchase. After a
      // network failure, on the contrary, the key is kept so that a retry is safe.
      if (error instanceof ApiError && error.status !== 0) cart.renewCheckoutKey();
    },
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: ['wallet'] });
      void queryClient.invalidateQueries({ queryKey: ['products'] });
      void queryClient.invalidateQueries({ queryKey: ['orders'] });
      void queryClient.invalidateQueries({ queryKey: ['history'] });
    },
  });

  if (paid) {
    const confirmed = paid.status === 'pending';
    return (
      <>
        <PageHeader title={confirmed ? 'Commande confirmée' : 'Commande enregistrée'} />
        <div className="space-y-4">
          <Alert tone={confirmed ? 'success' : 'info'}>
            {confirmed
              ? `Votre paiement de ${formatEpc(paid.total)} est confirmé. Vous serez prévenu quand votre commande sera prête à retirer.`
              : 'Votre paiement est en cours de confirmation. Suivez son avancement dans « Mes commandes ».'}
          </Alert>
          <Link to="/commandes" className="text-sm font-medium text-brand-600 underline underline-offset-2">
            Voir mes commandes
          </Link>
        </div>
      </>
    );
  }

  if (products.isPending) return <Loading />;

  return (
    <>
      <PageHeader title="Mon panier" />
      <ErrorAlert error={products.error} />
      {lines.length === 0 ? (
        <Empty>
          Votre panier est vide.{' '}
          <Link to="/boutique" className="font-medium text-brand-600 underline underline-offset-2">
            Découvrir la boutique
          </Link>
        </Empty>
      ) : (
        <div className="grid gap-6 lg:grid-cols-[1fr_20rem]">
          <ul className="space-y-3">
            {lines.map(({ product, quantity }) => (
              <li key={product.id}>
                <Card className="flex items-center gap-4">
                  <ProductImage product={product} className="size-16 shrink-0" />
                  <div className="min-w-0 flex-1">
                    <p className="truncate font-medium text-slate-900">{product.name}</p>
                    <p className="text-sm text-slate-600">{formatEpc(product.price)} l'unité</p>
                    {quantity > product.stock && (
                      <p className="text-sm text-red-700">
                        Stock insuffisant : il en reste {product.stock}.
                      </p>
                    )}
                  </div>
                  <div className="flex items-center gap-2">
                    <Button
                      variant="secondary"
                      size="sm"
                      aria-label={`Retirer un ${product.name}`}
                      onClick={() => cart.setQuantity(product.id, quantity - 1)}
                    >
                      −
                    </Button>
                    <span className="w-6 text-center tabular-nums" aria-live="polite">
                      <span className="sr-only">Quantité : </span>
                      {quantity}
                    </span>
                    <Button
                      variant="secondary"
                      size="sm"
                      aria-label={`Ajouter un ${product.name}`}
                      disabled={quantity >= product.stock}
                      onClick={() => cart.setQuantity(product.id, quantity + 1)}
                    >
                      +
                    </Button>
                  </div>
                  <p className="w-24 text-right font-medium tabular-nums">
                    {formatEpc(product.price * quantity)}
                  </p>
                </Card>
              </li>
            ))}
          </ul>

          <Card className="h-fit space-y-4">
            <dl className="space-y-2 text-sm">
              <div className="flex justify-between">
                <dt className="text-slate-600">Total</dt>
                <dd className="text-lg font-semibold tabular-nums" data-testid="cart-total">
                  {formatEpc(total)}
                </dd>
              </div>
              <div className="flex justify-between">
                <dt className="text-slate-600">Mon solde</dt>
                <dd className="tabular-nums">
                  {balance === undefined ? '…' : formatEpc(balance)}
                </dd>
              </div>
            </dl>
            {missing.length > 0 && (
              <Alert tone="warning">
                Un produit de votre panier n'est plus en vente et a été ignoré.{' '}
                <button
                  type="button"
                  className="underline"
                  onClick={() => missing.forEach((id) => cart.setQuantity(id, 0))}
                >
                  Le retirer
                </button>
              </Alert>
            )}
            {tooExpensive && (
              <Alert tone="warning">Votre solde ne suffit pas pour ce panier.</Alert>
            )}
            <ErrorAlert error={checkout.error} />
            <Button
              className="w-full"
              busy={checkout.isPending}
              disabled={tooExpensive || lines.some((line) => line.quantity > line.product.stock)}
              onClick={() => checkout.mutate()}
            >
              Payer {formatEpc(total)}
            </Button>
            <p className="text-xs text-slate-500">
              Le paiement est enregistré sur la blockchain EpiCoin ; sa confirmation prend
              quelques secondes.
            </p>
          </Card>
        </div>
      )}
    </>
  );
}
