use debouncr::{debounce_stateful_3, DebouncerStateful, Repeat3};

pub use debouncr::Edge;

pub trait Measurable {
    fn is_up(&self) -> bool;
}

pub struct Debounced<P: Measurable> {
    pin: P,
    debouncer: DebouncerStateful<u8, Repeat3>,
}

impl<P: Measurable> Debounced<P> {
    pub fn new(pin: P) -> Self {
        let debouncer = debounce_stateful_3(pin.is_up());
        Self { pin, debouncer }
    }

    pub fn poll(&mut self) -> Option<Edge> {
        self.debouncer.update(self.pin.is_up())
    }

    // pub fn raw(&self) -> bool {
    //     self.pin.is_up()
    // }
}

