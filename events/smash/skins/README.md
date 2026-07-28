# Kit skins

Each kit is a mob, so a player playing that kit wears that mob's skin. This
directory holds one pair of files per mob, and `crate::kit_skin!("zombie")` in
`src/module/kits/zombie.rs` is what puts the pair on the kit prefab.

    zombie.value   base64 of Mojang's textures payload: the profile property value
    zombie.sig     base64 RSA-SHA1 signature over those exact bytes

## An unsigned skin is a skin only its wearer can see

This is the constraint the whole directory exists to satisfy, and it is
invisible from the server: send an unsigned `textures` property and every packet
goes out clean, the wearer's own client renders it, and everyone else quietly
sees Steve.

Read from the 26.2 client jar, sha1 `2dc72797acbc1b63fc16a11c4ac393605f453754`,
which is `downloads.client` in the 26.2 version manifest. 26.2 ships
unobfuscated, so these are Mojang's names and not a mapping's guess:

- `PlayerInfo.createSkinLookup` builds the lookup with
  `requireSecure = !minecraft.isLocalPlayer(profile.id())`. Yourself, false.
  Everyone else, true.
- `SkinManager.createLookup` then keeps a skin only when
  `!requireSecure || skin.secure()`, falling back to `DefaultPlayerSkin`.
- `skin.secure()` is set where the skin is built, from
  `textures.signatureState() == SignatureState.SIGNED`.
- authlib 9.0.75 `YggdrasilMinecraftSessionService.getPropertySignatureState`
  returns `UNSIGNED` with no signature, `INVALID` when no Mojang profile
  property key validates it, and `SIGNED` otherwise.
- `unpackTextures` throws the whole payload away, signature and all, unless
  every texture URL satisfies `TextureUrlChecker.isAllowedTextureDomain`, whose
  allow-list is exactly `{"textures.minecraft.net"}`.

Two consequences worth stating plainly. Nothing about this is conditional on the
server's online-mode flag, so an offline-mode server does not get a pass. And a
skin cannot be self-hosted: the bytes must already live on Mojang's texture
host, which in practice means the payload has to come from a real account.

`nix run .#check-kit-skins` enforces all of it offline, against
`mojang-profile-keys.json` in this directory.

## Where these came from, and what has not been checked

Every payload is a real Mojang-signed property, fetched from the MineSkin public
API, which mints them by uploading through real accounts. A captured payload
stays valid forever: the signature covers the base64 value, and the texture
object behind the URL is immutable.

Selection was mechanical rather than by eye. MineSkin's own search filters by
name and tag, so every candidate was already tagged as the mob; within that pool
the winner is the skin whose colour histogram is closest to the mob's actual
vanilla texture out of the client jar, over opaque pixels, in a 6x6x6 RGB
histogram compared by L1 distance. Distance runs 0 (identical palette) to 1.

| mob | MineSkin name | palette distance | MineSkin uuid |
| --- | --- | --- | --- |
| blaze | (unnamed) | 0.141 | e4cc34136b53497590407d02fbd3ade8 |
| chicken | (unnamed) | 0.155 | 20a1a090e506414e93b9245af118444f |
| cow | (unnamed) | 0.178 | 93f3a3cb9e1f4f8ab9234a99d71644e7 |
| creeper | (unnamed) | 0.053 | bac3e4bb13b349f388031423a76b0947 |
| enderman | Enderman | 0.005 | 322e7010273247ebba0c00432b7b0e5c |
| guardian | (unnamed) | 0.602 | 6de8b51c394942eb992f3264ebdd9df6 |
| iron_golem | Iron Golem | 0.229 | 3e69f08abb184dc0acf3eb21b8078263 |
| skeleton | (unnamed) | 0.010 | 4f7dec981e0248dfa4a75b0872ae35d8 |
| sky_squid | (unnamed) | 0.481 | 03fcb26a59854104b672e3285b6c92bc |
| slime | (unnamed) | 0.321 | eb5ce4bf1d8c4266b47a518650f49b3b |
| snowman | kingnave87_BASE | 0.064 | 52594d9a4ff54cf89452708446d0b276 |
| spider | (unnamed) | 0.587 | 9fc6280bb6eb4c8c880689cd8311bbb5 |
| wither_skeleton | archmc-fd6aa49 | 0.019 | 6fa93c6e626d4bc1a592c61e6d8b13f5 |
| wolf | (unnamed) | 0.342 | 0a5d99ae4a1e4c56a796a557b903db0a |
| zombie | zombi | 0.159 | 949ada3ccf764d53b6d7e942d5235995 |

Two entries were taken from rank 2 rather than rank 1. Enderman, because rank 1
scored 0.0007 better and is named after an advertisement. Spider, because rank 1
is called "Spider man" and is the superhero.

**No human has looked at these fifteen images.** What is gated is that each
payload is genuinely Mojang-signed, points at a host the client will load from,
and is declared by exactly one kit. Whether the picture is a good likeness is
not gated and is not claimed. Palette distance is a weak proxy and it is weakest
where the mob is close to monochrome: a plain black skin scores near zero
against the enderman. The four worst matches are guardian (0.602), spider
(0.587), sky_squid (0.481) and wolf (0.342), and those are the ones to look at
first.

Replacing one is a two-file change with no code in it: put the new `value` and
`signature` in `<mob>.value` and `<mob>.sig` and run the check.
