//! Shared test environment: daemon core with fakes.

// Not every test file uses every helper.
#![allow(dead_code, clippy::unwrap_used)]

use std::sync::Arc;

use protonsync_daemon::config::PairConfig;
use protonsync_daemon::core::{Core, PairSetup, Settings};
use protonsync_domain::path::RelativePath;
use protonsync_domain::sync_pair::SyncDirection;
use protonsync_domain::time::Timestamp;
use protonsync_domain::trash::RetentionPeriod;
use protonsync_testkit::{FakeAccount, FakeLocal, FakeRemote, FixedClock, InMemoryStateRepo};

pub const T0: Timestamp = Timestamp::from_unix_seconds(1_760_000_000);
pub const PAIR: &str = "main";

pub type TestCore = Core<FakeAccount, Arc<FakeLocal>, Arc<InMemoryStateRepo>, Arc<FixedClock>>;

pub struct Fixture {
    pub core: TestCore,
    pub remote: Arc<FakeRemote>,
    pub local: Arc<FakeLocal>,
    pub clock: Arc<FixedClock>,
}

pub fn fixture_with(
    account: impl FnOnce(Arc<FakeRemote>) -> FakeAccount,
    dry_run: bool,
) -> Fixture {
    let remote = Arc::new(FakeRemote::new());
    let local = Arc::new(FakeLocal::new());
    let clock = Arc::new(FixedClock::at(T0));
    let pair = PairSetup {
        config: PairConfig {
            id: PAIR.into(),
            local: "/home/k/Proton".into(),
            remote: RelativePath::parse("protonsync").unwrap(),
            direction: SyncDirection::TwoWay,
        },
        local: Arc::clone(&local),
    };
    let core = Core::new(
        account(Arc::clone(&remote)),
        Arc::new(InMemoryStateRepo::new()),
        Arc::clone(&clock),
        Settings {
            dry_run,
            retention: RetentionPeriod::default(),
        },
        vec![pair],
    );
    Fixture {
        core,
        remote,
        local,
        clock,
    }
}

/// Logged in (stored session), automatic syncs really run.
pub async fn logged_in() -> Fixture {
    let f = fixture_with(|r| FakeAccount::new(r).with_stored_session(), false);
    f.core.startup().await;
    f
}
