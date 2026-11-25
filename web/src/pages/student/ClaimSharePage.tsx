// Scanning a share's QR code. The code opens `/partage#token=…` (ShareLinkPage), which keeps
// the token and moves on to `/partages/recuperer` (ClaimSharePage), through the login screen
// if needed.

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { ArrowRight, CircleCheck, QrCode as QrIcon } from 'lucide-react';
import { useState } from 'react';
import { Navigate, useLocation } from 'react-router-dom';
import { MentionChip, useSecondsLeft } from '../../components/shares';
import {
  Alert,
  Amount,
  Button,
  ButtonLink,
  Card,
  CardTitle,
  ErrorAlert,
  Loading,
  PageHeader,
} from '../../components/ui';
import { api } from '../../lib/api';
import { formatEpc, formatSeconds } from '../../lib/format';
import { forgetShareToken, pendingShareToken, rememberShareToken, tokenFromFragment } from '../../lib/shares';
import type { ShareClaim, SharePreview } from '../../lib/types';

/** The page a QR code opens: keeps its token out of the address bar, then goes on. */
export function ShareLinkPage() {
  const { hash } = useLocation();
  // Before rendering <Navigate>: its effect runs before ours would. Storing is idempotent.
  useState(() => {
    const token = tokenFromFragment(hash);
    if (token) rememberShareToken(token);
    return token;
  });
  return <Navigate to="/partages/recuperer" replace />;
}

function Claimed({ result }: { result: ShareClaim }) {
  return (
    <Card>
      <div className="flex flex-col items-center gap-3 py-4 text-center">
        <CircleCheck aria-hidden="true" className="size-12 text-emerald-700" strokeWidth={1.5} />
        <p className="display text-5xl text-emerald-700" data-testid="share-claimed">
          +<Amount units={result.amount + result.bonus} />
        </p>
        <p className="text-slate-700">
          Merci pour ce partage{result.staff_name ? ` avec ${result.staff_name}` : ''} !
        </p>
        <MentionChip code={result.mention} />
        {result.bonus > 0 && (
          <Alert tone="success">
            Série de {result.streak_days} jours : {formatEpc(result.bonus)} de bonus de régularité
            inclus.
          </Alert>
        )}
        {result.reward_status === 'pending' && (
          <p className="text-sm text-slate-600">Le versement est en cours d’inscription sur la blockchain.</p>
        )}
        <div className="mt-2">
          <ButtonLink to="/partages">
            Voir mes partages
            <ArrowRight aria-hidden="true" className="size-4" />
          </ButtonLink>
        </div>
      </div>
    </Card>
  );
}

export function ClaimSharePage() {
  const queryClient = useQueryClient();
  const [token] = useState(pendingShareToken);
  const preview = useQuery({
    queryKey: ['shares', 'preview', token],
    queryFn: () => api<SharePreview>('/api/shares/preview', { method: 'POST', body: { token } }),
    enabled: token !== '',
    retry: false,
    staleTime: Infinity,
  });
  const claim = useMutation({
    mutationFn: () => api<ShareClaim>('/api/shares/claim', { method: 'POST', body: { token } }),
    onSuccess: () => {
      forgetShareToken();
      void queryClient.invalidateQueries({ queryKey: ['shares', 'me'] });
      void queryClient.invalidateQueries({ queryKey: ['wallet'] });
      void queryClient.invalidateQueries({ queryKey: ['history'] });
    },
  });
  const deadline = preview.data ? preview.dataUpdatedAt + preview.data.expires_in * 1000 : null;
  const secondsLeft = useSecondsLeft(deadline);

  let content;
  if (claim.data) {
    content = <Claimed result={claim.data} />;
  } else if (token === '') {
    content = (
      <Alert tone="warning">
        Aucun QR code à récupérer. Scannez à nouveau le QR code que vous montre l’encadrant.
      </Alert>
    );
  } else if (preview.isPending) {
    content = <Loading label="Lecture du QR code…" />;
  } else if (preview.isError) {
    content = <ErrorAlert error={preview.error} />;
  } else {
    const share = preview.data;
    const expired = secondsLeft === 0;
    content = (
      <Card>
        <CardTitle icon={QrIcon} tone="ok">
          Un partage pour vous
        </CardTitle>
        <p className="display text-5xl text-emerald-700">
          +<Amount units={share.amount} />
        </p>
        <p className="mt-3 text-slate-800">
          <span className="font-medium">{share.topic || 'Partage de connaissance'}</span>
          {share.staff_name && <span className="text-slate-600"> · avec {share.staff_name}</span>}
        </p>
        {share.mention && (
          <p className="mt-2">
            <MentionChip code={share.mention} />
          </p>
        )}
        <div className="mt-5 space-y-3">
          <ErrorAlert error={claim.error} />
          <Button
            className="w-full sm:w-auto"
            busy={claim.isPending}
            disabled={expired}
            onClick={() => claim.mutate()}
          >
            Récupérer mes EPC
          </Button>
          <p className="font-mono text-xs text-slate-600" aria-live="polite">
            {expired
              ? 'Ce QR code a expiré : demandez à l’encadrant d’en générer un nouveau.'
              : `Valable encore ${formatSeconds(secondsLeft)}`}
          </p>
        </div>
      </Card>
    );
  }

  return (
    <>
      <PageHeader title="Récupérer un partage" />
      <div className="max-w-xl">{content}</div>
    </>
  );
}
