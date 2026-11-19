// Shapes of the API's JSON. Amounts are integers in units: 1 EPC = 100 units.

export type Role = 'student' | 'staff' | 'shop_manager' | 'admin';

export interface User {
  id: string;
  email: string;
  first_name: string;
  last_name: string;
  promo: string;
  role: Role;
  activated: boolean;
  disabled: boolean;
  mint_quota: number | null;
  created_at: string;
}

export interface AdminUser extends User {
  wallet_address: string;
}

export interface Me {
  user: User;
  csrf_token: string;
  wallet_address: string;
}

export interface Paged<T> {
  items: T[];
  total: number;
}

export interface Wallet {
  address: string;
  balance: number;
}

export type ReasonCode =
  | 'participation'
  | 'peer_help'
  | 'outstanding_project'
  | 'event'
  | 'campus_life'
  | 'challenge'
  | 'other';

export interface HistoryLine {
  tx_hash: string;
  kind: 'mint' | 'purchase' | 'refund' | 'transfer';
  status: 'included' | 'pending';
  amount: number | null;
  direction: 'credit' | 'debit';
  reason_code: ReasonCode | null;
  comment: string | null;
  /** Who granted or sent the EPC (mint, received transfer). */
  from_name: string | null;
  /** Who received the EPC (sent transfer). */
  to_name: string | null;
  order_id: string | null;
  created_at: string;
}

export interface Student {
  id: string;
  email: string;
  first_name: string;
  last_name: string;
  promo: string;
  activated: boolean;
}

export interface Cohort {
  code: string;
  /** Active students in the cohort. */
  students: number;
}

export type ChallengeMode = 'answer' | 'review';
export type RewardStatus = 'pending' | 'included' | 'failed';

export interface Challenge {
  id: string;
  title: string;
  description: string;
  /** Per winner, in units. */
  reward: number;
  mode: ChallengeMode;
  /** Empty: every student. */
  cohorts: string[];
  max_winners: number | null;
  closes_at: string | null;
  /** Takes participations right now. */
  open: boolean;
  winners: number;
  creator_name: string | null;
  created_at: string;
}

export interface StudentChallenge extends Challenge {
  submission: {
    status: 'pending' | 'approved' | 'rejected';
    proof: string;
    review_note: string;
    reward_status: RewardStatus | null;
  } | null;
}

export interface ManagedChallenge extends Challenge {
  creator_id: string | null;
  /** Closed by hand. */
  closed: boolean;
  /** Participations waiting for a review. */
  pending: number;
}

export interface ChallengeSubmission {
  id: string;
  student_id: string;
  student_name: string;
  student_email: string;
  student_promo: string;
  status: 'pending' | 'approved' | 'rejected';
  proof: string;
  review_note: string;
  reward_status: RewardStatus | null;
  submitted_at: string;
  decided_at: string | null;
}

export interface Recipient {
  id: string;
  first_name: string;
  last_name: string;
  email: string;
  promo: string;
}

export interface Transfer {
  tx_hash: string;
  status: 'included' | 'pending';
  amount: number;
  message: string;
  recipient_id: string | null;
  recipient_name: string | null;
  created_at: string;
}

export interface AdminTransfer {
  tx_hash: string;
  status: 'included' | 'pending' | 'failed';
  amount: number;
  message: string;
  sender_name: string | null;
  recipient_name: string | null;
  created_at: string;
}

export interface Quota {
  active: boolean;
  quota: number;
  minted: number;
  remaining: number;
  period_ends_at: string | null;
}

export interface Distribution {
  batch_id: string;
  status: 'included' | 'pending' | 'partial' | 'failed';
  amount: number;
  reason_code: ReasonCode;
  comment: string;
  created_at: string;
  requested: number;
  included: number;
  results: { user_id: string | null; name: string | null; tx_hash: string; status: string }[];
}

export interface DistributionLine {
  tx_hash: string;
  batch_id: string | null;
  status: 'included' | 'pending' | 'failed';
  amount: number | null;
  reason_code: ReasonCode | null;
  comment: string | null;
  recipient_id: string | null;
  recipient_name: string | null;
  created_at: string;
}

export interface Product {
  id: string;
  name: string;
  description: string;
  image_url: string | null;
  price: number;
  stock: number;
  active: boolean;
}

export type OrderStatus =
  | 'awaiting_payment'
  | 'failed'
  | 'pending'
  | 'ready'
  | 'delivered'
  | 'refunding'
  | 'cancelled';

export interface Order {
  id: string;
  status: OrderStatus;
  total: number;
  failure: string | null;
  created_at: string;
  updated_at: string;
  items: { product_id: string; name: string; unit_price: number; quantity: number }[];
  buyer?: { id: string; name: string; email: string };
}

export interface AuditEntry {
  id: number;
  at: string;
  actor_user_id: string | null;
  actor_name: string | null;
  action: string;
  target_type: string | null;
  target_id: string | null;
  details: Record<string, unknown>;
}

export interface Stats {
  supply: {
    total_minted: number;
    circulating: number;
    treasury_balance: number;
    held_by_users: number;
  };
  chain: { height: number; chain_id: string };
  minted_by_staff: { user_id: string | null; name: string | null; total: number; count: number }[];
  minted_by_reason: { reason_code: ReasonCode | null; total: number; count: number }[];
  shop: {
    totals: { orders: number; revenue: number; refunded_orders: number; refunded: number };
    by_product: { product_id: string; name: string; quantity: number; revenue: number }[];
  };
  users: { role: Role; total: number; activated: number }[];
}

// --- Blockchain explorer ---------------------------------------------------------------

export interface ChainStatus {
  chain_id: string;
  genesis_hash: string;
  mode: 'validator' | 'replica';
  height: number;
  tip_hash: string;
  tip_timestamp: number;
  state_root: string;
  mempool_size: number;
  total_minted: number;
  total_burned: number;
  circulating_supply: number;
  treasury: string;
  admin: string;
}

export interface BlockSummary {
  height: number;
  hash: string;
  prev_hash: string;
  timestamp: number;
  tx_count: number;
  validator: string;
}

export type TxPayload =
  | { kind: 'mint'; to: string; amount: number; reason: ReasonCode }
  | { kind: 'purchase'; amount: number; order_ref: string }
  | { kind: 'refund'; purchase: string }
  | {
      kind: 'authority_update';
      authority: string;
      action: { action: 'grant'; quota: number } | { action: 'revoke' };
    }
  | { kind: 'transfer'; to: string; amount: number };

export interface ChainTx {
  hash: string;
  status: 'pending' | 'included' | 'rejected' | 'unknown';
  height?: number;
  index?: number;
  timestamp?: number;
  tx: { from: string; nonce: number; payload: TxPayload } | null;
}

export interface Block extends BlockSummary {
  tx_root: string;
  state_root: string;
  signature: string;
  txs: ChainTx[];
}

export interface ChainAccount {
  address: string;
  balance: number;
  nonce: number;
  label: string | null;
  authority: { active: boolean; quota: number; minted: number; remaining: number } | null;
  transactions?: ChainTx[];
}
