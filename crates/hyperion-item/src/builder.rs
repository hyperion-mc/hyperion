use flecs_ecs::core::Entity;
use valence_protocol::{ItemKind, ItemStack, nbt, nbt::Value};

mod book;
pub use book::BookBuilder;

/// A builder for creating Minecraft items with NBT data
#[derive(Clone, Debug)]
#[must_use]
pub struct ItemBuilder {
    kind: ItemKind,
    count: i8,
    nbt: Option<nbt::Compound<String>>,
}

/// Represents a Minecraft attribute that can be applied to items
pub trait Attribute: Copy {
    fn create_modifier(&self) -> nbt::Compound<String>;
}

// Example attribute implementations - now as value-carrying structs
#[derive(Copy, Clone, Debug)]
pub struct AttackDamage(pub f64);

#[derive(Copy, Clone, Debug)]
pub struct AttackSpeed(pub f64);

#[derive(Copy, Clone, Debug)]
pub struct MaxHealth(pub f64);

// Implement Attribute for each type
impl Attribute for AttackDamage {
    fn create_modifier(&self) -> nbt::Compound<String> {
        let mut modifier = nbt::Compound::new();
        modifier.insert(
            "AttributeName",
            "minecraft:generic.attack_damage".to_string(),
        );
        modifier.insert("Name", "generic.attack_damage".to_string());
        modifier.insert("Amount", self.0);
        modifier.insert("Operation", 0_i32);
        modifier
    }
}

impl Attribute for AttackSpeed {
    fn create_modifier(&self) -> nbt::Compound<String> {
        let mut modifier = nbt::Compound::new();
        modifier.insert(
            "AttributeName",
            "minecraft:generic.attack_speed".to_string(),
        );
        modifier.insert("Name", "generic.attack_speed".to_string());
        modifier.insert("Amount", self.0);
        modifier.insert("Operation", 0_i32);
        modifier
    }
}

impl Attribute for MaxHealth {
    fn create_modifier(&self) -> nbt::Compound<String> {
        let mut modifier = nbt::Compound::new();
        modifier.insert("AttributeName", "minecraft:generic.max_health".to_string());
        modifier.insert("Name", "generic.max_health".to_string());
        modifier.insert("Amount", self.0);
        modifier.insert("Operation", 0_i32);
        modifier
    }
}

#[derive(Copy, Clone, Debug)]
pub struct Color(pub u8, pub u8, pub u8);

impl ItemBuilder {
    pub const fn new(kind: ItemKind) -> Self {
        Self {
            kind,
            count: 1,
            nbt: None,
        }
    }

    /// Sets the color of a leather armor item
    ///
    /// # Example
    /// ```
    /// // Create a red leather helmet
    /// use hyperion::ItemKind;
    /// use hyperion_item::builder::{Color, ItemBuilder};
    /// let item = ItemBuilder::new(ItemKind::LeatherHelmet)
    ///     .color(Color(255, 0, 0))
    ///     .build();
    /// ```
    pub fn color(mut self, color: Color) -> Self {
        let nbt = self.nbt.get_or_insert_with(nbt::Compound::new);

        // Create or get existing display compound
        let display = match nbt.remove("display") {
            Some(Value::Compound(display)) => display,
            _ => nbt::Compound::new(),
        };

        let r = color.0;
        let g = color.1;
        let b = color.2;

        // Create a new display compound with the color
        let mut new_display = display;
        let color = (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b);
        let color = bytemuck::cast(color);
        new_display.insert("color", Value::Int(color));

        // Insert the updated display compound
        nbt.insert("display", Value::Compound(new_display));
        self
    }

    pub const fn kind(mut self, kind: ItemKind) -> Self {
        self.kind = kind;
        self
    }

    /// Sets a custom name for the item
    pub fn name(mut self, name: impl Into<String>) -> Self {
        let nbt = self.nbt.get_or_insert_with(nbt::Compound::new);

        // Create or get existing display compound
        let display = match nbt.remove("display") {
            Some(Value::Compound(display)) => display,
            _ => nbt::Compound::new(),
        };

        // Create a new display compound with the name
        let mut new_display = display;

        let name = name.into();

        // '{"text":"Your Custom Name"}'
        let name = format!(r#"{{"text":"{name}"}}"#);

        new_display.insert("Name", Value::String(name));

        // Insert the updated display compound
        nbt.insert("display", Value::Compound(new_display));
        self
    }

    pub const fn count(mut self, count: i8) -> Self {
        self.count = count;
        self
    }

    pub fn handler(mut self, handler: Entity) -> Self {
        let nbt = self.nbt.get_or_insert_with(nbt::Compound::new);
        let id = handler.0;

        // we are explicitly casting to i64 because although sign might be lost, when we read it back,
        // we will revert it back to a u64.
        let id: i64 = bytemuck::cast(id);
        nbt.insert("Handler", Value::Long(id));
        self
    }

    pub fn glowing(mut self) -> Self {
        let nbt = self.nbt.get_or_insert_with(nbt::Compound::new);
        nbt.insert(
            "Enchantments",
            Value::List(nbt::list::List::Compound(vec![nbt::Compound::new()])),
        );
        self
    }

    pub fn add_attribute(mut self, attribute: impl Attribute) -> Self {
        let nbt = self.nbt.get_or_insert_with(nbt::Compound::new);
        let mut modifiers = match nbt.remove("AttributeModifiers") {
            Some(Value::List(nbt::list::List::Compound(modifiers))) => modifiers,
            _ => Vec::new(),
        };

        modifiers.push(attribute.create_modifier());

        nbt.insert(
            "AttributeModifiers",
            Value::List(nbt::list::List::Compound(modifiers)),
        );
        self
    }

    #[must_use]
    pub fn build(self) -> ItemStack {
        ItemStack::new(self.kind, self.count, self.nbt)
    }
}

// Example usage
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_item_builder() {
        let _sword = ItemBuilder::new(ItemKind::DiamondSword)
            .count(1)
            .glowing()
            .add_attribute(AttackDamage(7.0))
            .add_attribute(AttackSpeed(1.6))
            .build();

        // Add assertions here
    }
}
