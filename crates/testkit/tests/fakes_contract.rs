//! Die Fakes für das lokale Dateisystem und den Zustand bestehen dieselben
//! Contract-Tests wie die echten Adapter.

mod local {
    protonsync_testkit::local_storage_contract_tests!(protonsync_testkit::FakeLocal::new);
}

mod state {
    protonsync_testkit::state_repo_contract_tests!(protonsync_testkit::InMemoryStateRepo::new);
}
