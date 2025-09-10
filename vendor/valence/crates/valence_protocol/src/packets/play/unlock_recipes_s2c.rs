use std::io::Write;

use anyhow::bail;
use valence_bytes::Bytes;
use valence_ident::Ident;

use crate::{DecodeBytes, Encode, Packet, VarInt};

#[derive(Clone, PartialEq, Eq, Debug, Packet)]
pub struct UnlockRecipesS2c {
    pub action: UpdateRecipeBookAction,
    pub crafting_recipe_book_open: bool,
    pub crafting_recipe_book_filter_active: bool,
    pub smelting_recipe_book_open: bool,
    pub smelting_recipe_book_filter_active: bool,
    pub blast_furnace_recipe_book_open: bool,
    pub blast_furnace_recipe_book_filter_active: bool,
    pub smoker_recipe_book_open: bool,
    pub smoker_recipe_book_filter_active: bool,
    pub recipe_ids: Vec<Ident>,
}

impl DecodeBytes for UnlockRecipesS2c {
    fn decode_bytes(r: &mut Bytes) -> anyhow::Result<Self> {
        let action_id = VarInt::decode_bytes(r)?.0;

        let crafting_recipe_book_open = bool::decode_bytes(r)?;
        let crafting_recipe_book_filter_active = bool::decode_bytes(r)?;
        let smelting_recipe_book_open = bool::decode_bytes(r)?;
        let smelting_recipe_book_filter_active = bool::decode_bytes(r)?;
        let blast_furnace_recipe_book_open = bool::decode_bytes(r)?;
        let blast_furnace_recipe_book_filter_active = bool::decode_bytes(r)?;
        let smoker_recipe_book_open = bool::decode_bytes(r)?;
        let smoker_recipe_book_filter_active = bool::decode_bytes(r)?;
        let recipe_ids = Vec::decode_bytes(r)?;

        Ok(Self {
            action: match action_id {
                0 => UpdateRecipeBookAction::Init {
                    recipe_ids: Vec::decode_bytes(r)?,
                },
                1 => UpdateRecipeBookAction::Add,
                2 => UpdateRecipeBookAction::Remove,
                n => bail!("unknown recipe book action of {n}"),
            },
            crafting_recipe_book_open,
            crafting_recipe_book_filter_active,
            smelting_recipe_book_open,
            smelting_recipe_book_filter_active,
            blast_furnace_recipe_book_open,
            blast_furnace_recipe_book_filter_active,
            smoker_recipe_book_open,
            smoker_recipe_book_filter_active,
            recipe_ids,
        })
    }
}

impl Encode for UnlockRecipesS2c {
    fn encode(&self, _w: impl Write) -> anyhow::Result<()> {
        todo!()
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum UpdateRecipeBookAction {
    Init { recipe_ids: Vec<Ident> },
    Add,
    Remove,
}
