// Staff and admins: challenges, and the participations to review.

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useState, type FormEvent } from 'react';
import { useAuth } from '../../auth';
import {
  Alert,
  Badge,
  Button,
  Empty,
  ErrorAlert,
  Loading,
  Modal,
  PageHeader,
  SelectField,
  Table,
  Td,
  TextAreaField,
  TextField,
  Th,
} from '../../components/ui';
import { api } from '../../lib/api';
import {
  CHALLENGE_MODE_LABELS,
  cohortLabel,
  formatAmount,
  formatDateTime,
  formatEpc,
  parseEpc,
} from '../../lib/format';
import type {
  ChallengeMode,
  ChallengeSubmission,
  Cohort,
  ManagedChallenge,
} from '../../lib/types';

/** An ISO date as the value of a `datetime-local` input (local time, to the minute). */
function toLocalInput(iso: string | null): string {
  if (!iso) return '';
  const date = new Date(iso);
  const local = new Date(date.getTime() - date.getTimezoneOffset() * 60_000);
  return local.toISOString().slice(0, 16);
}

function useRefresh() {
  const queryClient = useQueryClient();
  return () => queryClient.invalidateQueries({ queryKey: ['manage-challenges'] });
}

// --- Create and edit -------------------------------------------------------------------

function ChallengeForm({
  challenge,
  onDone,
}: {
  challenge: ManagedChallenge | null;
  onDone: () => void;
}) {
  const refresh = useRefresh();
  const cohorts = useQuery({
    queryKey: ['promos'],
    queryFn: () => api<Cohort[]>('/api/staff/promos'),
  });
  const [title, setTitle] = useState(challenge?.title ?? '');
  const [description, setDescription] = useState(challenge?.description ?? '');
  const [rewardText, setRewardText] = useState(challenge ? formatAmount(challenge.reward) : '');
  const [mode, setMode] = useState<ChallengeMode>(challenge?.mode ?? 'answer');
  const [answer, setAnswer] = useState('');
  const [selected, setSelected] = useState<string[]>(challenge?.cohorts ?? []);
  const [maxText, setMaxText] = useState(challenge?.max_winners?.toString() ?? '');
  const initialDeadline = toLocalInput(challenge?.closes_at ?? null);
  const [deadline, setDeadline] = useState(initialDeadline);
  const [closed, setClosed] = useState(challenge?.closed ?? false);

  const reward = parseEpc(rewardText);
  const rewardInvalid = rewardText.trim() !== '' && (reward === null || reward === 0);
  const maxWinners = maxText.trim() === '' ? null : Number(maxText);
  const maxInvalid = maxWinners !== null && !(Number.isInteger(maxWinners) && maxWinners >= 1);

  const save = useMutation({
    mutationFn: () => {
      const body: Record<string, unknown> = {
        title,
        description,
        reward,
        cohorts: selected,
        max_winners: maxWinners,
      };
      // An unchanged deadline is not sent: it may already be past.
      if (!challenge || deadline !== initialDeadline) {
        body.closes_at = deadline ? new Date(deadline).toISOString() : null;
      }
      if (answer.trim() !== '') body.answer = answer;
      if (challenge) {
        body.closed = closed;
        return api<ManagedChallenge>(`/api/staff/challenges/${challenge.id}`, {
          method: 'PATCH',
          body,
        });
      }
      body.mode = mode;
      return api<ManagedChallenge>('/api/staff/challenges', { method: 'POST', body });
    },
    onSuccess: async () => {
      await refresh();
      onDone();
    },
  });

  const toggleCohort = (code: string) =>
    setSelected((current) =>
      current.includes(code) ? current.filter((c) => c !== code) : [...current, code],
    );
  const submit = (event: FormEvent) => {
    event.preventDefault();
    save.mutate();
  };

  return (
    <form onSubmit={submit} className="space-y-4">
      <ErrorAlert error={save.error} />
      <TextField
        label="Titre"
        required
        maxLength={100}
        value={title}
        onChange={(event) => setTitle(event.target.value)}
      />
      <TextAreaField
        label="Description"
        hint="Ce qu'il faut faire pour réussir. Les étudiants la lisent telle quelle."
        rows={4}
        maxLength={2000}
        value={description}
        onChange={(event) => setDescription(event.target.value)}
      />
      <div className="grid gap-3 sm:grid-cols-2">
        <TextField
          label="Gain par étudiant (EPC)"
          required
          inputMode="decimal"
          error={rewardInvalid ? 'Montant invalide.' : null}
          value={rewardText}
          onChange={(event) => setRewardText(event.target.value)}
        />
        {challenge ? (
          <TextField label="Validation" value={CHALLENGE_MODE_LABELS[challenge.mode]} disabled />
        ) : (
          <SelectField
            label="Validation"
            value={mode}
            hint={
              mode === 'answer'
                ? 'Gain versé dès que l’étudiant donne la bonne réponse.'
                : 'L’étudiant explique ; vous validez chaque participation.'
            }
            onChange={(event) => setMode(event.target.value as ChallengeMode)}
          >
            <option value="answer">{CHALLENGE_MODE_LABELS.answer}</option>
            <option value="review">{CHALLENGE_MODE_LABELS.review}</option>
          </SelectField>
        )}
      </div>
      {mode === 'answer' && (
        <TextField
          label={challenge ? 'Nouvelle réponse attendue' : 'Réponse attendue'}
          required={!challenge}
          maxLength={200}
          autoComplete="off"
          placeholder={challenge ? 'Laisser vide pour garder la réponse actuelle' : undefined}
          hint="Majuscules et espaces ne comptent pas. Elle n'est plus jamais affichée, même à vous ; 5 essais par étudiant toutes les 15 minutes."
          value={answer}
          onChange={(event) => setAnswer(event.target.value)}
        />
      )}
      <fieldset>
        <legend className="label-mono mb-1.5 text-slate-600">Cohortes</legend>
        <div className="flex flex-wrap gap-x-4 gap-y-2">
          {cohorts.data?.map((cohort) => (
            <label key={cohort.code} className="flex items-center gap-2 text-sm">
              <input
                type="checkbox"
                className="size-4"
                checked={selected.includes(cohort.code)}
                onChange={() => toggleCohort(cohort.code)}
              />
              {cohortLabel(cohort.code)}
            </label>
          ))}
        </div>
        <p className="mt-1 text-sm text-slate-600">Aucune case cochée : tous les étudiants.</p>
      </fieldset>
      <div className="grid gap-3 sm:grid-cols-2">
        <TextField
          label="Gagnants maximum"
          inputMode="numeric"
          placeholder="Sans limite"
          error={maxInvalid ? 'Un nombre entier, 1 au moins.' : null}
          value={maxText}
          onChange={(event) => setMaxText(event.target.value)}
        />
        <TextField
          label="Date limite"
          type="datetime-local"
          value={deadline}
          onChange={(event) => setDeadline(event.target.value)}
        />
      </div>
      {challenge && (
        <label className="flex items-center gap-2 text-sm">
          <input
            type="checkbox"
            className="size-4"
            checked={closed}
            onChange={(event) => setClosed(event.target.checked)}
          />
          Défi terminé (plus aucune nouvelle participation)
        </label>
      )}
      <p className="text-sm text-slate-600">
        Chaque gain est prélevé sur le quota de distribution du créateur du défi, au moment où
        l'étudiant réussit.
      </p>
      <div className="flex justify-end gap-2">
        <Button variant="secondary" onClick={onDone}>
          Annuler
        </Button>
        <Button
          type="submit"
          busy={save.isPending}
          disabled={reward === null || reward === 0 || maxInvalid}
        >
          {challenge ? 'Enregistrer' : 'Créer le défi'}
        </Button>
      </div>
    </form>
  );
}

