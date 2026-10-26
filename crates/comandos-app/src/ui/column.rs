//! A one-child container that keeps its child at most `max` pixels wide, centered,
//! with `side` pixels free on each side when there is room. GTK3 has no CSS
//! max-width, and margins or width requests change what the page asks for (a width
//! request is only a minimum, and margins computed from the allocation feed back).
use gtk::{glib, prelude::*, subclass::prelude::*};
use std::cell::Cell;

/// Width the child gets on `width` pixels: the column, never less than the child's own
/// minimum, never wider than what is there.
pub fn child_width(width: i32, side: i32, max: i32, child_min: i32) -> i32 {
    (width - 2 * side).min(max).max(child_min).min(width).max(0)
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Column {
        pub max: Cell<i32>,
        pub side: Cell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Column {
        const NAME: &'static str = "ComandosColumn";
        type Type = super::Column;
        type ParentType = gtk::Bin;
    }

    impl ObjectImpl for Column {}

    impl Column {
        fn child(&self) -> Option<gtk::Widget> {
            self.obj().child().filter(|c| c.is_visible())
        }
        fn width_for(&self, width: i32) -> i32 {
            let min = self.child().map_or(0, |c| c.preferred_width().0);
            child_width(width, self.side.get(), self.max.get(), min)
        }
    }

    impl WidgetImpl for Column {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }
        fn preferred_width(&self) -> (i32, i32) {
            let Some(child) = self.child() else { return (0, 0) };
            let (min, natural) = child.preferred_width();
            (min, natural.min(self.max.get()).max(min) + 2 * self.side.get())
        }
        fn preferred_height(&self) -> (i32, i32) {
            let natural = self.preferred_width().1;
            self.preferred_height_for_width(natural)
        }
        fn preferred_height_for_width(&self, width: i32) -> (i32, i32) {
            self.child().map_or((0, 0), |c| c.preferred_height_for_width(self.width_for(width)))
        }
        fn preferred_width_for_height(&self, _height: i32) -> (i32, i32) {
            self.preferred_width()
        }
        fn size_allocate(&self, allocation: &gtk::Allocation) {
            self.obj().set_allocation(allocation);
            if let Some(child) = self.child() {
                let width = self.width_for(allocation.width());
                let x = allocation.x() + (allocation.width() - width) / 2;
                child.size_allocate(&gtk::Allocation::new(x, allocation.y(), width, allocation.height()));
            }
        }
    }

    impl ContainerImpl for Column {}
    impl BinImpl for Column {}
}

glib::wrapper! {
    pub struct Column(ObjectSubclass<imp::Column>)
        @extends gtk::Bin, gtk::Container, gtk::Widget;
}

impl Column {
    /// A column at most `max` pixels wide with `side` pixels kept free on each side.
    pub fn new(child: &impl IsA<gtk::Widget>, max: i32, side: i32) -> Self {
        let column: Self = glib::Object::new();
        column.imp().max.set(max);
        column.imp().side.set(side);
        column.add(child);
        column
    }
}

#[cfg(test)]
mod tests {
    use super::child_width;

    #[test]
    fn the_column_is_capped_centered_room_and_never_below_the_childs_minimum() {
        assert_eq!(child_width(1400, 32, 820, 100), 820);
        assert_eq!(child_width(700, 32, 820, 100), 636);
        assert_eq!(child_width(300, 32, 820, 280), 280, "keeps the child's minimum");
        assert_eq!(child_width(200, 32, 820, 280), 200, "but never wider than what is there");
    }
}
