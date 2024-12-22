#![allow(missing_docs, reason = "todo: fix")]

use std::{collections::HashMap, io::Write};

use derive_build::Build;
use flecs_ecs::macros::Component;
use slotmap::{SecondaryMap, SlotMap, new_key_type};
use valence_protocol::{Encode, ItemKind, ItemStack, Packet};

/// Represents a packet sent from the server to the client to synchronize recipes.
#[derive(Clone, Debug, Encode, Packet)]
pub struct SynchronizeRecipesS2c {
    /// The list of recipes to synchronize.
    pub recipes: Vec<Recipe>,
}

/// Represents a single recipe in the Minecraft game.
#[derive(Clone, Debug, Encode)]
pub struct Recipe {
    /// The type of the recipe.
    pub kind: &'static str,
    /// The unique identifier for this recipe.
    pub recipe_id: String,
    /// The specific data for this recipe, depending on its type.
    pub data: RecipeData,
}

/// Represents the different types of recipe data.
#[derive(Clone, Debug)]
pub enum RecipeData {
    CraftingShapeless(CraftingShapelessData),
    // CraftingShaped(CraftingShapedData<'a>),
    // CraftingSpecialArmordye(CraftingSpecialData),
    // CraftingSpecialBookcloning(CraftingSpecialData),
    // CraftingSpecialMapcloning(CraftingSpecialData),
    // CraftingSpecialMapextending(CraftingSpecialData),
    // CraftingSpecialFireworkRocket(CraftingSpecialData),
    // CraftingSpecialFireworkStar(CraftingSpecialData),
    // CraftingSpecialFireworkStarFade(CraftingSpecialData),
    // CraftingSpecialRepairitem(CraftingSpecialData),
    // CraftingSpecialTippedarrow(CraftingSpecialData),
    // CraftingSpecialBannerduplicate(CraftingSpecialData),
    // CraftingSpecialShielddecoration(CraftingSpecialData),
    // CraftingSpecialShulkerboxcoloring(CraftingSpecialData),
    // CraftingSpecialSuspiciousstew(CraftingSpecialData),
    // CraftingDecoratedPot(CraftingSpecialData),
    // Smelting(SmeltingData<'a>),
    // Blasting(SmeltingData<'a>),
    // Smoking(SmeltingData<'a>),
    // CampfireCooking(SmeltingData<'a>),
    // Stonecutting(StonecuttingData<'a>),
    // SmithingTransform(SmithingTransformData<'a>),
    // SmithingTrim(SmithingTrimData<'a>),
}

impl Encode for RecipeData {
    fn encode(&self, w: impl Write) -> anyhow::Result<()> {
        match self {
            Self::CraftingShapeless(data) => data.encode(w),
            // RecipeData::CraftingShaped(data) => data.encode(w),
            // RecipeData::CraftingSpecialArmordye(data) => data.encode(w),
            // RecipeData::CraftingSpecialBookcloning(data) => data.encode(w),
            // RecipeData::CraftingSpecialMapcloning(data) => data.encode(w),
            // RecipeData::CraftingSpecialMapextending(data) => data.encode(w),
            // RecipeData::CraftingSpecialFireworkRocket(data) => data.encode(w),
            // RecipeData::CraftingSpecialFireworkStar(data) => data.encode(w),
            // RecipeData::CraftingSpecialFireworkStarFade(data) => data.encode(w),
            // RecipeData::CraftingSpecialRepairitem(data) => data.encode(w),
            // RecipeData::CraftingSpecialTippedarrow(data) => data.encode(w),
            // RecipeData::CraftingSpecialBannerduplicate(data) => data.encode(w),
            // RecipeData::CraftingSpecialShielddecoration(data) => data.encode(w),
            // RecipeData::CraftingSpecialShulkerboxcoloring(data) => data.encode(w),
            // RecipeData::CraftingSpecialSuspiciousstew(data) => data.encode(w),
            // RecipeData::CraftingDecoratedPot(data) => data.encode(w),
            // RecipeData::Smelting(data) => data.encode(w),
            // RecipeData::Blasting(data) => data.encode(w),
            // RecipeData::Smoking(data) => data.encode(w),
            // RecipeData::CampfireCooking(data) => data.encode(w),
            // RecipeData::Stonecutting(data) => data.encode(w),
            // RecipeData::SmithingTransform(data) => data.encode(w),
            // RecipeData::SmithingTrim(data) => data.encode(w),
        }
    }
}

/// Represents data for a shapeless crafting recipe.
#[derive(Clone, Debug, Build)]
pub struct CraftingShapelessData {
    /// Used to group similar recipes together in the recipe book.
    group: String,
    /// The category of the recipe.
    category: CraftingCategory,
    /// The list of ingredients for the recipe.
    ingredients: Vec<Ingredient>,
    /// The result of the crafting recipe.
    #[required]
    result: ItemStack,
}

/// Represents data for special crafting recipes.
#[derive(Clone, Debug)]
pub struct CraftingSpecialData {
    /// The category of the special crafting recipe.
    pub category: CraftingCategory,
}

/// Represents the categories for crafting recipes.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Encode, Default)]
pub enum CraftingCategory {
    Building,
    Redstone,
    Equipment,
    #[default]
    Misc,
}

/// Represents the categories for smelting recipes.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Encode)]
pub enum SmeltingCategory {
    Food,
    Blocks,
    Misc,
}

/// Represents an ingredient in a recipe, which can be multiple possible items.
#[derive(Encode, Clone, Debug)]
struct Ingredient(Vec<ItemStack>);

