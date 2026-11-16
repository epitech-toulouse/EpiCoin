// Login, account activation, forgotten password, password reset.

import { useMutation, useQuery } from '@tanstack/react-query';
import { useState, type FormEvent } from 'react';
import { Link, useLocation, useNavigate } from 'react-router-dom';
import { homeOf, useAuth } from '../../auth';
import { PublicFrame } from '../../components/Layout';
import { Alert, Button, ErrorAlert, Loading, TextField } from '../../components/ui';
import { api, setCsrfToken } from '../../lib/api';
import type { User } from '../../lib/types';

const MIN_PASSWORD = 10;

/**
 * Reads the single-use token of an emailed link. It travels in the URL fragment
 * (`/activation#token=…`), which browsers never send to servers, so it stays out of logs.
 */
function useLinkToken(): string {
  const { hash } = useLocation();
  return new URLSearchParams(hash.replace(/^#/, '')).get('token') ?? '';
}

export function LoginPage() {
  const { signedIn } = useAuth();
  const navigate = useNavigate();
  const location = useLocation();
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');

  const login = useMutation({
    mutationFn: () =>
      api<{ user: User }>('/api/auth/login', { method: 'POST', body: { email, password } }),
    onSuccess: async ({ user }) => {
      await signedIn();
      const from = (location.state as { from?: string } | null)?.from;
      navigate(from && from !== '/connexion' ? from : homeOf(user.role), { replace: true });
    },
  });

  const submit = (event: FormEvent) => {
    event.preventDefault();
    login.mutate();
  };

  return (
    <PublicFrame title="Connexion">
      <form onSubmit={submit} className="space-y-4">
        <ErrorAlert error={login.error} />
        <TextField
          label="Adresse email"
          type="email"
          autoComplete="username"
          required
          value={email}
          onChange={(event) => setEmail(event.target.value)}
        />
        <TextField
          label="Mot de passe"
          type="password"
          autoComplete="current-password"
          required
          value={password}
          onChange={(event) => setPassword(event.target.value)}
        />
        <Button type="submit" busy={login.isPending} className="w-full">
          Se connecter
        </Button>
        <p className="text-center text-sm">
          <Link to="/mot-de-passe-oublie" className="text-brand-700 underline">
            Mot de passe oublié ?
          </Link>
        </p>
      </form>
    </PublicFrame>
  );
}

/** Two password inputs that must match; returns the chosen password once valid. */
function NewPasswordFields({
  password,
  confirmation,
  onPassword,
  onConfirmation,
}: {
  password: string;
  confirmation: string;
  onPassword: (value: string) => void;
  onConfirmation: (value: string) => void;
}) {
  const mismatch = confirmation.length > 0 && confirmation !== password;
  return (
    <>
      <TextField
        label="Nouveau mot de passe"
        type="password"
        autoComplete="new-password"
        required
        minLength={MIN_PASSWORD}
        hint={`Au moins ${MIN_PASSWORD} caractères. Une phrase facile à retenir fait très bien l'affaire.`}
        value={password}
        onChange={(event) => onPassword(event.target.value)}
      />
      <TextField
        label="Confirmez le mot de passe"
        type="password"
        autoComplete="new-password"
        required
        error={mismatch ? 'Les deux mots de passe sont différents.' : null}
        value={confirmation}
        onChange={(event) => onConfirmation(event.target.value)}
      />
    </>
  );
}

export function ActivationPage() {
  const token = useLinkToken();
  const { signedIn, user: current, signOut } = useAuth();
  const navigate = useNavigate();
  const [password, setPassword] = useState('');
  const [confirmation, setConfirmation] = useState('');

  const invitation = useQuery({
    queryKey: ['invitation', token],
    queryFn: () =>
      api<{ email: string; first_name: string }>('/api/auth/invitation', {
        method: 'POST',
        body: { token },
      }),
    enabled: token !== '',
    retry: false,
    staleTime: Infinity,
  });

  const activate = useMutation({
    mutationFn: async () => {
      // Someone else's session in this browser must not survive the activation.
      if (current) {
        await signOut();
        setCsrfToken(null);
      }
      return api<{ user: User }>('/api/auth/activate', {
        method: 'POST',
        body: { token, password },
      });
    },
    onSuccess: async ({ user }) => {
      await signedIn();
      navigate(homeOf(user.role), { replace: true });
    },
  });

  if (token === '' || invitation.isError) {
    return (
      <PublicFrame title="Lien invalide">
        <div className="space-y-4">
          <Alert tone="error">
            Ce lien d'activation est invalide ou a expiré. Demandez à un administrateur de vous
            renvoyer une invitation.
          </Alert>
          <Link to="/connexion" className="text-sm text-brand-700 underline">
            Retour à la connexion
          </Link>
        </div>
      </PublicFrame>
    );
  }
  if (invitation.isPending) {
    return (
      <PublicFrame title="Activation du compte">
        <Loading />
      </PublicFrame>
    );
  }

  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (password === confirmation) activate.mutate();
  };

  return (
    <PublicFrame title={`Bienvenue ${invitation.data.first_name} !`}>
      <form onSubmit={submit} className="space-y-4">
        <p className="text-sm text-slate-600">
          Choisissez le mot de passe de votre compte <strong>{invitation.data.email}</strong>.
        </p>
        <ErrorAlert error={activate.error} />
        <NewPasswordFields
          password={password}
          confirmation={confirmation}
          onPassword={setPassword}
          onConfirmation={setConfirmation}
        />
        <Button
          type="submit"
          busy={activate.isPending}
          disabled={password !== confirmation}
          className="w-full"
        >
          Activer mon compte
        </Button>
      </form>
    </PublicFrame>
  );
}

export function ForgotPage() {
  const [email, setEmail] = useState('');
  const forgot = useMutation({
    mutationFn: () => api<void>('/api/auth/forgot', { method: 'POST', body: { email } }),
  });

  const submit = (event: FormEvent) => {
    event.preventDefault();
    forgot.mutate();
  };

  return (
    <PublicFrame title="Mot de passe oublié">
      {forgot.isSuccess ? (
        <div className="space-y-4">
          <Alert tone="success">
            Si un compte existe pour cette adresse, un email vient de lui être envoyé avec un lien
            pour choisir un nouveau mot de passe.
          </Alert>
          <Link to="/connexion" className="text-sm text-brand-700 underline">
            Retour à la connexion
          </Link>
        </div>
      ) : (
        <form onSubmit={submit} className="space-y-4">
          <ErrorAlert error={forgot.error} />
          <TextField
            label="Adresse email"
            type="email"
            autoComplete="username"
            required
            value={email}
            onChange={(event) => setEmail(event.target.value)}
          />
          <Button type="submit" busy={forgot.isPending} className="w-full">
            Recevoir un lien
          </Button>
          <p className="text-center text-sm">
            <Link to="/connexion" className="text-brand-700 underline">
              Retour à la connexion
            </Link>
          </p>
        </form>
      )}
    </PublicFrame>
  );
}

export function ResetPage() {
  const token = useLinkToken();
  const [password, setPassword] = useState('');
  const [confirmation, setConfirmation] = useState('');
  const reset = useMutation({
    mutationFn: () =>
      api<void>('/api/auth/reset', { method: 'POST', body: { token, password } }),
  });

  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (password === confirmation) reset.mutate();
  };

  if (reset.isSuccess) {
    return (
      <PublicFrame title="Mot de passe modifié">
        <div className="space-y-4">
          <Alert tone="success">Votre mot de passe a été modifié.</Alert>
          <Link to="/connexion" className="text-sm text-brand-700 underline">
            Se connecter
          </Link>
        </div>
      </PublicFrame>
    );
  }

  return (
    <PublicFrame title="Nouveau mot de passe">
      <form onSubmit={submit} className="space-y-4">
        {token === '' && (
          <Alert tone="error">Ce lien est incomplet. Ouvrez le lien reçu par email.</Alert>
        )}
        <ErrorAlert error={reset.error} />
        <NewPasswordFields
          password={password}
          confirmation={confirmation}
          onPassword={setPassword}
          onConfirmation={setConfirmation}
        />
        <Button
          type="submit"
          busy={reset.isPending}
          disabled={token === '' || password !== confirmation}
          className="w-full"
        >
          Enregistrer
        </Button>
      </form>
    </PublicFrame>
  );
}
