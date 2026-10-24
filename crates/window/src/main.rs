//! cranked-audio: the window. Shows the service's knobs live and sends on-screen turns and presses back.
//! It holds no state of its own; closing it frees everything and the service keeps running.

mod editor;
mod themes;

use std::cell::RefCell;
use std::f64::consts::PI;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cranked_ipc::settings::{Channel, Config, LedBlend, PopupStyle};
use cranked_ipc::{DeviceChoices, Entity, Knob, LearnedControl, ServerMessage, Snapshot};
use slint::{Color, ComponentHandle, Model, ModelRc, SharedString, VecModel};

slint::include_modules!();

/// Dial arcs: a 270° sweep from bottom-left, clockwise, in a 100 x 100 box.
const ARC_START: f64 = 135.0;
const ARC_SWEEP: f64 = 270.0;
const ARC_RADIUS: f64 = 44.0;

fn point(degrees: f64) -> (f64, f64) {
    let radians = degrees * PI / 180.0;
    (50.0 + ARC_RADIUS * radians.cos(), 50.0 + ARC_RADIUS * radians.sin())
}

fn arc(from: f64, to: f64) -> String {
    if to - from < 0.5 {
        return "M 50 50".into();
    }
    let ((x0, y0), (x1, y1)) = (point(from), point(to));
    let large = if to - from > 180.0 { 1 } else { 0 };
    format!("M {x0:.2} {y0:.2} A {ARC_RADIUS} {ARC_RADIUS} 0 {large} 1 {x1:.2} {y1:.2}")
}

/// (filled shapes, lines) for the button's press action, 24 x 24.
fn icon(action: &str) -> (&'static str, &'static str) {
    let speaker = "M 2 9 L 6 9 L 11 4 L 11 20 L 6 15 L 2 15 Z";
    match action {
        "Play/Pause" => ("M 2 5 L 2 19 L 11 12 Z", "M 15 5 L 15 19 M 20 5 L 20 19"),
        "Next" => ("M 2 5 L 2 19 L 10 12 Z M 10 5 L 10 19 L 18 12 Z", "M 20.5 5 L 20.5 19"),
        "Previous" => ("M 22 5 L 22 19 L 14 12 Z M 14 5 L 14 19 L 6 12 Z", "M 3.5 5 L 3.5 19"),
        "Mute" => (speaker, "M 15 9 L 21 15 M 21 9 L 15 15"),
        "Switch output" => (speaker, "M 14.5 8.5 Q 17 12 14.5 15.5 M 17.5 5.5 Q 22 12 17.5 18.5"),
        "Next profile" => ("", "M 4 6 L 20 6 M 4 12 L 20 12 M 4 18 L 14 18"),
        "Nothing" => ("", ""),
        // Home Assistant toggles and anything else: a power symbol
        _ => ("", "M 12 3 L 12 11 M 7.5 6 A 7.5 7.5 0 1 0 16.5 6"),
    }
}

fn color(hex: &str) -> Color {
    let value = hex.trim().strip_prefix('#').and_then(|h| u32::from_str_radix(h, 16).ok()).unwrap_or(0x808080);
    Color::from_rgb_u8((value >> 16) as u8, (value >> 8) as u8, value as u8)
}

// The theme in use (dial colours are mixed toward its card colour).
thread_local! {
    static THEME: RefCell<themes::Theme> = RefCell::new(themes::named("cranked"));
}

thread_local! {
    // knob numbers (from 1) the window leaves out (Settings > Look)
    static HIDDEN: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}

/// Shows the window the first time there's something in it, so it opens at the size its contents want
/// (compositors don't let a window that's already up resize itself).
fn open_window(window: &MainWindow) {
    if !window.window().is_visible() {
        let _ = window.show();
    }
}

/// Resizes the window to what its contents want (after the knobs arrive or the look changes); later the user's own
/// resizing is left alone.
fn fit_window(window: &MainWindow) {
    let weak = window.as_weak();
    // after the layout has taken the new contents into account
    slint::Timer::single_shot(Duration::from_millis(30), move || {
        let Some(w) = weak.upgrade() else { return };
        if w.get_page() == 0 {
            w.window().set_size(slint::LogicalSize::new(w.get_wanted_width(), w.get_wanted_height()));
        }
    });
}

/// The Look settings as the Settings page has them.
fn look_from(general: &General) -> cranked_ipc::settings::Look {
    cranked_ipc::settings::Look {
        style: match general.get_look_style() {
            1 => "sliders",
            2 => "faders",
            _ => "knobs",
        }
        .into(),
        scale: general.get_look_scale().round().clamp(30.0, 160.0) as u8,
        show_name: general.get_look_show_name(),
        show_percent: general.get_look_show_percent(),
        show_source: general.get_look_show_source(),
        show_button: general.get_look_show_button(),
        show_action_labels: general.get_look_show_action_labels(),
        buttons_in_dial: general.get_look_buttons_in_dial(),
        show_header: general.get_look_show_header(),
        hidden_knobs: general
            .get_look_shown()
            .iter()
            .enumerate()
            .filter(|(_, shown)| !shown)
            .map(|(i, _)| i + 1)
            .collect(),
    }
}

/// Applies the window's Look settings (size, style, what shows, which knobs).
fn apply_look(window: &MainWindow, look: &cranked_ipc::settings::Look) {
    let l = window.global::<Look>();
    l.set_scale(f32::from(look.scale.clamp(30, 160)) / 100.0);
    l.set_style(match look.style.as_str() {
        "sliders" => 1,
        "faders" => 2,
        _ => 0,
    });
    l.set_show_name(look.show_name);
    l.set_show_percent(look.show_percent);
    l.set_show_source(look.show_source);
    l.set_show_button(look.show_button);
    l.set_show_action_labels(look.show_action_labels);
    l.set_buttons_in_dial(look.buttons_in_dial);
    l.set_show_header(look.show_header);
    HIDDEN.with(|h| *h.borrow_mut() = look.hidden_knobs.clone());
    fit_window(window);
}

fn current_theme() -> themes::Theme {
    THEME.with(|t| *t.borrow())
}

/// Sets every colour role in the window from a theme.
fn apply_theme(window: &MainWindow, name: &str) {
    let theme = themes::named(name);
    THEME.with(|t| *t.borrow_mut() = theme);
    let p = window.global::<Palette>();
    p.set_background(theme.background);
    p.set_card(theme.card);
    p.set_centre(theme.centre);
    p.set_centre_hover(theme.centre_hover);
    p.set_track(theme.track);
    p.set_button(theme.button);
    p.set_button_hover(theme.button_hover);
    p.set_edge(theme.edge);
    p.set_text(theme.text);
    p.set_text_strong(theme.text_strong);
    p.set_text_dim(theme.text_dim);
    p.set_subtle(theme.subtle);
    p.set_faint(theme.faint);
    p.set_hint(theme.hint);
    p.set_list(theme.list);
    p.set_selected(theme.selected);
    p.set_selected_hover(theme.selected_hover);
    p.set_check(theme.check);
    p.set_ok(theme.ok);
    // the built-in widgets (buttons, lists) follow light or dark too
    let light =
        u32::from(theme.background.red()) + u32::from(theme.background.green()) + u32::from(theme.background.blue())
            > 384;
    window.global::<StdPalette>().set_color_scheme(if light {
        slint::language::ColorScheme::Light
    } else {
        slint::language::ColorScheme::Dark
    });
}

