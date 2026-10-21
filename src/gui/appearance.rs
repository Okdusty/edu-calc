//! Appearance: system default | light | dark. "system" follows the desktop's dark preference:
//! the XDG desktop portal's color-scheme (live), AppsUseLightTheme on Windows, else a GTK theme
//! named *-dark.
use gtk::prelude::*;
use gtk::{gio, glib};
use gtk4 as gtk;

/// ini value, menu label
pub const MODES: [(&str, &str); 3] = [("system", "System Default"), ("light", "Light"), ("dark", "Dark")];

/// Whether `mode` comes out dark on this desktop now.
pub fn is_dark(mode: &str, portal: Option<&gio::DBusProxy>) -> bool {
    match mode {
        "dark" => true,
        "light" => false,
        _ => system_dark(portal),
    }
}

/// GTK's own light/dark switch. GTK 4.20 also reads the portal itself, so its
/// gtk-interface-color-scheme is pinned as well, or "light" would still turn dark on a dark desktop.
pub fn apply(mode: &str, dark: bool) {
    let Some(s) = gtk::Settings::default() else { return };
    s.set_gtk_application_prefer_dark_theme(dark);
    let Some(p) = s.find_property("gtk-interface-color-scheme") else { return };
    let Some(class) = glib::EnumClass::with_type(p.value_type()) else { return };
    // GtkInterfaceColorScheme: DEFAULT 1, DARK 2, LIGHT 3
    let n = match mode {
        "dark" => 2,
        "light" => 3,
        _ => 1,
    };
    if let Some(v) = class.to_value(n) {
        s.set_property_from_value("gtk-interface-color-scheme", &v);
    }
}

/// The portal's Settings interface, when there is a portal.
pub fn portal() -> Option<gio::DBusProxy> {
    if cfg!(windows) {
        return None;
    }
    gio::DBusProxy::for_bus_sync(
        gio::BusType::Session,
        gio::DBusProxyFlags::DO_NOT_LOAD_PROPERTIES,
        None,
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.Settings",
        None::<&gio::Cancellable>,
    )
    .ok()
}

/// Run `f` when the desktop's color scheme changes.
pub fn on_change(portal: &gio::DBusProxy, f: impl Fn() + 'static) {
    portal.connect_local("g-signal", false, move |args| {
        let name = args.get(2).and_then(|v| v.get::<String>().ok());
        let params = args.get(3).and_then(|v| v.get::<glib::Variant>().ok());
        let key = params.as_ref().filter(|p| p.n_children() >= 2).map(|p| (p.child_value(0).str().map(String::from), p.child_value(1).str().map(String::from)));
        if name.as_deref() == Some("SettingChanged") && key == Some((Some("org.freedesktop.appearance".into()), Some("color-scheme".into()))) {
            f();
        }
        None
    });
}

fn system_dark(portal: Option<&gio::DBusProxy>) -> bool {
    // color-scheme: 0 no preference, 1 prefer dark, 2 prefer light
    match portal.and_then(portal_scheme) {
        Some(1) => return true,
        Some(2) => return false,
        _ => {}
    }
    if let Some(dark) = windows_dark() {
        return dark;
    }
    gtk::Settings::default().and_then(|s| s.gtk_theme_name()).is_some_and(|t| t.to_lowercase().ends_with("-dark"))
}

fn portal_scheme(p: &gio::DBusProxy) -> Option<u32> {
    let args = ("org.freedesktop.appearance", "color-scheme").to_variant();
    // ReadOne since portal version 2; Read wraps the value in one more variant
    let r = ["ReadOne", "Read"].iter().find_map(|m| p.call_sync(m, Some(&args), gio::DBusCallFlags::NONE, 500, None::<&gio::Cancellable>).ok())?;
    let mut v = r.child_value(0);
    while let Some(inner) = v.as_variant() {
        v = inner;
    }
    v.get::<u32>()
}

#[cfg(windows)]
fn windows_dark() -> Option<bool> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = std::process::Command::new("reg")
        .args(["query", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize", "/v", "AppsUseLightTheme"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let value = text.lines().find(|l| l.contains("AppsUseLightTheme"))?.split_whitespace().last()?;
    Some(value == "0x0")
}

#[cfg(not(windows))]
fn windows_dark() -> Option<bool> {
    None
}
