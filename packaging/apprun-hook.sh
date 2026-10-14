# AppRun hook, sourced after linuxdeploy-plugin-gtk's (alphabetical order). That one forces X11 and
# a light/dark Adwaita variant guessed from GNOME settings; qcalcp runs on Wayland natively (touch
# gestures) and switches dark/light itself.
if [ -n "${WAYLAND_DISPLAY:-}" ]; then
    unset GDK_BACKEND
fi
unset GTK_THEME