/// `amount` of `color` over `under`, as one solid colour.
fn mix(color: Color, under: Color, amount: f32) -> Color {
    let blend = |a: u8, b: u8| (f32::from(a) * amount + f32::from(b) * (1.0 - amount)).round() as u8;
    Color::from_rgb_u8(
        blend(color.red(), under.red()),
        blend(color.green(), under.green()),
        blend(color.blue(), under.blue()),
    )
}

fn row(index: usize, knob: &Knob) -> KnobData {
    let level = knob.level.map_or(-1, i32::from);
    let split = ARC_START + ARC_SWEEP * f64::from(level.max(0)) / 100.0;
    // the press action's icon, or the hold action's when pressing does nothing
    let (icon_fill, icon_lines) = icon(if knob.press == "Nothing" { &knob.hold } else { &knob.press });
    KnobData {
        index: index as i32,
        name: knob.name.as_str().into(),
        level,
        fill_arc: arc(ARC_START, split).into(),
        rest_arc: arc(split, ARC_START + ARC_SWEEP).into(),
        fill_color: if knob.muted {
            mix(color(&knob.muted_color), Color::from_rgb_u8(255, 255, 255), 0.6)
        } else if knob.lowered {
            mix(color(&knob.color), current_theme().card, 0.55)
        } else {
            color(&knob.color)
        },
        rest_color: match color(&knob.background) {
            c if (c.red(), c.green(), c.blue()) == (0, 0, 0) => current_theme().track,
            c => mix(c, current_theme().card, 0.55),
        },
        muted: knob.muted,
        lowered: knob.lowered,
        source: knob.source.as_str().into(),
        press: knob.press.as_str().into(),
        hold: knob.hold.as_str().into(),
        held: knob.held,
        icon_fill: icon_fill.into(),
        icon_lines: icon_lines.into(),
    }
}

/// The connection to the service, shared by the reader thread and the UI callbacks.
#[derive(Default)]
struct Link {
    stream: Option<UnixStream>,
    snapshot: Snapshot,
    config: Option<Config>,
    entities: Vec<Entity>,
    /// Last value sent per knob, so fast scrolling steps from it instead of from a slightly old snapshot.
    sent: [Option<(u8, Instant)>; 5],
}

impl Link {
    fn send(&mut self, line: &str) {
        if let Some(stream) = &mut self.stream {
            if writeln!(stream, "{line}").is_err() {
                self.stream = None;
            }
        }
    }

    fn set_knob(&mut self, index: usize, percent: u8) {
        if self.sent.get(index).copied().flatten().is_some_and(|(p, _)| p == percent) {
            return;
        }
        self.send(&format!("knob {index} {percent}"));
        if let Some(slot) = self.sent.get_mut(index) {
            *slot = Some((percent, Instant::now()));
        }
    }

    fn current(&self, index: usize) -> u8 {
        match self.sent.get(index).copied().flatten() {
            Some((percent, at)) if at.elapsed() < Duration::from_millis(600) => percent,
            _ => self.snapshot.knobs.get(index).and_then(|k| k.level).unwrap_or(0),
        }
    }
}

/// Settings being edited (main thread only): the ids behind the Home Assistant pickers.
#[derive(Default)]
struct Editor {
    lights: Vec<String>,
    toggles: Vec<String>,
    /// What the app list offers besides this knob's own apps.
    known_apps: Vec<String>,
    /// The full app list for the knob being edited (its order is fixed while the page is open).
    all_rows: Vec<AppRow>,
    playing: Vec<String>,
    other_knobs: Vec<(String, Vec<String>)>,
}

thread_local! {
    static EDITOR: RefCell<Editor> = RefCell::default();
}

fn strings(items: impl IntoIterator<Item = impl Into<SharedString>>) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(items.into_iter().map(Into::into).collect::<Vec<SharedString>>()))
}

fn active_profile(link: &Link) -> usize {
    let config = link.config.as_ref();
    config.and_then(|c| c.profiles.iter().position(|p| p.name == link.snapshot.profile)).unwrap_or(0)
}

/// Fills the Home Assistant pickers, keeping whatever entities the knob uses selected.
fn fill_entity_lists(
    window: &MainWindow,
    entities: &[Entity],
    light: Option<&str>,
    press: Option<&str>,
    hold: Option<&str>,
) {
    let lists = window.global::<Lists>();
    let knob = window.global::<KnobSettings>();
    let (light_labels, light_ids) = editor::choices(entities, &["light"], "(none)", &[light.unwrap_or_default()]);
    let keep = [press.unwrap_or_default(), hold.unwrap_or_default()];
    let (toggle_labels, toggle_ids) = editor::choices(entities, &[], "(pick one)", &keep);
    lists.set_lights(strings(light_labels));
    lists.set_toggles(strings(toggle_labels));
    knob.set_light_index(editor::index_of(&light_ids, light) as i32);
    knob.set_press_entity(editor::index_of(&toggle_ids, press) as i32);
    knob.set_hold_entity(editor::index_of(&toggle_ids, hold) as i32);
    EDITOR.with(|e| {
        let mut e = e.borrow_mut();
        e.lights = light_ids;
        e.toggles = toggle_ids;
    });
}

/// The entity ids the knob page currently has selected.
fn selected_entities(window: &MainWindow) -> (Option<String>, Option<String>, Option<String>) {
    let knob = window.global::<KnobSettings>();
    EDITOR.with(|e| {
        let e = e.borrow();
        let pick = |ids: &[String], index: i32| ids.get(index as usize).filter(|id| !id.is_empty()).cloned();
        (
            pick(&e.lights, knob.get_light_index()),
            pick(&e.toggles, knob.get_press_entity()),
            pick(&e.toggles, knob.get_hold_entity()),
        )
    })
}

/// Builds the full app list for the knob being edited, then shows it.
fn build_app_rows(window: &MainWindow) {
    let mine: Vec<String> = window.global::<KnobSettings>().get_apps().iter().map(|a| a.to_string()).collect();
    EDITOR.with(|e| {
        let mut e = e.borrow_mut();
        let rows = editor::app_choices(&mine, &e.known_apps, &e.playing, &e.other_knobs);
        e.all_rows = rows
            .into_iter()
            .map(|r| AppRow {
                name: r.name.into(),
                selected: r.selected,
                playing: r.playing,
                elsewhere: r.elsewhere.into(),
            })
            .collect();
    });
    show_app_rows(window);
}

