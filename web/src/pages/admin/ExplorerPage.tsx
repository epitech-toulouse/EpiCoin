// Admin: the blockchain explorer — blocks, transactions, balances by address.

import { useQuery } from '@tanstack/react-query';
import { Link, useParams } from 'react-router-dom';
import {
  Badge,
  Card,
  Empty,
  ErrorAlert,
  Loading,
  Mono,
  PageHeader,
  Stat,
  Table,
  Td,
  Th,
} from '../../components/ui';
import { api } from '../../lib/api';
import { formatDateTime, formatEpc, reasonLabel, shortHash } from '../../lib/format';
import type { Block, BlockSummary, ChainAccount, ChainStatus, ChainTx } from '../../lib/types';

const linkClass = 'font-medium text-brand-700 underline';

function AddressLink({ address }: { address: string }) {
  return (
    <Link to={`/admin/explorateur/comptes/${address}`} className={linkClass} title={address}>
      <span className="font-mono text-xs">{shortHash(address)}</span>
    </Link>
  );
}

/** One line of plain French for what a transaction does. */
function TxSummary({ tx }: { tx: ChainTx }) {
  const payload = tx.tx?.payload;
  if (!payload) return <em className="text-slate-500">Contenu inconnu</em>;
  switch (payload.kind) {
    case 'mint':
      return (
        <>
          Distribution de {formatEpc(payload.amount)} à <AddressLink address={payload.to} /> —{' '}
          {reasonLabel(payload.reason)}
        </>
      );
    case 'purchase':
      return <>Achat en boutique de {formatEpc(payload.amount)}</>;
    case 'refund':
      return (
        <>
          Remboursement de l'achat <Mono>{shortHash(payload.purchase)}</Mono>
        </>
      );
    case 'authority_update':
      return payload.action.action === 'grant' ? (
        <>
          Autorité accordée à <AddressLink address={payload.authority} /> — quota{' '}
          {formatEpc(payload.action.quota)}
        </>
      ) : (
        <>
          Autorité révoquée pour <AddressLink address={payload.authority} />
        </>
      );
    case 'transfer':
      return <>Transfert de {formatEpc(payload.amount)}</>;
  }
}

function TxTable({ txs, caption }: { txs: ChainTx[]; caption: string }) {
  if (txs.length === 0) return <Empty>Aucune transaction.</Empty>;
  return (
    <Table caption={caption}>
      <thead>
        <tr>
          <Th>Bloc</Th>
          <Th>Date</Th>
          <Th>Émetteur</Th>
          <Th>Opération</Th>
          <Th>Identifiant</Th>
        </tr>
      </thead>
      <tbody className="divide-y divide-slate-100">
        {txs.map((tx) => (
          <tr key={tx.hash}>
            <Td>
              {tx.height !== undefined ? (
                <Link to={`/admin/explorateur/blocs/${tx.height}`} className={linkClass}>
                  {tx.height}
                </Link>
              ) : (
                <Badge tone="warning">En attente</Badge>
              )}
            </Td>
            <Td className="whitespace-nowrap text-slate-600">
              {tx.timestamp !== undefined && formatDateTime(tx.timestamp)}
            </Td>
            <Td>{tx.tx && <AddressLink address={tx.tx.from} />}</Td>
            <Td>
              <TxSummary tx={tx} />
            </Td>
            <Td>
              <span title={tx.hash}>
                <Mono>{shortHash(tx.hash)}</Mono>
              </span>
            </Td>
          </tr>
        ))}
      </tbody>
    </Table>
  );
}

