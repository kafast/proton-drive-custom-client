use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use protonsync_application::ports::{Message, Notifier, NotifyError};

/// Notifier for tests: remembers every delivered message and can fail
/// the next `n` attempts on purpose.
#[derive(Debug, Default)]
pub struct FakeNotifier {
    sent: Mutex<Vec<Message>>,
    failures_left: AtomicUsize,
}

impl FakeNotifier {
    /// Notifier that delivers everything.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The next `count` attempts fail with [`NotifyError::Unavailable`].
    pub fn fail_next(&self, count: usize) {
        self.failures_left.store(count, Ordering::SeqCst);
    }

    /// All delivered messages, oldest first.
    #[must_use]
    pub fn sent(&self) -> Vec<Message> {
        self.messages().clone()
    }

    // A panicking test thread must not hide the messages from the others.
    fn messages(&self) -> std::sync::MutexGuard<'_, Vec<Message>> {
        self.sent
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

// The fake answers immediately; the port still requires `async`.
#[allow(clippy::unused_async_trait_impl)]
impl Notifier for FakeNotifier {
    fn channel(&self) -> &'static str {
        "fake"
    }

    async fn send(&self, message: &Message) -> Result<(), NotifyError> {
        // A plain load/store is enough: the fake is only used from one test at a time.
        let left = self.failures_left.load(Ordering::SeqCst);
        let failing = left > 0;
        if failing {
            self.failures_left.store(left - 1, Ordering::SeqCst);
        }
        if failing {
            return Err(NotifyError::Unavailable("fake outage".into()));
        }
        self.messages().push(message.clone());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message() -> Message {
        Message {
            subject: "s".into(),
            body: "b".into(),
        }
    }

    #[tokio::test]
    async fn records_messages_and_fails_on_request() {
        let notifier = FakeNotifier::new();
        notifier.fail_next(1);
        assert!(notifier.send(&message()).await.is_err());
        assert!(notifier.send(&message()).await.is_ok());
        assert_eq!(notifier.sent(), vec![message()]);
    }
}