/// Hands the list the rows to show: all of them, or (collapsed) just this knob's.
fn show_app_rows(window: &MainWindow) {
    let knob = window.global::<KnobSettings>();
    let expanded = knob.get_apps_expanded();
    let (rows, total) = EDITOR.with(|e| {
        let e = e.borrow();
        let rows: Vec<AppRow> = e.all_rows.iter().filter(|r| expanded || r.selected).cloned().collect();
        (rows, e.all_rows.len())
    });
    knob.set_app_total(total as i32);
    knob.set_app_rows(ModelRc::new(VecModel::from(rows)));
}

/// Marks `name` on or off this knob in the full list (adding it at the top if it's new).
fn mark_app(name: &str, selected: bool) {
    EDITOR.with(|e| {
        let mut e = e.borrow_mut();
        match e.all_rows.iter_mut().find(|r| r.name.eq_ignore_ascii_case(name)) {
            Some(row) => row.selected = selected,
            None => e.all_rows.insert(0, AppRow { name: name.into(), selected, playing: false, elsewhere: "".into() }),
        }
    });
}

fn load_knob(window: &MainWindow, link: &Link, index: usize) -> bool {
    let Some(config) = &link.config else { return false };
    let Some(channel) = config.profiles.get(active_profile(link)).and_then(|p| p.channels.get(index)) else {
        return false;
    };
    let knob = window.global::<KnobSettings>();
    knob.set_index(index as i32);
    knob.set_name(channel.name.as_str().into());
    knob.set_apps(strings(channel.apps.iter().map(String::as_str)));
    let profile = &config.profiles[active_profile(link)];
    let other_knobs = profile
        .channels
        .iter()
        .enumerate()
        .filter(|(i, c)| *i != index && !c.apps.is_empty())
        .map(|(i, c)| (c.label(i), c.apps.clone()))
        .collect();
    EDITOR.with(|e| {
        let mut e = e.borrow_mut();
        e.known_apps = link.snapshot.known_apps.clone();
        e.playing = link.snapshot.playing.clone();
        e.other_knobs = other_knobs;
    });
    // collapsed shows just this knob's apps; a knob with none opens on the full list
    knob.set_apps_expanded(channel.apps.is_empty());
    build_app_rows(window);
    knob.set_kind(if channel.light.is_some() {
        2
    } else if channel.master {
        1
    } else {
        0
    });
    knob.set_catch_all(channel.catch_all);
    knob.set_min(i32::from(channel.min));
    knob.set_max(i32::from(channel.max));
    knob.set_color(channel.color.as_str().into());
    knob.set_background(channel.background.as_str().into());
    knob.set_muted_color(channel.muted_color.as_str().into());
    knob.set_press_kind(editor::action_index(&channel.press) as i32);
    knob.set_hold_kind(editor::action_index(&channel.hold) as i32);
    window.global::<Lists>().set_playing(strings(link.snapshot.playing.iter().map(String::as_str)));
    fill_entity_lists(
        window,
        &link.entities,
        channel.light.as_deref(),
        editor::action_entity(&channel.press),
        editor::action_entity(&channel.hold),
    );
    true
}

fn store_knob(window: &MainWindow, channel: &mut Channel) {
    let knob = window.global::<KnobSettings>();
    let (light, press, hold) = selected_entities(window);
    channel.name = knob.get_name().trim().to_string();
    channel.apps = knob.get_apps().iter().map(|a| a.to_string()).collect();
    let kind = knob.get_kind();
    channel.master = kind == 1;
    channel.light = if kind == 2 { light } else { None };
    channel.catch_all = kind == 0 && knob.get_catch_all();
    channel.min = knob.get_min().clamp(0, 100) as u8;
    channel.max = knob.get_max().clamp(0, 100) as u8;
    channel.color = knob.get_color().trim().to_string();
    channel.background = knob.get_background().trim().to_string();
    channel.muted_color = knob.get_muted_color().trim().to_string();
    channel.press = editor::action_from(knob.get_press_kind() as usize, press);
    channel.hold = editor::action_from(knob.get_hold_kind() as usize, hold);
}

fn load_general(window: &MainWindow, link: &Link) -> bool {
    let Some(config) = &link.config else { return false };
    let profile_index = active_profile(link);
    let profile = &config.profiles[profile_index];
    let general = window.global::<General>();
    general.set_popups(config.popups);
    let look = &config.look;
    general.set_look_style(match look.style.as_str() {
        "sliders" => 1,
        "faders" => 2,
        _ => 0,
    });
    general.set_look_scale(f32::from(look.scale));
    general.set_look_show_name(look.show_name);
    general.set_look_show_percent(look.show_percent);
    general.set_look_show_source(look.show_source);
    general.set_look_show_button(look.show_button);
    general.set_look_show_action_labels(look.show_action_labels);
    general.set_look_buttons_in_dial(look.buttons_in_dial);
    general.set_look_show_header(look.show_header);
    let profile_channels = config.profiles.get(profile_index).map_or(0, |p| p.channels.len());
    general.set_look_shown(ModelRc::new(VecModel::from(
        (1..=profile_channels).map(|k| !look.hidden_knobs.contains(&k)).collect::<Vec<bool>>(),
    )));
    general.set_theme_index(themes::NAMES.iter().position(|(n, _)| *n == config.theme).unwrap_or(0) as i32);
    general.set_popup_style(match config.popup_style {
        PopupStyle::Auto => 0,
        PopupStyle::Island => 1,
        PopupStyle::Kde => 2,
        PopupStyle::Notifications => 3,
    });
    general.set_brightness(f32::from(config.leds.brightness));
    general.set_led_blend(match config.leds.blend {
        LedBlend::Classic => 0,
        LedBlend::Smooth => 1,
        LedBlend::Clean => 2,
    });
    general.set_dim_idle(config.leds.dim_idle);
    let auto = &profile.auto_lower;
    general.set_auto_enabled(auto.enabled);
    let flags =
        |list: &[usize]| ModelRc::new(VecModel::from((1..=5).map(|k| list.contains(&k)).collect::<Vec<bool>>()));
    general.set_lower(flags(&auto.lower));
    general.set_triggers(flags(&auto.triggers));
    general.set_voice_chat(auto.voice_chat);
    general.set_amount_index(editor::amount_index(auto.amount) as i32);
    general.set_auto_dim(auto.dim_leds);
    general.set_ha_url(config.home_assistant.url.clone().unwrap_or_default().into());
    general.set_ha_token("".into());
    general.set_profile_index(profile_index as i32);
    general.set_profile_name(profile.name.as_str().into());
    window.global::<Lists>().set_profiles(strings(config.profiles.iter().map(|p| p.name.as_str())));
    true
}

