use masonry::properties::types::{AsUnit};
use xilem::view::{flex_row, label, text_button};
use xilem::WidgetView;


pub enum TempoChangeEvent {
    Increment(f64),
    Decrement(f64),
}

pub fn tempo_spin_box<T: 'static>(value: f64) -> impl WidgetView<T, TempoChangeEvent> {
    flex_row((
        text_button("- 0.1".to_string(), |_| TempoChangeEvent::Decrement(0.1)),
        text_button("- 1".to_string(), |_| TempoChangeEvent::Decrement(1.0)),
        text_button("- 10".to_string(), |_| TempoChangeEvent::Decrement(10.0)),
        label(format!("{:.1}", value)),
        text_button("+ 10".to_string(), |_| TempoChangeEvent::Increment(10.0)),
        text_button("+ 1".to_string(), |_| TempoChangeEvent::Increment(1.0)),
        text_button("+ 0.1".to_string(), |_| TempoChangeEvent::Increment(0.1)),
    ))
        .gap(0.5.px())
}
