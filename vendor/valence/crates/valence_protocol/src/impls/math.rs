use std::io::Write;

use valence_math::*;

use crate::{Decode, Encode, impl_decode_bytes_auto};

impl Encode for Vec2 {
    fn encode(&self, mut w: impl Write) -> anyhow::Result<()> {
        self.x.encode(&mut w)?;
        self.y.encode(w)
    }
}

impl Decode for Vec2 {
    fn decode(r: &mut &[u8]) -> anyhow::Result<Self> {
        Ok(Self {
            x: f32::decode(r)?,
            y: f32::decode(r)?,
        })
    }
}

impl_decode_bytes_auto!(Vec2);

impl Encode for Vec3 {
    fn encode(&self, mut w: impl Write) -> anyhow::Result<()> {
        self.x.encode(&mut w)?;
        self.y.encode(&mut w)?;
        self.z.encode(w)
    }
}

impl Decode for Vec3 {
    fn decode(r: &mut &[u8]) -> anyhow::Result<Self> {
        Ok(Self {
            x: f32::decode(r)?,
            y: f32::decode(r)?,
            z: f32::decode(r)?,
        })
    }
}

impl_decode_bytes_auto!(Vec3);

impl Encode for Vec3A {
    fn encode(&self, mut w: impl Write) -> anyhow::Result<()> {
        self.x.encode(&mut w)?;
        self.y.encode(&mut w)?;
        self.z.encode(w)
    }
}

impl Decode for Vec3A {
    fn decode(r: &mut &[u8]) -> anyhow::Result<Self> {
        Ok(Self::new(f32::decode(r)?, f32::decode(r)?, f32::decode(r)?))
    }
}

impl_decode_bytes_auto!(Vec3A);

impl Encode for IVec3 {
    fn encode(&self, mut w: impl Write) -> anyhow::Result<()> {
        self.x.encode(&mut w)?;
        self.y.encode(&mut w)?;
        self.z.encode(w)
    }
}

impl Decode for IVec3 {
    fn decode(r: &mut &[u8]) -> anyhow::Result<Self> {
        Ok(Self {
            x: i32::decode(r)?,
            y: i32::decode(r)?,
            z: i32::decode(r)?,
        })
    }
}

impl_decode_bytes_auto!(IVec3);

impl Encode for Vec4 {
    fn encode(&self, mut w: impl Write) -> anyhow::Result<()> {
        self.x.encode(&mut w)?;
        self.y.encode(&mut w)?;
        self.z.encode(&mut w)?;
        self.w.encode(&mut w)
    }
}

impl Decode for Vec4 {
    fn decode(r: &mut &[u8]) -> anyhow::Result<Self> {
        Ok(Self::new(
            f32::decode(r)?,
            f32::decode(r)?,
            f32::decode(r)?,
            f32::decode(r)?,
        ))
    }
}

impl_decode_bytes_auto!(Vec4);

impl Encode for Quat {
    fn encode(&self, mut w: impl Write) -> anyhow::Result<()> {
        self.x.encode(&mut w)?;
        self.y.encode(&mut w)?;
        self.z.encode(&mut w)?;
        self.w.encode(w)
    }
}

impl Decode for Quat {
    fn decode(r: &mut &[u8]) -> anyhow::Result<Self> {
        Ok(Self::from_xyzw(
            f32::decode(r)?,
            f32::decode(r)?,
            f32::decode(r)?,
            f32::decode(r)?,
        ))
    }
}

impl_decode_bytes_auto!(Quat);

impl Encode for DVec2 {
    fn encode(&self, mut w: impl Write) -> anyhow::Result<()> {
        self.x.encode(&mut w)?;
        self.y.encode(w)
    }
}

impl Decode for DVec2 {
    fn decode(r: &mut &[u8]) -> anyhow::Result<Self> {
        Ok(Self {
            x: f64::decode(r)?,
            y: f64::decode(r)?,
        })
    }
}

impl_decode_bytes_auto!(DVec2);

impl Encode for DVec3 {
    fn encode(&self, mut w: impl Write) -> anyhow::Result<()> {
        self.x.encode(&mut w)?;
        self.y.encode(&mut w)?;
        self.z.encode(w)
    }
}

impl Decode for DVec3 {
    fn decode(r: &mut &[u8]) -> anyhow::Result<Self> {
        Ok(Self {
            x: f64::decode(r)?,
            y: f64::decode(r)?,
            z: f64::decode(r)?,
        })
    }
}

impl_decode_bytes_auto!(DVec3);

impl Encode for DQuat {
    fn encode(&self, mut w: impl Write) -> anyhow::Result<()> {
        self.x.encode(&mut w)?;
        self.y.encode(&mut w)?;
        self.z.encode(&mut w)?;
        self.w.encode(w)
    }
}

impl Decode for DQuat {
    fn decode(r: &mut &[u8]) -> anyhow::Result<Self> {
        Ok(Self::from_xyzw(
            f64::decode(r)?,
            f64::decode(r)?,
            f64::decode(r)?,
            f64::decode(r)?,
        ))
    }
}

impl_decode_bytes_auto!(DQuat);
