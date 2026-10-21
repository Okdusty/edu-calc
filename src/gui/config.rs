//! qcalcp.ini: mode, layout, window, plot theme, input tuning and key bindings.
//! DEFAULT_INI is the single source of defaults; a bad user value falls back to it with a warning.
use std::path::Path;

use gtk4::{gdk, glib};

use crate::plot::{Rgb, Theme};

pub const DEFAULT_INI: &str = "\
# qcalcp GUI settings. Delete a key to get its default back.
# [ui], [layout], [window], [calc] (but keypad_side) and [theme] appearance are rewritten on exit;
# comments survive.

[ui]
# calc: calculator (math entry, live result, answer history) | octave: editor, console, workspace
mode=calc
# on-screen keypad (keypad menu in the header, F9 shows/hides; its keys: keypad.ini):
# general | programming | algebra | custom | numpad (number pad only) | none
keypad_mode=general
# number pad beside the mode's keys
number_pad=true
# keypad in Octave mode too
keypad_octave=false

[calc]
# run an answer's visualization (shaded integral, ...) right after Enter
auto_visual=true
# keypad under the entry (bottom), beside it (right), or beside it when the view is wide (auto)
keypad_side=auto
# Calculator options (options menu in the header; `set angle deg` in the entry works too)
# angle: rad | deg | gra    approx: exact | try (exact when possible) | approx
angle=rad
approx=try
# significant digits of approximate results
precision=10
# fraction: auto | decimal | fraction | mixed    notation: auto | normal | sci | eng
fraction=auto
notation=auto
# result base and input base: 2, 8, 10, 16 (any of 2-36)
base=10
input_base=10
# negative bin/hex numbers as two's complement; bits: 0 (automatic) | 8 | 16 | 32 | 64
twos_complement=true
bits=0

[layout]
# horizontal: work side | plot side;  vertical: work side above the plot
orientation=horizontal
# plot-right | plot-left
side=plot-right
# divider positions in pixels
split=700
editor_split=420
show_editor=true
show_console=true
show_workspace=true

[window]
width=1280
height=800
maximized=false

[theme]
# system (follow the desktop's light / dark setting) | light | dark
appearance=light
# Plot overrides, applied to both dark and light. Any CSS color: #1e1e22, rgb(30,30,34), white
#plot_bg=#1e1e22
#plot_fg=#dcdcdc
#plot_grid=#414148
#palette=#4da6ff;#ff7846;#fac83c;#be78e6
#font=sans-serif
#font_size=14
# editor, console and REPL font (Pango description: family [style] size)
editor_font=Monospace 11

[input]
# 3D rotation in radians per dragged pixel
rotate_speed=0.01
# zoom factor per mouse-wheel notch
zoom_step=1.15
# data cursor readout under the mouse pointer
hover_cursor=true
# reach in pixels for picking data points and for removing a pin by clicking it
pin_radius=24

[keys]
# GTK accelerators, several separated by ;  empty value = unbound
run=<Control>Return;F5
# calculator <-> Octave mode
mode=F2
# show/hide the keypad (calculator: none <-> last mode); number pad beside it
keypad=F9
number_pad=
# data editor (spreadsheet; CSV/TSV/ODS/XLSX import)
data=<Control>d
# Octave selection or current line -> calculator entry, in math view
to_calc=<Control>m
auto_visual=
new=<Control>n
open=<Control>o
save=<Control>s
save_as=<Control><Shift>s
export=<Control>e
reset_view=<Control>0
trace=<Control>t
clear_pins=<Control><Shift>p
toggle_layout=F8
swap_sides=<Shift>F8
toggle_editor=F6
toggle_console=F7
toggle_workspace=<Shift>F9
dark=<Control><Shift>d
fullscreen=F11
quit=<Control>q
";

