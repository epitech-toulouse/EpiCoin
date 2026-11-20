// The frame around every page of a logged-in user: blue top bar, sectioned side menu.

import {
  ArrowLeftRight,
  Blocks,
  ClipboardList,
  Gift,
  History,
  LayoutDashboard,
  LogOut,
  Menu,
  Package,
  ScrollText,
  Send,
  ShoppingBag,
  ShoppingCart,
  Store,
  Trophy,
  Users,
  Wallet,
  X,
  type LucideIcon,
} from 'lucide-react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useEffect, useState, type ReactNode } from 'react';
import { Link, NavLink, Outlet, useLocation, useNavigate } from 'react-router-dom';
import { homeOf, useAuth } from '../auth';
import { useCart } from '../cart';
import { api } from '../lib/api';
import { fullName, ROLE_LABELS } from '../lib/format';
import type { NewsSection, Notifications, Role, TodoSection } from '../lib/types';
import { cx } from './ui';

interface NavItem {
  to: string;
  label: string;
  icon: LucideIcon;
  /** Colour of the icon when the item is not selected. */
  color: string;
  end?: boolean;
  /** Shows a "!" when this section has something new or waiting. */
  notify?: NewsSection | TodoSection;
}

/** The page of each news section, and the data to reload once its news is seen. */
const NEWS_PAGES: Record<string, { section: NewsSection; refresh: string[] }> = {
  '/': { section: 'balance', refresh: ['wallet', 'history'] },
  '/commandes': { section: 'orders', refresh: ['orders'] },
  '/defis': { section: 'challenges', refresh: ['challenges'] },
};

const TODO_SECTIONS: ReadonlySet<string> = new Set<TodoSection>(['shop_orders', 'challenge_reviews']);

/**
 * The counts behind the "!" of the menu, refreshed every 30 seconds. Opening the page of a
 * news section marks it as seen (and reloads its data, so that the news is on screen).
 */
function useNotifications(loggedIn: boolean): Notifications {
  const queryClient = useQueryClient();
  const { pathname } = useLocation();
  const counts = useQuery({
    queryKey: ['notifications'],
    queryFn: () => api<Notifications>('/api/me/notifications'),
    refetchInterval: 30_000,
    enabled: loggedIn,
  });
  const page = NEWS_PAGES[pathname];
  const section = page?.section;
  const hasNews = section !== undefined && (counts.data?.[section] ?? 0) > 0;
  const { mutate: markSeen } = useMutation({
    mutationFn: (seen: NewsSection) =>
      api<void>('/api/me/notifications/seen', { method: 'POST', body: { section: seen } }),
    onSuccess: (_, seen) => {
      void queryClient.invalidateQueries({ queryKey: ['notifications'] });
      const refresh = Object.values(NEWS_PAGES).find((p) => p.section === seen)?.refresh ?? [];
      for (const key of refresh) void queryClient.invalidateQueries({ queryKey: [key] });
    },
  });
  useEffect(() => {
    if (hasNews && section) markSeen(section);
  }, [hasNews, section, markSeen]);
  return counts.data ?? {};
}

/** The "!" next to a menu entry; screen readers hear what it means. */
function NotificationMark({ todo, inverted }: { todo: boolean; inverted: boolean }) {
  return (
    <span
      className={cx(
        'ml-auto inline-flex size-5 shrink-0 items-center justify-center rounded-full font-mono text-xs font-bold',
        inverted ? 'bg-white text-red-700' : 'bg-red-700 text-white',
      )}
    >
      <span aria-hidden="true">!</span>
      <span className="sr-only">{todo ? ' — à traiter' : ' — nouveau'}</span>
    </span>
  );
}

interface NavSection {
  title: string;
  items: NavItem[];
}

const DISTRIBUTION: NavSection = {
  title: 'Distribution',
  items: [
    { to: '/distribuer', label: 'Distribuer', icon: Gift, color: 'text-brand-600' },
    { to: '/distributions', label: 'Mes distributions', icon: History, color: 'text-accent' },
    {
      to: '/gestion/defis',
      label: 'Défis',
      icon: Trophy,
      color: 'text-teal',
      notify: 'challenge_reviews',
    },
  ],
};

const SHOP_MANAGEMENT: NavSection = {
  title: 'Boutique',
  items: [
    {
      to: '/gestion/commandes',
      label: 'Commandes',
      icon: ClipboardList,
      color: 'text-accent',
      notify: 'shop_orders',
    },
    { to: '/gestion/produits', label: 'Produits', icon: Package, color: 'text-teal' },
  ],
};

