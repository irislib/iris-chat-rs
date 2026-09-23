use std::rc::Rc;

use adw::prelude::*;
use iris_chat_core::{AppAction, AppState, RemoteSignerPhase};

use crate::app_manager::AppManager;
use crate::platform::clipboard;
use crate::screens::{entry, pill_button, primary_button, screen_container};
use crate::widgets::qr;

pub fn render(state: &AppState, manager: &Rc<AppManager>) -> gtk::Widget {
    let container = screen_container();
    container.set_spacing(20);
    let header = gtk::Label::new(Some("Signer app/device"));
    header.add_css_class("title-2");
    container.append(&header);
    let login = state.remote_signer_login.as_ref();
    let awaiting_approval = login.is_some_and(|login| {
        matches!(
            login.phase,
            RemoteSignerPhase::WaitingForApproval | RemoteSignerPhase::Finishing
        )
    });
    if let Some(uri) = login
        .and_then(|login| login.connection_uri.as_ref())
        .filter(|_| !awaiting_approval)
    {
        container.append(&hint("Scan with your signer app."));
        container.append(&qr::build(uri, 240));
        let copy = pill_button("Copy code");
        let uri = uri.clone();
        copy.connect_clicked(move |_| clipboard::copy(&uri));
        container.append(&copy);
    } else if let Some(login) = login {
        let spinner = gtk::Spinner::new();
        spinner.start();
        container.append(&spinner);
        container.append(&hint(match login.phase {
            RemoteSignerPhase::Connecting => "Connecting…",
            RemoteSignerPhase::WaitingForSigner => "Waiting for your signer…",
            RemoteSignerPhase::WaitingForApproval => "Approve in your signer app.",
            RemoteSignerPhase::Finishing => "Signing in…",
        }));
    } else {
        let retry = pill_button("Try again");
        let manager = manager.clone();
        retry.connect_clicked(move |_| manager.dispatch(AppAction::StartRemoteSignerLogin));
        container.append(&retry);
    }
    if let Some(url) = login
        .and_then(|login| login.auth_url.as_ref())
        .filter(|url| {
            gtk::glib::Uri::parse(url, gtk::glib::UriFlags::NONE).is_ok_and(|url| {
                matches!(url.scheme().as_str(), "http" | "https") && url.host().is_some()
            })
        })
    {
        let approve = primary_button("Open approval");
        let url = url.clone();
        approve.connect_clicked(move |_| {
            let _ =
                gtk::gio::AppInfo::launch_default_for_uri(&url, gtk::gio::AppLaunchContext::NONE);
        });
        container.append(&approve);
    }
    if !awaiting_approval {
        append_link_input(&container, manager);
    }
    container.upcast()
}

fn hint(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.add_css_class("dim-label");
    label.set_wrap(true);
    label
}

fn append_link_input(container: &gtk::Box, manager: &Rc<AppManager>) {
    let paste = pill_button("Paste signer link");
    let input = entry("Signer link");
    input.set_visible(false);
    let connect = primary_button("Connect");
    connect.set_visible(false);
    connect.set_sensitive(false);
    {
        let connect = connect.clone();
        input.connect_changed(move |input| connect.set_sensitive(!input.text().trim().is_empty()));
    }
    {
        let input = input.clone();
        let connect = connect.clone();
        paste.connect_clicked(move |paste| {
            paste.set_visible(false);
            input.set_visible(true);
            connect.set_visible(true);
            input.grab_focus();
            let input = input.clone();
            clipboard::paste(move |text| input.set_text(&text));
        });
    }
    {
        let input = input.clone();
        let manager = manager.clone();
        connect.connect_clicked(move |_| {
            manager.dispatch(AppAction::ConnectRemoteSigner {
                connection_uri: input.text().to_string(),
            });
        });
    }
    container.append(&paste);
    container.append(&input);
    container.append(&connect);
}
