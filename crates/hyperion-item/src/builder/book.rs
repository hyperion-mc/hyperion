use hyperion::{ItemKind, ItemStack};
use valence_protocol::nbt;

use crate::builder::ItemBuilder;

#[derive(Clone, Debug)]
#[must_use]
pub struct BookBuilder {
    item: ItemBuilder,
}

// /give @p minecraft:written_book{author:"AuthorName",title:"BookTitle",pages:['{"text":"Page content"}']}

impl BookBuilder {
    pub fn new(author: impl Into<String>, title: impl Into<String>) -> Self {
        let mut item = ItemBuilder::new(ItemKind::WrittenBook);

        let author = author.into();
        let title = title.into();

        let mut nbt = nbt::Compound::new();

        nbt.insert("author", nbt::Value::String(author));
        nbt.insert("resolved", nbt::Value::Byte(1));
        nbt.insert("title", nbt::Value::String(title));
        nbt.insert("pages", nbt::Value::List(nbt::List::String(Vec::new())));

        item.nbt = Some(nbt);

        Self { item }
    }

    pub fn add_page(mut self, page: impl Into<String>) -> Self {
        let page = page.into();
        let json = format!(r#"{{"text":"{page}"}}"#);

        if let Some(nbt) = &mut self.item.nbt
            && let nbt::Value::List(nbt::List::String(pages)) = nbt.get_mut("pages").unwrap()
        {
            pages.push(json);
        }

        self
    }

    #[must_use]
    pub fn build(self) -> ItemStack {
        self.item.build()
    }
}