fn store_general(window: &MainWindow, config: &mut Config, profile_index: usize) {
    let general = window.global::<General>();
    config.popups = general.get_popups();
    config.look = look_from(&general);
    config.theme =
        themes::NAMES.get(general.get_theme_index().max(0) as usize).map_or("cranked", |(n, _)| n).to_string();
    config.popup_style = match general.get_popup_style() {
        1 => PopupStyle::Island,
        2 => PopupStyle::Kde,
        3 => PopupStyle::Notifications,
        _ => PopupStyle::Auto,
    };
    config.leds.brightness = general.get_brightness().round().clamp(0.0, 100.0) as u8;
    config.leds.blend = match general.get_led_blend() {
        1 => LedBlend::Smooth,
        2 => LedBlend::Clean,
        _ => LedBlend::Classic,
    };
    config.leds.dim_idle = general.get_dim_idle();
    let url = general.get_ha_url().trim().to_string();
    config.home_assistant.url = (!url.is_empty()).then_some(url);
    if let Some(profile) = config.profiles.get_mut(profile_index) {
        let knobs = |model: ModelRc<bool>| model.iter().enumerate().filter(|(_, on)| *on).map(|(i, _)| i + 1).collect();
        let auto = &mut profile.auto_lower;
        auto.enabled = general.get_auto_enabled();
        auto.lower = knobs(general.get_lower());
        auto.triggers = knobs(general.get_triggers());
        auto.voice_chat = general.get_voice_chat();
        auto.amount = editor::AMOUNTS[(general.get_amount_index().max(0) as usize).min(editor::AMOUNTS.len() - 1)];
        auto.dim_leds = general.get_auto_dim();
    }
}

fn send_config(link: &mut Link, config: Config) {
    if let Ok(json) = serde_json::to_string(&config) {
        link.send(&format!("config {json}"));
        link.config = Some(config);
    }
}

thread_local! {
    /// What Settings > Devices can add (from the service) and the MIDI control last learned.
    static CHOICES: RefCell<DeviceChoices> = RefCell::new(DeviceChoices::default());
    static LEARNED: RefCell<Option<LearnedControl>> = const { RefCell::new(None) };
    /// keyboards already set up as keyboard knobs (not offered again), and the ones the picker shows
    static ADDED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static SHOWN_KEYBOARDS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// The keyboard picker: keyboards not already added, the ones that type marked (they're never taken over).
fn fill_keyboards(window: &MainWindow) {
    let devices = window.global::<Devices>();
    let (shown, labels): (Vec<String>, Vec<String>) = CHOICES.with(|c| {
        let choices = c.borrow();
        ADDED.with(|added| {
            let added = added.borrow();
            choices
                .keyboards
                .iter()
                .filter(|k| !added.contains(k))
                .map(|k| {
                    let label = if choices.typing.contains(k) { format!("{k} (also types)") } else { k.clone() };
                    (k.clone(), label)
                })
                .unzip()
        })
    });
    devices.set_keyboards(strings(labels.iter().map(String::as_str)));
    devices.set_keyboard_index(devices.get_keyboard_index().clamp(0, shown.len().saturating_sub(1) as i32));
    SHOWN_KEYBOARDS.with(|s| *s.borrow_mut() = shown);
}

fn show_device_choices(window: &MainWindow, choices: &DeviceChoices) {
    let devices = window.global::<Devices>();
    devices.set_serial_ports(strings(choices.serial.iter().map(String::as_str)));
    let clamp = |index: i32, len: usize| index.clamp(0, len.saturating_sub(1) as i32);
    devices.set_deej_port(clamp(devices.get_deej_port(), choices.serial.len()));
    if !devices.get_learning() && !devices.get_learned() {
        devices.set_learn_status(
            match choices.midi.len() {
                0 => "No MIDI controllers found. Plug one in, then click Look for devices again.".to_string(),
                _ => format!("Found: {}", choices.midi.join(", ")),
            }
            .into(),
        );
    }
    CHOICES.with(|c| *c.borrow_mut() = choices.clone());
    fill_keyboards(window);
}

fn show_learned(window: &MainWindow, learned: Option<LearnedControl>) {
    let devices = window.global::<Devices>();
    devices.set_learning(false);
    devices.set_learned(learned.is_some());
    devices.set_learn_status(
        match &learned {
            Some(l) => format!("Heard {} {} on channel {} from {}", l.kind.to_uppercase(), l.number, l.channel, l.port),
            None => "Nothing moved. Click Learn and try again.".to_string(),
        }
        .into(),
    );
    LEARNED.with(|l| *l.borrow_mut() = learned);
}

/// Settings > Devices: the configured devices, one line each.
fn show_devices(window: &MainWindow, link: &Link) {
    let Some(config) = &link.config else { return };
    let names: Vec<String> = config.profiles.get(active_profile(link)).map_or_else(Vec::new, |p| {
        p.channels
            .iter()
            .map(|c| if c.master { format!("{} (output volume)", c.name) } else { c.name.clone() })
            .collect()
    });
    let knob = |k: usize| names.get(k.wrapping_sub(1)).cloned().unwrap_or_else(|| format!("knob {k}"));
    let lines: Vec<String> = config.devices.iter().map(|d| editor::device_summary(d, &knob)).collect();
    window.global::<Devices>().set_summaries(strings(lines.iter().map(String::as_str)));
    let added = config
        .devices
        .iter()
        .filter_map(|d| match d {
            cranked_ipc::settings::Device::KeyboardKnob { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect();
    ADDED.with(|a| *a.borrow_mut() = added);
    fill_keyboards(window);
}

/// Settings > Devices: adding and removing keyboard knobs, MIDI controls and deej boxes.
fn devices_page(window: &MainWindow, link: &Arc<Mutex<Link>>) {
    use cranked_ipc::settings::{Device, MidiControl};
    let page = window.global::<Devices>();
    // runs `change` on a copy of the config, then saves it and redraws the list
    let edit = |change: fn(&MainWindow, &mut Vec<Device>, i32) -> bool| {
        let (link, weak) = (Arc::clone(link), window.as_weak());
        move |arg: i32| {
            let (Ok(mut link), Some(w)) = (link.lock(), weak.upgrade()) else { return };
            let Some(mut config) = link.config.clone() else { return };
            if change(&w, &mut config.devices, arg) {
                send_config(&mut link, config);
                show_devices(&w, &link);
            }
        }
    };
    page.on_remove(edit(|_, devices, index| {
        let index = index as usize;
        (index < devices.len()).then(|| devices.remove(index)).is_some()
    }));
    let add_keyboard = edit(|w, devices, _| {
        let page = w.global::<Devices>();
        let Some(name) = SHOWN_KEYBOARDS.with(|s| s.borrow().get(page.get_keyboard_index().max(0) as usize).cloned())
        else {
            return false;
        };
        devices.push(Device::KeyboardKnob {
            name,
            knob: page.get_keyboard_knob().max(0) as usize + 1,
            step: page.get_keyboard_step().clamp(1, 20) as u8,
            grab: page.get_keyboard_grab(),
        });
        true
    });
    page.on_add_keyboard(move || add_keyboard(0));
    let add_midi = edit(|w, devices, _| {
        let page = w.global::<Devices>();
        let Some(learned) = LEARNED.with(|l| l.borrow_mut().take()) else { return false };
        let button = page.get_midi_button();
        editor::add_midi_control(
            devices,
            &learned.port,
            MidiControl {
                channel: learned.channel,
                kind: learned.kind,
                number: learned.number,
                knob: page.get_midi_knob().max(0) as usize + 1,
                button,
                relative: page.get_midi_relative() && !button,
            },
        );
        page.set_learned(false);
        page.set_learn_status("Added. Click Learn to map another control.".into());
        true
    });
    page.on_add_midi(move || add_midi(0));
    let add_deej = edit(|w, devices, _| {
        let page = w.global::<Devices>();
        let Some(port) = CHOICES.with(|c| c.borrow().serial.get(page.get_deej_port().max(0) as usize).cloned()) else {
            return false;
        };
        let first = page.get_deej_first().max(0) as usize + 1;
        let count = page.get_deej_count().clamp(1, 16) as usize;
        let knobs = (first..first + count).map(|k| k.min(cranked_ipc::settings::MAX_CONTROLS)).collect();
        devices.push(Device::Deej { port, baud: 9600, knobs, invert: page.get_deej_invert() });
        true
    });
    page.on_add_deej(move || add_deej(0));
    {
        let (link, weak) = (Arc::clone(link), window.as_weak());
        page.on_learn(move || {
            let (Ok(mut link), Some(w)) = (link.lock(), weak.upgrade()) else { return };
            let page = w.global::<Devices>();
            page.set_learning(true);
            page.set_learned(false);
            page.set_learn_status("Move a knob or fader, or press a button, on your controller…".into());
            link.send("learn-midi");
        });
    }
    {
        let link = Arc::clone(link);
        page.on_refresh(move || {
            if let Ok(mut link) = link.lock() {
                link.send("list-devices");
            }
        });
    }
}

fn sibling(name: &str) -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join(name))).unwrap_or_else(|| name.into())
}

fn config_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("cranked-audio").join("config.toml")
}

