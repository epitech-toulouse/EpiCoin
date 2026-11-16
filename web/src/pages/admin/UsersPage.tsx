// Admin: user accounts.

import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useState, type FormEvent } from 'react';
import {
  Alert,
  Badge,
  Button,
  Empty,
  ErrorAlert,
  Loading,
  Modal,
  PageHeader,
  Pagination,
  SelectField,
  Table,
  Td,
  TextAreaField,
  TextField,
  Th,
} from '../../components/ui';
import { useAuth } from '../../auth';
import { api, ApiError, query } from '../../lib/api';
import { formatAmount, formatEpc, fullName, parseEpc, ROLE_LABELS } from '../../lib/format';
import type { AdminUser, Paged, Role } from '../../lib/types';

const PAGE = 50;
const ROLES = Object.keys(ROLE_LABELS) as Role[];

function useRefreshUsers() {
  const queryClient = useQueryClient();
  return () => queryClient.invalidateQueries({ queryKey: ['users'] });
}

// --- Import ----------------------------------------------------------------------------

interface ImportSummary {
  created: number;
  skipped_existing: number;
}

function ImportForm({ onDone }: { onDone: () => void }) {
  const refresh = useRefreshUsers();
  const [csv, setCsv] = useState('');
  const run = useMutation({
    mutationFn: () =>
      api<ImportSummary>('/api/admin/users/import', { method: 'POST', body: { csv } }),
    onSuccess: refresh,
  });
  const lineErrors =
    run.error instanceof ApiError
      ? ((run.error.details.errors as { line: number; message: string }[] | undefined) ?? [])
      : [];

  const submit = (event: FormEvent) => {
    event.preventDefault();
    run.mutate();
  };

  if (run.isSuccess) {
    return (
      <div className="space-y-4">
        <Alert tone="success">
          {run.data.created} compte(s) créé(s)
          {run.data.skipped_existing > 0 &&
            `, ${run.data.skipped_existing} déjà existant(s) ignoré(s)`}
          . Les invitations partent par email.
        </Alert>
        <div className="flex justify-end">
          <Button onClick={onDone}>Terminé</Button>
        </div>
      </div>
    );
  }

  return (
    <form onSubmit={submit} className="space-y-4">
      <p className="text-sm text-slate-600">
        Colonnes attendues : <code>email</code>, <code>prenom</code>, <code>nom</code>, et en
        option <code>promo</code> et <code>role</code> (student, staff, shop_manager, admin ;
        étudiant par défaut). Séparateur virgule ou point-virgule.
      </p>
      <TextField
        label="Fichier CSV"
        type="file"
        accept=".csv,text/csv"
        onChange={async (event) => {
          const file = event.target.files?.[0];
          if (file) setCsv(await file.text());
        }}
      />
      <TextAreaField
        label="Ou collez son contenu"
        rows={6}
        className="font-mono"
        placeholder={'email;prenom;nom;promo;role\nprenom.nom@epitech.eu;Prénom;Nom;2027;student'}
        value={csv}
        onChange={(event) => setCsv(event.target.value)}
      />
      <ErrorAlert error={run.error} />
      {lineErrors.length > 0 && (
        <ul className="max-h-40 list-disc overflow-y-auto pl-5 text-sm text-red-800">
          {lineErrors.map((error) => (
            <li key={`${error.line}-${error.message}`}>
              {error.line > 0 && `Ligne ${error.line} : `}
              {error.message}
            </li>
          ))}
        </ul>
      )}
      <div className="flex justify-end gap-2">
        <Button variant="secondary" onClick={onDone}>
          Annuler
        </Button>
        <Button type="submit" busy={run.isPending} disabled={csv.trim() === ''}>
          Importer
        </Button>
      </div>
    </form>
  );
}

// --- Create ----------------------------------------------------------------------------

