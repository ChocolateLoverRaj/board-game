use core::fmt::{Display, Write};

use defmt::Format;
use embedded_graphics::{
    geometry::{AnchorX, AnchorY},
    prelude::*,
    primitives::{PrimitiveStyleBuilder, Rectangle},
    text::renderer::TextRenderer,
};

use crate::DrawWriter;

pub enum ElementHeight {
    Fixed(u32),
    Dynamic,
}

#[derive(Debug, Format, Clone, Copy)]
pub struct DynamicHeight;

impl TryFrom<ElementHeight> for u32 {
    type Error = DynamicHeight;

    fn try_from(value: ElementHeight) -> Result<Self, Self::Error> {
        match value {
            ElementHeight::Fixed(height) => Ok(height),
            ElementHeight::Dynamic => Err(DynamicHeight),
        }
    }
}

/// All elements must not be wider than 126 px.
/// All elements without a scrollbar must not be taller than 64 px.
/// This is so that we don't need to implement horizontal scrolling
/// or scroll a single element vertically.
pub trait Element<D: DrawTarget> {
    /// The display will be clippped so you have the entire display all to yourself.
    /// Returns the part of the display you received that you actually used.
    fn draw(&self, display: &mut D, bounding_box: Rectangle) -> Result<Rectangle, D::Error>;

    /// The height that this element needs in order to be fully in view
    fn height(&self, width: u32) -> ElementHeight;
}

/// Currently only supports 1-byte UTF-8 characters
pub struct TextElement<T, S> {
    pub text: T,
    pub character_style: S,
}

impl<D, T: Display, S: TextRenderer + Clone> Element<D> for TextElement<T, S>
where
    D: DrawTarget<Color = S::Color>,
{
    fn draw(&self, display: &mut D, bounding_box: Rectangle) -> Result<Rectangle, D::Error> {
        // TODO: Wrap text if it is too long
        let mut clipped = display.clipped(&bounding_box);
        let mut draw_writer = DrawWriter::new(
            &mut clipped,
            bounding_box.top_left,
            self.character_style.clone(),
        );
        let _ = write!(draw_writer, "{}", self.text);
        Ok(Rectangle::new(
            Point::zero(),
            Size::new(bounding_box.size.width, self.character_style.line_height()),
        ))
    }

    fn height(&self, _width: u32) -> ElementHeight {
        // TODO: Multi-line text and text wrapping
        ElementHeight::Fixed(self.character_style.line_height())
    }
}

/// Similar to a vertical CSS Flexbox
pub struct FlexElement<'a, E> {
    /// All elements must have a fixed height besides up to 1 dynanmic height element, which must be noted.
    pub elements: &'a [E],
    /// Similar to CSS Flexbox, you can choose one element to have its height grown or shrinked.
    pub dynamic_element: Option<usize>,
}

impl<D: DrawTarget> Element<D> for FlexElement<'_, &dyn Element<D>> {
    fn draw(
        &self,
        display: &mut D,
        bounding_box: Rectangle,
    ) -> Result<Rectangle, <D as DrawTarget>::Error> {
        if let Some(bottom_right) = bounding_box.bottom_right() {
            let dynamic_element_height = bounding_box.size.height.saturating_sub(
                self.elements
                    .into_iter()
                    .map(|element| {
                        u32::try_from(element.height(bounding_box.size.width)).unwrap_or(0)
                    })
                    .sum(),
            );
            let mut used_y = 0_u32;
            for (i, element) in self.elements.into_iter().enumerate() {
                used_y = element
                    .draw(
                        display,
                        Rectangle::with_corners(
                            Point::new(bounding_box.top_left.x, used_y.try_into().unwrap()),
                            if self.dynamic_element == Some(i) {
                                Point::new(
                                    bottom_right.x,
                                    (used_y + dynamic_element_height).try_into().unwrap(),
                                )
                            } else {
                                bottom_right
                            },
                        ),
                    )?
                    .bottom_right()
                    .map_or(0, |point| point.y.try_into().unwrap());
            }
            Ok(Rectangle::new(
                Point::zero(),
                Size::new(display.bounding_box().size.width, used_y),
            ))
        } else {
            Ok(Rectangle::zero())
        }
    }

    fn height(&self, width: u32) -> ElementHeight {
        if self.dynamic_element.is_some() {
            ElementHeight::Dynamic
        } else {
            ElementHeight::Fixed(
                self.elements
                    .into_iter()
                    .map(|element| u32::try_from(element.height(width)).unwrap())
                    .sum(),
            )
        }
    }
}