impl From<Vec<ItemStack>> for Ingredient {
    fn from(value: Vec<ItemStack>) -> Self {
        Self(value)
    }
}

impl From<ItemStack> for Ingredient {
    fn from(value: ItemStack) -> Self {
        Self(vec![value])
    }
}

impl From<ItemKind> for Ingredient {
    fn from(value: ItemKind) -> Self {
        Self(vec![ItemStack::new(value, 1, None)])
    }
}

// Implement Encode for all structs
impl Encode for CraftingShapelessData {
    fn encode(&self, mut w: impl Write) -> anyhow::Result<()> {
        self.group.encode(&mut w)?;
        self.category.encode(&mut w)?;
        self.ingredients.encode(&mut w)?;
        self.result.encode(w)
    }
}

impl Encode for CraftingSpecialData {
    fn encode(&self, w: impl Write) -> anyhow::Result<()> {
        self.category.encode(w)
    }
}

#[derive(Debug, Encode, Packet)]
pub struct UnlockRecipesS2c {
    pub action: Action,
    pub crafting_recipe_book: RecipeBookState,
    pub smelting_recipe_book: RecipeBookState,
    pub blast_furnace_recipe_book: RecipeBookState,
    pub smoker_recipe_book: RecipeBookState,
    pub recipe_ids_1: Vec<String>,
    pub recipe_ids_2: Vec<String>,
}

#[derive(Debug, Encode)]
pub enum Action {
    Init,
    Add,
    Remove,
}

#[derive(Debug, Encode)]
pub struct RecipeBookState {
    pub open: bool,
    pub filter_active: bool,
}

impl RecipeBookState {
    pub const FALSE: Self = Self {
        open: false,
        filter_active: false,
    };
}

// since 3x3 grid are max
pub type Crafting3x3 = [ItemKind; 9];
pub type Crafting2x2 = [ItemKind; 4];

#[derive(Debug, Hash, PartialEq, Eq)]
struct SortedItemList(Crafting3x3);

impl From<Crafting3x3> for SortedItemList {
    fn from(mut list: Crafting3x3) -> Self {
        list.sort_unstable();
        Self(list)
    }
}

impl FromIterator<ItemKind> for SortedItemList {
    fn from_iter<T: IntoIterator<Item = ItemKind>>(iter: T) -> Self {
        let mut list: Crafting3x3 = [ItemKind::Air; 9];

        for (i, item) in iter.into_iter().enumerate() {
            // todo: more idiomatic way to do this? also without panic?
            list[i] = item;
        }

        Self::from(list)
    }
}

// Define a custom key type
new_key_type! { struct SortedItemId; }

#[derive(Component)]
pub struct CraftingRegistry {
    // changes when the registry is updated
    epoch: u64,

    shapeless_lookup: HashMap<SortedItemList, SortedItemId>,
    shapeless: SlotMap<SortedItemId, CraftingShapelessData>,
    shapeless_ids: SecondaryMap<SortedItemId, String>,
}

impl Default for CraftingRegistry {
    fn default() -> Self {
        let mut result = Self {
            epoch: 0,
            shapeless_lookup: HashMap::default(),
            shapeless: SlotMap::default(),
            shapeless_ids: SecondaryMap::default(),
        };

        let shapeless = CraftingShapelessData::new(ItemStack::new(ItemKind::OakPlanks, 4, None))
            .ingredient(ItemKind::OakLog);

        result.register_shapeless("hyperion:plank".to_string(), shapeless);

        result
    }
}

pub struct ShapelessRecipe<'a> {
    // recipe_id: &'a RecipeIdentifier,
    pub data: &'a CraftingShapelessData,
}

impl CraftingRegistry {
    fn mark_changed(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
    }

    #[must_use]
    pub fn packet(&self) -> Option<SynchronizeRecipesS2c> {
        if self.epoch == 0 {
            // we have not added anything
            return None;
        }

        let recipes: Vec<_> = self
            .shapeless
            .iter()
            .map(|(id, data)| {
                let recipe_id = self.shapeless_ids.get(id).unwrap();

                Recipe {
                    kind: "minecraft:crafting_shapeless",
                    recipe_id: recipe_id.to_string(),
                    data: RecipeData::CraftingShapeless(data.clone()),
                }
            })
            .collect();

        Some(SynchronizeRecipesS2c { recipes })
    }

    pub fn get_shapeless(
        &self,
        input: impl IntoIterator<Item = ItemKind>,
    ) -> Option<ShapelessRecipe<'_>> {
        let list: SortedItemList = input.into_iter().collect();
        let id = self.shapeless_lookup.get(&list).copied()?;

        // let recipe_id = self.shapeless_ids.get(id).unwrap();
        let data = self.shapeless.get(id).unwrap();

        Some(ShapelessRecipe { data })
    }

    fn register_shapeless(&mut self, recipe_id: String, data: CraftingShapelessData) {
        let list: SortedItemList = data
            .ingredients
            .iter()
            .flat_map(|x| &x.0)
            .map(|x| x.item)
            .collect();

        let entity_id = self.shapeless.insert(data);
        self.shapeless_ids.insert(entity_id, recipe_id);

        self.shapeless_lookup.insert(list, entity_id);

        self.mark_changed();
    }

    #[must_use]
    pub fn get_result_2x2(&self, grid: Crafting2x2) -> Option<&ItemStack> {
        if let Some(shapeless) = self.get_shapeless(grid) {
            return Some(&shapeless.data.result);
        }

        None
    }
}
