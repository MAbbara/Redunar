//! Native session-bus tray client. GIO is already part of GTK; no indicator DSO.
mod interfaces;
mod menu;
#[cfg(test)]
mod tests;
mod watcher;

use gio::prelude::*;
use glib::{variant::ToVariant, Variant};
use gtk::{gio, glib};
use std::{
    sync::{mpsc, Arc, Mutex},
    thread,
    time::Duration,
};

pub(super) const ITEM_PATH: &str = "/StatusNotifierItem";
pub(super) const MENU_PATH: &str = "/StatusNotifierItem/Menu";
pub(super) const WATCHER: &str = "org.kde.StatusNotifierWatcher";
pub(super) const WATCHER_PATH: &str = "/StatusNotifierWatcher";
pub(super) const ITEM_INTERFACE: &str = "org.kde.StatusNotifierItem";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Action {
    Open,
    Quit,
    Recover,
}
pub(super) type Callback = Arc<dyn Fn(Action) + Send + Sync>;
type Shared = Arc<Mutex<watcher::State>>;

pub(crate) struct Icon {
    width: i32,
    height: i32,
    argb: Vec<u8>,
    tooltip: Arc<Mutex<String>>,
}

impl Icon {
    pub(crate) fn from_rgba(width: u32, height: u32, rgba: &[u8]) -> Result<Self, String> {
        if width == 0
            || height == 0
            || width > 256
            || height > 256
            || rgba.len() != (width * height * 4) as usize
        {
            return Err("The tray icon is unavailable".into());
        }
        let argb = rgba
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|pixel| [pixel[3], pixel[0], pixel[1], pixel[2]])
            .collect();
        Ok(Self {
            width: width as i32,
            height: height as i32,
            argb,
            tooltip: Arc::new(Mutex::new("Open Redunar".into())),
        })
    }

    fn pixmaps(&self) -> Vec<(i32, i32, Vec<u8>)> {
        vec![(self.width, self.height, self.argb.clone())]
    }

    fn property(&self, name: &str) -> Variant {
        match name {
            "Category" => "ApplicationStatus".to_variant(),
            "Id" => "redunar".to_variant(),
            "Title" => "Redunar".to_variant(),
            "Status" => "Active".to_variant(),
            "WindowId" => 0_u32.to_variant(),
            "IconName" => "com.redunar.Redunar".to_variant(),
            "IconPixmap" => self.pixmaps().to_variant(),
            "ToolTip" => (
                "com.redunar.Redunar",
                self.pixmaps(),
                "Redunar",
                self.tooltip.lock().expect("tray tooltip").as_str(),
            )
                .to_variant(),
            "ItemIsMenu" => false.to_variant(),
            "Menu" => glib::variant::ObjectPath::try_from(MENU_PATH)
                .expect("static menu path")
                .to_variant(),
            "AttentionIconPixmap" | "OverlayIconPixmap" => {
                Vec::<(i32, i32, Vec<u8>)>::new().to_variant()
            }
            _ => "".to_variant(),
        }
    }
}

/// Each enabled instance owns a private connection. Closing it unregisters the
/// item and avoids reused AppIndicator objects, duplicate paths or stale names.
pub(crate) struct NativeTray {
    bus: gio::DBusConnection,
    state: Shared,
    action: Callback,
    main_loop: glib::MainLoop,
    cancellable: gio::Cancellable,
    worker: Option<thread::JoinHandle<()>>,
    tooltip: Arc<Mutex<String>>,
}

impl NativeTray {
    pub(crate) fn start(icon: Icon, action: Callback) -> Result<Self, String> {
        Self::start_at(None, icon, action)
    }

    fn start_at(address: Option<String>, icon: Icon, action: Callback) -> Result<Self, String> {
        let tooltip = icon.tooltip.clone();
        let context = glib::MainContext::new();
        let main_loop = glib::MainLoop::new(Some(&context), false);
        let cancellable = gio::Cancellable::new();
        let state = Arc::new(Mutex::new(watcher::State::default()));
        let (ready, startup) = mpsc::sync_channel(1);
        let run_loop = main_loop.clone();
        let run_cancel = cancellable.clone();
        let run_state = state.clone();
        let run_action = action.clone();
        let worker = thread::Builder::new()
            .name("redunar-tray".into())
            .spawn(move || {
                let _ = context.with_thread_default(|| {
                    match Runtime::new(address.as_deref(), icon, &run_cancel, run_state, run_action)
                    {
                        Ok(runtime) => {
                            let _ = ready.send(Ok(runtime.bus.clone()));
                            run_loop.run();
                            drop(runtime);
                        }
                        Err(_) => {
                            let _ = ready
                                .send(Err("The desktop session bus is unavailable".to_owned()));
                        }
                    }
                });
            })
            .map_err(|_| "The native tray could not start")?;
        match startup.recv_timeout(Duration::from_secs(2)) {
            Ok(Ok(bus)) => Ok(Self {
                bus,
                state,
                action,
                main_loop,
                cancellable,
                worker: Some(worker),
                tooltip,
            }),
            result => {
                cancellable.cancel();
                let quit_loop = main_loop.clone();
                main_loop.context().invoke(move || quit_loop.quit());
                let _ = worker.join();
                Err(result
                    .ok()
                    .and_then(Result::err)
                    .unwrap_or_else(|| "The desktop session bus did not respond".into()))
            }
        }
    }