/// Opens config.toml in the user's editor, writing the defaults first if there's no file yet.
fn open_settings() {
    let path = config_path();
    if !path.exists() {
        if let Ok(output) = Command::new(sibling("cranked-audiod")).arg("--print-config").output() {
            let _ = std::fs::create_dir_all(path.parent().unwrap_or(&path));
            let _ = std::fs::write(&path, output.stdout);
        }
    }
    let _ =
        Command::new("xdg-open").arg(&path).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
}

fn show(window: &MainWindow, snapshot: &Snapshot) {
    let hidden = HIDDEN.with(|h| h.borrow().clone());
    let rows: Vec<KnobData> = snapshot
        .knobs
        .iter()
        .enumerate()
        .filter(|(i, _)| !hidden.contains(&(i + 1)))
        .map(|(i, knob)| row(i, knob))
        .collect();
    let model = window.get_knobs();
    match model.as_any().downcast_ref::<VecModel<KnobData>>() {
        // same knobs: update in place so a knob being dragged keeps its touch state
        Some(model) if model.row_count() == rows.len() => {
            for (i, data) in rows.into_iter().enumerate() {
                if model.row_data(i).as_ref() != Some(&data) {
                    model.set_row_data(i, data);
                }
            }
        }
        _ => {
            window.set_knobs(ModelRc::new(VecModel::from(rows)));
            fit_window(window);
        }
    }
    window.set_profile(snapshot.profile.as_str().into());
    let lists = window.global::<Lists>();
    lists.set_home_assistant(snapshot.home_assistant);
    lists.set_knob_names(strings(snapshot.knobs.iter().map(|k| k.name.as_str())));
    window.set_ok(true);
    let status = if snapshot.mixer { "Mixer connected" } else { "On-screen controls" };
    window.set_status(SharedString::from(status));
}

fn set_status(window: &slint::Weak<MainWindow>, text: &'static str) {
    let _ = window.upgrade_in_event_loop(move |w| {
        w.set_ok(false);
        w.set_status(text.into());
    });
}

/// Connects (starting the service once if it isn't running), then forwards snapshots to the window. Reconnects.
fn run_link(link: Arc<Mutex<Link>>, window: slint::Weak<MainWindow>) {
    let mut started = false;
    loop {
        let stream = match UnixStream::connect(cranked_ipc::socket_path()) {
            Ok(stream) => {
                started = true;
                stream
            }
            Err(_) => {
                if !started {
                    started = true;
                    set_status(&window, "Starting the service…");
                    let started_unit = std::env::var_os("CRANKED_SOCKET").is_none()
                        && Command::new("systemctl")
                            .args(["--user", "start", "cranked-audio.service"])
                            .stdin(Stdio::null())
                            .stdout(Stdio::null())
                            .stderr(Stdio::null())
                            .status()
                            .is_ok_and(|s| s.success());
                    if !started_unit {
                        let _ = Command::new(sibling("cranked-audiod"))
                            .stdin(Stdio::null())
                            .stdout(Stdio::null())
                            .stderr(Stdio::null())
                            .spawn();
                    }
                } else {
                    set_status(&window, "Service not running");
                }
                std::thread::sleep(Duration::from_millis(500));
                continue;
            }
        };
        let Ok(reader) = stream.try_clone() else { continue };
        if let Ok(mut link) = link.lock() {
            link.stream = Some(stream);
        }
        if let Ok(mut link) = link.lock() {
            link.send("get-config");
        }
        for line in BufReader::new(reader).lines().map_while(Result::ok) {
            let Ok(message) = serde_json::from_str::<ServerMessage>(&line) else { continue };
            match message {
                ServerMessage::Snapshot(snapshot) => {
                    if let Ok(mut link) = link.lock() {
                        link.snapshot = snapshot.clone();
                    }
                    let _ = window.upgrade_in_event_loop(move |w| {
                        if !snapshot.version.is_empty() {
                            w.global::<General>().set_current_version(snapshot.version.clone().into());
                        }
                        show(&w, &snapshot);
                    });
                }
                ServerMessage::Config(config) => {
                    let (theme, look) = (config.theme.clone(), config.look.clone());
                    let snapshot = link
                        .lock()
                        .map(|mut link| {
                            link.config = Some(config);
                            link.snapshot.clone()
                        })
                        .ok();
                    let _ = window.upgrade_in_event_loop(move |w| {
                        apply_theme(&w, &theme);
                        apply_look(&w, &look);
                        // redraw the dials in the new colours
                        if let Some(snapshot) = snapshot {
                            show(&w, &snapshot);
                        }
                        // knobs and look are both in: now the window opens at the right size
                        open_window(&w);
                    });
                }
                ServerMessage::Entities(entities) => {
                    if let Ok(mut link) = link.lock() {
                        link.entities = entities.clone();
                    }
                    // refresh the pickers, keeping the current selections
                    let _ = window.upgrade_in_event_loop(move |w| {
                        let (light, press, hold) = selected_entities(&w);
                        fill_entity_lists(&w, &entities, light.as_deref(), press.as_deref(), hold.as_deref());
                    });
                }
                ServerMessage::UpdateStatus { status, latest_version, can_install } => {
                    let _ = window.upgrade_in_event_loop(move |w| {
                        let general = w.global::<General>();
                        general.set_checking_update(false);
                        general.set_updating(false);
                        general.set_update_status(status.into());
                        general.set_update_available(can_install);
                        if let Some(v) = latest_version {
                            general.set_latest_version(v.into());
                        }
                    });
                }
                // the Devices page (below) asks for these
                ServerMessage::Devices(choices) => {
                    let _ = window.upgrade_in_event_loop(move |w| show_device_choices(&w, &choices));
                }
                ServerMessage::MidiLearned(learned) => {
                    let _ = window.upgrade_in_event_loop(move |w| show_learned(&w, learned));
                }
                ServerMessage::Quit => {
                    let _ = slint::quit_event_loop();
                    return;
                }
            }
        }
        if let Ok(mut link) = link.lock() {
            link.stream = None;
        }
        set_status(&window, "Service stopped");
        std::thread::sleep(Duration::from_millis(500));
    }
}

