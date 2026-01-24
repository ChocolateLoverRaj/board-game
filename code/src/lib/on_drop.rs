pub struct OnDrop<F: FnOnce()> {
    on_drop: Option<F>,
}

impl<F: FnOnce()> OnDrop<F> {
    pub fn new(on_drop: F) -> Self {
        Self {
            on_drop: Some(on_drop),
        }
    }
}

impl<F: FnOnce()> Drop for OnDrop<F> {
    fn drop(&mut self) {
        self.on_drop.take().unwrap()()
    }
}
