//! Embedded icons work without relying on the host's theme or installation path.
use gtk4 as gtk;

pub(super) fn install() {
    static REGISTER: std::sync::Once = std::sync::Once::new();
    REGISTER.call_once(|| {
        gtk::glib::set_application_name("EduCalculator");
        gtk::gio::resources_register_include!("qcalcp.gresource").expect("embedded application icons");
    });
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::IconTheme::for_display(&display).add_resource_path("/com/dusty/qcalcp/icons");
    }
    gtk::Window::set_default_icon_name("qcalcp");
}
