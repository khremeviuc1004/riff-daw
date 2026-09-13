/// Replacement for the `gladis` crate's `Gladis` derive macro for GTK4.
///
/// Provides a macro that generates `from_string` and `from_builder` methods
/// to extract widget fields from a `gtk4::Builder`.

pub trait FromGtk4Builder: Sized {
    fn from_builder(builder: &gtk4::Builder) -> Option<Self>;
    fn from_string(s: &str) -> Option<Self> {
        let builder = gtk4::Builder::from_string(s);
        Self::from_builder(&builder)
    }
}

/// Implement `FromGtk4Builder` for a struct by extracting each field from a
/// `gtk4::Builder` by its field name.
///
/// # Example
/// ```ignore
/// gtk4_builder_from!(MyWidget {
///     some_button: gtk4::Button,
///     some_label: gtk4::Label,
/// });
/// ```
#[macro_export]
macro_rules! gtk4_builder_from {
    ($struct_name:ident { $( $field:ident : $type:ty ),* $(,)? }) => {
        impl $crate::gladis4::FromGtk4Builder for $struct_name {
            fn from_builder(builder: &gtk4::Builder) -> Option<Self> {
                Some(Self {
                    $( $field: builder.object::<$type>(stringify!($field))?, )*
                })
            }
            fn from_string(s: &str) -> Option<Self> {
                let builder = gtk4::Builder::from_string(s);
                Self::from_builder(&builder)
            }
        }
    };
}
