// The catalogue.

import { useQuery } from '@tanstack/react-query';
import { Link } from 'react-router-dom';
import { useCart } from '../../cart';
import { Badge, Button, Empty, ErrorAlert, Loading, PageHeader } from '../../components/ui';
import { api } from '../../lib/api';
import { formatEpc } from '../../lib/format';
import type { Product } from '../../lib/types';

export function ProductImage({ product, className }: { product: Product; className: string }) {
  if (!product.image_url) {
    return (
      <div
        aria-hidden="true"
        className={`flex items-center justify-center bg-slate-100 text-3xl font-semibold text-slate-400 ${className}`}
      >
        {product.name.charAt(0).toUpperCase()}
      </div>
    );
  }
  // Decorative: the product name is right next to it.
  return <img src={product.image_url} alt="" loading="lazy" className={`object-cover ${className}`} />;
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
            <Link
              to="/panier"
              className="inline-flex min-h-11 items-center rounded-md bg-brand-600 px-4 text-sm font-medium text-white hover:bg-brand-700"
            >
              Voir mon panier ({cart.count})
            </Link>
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
              <li
                key={product.id}
                className="flex flex-col overflow-hidden rounded-lg bg-white shadow-sm ring-1 ring-slate-200"
              >
                <ProductImage product={product} className="h-40 w-full" />
                <div className="flex flex-1 flex-col gap-2 p-4">
                  <h2 className="font-semibold text-slate-900">{product.name}</h2>
                  {product.description && (
                    <p className="text-sm text-slate-600">{product.description}</p>
                  )}
                  <div className="mt-auto flex items-center justify-between gap-3 pt-3">
                    <div>
                      <p className="font-semibold tabular-nums text-slate-900">
                        {formatEpc(product.price)}
                      </p>
                      {soldOut ? (
                        <Badge tone="neutral">Épuisé</Badge>
                      ) : (
                        product.stock <= 5 && (
                          <Badge tone="warning">Plus que {product.stock}</Badge>
                        )
                      )}
                    </div>
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
