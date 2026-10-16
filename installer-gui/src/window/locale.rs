use super::*;

/// Keys that switch between layouts: label, XKB option.
const SWITCH_KEYS: &[(&str, &str)] = &[
    ("Caps Lock", "grp:caps_toggle"),
    ("Alt + Shift", "grp:alt_shift_toggle"),
    ("Ctrl + Shift", "grp:ctrl_shift_toggle"),
    ("Super + Space", "grp:win_space_toggle"),
    ("Alt + Space", "grp:alt_space_toggle"),
];

/// At most this many layouts (XKB itself allows four).
const MAX_LAYOUTS: usize = 4;

/// Everything the keyboard page redraws from.
struct KeyboardUi {
    state: Rc<WizardState>,
    layouts: Vec<keyboard::Layout>,
    chosen: RefCell<Vec<String>>,
    list: gtk::ListBox,
    switch_group: adw::PreferencesGroup,
    switch_key: gtk::DropDown,
}

/// Whether this is the live medium (not a developer's own session): only there the session's layout is changed while
/// the wizard runs, so the test field below types what the person chose.
fn on_live_medium() -> bool {
    std::path::Path::new("/run/initramfs/live").is_dir()
        && std::env::var_os("GENTOO_INSTALLER_SIMULATE").is_none()
}

/// Makes the running niri use `layouts`: its config is reloaded when the file changes.
fn apply_to_live_session(layouts: &str, options: &str) {
    if !on_live_medium() {
        return;
    }
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let path = std::path::Path::new(&home).join(".config/niri/config.kdl");
    let Ok(kdl) = std::fs::read_to_string(&path) else {
        return;
    };
    if let Some(patched) = keyboard::patch_niri_xkb(&kdl, layouts, options) {
        if patched != kdl {
            let _ = std::fs::write(&path, patched);
        }
    }
}

impl KeyboardUi {
    fn description(&self, code: &str) -> String {
        self.layouts
            .iter()
            .find(|l| l.code == code)
            .map(|l| l.description.clone())
            .unwrap_or_default()
    }
}

/// Redraws "Your layouts", stores the choice in the wizard state and applies it to the live session.
fn keyboard_refresh(ui: &Rc<KeyboardUi>) {
    while let Some(child) = ui.list.first_child() {
        ui.list.remove(&child);
    }
    let chosen = ui.chosen.borrow().clone();
    for (i, code) in chosen.iter().enumerate() {
        let row = adw::ActionRow::builder()
            .use_markup(false)
            .title(ui.description(code))
            .subtitle(if i == 0 {
                format!("{code} · main")
            } else {
                code.clone()
            })
            .build();
        if i > 0 {
            let up = gtk::Button::builder()
                .icon_name("go-up-symbolic")
                .valign(gtk::Align::Center)
                .css_classes(vec!["flat".to_string()])
                .tooltip_text("Move up")
                .build();
            let ui2 = ui.clone();
            up.connect_clicked(move |_| {
                ui2.chosen.borrow_mut().swap(i, i - 1);
                let ui3 = ui2.clone();
                glib::idle_add_local_once(move || keyboard_refresh(&ui3));
            });
            row.add_suffix(&up);
        }
        if chosen.len() > 1 {
            let remove = gtk::Button::builder()
                .icon_name("user-trash-symbolic")
                .valign(gtk::Align::Center)
                .css_classes(vec!["flat".to_string()])
                .tooltip_text("Remove")
                .build();
            let ui2 = ui.clone();
            remove.connect_clicked(move |_| {
                ui2.chosen.borrow_mut().remove(i);
                let ui3 = ui2.clone();
                glib::idle_add_local_once(move || keyboard_refresh(&ui3));
            });
            row.add_suffix(&remove);
        }
        ui.list.append(&row);
    }
    ui.switch_group.set_visible(chosen.len() > 1);
    // With Caps Lock as the switch key the real Caps Lock moves to Shift + Caps Lock.
    let caps = SWITCH_KEYS
        .get(ui.switch_key.selected() as usize)
        .is_some_and(|(_, o)| *o == "grp:caps_toggle");
    ui.switch_group.set_description(if caps {
        Some("Press Shift + Caps Lock to use it as a regular Caps Lock.")
    } else {
        None
    });
    let layouts = chosen.join(",");
    let options = if chosen.len() > 1 {
        SWITCH_KEYS
            .get(ui.switch_key.selected() as usize)
            .map(|(_, o)| o.to_string())
            .unwrap_or_default()
    } else {
        String::new()
    };
    *ui.state.keyboard_layout.borrow_mut() = layouts.clone();
    *ui.state.keyboard_options.borrow_mut() = options.clone();
    apply_to_live_session(&layouts, &options);
}

