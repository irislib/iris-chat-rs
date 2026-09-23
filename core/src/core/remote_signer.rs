use super::remote_signer_rpc::SignerRpc;
use super::remote_signer_transport::{run, RemoteSignRequest};
use super::remote_signer_uri::{
    client_connection_uri, parse_signer_connection, validate_signer_relays,
};
use super::*;
use crate::{RemoteSignerLoginSnapshot, RemoteSignerPhase};
use tokio::sync::{mpsc, oneshot};

pub(super) struct PendingRemoteSigner {
    pub(super) token: String,
    commands: mpsc::Sender<RemoteSignRequest>,
    cancel: oneshot::Sender<()>,
    deferred: Vec<InternalEvent>,
}

impl AppCore {
    pub(super) fn start_remote_signer_login(&mut self, input: Option<&str>) {
        if self.logged_in.is_some()
            || self
                .pending_signer_login
                .as_ref()
                .is_some_and(|p| p.publishing)
        {
            return;
        }
        let result = (|| -> anyhow::Result<_> {
            let connection = input.map(parse_signer_connection).transpose()?;
            let relays = match &connection {
                Some(connection) => connection.relays.clone(),
                None => validate_signer_relays(&self.preferences.nostr_relay_urls)?,
            };
            Ok((connection, relays))
        })();
        let (connection, relays) = match result {
            Ok(result) => result,
            Err(_) => {
                self.state.toast = Some(
                    if input.is_some() {
                        "That signer link is not valid."
                    } else {
                        "Add a message server before signing in."
                    }
                    .into(),
                );
                self.emit_state();
                return;
            }
        };
        self.cancel_remote_signer_login();
        self.pending_signer_login = None;
        self.stop_pending_linked_device();
        let keys = Keys::generate();
        let token = uuid::Uuid::new_v4().to_string();
        let challenge = uuid::Uuid::new_v4().to_string();
        let uri = connection
            .is_none()
            .then(|| client_connection_uri(&keys, &relays, &challenge));
        let signer = connection.as_ref().map(|connection| connection.signer);
        let client = Client::new(keys.clone());
        let notifications = client.notifications();
        let rpc = SignerRpc {
            client,
            keys,
            signer,
            notifications,
            token: token.clone(),
            tx: self.core_sender.clone(),
            started_at: Timestamp::from(Timestamp::now().as_secs().saturating_sub(10)),
        };
        let (commands, receiver) = mpsc::channel(1);
        let (cancel, cancelled) = oneshot::channel();
        self.pending_remote_signer = Some(PendingRemoteSigner {
            token,
            commands,
            cancel,
            deferred: Vec::new(),
        });
        self.state.remote_signer_login = Some(RemoteSignerLoginSnapshot {
            connection_uri: uri,
            phase: RemoteSignerPhase::Connecting,
            auth_url: None,
        });
        self.state.busy.restoring_session = true;
        self.screen_stack = vec![Screen::RestoreAccount, Screen::RemoteSigner];
        self.active_chat_id = None;
        self.runtime
            .spawn(run(rpc, relays, connection, challenge, receiver, cancelled));
        self.rebuild_state();
        self.emit_state();
    }

    pub(super) fn handle_remote_signer_event(&mut self, event: InternalEvent) {
        let token = match &event {
            InternalEvent::RemoteSignerProgress { token, .. }
            | InternalEvent::RemoteSignerConnected { token, .. }
            | InternalEvent::RemoteSignerSigned { token, .. }
            | InternalEvent::RemoteSignerFailed { token, .. } => token,
            _ => return,
        };
        if !self
            .pending_remote_signer
            .as_ref()
            .is_some_and(|pending| &pending.token == token)
        {
            return;
        }
        match event {
            InternalEvent::RemoteSignerProgress {
                phase, auth_url, ..
            } => {
                if let Some(snapshot) = &mut self.state.remote_signer_login {
                    snapshot.phase = phase;
                    snapshot.auth_url = auth_url;
                }
                self.emit_state();
            }
            InternalEvent::RemoteSignerConnected {
                owner_pubkey_hex, ..
            } => self.begin_signer_login(&owner_pubkey_hex),
            InternalEvent::RemoteSignerSigned {
                request_id,
                signed_event_json,
                ..
            } => {
                if let Some(snapshot) = &mut self.state.remote_signer_login {
                    snapshot.phase = RemoteSignerPhase::Finishing;
                    snapshot.auth_url = None;
                }
                self.complete_signer_login(&request_id, &signed_event_json);
                self.emit_state();
            }
            InternalEvent::RemoteSignerFailed { message, .. } => self.fail_signer_login(&message),
            _ => {}
        }
    }

    pub(super) fn send_remote_signer_request(
        &mut self,
        request_id: &str,
        unsigned_event_json: &str,
    ) -> bool {
        let Some(pending) = &self.pending_remote_signer else {
            return false;
        };
        if pending
            .commands
            .try_send(RemoteSignRequest {
                request_id: request_id.into(),
                unsigned_event_json: unsigned_event_json.into(),
            })
            .is_err()
        {
            self.fail_signer_login("Could not reach the signer. Try again.");
        }
        true
    }

    pub(super) fn defer_remote_signer_event(&mut self, event: InternalEvent) {
        let Some(pending) = &mut self.pending_remote_signer else {
            return;
        };
        if !matches!(
            event,
            InternalEvent::RemoteSignerProgress { .. }
                | InternalEvent::RemoteSignerConnected { .. }
                | InternalEvent::RemoteSignerSigned { .. }
                | InternalEvent::RemoteSignerFailed { .. }
                | InternalEvent::SignerLoginFetched { .. }
                | InternalEvent::SignerLoginPublished { .. }
                | InternalEvent::SignerLoginTimedOut { .. }
        ) {
            return;
        }
        // Progress is replaceable; approval and publication results must survive
        // opening an external approval page while the app is in the background.
        if matches!(event, InternalEvent::RemoteSignerProgress { .. }) {
            pending
                .deferred
                .retain(|event| !matches!(event, InternalEvent::RemoteSignerProgress { .. }));
        }
        if pending.deferred.len() < 16 {
            pending.deferred.push(event);
        }
    }

    pub(super) fn resume_remote_signer_events(&mut self) {
        let events = self
            .pending_remote_signer
            .as_mut()
            .map(|pending| std::mem::take(&mut pending.deferred))
            .unwrap_or_default();
        for event in events {
            self.handle_internal(event);
        }
    }

    pub(super) fn stop_remote_signer(&mut self) {
        if let Some(pending) = self.pending_remote_signer.take() {
            let _ = pending.cancel.send(());
        }
        self.state.remote_signer_login = None;
    }

    pub(super) fn cancel_remote_signer_login(&mut self) {
        if self.pending_remote_signer.is_none()
            || self
                .pending_signer_login
                .as_ref()
                .is_some_and(|pending| pending.publishing)
        {
            return;
        }
        self.stop_remote_signer();
        self.pending_signer_login = None;
        self.state.busy.restoring_session = false;
    }
}
