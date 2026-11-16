// Who is logged in, shared with the whole application.

import { useQuery, useQueryClient, type QueryClient } from '@tanstack/react-query';
import { createContext, useCallback, useContext, useMemo, type ReactNode } from 'react';
import { api, ApiError, setCsrfToken } from './lib/api';
import type { Me, Role, User } from './lib/types';

interface AuthContext {
  /** `undefined` while the session is being checked, `null` when logged out. */
  user: User | null | undefined;
  walletAddress: string | null;
  /** Call after a login or an activation, with what the API answered. */
  signedIn: () => Promise<void>;
  signOut: () => Promise<void>;
}

const Context = createContext<AuthContext | null>(null);

/**
 * Forgets the session in the page: the user becomes "logged out" (which sends them to the
 * login screen) and nothing of their data stays in memory.
 */
export function forgetSession(queryClient: QueryClient): void {
  setCsrfToken(null);
  // Update `me` in place: its observer must see the change, so it is not removed.
  queryClient.setQueryData(['me'], null);
  queryClient.removeQueries({ predicate: (query) => query.queryKey[0] !== 'me' });
}

async function fetchMe(): Promise<Me | null> {
  try {
    return await api<Me>('/api/auth/me');
  } catch (error) {
    if (error instanceof ApiError && error.status === 401) return null;
    throw error;
  }
}

export function AuthProvider({ children }: { children: ReactNode }) {
  const queryClient = useQueryClient();
  const me = useQuery({ queryKey: ['me'], queryFn: fetchMe, staleTime: 60_000 });

  const signedIn = useCallback(async () => {
    await queryClient.invalidateQueries({ queryKey: ['me'] });
  }, [queryClient]);

  const signOut = useCallback(async () => {
    try {
      await api('/api/auth/logout', { method: 'POST' });
    } finally {
      forgetSession(queryClient);
    }
  }, [queryClient]);

  const value = useMemo<AuthContext>(
    () => ({
      user: me.isPending ? undefined : (me.data?.user ?? null),
      walletAddress: me.data?.wallet_address ?? null,
      signedIn,
      signOut,
    }),
    [me.isPending, me.data, signedIn, signOut],
  );
  return <Context.Provider value={value}>{children}</Context.Provider>;
}

export function useAuth(): AuthContext {
  const context = useContext(Context);
  if (!context) throw new Error('useAuth must be used inside <AuthProvider>');
  return context;
}

/** Where each role lands after logging in. */
export function homeOf(role: Role): string {
  switch (role) {
    case 'student':
      return '/';
    case 'staff':
      return '/distribuer';
    case 'shop_manager':
      return '/gestion/commandes';
    case 'admin':
      return '/admin/utilisateurs';
  }
}
