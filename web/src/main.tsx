import { MutationCache, QueryCache, QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { BrowserRouter } from 'react-router-dom';
import { App } from './App';
import { AuthProvider, forgetSession } from './auth';
import { CartProvider } from './cart';
import { ApiError } from './lib/api';
import './index.css';

/** A session that expired or was ended elsewhere: back to the login screen. */
function onError(error: unknown) {
  if (error instanceof ApiError && error.status === 401 && error.code === 'unauthorized') {
    forgetSession(queryClient);
  }
}

const queryClient: QueryClient = new QueryClient({
  queryCache: new QueryCache({ onError }),
  mutationCache: new MutationCache({ onError }),
  defaultOptions: {
    queries: {
      // Balances, stock and orders are changed by other people: every screen reloads its
      // data when it is displayed (cached data shows instantly in the meantime).
      staleTime: 0,
      refetchOnWindowFocus: true,
      // Client errors will not fix themselves; only retry what may be transient.
      retry: (failures, error) =>
        failures < 2 && !(error instanceof ApiError && error.status >= 400 && error.status < 500),
    },
  },
});

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <BrowserRouter>
        <AuthProvider>
          <CartProvider>
            <App />
          </CartProvider>
        </AuthProvider>
      </BrowserRouter>
    </QueryClientProvider>
  </StrictMode>,
);
