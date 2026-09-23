//! Opt-in interoperability driver using the same FFI actions and updates as
//! native shells. Fresh test account only; echoes media after accepting a call.
use anyhow::{bail, Context, Result};
use iris_chat_core::{AppAction, AppReconciler, AppUpdate, FfiApp};
use serde_json::json;
use std::io::{self, BufRead, Write};
use std::time::{Duration, Instant};

struct Updates(flume::Sender<AppUpdate>);
impl AppReconciler for Updates {
    fn reconcile(&self, update: AppUpdate) {
        let _ = self.0.try_send(update);
    }
}
fn emit(value: serde_json::Value) -> Result<()> {
    println!("{value}");
    io::stdout().flush().context("flush fixture output")
}
fn main() -> Result<()> {
    let dir = std::env::args()
        .nth(1)
        .context("usage: iris-call-fixture <fresh-data-dir>")?;
    if std::path::Path::new(&dir)
        .read_dir()
        .is_ok_and(|mut entries| entries.next().is_some())
    {
        bail!("Call fixture requires a fresh data directory");
    }
    let app = FfiApp::new(dir, String::new(), String::new());
    let (tx, updates) = flume::bounded(96);
    app.listen_for_updates(Box::new(Updates(tx)));
    app.dispatch(AppAction::CreateAccount {
        name: "Call test".into(),
    });
    app.dispatch(AppAction::SetNearbyLanEnabled { enabled: true });
    app.dispatch(AppAction::CreatePublicInvite);
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let state = app.state();
        if let (Some(account), Some(invite)) = (state.account, state.public_invite) {
            emit(
                json!({"event":"ready","owner":account.public_key_hex,"device":account.device_public_key_hex,"invite":invite.url}),
            )?;
            break;
        }
        if Instant::now() > deadline {
            bail!("Account and invite did not become ready: {:?}", state.toast);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let (commands_tx, commands) = flume::bounded(16);
    std::thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            let Ok(line) = line else {
                break;
            };
            if commands_tx.send(line).is_err() {
                break;
            }
        }
    });
    let mut auto_answer = true;
    let mut answer_voice = false;
    let mut last_call = String::new();
    let mut audio = 0u64;
    let mut video = 0u64;
    let mut audio_nonzero = 0u64;
    loop {
        match commands.recv_timeout(Duration::from_millis(10)) {
            Ok(line) => {
                let parts = line.split_whitespace().collect::<Vec<_>>();
                match parts.as_slice() {
                    ["accept", owner] => {
                        app.dispatch(AppAction::CreateChat {
                            peer_input: (*owner).into(),
                        });
                        app.dispatch(AppAction::SetMessageRequestAccepted {
                            chat_id: (*owner).into(),
                        });
                        emit(json!({"event":"accepted"}))?;
                    }
                    ["call", owner, kind] => app.dispatch(AppAction::StartCall {
                        chat_id: (*owner).into(),
                        video: *kind == "video",
                    }),
                    ["answer", kind] => {
                        auto_answer = true;
                        answer_voice = *kind == "voice";
                    }
                    ["manual"] => auto_answer = false,
                    ["status"] => {
                        let state = app.state();
                        let call=state.call.map(|s|json!({"id":s.call_id,"phase":s.phase,"video":s.video_capable,"muted":s.remote_muted}));
                        emit(
                            json!({"event":"status","call":call,"audio_frames":audio,"video_frames":video,"nonzero_audio_frames":audio_nonzero}),
                        )?;
                    }
                    ["end"] => {
                        if let Some(c) = app.state().call {
                            app.dispatch(AppAction::EndCall { call_id: c.call_id });
                        }
                    }
                    ["stop"] => break,
                    _ => emit(json!({"event":"error","reason":"Unknown fixture command"}))?,
                }
            }
            Err(flume::RecvTimeoutError::Disconnected) => break,
            Err(flume::RecvTimeoutError::Timeout) => {}
        }
        for update in updates.try_iter().take(128) {
            if let AppUpdate::CallMedia {
                call_id,
                kind,
                data,
            } = update
            {
                if kind == 1 {
                    audio += 1;
                    if data.iter().any(|b| *b != 0) {
                        audio_nonzero += 1;
                    }
                } else if kind == 2 {
                    video += 1;
                }
                app.dispatch(AppAction::SendCallMedia {
                    call_id,
                    kind,
                    data,
                });
            }
        }
        if let Some(call) = app.state().call {
            let state = format!("{}:{}", call.call_id, call.phase);
            if last_call != state {
                emit(
                    json!({"event":"call","id":call.call_id,"phase":call.phase,"video":call.video_capable}),
                )?;
                last_call = state;
                if auto_answer && call.phase == "incoming" {
                    app.dispatch(if answer_voice {
                        AppAction::AnswerCallWithVoice {
                            call_id: call.call_id,
                        }
                    } else {
                        AppAction::AnswerCall {
                            call_id: call.call_id,
                        }
                    });
                }
            }
        }
    }
    app.shutdown();
    emit(json!({"event":"stopped"}))
}
