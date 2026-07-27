//! Typed views over the components gameplay actually reaches for.
//!
//! A [`DataComponentPatch`](crate::item::DataComponentPatch) holds every value
//! as the bytes it occupied, which is what makes forwarding lossless. This
//! module is the other half: reading one of those spans as a structure, and
//! turning a structure back into a span.
//!
//! Only a subset is modelled. That is a deliberate split rather than an
//! unfinished one -- a component with no [`Payload`] here still survives a
//! decode and re-encode unchanged, because losslessness comes from
//! [`ComponentType::shape`] and not from anything in this file. Adding a type
//! here buys convenience, never correctness.

use crate::{
    Encode, Error, Reader, Result, Writer,
    item::{ComponentType, DataComponentPatch, nbt::NbtScan},
};

/// A component value this crate can read as a structure.
pub trait Payload<'a>: Sized {
    /// The component type this payload belongs to.
    const TYPE: ComponentType;

    /// Parse the value bytes for this component.
    ///
    /// `bytes` is exactly the span the value occupied, so an implementation
    /// that leaves anything unread has misread the layout and must say so.
    /// `nbt` is needed only by payloads that contain tags.
    ///
    /// # Errors
    /// Returns an error when the bytes do not match this component's layout.
    fn from_value(bytes: &'a [u8], nbt: &dyn NbtScan) -> Result<Self>;

    /// Write this value in the form the component's codec expects.
    ///
    /// # Errors
    /// Returns an error when a field violates a protocol limit.
    fn to_value(&self, writer: &mut Writer) -> Result<()>;
}

impl DataComponentPatch<'_> {
    /// Read one component as a structure.
    ///
    /// Returns `None` when the patch does not set that component at all, which
    /// is not the same as setting it to an empty value.
    ///
    /// The result borrows from the patch rather than from the buffer the patch
    /// was decoded out of, so this works the same whether the value is still a
    /// slice of the original packet or has since been replaced.
    ///
    /// # Errors
    /// Returns an error when the stored bytes do not parse as `T`.
    pub fn get<'p, T: Payload<'p>>(&'p self, nbt: &dyn NbtScan) -> Option<Result<T>> {
        Some(T::from_value(self.raw(T::TYPE)?, nbt))
    }

    /// Set one component from a structure.
    ///
    /// # Errors
    /// Returns an error when the value cannot be encoded.
    pub fn set<'v, T: Payload<'v>>(&mut self, value: &T) -> Result<()> {
        let mut writer = Writer::new();
        value.to_value(&mut writer)?;
        self.set_raw(T::TYPE, writer.into_vec());
        Ok(())
    }
}

/// Run `parse` over `bytes` and insist it consumed all of them.
fn exactly<'a, T>(
    bytes: &'a [u8],
    parse: impl FnOnce(&mut Reader<'a>) -> Result<T>,
) -> Result<T> {
    let mut reader = Reader::new(bytes);
    let value = parse(&mut reader)?;
    reader.finish()?;
    Ok(value)
}

/// A text component, held as the NBT tag it is on the wire.
///
/// Text is another module's slice. Carrying the bytes keeps that boundary
/// honest: an item can be read, edited and forwarded without a text
/// implementation present, and a caller that has one hands it [`Text::bytes`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Text<'a>(&'a [u8]);

impl<'a> Text<'a> {
    /// Wrap bytes that are already a network NBT tag.
    #[must_use]
    pub const fn from_bytes(bytes: &'a [u8]) -> Self {
        Self(bytes)
    }

    /// The tag, ready to hand to an NBT or text decoder.
    #[must_use]
    pub const fn bytes(self) -> &'a [u8] {
        self.0
    }

    /// Read one tag, using `nbt` to find where it ends.
    fn read(reader: &mut Reader<'a>, nbt: &dyn NbtScan) -> Result<Self> {
        let length = nbt.tag_len(reader.remaining())?;
        Ok(Self(reader.take(length)?))
    }
}