function CreateForm({ onDone }: { onDone: () => void }) {
  const refresh = useRefreshUsers();
  const [form, setForm] = useState({
    email: '',
    first_name: '',
    last_name: '',
    promo: '',
    role: 'student' as Role,
  });
  const create = useMutation({
    mutationFn: () => api<AdminUser>('/api/admin/users', { method: 'POST', body: form }),
    onSuccess: async () => {
      await refresh();
      onDone();
    },
  });
  const set = (field: keyof typeof form) => (event: { target: { value: string } }) =>
    setForm((current) => ({ ...current, [field]: event.target.value }));

  const submit = (event: FormEvent) => {
    event.preventDefault();
    create.mutate();
  };

  return (
    <form onSubmit={submit} className="space-y-4">
      <ErrorAlert error={create.error} />
      <TextField label="Adresse email" type="email" required value={form.email} onChange={set('email')} />
      <div className="grid grid-cols-2 gap-3">
        <TextField label="Prénom" required value={form.first_name} onChange={set('first_name')} />
        <TextField label="Nom" required value={form.last_name} onChange={set('last_name')} />
      </div>
      <div className="grid grid-cols-2 gap-3">
        <TextField label="Promo" value={form.promo} onChange={set('promo')} />
        <SelectField label="Rôle" value={form.role} onChange={set('role')}>
          {ROLES.map((role) => (
            <option key={role} value={role}>
              {ROLE_LABELS[role]}
            </option>
          ))}
        </SelectField>
      </div>
      <p className="text-sm text-slate-600">Une invitation lui sera envoyée par email.</p>
      <div className="flex justify-end gap-2">
        <Button variant="secondary" onClick={onDone}>
          Annuler
        </Button>
        <Button type="submit" busy={create.isPending}>
          Créer le compte
        </Button>
      </div>
    </form>
  );
}

// --- Edit ------------------------------------------------------------------------------

function EditForm({ user, onDone }: { user: AdminUser; onDone: () => void }) {
  const { user: me } = useAuth();
  const refresh = useRefreshUsers();
  const [role, setRole] = useState<Role>(user.role);
  const [quotaText, setQuotaText] = useState(
    user.mint_quota === null ? '' : formatAmount(user.mint_quota),
  );
  const [disabled, setDisabled] = useState(user.disabled);
  const isSelf = me?.id === user.id;

  const quota = quotaText.trim() === '' ? null : parseEpc(quotaText);
  const quotaInvalid = quotaText.trim() !== '' && quota === null;

  const done = async () => {
    await refresh();
    onDone();
  };
  const save = useMutation({
    mutationFn: () =>
      api<AdminUser>(`/api/admin/users/${user.id}`, {
        method: 'PATCH',
        body: { role, disabled, mint_quota: role === 'staff' ? quota : null },
      }),
    onSuccess: done,
  });
  const invite = useMutation({
    mutationFn: () => api<void>(`/api/admin/users/${user.id}/invitation`, { method: 'POST' }),
  });
  const remove = useMutation({
    mutationFn: () => api<void>(`/api/admin/users/${user.id}`, { method: 'DELETE' }),
    onSuccess: done,
  });

  const submit = (event: FormEvent) => {
    event.preventDefault();
    save.mutate();
  };

  return (
    <form onSubmit={submit} className="space-y-4">
      <p className="text-sm text-slate-600">{user.email}</p>
      <ErrorAlert error={save.error} />
      <ErrorAlert error={invite.error} />
      <ErrorAlert error={remove.error} />
      {invite.isSuccess && <Alert tone="success">Invitation renvoyée.</Alert>}

      <SelectField
        label="Rôle"
        value={role}
        disabled={isSelf}
        hint={isSelf ? 'Vous ne pouvez pas modifier votre propre rôle.' : undefined}
        onChange={(event) => setRole(event.target.value as Role)}
      >
        {ROLES.map((value) => (
          <option key={value} value={value}>
            {ROLE_LABELS[value]}
          </option>
        ))}
      </SelectField>
      {role === 'staff' && (
        <TextField
          label="Quota de distribution par période (EPC)"
          inputMode="decimal"
          placeholder="Quota par défaut"
          hint="Laissez vide pour appliquer le quota par défaut."
          error={quotaInvalid ? 'Montant invalide.' : null}
          value={quotaText}
          onChange={(event) => setQuotaText(event.target.value)}
        />
      )}
      {!isSelf && (
        <label className="flex items-center gap-2 text-sm">
          <input
            type="checkbox"
            className="size-4"
            checked={disabled}
            onChange={(event) => setDisabled(event.target.checked)}
          />
          Compte désactivé (connexion impossible)
        </label>
      )}

      <div className="flex flex-wrap justify-between gap-2 border-t border-line pt-4">
        <div className="flex flex-wrap gap-2">
          {!user.activated && (
            <Button variant="secondary" size="sm" busy={invite.isPending} onClick={() => invite.mutate()}>
              Renvoyer l'invitation
            </Button>
          )}
          {!isSelf && (
            <Button
              variant="danger"
              size="sm"
              busy={remove.isPending}
              onClick={() => {
                if (
                  window.confirm(
                    `Supprimer définitivement ${fullName(user)} ? Ses données personnelles et la clé de son portefeuille seront effacées ; les EPC qu'il lui reste seront perdus.`,
                  )
                ) {
                  remove.mutate();
                }
              }}
            >
              Supprimer
            </Button>
          )}
        </div>
        <div className="flex gap-2">
          <Button variant="secondary" onClick={onDone}>
            Annuler
          </Button>
          <Button type="submit" busy={save.isPending} disabled={quotaInvalid}>
            Enregistrer
          </Button>
        </div>
      </div>
    </form>
  );
}