export function ExplorerPage() {
  const status = useQuery({
    queryKey: ['explorer', 'status'],
    queryFn: () => api<ChainStatus>('/api/explorer/status'),
    refetchInterval: 5_000,
  });
  const height = status.data?.height;
  // Everything below is refreshed whenever a new block appears.
  const blocks = useQuery({
    queryKey: ['explorer', 'blocks', height],
    queryFn: () => api<BlockSummary[]>('/api/explorer/blocks?limit=10'),
    enabled: height !== undefined,
  });
  const txs = useQuery({
    queryKey: ['explorer', 'txs', height],
    queryFn: () => api<ChainTx[]>('/api/explorer/txs?limit=15'),
    enabled: height !== undefined,
  });
  const accounts = useQuery({
    queryKey: ['explorer', 'accounts', height],
    queryFn: () => api<{ items: ChainAccount[] }>('/api/explorer/accounts'),
    enabled: height !== undefined,
  });

  if (status.isPending) return <Loading />;
  if (status.isError) return <ErrorAlert error={status.error} />;

  return (
    <>
      <PageHeader
        title="Blockchain"
        description="Le registre public d'EpiCoin : il ne contient que des adresses, des montants et des codes, jamais de nom."
      />
      <div className="mb-8 grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
        <Stat label="Hauteur" value={status.data.height} hint={`Chaîne ${status.data.chain_id}`} />
        <Stat label="EPC émis" value={formatEpc(status.data.total_minted)} />
        <Stat label="EPC en circulation" value={formatEpc(status.data.circulating_supply)} />
        <Stat label="Transactions en attente" value={status.data.mempool_size} />
      </div>

      <section className="mb-8">
        <h2 className="mb-3 text-lg font-semibold">Derniers blocs</h2>
        <ErrorAlert error={blocks.error} />
        {blocks.isPending ? (
          <Loading />
        ) : blocks.data && blocks.data.length === 0 ? (
          <Empty>Aucun bloc n'a encore été produit.</Empty>
        ) : (
          <Table caption="Derniers blocs">
            <thead>
              <tr>
                <Th>Hauteur</Th>
                <Th>Date</Th>
                <Th className="text-right">Transactions</Th>
                <Th>Hash</Th>
              </tr>
            </thead>
            <tbody className="divide-y divide-slate-100">
              {blocks.data?.map((block) => (
                <tr key={block.hash}>
                  <Td>
                    <Link to={`/admin/explorateur/blocs/${block.height}`} className={linkClass}>
                      {block.height}
                    </Link>
                  </Td>
                  <Td className="whitespace-nowrap text-slate-600">
                    {formatDateTime(block.timestamp)}
                  </Td>
                  <Td className="text-right tabular-nums">{block.tx_count}</Td>
                  <Td>
                    <span title={block.hash}>
                      <Mono>{shortHash(block.hash)}</Mono>
                    </span>
                  </Td>
                </tr>
              ))}
            </tbody>
          </Table>
        )}
      </section>

      <section className="mb-8">
        <h2 className="mb-3 text-lg font-semibold">Dernières transactions</h2>
        <ErrorAlert error={txs.error} />
        {txs.isPending ? <Loading /> : <TxTable txs={txs.data ?? []} caption="Dernières transactions" />}
      </section>

      <section>
        <h2 className="mb-3 text-lg font-semibold">Soldes par adresse</h2>
        <ErrorAlert error={accounts.error} />
        {accounts.isPending ? (
          <Loading />
        ) : accounts.data && accounts.data.items.length === 0 ? (
          <Empty>Aucun compte n'a encore été utilisé.</Empty>
        ) : (
          <Table caption="Soldes par adresse">
            <thead>
              <tr>
                <Th>Adresse</Th>
                <Th>Titulaire</Th>
                <Th className="text-right">Solde</Th>
              </tr>
            </thead>
            <tbody className="divide-y divide-slate-100">
              {accounts.data?.items.map((account) => (
                <tr key={account.address}>
                  <Td>
                    <AddressLink address={account.address} />
                  </Td>
                  <Td>
                    {account.label ?? <em className="text-slate-500">Inconnu</em>}
                    {account.authority?.active && (
                      <span className="ml-2">
                        <Badge>Peut distribuer</Badge>
                      </span>
                    )}
                  </Td>
                  <Td className="text-right whitespace-nowrap tabular-nums">
                    {formatEpc(account.balance)}
                  </Td>
                </tr>
              ))}
            </tbody>
          </Table>
        )}
      </section>
    </>
  );
}