/// Written next to qcalcp.ini on first run, as documentation.
pub const STYLE_CSS: &str = "\
/* qcalcp user CSS, reloaded on save. Reference: https://docs.gtk.org/gtk4/css-properties.html
 * Names: window.qcalcp  headerbar  .editor  .console  .repl  .workspace  .plot-bar
 *        .figure-selector  .plot  paned > separator
 *   calculator: .calc  .calc-entry  .calc-preview  .calc-bases (programming readout)  .calc-card
 *        .calc-input  .calc-result  .calc-approx  .calc-message  .calc-chip
 *   completion popup: popover.completion  .completion row  .completion .desc
 *   keypad (keypad.ini): .keypad  .keypad button  .keypad .op  .keypad .fn
 *        .keypad .general / .programming / .number-pad ... (group name)
 * Examples:
 *   .editor { font-size: 13pt; }
 *   .keypad button { min-height: 56px; }   .keypad .op { color: #4da6ff; }
 *   .console { background: #101014; }
 *   paned > separator { min-width: 14px; min-height: 14px; }   fatter touch handles
 */
";

#[derive(Clone, Debug)]
pub struct Config {
    /// [ui] mode=calc
    pub calc_mode: bool,
    /// calculator keypad: keypad::MODES name or "none"; the last shown mode (F9 brings it back)
    pub keypad_mode: &'static str,
    pub keypad_last: &'static str,
    pub number_pad: bool,
    /// keypad shown in Octave mode
    pub keypad_octave: bool,
    pub auto_visual: bool,
    /// "auto" | "bottom" | "right"
    pub keypad_side: &'static str,
    pub vertical: bool,
    pub plot_left: bool,
    pub split: i32,
    pub editor_split: i32,
    pub show_editor: bool,
    pub show_console: bool,
    pub show_workspace: bool,
    pub width: i32,
    pub height: i32,
    pub maximized: bool,
    /// appearance::MODES name
    pub appearance: &'static str,
    /// what the appearance comes out as now ("system" is resolved by the GUI)
    pub dark: bool,
    pub editor_font: String,
    pub rotate_speed: f64,
    pub zoom_step: f64,
    pub hover_cursor: bool,
    pub pin_radius: f64,
    /// [light, dark] with ini overrides applied
    pub themes: [Theme; 2],
    /// ini key (save_as) -> accelerators, validated by the GUI
    pub keys: Vec<(String, Vec<String>)>,
}

impl Config {
    pub fn theme(&self) -> &Theme {
        &self.themes[self.dark as usize]
    }

    /// Read `path`, writing the defaults (and a style.css template) on first run. Returns warnings.
    pub fn load(path: &Path) -> (Config, String) {
        let mut warn = String::new();
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(_) => {
                if let Err(e) = write_defaults(path) {
                    warn += &format!("{}: {e}\n", path.display());
                }
                DEFAULT_INI.into()
            }
        };
        let (cfg, w) = Config::parse(&text);
        (cfg, warn + &w)
    }

    pub fn parse(text: &str) -> (Config, String) {
        let mut ini = Ini { user: glib::KeyFile::new(), def: glib::KeyFile::new(), warn: String::new() };
        let _ = ini.def.load_from_data(DEFAULT_INI, glib::KeyFileFlags::NONE);
        if let Err(e) = ini.user.load_from_data(text, glib::KeyFileFlags::NONE) {
            ini.warn += &format!("qcalcp.ini: {e}; using defaults\n");
        }
        let int = |s: &str| s.parse::<i32>().ok().filter(|v| *v >= 0);
        let pos = |s: &str| s.parse::<f64>().ok().filter(|v| v.is_finite() && *v > 0.0);
        let font_size = ini.get("theme", "font_size", |s| s.parse::<u32>().ok().filter(|v| *v > 0));
        let font = ini.get("theme", "font", |s| Some(s.to_string()).filter(|s| !s.is_empty()));
        let (bg, fg, grid) = (ini.get("theme", "plot_bg", color), ini.get("theme", "plot_fg", color), ini.get("theme", "plot_grid", color));
        let palette = ini.get("theme", "palette", |s| {
            s.split(';').filter(|c| !c.trim().is_empty()).map(color).collect::<Option<Vec<_>>>().filter(|p| !p.is_empty())
        });
        // the previous version's [ui] keypad=false hid the keypad
        let hidden = ini.user.string("ui", "keypad_mode").is_err() && ini.user.boolean("ui", "keypad").is_ok_and(|on| !on);
        let mode = |s: &str| super::keypad::MODES.iter().map(|m| m.0).chain(["none"]).find(|m| *m == s);
        let keypad_mode = if hidden { "none" } else { ini.get("ui", "keypad_mode", mode).unwrap_or("general") };
        // the previous version's [theme] dark=true was its default, so it isn't carried over
        let appearance = ini.get("theme", "appearance", |s| super::appearance::MODES.iter().map(|m| m.0).find(|m| *m == s)).unwrap_or("light");
        let mut themes = [Theme::light(), Theme::dark()];
        for t in &mut themes {
            t.bg = bg.unwrap_or(t.bg);
            t.fg = fg.unwrap_or(t.fg);
            t.grid = grid.unwrap_or(t.grid);
            t.palette = palette.clone().unwrap_or(std::mem::take(&mut t.palette));
            t.font = font.clone().unwrap_or(std::mem::take(&mut t.font));
            t.font_size = font_size.unwrap_or(t.font_size);
        }
        let cfg = Config {
            calc_mode: ini.get("ui", "mode", |s| match s {
                "calc" => Some(true),
                "octave" => Some(false),
                _ => None,
            }).unwrap_or(true),
            keypad_mode,
            keypad_last: if keypad_mode == "none" { "general" } else { keypad_mode },
            number_pad: ini.get("ui", "number_pad", boolean).unwrap_or(true),
            keypad_octave: ini.get("ui", "keypad_octave", boolean).unwrap_or_default(),
            auto_visual: ini.get("calc", "auto_visual", boolean).unwrap_or(true),
            keypad_side: ini.get("calc", "keypad_side", |s| ["auto", "bottom", "right"].into_iter().find(|v| *v == s)).unwrap_or("auto"),
            vertical: ini.get("layout", "orientation", |s| match s {
                "horizontal" => Some(false),
                "vertical" => Some(true),
                _ => None,
            }).unwrap_or_default(),
            plot_left: ini.get("layout", "side", |s| match s {
                "plot-right" => Some(false),
                "plot-left" => Some(true),
                _ => None,
            }).unwrap_or_default(),
            split: ini.get("layout", "split", int).unwrap_or_default(),
            editor_split: ini.get("layout", "editor_split", int).unwrap_or_default(),
            show_editor: ini.get("layout", "show_editor", boolean).unwrap_or(true),
            show_console: ini.get("layout", "show_console", boolean).unwrap_or(true),
            show_workspace: ini.get("layout", "show_workspace", boolean).unwrap_or(true),
            width: ini.get("window", "width", int).unwrap_or_default(),
            height: ini.get("window", "height", int).unwrap_or_default(),
            maximized: ini.get("window", "maximized", boolean).unwrap_or_default(),
            appearance,
            dark: appearance == "dark",
            editor_font: ini.get("theme", "editor_font", |s| Some(s.to_string())).unwrap_or_default(),
            rotate_speed: ini.get("input", "rotate_speed", pos).unwrap_or(0.01),
            zoom_step: ini.get("input", "zoom_step", pos).unwrap_or(1.15),
            hover_cursor: ini.get("input", "hover_cursor", boolean).unwrap_or(true),
            pin_radius: ini.get("input", "pin_radius", pos).unwrap_or(24.0),
            themes,
            keys: ini.keys(),
        };
        (cfg, ini.warn)
    }

    /// Write mode/keypad/auto_visual/layout/window/appearance back into `path`, keeping the user's other keys and comments.
    pub fn save_state(&self, path: &Path) -> Result<(), String> {
        let kf = glib::KeyFile::new();
        let text = std::fs::read_to_string(path).unwrap_or_else(|_| DEFAULT_INI.into());
        // a file we can't parse is the user's; never clobber it
        kf.load_from_data(&text, glib::KeyFileFlags::KEEP_COMMENTS).map_err(|e| format!("{e}; not saving layout"))?;
        kf.set_string("ui", "mode", if self.calc_mode { "calc" } else { "octave" });
        kf.set_string("ui", "keypad_mode", self.keypad_mode);
        kf.set_boolean("ui", "number_pad", self.number_pad);
        let _ = kf.remove_key("ui", "keypad"); // the previous version's
        kf.set_boolean("ui", "keypad_octave", self.keypad_octave);
        kf.set_boolean("calc", "auto_visual", self.auto_visual);
        kf.set_string("layout", "orientation", if self.vertical { "vertical" } else { "horizontal" });
        kf.set_string("layout", "side", if self.plot_left { "plot-left" } else { "plot-right" });
        kf.set_integer("layout", "split", self.split);
        kf.set_integer("layout", "editor_split", self.editor_split);
        kf.set_boolean("layout", "show_editor", self.show_editor);
        kf.set_boolean("layout", "show_console", self.show_console);
        kf.set_boolean("layout", "show_workspace", self.show_workspace);
        kf.set_integer("window", "width", self.width);
        kf.set_integer("window", "height", self.height);
        kf.set_boolean("window", "maximized", self.maximized);
        kf.set_string("theme", "appearance", self.appearance);
        let _ = kf.remove_key("theme", "dark"); // the previous version's
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        kf.save_to_file(path).map_err(|e| e.to_string())
    }
}

/// Previous versions' default bindings that a current default has taken over.
const OLD_KEYS: &[(&str, &str)] = &[("toggle_workspace", "F9")];

fn write_defaults(path: &Path) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
        let css = d.join("style.css");
        if !css.exists() {
            std::fs::write(css, STYLE_CSS)?;
        }
    }
    std::fs::write(path, DEFAULT_INI)
}

