use super::*;
use std::collections::VecDeque;
use std::rc::Rc;

const CACHE_LIMIT: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Key {
    generation: u64,
    account: Option<String>,
    query: String,
    scope: Option<String>,
    limit: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Request {
    key: Key,
    revision: u64,
}

struct Response {
    request: Request,
    result: Result<SearchResultSnapshot, String>,
}

struct Cached {
    request: Request,
    result: SearchResultSnapshot,
}

#[derive(Default)]
pub(super) struct Search {
    generation: u64,
    worker: Option<Worker>,
    in_flight: Option<Request>,
    desired: Option<Request>,
    cache: VecDeque<Cached>,
}

impl Search {
    pub(super) fn clear(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.desired = None;
        self.cache.clear();
        // Let an old query finish on the same worker, but never accept its
        // result after logout, even if the same account/query is opened again.
    }

    fn request(&mut self, request: Request) -> Option<SearchResultSnapshot> {
        let cached = self
            .cache
            .iter()
            .find(|cached| cached.request.key == request.key)
            .map(|cached| cached.result.clone());
        // Only the latest edit is retained while a search is in progress.
        self.desired = Some(request);
        self.start_if_needed();
        cached
    }

    fn start_if_needed(&mut self) {
        if self.in_flight.is_some() {
            return;
        }
        let Some(request) = self.desired.as_ref() else {
            return;
        };
        if self.cache.iter().any(|cached| {
            cached.request.key == request.key && cached.request.revision >= request.revision
        }) {
            return;
        }
        if let Some(worker) = &self.worker {
            if worker.requests.try_send(request.clone()).is_ok() {
                self.in_flight = Some(request.clone());
            }
        }
    }

    fn complete(&mut self, response: Response, current: Request) -> Option<String> {
        if self.in_flight.as_ref() != Some(&response.request) {
            return None;
        }
        self.in_flight = None;
        let mut error = None;
        if response.request.key == current.key {
            let result = response.result.unwrap_or_else(|detail| {
                error = Some(detail);
                SearchResultSnapshot::empty(current.key.query.clone(), current.key.scope.clone())
            });
            self.cache
                .retain(|cached| cached.request.key != current.key);
            self.cache.push_front(Cached {
                request: response.request,
                result,
            });
            self.cache.truncate(CACHE_LIMIT);
        }
        // A source-state update does not invalidate a completed result for the
        // same query. Show it immediately, then refresh once at the latest rev.
        // Requiring an exact revision here would starve searches during sync.
        if self
            .desired
            .as_ref()
            .is_some_and(|desired| desired.key == current.key)
        {
            self.desired = Some(current);
        } else {
            self.desired = None;
        }
        self.start_if_needed();
        error
    }
}

struct Worker {
    requests: async_channel::Sender<Request>,
}

impl Worker {
    fn start(
        mut execute: impl FnMut(&Request) -> SearchResultSnapshot + Send + 'static,
    ) -> (Self, async_channel::Receiver<Response>) {
        let (requests, incoming) = async_channel::bounded::<Request>(1);
        let (finished, responses) = async_channel::bounded(1);
        // The UI keeps one running query and one latest pending query. This
        // worker lives for the manager's lifetime instead of per keystroke.
        thread::spawn(move || {
            while let Ok(request) = incoming.recv_blocking() {
                let result = catch_unwind(AssertUnwindSafe(|| execute(&request)))
                    .map_err(|payload| panic_payload_message(payload.as_ref()));
                if finished
                    .send_blocking(Response { request, result })
                    .is_err()
                {
                    break;
                }
            }
        });
        (Self { requests }, responses)
    }
}

impl AppManager {
    /// Return only this query's cached results, and search without blocking GTK.
    /// None means the first result is still pending; old results remain usable
    /// during refreshes, including when profile/history sync is running.
    pub fn search_results(self: &Rc<Self>, limit: u32) -> Option<SearchResultSnapshot> {
        let request = self.search_request(limit);
        if request.key.account.is_none()
            || (request.key.query.trim().is_empty() && request.key.scope.is_none())
        {
            self.search.borrow_mut().desired = None;
            return Some(SearchResultSnapshot::empty(
                request.key.query,
                request.key.scope,
            ));
        }
        self.start_search_worker();
        self.search.borrow_mut().request(request)
    }

    fn search_request(&self, limit: u32) -> Request {
        let ui = self.search_ui.borrow();
        let state = self.local_state.borrow();
        Request {
            key: Key {
                generation: self.search.borrow().generation,
                account: state
                    .account
                    .as_ref()
                    .map(|account| account.public_key_hex.clone()),
                query: ui.query.clone(),
                scope: ui.scope_chat_id.clone(),
                limit,
            },
            revision: state.rev,
        }
    }

    fn start_search_worker(self: &Rc<Self>) {
        if self.search.borrow().worker.is_some() {
            return;
        }
        let ffi = self.ffi.clone();
        let (worker, responses) = Worker::start(move |request| {
            ffi.search(
                request.key.query.clone(),
                request.key.scope.clone(),
                request.key.limit,
            )
        });
        self.search.borrow_mut().worker = Some(worker);
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            while let Ok(response) = responses.recv().await {
                let Some(manager) = weak.upgrade() else {
                    break;
                };
                let limit = manager
                    .search
                    .borrow()
                    .desired
                    .as_ref()
                    .map(|request| request.key.limit)
                    .unwrap_or(response.request.key.limit);
                let current = manager.search_request(limit);
                let relevant = response.request.key == current.key;
                let error = manager.search.borrow_mut().complete(response, current);
                if let Some(detail) = error {
                    manager.log_client_failure("ffiapp.search", &detail);
                }
                if relevant {
                    manager.redraw_ui();
                }
            }
        });
    }
}

#[cfg(feature = "ui-tests")]
#[path = "search_tests.rs"]
mod tests;
#[cfg(feature = "ui-tests")]
pub use tests::verify_ui;