function Back() {
  return (
    <p className="mb-4">
      <Link to="/admin/explorateur" className={linkClass}>
        ← Retour à l'explorateur
      </Link>
    </p>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="grid gap-1 py-2 sm:grid-cols-[12rem_1fr]">
      <dt className="text-sm text-slate-600">{label}</dt>
      <dd className="text-sm">{children}</dd>
    </div>
  );
}

export function ExplorerBlockPage() {
  const { id = '' } = useParams();
  const block = useQuery({
    queryKey: ['explorer', 'block', id],
    queryFn: () => api<Block>(`/api/explorer/blocks/${encodeURIComponent(id)}`),
    retry: false,
  });

  return (
    <>
      <Back />
      {block.isPending ? (
        <Loading />
      ) : block.isError ? (
        <ErrorAlert error={block.error} />
      ) : (
        <>
          <PageHeader title={`Bloc n° ${block.data.height}`} />
          <Card className="mb-6">
            <dl className="divide-y divide-slate-100">
              <Field label="Date">{formatDateTime(block.data.timestamp)}</Field>
              <Field label="Hash">
                <Mono>{block.data.hash}</Mono>
              </Field>
              <Field label="Bloc précédent">
                {block.data.height > 1 ? (
                  <Link to={`/admin/explorateur/blocs/${block.data.height - 1}`} className={linkClass}>
                    <Mono>{block.data.prev_hash}</Mono>
                  </Link>
                ) : (
                  <>
                    <Mono>{block.data.prev_hash}</Mono> (genesis)
                  </>
                )}
              </Field>
              <Field label="Racine des transactions">
                <Mono>{block.data.tx_root}</Mono>
              </Field>
              <Field label="Racine de l'état">
                <Mono>{block.data.state_root}</Mono>
              </Field>
              <Field label="Validateur">
                <Mono>{block.data.validator}</Mono>
              </Field>
              <Field label="Signature">
                <Mono>{block.data.signature}</Mono>
              </Field>
            </dl>
          </Card>
          <h2 className="mb-3 text-lg font-semibold">
            {block.data.txs.length} transaction(s)
          </h2>
          <TxTable txs={block.data.txs} caption="Transactions du bloc" />
        </>
      )}
    </>
  );
}

export function ExplorerAccountPage() {
  const { address = '' } = useParams();
  const account = useQuery({
    queryKey: ['explorer', 'account', address],
    queryFn: () => api<ChainAccount>(`/api/explorer/accounts/${encodeURIComponent(address)}`),
    retry: false,
  });

  return (
    <>
      <Back />
      {account.isPending ? (
        <Loading />
      ) : account.isError ? (
        <ErrorAlert error={account.error} />
      ) : (
        <>
          <PageHeader title={account.data.label ?? 'Compte'} />
          <Card className="mb-6">
            <dl className="divide-y divide-slate-100">
              <Field label="Adresse">
                <Mono>{account.data.address}</Mono>
              </Field>
              <Field label="Solde">{formatEpc(account.data.balance)}</Field>
              <Field label="Transactions émises">{account.data.nonce}</Field>
              {account.data.authority && (
                <Field label="Autorité de distribution">
                  {account.data.authority.active ? 'Active' : 'Révoquée'} — quota{' '}
                  {formatEpc(account.data.authority.quota)}, reste{' '}
                  {formatEpc(account.data.authority.remaining)} sur la période
                </Field>
              )}
            </dl>
          </Card>
          <h2 className="mb-3 text-lg font-semibold">Transactions</h2>
          <TxTable txs={account.data.transactions ?? []} caption="Transactions du compte" />
        </>
      )}
    </>
  );
}
