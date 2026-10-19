use super::*;

pub(super) fn new_ffi_app_inner(data_dir: String) -> Arc<FfiApp> {
    let (update_tx, update_rx) = flume::unbounded();
    let (foreground_tx, foreground_rx) = flume::unbounded();
    let (background_tx, background_rx) = flume::unbounded();
    let shared_state = Arc::new(RwLock::new(AppState::empty()));
    let queue_metrics = Arc::new(CoreQueueMetrics::default());
    let recovery = Arc::new(CoreRecoveryState::default());
    let shared_db = Arc::new(RwLock::new(None));
    let shared_user_profiles: user_profile::SharedUserProfiles = Default::default();
    let mut core_worker = None;

    let update_tx_for_error = update_tx.clone();
    match AppCore::try_new_with_priority_sender(
        update_tx.clone(),
        background_tx.clone(),
        foreground_tx.clone(),
        data_dir.clone(),
        shared_state.clone(),
    ) {
        Ok(mut core) => {
            core.set_shared_user_profiles(Arc::clone(&shared_user_profiles));
            set_shared_db(&shared_db, Some(core.shared_db()));
            let spawn_result = spawn_core_supervisor(
                core,
                CoreSupervisor {
                    data_dir,
                    update_tx: update_tx.clone(),
                    core_sender: background_tx.clone(),
                    priority_sender: foreground_tx.clone(),
                    foreground_rx: foreground_rx.clone(),
                    background_rx: background_rx.clone(),
                    shared_state: shared_state.clone(),
                    shared_db: shared_db.clone(),
                    shared_user_profiles: Arc::clone(&shared_user_profiles),
                    queue_metrics: queue_metrics.clone(),
                    recovery: recovery.clone(),
                },
            );
            match spawn_result {
                Ok(worker) => core_worker = Some(worker),
                Err(error) => {
                    set_shared_db(&shared_db, None);
                    publish_core_failure_state(
                        &shared_state,
                        &update_tx_for_error,
                        format!("Iris could not start: {error}"),
                    );
                }
            }
        }
        Err(error) => {
            publish_core_failure_state(&shared_state, &update_tx_for_error, error.to_string());
        }
    }

    Arc::new(FfiApp {
        foreground_tx,
        foreground_rx,
        background_tx,
        background_rx,
        update_rx,
        listening: AtomicBool::new(false),
        shared_state,
        shared_db,
        shared_user_profiles,
        core_worker: Mutex::new(core_worker),
        perf: FfiPerfCounters::default(),
        queue_metrics,
        recovery,
    })
}

pub(super) fn ffi_app_failure(message: String) -> Arc<FfiApp> {
    let (_update_tx, update_rx) = flume::unbounded();
    let (foreground_tx, foreground_rx) = flume::unbounded();
    let (background_tx, background_rx) = flume::unbounded();
    let mut state = AppState::empty();
    state.toast = Some(message);
    state.rev = 1;
    let shared_state = Arc::new(RwLock::new(state));
    Arc::new(FfiApp {
        foreground_tx,
        foreground_rx,
        background_tx,
        background_rx,
        update_rx,
        listening: AtomicBool::new(false),
        shared_state,
        shared_db: Arc::new(RwLock::new(None)),
        shared_user_profiles: Default::default(),
        core_worker: Mutex::new(None),
        perf: FfiPerfCounters::default(),
        queue_metrics: Arc::new(CoreQueueMetrics::default()),
        recovery: Arc::new(CoreRecoveryState::default()),
    })
}
