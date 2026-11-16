// Shop manager: the catalogue.

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useState, type FormEvent } from 'react';
import {
  Badge,
  Button,
  Empty,
  ErrorAlert,
  Loading,
  Modal,
  PageHeader,
  Table,
  Td,
  TextAreaField,
  TextField,
  Th,
} from '../../components/ui';
import { api } from '../../lib/api';
import { formatAmount, formatEpc, parseEpc } from '../../lib/format';
import type { Product } from '../../lib/types';
import { ProductImage } from '../student/ShopPage';

const MAX_IMAGE_BYTES = 2 * 1024 * 1024;

function ProductForm({ product, onDone }: { product: Product | null; onDone: () => void }) {
  const queryClient = useQueryClient();
  const [name, setName] = useState(product?.name ?? '');
  const [description, setDescription] = useState(product?.description ?? '');
  const [priceText, setPriceText] = useState(product ? formatAmount(product.price) : '');
  const [stockText, setStockText] = useState(product ? String(product.stock) : '0');
  const [active, setActive] = useState(product?.active ?? true);
  const [image, setImage] = useState<File | null>(null);

  const price = parseEpc(priceText);
  const stock = /^\d{1,7}$/.test(stockText.trim()) ? Number(stockText.trim()) : null;
  const imageTooLarge = image !== null && image.size > MAX_IMAGE_BYTES;

  const save = useMutation({
    mutationFn: async () => {
      const body = { name, description, price, stock, active };
      const saved = product
        ? await api<Product>(`/api/shop/manage/products/${product.id}`, { method: 'PATCH', body })
        : await api<Product>('/api/shop/manage/products', { method: 'POST', body });
      if (image) {
        await api<Product>(`/api/shop/manage/products/${saved.id}/image`, {
          method: 'POST',
          rawBody: image,
        });
      }
    },
    onSuccess: onDone,
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: ['manage-products'] });
      void queryClient.invalidateQueries({ queryKey: ['products'] });
    },
  });

  const submit = (event: FormEvent) => {
    event.preventDefault();
    save.mutate();
  };

  return (
    <form onSubmit={submit} className="space-y-4">
      <ErrorAlert error={save.error} />
      <TextField
        label="Nom"
        required
        maxLength={100}
        value={name}
        onChange={(event) => setName(event.target.value)}
      />
      <TextAreaField
        label="Description"
        rows={3}
        maxLength={2000}
        value={description}
        onChange={(event) => setDescription(event.target.value)}
      />
      <div className="grid grid-cols-2 gap-3">
        <TextField
          label="Prix (EPC)"
          inputMode="decimal"
          required
          error={priceText !== '' && (price === null || price === 0) ? 'Prix invalide.' : null}
          value={priceText}
          onChange={(event) => setPriceText(event.target.value)}
        />
        <TextField
          label="Stock"
          inputMode="numeric"
          required
          error={stock === null ? 'Nombre entier attendu.' : null}
          value={stockText}
          onChange={(event) => setStockText(event.target.value)}
        />
      </div>
      <TextField
        label="Image (PNG, JPEG ou WebP, 2 Mo maximum)"
        type="file"
        accept="image/png,image/jpeg,image/webp"
        error={imageTooLarge ? 'Cette image dépasse 2 Mo.' : null}
        onChange={(event) => setImage(event.target.files?.[0] ?? null)}
      />
      <label className="flex items-center gap-2 text-sm">
        <input
          type="checkbox"
          className="size-4"
          checked={active}
          onChange={(event) => setActive(event.target.checked)}
        />
        En vente (visible dans la boutique)
      </label>
      <div className="flex justify-end gap-2 pt-2">
        <Button variant="secondary" onClick={onDone}>
          Annuler
        </Button>
        <Button
          type="submit"
          busy={save.isPending}
          disabled={!price || stock === null || imageTooLarge || name.trim() === ''}
        >
          Enregistrer
        </Button>
      </div>
    </form>
  );
}

export function ManageProductsPage() {
  const queryClient = useQueryClient();
  const [editing, setEditing] = useState<Product | 'new' | null>(null);
  const products = useQuery({
    queryKey: ['manage-products'],
    queryFn: () => api<Product[]>('/api/shop/manage/products'),
  });
  const remove = useMutation({
    mutationFn: (product: Product) =>
      api<void>(`/api/shop/manage/products/${product.id}`, { method: 'DELETE' }),
    onSettled: () => queryClient.invalidateQueries({ queryKey: ['manage-products'] }),
  });

  return (
    <>
      <PageHeader
        title="Produits"
        actions={<Button onClick={() => setEditing('new')}>Nouveau produit</Button>}
      />
      <ErrorAlert error={products.error} />
      <ErrorAlert error={remove.error} />
      {products.isPending ? (
        <Loading />
      ) : products.data && products.data.length === 0 ? (
        <Empty>Aucun produit. Créez le premier avec « Nouveau produit ».</Empty>
      ) : (
        <Table caption="Produits de la boutique">
          <thead>
            <tr>
              <Th>Produit</Th>
              <Th className="text-right">Prix</Th>
              <Th className="text-right">Stock</Th>
              <Th>État</Th>
              <Th>
                <span className="sr-only">Actions</span>
              </Th>
            </tr>
          </thead>
          <tbody className="divide-y divide-line">
            {products.data?.map((product) => (
              <tr key={product.id}>
                <Td>
                  <div className="flex items-center gap-3">
                    <ProductImage product={product} className="size-10 shrink-0" />
                    <div className="min-w-0">
                      <p className="font-medium">{product.name}</p>
                      <p className="max-w-md truncate text-slate-600">{product.description}</p>
                    </div>
                  </div>
                </Td>
                <Td className="text-right whitespace-nowrap tabular-nums">
                  {formatEpc(product.price)}
                </Td>
                <Td className="text-right tabular-nums">{product.stock}</Td>
                <Td>
                  {product.active ? (
                    <Badge tone="success">En vente</Badge>
                  ) : (
                    <Badge tone="neutral">Retiré</Badge>
                  )}
                </Td>
                <Td className="text-right whitespace-nowrap">
                  <Button variant="ghost" size="sm" onClick={() => setEditing(product)}>
                    Modifier
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => {
                      if (window.confirm(`Supprimer « ${product.name} » ?`)) remove.mutate(product);
                    }}
                  >
                    Supprimer
                  </Button>
                </Td>
              </tr>
            ))}
          </tbody>
        </Table>
      )}

      <Modal
        open={editing !== null}
        title={editing === 'new' ? 'Nouveau produit' : 'Modifier le produit'}
        onClose={() => setEditing(null)}
      >
        {editing !== null && (
          <ProductForm
            key={editing === 'new' ? 'new' : editing.id}
            product={editing === 'new' ? null : editing}
            onDone={() => setEditing(null)}
          />
        )}
      </Modal>
    </>
  );
}
