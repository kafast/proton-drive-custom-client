//! Die Fakes für das lokale Dateisystem und den Zustand bestehen dieselben
//! Contract-Tests wie die echten Adapter.

mod local {
    protonsync_testkit::local_storage_contract_tests!(protonsync_testkit::FakeLocal::new);
}

mod state {
    protonsync_testkit::state_repo_contract_tests!(protonsync_testkit::InMemoryStateRepo::new);
}

/// Geteilte Speicher (`Arc`) müssen sich genauso verhalten.
mod shared_local {
    protonsync_testkit::local_storage_contract_tests!(|| std::sync::Arc::new(
        protonsync_testkit::FakeLocal::new()
    ));
}

mod shared_state {
    protonsync_testkit::state_repo_contract_tests!(|| std::sync::Arc::new(
        protonsync_testkit::InMemoryStateRepo::new()
    ));
}

mod job_log {
    protonsync_testkit::job_log_contract_tests!(protonsync_testkit::InMemoryStateRepo::new);
}

mod drive_repo {
    protonsync_testkit::drive_repo_contract_tests!(protonsync_testkit::InMemoryStateRepo::new);
}

mod content_cache {
    protonsync_testkit::content_cache_contract_tests!(
        protonsync_testkit::InMemoryContentCache::new
    );
}