/// Advanced-setup only: keyboard layouts (one or several, with the key that switches them), auto-detected default via
/// `keyboard::detect_current`.
pub(super) fn keyboard_select_page(
    nav: adw::NavigationView,
    state: Rc<WizardState>,
    timezone_page: adw::NavigationPage,
) -> adw::NavigationPage {
    let heading = gtk::Label::builder()
        .label("Keyboard layouts")
        .css_classes(vec!["hero-title".to_string()])
        .halign(gtk::Align::Start)
        .build();
    let body = gtk::Label::builder()
        .label("Add every layout you type in. The first is the main one. Type in the box at the bottom to check.")
        .css_classes(vec!["dim-label".to_string()])
        .halign(gtk::Align::Start)
        .wrap(true)
        .build();

    let layouts = keyboard::list_layouts();
    let mut chosen: Vec<String> = state
        .keyboard_layout
        .borrow()
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if chosen.is_empty() {
        chosen.push("us".into());
    }

    // What is chosen.
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(vec!["boxed-list".to_string()])
        .build();
    let switch_labels: Vec<&str> = SWITCH_KEYS.iter().map(|(l, _)| *l).collect();
    let switch_key = gtk::DropDown::from_strings(&switch_labels);
    switch_key.set_valign(gtk::Align::Center);
    let switch_row = adw::ActionRow::builder()
        .title("Switch layouts with")
        .build();
    switch_row.add_suffix(&switch_key);
    let switch_group = adw::PreferencesGroup::new();
    switch_group.add(&switch_row);

    let ui = Rc::new(KeyboardUi {
        state: state.clone(),
        layouts: layouts.clone(),
        chosen: RefCell::new(chosen),
        list: list.clone(),
        switch_group: switch_group.clone(),
        switch_key: switch_key.clone(),
    });
    {
        let ui = ui.clone();
        switch_key.connect_selected_notify(move |_| keyboard_refresh(&ui));
    }

    // What can be added: a list that a search box narrows.
    let search = gtk::SearchEntry::builder()
        .placeholder_text("Search layouts: name or code (ua, German, dvorak...)")
        .build();
    let available = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .activate_on_single_click(true)
        .css_classes(vec!["boxed-list".to_string()])
        .build();
    for l in &layouts {
        let row = adw::ActionRow::builder()
            .use_markup(false)
            .title(&l.description)
            .subtitle(&l.code)
            .activatable(true)
            .build();
        row.add_suffix(&gtk::Image::from_icon_name("list-add-symbolic"));
        available.append(&row);
    }
    let query = Rc::new(RefCell::new(String::new()));
    {
        let query = query.clone();
        available.set_filter_func(move |row| {
            let q = query.borrow().to_lowercase();
            if q.is_empty() {
                return true;
            }
            let Some(row) = row.downcast_ref::<adw::ActionRow>() else {
                return true;
            };
            let text =
                format!("{} {}", row.title(), row.subtitle().unwrap_or_default()).to_lowercase();
            q.split_whitespace().all(|w| text.contains(w))
        });
    }
    {
        let query = query.clone();
        let available = available.clone();
        search.connect_search_changed(move |s| {
            *query.borrow_mut() = s.text().to_string();
            available.invalidate_filter();
        });
    }
    {
        let ui = ui.clone();
        let layouts = layouts.clone();
        available.connect_row_activated(move |_, row| {
            let Some(l) = layouts.get(row.index() as usize) else {
                return;
            };
            {
                let mut chosen = ui.chosen.borrow_mut();
                if chosen.contains(&l.code) || chosen.len() >= MAX_LAYOUTS {
                    return;
                }
                chosen.push(l.code.clone());
            }
            keyboard_refresh(&ui);
        });
    }
    let available_scroll = gtk::ScrolledWindow::builder()
        .child(&available)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .min_content_height(190)
        .max_content_height(190)
        .build();

    let test_field = gtk::Entry::builder()
        .placeholder_text("Type here to test")
        .build();

    let your_label = gtk::Label::builder()
        .label("Your layouts")
        .css_classes(vec!["section-heading".to_string()])
        .halign(gtk::Align::Start)
        .build();
    let add_label = gtk::Label::builder()
        .label("Add a layout")
        .css_classes(vec!["section-heading".to_string()])
        .halign(gtk::Align::Start)
        .margin_top(8)
        .build();

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_start(24)
        .margin_end(24)
        .margin_top(48)
        .margin_bottom(24)
        .build();
    content.append(&heading);
    content.append(&body);
    content.append(&your_label);
    content.append(&list);
    content.append(&switch_group);
    content.append(&add_label);
    content.append(&search);
    content.append(&available_scroll);
    content.append(&test_field);

    let next_button = gtk::Button::builder()
        .label("Continue")
        .css_classes(vec!["suggested-action".to_string(), "pill".to_string()])
        .halign(gtk::Align::End)
        .margin_top(24)
        .build();
    content.append(&next_button);
    {
        let nav = nav.clone();
        next_button.connect_clicked(move |_| nav.push(&timezone_page));
    }
    keyboard_refresh(&ui);

    let scroll = gtk::ScrolledWindow::builder()
        .child(&content)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    let clamp = adw::Clamp::builder()
        .child(&scroll)
        .maximum_size(560)
        .build();
    let page = adw::NavigationPage::builder()
        .title("Keyboard")
        .child(&clamp)
        .build();
    page.set_tag(Some("keyboard"));
    page
}