pub struct ScrollYElement<'a, D: DrawTarget, E> {
    pub element: &'a E,
    /// The y position of drawn elements will be subtracted by this amount to make elements
    /// that would otherwise be "below" the screen visible.
    pub scroll_y: u32,
    pub scrollbar_width: u32,
    pub scrollbar_color: D::Color,
}

impl<D: DrawTarget, E> ScrollYElement<'_, D, E> {
    /// Returns the new `scroll_y` to do just enough scrolling for the entire element to be seen.
    /// `size` is the size of the bounding box this element will be drawn with.
    pub fn scroll_into_view(&self, size: Size, element: BoundingHeight) -> u32 {
        todo!()
    }
}

impl<D: DrawTarget, E: Element<D>> Element<D> for ScrollYElement<'_, D, E> {
    fn draw(
        &self,
        display: &mut D,
        bounding_box: Rectangle,
    ) -> Result<Rectangle, <D as DrawTarget>::Error> {
        if let Some(element_width) = bounding_box.size.width.checked_sub(self.scrollbar_width) {
            self.element.draw(
                display,
                bounding_box.resized_width(element_width, AnchorX::Left),
            )?;
            // Draw the scrollbar
            let total_height = u32::try_from(self.element.height(element_width)).unwrap() as f64;
            let display_height = bounding_box.size.height as f64;
            if total_height > display_height {
                let scrollbar_height =
                    ((display_height / total_height * display_height) as u32).max(1);
                let scrollbar_y = (self.scroll_y as f64 / total_height * display_height) as u32;
                Rectangle::new(
                    bounding_box.top_left + Point::new(element_width as i32, scrollbar_y as i32),
                    Size::new(self.scrollbar_width, scrollbar_height),
                )
                .into_styled(
                    PrimitiveStyleBuilder::new()
                        .fill_color(self.scrollbar_color)
                        .build(),
                )
                .draw(
                    &mut display
                        .clipped(&bounding_box.resized_width(self.scrollbar_width, AnchorX::Right)),
                )?;
            }
            Ok(bounding_box)
        } else {
            // Width is too small to draw scrollbar, don't even try to draw anything
            Ok(bounding_box)
        }
    }

    fn height(&self, width: u32) -> ElementHeight {
        let _ = width;
        ElementHeight::Dynamic
    }
}

pub struct ListElement<I> {
    pub elements: I,
}

impl<D, E, I> Element<D> for ListElement<I>
where
    D: DrawTarget,
    E: Element<D>,
    I: IntoIterator<Item = E> + Clone,
{
    fn draw(
        &self,
        display: &mut D,
        bounding_box: Rectangle,
    ) -> Result<Rectangle, <D as DrawTarget>::Error> {
        let mut used_y = 0_u32;
        for element in self.elements.clone() {
            used_y = element
                .draw(
                    display,
                    bounding_box.resized_height(bounding_box.size.height - used_y, AnchorY::Bottom),
                )?
                .bottom_right()
                .map_or(0, |point| point.y.try_into().unwrap());
        }
        Ok(Rectangle::new(
            Point::zero(),
            Size::new(display.bounding_box().size.width, used_y),
        ))
    }

    fn height(&self, width: u32) -> ElementHeight {
        ElementHeight::Fixed(
            self.elements
                .clone()
                .into_iter()
                .map(|element| u32::try_from(element.height(width)).unwrap())
                .sum(),
        )
    }
}

#[derive(Debug, Format, Clone, Copy)]
pub struct BoundingHeight {
    pub y: u32,
    pub height: u32,
}

impl<I> ListElement<I> {
    pub fn bounding_box_of_element<D, E>(&self, width: u32, index: usize) -> BoundingHeight
    where
        D: DrawTarget,
        E: Element<D>,
        I: IntoIterator<Item = E> + Clone,
    {
        let mut elements = self.elements.clone().into_iter();
        let mut y = 0;
        for _ in 0..index {
            y += u32::try_from(elements.next().unwrap().height(width)).unwrap();
        }
        let height = elements.next().unwrap().height(width).try_into().unwrap();
        BoundingHeight { y, height }
    }
}
