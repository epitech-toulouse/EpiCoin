// Students: challenges, and the EPC they pay as soon as they are won.

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useId, useState, type FormEvent } from 'react';
import {
  Alert,
  Badge,
  Button,
  Empty,
  ErrorAlert,
  Loading,
  PageHeader,
  SectionTitle,
  TextAreaField,
  TextField,
  TONE_TEXT,
} from '../../components/ui';
import { api } from '../../lib/api';
import { CHALLENGE_MODE_LABELS, cohortLabel, formatDateTime, formatEpc } from '../../lib/format';
import type { StudentChallenge } from '../../lib/types';

const MAX_PROOF = 1000;

/** Won: approved, with a reward the chain did not refuse. */
function won(challenge: StudentChallenge): boolean {
  const submission = challenge.submission;
  return submission?.status === 'approved' && submission.reward_status !== 'failed';
}

function useRefresh() {
  const queryClient = useQueryClient();
  return () => queryClient.invalidateQueries({ queryKey: ['challenges'] });
}

function AnswerForm({ challenge }: { challenge: StudentChallenge }) {
  const refresh = useRefresh();
  const [answer, setAnswer] = useState('');
  const send = useMutation({
    mutationFn: () =>
      api<{ amount: number; reward_status: string }>(`/api/challenges/${challenge.id}/answer`, {
        method: 'POST',
        body: { answer },
      }),
    onSuccess: refresh,
  });
  const submit = (event: FormEvent) => {
    event.preventDefault();
    send.mutate();
  };
  return (
    <form onSubmit={submit} className="space-y-3">
      <ErrorAlert error={send.error} />
      <div className="flex flex-wrap items-end gap-2">
        <div className="min-w-48 flex-1">
          <TextField
            label="Votre réponse"
            value={answer}
            maxLength={200}
            onChange={(event) => setAnswer(event.target.value)}
          />
        </div>
        <Button type="submit" busy={send.isPending} disabled={answer.trim() === ''}>
          Valider
        </Button>
      </div>
    </form>
  );
}

function ProofForm({ challenge }: { challenge: StudentChallenge }) {
  const refresh = useRefresh();
  const [proof, setProof] = useState('');
  const send = useMutation({
    mutationFn: () =>
      api<void>(`/api/challenges/${challenge.id}/submission`, {
        method: 'POST',
        body: { proof },
      }),
    onSuccess: refresh,
  });
  const submit = (event: FormEvent) => {
    event.preventDefault();
    send.mutate();
  };
  return (
    <form onSubmit={submit} className="space-y-3">
      <ErrorAlert error={send.error} />
      <TextAreaField
        label="Comment avez-vous réussi ?"
        hint={`Un lien, une explication… L'encadrant le lit avant de valider. ${proof.length}/${MAX_PROOF}`}
        rows={3}
        maxLength={MAX_PROOF}
        value={proof}
        onChange={(event) => setProof(event.target.value)}
      />
      <div className="flex justify-end">
        <Button type="submit" busy={send.isPending} disabled={proof.trim() === ''}>
          Envoyer
        </Button>
      </div>
    </form>
  );
}

function ChallengeCard({ challenge }: { challenge: StudentChallenge }) {
  const titleId = useId();
  const submission = challenge.submission;
  const placesLeft =
    challenge.max_winners === null ? null : Math.max(challenge.max_winners - challenge.winners, 0);
  const form = challenge.mode === 'answer' ? <AnswerForm challenge={challenge} /> : <ProofForm challenge={challenge} />;

  let status;
  if (won(challenge)) {
    status = (
      <Alert tone="success">
        Défi réussi : {formatEpc(challenge.reward)} gagnés
        {submission?.reward_status === 'pending' && ' (versement en cours)'}.
      </Alert>
    );
  } else if (submission?.status === 'pending') {
    status = (
      <Alert tone="info">
        En attente de validation par un encadrant.
        <span className="mt-1 block whitespace-pre-line text-slate-600">« {submission.proof} »</span>
      </Alert>
    );
  } else if (!challenge.open) {
    status = <Alert tone="info">Ce défi est terminé.</Alert>;
  } else {
    status = (
      <div className="space-y-3">
        {submission?.status === 'rejected' && (
          <Alert tone="warning">
            Participation refusée{submission.review_note && ` : ${submission.review_note}`}. Vous
            pouvez réessayer.
          </Alert>
        )}
        {submission?.reward_status === 'failed' && (
          <Alert tone="error">Le versement de vos EPC a échoué. Vous pouvez réessayer.</Alert>
        )}
        {form}
      </div>
    );
  }

  return (
    <article aria-labelledby={titleId} className="min-w-0 space-y-4 bg-white p-5 ring-1 ring-line">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <h2 id={titleId} className="display text-xl text-slate-900 uppercase">
          {challenge.title}
        </h2>
        <span className={`display text-2xl ${TONE_TEXT.ok}`}>+{formatEpc(challenge.reward)}</span>
      </div>
      <div className="flex flex-wrap gap-2">
        <Badge tone="neutral">{CHALLENGE_MODE_LABELS[challenge.mode]}</Badge>
        {challenge.closes_at && (
          <Badge tone="warning">Jusqu'au {formatDateTime(challenge.closes_at)}</Badge>
        )}
        {placesLeft !== null && challenge.open && (
          <Badge tone="info">
            {placesLeft} place{placesLeft > 1 ? 's' : ''} restante{placesLeft > 1 ? 's' : ''}
          </Badge>
        )}
        {challenge.cohorts.map((code) => (
          <Badge key={code} tone="neutral">
            {cohortLabel(code)}
          </Badge>
        ))}
      </div>
      {challenge.description && (
        <p className="text-sm whitespace-pre-line text-slate-700">{challenge.description}</p>
      )}
      {challenge.creator_name && (
        <p className="text-xs text-slate-500">Proposé par {challenge.creator_name}</p>
      )}
      {status}
    </article>
  );
}

export function ChallengesPage() {
  const challenges = useQuery({
    queryKey: ['challenges'],
    queryFn: () => api<StudentChallenge[]>('/api/challenges'),
  });
  const all = challenges.data ?? [];
  const current = all.filter((c) => c.open && !won(c));
  const past = all.filter((c) => !c.open || won(c));

  return (
    <>
      <PageHeader
        title="Défis"
        description="Réussissez un défi pour gagner des EPC : ils arrivent sur votre solde dès que le défi est validé."
      />
      <ErrorAlert error={challenges.error} />
      {challenges.isPending ? (
        <Loading />
      ) : (
        <>
          <section className="mb-10">
            <SectionTitle>Défis ouverts</SectionTitle>
            {current.length === 0 ? (
              <Empty>Aucun défi ouvert pour l'instant. Revenez bientôt !</Empty>
            ) : (
              <div className="grid gap-5 lg:grid-cols-2">
                {current.map((challenge) => (
                  <ChallengeCard key={challenge.id} challenge={challenge} />
                ))}
              </div>
            )}
          </section>
          {past.length > 0 && (
            <section>
              <SectionTitle tone="ok">Réussis et terminés</SectionTitle>
              <div className="grid gap-5 lg:grid-cols-2">
                {past.map((challenge) => (
                  <ChallengeCard key={challenge.id} challenge={challenge} />
                ))}
              </div>
            </section>
          )}
        </>
      )}
    </>
  );
}
