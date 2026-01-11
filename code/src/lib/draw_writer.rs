use core::fmt::Write;

use embedded_graphics::{
    prelude::*,
    text::{Baseline, Text, renderer::TextRenderer},
};

pub struct DrawWriter<'a, D, S> {
    display: &'a mut D,
    position: Point,
    character_style: S,
}
impl<'a, D, S> DrawWriter<'a, D, S> {
    pub fn new(display: &'a mut D, position: Point, character_style: S) -> Self {
        Self {
            display,
            position,
            character_style,
        }
    }
}
impl<D, S: TextRenderer + Clone> Write for DrawWriter<'_, D, S>
where
    D: DrawTarget<Color = S::Color>,
{
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.position = Text::with_baseline(
            s,
            self.position,
            self.character_style.clone(),
            Baseline::Top,
        )
        .draw(self.display)
        .map_err(|_| core::fmt::Error)?;
        Ok(())
    }
}