const MENUS: Record<Role, NavSection[]> = {
  student: [
    {
      title: 'Mon espace',
      items: [
        {
          to: '/',
          label: 'Mon solde',
          icon: Wallet,
          color: 'text-brand-600',
          end: true,
          notify: 'balance',
        },
        {
          to: '/commandes',
          label: 'Mes commandes',
          icon: ShoppingBag,
          color: 'text-accent',
          notify: 'orders',
        },
        { to: '/defis', label: 'Défis', icon: Trophy, color: 'text-ok', notify: 'challenges' },
        { to: '/envoyer', label: 'Envoyer des EPC', icon: Send, color: 'text-teal' },
      ],
    },
    {
      title: 'Boutique',
      items: [
        { to: '/boutique', label: 'Boutique', icon: Store, color: 'text-teal' },
        { to: '/panier', label: 'Panier', icon: ShoppingCart, color: 'text-ok' },
      ],
    },
  ],
  staff: [DISTRIBUTION],
  shop_manager: [SHOP_MANAGEMENT],
  admin: [
    {
      title: "Vue d'ensemble",
      items: [
        { to: '/admin', label: 'Tableau de bord', icon: LayoutDashboard, color: 'text-brand-600', end: true },
        { to: '/admin/explorateur', label: 'Blockchain', icon: Blocks, color: 'text-teal' },
      ],
    },
    {
      title: 'Administration',
      items: [
        { to: '/admin/utilisateurs', label: 'Utilisateurs', icon: Users, color: 'text-ok' },
        { to: '/admin/audit', label: 'Audit', icon: ScrollText, color: 'text-accent' },
        { to: '/admin/echanges', label: 'Échanges', icon: ArrowLeftRight, color: 'text-teal' },
      ],
    },
    DISTRIBUTION,
    SHOP_MANAGEMENT,
  ],
};

/** `{EPICOIN}`, in the spirit of the campus intranet's wordmark. */
export function Logo({ className }: { className?: string }) {
  return (
    <span className={cx('display text-2xl tracking-wide text-white', className)}>
      <span aria-hidden="true">{'{'}</span>
      EPICOIN
      <span aria-hidden="true">{'}'}</span>
    </span>
  );
}

function Navigation({
  notifications,
  onNavigate,
}: {
  notifications: Notifications;
  onNavigate?: () => void;
}) {
  const { user } = useAuth();
  const { count } = useCart();
  if (!user) return null;
  return (
    <>
      {MENUS[user.role].map((section) => (
        <div key={section.title} className="mb-6">
          <p className="mb-2 px-3 font-mono text-xs tracking-[0.18em] text-slate-500 uppercase">
            <span aria-hidden="true">&lt; </span>
            {section.title}
            <span aria-hidden="true"> /&gt;</span>
          </p>
          <ul className="space-y-0.5">
            {section.items.map(({ to, label, icon: Icon, color, end, notify }) => (
              <li key={to}>
                <NavLink
                  to={to}
                  end={end}
                  onClick={onNavigate}
                  className={({ isActive }) =>
                    cx(
                      'flex items-center gap-3 rounded-sm px-3 py-2 text-sm font-medium',
                      isActive ? 'bg-brand-600 text-white' : 'text-slate-800 hover:bg-slate-100',
                    )
                  }
                >
                  {({ isActive }) => (
                    <>
                      <Icon
                        aria-hidden="true"
                        className={cx('size-4 shrink-0', isActive ? 'text-white' : color)}
                        strokeWidth={1.75}
                      />
                      {label}
                      {notify && (notifications[notify] ?? 0) > 0 && (
                        <NotificationMark todo={TODO_SECTIONS.has(notify)} inverted={isActive} />
                      )}
                      {to === '/panier' && count > 0 && (
                        <span
                          className={cx(
                            'ml-auto px-1.5 font-mono text-xs',
                            isActive ? 'bg-white text-brand-700' : 'bg-brand-600 text-white',
                          )}
                        >
                          {count}
                          <span className="sr-only"> article(s)</span>
                        </span>
                      )}
                    </>
                  )}
                </NavLink>
              </li>
            ))}
          </ul>
        </div>
      ))}
    </>
  );
}

