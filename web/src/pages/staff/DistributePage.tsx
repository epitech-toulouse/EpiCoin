// Staff: give EPC to one or several students.

import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Coins, Gauge, Users } from 'lucide-react';
import { useRef, useState, type ChangeEvent, type FormEvent } from 'react';
import {
  Alert,
  Badge,
  Button,
  Card,
  CardTitle,
  Empty,
  ErrorAlert,
  Loading,
  Meter,
  PageHeader,
  SelectField,
  Stat,
  TextAreaField,
  TextField,
} from '../../components/ui';
import { api, ApiError, query } from '../../lib/api';
import { extractEmails } from '../../lib/csv';
import {
  cohortLabel,
  formatDate,
  formatEpc,
  fullName,
  parseEpc,
  DISTRIBUTION_REASONS,
  REASON_LABELS,
} from '../../lib/format';
import type { Cohort, Distribution, Paged, Quota, ReasonCode, Student } from '../../lib/types';

const MAX_COMMENT = 280;

export function DistributePage() {
  const queryClient = useQueryClient();
  const [search, setSearch] = useState('');
  const [promo, setPromo] = useState('');
  const [selected, setSelected] = useState<Map<string, Student>>(new Map());
  const [amountText, setAmountText] = useState('');
  const [reason, setReason] = useState<ReasonCode>('participation');
  const [comment, setComment] = useState('');
  const [attemptKey, setAttemptKey] = useState(() => crypto.randomUUID());
  const [done, setDone] = useState<Distribution | null>(null);
  const [csvReport, setCsvReport] = useState<{ added: number; missing: string[] } | null>(null);
  const fileInput = useRef<HTMLInputElement>(null);

  const quota = useQuery({ queryKey: ['quota'], queryFn: () => api<Quota>('/api/staff/quota') });
  const cohorts = useQuery({
    queryKey: ['promos'],
    queryFn: () => api<Cohort[]>('/api/staff/promos'),
    staleTime: 300_000,
  });
  const students = useQuery({
    queryKey: ['students', search, promo],
    queryFn: () =>
      api<Paged<Student>>(`/api/staff/students${query({ q: search, promo, limit: 200 })}`),
    placeholderData: keepPreviousData,
  });

  // Any change to what is about to be sent makes it a new distribution.
  const touch = () => {
    setAttemptKey(crypto.randomUUID());
    setDone(null);
  };

  const toggle = (student: Student) => {
    setSelected((current) => {
      const next = new Map(current);
      if (next.has(student.id)) next.delete(student.id);
      else next.set(student.id, student);
      return next;
    });
    touch();
  };

  const selectAllShown = () => {
    setSelected((current) => {
      const next = new Map(current);
      for (const student of students.data?.items ?? []) next.set(student.id, student);
      return next;
    });
    touch();
  };

  const importCsv = useMutation({
    mutationFn: async (file: File) => {
      const emails = extractEmails(await file.text());
      if (emails.length === 0) {
        throw new ApiError(0, 'no_email', "Aucune adresse email n'a été trouvée dans ce fichier.", {});
      }
      return api<{ found: Student[]; missing: string[] }>('/api/staff/students/resolve', {
        method: 'POST',
        body: { emails },
      });
    },
    onSuccess: ({ found, missing }) => {
      setSelected((current) => {
        const next = new Map(current);
        for (const student of found) next.set(student.id, student);
        return next;
      });
      setCsvReport({ added: found.length, missing });
      touch();
    },
  });

  const onFile = (event: ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0];
    if (file) importCsv.mutate(file);
    event.target.value = '';
  };

  const amount = parseEpc(amountText);
  const total = amount !== null ? amount * selected.size : 0;
  const remaining = quota.data?.remaining ?? 0;
  const overQuota = quota.data !== undefined && total > remaining;
  const amountError =
    amountText !== '' && (amount === null || amount === 0)
      ? 'Indiquez un montant en EPC supérieur à zéro, par exemple 5 ou 2,50.'
      : null;

  const distribute = useMutation({
    mutationFn: () =>
      api<Distribution>('/api/staff/distributions', {
        method: 'POST',
        body: {
          recipient_ids: [...selected.keys()],
          amount,
          reason_code: reason,
          comment,
        },
        idempotencyKey: attemptKey,
      }),
    onSuccess: (result) => {
      setDone(result);
      if (result.status === 'included' || result.status === 'pending') {
        setSelected(new Map());
        setAmountText('');
        setComment('');
        setCsvReport(null);
      }
      setAttemptKey(crypto.randomUUID());
    },
    onError: (error) => {
      if (error instanceof ApiError && error.status !== 0) setAttemptKey(crypto.randomUUID());
    },
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: ['quota'] });
      void queryClient.invalidateQueries({ queryKey: ['distributions'] });
    },
  });

  const submit = (event: FormEvent) => {
    event.preventDefault();
    distribute.mutate();
  };

  const canSubmit =
    selected.size > 0 && amount !== null && amount > 0 && !overQuota && quota.data?.active === true;

  return (
    <>
      <PageHeader
        title="Distribuer des EPC"
        description="Récompensez un ou plusieurs étudiants. Chaque distribution est enregistrée sur la blockchain."
      />

      {quota.isPending ? (
        <Loading />
      ) : quota.isError ? (
        <ErrorAlert error={quota.error} />
      ) : !quota.data.active ? (
        <div className="mb-6">
          <Alert tone="warning">
            Votre autorisation de distribution n'est pas encore active sur la blockchain. Elle le
            devient en général moins d'une minute après l'attribution de votre rôle, au plus
            quelques minutes ; sinon, contactez un administrateur.
          </Alert>
        </div>
      ) : (
        <Card className="mb-8">
          <CardTitle icon={Gauge} to="/distributions">
            Quota de la période
          </CardTitle>
          <div className="mb-5 grid gap-6 sm:grid-cols-3">
            <Stat
              label="Quota restant"
              value={formatEpc(quota.data.remaining)}
              tone={quota.data.remaining > 0 ? 'ok' : 'danger'}
              size="lg"
            />
            <Stat label="Déjà distribué sur la période" value={formatEpc(quota.data.minted)} />
            <Stat
              label="Quota de la période"
              value={formatEpc(quota.data.quota)}
              tone="neutral"
              hint={
                quota.data.period_ends_at
                  ? `Remis à zéro le ${formatDate(quota.data.period_ends_at)}`
                  : undefined
              }
            />
          </div>
          <Meter
            value={quota.data.minted}
            max={quota.data.quota}
            label="Part du quota déjà distribuée"
          />
        </Card>
      )}

      <form onSubmit={submit} className="grid gap-6 lg:grid-cols-[1fr_22rem]">
        <Card>
          <CardTitle icon={Users}>1 · Choisir les étudiants</CardTitle>
          <div className="mb-4 grid gap-3 sm:grid-cols-[1fr_13rem]">
            <TextField
              label="Rechercher"
              type="search"
              placeholder="Nom, prénom ou email"
              value={search}
              onChange={(event) => setSearch(event.target.value)}
            />
            <SelectField label="Cohorte" value={promo} onChange={(event) => setPromo(event.target.value)}>
              <option value="">Toutes</option>
              {cohorts.data?.map((cohort) => (
                <option key={cohort.code} value={cohort.code}>
                  {cohortLabel(cohort.code)} ({cohort.students})
                </option>
              ))}
            </SelectField>
          </div>

          <div className="mb-3 flex flex-wrap items-center gap-2">
            <Button variant="secondary" size="sm" onClick={selectAllShown}>
              Tout sélectionner ({students.data?.items.length ?? 0})
            </Button>
            <Button
              variant="secondary"
              size="sm"
              busy={importCsv.isPending}
              onClick={() => fileInput.current?.click()}
            >
              Importer un fichier CSV
            </Button>
            <input
              ref={fileInput}
              type="file"
              accept=".csv,.txt,text/csv,text/plain"
              className="sr-only"
              aria-label="Fichier CSV contenant les adresses email des étudiants"
              onChange={onFile}
            />
            {selected.size > 0 && (
              <Button
                variant="ghost"
                size="sm"
                onClick={() => {
                  setSelected(new Map());
                  setCsvReport(null);
                  touch();
                }}
              >
                Vider la sélection
              </Button>
            )}
          </div>
          <ErrorAlert error={importCsv.error} />
          {csvReport && (
            <div className="mb-3">
              <Alert tone={csvReport.missing.length > 0 ? 'warning' : 'success'}>
                {csvReport.added} étudiant(s) ajouté(s) depuis le fichier.
                {csvReport.missing.length > 0 && (
                  <>
                    {' '}
                    Adresses inconnues ou non étudiantes : {csvReport.missing.join(', ')}.
                  </>
                )}
              </Alert>
            </div>
          )}

          <ErrorAlert error={students.error} />
          {students.isPending ? (
            <Loading />
          ) : students.data && students.data.items.length === 0 ? (
            <Empty>Aucun étudiant ne correspond à cette recherche.</Empty>
          ) : (
            <ul className="max-h-96 divide-y divide-line overflow-y-auto ring-1 ring-line">
              {students.data?.items.map((student) => (
                <li key={student.id}>
                  <label className="flex cursor-pointer items-center gap-3 px-3 py-2 hover:bg-slate-50">
                    <input
                      type="checkbox"
                      className="size-4"
                      checked={selected.has(student.id)}
                      onChange={() => toggle(student)}
                    />
                    <span className="min-w-0 flex-1">
                      <span className="block truncate font-medium text-slate-900">
                        {fullName(student)}
                      </span>
                      <span className="block truncate text-sm text-slate-600">{student.email}</span>
                    </span>
                    {student.promo && <Badge tone="neutral">{cohortLabel(student.promo)}</Badge>}
                    {!student.activated && <Badge tone="warning">Compte non activé</Badge>}
                  </label>
                </li>
              ))}
            </ul>
          )}
          {students.data && students.data.total > students.data.items.length && (
            <p className="mt-2 text-sm text-slate-600">
              {students.data.items.length} étudiants affichés sur {students.data.total} : affinez la
              recherche pour voir les autres.
            </p>
          )}
        </Card>

        <Card className="h-fit space-y-4">
          <CardTitle icon={Coins} tone="ok">
            2 · Montant et motif
          </CardTitle>
          <TextField
            label="Montant par étudiant (EPC)"
            inputMode="decimal"
            placeholder="5"
            required
            error={amountError}
            value={amountText}
            onChange={(event) => {
              setAmountText(event.target.value);
              touch();
            }}
          />
          <SelectField
            label="Motif"
            value={reason}
            onChange={(event) => {
              setReason(event.target.value as ReasonCode);
              touch();
            }}
          >
            {DISTRIBUTION_REASONS.map((code) => (
              <option key={code} value={code}>
                {REASON_LABELS[code]}
              </option>
            ))}
          </SelectField>
          <TextAreaField
            label="Commentaire (facultatif)"
            rows={3}
            maxLength={MAX_COMMENT}
            hint="Visible par l'étudiant. Il n'est pas inscrit sur la blockchain."
            value={comment}
            onChange={(event) => {
              setComment(event.target.value);
              touch();
            }}
          />

          <dl className="space-y-1 border-t border-line pt-3 text-sm">
            <div className="flex justify-between">
              <dt className="text-slate-600">Étudiants sélectionnés</dt>
              <dd className="font-medium tabular-nums" data-testid="selected-count">
                {selected.size}
              </dd>
            </div>
            <div className="flex justify-between">
              <dt className="text-slate-600">Total à distribuer</dt>
              <dd className="font-semibold tabular-nums">{formatEpc(total)}</dd>
            </div>
          </dl>

          {overQuota && (
            <Alert tone="warning">
              Ce total dépasse votre quota restant ({formatEpc(remaining)}).
            </Alert>
          )}
          <ErrorAlert error={distribute.error} />
          {done && (
            <Alert tone={done.status === 'included' ? 'success' : done.status === 'pending' ? 'info' : 'error'}>
              {done.status === 'included' &&
                `Distribution confirmée : ${formatEpc(done.amount)} pour ${done.included} étudiant(s).`}
              {done.status === 'pending' &&
                'Distribution enregistrée ; sa confirmation sur la blockchain est en cours.'}
              {(done.status === 'partial' || done.status === 'failed') &&
                `La distribution n'a pas pu être menée à terme : ${done.included} étudiant(s) crédité(s) sur ${done.requested}. Consultez « Mes distributions ».`}
            </Alert>
          )}
          <Button type="submit" className="w-full" busy={distribute.isPending} disabled={!canSubmit}>
            Distribuer
          </Button>
        </Card>
      </form>
    </>
  );
}
