//! Outgoing email, behind a trait: SMTP in production, the console in development, memory
//! in tests.

use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;

use lettre::message::Mailbox;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mail {
    pub to: String,
    pub subject: String,
    /// Plain text.
    pub body: String,
}

#[derive(Debug, thiserror::Error)]
pub enum MailError {
    #[error("invalid address: {0}")]
    Address(String),
    #[error("cannot build the message: {0}")]
    Build(String),
    #[error("cannot send the message: {0}")]
    Transport(String),
}

pub type MailFuture<'a> = Pin<Box<dyn Future<Output = Result<(), MailError>> + Send + 'a>>;

pub trait Mailer: Send + Sync {
    fn send<'a>(&'a self, mail: &'a Mail) -> MailFuture<'a>;
}

/// Development mailer: writes the message to the log instead of sending it.
#[derive(Debug, Default)]
pub struct ConsoleMailer;

impl Mailer for ConsoleMailer {
    fn send<'a>(&'a self, mail: &'a Mail) -> MailFuture<'a> {
        Box::pin(async move {
            tracing::info!(
                to = %mail.to,
                subject = %mail.subject,
                "email (console mailer, not sent):\n{}",
                mail.body
            );
            Ok(())
        })
    }
}

/// Test mailer: keeps every message in memory.
#[derive(Debug, Default)]
pub struct MemoryMailer {
    sent: Mutex<Vec<Mail>>,
}

impl MemoryMailer {
    pub fn sent(&self) -> Vec<Mail> {
        self.sent.lock().expect("mail store lock poisoned").clone()
    }

    /// The most recent message sent to `address`.
    pub fn last_to(&self, address: &str) -> Option<Mail> {
        self.sent()
            .into_iter()
            .rev()
            .find(|mail| mail.to == address)
    }
}

impl Mailer for MemoryMailer {
    fn send<'a>(&'a self, mail: &'a Mail) -> MailFuture<'a> {
        Box::pin(async move {
            self.sent
                .lock()
                .expect("mail store lock poisoned")
                .push(mail.clone());
            Ok(())
        })
    }
}

/// Production mailer.
pub struct SmtpMailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
}

impl SmtpMailer {
    /// `url` is an SMTP URL such as `smtps://user:password@smtp.example.org:465`,
    /// `smtp://user:password@smtp.example.org:587?tls=required` or `smtp://mailpit:1025`.
    pub fn new(url: &str, from: &str) -> Result<Self, MailError> {
        let transport = AsyncSmtpTransport::<Tokio1Executor>::from_url(url)
            // Never echo the URL: it contains the SMTP password.
            .map_err(|_| MailError::Transport("invalid SMTP URL".into()))?
            .build();
        let from = from
            .parse()
            .map_err(|_| MailError::Address(from.to_owned()))?;
        Ok(Self { transport, from })
    }
}

impl Mailer for SmtpMailer {
    fn send<'a>(&'a self, mail: &'a Mail) -> MailFuture<'a> {
        Box::pin(async move {
            let to: Mailbox = mail
                .to
                .parse()
                .map_err(|_| MailError::Address(mail.to.clone()))?;
            let message = Message::builder()
                .from(self.from.clone())
                .to(to)
                .subject(mail.subject.clone())
                .body(mail.body.clone())
                .map_err(|e| MailError::Build(e.to_string()))?;
            self.transport
                .send(message)
                .await
                .map_err(|e| MailError::Transport(e.to_string()))?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn memory_mailer_records_messages() {
        let mailer = MemoryMailer::default();
        let mail = |to: &str, subject: &str| Mail {
            to: to.into(),
            subject: subject.into(),
            body: "hello".into(),
        };
        mailer.send(&mail("a@epitech.eu", "first")).await.unwrap();
        mailer.send(&mail("b@epitech.eu", "other")).await.unwrap();
        mailer.send(&mail("a@epitech.eu", "second")).await.unwrap();
        assert_eq!(mailer.sent().len(), 3);
        assert_eq!(mailer.last_to("a@epitech.eu").unwrap().subject, "second");
        assert!(mailer.last_to("c@epitech.eu").is_none());
    }

    // The pooled SMTP transport needs a runtime as soon as it is built.
    #[tokio::test]
    async fn smtp_mailer_rejects_bad_configuration_without_leaking_it() {
        let error = SmtpMailer::new("not a url with secret-password", "noreply@epitech.eu")
            .err()
            .unwrap();
        assert!(!error.to_string().contains("secret-password"));
        assert!(SmtpMailer::new("smtp://localhost:1025", "not an address").is_err());
        assert!(SmtpMailer::new("smtp://localhost:1025", "EpiCoin <noreply@epitech.eu>").is_ok());
    }
}
