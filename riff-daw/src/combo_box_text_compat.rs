//! GTK4 replacement for the deprecated `GtkComboBoxText`.
//!
//! `GtkComboBoxText` was deprecated in GTK 4.10 in favour of `GtkDropDown`
//! bound to a list model through an expression. This module keeps the legacy
//! string-based API (id/text item pairs) working by backing each `GtkDropDown`
//! with a `gio::ListStore` of lightweight `ComboItemObject`s and exposing the
//! old method names as an extension trait, so the existing call sites stay
//! unchanged.

use std::cell::RefCell;

use glib::prelude::*;
use glib::subclass::prelude::*;
use gtk4::prelude::*;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct ComboItem {
        pub id: RefCell<Option<String>>,
        pub text: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ComboItem {
        const NAME: &'static str = "RiffComboItem";
        type Type = super::ComboItemObject;
        type ParentType = glib::Object;
    }

    impl ObjectImpl for ComboItem {
        fn properties() -> &'static [glib::ParamSpec] {
            use std::sync::OnceLock;

            static PROPERTIES: OnceLock<Vec<glib::ParamSpec>> = OnceLock::new();
            PROPERTIES
                .get_or_init(|| {
                    vec![
                        glib::ParamSpecString::builder("id").build(),
                        glib::ParamSpecString::builder("text").build(),
                    ]
                })
                .as_slice()
        }

        fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            match pspec.name() {
                "id" => {
                    self.id.replace(value.get::<Option<String>>().ok().flatten());
                }
                "text" => {
                    self.text.replace(value.get::<String>().unwrap_or_default());
                }
                _ => unimplemented!(),
            }
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "id" => match self.id.borrow().as_deref() {
                    Some(id) => id.to_value(),
                    None => None::<&str>.into(),
                },
                "text" => self.text.borrow().as_str().to_value(),
                _ => unimplemented!(),
            }
        }
    }
}

glib::wrapper! {
    pub struct ComboItemObject(ObjectSubclass<imp::ComboItem>);
}

impl ComboItemObject {
    pub fn new(id: Option<&str>, text: &str) -> Self {
        let item: Self = glib::Object::new();
        item.imp().id.replace(id.map(str::to_string));
        item.imp().text.replace(text.to_string());
        item
    }

    pub fn id(&self) -> Option<String> {
        self.imp().id.borrow().clone()
    }

    pub fn text(&self) -> String {
        self.imp().text.borrow().clone()
    }

    pub fn set_text(&self, text: &str) {
        self.imp().text.replace(text.to_string());
        self.notify("text");
    }
}

fn list_store_for(drop_down: &gtk4::DropDown) -> gio::ListStore {
    if let Some(model) = drop_down.model() {
        if let Ok(store) = model.downcast::<gio::ListStore>() {
            return store;
        }
    }
    let store = gio::ListStore::new::<ComboItemObject>();
    drop_down.set_model(Some(&store));
    drop_down.set_expression(Some(gtk4::PropertyExpression::new(
        ComboItemObject::static_type(),
        None::<gtk4::ConstantExpression>,
        "text",
    )));
    store
}

fn store_for(drop_down: &gtk4::DropDown) -> Option<gio::ListStore> {
    drop_down
        .model()
        .and_then(|model| model.downcast::<gio::ListStore>().ok())
}

fn selected_item_text(item: glib::Object) -> Option<String> {
    item.downcast::<ComboItemObject>().ok().map(|item| item.text())
}

fn selected_item_id(item: glib::Object) -> Option<String> {
    item.downcast::<ComboItemObject>().ok().and_then(|item| item.id())
}

/// Extension trait exposing the legacy `GtkComboBoxText` string API on
/// `GtkDropDown`, so call sites written against `ComboBoxText` keep working.
pub trait ComboBoxTextCompat {
    fn append(&self, id: Option<&str>, text: &str);
    fn remove_all(&self);
    fn remove(&self, position: i32);
    fn active_id(&self) -> Option<String>;
    fn set_active_id(&self, id: Option<&str>) -> bool;
    fn active_text(&self) -> Option<String>;
    fn active(&self) -> Option<u32>;
    fn set_active(&self, index: Option<u32>);
    fn len(&self) -> u32;
    /// Replaces the display text of the item with the given id (used by the
    /// legacy "rename the selected item" call sites). Returns true if found.
    fn update_text(&self, id: &str, new_text: &str) -> bool;
    fn connect_changed<F>(&self, f: F) -> glib::SignalHandlerId
    where
        F: Fn(&gtk4::DropDown) + 'static;
}