struct Ini {
    user: glib::KeyFile,
    def: glib::KeyFile,
    warn: String,
}

impl Ini {
    /// User value if it parses (warn if it doesn't), else the DEFAULT_INI value, else None.
    fn get<T>(&mut self, group: &str, key: &str, parse: impl Fn(&str) -> Option<T>) -> Option<T> {
        if let Ok(v) = self.user.string(group, key) {
            match parse(v.trim()) {
                Some(x) => return Some(x),
                None => self.warn += &format!("qcalcp.ini: [{group}] {key}={v}: bad value, using default\n"),
            }
        }
        self.def.string(group, key).ok().and_then(|v| parse(v.trim()))
    }

    /// Default bindings, overridden per key, plus any extra keys the user added. A user value that
    /// is just a previous version's default (OLD_KEYS) now taken by another action counts as unset.
    fn keys(&self) -> Vec<(String, Vec<String>)> {
        let names = |kf: &glib::KeyFile| kf.keys("keys").map(|k| k.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap_or_default();
        let mut all = names(&self.def);
        for k in names(&self.user) {
            if !all.contains(&k) {
                all.push(k);
            }
        }
        let list = |kf: &glib::KeyFile, k: &str| kf.string_list("keys", k).ok().map(|l| l.iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect::<Vec<_>>());
        let taken = |k: &str, a: &str| names(&self.def).iter().any(|o| o != k && list(&self.def, o).is_some_and(|l| l.iter().any(|d| d == a)));
        let stale = |k: &str, l: &Vec<String>| matches!(&l[..], [a] if OLD_KEYS.contains(&(k, a.as_str())) && taken(k, a));
        all.into_iter()
            .map(|k| {
                let v = list(&self.user, &k).filter(|l| !stale(&k, l)).or_else(|| list(&self.def, &k)).unwrap_or_default();
                (k, v)
            })
            .collect()
    }
}

fn boolean(s: &str) -> Option<bool> {
    match s {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
}

/// Any CSS color GDK understands (#rgb, #rrggbb, rgb(), names); alpha ignored.
pub fn color(s: &str) -> Option<Rgb> {
    let c = gdk::RGBA::parse(s.trim()).ok()?;
    Some([c.red(), c.green(), c.blue()].map(|v| (v * 255.0).round() as u8))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors() {
        assert_eq!(color("#ff8000"), Some([255, 128, 0]));
        assert_eq!(color(" #fff "), Some([255, 255, 255]));
        assert_eq!(color("rgb(1,2,3)"), Some([1, 2, 3]));
        assert_eq!(color("nope"), None);
    }

    #[test]
    fn defaults_and_overrides() {
        let (d, w) = Config::parse(DEFAULT_INI);
        assert!(w.is_empty(), "{w}");
        assert!(!d.vertical && !d.plot_left && !d.dark && d.show_console);
        assert_eq!(d.appearance, "light");
        assert_eq!((d.split, d.width, d.zoom_step), (700, 1280, 1.15));
        assert!(d.hover_cursor && d.pin_radius == 24.0);
        assert!(d.calc_mode && !d.keypad_octave && d.auto_visual && d.keypad_side == "auto");
        assert_eq!((d.keypad_mode, d.keypad_last, d.number_pad), ("general", "general", true));
        assert!(d.keys.contains(&("data".into(), vec!["<Control>d".into()])));
        assert!(d.keys.contains(&("keypad".into(), vec!["F9".into()])));
        assert!(d.keys.contains(&("trace".into(), vec!["<Control>t".into()])));
        assert_eq!(d.themes[1].bg, Theme::dark().bg);
        assert!(d.keys.contains(&("run".into(), vec!["<Control>Return".into(), "F5".into()])));

        let ini = "[ui]\nmode=octave\n[calc]\nkeypad_side=left\n[layout]\norientation=vertical\nsplit=abc\n[theme]\nplot_bg=#000000\npalette=#ff0000;#00ff00\n\
                   font_size=-3\n[keys]\nrun=F6\nquit=\nmine=<Alt>x\n";
        let (c, w) = Config::parse(ini);
        assert!(c.vertical && !c.calc_mode && c.keypad_side == "auto");
        assert!(w.contains("keypad_side=left"));
        assert_eq!(c.split, 700); // bad value -> default
        assert!(w.contains("split") && w.contains("font_size"));
        assert_eq!(c.themes[0].bg, [0, 0, 0]);
        assert_eq!(c.themes[1].palette, vec![[255, 0, 0], [0, 255, 0]]);
        assert_eq!(c.themes[0].font_size, 14);
        let key = |k: &str| c.keys.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        assert_eq!(key("run"), Some(vec!["F6".into()]));
        assert_eq!(key("quit"), Some(vec![]));
        assert_eq!(key("mine"), Some(vec!["<Alt>x".into()]));
        assert_eq!(key("open"), Some(vec!["<Control>o".into()]));

        let (_, w) = Config::parse("not an ini");
        assert!(!w.is_empty());
    }

    #[test]
    fn previous_version() {
        // keypad=false hid the keypad; keypad_mode wins once saved
        let (c, w) = Config::parse("[ui]\nkeypad=false\n[keys]\ntoggle_workspace=F9\n");
        assert_eq!((c.keypad_mode, c.keypad_last, w.as_str()), ("none", "general", ""));
        // its toggle_workspace=F9 default gives way to keypad=F9
        let key = |c: &Config, k: &str| c.keys.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()).unwrap();
        assert_eq!(key(&c, "toggle_workspace"), ["<Shift>F9"]);
        assert_eq!(key(&c, "keypad"), ["F9"]);
        let (c, _) = Config::parse("[ui]\nkeypad=false\nkeypad_mode=programming\n[keys]\ntoggle_workspace=F8\nkeypad=F10\n");
        assert_eq!((c.keypad_mode, c.keypad_last), ("programming", "programming"));
        assert_eq!(key(&c, "toggle_workspace"), ["F8"]); // a real rebind stays (the GUI warns about the clash)
        // dark=true was the default; appearance is the new setting
        let (c, w) = Config::parse("[theme]\ndark=true\n");
        assert!(c.appearance == "light" && !c.dark && w.is_empty());
        let (c, _) = Config::parse("[theme]\nappearance=system\n");
        assert_eq!(c.appearance, "system");
        let (c, w) = Config::parse("[theme]\nappearance=blue\n");
        assert!(c.appearance == "light" && w.contains("appearance"));
        let (c, w) = Config::parse("[ui]\nkeypad_mode=sideways\n");
        assert!(c.keypad_mode == "general" && w.contains("keypad_mode"));
    }
}