/// Generate a payload that is a single `VarInt`.
macro_rules! var_int_payload {
    ($(#[$meta:meta])* $name:ident => $kind:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub struct $name(pub i32);

        impl Payload<'_> for $name {
            const TYPE: ComponentType = ComponentType::$kind;

            fn from_value(bytes: &[u8], _nbt: &dyn NbtScan) -> Result<Self> {
                exactly(bytes, |reader| reader.var_int().map(Self))
            }

            fn to_value(&self, writer: &mut Writer) -> Result<()> {
                writer.var_int(self.0);
                Ok(())
            }
        }
    };
}

var_int_payload! {
    /// `minecraft:damage`: durability already used, counting up from zero.
    Damage => Damage
}

var_int_payload! {
    /// `minecraft:max_damage`: durability the item starts with.
    MaxDamage => MaxDamage
}

var_int_payload! {
    /// `minecraft:max_stack_size`, between 1 and 99.
    MaxStackSize => MaxStackSize
}

var_int_payload! {
    /// `minecraft:repair_cost`: the anvil's accumulated prior-work penalty.
    RepairCost => RepairCost
}

/// `minecraft:custom_name`: the name a player gave the item.
///
/// Distinct from `minecraft:item_name`, which is the item type's own name and
/// which an anvil rename does not touch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CustomName<'a>(pub Text<'a>);

impl<'a> Payload<'a> for CustomName<'a> {
    const TYPE: ComponentType = ComponentType::CustomName;

    fn from_value(bytes: &'a [u8], nbt: &dyn NbtScan) -> Result<Self> {
        exactly(bytes, |reader| Text::read(reader, nbt).map(Self))
    }

    fn to_value(&self, writer: &mut Writer) -> Result<()> {
        writer.raw(self.0.bytes());
        Ok(())
    }
}

/// `minecraft:item_name`: the item type's name, overridable by a datapack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemName<'a>(pub Text<'a>);

impl<'a> Payload<'a> for ItemName<'a> {
    const TYPE: ComponentType = ComponentType::ItemName;

    fn from_value(bytes: &'a [u8], nbt: &dyn NbtScan) -> Result<Self> {
        exactly(bytes, |reader| Text::read(reader, nbt).map(Self))
    }

    fn to_value(&self, writer: &mut Writer) -> Result<()> {
        writer.raw(self.0.bytes());
        Ok(())
    }
}

/// `minecraft:lore`: the extra tooltip lines under the name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lore<'a>(pub Vec<Text<'a>>);

impl<'a> Payload<'a> for Lore<'a> {
    const TYPE: ComponentType = ComponentType::Lore;

    fn from_value(bytes: &'a [u8], nbt: &dyn NbtScan) -> Result<Self> {
        exactly(bytes, |reader| {
            let count = crate::item::shape::check_count(reader.var_int()?, reader.remaining_len())?;
            let mut lines = Vec::with_capacity(count.min(MAX_LORE_LINES));
            for _ in 0..count {
                lines.push(Text::read(reader, nbt)?);
            }
            Ok(Self(lines))
        })
    }

    fn to_value(&self, writer: &mut Writer) -> Result<()> {
        let count = i32::try_from(self.0.len()).map_err(|_| Error::NegativeLength(-1))?;
        writer.var_int(count);
        for line in &self.0 {
            writer.raw(line.bytes());
        }
        Ok(())
    }
}

/// `ItemLore.STREAM_CODEC` uses `ByteBufCodecs.list(256)`; the cap bounds the
/// preallocation without rejecting a longer list the server would have taken.
const MAX_LORE_LINES: usize = 256;

/// `minecraft:enchantments` and `minecraft:stored_enchantments`.
///
/// Both share `ItemEnchantments.STREAM_CODEC`; the second is what an enchanted
/// book carries, since the book itself is not enchanted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Enchantments {
    /// `minecraft:enchantment` registry id paired with its level.
    pub levels: Vec<(i32, i32)>,
    /// False when this is `stored_enchantments`.
    pub applied: bool,
}