/// Advanced-setup only: time zone, auto-detected default via `timezone::detect_current`, found by searching places and
/// countries, or by asking the internet connection.
/// `next_page` is Disk directly if there's already an ethernet link, else Network.
pub(super) fn timezone_select_page(
    nav: adw::NavigationView,
    state: Rc<WizardState>,
    next_page: adw::NavigationPage,
) -> adw::NavigationPage {
    let heading = gtk::Label::builder()
        .label("Where are you?")
        .css_classes(vec!["hero-title".to_string()])
        .halign(gtk::Align::Start)
        .build();
    let body = gtk::Label::builder()
        .label("This sets your time zone and clock. Search for a city, a capital or a country.")
        .css_classes(vec!["dim-label".to_string()])
        .halign(gtk::Align::Start)
        .wrap(true)
        .build();

    let zones = Rc::new(tz::list_zone_details());
    let current = state.timezone.borrow().clone();

    let search = gtk::SearchEntry::builder()
        .placeholder_text("Search: Kyiv, Ukraine, Tokyo, Berlin...")
        .build();
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .css_classes(vec!["boxed-list".to_string()])
        .build();
    for z in zones.iter() {
        let subtitle = [z.id.as_str(), z.country.as_str(), z.note.as_str()]
            .iter()
            .filter(|s| !s.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join(" · ");
        let row = adw::ActionRow::builder()
            .use_markup(false)
            .title(z.city())
            .subtitle(subtitle)
            .activatable(true)
            .build();
        list.append(&row);
        if z.id == current {
            list.select_row(Some(&row));
        }
    }
    let query = Rc::new(RefCell::new(String::new()));
    {
        let query = query.clone();
        let zones = zones.clone();
        list.set_filter_func(move |row| {
            let q = query.borrow();
            zones
                .get(row.index() as usize)
                .is_none_or(|z| z.matches(&q))
        });
    }
    {
        let query = query.clone();
        let list = list.clone();
        search.connect_search_changed(move |s| {
            *query.borrow_mut() = s.text().to_string();
            list.invalidate_filter();
        });
    }
    // `GENTOO_INSTALLER_GUI_SEARCH=text` pre-fills the search (for looking at the page).
    if let Ok(text) = std::env::var("GENTOO_INSTALLER_GUI_SEARCH") {
        search.set_text(&text);
    }
    let chosen_label = gtk::Label::builder()
        .label(format!("Selected: {current}"))
        .css_classes(vec!["dim-label".to_string()])
        .halign(gtk::Align::Start)
        .build();
    // Selecting a row from code (the connection's answer) is not the person's choice.
    let by_code = Rc::new(Cell::new(false));
    {
        let state = state.clone();
        let zones = zones.clone();
        let chosen_label = chosen_label.clone();
        let by_code = by_code.clone();
        list.connect_row_selected(move |_, row| {
            if let Some(z) = row.and_then(|r| zones.get(r.index() as usize)) {
                *state.timezone.borrow_mut() = z.id.clone();
                if !by_code.get() {
                    state.timezone_touched.set(true);
                }
                chosen_label.set_label(&format!("Selected: {}", z.id));
            }
        });
    }
    {
        let list = list.clone();
        let zones = zones.clone();
        let chosen_label = chosen_label.clone();
        let by_code = by_code.clone();
        *state.tz_select.borrow_mut() = Some(Rc::new(move |zone: &str| {
            if let Some(i) = zones.iter().position(|z| z.id == zone) {
                if let Some(row) = list.row_at_index(i as i32) {
                    by_code.set(true);
                    list.select_row(Some(&row));
                    by_code.set(false);
                    chosen_label.set_label(&format!("Selected: {zone} (from your connection)"));
                }
            }
        }));
    }
    let list_scroll = gtk::ScrolledWindow::builder()
        .child(&list)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .min_content_height(240)
        .max_content_height(240)
        .build();
    // Scroll to the current zone once the page has a size.
    {
        let list = list.clone();
        let list_scroll = list_scroll.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(250), move || {
            if let Some(row) = list.selected_row() {
                if let Some((_, y)) = row.translate_coordinates(&list, 0.0, 0.0) {
                    list_scroll.vadjustment().set_value((y - 80.0).max(0.0));
                }
            }
        });
    }

    // One request to a location service, only when asked.
    let detect = gtk::Button::builder()
        .label("Find it from the internet connection")
        .halign(gtk::Align::Start)
        .css_classes(vec!["flat".to_string()])
        .sensitive(!installer_core::offline::is_active())
        .tooltip_text("Asks ipapi.co for the time zone of this connection's address")
        .build();
    {
        let list = list.clone();
        let zones = zones.clone();
        let detect2 = detect.clone();
        let query = query.clone();
        let search = search.clone();
        detect.connect_clicked(move |_| {
            detect2.set_sensitive(false);
            detect2.set_label("Looking...");
            let (tx, rx) = std_mpsc::channel::<Option<String>>();
            std::thread::spawn(move || {
                let _ = tx.send(tz::detect_by_ip());
            });
            let list = list.clone();
            let zones = zones.clone();
            let detect = detect2.clone();
            let query = query.clone();
            let search = search.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
                match rx.try_recv() {
                    Err(std_mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                    Ok(Some(zone)) => {
                        // Show the whole list again and select the answer.
                        search.set_text("");
                        query.borrow_mut().clear();
                        list.invalidate_filter();
                        if let Some(i) = zones.iter().position(|z| z.id == zone) {
                            if let Some(row) = list.row_at_index(i as i32) {
                                list.select_row(Some(&row));
                                row.grab_focus();
                            }
                        }
                        detect.set_label("Find it from the internet connection");
                        detect.set_sensitive(true);
                        glib::ControlFlow::Break
                    }
                    _ => {
                        detect.set_label("No answer: check the connection, or search by hand");
                        detect.set_sensitive(true);
                        glib::ControlFlow::Break
                    }
                }
            });
        });
    }

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_start(24)
        .margin_end(24)
        .margin_top(48)
        .margin_bottom(24)
        .build();
    content.append(&heading);
    content.append(&body);
    content.append(&search);
    content.append(&list_scroll);
    content.append(&chosen_label);
    content.append(&detect);

    let next_button = gtk::Button::builder()
        .label("Continue")
        .css_classes(vec!["suggested-action".to_string(), "pill".to_string()])
        .halign(gtk::Align::End)
        .margin_top(24)
        .build();
    content.append(&next_button);
    {
        let nav = nav.clone();
        next_button.connect_clicked(move |_| nav.push(&next_page));
    }

    let scroll = gtk::ScrolledWindow::builder()
        .child(&content)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    let clamp = adw::Clamp::builder()
        .child(&scroll)
        .maximum_size(560)
        .build();
    let page = adw::NavigationPage::builder()
        .title("Time zone")
        .child(&clamp)
        .build();
    page.set_tag(Some("timezone"));
    page
}