fn setup_settings(window: &MainWindow, link: &Arc<Mutex<Link>>) {
    let lists = window.global::<Lists>();
    lists.set_actions(strings(editor::ACTIONS));
    lists.set_amounts(strings(editor::AMOUNTS.iter().map(|a| format!("{a}%"))));
    lists.set_swatches(ModelRc::new(VecModel::from(editor::SWATCHES.iter().map(|s| color(s)).collect::<Vec<Color>>())));
    lists.set_swatch_hex(strings(editor::SWATCHES));
    lists.on_parse_color(|text| color(&text));

    let knob = window.global::<KnobSettings>();
    {
        let weak = window.as_weak();
        knob.on_add_app(move |name| {
            let Some(w) = weak.upgrade() else { return };
            let name = name.trim().to_string();
            let apps = w.global::<KnobSettings>().get_apps();
            if name.is_empty() || apps.iter().any(|a| a.eq_ignore_ascii_case(&name)) {
                return;
            }
            if let Some(model) = apps.as_any().downcast_ref::<VecModel<SharedString>>() {
                model.push(name.as_str().into());
            }
            // highlight it where it is, or add it at the top (of the full list and of what's shown)
            mark_app(&name, true);
            let rows = w.global::<KnobSettings>().get_app_rows();
            match (0..rows.row_count()).find(|&i| rows.row_data(i).is_some_and(|r| r.name.eq_ignore_ascii_case(&name)))
            {
                Some(i) => {
                    if let Some(row) = rows.row_data(i) {
                        rows.set_row_data(i, AppRow { selected: true, ..row });
                    }
                }
                None => {
                    if let Some(model) = rows.as_any().downcast_ref::<VecModel<AppRow>>() {
                        model.insert(
                            0,
                            AppRow { name: name.as_str().into(), selected: true, playing: false, elsewhere: "".into() },
                        );
                    }
                }
            }
        });
    }
    {
        let (link, weak) = (Arc::clone(link), window.as_weak());
        knob.on_remove_knob(move || {
            let (Ok(mut link), Some(w)) = (link.lock(), weak.upgrade()) else { return };
            let Some(mut config) = link.config.clone() else { return };
            let index = w.global::<KnobSettings>().get_index() as usize;
            if editor::remove_knob(&mut config, active_profile(&link), index) {
                send_config(&mut link, config);
                w.set_page(0);
            }
        });
    }
    {
        let weak = window.as_weak();
        knob.on_set_apps_expanded(move |expanded| {
            let Some(w) = weak.upgrade() else { return };
            w.global::<KnobSettings>().set_apps_expanded(expanded);
            show_app_rows(&w);
        });
    }
    {
        let weak = window.as_weak();
        knob.on_toggle_app(move |index| {
            let Some(w) = weak.upgrade() else { return };
            let knob = w.global::<KnobSettings>();
            let Some(row) = knob.get_app_rows().row_data(index as usize) else { return };
            let apps = knob.get_apps();
            let Some(model) = apps.as_any().downcast_ref::<VecModel<SharedString>>() else { return };
            let selected = match (0..model.row_count())
                .find(|&i| model.row_data(i).is_some_and(|a| a.eq_ignore_ascii_case(&row.name)))
            {
                Some(i) => {
                    model.remove(i);
                    false
                }
                None => {
                    model.push(row.name.clone());
                    true
                }
            };
            // only the highlight changes; rows stay where they are (even when collapsed, so it can be ticked back)
            mark_app(&row.name, selected);
            knob.get_app_rows().set_row_data(index as usize, AppRow { selected, ..row });
        });
    }

    let general = window.global::<General>();
    general.set_current_version(format!("v{}", env!("CARGO_PKG_VERSION")).into());
    let flip = |pick: fn(&General) -> ModelRc<bool>, weak: slint::Weak<MainWindow>| {
        move |index: i32| {
            let Some(w) = weak.upgrade() else { return };
            let model = pick(&w.global::<General>());
            if let Some(value) = model.row_data(index as usize) {
                model.set_row_data(index as usize, !value);
            }
        }
    };
    general.on_toggle_lower(flip(|g| g.get_lower(), window.as_weak()));
    general.on_toggle_trigger(flip(|g| g.get_triggers(), window.as_weak()));
    general.on_edit_file(open_settings);
    general.set_theme_names(strings(themes::NAMES.iter().map(|(_, label)| *label)));
    {
        // picking a theme or a look option previews it right away; Back puts the saved ones back
        let weak = window.as_weak();
        let weak_shown = window.as_weak();
        let weak_look = window.as_weak();
        let link_look = Arc::clone(link);
        general.on_look_changed(move || {
            let Some(w) = weak_look.upgrade() else { return };
            apply_look(&w, &look_from(&w.global::<General>()));
            if let Ok(link) = link_look.lock() {
                show(&w, &link.snapshot);
            }
        });
        general.on_toggle_shown(move |index| {
            let Some(w) = weak_shown.upgrade() else { return };
            let model = w.global::<General>().get_look_shown();
            if let Some(value) = model.row_data(index as usize) {
                model.set_row_data(index as usize, !value);
            }
            w.global::<General>().invoke_look_changed();
        });
        general.on_theme_changed(move |index| {
            let Some(w) = weak.upgrade() else { return };
            if let Some((name, _)) = themes::NAMES.get(index.max(0) as usize) {
                apply_theme(&w, name);
            }
        });
    }
    {
        let (link, weak) = (Arc::clone(link), window.as_weak());
        general.on_open_devices(move || {
            let (Ok(mut link), Some(w)) = (link.lock(), weak.upgrade()) else { return };
            show_devices(&w, &link);
            link.send("list-devices");
            w.set_page(5);
        });
    }
    devices_page(window, link);
    {
        let (link, weak) = (Arc::clone(link), window.as_weak());
        general.on_open_calibration(move || {
            let (Ok(mut link), Some(w)) = (link.lock(), weak.upgrade()) else { return };
            let gamma = link.config.as_ref().map_or([1.0; 3], |c| c.leds.gamma);
            let general = w.global::<General>();
            general.set_gamma_red(gamma[0] as f32);
            general.set_gamma_green(gamma[1] as f32);
            general.set_gamma_blue(gamma[2] as f32);
            link.send(&format!("calibrate {} {} {}", gamma[0], gamma[1], gamma[2]));
            w.set_page(4);
        });
    }
    {
        let (link, weak) = (Arc::clone(link), window.as_weak());
        general.on_gamma_changed(move || {
            let (Ok(mut link), Some(w)) = (link.lock(), weak.upgrade()) else { return };
            let general = w.global::<General>();
            link.send(&format!(
                "calibrate {} {} {}",
                general.get_gamma_red(),
                general.get_gamma_green(),
                general.get_gamma_blue()
            ));
        });
    }
    {
        let (link, weak) = (Arc::clone(link), window.as_weak());
        window.on_back(move || {
            let (Ok(mut link), Some(w)) = (link.lock(), weak.upgrade()) else { return };
            if w.get_page() == 3 {
                // leaving Settings without saving: back to the saved theme
                let theme = link.config.as_ref().map_or("cranked".to_string(), |c| c.theme.clone());
                apply_theme(&w, &theme);
                let look = link.config.as_ref().map(|c| c.look.clone()).unwrap_or_default();
                apply_look(&w, &look);
                show(&w, &link.snapshot);
            }
            if w.get_page() == 4 {
                // leave the pattern; nothing saved
                link.send("calibrate off");
                if load_general(&w, &link) {
                    w.set_page(3);
                    return;
                }
            }
            if w.get_page() == 5 {
                // devices save as they change; back to Settings with its unsaved edits as they were
                w.set_page(3);
                return;
            }
            w.set_page(0);
        });
    }
    {
        let (link, weak) = (Arc::clone(link), window.as_weak());
        general.on_switch_profile(move |index| {
            let Ok(mut link) = link.lock() else { return };
            let Some(name) = link.config.as_ref().and_then(|c| c.profiles.get(index as usize)).map(|p| p.name.clone())
            else {
                return;
            };
            link.send(&format!("profile {name}"));
            link.snapshot.profile = name;
            if let Some(w) = weak.upgrade() {
                load_general(&w, &link);
            }
        });
    }
    {
        let (link, weak) = (Arc::clone(link), window.as_weak());
        general.on_new_profile(move || {
            let Ok(mut link) = link.lock() else { return };
            let current = active_profile(&link);
            let Some(mut config) = link.config.clone() else { return };
            let mut profile = config.profiles[current].clone();
            let mut n = config.profiles.len() + 1;
            while config.profiles.iter().any(|p| p.name == format!("Profile {n}")) {
                n += 1;
            }
            profile.name = format!("Profile {n}");
            let name = profile.name.clone();
            config.profiles.push(profile);
            send_config(&mut link, config);
            link.send(&format!("profile {name}"));
            link.snapshot.profile = name;
            if let Some(w) = weak.upgrade() {
                load_general(&w, &link);
            }
        });
    }
    {
        let (link, weak) = (Arc::clone(link), window.as_weak());
        general.on_rename_profile(move |name| {
            let name = name.trim().to_string();
            let Ok(mut link) = link.lock() else { return };
            let current = active_profile(&link);
            let Some(mut config) = link.config.clone() else { return };
            if name.is_empty() || config.profiles.iter().any(|p| p.name == name) {
                return;
            }
            config.profiles[current].name = name.clone();
            send_config(&mut link, config);
            link.send(&format!("profile {name}"));
            link.snapshot.profile = name;
            if let Some(w) = weak.upgrade() {
                load_general(&w, &link);
            }
        });
    }
    {
        let (link, weak) = (Arc::clone(link), window.as_weak());
        general.on_delete_profile(move || {
            let Ok(mut link) = link.lock() else { return };
            let current = active_profile(&link);
            let Some(mut config) = link.config.clone() else { return };
            if config.profiles.len() < 2 {
                return;
            }
            config.profiles.remove(current);
            let name = config.profiles[0].name.clone();
            send_config(&mut link, config);
            link.send(&format!("profile {name}"));
            link.snapshot.profile = name;
            if let Some(w) = weak.upgrade() {
                load_general(&w, &link);
            }
        });
    }
    {
        let (link, weak) = (Arc::clone(link), window.as_weak());
        general.on_check_update(move || {
            let Ok(mut link) = link.lock() else { return };
            link.send("check-update");
            if let Some(w) = weak.upgrade() {
                let g = w.global::<General>();
                g.set_checking_update(true);
                g.set_update_status("Checking for updates...".into());
            }
        });
    }
    {
        let (link, weak) = (Arc::clone(link), window.as_weak());
        general.on_apply_update(move || {
            let Ok(mut link) = link.lock() else { return };
            link.send("apply-update");
            if let Some(w) = weak.upgrade() {
                let g = w.global::<General>();
                g.set_updating(true);
                g.set_update_status("Downloading and installing...".into());
            }
        });
    }

    {
        let (link, weak) = (Arc::clone(link), window.as_weak());
        window.on_open_knob(move |index| {
            let (Ok(mut link), Some(w)) = (link.lock(), weak.upgrade()) else { return };
            if load_knob(&w, &link, index as usize) {
                link.send("entities");
                w.set_page(1);
            }
        });
    }
    {
        let (link, weak) = (Arc::clone(link), window.as_weak());
        window.on_open_button(move |index| {
            let (Ok(mut link), Some(w)) = (link.lock(), weak.upgrade()) else { return };
            if load_knob(&w, &link, index as usize) {
                link.send("entities");
                w.set_page(2);
            }
        });
    }
    // adds a knob (from the + card or Settings) and opens its settings so it can be set up right away
    let add_knob = {
        let (link, weak) = (Arc::clone(link), window.as_weak());
        move || {
            let (Ok(mut link), Some(w)) = (link.lock(), weak.upgrade()) else { return };
            let Some(mut config) = link.config.clone() else { return };
            let profile = active_profile(&link);
            let Some(index) = editor::add_knob(&mut config.profiles[profile]) else { return };
            send_config(&mut link, config);
            if load_knob(&w, &link, index) {
                link.send("entities");
                w.set_page(1);
            }
        }
    };
    window.on_add_knob(add_knob.clone());
    window.global::<General>().on_add_knob(add_knob);
    {
        let (link, weak) = (Arc::clone(link), window.as_weak());
        window.on_open_general(move || {
            let (Ok(link), Some(w)) = (link.lock(), weak.upgrade()) else { return };
            if load_general(&w, &link) {
                w.set_page(3);
            }
        });
    }
    {
        let (link, weak) = (Arc::clone(link), window.as_weak());
        window.on_save(move || {
            let (Ok(mut link), Some(w)) = (link.lock(), weak.upgrade()) else { return };
            let Some(mut config) = link.config.clone() else { return };
            let profile = active_profile(&link);
            match w.get_page() {
                1 | 2 => {
                    let index = w.global::<KnobSettings>().get_index() as usize;
                    if let Some(channel) = config.profiles[profile].channels.get_mut(index) {
                        store_knob(&w, channel);
                    }
                }
                4 => {
                    let general = w.global::<General>();
                    let (low, high) = cranked_ipc::settings::GAMMA_RANGE;
                    config.leds.gamma = [general.get_gamma_red(), general.get_gamma_green(), general.get_gamma_blue()]
                        .map(|g| (f64::from(g) * 100.0).round().clamp(low * 100.0, high * 100.0) / 100.0);
                    link.send("calibrate off");
                }
                3 => {
                    store_general(&w, &mut config, profile);
                    let token = w.global::<General>().get_ha_token().trim().to_string();
                    if !token.is_empty() {
                        link.send(&format!("token {token}"));
                    }
                }
                _ => return,
            }
            send_config(&mut link, config);
            w.set_page(0);
        });
    }
}

