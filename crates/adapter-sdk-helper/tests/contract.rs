//! `HelperRemote` against a fake helper must pass the same contract tests as
//! every other `RemoteStorage`. This checks the Rust side and the protocol;
//! the TypeScript helper is checked with live tests on a real account.

mod support;

protonsync_testkit::remote_storage_contract_tests!(support::helper_remote);