// --- Participations --------------------------------------------------------------------

function SubmissionItem({
  challenge,
  submission,
}: {
  challenge: ManagedChallenge;
  submission: ChallengeSubmission;
}) {
  const queryClient = useQueryClient();
  const [note, setNote] = useState('');
  const decide = useMutation({
    mutationFn: (approve: boolean) =>
      api<ChallengeSubmission>(
        `/api/staff/challenges/${challenge.id}/submissions/${submission.id}`,
        { method: 'POST', body: { approve, note } },
      ),
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: ['challenge-submissions', challenge.id] });
      void queryClient.invalidateQueries({ queryKey: ['manage-challenges'] });
    },
  });

  return (
    <li className="space-y-2 py-3">
      <div className="flex flex-wrap items-center gap-2">
        <span className="font-medium text-slate-900">{submission.student_name}</span>
        <span className="text-sm text-slate-600">{submission.student_email}</span>
        {submission.student_promo && (
          <Badge tone="neutral">{cohortLabel(submission.student_promo)}</Badge>
        )}
        {submission.status === 'pending' && <Badge tone="warning">À valider</Badge>}
        {submission.status === 'rejected' && <Badge tone="error">Refusée</Badge>}
        {submission.status === 'approved' &&
          (submission.reward_status === 'failed' ? (
            <Badge tone="error">Versement échoué</Badge>
          ) : (
            <Badge tone="success">Réussi</Badge>
          ))}
        <span className="text-xs text-slate-500">{formatDateTime(submission.submitted_at)}</span>
      </div>
      {submission.proof && (
        <p className="text-sm whitespace-pre-line text-slate-700">{submission.proof}</p>
      )}
      {submission.status === 'rejected' && submission.review_note && (
        <p className="text-sm text-slate-600">Motif : {submission.review_note}</p>
      )}
      {submission.status === 'pending' && (
        <div className="space-y-2">
          <ErrorAlert error={decide.error} />
          <div className="flex flex-wrap items-end gap-2">
            <div className="min-w-48 flex-1">
              <TextField
                label={`Motif du refus pour ${submission.student_name} (facultatif)`}
                maxLength={280}
                value={note}
                onChange={(event) => setNote(event.target.value)}
              />
            </div>
            <Button
              variant="secondary"
              busy={decide.isPending && decide.variables === false}
              onClick={() => decide.mutate(false)}
            >
              Refuser
            </Button>
            <Button
              busy={decide.isPending && decide.variables === true}
              onClick={() => decide.mutate(true)}
            >
              Valider (+{formatEpc(challenge.reward)})
            </Button>
          </div>
        </div>
      )}
    </li>
  );
}

