// The frame around every page of a logged-in user: navigation by role, and sign-out.

import { useState } from 'react';
import { NavLink, Outlet, useNavigate } from 'react-router-dom';
import { useAuth } from '../auth';
import { useCart } from '../cart';
import { fullName, ROLE_LABELS } from '../lib/format';
import type { Role } from '../lib/types';

interface Link {
  to: string;
  label: string;
  end?: boolean;
}

const LINKS: Record<Role, Link[]> = {
  student: [
    { to: '/', label: 'Mon solde', end: true },
    { to: '/boutique', label: 'Boutique' },
    { to: '/panier', label: 'Panier' },
    { to: '/commandes', label: 'Mes commandes' },
  ],
  staff: [
    { to: '/distribuer', label: 'Distribuer' },
    { to: '/distributions', label: 'Mes distributions' },
  ],
  shop_manager: [
    { to: '/gestion/commandes', label: 'Commandes' },
    { to: '/gestion/produits', label: 'Produits' },
  ],
  admin: [
    { to: '/admin/utilisateurs', label: 'Utilisateurs' },
    { to: '/admin/statistiques', label: 'Statistiques' },
    { to: '/admin/explorateur', label: 'Blockchain' },
    { to: '/admin/audit', label: 'Audit' },
    { to: '/gestion/commandes', label: 'Commandes' },
    { to: '/gestion/produits', label: 'Produits' },
  ],
};

export function Layout() {
  const { user, signOut } = useAuth();
  const { count } = useCart();
  const navigate = useNavigate();
  const [menuOpen, setMenuOpen] = useState(false);
  if (!user) return null;

  const links = LINKS[user.role];
  const linkClass = ({ isActive }: { isActive: boolean }) =>
    `block rounded-md px-3 py-2 text-sm font-medium ${
      isActive ? 'bg-brand-50 text-brand-700' : 'text-slate-700 hover:bg-slate-100'
    }`;

  const logout = async () => {
    await signOut();
    navigate('/connexion');
  };

  return (
    <div className="min-h-dvh">
      <a
        href="#contenu"
        className="sr-only focus:not-sr-only focus:absolute focus:top-2 focus:left-2 focus:z-50 focus:rounded-md focus:bg-white focus:px-3 focus:py-2 focus:shadow"
      >
        Aller au contenu
      </a>
      <header className="border-b border-slate-200 bg-white">
        <div className="mx-auto flex max-w-6xl items-center gap-4 px-4 py-3">
          <span className="text-lg font-bold tracking-tight text-brand-700">EpiCoin</span>

          <nav aria-label="Navigation principale" className="hidden flex-1 gap-1 md:flex">
            {links.map((link) => (
              <NavLink key={link.to} to={link.to} end={link.end} className={linkClass}>
                {link.label}
                {link.to === '/panier' && count > 0 && (
                  <span className="ml-1 rounded-full bg-brand-600 px-1.5 py-0.5 text-xs text-white">
                    {count}
                    <span className="sr-only"> article(s)</span>
                  </span>
                )}
              </NavLink>
            ))}
          </nav>

          <div className="ml-auto hidden items-center gap-3 md:flex">
            <span className="text-right text-sm leading-tight">
              <span className="block font-medium text-slate-900">{fullName(user)}</span>
              <span className="block text-slate-500">{ROLE_LABELS[user.role]}</span>
            </span>
            <button
              type="button"
              onClick={logout}
              className="rounded-md px-3 py-2 text-sm font-medium text-slate-700 hover:bg-slate-100"
            >
              Se déconnecter
            </button>
          </div>

          <button
            type="button"
            className="ml-auto rounded-md px-3 py-2 text-sm font-medium text-slate-700 hover:bg-slate-100 md:hidden"
            aria-expanded={menuOpen}
            aria-controls="menu-mobile"
            onClick={() => setMenuOpen((open) => !open)}
          >
            Menu
          </button>
        </div>

        {menuOpen && (
          <nav
            id="menu-mobile"
            aria-label="Navigation principale"
            className="space-y-1 border-t border-slate-200 px-4 py-3 md:hidden"
          >
            {links.map((link) => (
              <NavLink
                key={link.to}
                to={link.to}
                end={link.end}
                className={linkClass}
                onClick={() => setMenuOpen(false)}
              >
                {link.label}
                {link.to === '/panier' && count > 0 && ` (${count})`}
              </NavLink>
            ))}
            <p className="px-3 pt-2 text-sm text-slate-500">
              {fullName(user)} · {ROLE_LABELS[user.role]}
            </p>
            <button
              type="button"
              onClick={logout}
              className="block w-full rounded-md px-3 py-2 text-left text-sm font-medium text-slate-700 hover:bg-slate-100"
            >
              Se déconnecter
            </button>
          </nav>
        )}
      </header>

      <main id="contenu" className="mx-auto max-w-6xl px-4 py-8">
        <Outlet />
      </main>
    </div>
  );
}

/** The frame of the public pages (login, activation, password reset). */
export function PublicFrame({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <main className="mx-auto flex min-h-dvh max-w-md flex-col justify-center px-4 py-10">
      <p className="mb-6 text-center text-2xl font-bold tracking-tight text-brand-700">EpiCoin</p>
      <div className="rounded-lg bg-white p-6 shadow-sm ring-1 ring-slate-200">
        <h1 className="mb-5 text-xl font-semibold text-slate-900">{title}</h1>
        {children}
      </div>
    </main>
  );
}
