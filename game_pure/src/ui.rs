pub enum SelectedItem {
    /// The back button is selected
    Back,
    Item(usize),
}

pub struct Screen<Title, Items> {
    pub title: Title,
    pub can_go_back: bool,
    pub items: Items,
    pub selected_item: SelectedItem,
}
