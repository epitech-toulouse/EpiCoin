// Routes. Each role only reaches its own screens; the API enforces the same rules again.

import type { ReactNode } from 'react';
import { Navigate, Route, Routes, useLocation } from 'react-router-dom';
import { homeOf, useAuth } from './auth';
import { Layout } from './components/Layout';
import { Loading } from './components/ui';
import type { Role } from './lib/types';
import { AuditPage } from './pages/admin/AuditPage';
import { ExplorerAccountPage, ExplorerBlockPage, ExplorerPage } from './pages/admin/ExplorerPage';
import { DashboardPage } from './pages/admin/DashboardPage';
import { TransfersPage } from './pages/admin/TransfersPage';
import { UsersPage } from './pages/admin/UsersPage';
import { ActivationPage, ForgotPage, LoginPage, ResetPage } from './pages/auth/AuthPages';
import { ManageOrdersPage } from './pages/shop/ManageOrdersPage';
import { ManageProductsPage } from './pages/shop/ManageProductsPage';
import { DistributePage } from './pages/staff/DistributePage';
import { DistributionsPage } from './pages/staff/DistributionsPage';
import { ManageChallengesPage } from './pages/staff/ManageChallengesPage';
import { ChallengesPage } from './pages/student/ChallengesPage';
import { CartPage } from './pages/student/CartPage';
import { HomePage } from './pages/student/HomePage';
import { OrdersPage } from './pages/student/OrdersPage';
import { ShopPage } from './pages/student/ShopPage';
import { TransferPage } from './pages/student/TransferPage';

/** Shows its content only to logged-in users, and only to the given roles. */
function Protected({ roles, children }: { roles?: Role[]; children: ReactNode }) {
  const { user } = useAuth();
  const location = useLocation();
  if (user === undefined) return <Loading />;
  if (user === null) return <Navigate to="/connexion" replace state={{ from: location.pathname }} />;
  if (roles && !roles.includes(user.role)) return <Navigate to={homeOf(user.role)} replace />;
  return children;
}

/** Sends users who are already logged in to their home page. */
function PublicOnly({ children }: { children: ReactNode }) {
  const { user } = useAuth();
  if (user === undefined) return <Loading />;
  if (user) return <Navigate to={homeOf(user.role)} replace />;
  return children;
}

function Fallback() {
  const { user } = useAuth();
  if (user === undefined) return <Loading />;
  return <Navigate to={user ? homeOf(user.role) : '/connexion'} replace />;
}

const MANAGERS: Role[] = ['shop_manager', 'admin'];
const DISTRIBUTORS: Role[] = ['staff', 'admin'];

export function App() {
  return (
    <Routes>
      <Route path="/connexion" element={<PublicOnly><LoginPage /></PublicOnly>} />
      <Route path="/mot-de-passe-oublie" element={<PublicOnly><ForgotPage /></PublicOnly>} />
      {/* Reachable whoever is logged in: the link in the email decides. */}
      <Route path="/activation" element={<ActivationPage />} />
      <Route path="/reinitialisation" element={<ResetPage />} />

      <Route element={<Protected><Layout /></Protected>}>
        <Route path="/" element={<Protected roles={['student']}><HomePage /></Protected>} />
        <Route path="/boutique" element={<Protected roles={['student']}><ShopPage /></Protected>} />
        <Route path="/panier" element={<Protected roles={['student']}><CartPage /></Protected>} />
        <Route path="/commandes" element={<Protected roles={['student']}><OrdersPage /></Protected>} />
        <Route path="/defis" element={<Protected roles={['student']}><ChallengesPage /></Protected>} />
        <Route path="/envoyer" element={<Protected roles={['student']}><TransferPage /></Protected>} />

        <Route path="/distribuer" element={<Protected roles={DISTRIBUTORS}><DistributePage /></Protected>} />
        <Route path="/distributions" element={<Protected roles={DISTRIBUTORS}><DistributionsPage /></Protected>} />
        <Route path="/gestion/defis" element={<Protected roles={DISTRIBUTORS}><ManageChallengesPage /></Protected>} />

        <Route path="/gestion/commandes" element={<Protected roles={MANAGERS}><ManageOrdersPage /></Protected>} />
        <Route path="/gestion/produits" element={<Protected roles={MANAGERS}><ManageProductsPage /></Protected>} />

        <Route path="/admin/utilisateurs" element={<Protected roles={['admin']}><UsersPage /></Protected>} />
        <Route path="/admin" element={<Protected roles={['admin']}><DashboardPage /></Protected>} />
        <Route path="/admin/audit" element={<Protected roles={['admin']}><AuditPage /></Protected>} />
        <Route path="/admin/echanges" element={<Protected roles={['admin']}><TransfersPage /></Protected>} />
        <Route path="/admin/explorateur" element={<Protected roles={['admin']}><ExplorerPage /></Protected>} />
        <Route path="/admin/explorateur/blocs/:id" element={<Protected roles={['admin']}><ExplorerBlockPage /></Protected>} />
        <Route path="/admin/explorateur/comptes/:address" element={<Protected roles={['admin']}><ExplorerAccountPage /></Protected>} />
      </Route>

      <Route path="*" element={<Fallback />} />
    </Routes>
  );
}