// --- Page ------------------------------------------------------------------------------

type Dialog = { kind: 'import' } | { kind: 'create' } | { kind: 'edit'; user: AdminUser } | null;

export function UsersPage() {
  const [search, setSearch] = useState('');
  const [role, setRole] = useState<Role | ''>('');
  const [offset, setOffset] = useState(0);
  const [dialog, setDialog] = useState<Dialog>(null);

  const users = useQuery({
    queryKey: ['users', search, role, offset],
    queryFn: () =>
      api<Paged<AdminUser>>(`/api/admin/users${query({ q: search, role, limit: PAGE, offset })}`),
    placeholderData: keepPreviousData,
  });

  return (
    <>
      <PageHeader
        title="Utilisateurs"
        actions={
          <>
            <Button variant="secondary" onClick={() => setDialog({ kind: 'create' })}>
              Ajouter un utilisateur
            </Button>
            <Button onClick={() => setDialog({ kind: 'import' })}>Importer un CSV</Button>
          </>
        }
      />

      <div className="mb-4 grid gap-3 sm:grid-cols-[1fr_12rem]">
        <TextField
          label="Rechercher"
          type="search"
          placeholder="Nom, email ou promo"
          value={search}
          onChange={(event) => {
            setSearch(event.target.value);
            setOffset(0);
          }}
        />
        <SelectField
          label="Rôle"
          value={role}
          onChange={(event) => {
            setRole(event.target.value as Role | '');
            setOffset(0);
          }}
        >
          <option value="">Tous</option>
          {ROLES.map((value) => (
            <option key={value} value={value}>
              {ROLE_LABELS[value]}
            </option>
          ))}
        </SelectField>
      </div>

      <ErrorAlert error={users.error} />
      {users.isPending ? (
        <Loading />
      ) : users.data && users.data.items.length === 0 ? (
        <Empty>Aucun utilisateur ne correspond.</Empty>
      ) : (
        users.data && (
          <>
            <Table caption="Utilisateurs">
              <thead>
                <tr>
                  <Th>Nom</Th>
                  <Th>Promo</Th>
                  <Th>Rôle</Th>
                  <Th>État</Th>
                  <Th>
                    <span className="sr-only">Actions</span>
                  </Th>
                </tr>
              </thead>
              <tbody className="divide-y divide-line">
                {users.data.items.map((user) => (
                  <tr key={user.id}>
                    <Td>
                      <span className="font-medium">{fullName(user)}</span>
                      <span className="block text-slate-600">{user.email}</span>
                    </Td>
                    <Td>{user.promo}</Td>
                    <Td>
                      {ROLE_LABELS[user.role]}
                      {user.role === 'staff' && user.mint_quota !== null && (
                        <span className="block text-slate-600">
                          Quota : {formatEpc(user.mint_quota)}
                        </span>
                      )}
                    </Td>
                    <Td>
                      {user.disabled ? (
                        <Badge tone="error">Désactivé</Badge>
                      ) : user.activated ? (
                        <Badge tone="success">Actif</Badge>
                      ) : (
                        <Badge tone="warning">Invitation envoyée</Badge>
                      )}
                    </Td>
                    <Td className="text-right">
                      <Button
                        variant="ghost"
                        size="sm"
                        aria-label={`Modifier ${fullName(user)}`}
                        onClick={() => setDialog({ kind: 'edit', user })}
                      >
                        Modifier
                      </Button>
                    </Td>
                  </tr>
                ))}
              </tbody>
            </Table>
            <Pagination offset={offset} limit={PAGE} total={users.data.total} onChange={setOffset} />
          </>
        )
      )}

      <Modal
        open={dialog !== null}
        title={
          dialog?.kind === 'import'
            ? 'Importer des utilisateurs'
            : dialog?.kind === 'create'
              ? 'Ajouter un utilisateur'
              : dialog
                ? fullName(dialog.user)
                : ''
        }
        onClose={() => setDialog(null)}
      >
        {dialog?.kind === 'import' && <ImportForm onDone={() => setDialog(null)} />}
        {dialog?.kind === 'create' && <CreateForm onDone={() => setDialog(null)} />}
        {dialog?.kind === 'edit' && (
          <EditForm key={dialog.user.id} user={dialog.user} onDone={() => setDialog(null)} />
        )}
      </Modal>
    </>
  );
}