function Submissions({ challenge }: { challenge: ManagedChallenge }) {
  const submissions = useQuery({
    queryKey: ['challenge-submissions', challenge.id],
    queryFn: () =>
      api<ChallengeSubmission[]>(`/api/staff/challenges/${challenge.id}/submissions`),
  });
  if (submissions.isPending) return <Loading />;
  if (submissions.isError) return <ErrorAlert error={submissions.error} />;
  if (submissions.data.length === 0) {
    return <Empty>Personne n'a encore participé.</Empty>;
  }
  return (
    <ul className="max-h-[60vh] divide-y divide-line overflow-y-auto">
      {submissions.data.map((submission) => (
        <SubmissionItem key={submission.id} challenge={challenge} submission={submission} />
      ))}
    </ul>
  );
}

// --- Page ------------------------------------------------------------------------------

type Dialog =
  | { kind: 'create' }
  | { kind: 'edit'; challenge: ManagedChallenge }
  | { kind: 'submissions'; challenge: ManagedChallenge }
  | null;

export function ManageChallengesPage() {
  const { user } = useAuth();
  const [dialog, setDialog] = useState<Dialog>(null);
  const challenges = useQuery({
    queryKey: ['manage-challenges'],
    queryFn: () => api<ManagedChallenge[]>('/api/staff/challenges'),
  });

  return (
    <>
      <PageHeader
        title="Défis"
        description={
          user?.role === 'admin'
            ? 'Tous les défis du campus. Les gains sont versés dès qu’un étudiant réussit.'
            : 'Vos défis. Les gains sont versés dès qu’un étudiant réussit, sur votre quota.'
        }
        actions={<Button onClick={() => setDialog({ kind: 'create' })}>Nouveau défi</Button>}
      />

      <ErrorAlert error={challenges.error} />
      {challenges.isPending ? (
        <Loading />
      ) : challenges.data && challenges.data.length === 0 ? (
        <Empty>Aucun défi pour l'instant. Créez le premier !</Empty>
      ) : (
        challenges.data && (
          <Table caption="Défis">
            <thead>
              <tr>
                <Th>Défi</Th>
                <Th>Validation</Th>
                <Th className="text-right">Gain</Th>
                <Th className="text-right">Gagnants</Th>
                <Th>État</Th>
                <Th>
                  <span className="sr-only">Actions</span>
                </Th>
              </tr>
            </thead>
            <tbody className="divide-y divide-line">
              {challenges.data.map((challenge) => (
                <tr key={challenge.id}>
                  <Td>
                    <span className="font-medium">{challenge.title}</span>
                    <span className="block text-slate-600">
                      {challenge.cohorts.length > 0
                        ? challenge.cohorts.map(cohortLabel).join(', ')
                        : 'Tous les étudiants'}
                      {challenge.closes_at && ` · jusqu'au ${formatDateTime(challenge.closes_at)}`}
                    </span>
                    {user?.role === 'admin' && challenge.creator_name && (
                      <span className="block text-xs text-slate-500">
                        par {challenge.creator_name}
                      </span>
                    )}
                  </Td>
                  <Td>
                    {CHALLENGE_MODE_LABELS[challenge.mode]}
                    {challenge.pending > 0 && (
                      <span className="mt-1 block">
                        <Badge tone="warning">{challenge.pending} à valider</Badge>
                      </span>
                    )}
                  </Td>
                  <Td className="text-right whitespace-nowrap">{formatEpc(challenge.reward)}</Td>
                  <Td className="text-right">
                    {challenge.winners}
                    {challenge.max_winners !== null && ` / ${challenge.max_winners}`}
                  </Td>
                  <Td>
                    {challenge.open ? (
                      <Badge tone="success">Ouvert</Badge>
                    ) : (
                      <Badge tone="neutral">Terminé</Badge>
                    )}
                  </Td>
                  <Td className="text-right whitespace-nowrap">
                    <Button
                      variant="ghost"
                      size="sm"
                      aria-label={`Participants au défi ${challenge.title}`}
                      onClick={() => setDialog({ kind: 'submissions', challenge })}
                    >
                      Participants
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      aria-label={`Modifier le défi ${challenge.title}`}
                      onClick={() => setDialog({ kind: 'edit', challenge })}
                    >
                      Modifier
                    </Button>
                  </Td>
                </tr>
              ))}
            </tbody>
          </Table>
        )
      )}

      <Modal
        open={dialog !== null}
        title={
          dialog?.kind === 'create'
            ? 'Nouveau défi'
            : dialog?.kind === 'edit'
              ? `Modifier « ${dialog.challenge.title} »`
              : dialog
                ? `Participants · ${dialog.challenge.title}`
                : ''
        }
        onClose={() => setDialog(null)}
      >
        {dialog?.kind === 'create' && <ChallengeForm challenge={null} onDone={() => setDialog(null)} />}
        {dialog?.kind === 'edit' && (
          <ChallengeForm
            key={dialog.challenge.id}
            challenge={dialog.challenge}
            onDone={() => setDialog(null)}
          />
        )}
        {dialog?.kind === 'submissions' && (
          <div className="space-y-4">
            {dialog.challenge.mode === 'answer' && (
              <Alert tone="info">
                Les gagnants de ce défi ont donné la bonne réponse : ils ont été payés aussitôt.
              </Alert>
            )}
            <Submissions challenge={dialog.challenge} />
          </div>
        )}
      </Modal>
    </>
  );
}