export function Layout() {
  const { user, signOut } = useAuth();
  const navigate = useNavigate();
  const [menuOpen, setMenuOpen] = useState(false);
  const notifications = useNotifications(user !== null && user !== undefined);
  if (!user) return null;
  const anyNotification = Object.values(notifications).some((count) => (count ?? 0) > 0);

  const logout = async () => {
    setMenuOpen(false);
    await signOut();
    navigate('/connexion');
  };

  return (
    <div className="flex min-h-dvh flex-col">
      <a
        href="#contenu"
        className="sr-only focus:not-sr-only focus:absolute focus:top-2 focus:left-2 focus:z-50 focus:bg-white focus:px-3 focus:py-2 focus:shadow"
      >
        Aller au contenu
      </a>

      <header className="sticky top-0 z-30 bg-brand-600 text-white">
        <div className="flex h-14 items-center gap-4 px-4">
          <Link to={homeOf(user.role)} aria-label="EpiCoin, accueil">
            <Logo />
          </Link>

          <div className="ml-auto hidden items-center gap-4 md:flex">
            <span className="flex items-center gap-3">
              <span
                aria-hidden="true"
                className="display flex size-8 items-center justify-center bg-white text-sm text-brand-700"
              >
                {user.first_name.charAt(0)}
                {user.last_name.charAt(0)}
              </span>
              <span className="leading-tight">
                <span className="block text-sm font-bold tracking-wide uppercase">
                  {fullName(user)}
                </span>
                <span className="block font-mono text-[0.6875rem] tracking-[0.14em] text-brand-100 uppercase">
                  {ROLE_LABELS[user.role]}
                </span>
              </span>
            </span>
            <button
              type="button"
              onClick={logout}
              className="flex items-center gap-2 rounded-sm px-3 py-2 text-sm hover:bg-white/10"
            >
              <LogOut aria-hidden="true" className="size-4" strokeWidth={1.75} />
              Se déconnecter
            </button>
          </div>

          <button
            type="button"
            className="ml-auto flex items-center gap-2 rounded-sm px-3 py-2 text-sm font-medium hover:bg-white/10 md:hidden"
            aria-expanded={menuOpen}
            aria-controls="menu-mobile"
            onClick={() => setMenuOpen((open) => !open)}
          >
            {menuOpen ? (
              <X aria-hidden="true" className="size-5" />
            ) : (
              <Menu aria-hidden="true" className="size-5" />
            )}
            Menu
            {anyNotification && !menuOpen && (
              <span className="inline-flex size-5 items-center justify-center rounded-full bg-white font-mono text-xs font-bold text-red-700">
                <span aria-hidden="true">!</span>
                <span className="sr-only"> — nouveautés</span>
              </span>
            )}
          </button>
        </div>
      </header>

      {menuOpen && (
        <nav
          id="menu-mobile"
          aria-label="Navigation principale"
          className="border-b border-line bg-white px-3 pt-5 pb-3 md:hidden"
        >
          <Navigation notifications={notifications} onNavigate={() => setMenuOpen(false)} />
          <div className="border-t border-line px-3 pt-3">
            <p className="text-sm font-medium">{fullName(user)}</p>
            <p className="label-mono mb-2 text-slate-500">{ROLE_LABELS[user.role]}</p>
            <button
              type="button"
              onClick={logout}
              className="flex items-center gap-2 py-2 text-sm font-medium text-slate-800"
            >
              <LogOut aria-hidden="true" className="size-4" strokeWidth={1.75} />
              Se déconnecter
            </button>
          </div>
        </nav>
      )}

      <div className="flex flex-1">
        <aside className="hidden w-60 shrink-0 border-r border-line bg-white md:block">
          <nav
            aria-label="Navigation principale"
            className="sticky top-14 max-h-[calc(100dvh-3.5rem)] overflow-y-auto px-3 pt-6"
          >
            <Navigation notifications={notifications} />
          </nav>
        </aside>
        <main id="contenu" className="min-w-0 flex-1 px-4 py-6 md:px-6">
          <div className="mx-auto max-w-7xl">
            <Outlet />
          </div>
        </main>
      </div>
    </div>
  );
}

/** The frame of the public pages (login, activation, password reset). */
export function PublicFrame({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="flex min-h-dvh flex-col">
      <header className="flex h-14 items-center bg-brand-600 px-4">
        <Logo />
      </header>
      <main className="flex flex-1 items-center justify-center px-4 py-10">
        <div className="w-full max-w-md border-t-2 border-brand-600 bg-white p-7 ring-1 ring-line">
          <h1 className="display mb-6 text-3xl text-brand-600 uppercase">
            {title}
            <span aria-hidden="true">_</span>
          </h1>
          {children}
        </div>
      </main>
    </div>
  );
}
