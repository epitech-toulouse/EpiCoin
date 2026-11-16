//! Shared application state.

use std::ops::Deref;
use std::sync::Arc;

use epicoin_core::Keypair;
use epicoin_node::NodeClient;
use sqlx::PgPool;

use crate::chain::ChainService;
use crate::config::Settings;
use crate::mailer::Mailer;
use crate::ratelimit::RateLimiter;
use crate::vault::Vault;

pub struct Inner {
    pub db: PgPool,
    pub node: NodeClient,
    pub chain: ChainService,
    pub vault: Vault,
    pub mailer: Arc<dyn Mailer>,
    pub settings: Settings,
    pub limiter: RateLimiter,
    /// The chain's admin key: signs authority updates.
    pub admin_key: Keypair,
    /// The shop treasury key: signs refunds.
    pub treasury_key: Keypair,
}

/// Cheap to clone; shared by every request handler and background task.
#[derive(Clone)]
pub struct AppState(Arc<Inner>);

impl AppState {
    pub fn new(
        db: PgPool,
        node: NodeClient,
        vault: Vault,
        mailer: Arc<dyn Mailer>,
        settings: Settings,
        admin_key: Keypair,
        treasury_key: Keypair,
    ) -> Self {
        let chain = ChainService::new(
            db.clone(),
            node.clone(),
            admin_key.address(),
            treasury_key.address(),
        );
        Self(Arc::new(Inner {
            db,
            node,
            chain,
            vault,
            mailer,
            settings,
            limiter: RateLimiter::default(),
            admin_key,
            treasury_key,
        }))
    }
}

impl Deref for AppState {
    type Target = Inner;

    fn deref(&self) -> &Inner {
        &self.0
    }
}