impl Enchantments {
    fn read(bytes: &[u8], applied: bool) -> Result<Self> {
        exactly(bytes, |reader| {
            let count = crate::item::shape::check_count(reader.var_int()?, reader.remaining_len())?;
            let mut levels = Vec::with_capacity(count.min(SANE_PREALLOC));
            for _ in 0..count {
                levels.push((reader.var_int()?, reader.var_int()?));
            }
            Ok(Self { levels, applied })
        })
    }
}

impl Payload<'_> for Enchantments {
    const TYPE: ComponentType = ComponentType::Enchantments;

    fn from_value(bytes: &[u8], _nbt: &dyn NbtScan) -> Result<Self> {
        Self::read(bytes, true)
    }

    fn to_value(&self, writer: &mut Writer) -> Result<()> {
        let count = i32::try_from(self.levels.len()).map_err(|_| Error::NegativeLength(-1))?;
        writer.var_int(count);
        for (enchantment, level) in &self.levels {
            writer.var_int(*enchantment);
            writer.var_int(*level);
        }
        Ok(())
    }
}

/// `minecraft:stored_enchantments`, the enchantments an enchanted book offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredEnchantments(pub Enchantments);

impl Payload<'_> for StoredEnchantments {
    const TYPE: ComponentType = ComponentType::StoredEnchantments;

    fn from_value(bytes: &[u8], _nbt: &dyn NbtScan) -> Result<Self> {
        Enchantments::read(bytes, false).map(Self)
    }

    fn to_value(&self, writer: &mut Writer) -> Result<()> {
        self.0.to_value(writer)
    }
}

/// How a modifier combines with the attribute's base value.
///
/// `AttributeModifier.Operation`, written as its `idMapper` id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    /// Added to the base value.
    AddValue,
    /// Added as a fraction of the base value.
    AddMultipliedBase,
    /// Multiplied into the running total.
    AddMultipliedTotal,
}

impl Operation {
    const fn from_id(id: i32) -> Option<Self> {
        match id {
            0 => Some(Self::AddValue),
            1 => Some(Self::AddMultipliedBase),
            2 => Some(Self::AddMultipliedTotal),
            _ => None,
        }
    }

    const fn id(self) -> i32 {
        match self {
            Self::AddValue => 0,
            Self::AddMultipliedBase => 1,
            Self::AddMultipliedTotal => 2,
        }
    }
}

/// How a modifier is shown in the tooltip.
///
/// `ItemAttributeModifiers.Display`. The override text is a text component, so
/// it is carried as bytes for the same reason [`Text`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Display<'a> {
    /// The usual generated line.
    Default,
    /// Nothing at all.
    Hidden,
    /// A replacement line.
    Override(Text<'a>),
}

/// One entry of `minecraft:attribute_modifiers`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AttributeModifier<'a> {
    /// `minecraft:attribute` registry id.
    pub attribute: i32,
    /// Identifier distinguishing this modifier from others on the same item.
    pub id: &'a str,
    /// Magnitude, interpreted per [`Operation`].
    pub amount: f64,
    /// How the amount combines with the base value.
    pub operation: Operation,
    /// `EquipmentSlotGroup` id: which slots the modifier applies in.
    pub slot: i32,
    /// Tooltip treatment.
    pub display: Display<'a>,
}

/// `minecraft:attribute_modifiers`.
#[derive(Debug, Clone, PartialEq)]
pub struct AttributeModifiers<'a>(pub Vec<AttributeModifier<'a>>);

