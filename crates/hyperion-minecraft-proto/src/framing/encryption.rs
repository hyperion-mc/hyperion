//! AES-CFB8, the cipher a connection switches to after `login_key`.
//!
//! `Crypt.getCipher` is the whole specification:
//!
//! ```java
//! Cipher cip = Cipher.getInstance("AES/CFB8/NoPadding");
//! cip.init(opMode, key, new IvParameterSpec(key.getEncoded()));
//! ```
//!
//! The key is the 16-byte shared secret the client generated and sent under
//! the server's RSA key, and the IV is *the same bytes*. There is no separate
//! IV on the wire and no nonce, which is why a connection can only ever be
//! encrypted once.
//!
//! CFB8 turns the block cipher into a byte-granular stream cipher, so a
//! ciphertext byte depends on every plaintext byte before it. That is what
//! forces [`FrameDecoder`](super::FrameDecoder) to decipher bytes as they
//! arrive rather than a frame at a time: the length prefix of frame *n* has
//! already been mixed into the keystream by the body of frame *n - 1*.

use aes::{Aes128, cipher::KeyIvInit};

/// Length of the shared secret, in bytes (`Crypt.SYMMETRIC_BITS` / 8).
pub const SHARED_SECRET_LEN: usize = 16;

/// AES-128 in CFB8, enciphering.
pub type Encryptor = cfb8::Encryptor<Aes128>;
/// AES-128 in CFB8, deciphering.
pub type Decryptor = cfb8::Decryptor<Aes128>;

/// One direction of a connection's cipher.
///
/// Separate instances for send and receive: CFB8 keeps a shift register that
/// advances with the bytes it has seen, and the two directions see different
/// bytes.
pub enum Cipher {
    /// Enciphers outbound bytes.
    Encrypt(Box<Encryptor>),
    /// Deciphers inbound bytes.
    Decrypt(Box<Decryptor>),
}

impl Cipher {
    /// A cipher that enciphers outbound bytes.
    #[must_use]
    pub fn encryptor(secret: &[u8; SHARED_SECRET_LEN]) -> Self {
        Self::Encrypt(Box::new(Encryptor::new(secret.into(), secret.into())))
    }

    /// A cipher that deciphers inbound bytes.
    #[must_use]
    pub fn decryptor(secret: &[u8; SHARED_SECRET_LEN]) -> Self {
        Self::Decrypt(Box::new(Decryptor::new(secret.into(), secret.into())))
    }

    /// Transform `bytes` in place, advancing the shift register past them.
    ///
    /// CFB8's block size is one byte, so every slice length is a whole number
    /// of blocks and a caller may split a stream wherever it likes.
    pub fn apply(&mut self, bytes: &mut [u8]) {
        match self {
            Self::Encrypt(cipher) => cipher.encrypt(bytes),
            Self::Decrypt(cipher) => cipher.decrypt(bytes),
        }
    }
}

// A derived Debug would print the round keys, which are the shared secret in
// expanded form. This one says only which direction the cipher runs in.
impl std::fmt::Debug for Cipher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let direction = match self {
            Self::Encrypt(_) => "encrypt",
            Self::Decrypt(_) => "decrypt",
        };
        f.debug_struct("Cipher")
            .field("direction", &direction)
            .finish()
    }
}