    pub(crate) fn is_running(&self) -> bool {
        watcher::running(&self.state) && !self.bus.is_closed()
    }

    pub(crate) fn set_tooltip(&self, text: &str) {
        if text.len() <= 128 {
            *self.tooltip.lock().expect("tray tooltip") = text.to_owned();
            let _ = self.bus.emit_signal(
                None,
                ITEM_PATH,
                ITEM_INTERFACE,
                "NewToolTip",
                Some(&().to_variant()),
            );
        }
    }

    pub(crate) fn registered(&self) -> bool {
        watcher::verify_sync(&self.bus, &self.state, &self.action)
    }
}

impl Drop for NativeTray {
    fn drop(&mut self) {
        watcher::stop(&self.state);
        self.cancellable.cancel();
        let quit_loop = self.main_loop.clone();
        self.main_loop.context().invoke(move || quit_loop.quit());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct Runtime {
    bus: gio::DBusConnection,
    registrations: Vec<gio::RegistrationId>,
    signals: Vec<gio::SignalSubscriptionId>,
}

impl Runtime {
    fn new(
        address: Option<&str>,
        icon: Icon,
        cancel: &gio::Cancellable,
        state: Shared,
        action: Callback,
    ) -> Result<Self, glib::Error> {
        let address = address.map(str::to_owned).map_or_else(
            || {
                gio::dbus_address_get_for_bus_sync(gio::BusType::Session, Some(cancel))
                    .map(|s| s.to_string())
            },
            Ok,
        )?;
        let bus = gio::DBusConnection::for_address_sync(
            &address,
            gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
                | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
            None,
            Some(cancel),
        )?;
        bus.set_exit_on_close(false);
        let mut runtime = Self {
            bus,
            registrations: Vec::new(),
            signals: Vec::new(),
        };
        runtime.export_item(icon, state.clone(), action.clone())?;
        runtime.export_menu(state.clone(), action.clone())?;
        runtime.signals = watcher::observe(&runtime.bus, state, action, cancel);
        Ok(runtime)
    }

    fn export_item(
        &mut self,
        icon: Icon,
        state: Shared,
        action: Callback,
    ) -> Result<(), glib::Error> {
        let info = gio::DBusNodeInfo::for_xml(interfaces::ITEM)?
            .lookup_interface(ITEM_INTERFACE)
            .expect("static item interface");
        let id = self.bus.register_object(
            ITEM_PATH,
            &info,
            move |_, _, _, _, method, parameters, invocation| {
                if !watcher::running(&state) {
                    invocation.return_dbus_error(
                        "org.freedesktop.DBus.Error.Failed",
                        "Tray is shutting down",
                    );
                    return;
                }
                let result = match method {
                    "Activate" | "SecondaryActivate" | "ContextMenu"
                        if parameters.get::<(i32, i32)>().is_some() =>
                    {
                        action(Action::Open);
                        true
                    }
                    "Scroll" => parameters
                        .get::<(i32, String)>()
                        .is_some_and(|(_, direction)| {
                            direction == "vertical" || direction == "horizontal"
                        }),
                    _ => false,
                };
                if result {
                    invocation.return_value(Some(&().to_variant()));
                } else {
                    invocation.return_dbus_error(
                        "org.freedesktop.DBus.Error.InvalidArgs",
                        "Invalid tray request",
                    );
                }
            },
            move |_, _, _, _, name| icon.property(name),
            |_, _, _, _, _, _| false,
        )?;
        self.registrations.push(id);
        Ok(())
    }

    fn export_menu(&mut self, state: Shared, action: Callback) -> Result<(), glib::Error> {
        let info = gio::DBusNodeInfo::for_xml(interfaces::MENU)?
            .lookup_interface("com.canonical.dbusmenu")
            .expect("static menu interface");
        let id = self.bus.register_object(
            MENU_PATH,
            &info,
            move |_, _, _, _, method, parameters, invocation| {
                if !watcher::running(&state) {
                    invocation.return_dbus_error(
                        "org.freedesktop.DBus.Error.Failed",
                        "Tray is shutting down",
                    );
                    return;
                }
                match menu::call(method, &parameters, action.as_ref()) {
                    Ok(reply) => invocation.return_value(Some(&reply)),
                    Err(message) => invocation
                        .return_dbus_error("org.freedesktop.DBus.Error.InvalidArgs", message),
                }
            },
            |_, _, _, _, name| menu::property(name),
            |_, _, _, _, _, _| false,
        )?;
        self.registrations.push(id);
        Ok(())
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        for signal in self.signals.drain(..) {
            self.bus.signal_unsubscribe(signal);
        }
        for registration in self.registrations.drain(..) {
            let _ = self.bus.unregister_object(registration);
        }
        let _ = self.bus.close_sync(gio::Cancellable::NONE);
    }
}