impl<'a> Payload<'a> for AttributeModifiers<'a> {
    const TYPE: ComponentType = ComponentType::AttributeModifiers;

    fn from_value(bytes: &'a [u8], nbt: &dyn NbtScan) -> Result<Self> {
        exactly(bytes, |reader| {
            let count = crate::item::shape::check_count(reader.var_int()?, reader.remaining_len())?;
            let mut entries = Vec::with_capacity(count.min(SANE_PREALLOC));
            for _ in 0..count {
                let attribute = reader.var_int()?;
                let id = reader.string()?;
                let amount = reader.f64()?;
                let operation_id = reader.var_int()?;
                let operation =
                    Operation::from_id(operation_id).ok_or(Error::InvalidEnum {
                        name: "attribute modifier operation",
                        value: operation_id,
                    })?;
                let slot = reader.var_int()?;
                let display_id = reader.var_int()?;
                let display = match display_id {
                    1 => Display::Hidden,
                    2 => Display::Override(Text::read(reader, nbt)?),
                    // `ByIdMap.continuous(..., ZERO)` maps every other id to
                    // `default`, so this arm has to be the catch-all rather
                    // than an error, or a stream the server accepts is refused.
                    _ => Display::Default,
                };
                entries.push(AttributeModifier {
                    attribute,
                    id,
                    amount,
                    operation,
                    slot,
                    display,
                });
            }
            Ok(Self(entries))
        })
    }

    fn to_value(&self, writer: &mut Writer) -> Result<()> {
        let count = i32::try_from(self.0.len()).map_err(|_| Error::NegativeLength(-1))?;
        writer.var_int(count);
        for entry in &self.0 {
            writer.var_int(entry.attribute);
            writer.string(entry.id)?;
            writer.f64(entry.amount);
            writer.var_int(entry.operation.id());
            writer.var_int(entry.slot);
            match entry.display {
                Display::Default => writer.var_int(0),
                Display::Hidden => writer.var_int(1),
                Display::Override(text) => {
                    writer.var_int(2);
                    writer.raw(text.bytes());
                }
            }
        }
        Ok(())
    }
}

/// `minecraft:custom_data`: arbitrary NBT a datapack or plugin attached.
///
/// The one component with no vanilla meaning at all, and the usual place a
/// server keeps its own per-item state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CustomData<'a>(pub &'a [u8]);

impl<'a> Payload<'a> for CustomData<'a> {
    const TYPE: ComponentType = ComponentType::CustomData;

    fn from_value(bytes: &'a [u8], nbt: &dyn NbtScan) -> Result<Self> {
        // The span is one tag by construction, but measuring it again catches a
        // shape table that disagrees with the NBT reader.
        let length = nbt.tag_len(bytes)?;
        if length == bytes.len() {
            Ok(Self(bytes))
        } else {
            Err(Error::TrailingBytes(bytes.len() - length))
        }
    }

    fn to_value(&self, writer: &mut Writer) -> Result<()> {
        writer.raw(self.0);
        Ok(())
    }
}

/// `minecraft:item_model`: the model this item renders as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemModel<'a>(pub &'a str);

impl<'a> Payload<'a> for ItemModel<'a> {
    const TYPE: ComponentType = ComponentType::ItemModel;

    fn from_value(bytes: &'a [u8], _nbt: &dyn NbtScan) -> Result<Self> {
        exactly(bytes, |reader| reader.string().map(Self))
    }

    fn to_value(&self, writer: &mut Writer) -> Result<()> {
        writer.string(self.0)
    }
}

/// `minecraft:unbreakable`: a marker with no payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Unbreakable;

impl Payload<'_> for Unbreakable {
    const TYPE: ComponentType = ComponentType::Unbreakable;

    fn from_value(bytes: &[u8], _nbt: &dyn NbtScan) -> Result<Self> {
        if bytes.is_empty() {
            Ok(Self)
        } else {
            Err(Error::TrailingBytes(bytes.len()))
        }
    }

    fn to_value(&self, _writer: &mut Writer) -> Result<()> {
        Ok(())
    }
}

/// Matches the server's own clamp on collection preallocation
/// (`ByteBufCodecs.collection`), so a bogus count cannot force a large
/// allocation before the truncated read is noticed.
const SANE_PREALLOC: usize = 65536;

impl Encode for Text<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.raw(self.0);
        Ok(())
    }
}
