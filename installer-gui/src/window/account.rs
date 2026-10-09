use super::*;

/// Screen script §9: username + password, before Confirm. Root stays locked; this
/// account gets `wheel` (see `installer_core::account::create`).
pub(super) fn account_page_build(
    nav: adw::NavigationView,
    state: Rc<WizardState>,
) -> adw::NavigationPage {
    let heading = gtk::Label::builder()
        .label("Create your account")
        .css_classes(vec!["hero-title".to_string()])
        .halign(gtk::Align::Start)
        .build();
    let body = gtk::Label::builder()
        .label("This account can install software and change system settings.")
        .css_classes(vec!["dim-label".to_string()])
        .halign(gtk::Align::Start)
        .build();

    let username_entry = gtk::Entry::builder()
        .placeholder_text("Username")
        .text(state.username.borrow().as_str())
        .build();
    let password_entry = gtk::PasswordEntry::builder()
        .placeholder_text("Password")
        .show_peek_icon(true)
        .build();
    let confirm_entry = gtk::PasswordEntry::builder()
        .placeholder_text("Confirm password")
        .show_peek_icon(true)
        .build();

    let error_label = gtk::Label::builder()
        .halign(gtk::Align::Start)
        .wrap(true)
        .visible(false)
        .css_classes(vec!["error".to_string()])
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
    content.append(&username_entry);
    content.append(&password_entry);
    content.append(&confirm_entry);
    content.append(&error_label);

    let next_button = gtk::Button::builder()
        .label("Continue")
        .css_classes(vec!["suggested-action".to_string(), "pill".to_string()])
        .halign(gtk::Align::End)
        .margin_top(24)
        .build();
    content.append(&next_button);

    let confirm_page = Rc::new(RefCell::new(None::<adw::NavigationPage>));
    {
        let nav = nav.clone();
        let state = state.clone();
        let confirm_page = confirm_page.clone();
        next_button.connect_clicked(move |_| {
            let username = username_entry.text().to_string();
            let password = password_entry.text().to_string();
            let confirm = confirm_entry.text().to_string();

            if username.trim().is_empty() {
                error_label.set_label("Enter a username.");
                error_label.set_visible(true);
                return;
            }
            if password.len() < 8 {
                error_label.set_label("Password needs to be at least 8 characters.");
                error_label.set_visible(true);
                return;
            }
            if password != confirm {
                error_label.set_label("Passwords don't match.");
                error_label.set_visible(true);
                return;
            }
            error_label.set_visible(false);

            *state.username.borrow_mut() = username;
            *state.password.borrow_mut() = password;

            if confirm_page.borrow().is_none() {
                let page = confirm_page_build(nav.clone(), state.clone());
                nav.add(&page);
                *confirm_page.borrow_mut() = Some(page);
            }
            if let Some(page) = confirm_page.borrow().as_ref() {
                nav.push(page);
            }
        });
    }

    let clamp = adw::Clamp::builder()
        .child(&content)
        .maximum_size(560)
        .build();
    let page = adw::NavigationPage::builder()
        .title("Account")
        .child(&clamp)
        .build();
    page.set_tag(Some("account"));
    page
}
