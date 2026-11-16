// The shopping cart: product ids and quantities, kept in this browser tab.

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from 'react';

type Lines = Record<string, number>;

interface CartContext {
  lines: Lines;
  count: number;
  /** Identifies one checkout attempt: resending the same cart with it cannot pay twice. */
  checkoutKey: string;
  setQuantity: (productId: string, quantity: number) => void;
  add: (productId: string) => void;
  clear: () => void;
  /** Starts a new attempt (after an order went through, or was definitely refused). */
  renewCheckoutKey: () => void;
}

const STORAGE_KEY = 'epicoin.cart';
const MAX_QUANTITY = 100;
const Context = createContext<CartContext | null>(null);

function load(): { lines: Lines; checkoutKey: string } {
  try {
    const stored = JSON.parse(sessionStorage.getItem(STORAGE_KEY) ?? 'null') as {
      lines?: Lines;
      checkoutKey?: string;
    } | null;
    if (stored?.lines && typeof stored.checkoutKey === 'string') {
      return { lines: stored.lines, checkoutKey: stored.checkoutKey };
    }
  } catch {
    // Unreadable storage: start with an empty cart.
  }
  return { lines: {}, checkoutKey: crypto.randomUUID() };
}

export function CartProvider({ children }: { children: ReactNode }) {
  const [state, setState] = useState(load);

  useEffect(() => {
    sessionStorage.setItem(STORAGE_KEY, JSON.stringify(state));
  }, [state]);

  const setQuantity = useCallback((productId: string, quantity: number) => {
    setState((current) => {
      const lines = { ...current.lines };
      const clamped = Math.min(Math.max(Math.trunc(quantity), 0), MAX_QUANTITY);
      if (clamped === 0) delete lines[productId];
      else lines[productId] = clamped;
      // A different cart is a different purchase.
      return { lines, checkoutKey: crypto.randomUUID() };
    });
  }, []);

  const add = useCallback((productId: string) => {
    setState((current) => ({
      lines: {
        ...current.lines,
        [productId]: Math.min((current.lines[productId] ?? 0) + 1, MAX_QUANTITY),
      },
      checkoutKey: crypto.randomUUID(),
    }));
  }, []);

  const clear = useCallback(() => {
    setState({ lines: {}, checkoutKey: crypto.randomUUID() });
  }, []);

  const renewCheckoutKey = useCallback(() => {
    setState((current) => ({ ...current, checkoutKey: crypto.randomUUID() }));
  }, []);

  const value = useMemo<CartContext>(
    () => ({
      lines: state.lines,
      count: Object.values(state.lines).reduce((sum, quantity) => sum + quantity, 0),
      checkoutKey: state.checkoutKey,
      setQuantity,
      add,
      clear,
      renewCheckoutKey,
    }),
    [state, setQuantity, add, clear, renewCheckoutKey],
  );
  return <Context.Provider value={value}>{children}</Context.Provider>;
}

export function useCart(): CartContext {
  const context = useContext(Context);
  if (!context) throw new Error('useCart must be used inside <CartProvider>');
  return context;
}
