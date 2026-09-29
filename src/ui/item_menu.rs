//! The per-item menu (⋮ button and right-click) shared by list rows and
//! gallery tiles, and the large image tooltip.

use gtk::{gio, glib, prelude::*};

use crate::{
    i18n::gettext,
    model::{ClipItem, ClipKind},
};

/// Menu actions name the item by its content key, not its list position:
/// rows are not rebound when clips are added or deleted above them.
pub fn item_menu(item: &ClipItem) -> gio::Menu {
    let key = item.key().to_variant();
    let add = |section: &gio::Menu, label: &str, action: &str| {
        let mi = gio::MenuItem::new(Some(label), None);
        mi.set_action_and_target_value(Some(action), Some(&key));
        section.append_item(&mi);
    };
    let menu = gio::Menu::new();
    let section = gio::Menu::new();
    add(&section, &gettext("Paste"), "win.paste-item");
    add(&section, &gettext("Copy"), "win.copy-item");
    add(
        &section,
        &if item.pinned() {
            gettext("Unpin")
        } else {
            gettext("Pin")
        },
        "win.pin-item",
    );
    add(&section, &gettext("Details…"), "win.show-item");
    menu.append_section(None, &section);
    let danger = gio::Menu::new();
    add(&danger, &gettext("Delete"), "win.delete-item");
    menu.append_section(None, &danger);
    menu
}

/// Right-click on `widget` opens `menu_for()` at the pointer. The popover is
/// created once and unparented in `dispose` by the caller.
pub fn attach_context_menu(
    widget: &impl IsA<gtk::Widget>,
    popover: &glib::WeakRef<gtk::PopoverMenu>,
    menu_for: impl Fn() -> Option<gio::Menu> + 'static,
) {
    let gesture = gtk::GestureClick::builder().button(3).build();
    let popover = popover.clone();
    gesture.connect_pressed(move |gesture, _, x, y| {
        let (Some(popover), Some(menu)) = (popover.upgrade(), menu_for()) else {
            return;
        };
        gesture.set_state(gtk::EventSequenceState::Claimed);
        popover.set_menu_model(Some(&menu));
        popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover.popup();
    });
    widget.as_ref().add_controller(gesture);
}

/// Longest edge of the hover preview, in logical pixels.
const TOOLTIP_SIZE: f64 = 360.0;

/// Hovering an image shows it large, without opening Details.
pub fn attach_image_tooltip(
    widget: &impl IsA<gtk::Widget>,
    item_for: impl Fn() -> Option<ClipItem> + 'static,
) {
    let widget = widget.as_ref();
    widget.set_has_tooltip(true);
    widget.connect_query_tooltip(move |_, _, _, _, tooltip| {
        let Some(item) = item_for().filter(|i| i.kind() == ClipKind::Image) else {
            return false;
        };
        let Some(texture) = item.thumbnail() else {
            return false;
        };
        let (w, h) = (f64::from(texture.width()), f64::from(texture.height()));
        let scale = (TOOLTIP_SIZE / w.max(h).max(1.0)).min(1.0);
        // A picture's natural size is the texture's, so size a backdrop and
        // lay the picture over it (overlays do not affect the size).
        let backdrop = gtk::Box::new(gtk::Orientation::Vertical, 0);
        backdrop.set_size_request((w * scale) as i32, (h * scale) as i32);
        let picture = gtk::Picture::for_paintable(&texture);
        picture.set_can_shrink(true);
        picture.set_content_fit(gtk::ContentFit::Contain);
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&backdrop));
        overlay.add_overlay(&picture);
        tooltip.set_custom(Some(&overlay));
        true
    });
}
