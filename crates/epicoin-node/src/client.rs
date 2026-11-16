//! HTTP client for the node API, used by replicas, the CLI and `epicoin-api`.

use std::time::Duration;

use epicoin_core::{Address, Block, Genesis, Hash, Transaction, TxError};
use reqwest::{Response, StatusCode};
use serde::de::DeserializeOwned;

use crate::views::{
    AccountView, AuthorityView, BlockSummary, BlockView, ErrorView, StatusView, SubmitView,
    TxStatus, TxView,
};

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// The state machine refused the transaction; retrying the same bytes cannot succeed.
    #[error("transaction rejected: {0}")]
    Rejected(TxError),
    /// The node answered with an error.
    #[error("node error {status} ({code}): {message}")]
    Api {
        status: u16,
        code: String,
        message: String,
    },
    /// The node could not be reached or the request timed out; the outcome is unknown.
    #[error("cannot reach the node: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("unexpected response from the node: {0}")]
    Decode(String),
}

#[derive(Debug, Clone)]
pub struct NodeClient {
    http: reqwest::Client,
    base: String,
}

impl NodeClient {
    /// `base_url` is e.g. `http://node:7000`.
    pub fn new(base_url: &str) -> Result<Self, ClientError> {
        let http = reqwest::Client::builder()
            // Longer than the node's maximum inclusion wait.
            .timeout(Duration::from_secs(40))
            .connect_timeout(Duration::from_secs(5))
            .build()?;
        Ok(Self {
            http,
            base: base_url.trim_end_matches('/').to_owned(),
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, ClientError> {
        let response = self.http.get(self.url(path)).send().await?;
        json(check(response).await?).await
    }

    /// Like [`Self::get`], but maps a 404 to `None`.
    async fn get_optional<T: DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<Option<T>, ClientError> {
        let response = self.http.get(self.url(path)).send().await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        Ok(Some(json(check(response).await?).await?))
    }

    pub async fn health(&self) -> Result<(), ClientError> {
        let response = self.http.get(self.url("/health")).send().await?;
        check(response).await.map(|_| ())
    }

    pub async fn status(&self) -> Result<StatusView, ClientError> {
        self.get("/status").await
    }

    pub async fn genesis(&self) -> Result<Genesis, ClientError> {
        self.get("/genesis").await
    }

    /// Submits a signed transaction. With `wait`, the call returns once the transaction is
    /// in a block (or the wait elapses, in which case the status is still `Pending`).
    ///
    /// Safe to retry with the same transaction: the node answers with its current status.
    /// A transaction refused by the state machine — at admission or later — is reported as
    /// [`ClientError::Rejected`].
    pub async fn submit(
        &self,
        tx: &Transaction,
        wait: Option<Duration>,
    ) -> Result<SubmitView, ClientError> {
        let mut request = self
            .http
            .post(self.url("/tx"))
            .header("content-type", "application/octet-stream")
            .body(tx.to_bytes());
        if let Some(wait) = wait {
            request = request.query(&[("wait_ms", wait.as_millis() as u64)]);
        }
        let view: SubmitView = json(check(request.send().await?).await?).await?;
        match view.status {
            TxStatus::Rejected { error } => Err(ClientError::Rejected(error)),
            _ => Ok(view),
        }
    }

    pub async fn tx(&self, hash: &Hash) -> Result<Option<TxView>, ClientError> {
        self.get_optional(&format!("/tx/{hash}")).await
    }

    pub async fn recent_txs(&self, limit: usize) -> Result<Vec<TxView>, ClientError> {
        self.get(&format!("/txs?limit={limit}")).await
    }

    pub async fn account(&self, address: &Address) -> Result<AccountView, ClientError> {
        self.get(&format!("/accounts/{address}")).await
    }

    pub async fn accounts(&self) -> Result<Vec<AccountView>, ClientError> {
        self.get("/accounts").await
    }

    pub async fn account_txs(
        &self,
        address: &Address,
        limit: usize,
    ) -> Result<Vec<TxView>, ClientError> {
        self.get(&format!("/accounts/{address}/txs?limit={limit}"))
            .await
    }

    pub async fn authorities(&self) -> Result<Vec<AuthorityView>, ClientError> {
        self.get("/authorities").await
    }

    /// Block summaries, newest first; `before` pages backwards.
    pub async fn blocks(
        &self,
        before: Option<u64>,
        limit: usize,
    ) -> Result<Vec<BlockSummary>, ClientError> {
        let cursor = before.map_or(String::new(), |b| format!("&before={b}"));
        self.get(&format!("/blocks?limit={limit}{cursor}")).await
    }

    /// A block by height or by hash.
    pub async fn block(&self, id: &str) -> Result<Option<BlockView>, ClientError> {
        self.get_optional(&format!("/blocks/{id}")).await
    }

    /// Raw blocks in ascending order from height `from`, for verification and replication.
    pub async fn sync_blocks(&self, from: u64, limit: usize) -> Result<Vec<Block>, ClientError> {
        let response = self
            .http
            .get(self.url(&format!("/sync/blocks?from={from}&limit={limit}")))
            .send()
            .await?;
        let bytes = check(response).await?.bytes().await?;
        borsh::from_slice(&bytes).map_err(|e| ClientError::Decode(e.to_string()))
    }
}

/// Turns an error response into a [`ClientError`].
async fn check(response: Response) -> Result<Response, ClientError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body = response.text().await.unwrap_or_default();
    match serde_json::from_str::<ErrorView>(&body) {
        Ok(ErrorView {
            detail: Some(error),
            ..
        }) => Err(ClientError::Rejected(error)),
        Ok(view) => Err(ClientError::Api {
            status: status.as_u16(),
            code: view.error,
            message: view.message,
        }),
        Err(_) => Err(ClientError::Api {
            status: status.as_u16(),
            code: "unknown".into(),
            message: body.chars().take(200).collect(),
        }),
    }
}

async fn json<T: DeserializeOwned>(response: Response) -> Result<T, ClientError> {
    let bytes = response.bytes().await?;
    serde_json::from_slice(&bytes).map_err(|e| ClientError::Decode(e.to_string()))
}
