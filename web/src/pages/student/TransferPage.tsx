// Students: send EPC to a classmate.

import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Search, Send, Wallet } from 'lucide-react';
import { useState, type FormEvent } from 'react';
import {
  Alert,
  Amount,
  Badge,
  Button,
  Card,
  CardTitle,
  Empty,
  ErrorAlert,
  PageHeader,
  TextAreaField,
  TextField,
} from '../../components/ui';
import { api, ApiError, query } from '../../lib/api';
import { cohortLabel, formatEpc, fullName, parseEpc } from '../../lib/format';
import type { Recipient, Transfer, Wallet as WalletView } from '../../lib/types';

const MAX_MESSAGE = 140;

export function TransferPage() {
  const queryClient = useQueryClient();
  const [search, setSearch] = useState('');
  const [recipient, setRecipient] = useState<Recipient | null>(null);
  const [amountText, setAmountText] = useState('');
  const [message, setMessage] = useState('');
  const [attemptKey, setAttemptKey] = useState(() => crypto.randomUUID());
  const [done, setDone] = useState<Transfer | null>(null);

  const wallet = useQuery({ queryKey: ['wallet'], queryFn: () => api<WalletView>('/api/me/wallet') });
  const term = search.trim();
  const found = useQuery({
    queryKey: ['recipients', term],
    queryFn: () => api<Recipient[]>(`/api/transfers/recipients${query({ q: term })}`),
    enabled: term.length >= 2,
    placeholderData: keepPreviousData,
  });

  const amount = parseEpc(amountText);
  const amountInvalid = amountText.trim() !== '' && (amount === null || amount === 0);
  const overBalance = amount !== null && wallet.data !== undefined && amount > wallet.data.balance;

  // Any change to what is about to be sent makes it a new transfer.
  const touch = () => {
    setAttemptKey(crypto.randomUUID());
    setDone(null);
  };

  const send = useMutation({
    mutationFn: () =>
      api<Transfer>('/api/transfers', {
        method: 'POST',
        body: { recipient_id: recipient?.id, amount, message },
        idempotencyKey: attemptKey,
      }),
    onSuccess: (result) => {
      setDone(result);
      setRecipient(null);
      setSearch('');
      setAmountText('');
      setMessage('');
      setAttemptKey(crypto.randomUUID());
    },
    // A refusal is final: the next attempt is a new transfer. A network error is not: the
    // same key lets a retry find the first attempt instead of sending twice.
    onError: (error) => {
      if (error instanceof ApiError && error.status !== 0) setAttemptKey(crypto.randomUUID());
    },
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: ['wallet'] });
      void queryClient.invalidateQueries({ queryKey: ['history'] });
    },
  });

  const submit = (event: FormEvent) => {
    event.preventDefault();
    send.mutate();
  };
  const canSend = recipient !== null && amount !== null && amount > 0 && !overBalance;

  return (
    <>
      <PageHeader
        title="Envoyer des EPC"
        description="Envoyez des EPC à un autre étudiant. L'envoi est inscrit sur la blockchain et ne peut pas être annulé."
      />

      {done && (
        <div className="mb-6">
          <Alert tone="success">
            {formatEpc(done.amount)} envoyés à {done.recipient_name}
            {done.status === 'pending' && ' (confirmation en cours)'}.
          </Alert>
        </div>
      )}

      <form onSubmit={submit} className="grid gap-6 lg:grid-cols-[1fr_22rem]">
        <Card>
          <CardTitle icon={Search}>1 · Choisir le destinataire</CardTitle>
          {recipient ? (
            <div className="flex flex-wrap items-center justify-between gap-3 bg-slate-50 p-3 ring-1 ring-line">
              <div className="min-w-0">
                <p className="font-medium text-slate-900">{fullName(recipient)}</p>
                <p className="truncate text-sm text-slate-600">{recipient.email}</p>
              </div>
              <div className="flex items-center gap-2">
                {recipient.promo && <Badge tone="neutral">{cohortLabel(recipient.promo)}</Badge>}
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => {
                    setRecipient(null);
                    touch();
                  }}
                >
                  Changer
                </Button>
              </div>
            </div>
          ) : (
            <div className="space-y-3">
              <TextField
                label="Rechercher un étudiant"
                type="search"
                placeholder="Nom, prénom ou email (2 lettres au moins)"
                value={search}
                onChange={(event) => setSearch(event.target.value)}
              />
              <ErrorAlert error={found.error} />
              {term.length >= 2 &&
                found.data &&
                (found.data.length === 0 ? (
                  <Empty>Aucun étudiant ne correspond.</Empty>
                ) : (
                  <ul className="max-h-80 divide-y divide-line overflow-y-auto ring-1 ring-line">
                    {found.data.map((student) => (
                      <li key={student.id}>
                        <button
                          type="button"
                          className="flex w-full items-center gap-3 px-3 py-2 text-left hover:bg-slate-50"
                          onClick={() => {
                            setRecipient(student);
                            touch();
                          }}
                        >
                          <span className="min-w-0 flex-1">
                            <span className="block truncate font-medium text-slate-900">
                              {fullName(student)}
                            </span>
                            <span className="block truncate text-sm text-slate-600">
                              {student.email}
                            </span>
                          </span>
                          {student.promo && (
                            <Badge tone="neutral">{cohortLabel(student.promo)}</Badge>
                          )}
                        </button>
                      </li>
                    ))}
                  </ul>
                ))}
            </div>
          )}
        </Card>

        <Card>
          <CardTitle icon={Send}>2 · Envoyer</CardTitle>
          <div className="space-y-4">
            <div className="flex items-center gap-2 text-sm text-slate-600">
              <Wallet aria-hidden="true" className="size-4" strokeWidth={1.75} />
              Votre solde :{' '}
              {wallet.data ? <Amount units={wallet.data.balance} className="text-slate-900" /> : '…'}
            </div>
            <TextField
              label="Montant (EPC)"
              inputMode="decimal"
              placeholder="5"
              error={
                amountInvalid
                  ? 'Montant invalide.'
                  : overBalance
                    ? 'C’est plus que votre solde.'
                    : null
              }
              value={amountText}
              onChange={(event) => {
                setAmountText(event.target.value);
                touch();
              }}
            />
            <TextAreaField
              label="Message (facultatif)"
              hint={`Le destinataire le lit dans son historique. ${message.length}/${MAX_MESSAGE}`}
              rows={3}
              maxLength={MAX_MESSAGE}
              value={message}
              onChange={(event) => {
                setMessage(event.target.value);
                touch();
              }}
            />
            <ErrorAlert error={send.error} />
            <Button type="submit" busy={send.isPending} disabled={!canSend} className="w-full">
              {recipient && amount
                ? `Envoyer ${formatEpc(amount)} à ${recipient.first_name}`
                : 'Envoyer'}
            </Button>
            <p className="text-xs text-slate-500">
              Un envoi est définitif : seul le destinataire peut vous renvoyer des EPC.
            </p>
          </div>
        </Card>
      </form>
    </>
  );
}
