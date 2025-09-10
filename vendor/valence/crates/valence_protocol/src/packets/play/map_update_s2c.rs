use std::borrow::Cow;
use std::io::Write;

use valence_bytes::{Bytes, CowBytes};
use valence_text::Text;

use crate::{Decode, DecodeBytes, DecodeBytesAuto, Encode, Packet, VarInt};

#[derive(Clone, PartialEq, Debug, Packet)]
pub struct MapUpdateS2c<'a> {
    pub map_id: VarInt,
    pub scale: i8,
    pub locked: bool,
    pub icons: Option<Vec<Icon<'a>>>,
    pub data: Option<Data<'a>>,
}

#[derive(Clone, PartialEq, Debug, Encode, Decode, DecodeBytesAuto)]
pub struct Icon<'a> {
    pub icon_type: IconType,
    /// In map coordinates; -128 for furthest left, +127 for furthest right
    pub position: [i8; 2],
    /// 0 is a vertical icon and increments by 22.5°
    pub direction: i8,
    pub display_name: Option<Cow<'a, Text>>,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, Encode, Decode, DecodeBytesAuto)]
pub enum IconType {
    WhiteArrow,
    GreenArrow,
    RedArrow,
    BlueArrow,
    WhiteCross,
    RedPointer,
    WhiteCircle,
    SmallWhiteCircle,
    Mansion,
    Temple,
    WhiteBanner,
    OrangeBanner,
    MagentaBanner,
    LightBlueBanner,
    YellowBanner,
    LimeBanner,
    PinkBanner,
    GrayBanner,
    LightGrayBanner,
    CyanBanner,
    PurpleBanner,
    BlueBanner,
    BrownBanner,
    GreenBanner,
    RedBanner,
    BlackBanner,
    TreasureMarker,
}

#[derive(Clone, PartialEq, Eq, Debug, Encode)]
pub struct Data<'a> {
    pub columns: u8,
    pub rows: u8,
    pub position: [i8; 2],
    pub data: CowBytes<'a>,
}

impl Encode for MapUpdateS2c<'_> {
    fn encode(&self, mut w: impl Write) -> anyhow::Result<()> {
        self.map_id.encode(&mut w)?;
        self.scale.encode(&mut w)?;
        self.locked.encode(&mut w)?;
        self.icons.encode(&mut w)?;

        match &self.data {
            None => 0u8.encode(&mut w)?,
            Some(data) => data.encode(&mut w)?,
        }

        Ok(())
    }
}

impl<'a> DecodeBytes for MapUpdateS2c<'a> {
    fn decode_bytes(r: &mut Bytes) -> anyhow::Result<Self> {
        let map_id = VarInt::decode_bytes(r)?;
        let scale = i8::decode_bytes(r)?;
        let locked = bool::decode_bytes(r)?;
        let icons = <Option<Vec<Icon<'static>>>>::decode_bytes(r)?;
        let columns = u8::decode_bytes(r)?;

        let data = if columns > 0 {
            let rows = u8::decode_bytes(r)?;
            let position = <[i8; 2]>::decode_bytes(r)?;
            let data = CowBytes::decode_bytes(r)?;

            Some(Data {
                columns,
                rows,
                position,
                data,
            })
        } else {
            None
        };

        Ok(Self {
            map_id,
            scale,
            locked,
            icons,
            data,
        })
    }
}