/// Matches the app-menu entry and icon names, so the desktop shows the right icon for the window.
const APP_ID: &str = "io.github.zach9113.CrankedAudio";

fn main() -> Result<(), slint::PlatformError> {
    let window = MainWindow::new()?;
    // needs the platform the window just started, and must come before the window is shown
    slint::set_xdg_app_id(APP_ID)?;
    window.set_knobs(ModelRc::new(VecModel::<KnobData>::default()));
    window.set_status("Connecting…".into());
    let link = Arc::new(Mutex::new(Link::default()));

    {
        let link = Arc::clone(&link);
        window.on_knob_set(move |index, value| {
            if let Ok(mut link) = link.lock() {
                link.set_knob(index as usize, value.clamp(0, 100) as u8);
            }
        });
    }
    {
        let link = Arc::clone(&link);
        window.on_knob_step(move |index, delta| {
            if let Ok(mut link) = link.lock() {
                let next = (i32::from(link.current(index as usize)) + delta).clamp(0, 100) as u8;
                link.set_knob(index as usize, next);
            }
        });
    }
    {
        let link = Arc::clone(&link);
        window.on_button(move |index, down| {
            if let Ok(mut link) = link.lock() {
                link.send(&format!("{} {index}", if down { "down" } else { "up" }));
            }
        });
    }
    setup_settings(&window, &link);

    {
        let link = Arc::clone(&link);
        let weak = window.as_weak();
        std::thread::Builder::new().name("link".into()).spawn(move || run_link(link, weak)).unwrap();
    }
    // for screenshots and tests: CRANKED_OPEN="knob 2" | "button 2" | "settings" | "leds" | "devices" opens that page once loaded
    if let Ok(open) = std::env::var("CRANKED_OPEN") {
        let weak = window.as_weak();
        slint::Timer::single_shot(Duration::from_millis(1500), move || {
            let Some(w) = weak.upgrade() else { return };
            let mut words = open.split_whitespace();
            let (page, index) = (words.next().unwrap_or_default(), words.next().and_then(|n| n.parse::<i32>().ok()));
            match (page, index) {
                ("knob", Some(i)) => {
                    w.invoke_open_knob(i);
                    if words.next() == Some("all") {
                        w.global::<KnobSettings>().invoke_set_apps_expanded(true);
                    }
                }
                ("button", Some(i)) => w.invoke_open_button(i),
                ("settings", _) => w.invoke_open_general(),
                ("leds", _) => w.global::<General>().invoke_open_calibration(),
                ("devices", _) => {
                    w.global::<General>().invoke_open_devices();
                    if open.ends_with("learn") {
                        w.global::<Devices>().invoke_learn();
                    }
                }
                _ => {}
            }
        });
    }
    // opens when the knobs arrive (see open_window); if the service doesn't answer soon, open anyway
    let weak = window.as_weak();
    slint::Timer::single_shot(Duration::from_millis(1200), move || {
        if let Some(w) = weak.upgrade() {
            open_window(&w);
        }
    });
    window.window().on_close_requested(|| {
        let _ = slint::quit_event_loop();
        slint::CloseRequestResponse::HideWindow
    });
    slint::run_event_loop_until_quit()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dial_arcs() {
        assert_eq!(arc(ARC_START, ARC_START), "M 50 50", "empty arc draws nothing");
        let full = arc(ARC_START, ARC_START + ARC_SWEEP);
        assert!(full.starts_with("M 18.89 81.11 A 44 44 0 1 1 81.11 81.11"), "{full}");
        let quarter = arc(ARC_START, ARC_START + 90.0);
        assert!(quarter.contains(" 0 0 1 "), "short arcs use the small-arc flag: {quarter}");
    }

    #[test]
    fn colors_and_icons() {
        assert_eq!(color("#FF8000"), Color::from_rgb_u8(255, 128, 0));
        assert_eq!(color("nope"), Color::from_rgb_u8(128, 128, 128));
        assert_eq!(icon("Toggle Coffee Maker").1, icon("Turn off Desk Lamp").1);
        assert!(!icon("Mute").0.is_empty());
    }

    #[test]
    fn dial_colours_are_solid() {
        let panel = themes::named("cranked").card;
        assert_eq!(mix(Color::from_rgb_u8(200, 0, 100), panel, 1.0), Color::from_rgb_u8(200, 0, 100));
        let knob = Knob { color: "#00FF00".into(), background: "#B80045".into(), ..Knob::default() };
        let data = row(0, &knob);
        assert_eq!(data.rest_color.alpha(), 255);
        assert_eq!(data.rest_color, mix(Color::from_rgb_u8(0xB8, 0, 0x45), panel, 0.55));
        let dark = row(0, &Knob { background: "#000000".into(), ..knob });
        assert_eq!(dark.rest_color, themes::named("cranked").track);
    }
}
