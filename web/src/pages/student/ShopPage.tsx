// The catalogue.

import { useQuery } from '@tanstack/react-query';
import { ShoppingCart } from 'lucide-react';
import { useCart } from '../../cart';
import {
  Amount,
  Badge,
  Button,
  ButtonLink,
  cx,
  Empty,
  ErrorAlert,
  Loading,
  PageHeader,
} from '../../components/ui';
import { api } from '../../lib/api';
import type { Product } from '../../lib/types';

export function ProductImage({ product, className }: { product: Product; className: string }) {
  if (!product.image_url) {
    return (
      <div
        aria-hidden="true"
        className={cx(
          'display flex items-center justify-center bg-brand-50 text-3xl text-brand-200',
          className,
        )}
      >
        {product.name.charAt(0).toUpperCase()}
      </div>
    );
  }
  // Decorative: the product name is right next to it.
  return (
    <img src={product.image_url} alt="" loading="lazy" className={cx('object-cover', className)} />
  );
}

export function ShopPage() {
  const cart = useCart();
  const products = useQuery({
    queryKey: ['products'],
    queryFn: () => api<Product[]>('/api/shop/products'),
  });

  return (
    <>
      <PageHeader
        title="Boutique"
        description="Échangez vos EPC contre des goodies et des avantages."
        actions={
          cart.count > 0 && (
            <ButtonLink to="/panier">
              <ShoppingCart aria-hidden="true" className="size-4" />
              Voir mon panier ({cart.count})
            </ButtonLink>
          )
        }
      />
      <ErrorAlert error={products.error} />
      {products.isPending ? (
        <Loading />
      ) : products.data && products.data.length === 0 ? (
        <Empty>La boutique est vide pour le moment.</Empty>
      ) : (
        <ul className="grid gap-5 sm:grid-cols-2 lg:grid-cols-3">
          {products.data?.map((product) => {
            const inCart = cart.lines[product.id] ?? 0;
            const soldOut = product.stock === 0;
            return (
              <li key={product.id} className="flex flex-col bg-white ring-1 ring-line">
                <ProductImage product={product} className="h-40 w-full" />
                <div className="flex flex-1 flex-col gap-2 border-t border-line p-5">
                  <div className="flex items-start justify-between gap-3">
                    <h2 className="font-semibold text-slate-900">{product.name}</h2>
                    {soldOut ? (
                      <Badge tone="neutral">Épuisé</Badge>
                    ) : (
                      product.stock <= 5 && <Badge tone="warning">Plus que {product.stock}</Badge>
                    )}
                  </div>
                  {product.description && (
                    <p className="text-sm text-slate-600">{product.description}</p>
                  )}
                  <div className="mt-auto flex items-end justify-between gap-3 pt-4">
                    <p className="display text-3xl text-brand-600">
                      <Amount units={product.price} />
                    </p>
                    <Button
                      size="sm"
                      disabled={soldOut || inCart >= product.stock}
                      onClick={() => cart.add(product.id)}
                      aria-label={`Ajouter ${product.name} au panier`}
                    >
                      {inCart > 0 ? `Ajouter (${inCart} au panier)` : 'Ajouter au panier'}
                    </Button>
                  </div>
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </>
  );
}
