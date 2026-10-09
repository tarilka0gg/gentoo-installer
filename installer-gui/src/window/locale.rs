use super::*;

/// Advanced-setup only: keyboard layout, auto-detected default via `keyboard::detect_current`.
pub(super) fn keyboard_select_page(
    nav: adw::NavigationView,
    state: Rc<WizardState>,
    timezone_page: adw::NavigationPage,
) -> adw::NavigationPage {
    let heading = gtk::Label::builder()
        .label("Keyboard layout")
        .css_classes(vec!["hero-title".to_string()])
        .halign(gtk::Align::Start)
        .build();
    let body = gtk::Label::builder()
        .label("Type below to check it's right.")
        .css_classes(vec!["dim-label".to_string()])
        .halign(gtk::Align::Start)
        .build();

    let layouts = keyboard::list_layouts();
    let current = state.keyboard_layout.borrow().clone();
    let display_strings: Vec<String> = layouts
        .iter()
        .map(|l| format!("{} — {}", l.code, l.description))
        .collect();
    let selected_index = layouts.iter().position(|l| l.code == current).unwrap_or(0) as u32;

    let model = gtk::StringList::new(
        &display_strings
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
    );
    let dropdown = gtk::DropDown::builder()
        .model(&model)
        .selected(selected_index)
        .build();
    dropdown.set_enable_search(true);
    // A StringList has no default search key: without this the search box matches nothing.
    dropdown.set_expression(Some(&gtk::PropertyExpression::new(
        gtk::StringObject::static_type(),
        None::<gtk::Expression>,
        "string",
    )));

    {
        let state = state.clone();
        let layouts = layouts.clone();
        dropdown.connect_selected_notify(move |dd| {
            if let Some(l) = layouts.get(dd.selected() as usize) {
                *state.keyboard_layout.borrow_mut() = l.code.clone();
            }
        });
    }

    let test_field = gtk::Entry::builder()
        .placeholder_text("Type here to test")
        .build();

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(16)
        .margin_start(24)
        .margin_end(24)
        .margin_top(48)
        .margin_bottom(24)
        .build();
    content.append(&heading);
    content.append(&body);
    content.append(&dropdown);
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

    let clamp = adw::Clamp::builder()
        .child(&content)
        .maximum_size(560)
        .build();
    let page = adw::NavigationPage::builder()
        .title("Keyboard")
        .child(&clamp)
        .build();
    page.set_tag(Some("keyboard"));
    page
}

/// Advanced-setup only: time zone, auto-detected default via `timezone::detect_current`.
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
        .label("This sets your time zone and clock.")
        .css_classes(vec!["dim-label".to_string()])
        .halign(gtk::Align::Start)
        .build();

    let zones = tz::list_zones();
    let current = state.timezone.borrow().clone();
    let selected_index = zones.iter().position(|z| *z == current).unwrap_or(0) as u32;

    let model = gtk::StringList::new(&zones.iter().map(String::as_str).collect::<Vec<_>>());
    let dropdown = gtk::DropDown::builder()
        .model(&model)
        .selected(selected_index)
        .build();
    dropdown.set_enable_search(true);
    // A StringList has no default search key: without this the search box matches nothing.
    dropdown.set_expression(Some(&gtk::PropertyExpression::new(
        gtk::StringObject::static_type(),
        None::<gtk::Expression>,
        "string",
    )));

    {
        let state = state.clone();
        let zones = zones.clone();
        dropdown.connect_selected_notify(move |dd| {
            if let Some(z) = zones.get(dd.selected() as usize) {
                *state.timezone.borrow_mut() = z.clone();
            }
        });
    }

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(16)
        .margin_start(24)
        .margin_end(24)
        .margin_top(48)
        .margin_bottom(24)
        .build();
    content.append(&heading);
    content.append(&body);
    content.append(&dropdown);

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

    let clamp = adw::Clamp::builder()
        .child(&content)
        .maximum_size(560)
        .build();
    let page = adw::NavigationPage::builder()
        .title("Time zone")
        .child(&clamp)
        .build();
    page.set_tag(Some("timezone"));
    page
}
