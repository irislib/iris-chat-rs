use super::*;
use std::sync::mpsc;

pub fn verify_ui(manager: Rc<AppManager>) {
    verify_worker();
    let previous = manager.search_ui();
    manager.clear_chat_scope();
    manager.set_search_query("search-worker-production-fixture".into());
    let before = Instant::now();
    assert!(manager.search_results(50).is_none());
    assert!(
        before.elapsed() < Duration::from_millis(100),
        "search must return without awaiting the core"
    );
    let idle_ran = Rc::new(Cell::new(false));
    let idle = idle_ran.clone();
    glib::idle_add_local_once(move || idle.set(true));
    let updates = manager.update_rx();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let context = glib::MainContext::default();
        while context.pending() {
            context.iteration(false);
        }
        while let Ok(update) = updates.try_recv() {
            manager.apply_update(update);
        }
        if let Some(result) = manager.search_results(50) {
            assert_eq!(result.query, "search-worker-production-fixture");
            assert_eq!(result.scope_chat_id, None);
            break;
        }
        assert!(
            Instant::now() < deadline,
            "background core search completion timed out"
        );
        thread::sleep(Duration::from_millis(2));
    }
    assert!(
        idle_ran.get(),
        "GTK remains responsive while the search runs"
    );
    *manager.search_ui.borrow_mut() = previous;
    manager.search.borrow_mut().desired = None;
    println!("PASS: background search worker, coalesced edits, stale-query/scope/account rejection, cached refresh, GTK completion");
}

fn request(query: &str, revision: u64) -> Request {
    Request {
        key: Key {
            generation: 0,
            account: Some("account-a".into()),
            query: query.into(),
            scope: None,
            limit: 50,
        },
        revision,
    }
}

fn verify_worker() {
    let main_thread = thread::current().id();
    let (started, starts) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let (worker, responses) = Worker::start(move |request| {
        assert_ne!(thread::current().id(), main_thread);
        started.send(request.clone()).unwrap();
        released.recv().unwrap();
        SearchResultSnapshot::empty(request.key.query.clone(), request.key.scope.clone())
    });
    let mut search = Search {
        worker: Some(worker),
        ..Default::default()
    };
    let first = request("a", 1);
    assert!(search.request(first.clone()).is_none());
    assert_eq!(starts.recv_timeout(Duration::from_secs(3)).unwrap(), first);
    // These calls run while the backend is deliberately blocked. Only the last
    // edit should reach the worker, with no thread or queue per keystroke.
    assert!(search.request(request("ab", 2)).is_none());
    let latest = request("abc", 3);
    assert!(search.request(latest.clone()).is_none());
    assert!(starts.try_recv().is_err());
    release.send(()).unwrap();
    search.complete(response(&responses), latest.clone());
    assert!(
        search.cache.is_empty(),
        "stale query must not enter the result cache"
    );
    assert_eq!(starts.recv_timeout(Duration::from_secs(3)).unwrap(), latest);

    // A stream of sync revisions must not discard an otherwise current result.
    let fresh = request("abc", 20);
    for revision in 4..=20 {
        assert!(search.request(request("abc", revision)).is_none());
    }
    release.send(()).unwrap();
    search.complete(response(&responses), fresh.clone());
    assert!(
        search.request(fresh.clone()).is_some(),
        "show current-query results during refresh"
    );
    assert_eq!(starts.recv_timeout(Duration::from_secs(3)).unwrap(), fresh);
    release.send(()).unwrap();
    search.complete(response(&responses), fresh.clone());
    for _ in 0..5 {
        assert!(search.request(fresh.clone()).is_some());
        assert!(
            search.in_flight.is_none(),
            "completion redraw at same rev must not loop"
        );
    }
    assert!(starts.try_recv().is_err());

    let mut scoped = fresh.clone();
    scoped.key.scope = Some("another-chat".into());
    assert!(search.request(scoped.clone()).is_none());
    assert_eq!(starts.recv_timeout(Duration::from_secs(3)).unwrap(), scoped);
    let mut account = scoped.clone();
    account.key.account = Some("account-b".into());
    assert!(search.request(account.clone()).is_none());
    release.send(()).unwrap();
    search.complete(response(&responses), account.clone());
    assert!(!search
        .cache
        .iter()
        .any(|cached| cached.request.key == scoped.key));
    assert_eq!(
        starts.recv_timeout(Duration::from_secs(3)).unwrap(),
        account
    );
    release.send(()).unwrap();
    search.complete(response(&responses), account.clone());
    let result = search.request(account).unwrap();
    assert_eq!(result.scope_chat_id.as_deref(), Some("another-chat"));
    assert!(
        search.request(fresh).is_some(),
        "prior query/scope cache can be reused"
    );
    assert!(search.in_flight.is_none());

    let before_logout = request("same-account-query", 30);
    assert!(search.request(before_logout.clone()).is_none());
    assert_eq!(
        starts.recv_timeout(Duration::from_secs(3)).unwrap(),
        before_logout
    );
    search.clear();
    assert!(
        search.cache.is_empty(),
        "logout removes cached search results"
    );
    assert!(search.desired.is_none(), "logout removes pending queries");
    let mut after_login = before_logout.clone();
    after_login.key.generation = search.generation;
    assert!(search.request(after_login.clone()).is_none());
    release.send(()).unwrap();
    search.complete(response(&responses), after_login.clone());
    assert!(
        search.cache.is_empty(),
        "pre-logout reply must be rejected after same-account login"
    );
    assert_eq!(
        starts.recv_timeout(Duration::from_secs(3)).unwrap(),
        after_login
    );
    release.send(()).unwrap();
    search.complete(response(&responses), after_login.clone());
    assert!(search.request(after_login).is_some());
}

fn response(responses: &async_channel::Receiver<Response>) -> Response {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Ok(response) = responses.try_recv() {
            return response;
        }
        assert!(Instant::now() < deadline, "search worker timed out");
        thread::sleep(Duration::from_millis(2));
    }
}