impl ComboBoxTextCompat for gtk4::DropDown {
    fn append(&self, id: Option<&str>, text: &str) {
        list_store_for(self).append(&ComboItemObject::new(id, text));
    }

    fn remove_all(&self) {
        let store = gio::ListStore::new::<ComboItemObject>();
        self.set_model(Some(&store));
        self.set_expression(Some(gtk4::PropertyExpression::new(
            ComboItemObject::static_type(),
            None::<gtk4::ConstantExpression>,
            "text",
        )));
    }

    fn remove(&self, position: i32) {
        if position >= 0 {
            if let Some(store) = store_for(self) {
                store.remove(position as u32);
            }
        }
    }

    fn active_id(&self) -> Option<String> {
        self.selected_item().and_then(selected_item_id)
    }

    fn set_active_id(&self, id: Option<&str>) -> bool {
        let Some(id) = id else {
            return false;
        };
        let Some(store) = store_for(self) else {
            return false;
        };
        for index in 0..store.n_items() {
            let Some(item) = store.item(index) else {
                continue;
            };
            if let Ok(item) = item.downcast::<ComboItemObject>() {
                if item.id().as_deref() == Some(id) {
                    self.set_selected(index);
                    return true;
                }
            }
        }
        false
    }

    fn active_text(&self) -> Option<String> {
        self.selected_item().and_then(selected_item_text)
    }

    fn active(&self) -> Option<u32> {
        if self.selected_item().is_some() {
            Some(self.selected())
        } else {
            None
        }
    }

    fn set_active(&self, index: Option<u32>) {
        if let Some(index) = index {
            self.set_selected(index);
        }
    }

    fn update_text(&self, id: &str, new_text: &str) -> bool {
        match store_for(self) {
            Some(store) => {
                for item in store.iter::<ComboItemObject>().flatten() {
                    if item.id().as_deref() == Some(id) {
                        item.set_text(new_text);
                        return true;
                    }
                }
                false
            }
            None => false,
        }
    }

    fn len(&self) -> u32 {
        store_for(self).map_or(0, |store| store.n_items())
    }

    fn connect_changed<F>(&self, f: F) -> glib::SignalHandlerId
    where
        F: Fn(&gtk4::DropDown) + 'static,
    {
        self.connect_selected_notify(move |drop_down| {
            f(drop_down);
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combo_box_text_compat_behaviour() {
        gtk4::init().expect("failed to init gtk4");

        let dd = gtk4::DropDown::builder().build();

        dd.append(Some("a"), "Alpha");
        dd.append(Some("b"), "Beta");
        dd.append(Some("c"), "Gamma");

        assert_eq!(dd.len(), 3);
        assert_eq!(dd.active(), Some(0));
        assert_eq!(dd.active_id().as_deref(), Some("a"));
        assert_eq!(dd.active_text().as_deref(), Some("Alpha"));

        dd.set_active_id(Some("c"));
        assert_eq!(dd.active(), Some(2));
        assert_eq!(dd.active_text().as_deref(), Some("Gamma"));

        dd.set_active(Some(1));
        assert_eq!(dd.active_text().as_deref(), Some("Beta"));

        dd.remove(1);
        assert_eq!(dd.len(), 2);
        assert_eq!(dd.active(), Some(1));
        assert_eq!(dd.active_text().as_deref(), Some("Gamma"));

        let fired = std::rc::Rc::new(RefCell::new(Vec::new()));
        let fired_for_closure = fired.clone();
        dd.connect_changed(move |widget| {
            fired_for_closure
                .borrow_mut()
                .push(widget.active_text().unwrap_or_default());
        });
        dd.set_active(Some(0));
        assert_eq!(fired.borrow().len(), 1);
        assert_eq!(fired.borrow()[0], "Alpha");
        dd.set_active_id(Some("missing"));
        assert_eq!(fired.borrow().len(), 1);

        dd.remove_all();
        assert_eq!(dd.len(), 0);
        assert_eq!(dd.active(), None);
        assert_eq!(dd.active_text(), None);
        assert_eq!(dd.active_id(), None);

        dd.append(None, "No id");
        assert_eq!(dd.len(), 1);
        assert_eq!(dd.active_id(), None);
        assert_eq!(dd.active_text().as_deref(), Some("No id"));
    }
}